# csd — Claude Session Driver

[![crates.io](https://img.shields.io/crates/v/claude-session-driver.svg)](https://crates.io/crates/claude-session-driver)
[![docs.rs](https://img.shields.io/docsrs/claude-session-driver)](https://docs.rs/claude-session-driver)
[![CI](https://github.com/marshallku/csd/actions/workflows/ci.yml/badge.svg)](https://github.com/marshallku/csd/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/claude-session-driver.svg)](./LICENSE)

Drive an interactive `claude` REPL inside a detached **tmux** session — on the flat-rate
**subscription seat** — then inject prompts reliably and read session state back as JSON. Built so
other tools (copad, a life-assistant server, your own scripts) can run autonomous Claude work by
shelling out to `csd` and parsing its output.

```text
        ┌──────────┐  spawn/send/approve   ┌───────────────────────────┐
  your  │   csd    │ ────────────────────▶ │ tmux session              │
 script │  (JSON)  │                       │   └─ interactive `claude`  │
        │          │ ◀──────────────────── │      (subscription seat)   │
        └──────────┘   state (hybrid)      └───────────────────────────┘
```

## Why

From 2026-06-15 `claude -p` / the Agent SDK becomes metered, so the only flat-rate substrate for
heavy autonomous agent work is the **interactive REPL** — which has no programmatic API. `csd` is
the missing driver: it scripts that TUI over tmux (spawn, input injection with readiness retries,
and a hybrid state detector) and exposes a clean JSON surface. See
[`docs/poc-findings.md`](docs/poc-findings.md) for the empirical basis.

> ⚠️ Driving the interactive REPL programmatically is undocumented territory. The backend is kept
> swappable (`claude` today, `codex` later) so a forced move is a config flip, not a rewrite.

## Install

```bash
cargo install claude-session-driver      # installs the `csd` binary
```

…or from source:

```bash
git clone https://github.com/marshallku/csd && cd csd
cargo install --path .
```

**Requirements:** `tmux` on `PATH`, and a logged-in `claude` CLI on the subscription seat (no
`ANTHROPIC_API_KEY` — that switches to metered billing).

## Quick start

The `claude -p` shape — one synchronous command, answer on stdout:

```bash
# Ask, block until the turn finishes, print the answer, clean up
csd run --cwd /tmp/work "Summarize what this repo does in one paragraph."

# Pin the id to make the run resumable, then continue the conversation later
id=$(uuidgen)
csd run --cwd /tmp/work --session-id "$id" "Read main.rs and list its public functions."
csd run --cwd /tmp/work --resume "$id"     "Now write doc comments for the first one."

# Prompts can come from stdin, and --json gives the structured result instead of plain text
cat prompt.md | csd run --cwd /tmp/work --json
```

Or drive the session manually with the async primitives:

```bash
# 1. Spawn a detached session in a directory (auto-clear the first-run folder-trust gate)
name=$(csd spawn --cwd /tmp/work --trust | jq -r .name)

# 2. Send a prompt — csd verifies the keystrokes landed, then submits
csd send "$name" "Plan a small change to README, then stop for approval."

# 3. Poll state until the plan-approval gate is up
csd state "$name"     # {"status":"plan_ready","plan_file":"…","plan":"…"}

# 4. Approve it (option 1 = "Yes, and use auto mode")
csd approve "$name"

# 5. List everything you're driving, then clean up
csd ps
csd kill "$name"
```

## Command reference

Every structured command prints JSON to **stdout**; errors print `{"error": "..."}` to **stderr**
and exit non-zero. So `stdout` is always safe to pipe into `jq`. (`csd run` is the one deliberate
exception: its default success output is the plain answer text — pass `--json` for the structured
form.)

### `csd spawn`

Start a detached interactive agent and track it.

| Flag | Default | Meaning |
| --- | --- | --- |
| `--cwd <DIR>` | current dir | Working directory for the agent. |
| `--session-id <UUID>` | random v4 | Pin the session id (makes the transcript path deterministic). |
| `--name <NAME>` | `csd-<dir>-<short>` | tmux session name. Must be `[A-Za-z0-9._-]`, no `..`. |
| `--backend <NAME>` | `claude` | Driving backend. |
| `--trust` | off | Auto-clear the one-time *"trust this folder?"* startup gate. |
| `--permission-mode <MODE>` | — | `plan` · `acceptEdits` · `auto` · `bypassPermissions` · `default` · `dontAsk`. |
| `--auto-accept` | off | Shortcut for `--permission-mode acceptEdits`. |
| `--bypass-permissions` | off | Shortcut for `--permission-mode bypassPermissions` (skip all permission checks). |
| `--yolo` | off | `--dangerously-skip-permissions` **and** auto-trust — zero friction. |

The four permission-posture flags (`--permission-mode`, `--auto-accept`, `--bypass-permissions`,
`--yolo`) are **mutually exclusive**.

```bash
csd spawn --cwd /tmp/work --trust --name myjob
# {
#   "name": "myjob",
#   "session_id": "0a1b2c3d-…",
#   "cwd": "/tmp/work",
#   "backend": "claude",
#   "jsonl_path": "/home/you/.claude/projects/-tmp-work/0a1b2c3d-….jsonl",
#   "permission_mode": null,
#   "backend_version": "2.1.173",
#   "created": 1780127841
# }
```

A `marker_warning` field is appended when `backend_version` is not marker-verified — see
[Marker version guard](#marker-version-guard).

### `csd run [flags] <prompt…>`

Run one prompt synchronously: spawn (or reuse/resume) a session, inject the prompt, block until
the turn reaches a terminal state, report it. On success the answer text is printed to **stdout**
(`claude -p` parity); every non-`done` outcome is JSON on **stderr** with a distinct exit code, so
pipes can never mistake a question or a gate for an answer. `--json` changes that routing: every
outcome — `done` or not — becomes JSON on **stdout** (one machine-readable stream; the exit code
still tells you which it was). The prompt is read from stdin when no positional prompt is given.

| Flag | Default | Meaning |
| --- | --- | --- |
| `--cwd <DIR>` | current dir | Working directory for a freshly spawned agent. |
| `--session-id <UUID>` | random v4 | Pin the new session's id — makes the run resumable later. |
| `--resume <UUID>` | — | Resume a previous run's transcript (`claude --resume`). Requires the original `--cwd`; fails fast if the transcript isn't there. |
| `--session <NAME>` | — | Reuse a **live** csd session (kept with `--keep`, or reported by a non-`done` outcome). |
| `--permission-mode` / `--auto-accept` / `--bypass-permissions` / `--yolo` | — | Same postures as `spawn`. |
| `--keep` | off | Keep the session alive after a successful run for fast `--session` follow-ups. |
| `--timeout <SECS>` | `600` | Max wait for the turn to finish (`0` = no limit). |
| `--approve-plan` | off | Auto-approve **plan** gates (option 1). Permission gates are *never* auto-approved — use a permission posture instead. |
| `--json` | off | Print every outcome (including `done`) as JSON on stdout. |

`run` always auto-clears the folder-trust gate, and `--resume` / `--session` / `--session-id` are
mutually exclusive.

**Exit codes**

| Code | Outcome | Session afterwards |
| --- | --- | --- |
| `0` | `done` — answer on stdout | Killed (kept with `--keep`). |
| `1` | error (`{"error":…}` on stderr) | Killed if `run` spawned it. |
| `2` | `timed_out` — turn still in flight | **Kept** — inspect with `csd state`, reclaim with `csd kill`. |
| `3` | `awaiting_answer` — assistant asked a question | **Kept** — answer with `csd run --session <name> "…"`. |
| `4` | `gated` — plan/permission/trust gate on screen | **Kept** — inspect with `csd state`, answer with `csd approve`. |

Non-`done` outcomes always include the `session` name, so the conversation stays answerable:

```bash
out=$(csd run --cwd /tmp/work --json "Refactor the config loader.")
case $? in
    0) jq -r .text <<<"$out" ;;
    3) csd run --session "$(jq -r .session <<<"$out")" "Yes, keep the env-var fallback." ;;
esac
```

### `csd send <session> <prompt…>`

Inject a prompt. Keystrokes sent before the TUI is ready are silently dropped, so `csd` types the
text, **verifies it echoed** in the pane, retries if not, then presses Enter.

| Flag | Default | Meaning |
| --- | --- | --- |
| `--no-submit` | off | Type the prompt but don't press Enter. |
| `--retries <N>` | `5` | Max send → verify-echo attempts. |

```bash
csd send myjob "List the files here."     # {"ok": true, "attempts": 1}
```

### `csd state <session>`

Report the hybrid-detector verdict (always JSON). See [State model](#state-model).

```bash
csd state myjob     # {"status":"awaiting_answer","question":"Which directory did you mean?"}
```

### `csd approve <session> [--option N]`

Answer a plan / permission / trust gate by selecting a numbered menu option (default `1`).

```bash
csd approve myjob            # selects option 1
csd approve myjob --option 3 # selects "No"
```

### `csd ps [--json]`

List tracked sessions with liveness and live state. Human table by default; `--json` for machines.

```bash
csd ps --json
# {"sessions":[{"name":"myjob","session_id":"…","cwd":"/tmp/work","backend":"claude",
#   "jsonl_path":"…","alive":true,"state":{"status":"working","tools":["Bash"]}}]}
```

### `csd kill <session>`

Stop the session and remove its sidecar. Idempotent — a session that's already gone still gets its
sidecar cleaned up.

```bash
csd kill myjob     # {"ok": true}
```

## State model

`csd state` (and the `state` field of `csd ps`) returns one tagged object:

| `status` | Extra fields | Meaning |
| --- | --- | --- |
| `spawning` | — | Session is up but hasn't produced a turn yet. |
| `working` | `tools: []` | Assistant is streaming or a tool call is in flight. |
| `awaiting_answer` | `question` | Assistant asked a clarifying question and is waiting. |
| `plan_ready` | `plan_file`, `plan` | A plan is ready and the approval gate is on screen. |
| `blocked` | `gate` (`trust`\|`permission`), `prompt`, `options` | A TUI gate needs an answer — use `csd approve`. |
| `idle_done` | `text` | Assistant finished its turn cleanly; waiting for you. |
| `dead` | — | The tmux session is gone. |
| `unknown` | — | Up but produced output csd doesn't recognize. |

## Marker version guard

Gate detection depends on capture-pane markers matching claude's TUI wording, which can change in
any release. To keep that drift from silently stalling unattended sessions, csd probes
`claude --version` once at spawn, records it in the sidecar (`backend_version`), and attaches a
`marker_warning` field to `spawn` / `state` / `ps` JSON whenever the session's version is not in
the marker-verified set:

```json
{
  "status": "working",
  "tools": [],
  "marker_warning": "claude 2.1.199 is not marker-verified (verified: [\"2.1.158\", \"2.1.173\"]); …"
}
```

The field is **absent** when the version is verified — consumers can alert on its presence. The
warning is advisory: detection still runs, it just may miss gates. After re-verifying the gates on
a new release (run `./scripts/e2e.sh`), either extend `verified_versions` in `src/backend.rs` or
silence it locally without a csd release:

```bash
export CSD_VERIFIED_VERSIONS="2.1.199"   # comma-separated
```

## Permission postures (unattended runs)

| You want… | Use | Effect |
| --- | --- | --- |
| Approve each tool yourself | *(default)* + poll for `blocked` | csd surfaces gates; you `csd approve`. |
| Auto-approve file edits only | `--auto-accept` | Edits run; other tools still gate. |
| Skip **all** permission checks | `--bypass-permissions` | No permission gates (add `--trust` for a new dir). |
| Maximum zero-friction | `--yolo` | Skips permission checks **and** the folder-trust gate. |

> Note: recent `claude` auto-approves read-only Bash (`ls`, `whoami`) even in `default` mode; only
> mutating/uncategorized commands raise a gate.

## Consuming from a script

The whole point is shell-out + JSON. A minimal "ask a question, wait, answer" loop:

```bash
name=$(csd spawn --cwd "$PWD" --trust | jq -r .name)
csd send "$name" "Refactor foo(), but ask me before touching the public API."

while :; do
  s=$(csd state "$name")
  case $(jq -r .status <<<"$s") in
    awaiting_answer) jq -r .question <<<"$s"; read -r reply; csd send "$name" "$reply" ;;
    plan_ready|blocked) csd approve "$name" ;;
    idle_done|dead) break ;;
    *) sleep 3 ;;
  esac
done
csd kill "$name"
```

## How it works

A **hybrid detector** combines three signals — no single one is sufficient:

- **session JSONL** (`~/.claude/projects/<slug>/<id>.jsonl`) — reliable for questions and normal
  turn completion, but lags the live TUI by ~one tool-call.
- **capture-pane** — the only source for TUI-interrupt gates (plan / permission / trust) that never
  reach the transcript until answered. Markers are release-dependent and live in one place
  (`src/backend.rs`).
- **filesystem** — the plan markdown under `~/.claude/plans/`, correlated to the session by spawn
  time (the dir is global).

Input injection uses `tmux send-keys -l` with a send → verify-echo → retry loop (a fixed delay is
not reliable). Each session is tracked by a small JSON sidecar under `~/.local/state/csd/sessions/`.

## Development

```bash
cargo test                                  # unit tests (transcript classifier, pane/send/posture)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
./scripts/e2e.sh                            # live end-to-end (needs the subscription seat + network)
```

> Verified against `claude` v2.1.158 and v2.1.173. capture-pane gate markers and the plan-file
> naming shift between releases — on upgrade, run `./scripts/e2e.sh` and extend
> `verified_versions` in `src/backend.rs` (the marker version guard warns until you do).

## License

[MIT](./LICENSE) © Marshall Ku

# CSD (Claude Session Driver)

Drive an interactive `claude` REPL in a detached `tmux` session on the flat-rate **subscription
seat**, inject input reliably, and detect session state — emitting JSON for programmatic consumers
(copad, life-assistant).

**Why:** from 2026-06-15 `claude -p` / Agent SDK becomes metered, so the only flat-rate substrate
for heavy autonomous agent work is the interactive REPL driven over `tmux`. `csd` is the standalone
tool that does that driving. See `docs/poc-findings.md` for the empirical basis and
`~/dev/copad/docs/decisions.md` #49 for the strategic context.

## Install

```bash
cargo build --release   # → target/release/csd
```

Requires `tmux` and a logged-in `claude` on the subscription seat.

## Commands

All structured commands emit JSON on stdout (errors go to stderr as `{"error": ...}`).

| Command | What it does |
| --- | --- |
| `csd spawn [--cwd DIR] [--session-id UUID] [--permission-mode MODE] [--name NAME] [--trust] [--auto-accept\|--bypass-permissions\|--yolo]` | Start a detached session. `--trust` auto-clears the one-time folder-trust gate. Permission posture (mutually exclusive): `--auto-accept` ⇒ `acceptEdits`; `--bypass-permissions` ⇒ `bypassPermissions` (skip all checks); `--yolo` ⇒ `--dangerously-skip-permissions` + auto-trust (zero friction). |
| `csd send <session> <prompt…> [--no-submit] [--retries N]` | Inject a prompt (send → verify echo → retry → Enter). |
| `csd state <session>` | Hybrid-detector verdict (see below). |
| `csd approve <session> [--option N]` | Answer a plan / permission / trust gate (default option `1`). |
| `csd ps [--json]` | List tracked sessions with liveness + live state. |
| `csd kill <session>` | Stop a session and remove its sidecar (idempotent). |

### State values (`status`)

`spawning` · `working` · `awaiting_answer` (with `question`) · `plan_ready` (with `plan_file` /
`plan`) · `blocked` (with `gate` = `trust`|`permission`, `prompt`, `options`) · `idle_done` (with
`text`) · `dead` · `unknown`.

## Example

```bash
name=$(csd spawn --cwd /tmp/work --trust | jq -r .name)
csd send "$name" "Plan a small change, then stop for approval."
# poll until plan_ready
csd state "$name"            # → {"status":"plan_ready","plan_file":"…","plan":"…"}
csd approve "$name"          # selects "Yes, and use auto mode"
csd kill "$name"
```

## How it works

A hybrid detector combines three signals (no single one suffices):

- **session JSONL** (`~/.claude/projects/<slug>/<id>.jsonl`) — questions, normal turns; lags TUI
  gates by ~one tool-call.
- **capture-pane** — plan / permission / trust gates (TUI interrupts that never reach the JSONL
  until answered). Markers are release-dependent and live in one place (`src/backend.rs`).
- **filesystem** — the plan markdown under `~/.claude/plans/`, correlated by spawn time.

Backend-agnostic by design (`claude` today, `codex` later) — the dispatch backend is a swappable
trait so a forced move off the interactive REPL is a config flip, not a rewrite.

## Develop

```bash
cargo test                          # unit tests (transcript classifier, pane/send helpers)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
./scripts/e2e.sh                    # live end-to-end (needs the subscription seat + network)
```

> Verified against `claude` v2.1.158. capture-pane gate markers and the plan-file naming shift
> between releases — re-verify `src/backend.rs` and `src/detect/plan.rs` on upgrade.

#!/usr/bin/env bash
#
# Live end-to-end check for csd — drives a real interactive `claude` over tmux through the two PoC
# interactions and asserts the JSON output of `csd state`/`spawn`/`approve` at each step.
#
# Requires: a logged-in `claude` on the subscription seat, `tmux`, `jq`, and network. NOT run in CI
# (needs the seat). Run manually:  ./scripts/e2e.sh
#
# This is the asserted form of docs/poc-findings.md §6.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CSD="${CSD_BIN:-$ROOT/target/debug/csd}"
CWD="${CSD_E2E_CWD:-/tmp/csd-e2e}"
POLL_TIMEOUT="${CSD_E2E_TIMEOUT:-90}"

SPAWNED=()
cleanup() {
    for name in "${SPAWNED[@]:-}"; do
        [ -n "$name" ] && "$CSD" kill "$name" >/dev/null 2>&1 || true
    done
}
trap cleanup EXIT

log() { printf '\033[36m[e2e]\033[0m %s\n' "$*"; }
fail() { printf '\033[31m[e2e] FAIL:\033[0m %s\n' "$*" >&2; exit 1; }

command -v jq >/dev/null || fail "jq is required"
command -v tmux >/dev/null || fail "tmux is required"
[ -x "$CSD" ] || { log "building csd…"; cargo build --manifest-path "$ROOT/Cargo.toml"; }

mkdir -p "$CWD"

# Poll `csd state <name>` until its .status equals $1, or timeout. Echoes the final JSON.
wait_for_status() {
    local name="$1" want="$2" deadline=$((SECONDS + POLL_TIMEOUT)) json status
    while [ "$SECONDS" -lt "$deadline" ]; do
        json="$("$CSD" state "$name")"
        status="$(jq -r '.status' <<<"$json")"
        if [ "$status" = "$want" ]; then
            echo "$json"
            return 0
        fi
        sleep 2
    done
    fail "timed out waiting for status=$want on $name (last: ${status:-none})"
}

spawn() { # args passed through to `csd spawn`; echoes the new session name
    # NOTE: called via command substitution (a subshell), so this MUST NOT touch SPAWNED — array
    # appends here would not reach the parent. The caller records the name for cleanup.
    local json name
    json="$("$CSD" spawn --cwd "$CWD" --trust "$@")"
    name="$(jq -r '.name' <<<"$json")"
    [ -n "$name" ] && [ "$name" != "null" ] || fail "spawn returned no name: $json"
    echo "$name"
}

# ── Test 1: clarifying-question detection ───────────────────────────────────────────────────────
log "test 1: clarifying question"
name1="$(spawn)"; SPAWNED+=("$name1")
"$CSD" send "$name1" "Ask me one clarifying question before doing anything, then wait." >/dev/null
state="$(wait_for_status "$name1" awaiting_answer)"
question="$(jq -r '.question' <<<"$state")"
[ -n "$question" ] && [ "$question" != "null" ] || fail "awaiting_answer had no question"
log "  ✓ awaiting_answer: \"$question\""
"$CSD" kill "$name1" >/dev/null

# ── Test 2: plan-ready → approve → work proceeds ────────────────────────────────────────────────
log "test 2: plan mode → approve"
name2="$(spawn --permission-mode plan)"; SPAWNED+=("$name2")
"$CSD" send "$name2" "Make a trivial plan to create a file hello.txt containing 'hi', then stop for approval." >/dev/null
state="$(wait_for_status "$name2" plan_ready)"
log "  ✓ plan_ready (plan_file: $(jq -r '.plan_file' <<<"$state"))"
"$CSD" approve "$name2" >/dev/null
# After approval the gate clears; state should leave plan_ready (working or idle_done).
deadline=$((SECONDS + POLL_TIMEOUT))
while [ "$SECONDS" -lt "$deadline" ]; do
    s="$("$CSD" state "$name2" | jq -r '.status')"
    [ "$s" != "plan_ready" ] && break
    sleep 2
done
[ "$s" != "plan_ready" ] || fail "still plan_ready after approve"
log "  ✓ post-approve status: $s"
"$CSD" kill "$name2" >/dev/null

# ── Test 3: tool-permission gate → approve (live-verifies the permission markers) ───────────────
log "test 3: permission gate → approve"
name3="$(spawn)"; SPAWNED+=("$name3")
# Needs a command claude neither allowlists nor auto-approves as safe — network access qualifies.
"$CSD" send "$name3" "Run this exact bash command and nothing else: curl -s http://example.com" >/dev/null
state="$(wait_for_status "$name3" blocked)"
gate="$(jq -r '.gate' <<<"$state")"
[ "$gate" = "permission" ] || fail "expected permission gate, got: $gate"
log "  ✓ blocked on permission gate ($(jq -r '.prompt' <<<"$state"))"
"$CSD" approve "$name3" >/dev/null
deadline=$((SECONDS + POLL_TIMEOUT))
while [ "$SECONDS" -lt "$deadline" ]; do
    s="$("$CSD" state "$name3" | jq -r '.status')"
    [ "$s" != "blocked" ] && break
    sleep 2
done
[ "$s" != "blocked" ] || fail "still blocked after approve"
log "  ✓ post-approve status: $s"
"$CSD" kill "$name3" >/dev/null

# ── Test 4: marker version guard ────────────────────────────────────────────────────────────────
# A PATH shim reports a bogus version to the probe but execs the real claude for everything else,
# so the warning branch is deterministic regardless of what's in the built-in verified list.
log "test 4: marker version guard"
SHIM_DIR="$(mktemp -d /tmp/csd-e2e-shim.XXXXXX)"
REAL_CLAUDE="$(command -v claude)" || fail "claude is required"
cat > "$SHIM_DIR/claude" <<EOF
#!/bin/sh
if [ "\$1" = "--version" ]; then echo "0.0.1 (Claude Code)"; exit 0; fi
exec "$REAL_CLAUDE" "\$@"
EOF
chmod +x "$SHIM_DIR/claude"

json="$(PATH="$SHIM_DIR:$PATH" "$CSD" spawn --cwd "$CWD" --trust)"
name4="$(jq -r '.name' <<<"$json")"; SPAWNED+=("$name4")
[ "$(jq -r '.backend_version' <<<"$json")" = "0.0.1" ] || fail "spawn did not record backend_version: $json"
jq -e '.marker_warning' <<<"$json" >/dev/null || fail "spawn missing marker_warning for unverified version: $json"
log "  ✓ spawn warns on unverified version"

"$CSD" state "$name4" | jq -e '.marker_warning' >/dev/null || fail "state missing marker_warning"
CSD_VERIFIED_VERSIONS="0.0.1" "$CSD" state "$name4" | jq -e '.marker_warning' >/dev/null \
    && fail "CSD_VERIFIED_VERSIONS did not silence the warning"
log "  ✓ state warns; CSD_VERIFIED_VERSIONS silences"

# The real, unshimmed binary on a marker-verified release must produce no warning.
json="$("$CSD" spawn --cwd "$CWD" --trust --name "csd-e2e-verified")"
SPAWNED+=("csd-e2e-verified")
if jq -e '.marker_warning' <<<"$json" >/dev/null; then
    log "  ⚠ installed claude $("$REAL_CLAUDE" --version) is not in the verified list —"
    log "    re-verify the gate markers (tests 1-3 above) and extend verified_versions"
else
    log "  ✓ no warning on the installed (verified) claude"
fi
"$CSD" kill "$name4" >/dev/null
"$CSD" kill "csd-e2e-verified" >/dev/null
rm -rf "$SHIM_DIR"

# ── Test 5: folder-trust gate (live-verifies the trust markers) ─────────────────────────────────
# Needs a directory claude has never seen, spawned WITHOUT --trust so the gate stays on screen.
log "test 5: trust gate"
FRESH_DIR="$(mktemp -d /tmp/csd-e2e-trust.XXXXXX)"
json="$("$CSD" spawn --cwd "$FRESH_DIR" --name csd-e2e-trust)"
SPAWNED+=("csd-e2e-trust")
state="$(wait_for_status csd-e2e-trust blocked)"
gate="$(jq -r '.gate' <<<"$state")"
[ "$gate" = "trust" ] || fail "expected trust gate, got: $gate"
log "  ✓ blocked on trust gate"
"$CSD" kill csd-e2e-trust >/dev/null
rm -rf "$FRESH_DIR"

# ── ps sees nothing leaked ──────────────────────────────────────────────────────────────────────
log "ps after cleanup:"
"$CSD" ps

log "all e2e checks passed ✓"

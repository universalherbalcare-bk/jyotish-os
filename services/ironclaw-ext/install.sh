#!/usr/bin/env bash
# Jyotish-OS -> IronClaw installer (idempotent, fail-closed).
#
#   ./install.sh            # install everything the binary allows; report what it could not
#   ./install.sh --dry-run  # print the plan only
#   ./install.sh --remove   # remove the extension package + persona (keeps IronClaw config)
#
# What it does, in order (each step is skipped if already done):
#   1. verifies `ironclaw` (>= 1.4.0) and Ollama are reachable
#   2. onboards non-interactively (local-dev profile, no OS service) if not yet onboarded
#   3. sets provider ollama / model qwen3.6:latest if not already set
#   4. installs the persona into <reborn_home>/<profile>/system/prompts/default-system.md
#      (backs up the seeded file once as default-system.md.orig)
#   5. copies the `jyotish/` extension package into
#      <reborn_home>/<profile>/system/extensions/jyotish/ (IronClaw's filesystem catalog dir)
#   6. runs `ironclaw extension install jyotish`; if that fails because the contract
#      endpoint is http://127.0.0.1:7791/mcp, it says exactly why and exits 2
#      (IronClaw 1.4.0 accepts only https hosted-MCP endpoints and denies loopback egress;
#      see README.md "Blocker"). Nothing is faked.
#   7. probes http://127.0.0.1:7791/health so you know whether jyotish-mcp is up
#
# It never writes credentials, never touches the OS keychain beyond what `ironclaw onboard`
# itself does, and never starts the OS service.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODE="${1:-install}"
PROVIDER="ollama"
MODEL="${JYOTISH_OLLAMA_MODEL:-qwen3.6:latest}"
MCP_HEALTH="http://127.0.0.1:7791/health"

log()  { printf '[install] %s\n' "$*"; }
fail() { printf '[install] ERROR: %s\n' "$*" >&2; exit 1; }
run()  { if [[ "$MODE" == "--dry-run" ]]; then printf '[dry-run] %s\n' "$*"; else "$@"; fi; }

command -v ironclaw >/dev/null 2>&1 || fail "ironclaw not on PATH. Install: brew install ironclaw"
IC_VERSION="$(ironclaw --version | awk '{print $2}')"
log "ironclaw $IC_VERSION"

# Resolve home/profile from the binary itself, never from assumptions.
PATHS="$(ironclaw config path)"
REBORN_HOME="$(printf '%s\n' "$PATHS" | awk -F': ' '/^reborn_home:/{print $2}')"
PROFILE="$(printf '%s\n' "$PATHS" | awk -F': ' '/^profile:/{print $2}')"
[[ -n "$REBORN_HOME" && -n "$PROFILE" ]] || fail "could not resolve reborn_home/profile from 'ironclaw config path'"
STORAGE_ROOT="$REBORN_HOME/$PROFILE"
EXT_DIR="$STORAGE_ROOT/system/extensions/jyotish"
PROMPT_FILE="$STORAGE_ROOT/system/prompts/default-system.md"
log "reborn_home=$REBORN_HOME profile=$PROFILE"

if [[ "$MODE" == "--remove" ]]; then
  if ironclaw extension search jyotish 2>/dev/null | grep -q '^- jyotish:'; then
    run ironclaw extension remove jyotish || true
  fi
  run rm -rf "$EXT_DIR"
  if [[ -f "$PROMPT_FILE.orig" ]]; then run cp "$PROMPT_FILE.orig" "$PROMPT_FILE"; fi
  log "removed extension package and restored the original system prompt"
  exit 0
fi

# 1. Ollama reachable with the model present
if ! curl -fsS --max-time 5 http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then
  fail "Ollama is not reachable on 127.0.0.1:11434 (start it: 'ollama serve')"
fi
if ! curl -fsS --max-time 5 http://127.0.0.1:11434/api/tags | grep -q "\"name\":\"$MODEL\""; then
  fail "model $MODEL is not pulled in Ollama (ollama pull $MODEL)"
fi
log "ollama ok, model $MODEL present"

# 2. onboard (non-interactive, no OS service)
if [[ ! -f "$REBORN_HOME/.onboard-completed.json" ]]; then
  run ironclaw onboard --no-service < /dev/null
else
  log "already onboarded"
fi

# 3. provider/model
CUR_PROVIDER="$(ironclaw config get llm.default.provider_id 2>/dev/null | awk '{print $NF}' || true)"
CUR_MODEL="$(ironclaw config get llm.default.model 2>/dev/null | awk '{print $NF}' || true)"
if [[ "$CUR_PROVIDER" != "$PROVIDER" || "$CUR_MODEL" != "$MODEL" ]]; then
  run ironclaw models set-provider "$PROVIDER" --model "$MODEL"
else
  log "provider already $PROVIDER / $MODEL"
fi

# 4. persona -> seeded system prompt file (binary-verified path; see README)
run mkdir -p "$(dirname "$PROMPT_FILE")"
if [[ -f "$PROMPT_FILE" && ! -f "$PROMPT_FILE.orig" ]]; then run cp "$PROMPT_FILE" "$PROMPT_FILE.orig"; fi
if ! cmp -s "$HERE/identity/default-system.md" "$PROMPT_FILE" 2>/dev/null; then
  run cp "$HERE/identity/default-system.md" "$PROMPT_FILE"
  if [[ "$MODE" == "--dry-run" ]]; then log "would install persona at $PROMPT_FILE"; else log "persona installed at $PROMPT_FILE"; fi
else
  log "persona already current"
fi

# 5. extension package -> filesystem catalog dir
run mkdir -p "$(dirname "$EXT_DIR")"
run rm -rf "$EXT_DIR"
run cp -R "$HERE/jyotish" "$EXT_DIR"
log "package copied to $EXT_DIR"

# 6. install (= install + activate in 1.4.0; there is no separate 'activate' subcommand)
if [[ "$MODE" == "--dry-run" ]]; then
  printf '[dry-run] ironclaw extension install jyotish --json\n'
else
  set +e
  OUT="$(ironclaw extension install jyotish --json 2>&1)"
  RC=$?
  set -e
  printf '%s\n' "$OUT"
  if [[ $RC -ne 0 ]]; then
    if printf '%s' "$OUT" | grep -q 'must use the https scheme'; then
      cat >&2 <<'EOF'
[install] BLOCKED by IronClaw, not by this package:
  IronClaw 1.4.0 types a hosted-MCP `server` as an https-only endpoint and its hosted-MCP
  egress policy denies private/loopback IPs, so the contract endpoint
  http://127.0.0.1:7791/mcp cannot be registered or called. The manifest is otherwise
  valid (an https:// copy installs and activates with all 10 tools visible; see
  logs/ext-install-https-variant.txt). Options are listed in README.md "Blocker".
EOF
      exit 2
    fi
    fail "extension install failed (rc=$RC); see output above"
  fi
fi

# 7. is jyotish-mcp up?
if curl -fsS --max-time 3 "$MCP_HEALTH" >/dev/null 2>&1; then
  log "jyotish-mcp health OK at $MCP_HEALTH"
else
  log "jyotish-mcp is NOT answering at $MCP_HEALTH (Phase 1-4 service not running yet); the extension cannot serve tools until it is"
fi
log "done"

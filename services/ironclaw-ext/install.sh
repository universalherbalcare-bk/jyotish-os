#!/usr/bin/env bash
# Jyotish-OS -> IronClaw installer (idempotent, fail-closed).
#
#   ./install.sh            # install everything the binary allows; report what it could not
#   ./install.sh --dry-run  # print the plan only
#   ./install.sh --remove   # remove the extension package + persona (keeps IronClaw config)
#   ./install.sh --register # Phase 7: register jyotish-mcp as a hosted MCP server through
#                           # IronClaw's own builtin.extension_register_hosted_mcp (needs the
#                           # patched ~/.local/bin/ironclaw-jyotish; drives one agent turn with
#                           # tools/stub_ollama.py, no real model, provider restored afterwards)
#
# What it does, in order (each step is skipped if already done):
#   1. verifies `ironclaw` (>= 1.4.0); if the provider it would set is ollama, that Ollama is up
#   2. onboards non-interactively (local-dev profile, no OS service) if not yet onboarded
#   3. sets the LLM provider only when none is configured (default: anthropic / claude-sonnet-5;
#      override with JYOTISH_PROVIDER / JYOTISH_MODEL); never overwrites an existing selection
#   4. installs the persona into <reborn_home>/<profile>/system/prompts/default-system.md
#      (backs up the seeded file once as default-system.md.orig)
#   5. removes a previous install (while IronClaw's registered manifest hash still matches)
#   6. copies the `jyotish/` extension package into
#      <reborn_home>/<profile>/system/extensions/jyotish/ (IronClaw's filesystem catalog dir)
#      and runs `ironclaw extension install jyotish` (endpoint https://127.0.0.1:7791/mcp; on
#      1.4.0 install == install + activate). Nothing is faked.
#   7. probes https://127.0.0.1:7791/health with certs/ironclaw-ca-bundle.pem so you know whether
#      jyotish-mcp is up over TLS, and prints the SSL_CERT_FILE line the IronClaw process needs
#      (README.md "TLS on loopback").
#
# It never writes credentials, never touches the OS keychain beyond what `ironclaw onboard`
# itself does, and never starts the OS service.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
MODE="${1:-install}"
PROVIDER="${JYOTISH_PROVIDER:-anthropic}"
MODEL="${JYOTISH_MODEL:-claude-sonnet-5}"
CA_BUNDLE="$ROOT/certs/ironclaw-ca-bundle.pem"
MCP_HEALTH="https://127.0.0.1:7791/health"

log()  { printf '[install] %s\n' "$*"; }
fail() { printf '[install] ERROR: %s\n' "$*" >&2; exit 1; }
run()  { if [[ "$MODE" == "--dry-run" ]]; then printf '[dry-run] %s\n' "$*"; else "$@"; fi; }

# Phase 7: prefer the locally built, loopback-patched binary when it exists (README.md
# "Phase 7"); the Homebrew binary is untouched and remains the fallback. Every `ironclaw`
# invocation below goes through this function so both binaries share ~/.ironclaw/reborn.
IRONCLAW_BIN="${IRONCLAW_BIN:-$HOME/.local/bin/ironclaw-jyotish}"
if [[ ! -x "$IRONCLAW_BIN" ]]; then
  command -v ironclaw >/dev/null 2>&1 || fail "ironclaw not on PATH. Install: brew install ironclaw"
  IRONCLAW_BIN="$(command -v ironclaw)"
fi
ironclaw() { "$IRONCLAW_BIN" "$@"; }
IC_VERSION="$(ironclaw --version | awk '{print $2}')"
log "ironclaw $IC_VERSION ($IRONCLAW_BIN)"
if [[ "$IRONCLAW_BIN" == "$HOME/.local/bin/ironclaw-jyotish" ]]; then
  # Opt-in loopback egress (patches/ironclaw-1.4.0-allow-loopback-egress.patch). Exported
  # here so the lifecycle commands below and any process this script spawns carry it;
  # the owner's shell still needs the `export` line printed at the end.
  export IRONCLAW_EGRESS_ALLOW_LOOPBACK=1
  log "IRONCLAW_EGRESS_ALLOW_LOOPBACK=1 (patched binary: loopback 127.0.0.0/8 and ::1 only; other private ranges stay denied)"
else
  log "WARNING: unpatched binary; jyotish.* tool calls will be refused with network_denied (README.md 'Phase 7')"
fi

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

# Phase 7 --register: the filesystem package below is ManifestSource::InstalledLocal, which
# IronClaw 1.4.0 never treats as a hosted-MCP provider (its pinned tools carry an empty
# egress allow-list and every call fails with network_denied). The callable extension is the
# one registered through builtin.extension_register_hosted_mcp (ManifestSource::UserRegistered):
# registration -> install -> live tools/list discovery over loopback TLS -> ten
# mcp-jyotish-local.* tools. README.md "Phase 7".
if [[ "$MODE" == "--register" ]]; then
  [[ "$IRONCLAW_BIN" == "$HOME/.local/bin/ironclaw-jyotish" ]] || fail "--register needs the patched binary at ~/.local/bin/ironclaw-jyotish (README.md 'Phase 7')"
  curl -fsS --max-time 3 --cacert "$CA_BUNDLE" "$MCP_HEALTH" >/dev/null 2>&1 || fail "jyotish-mcp is not answering over TLS at $MCP_HEALTH; start it first"
  if ironclaw extension search jyotish-local 2>/dev/null | grep -q 'capability: mcp-jyotish-local.catalog.list'; then
    log "mcp-jyotish-local already registered and installed (10 tools); nothing to do"
    exit 0
  fi
  SCRIPT='[{"tool":"builtin__extension_register_hosted_mcp","arguments":{"desired_id":"jyotish-local","desired_name":"Jyotish-OS (local jyotish-mcp)","endpoint":"https://127.0.0.1:7791/mcp","auth_type":"no_auth"}}]'
  IRONCLAW_BIN="$IRONCLAW_BIN" "$HERE/tools/p7_tool_call.sh" install-register 1 script "$SCRIPT" | grep -v '^\[p7\] capability' || true
  IRONCLAW_BIN="$IRONCLAW_BIN" "$HERE/tools/p7_tool_call.sh" install-activate 1 script '[{"tool":"builtin__extension_install","arguments":{"extension_id":"mcp-jyotish-local"}}]' | grep -v '^\[p7\] capability' || true
  if ironclaw extension search jyotish-local 2>/dev/null | grep -q 'capability: mcp-jyotish-local.catalog.list'; then
    log "mcp-jyotish-local registered, installed and active (tools discovered live from jyotish-mcp)"
  else
    fail "registration/activation did not produce mcp-jyotish-local.* tools; see logs/install-register.turn.txt and logs/install-activate.turn.txt"
  fi
  exit 0
fi

# 1. provider prerequisites (only matter when step 3 will set the provider)
if [[ "$PROVIDER" == "ollama" ]]; then
  if ! curl -fsS --max-time 5 http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then
    fail "Ollama is not reachable on 127.0.0.1:11434 (start it: 'ollama serve')"
  fi
  if ! curl -fsS --max-time 5 http://127.0.0.1:11434/api/tags | grep -q "\"name\":\"$MODEL\""; then
    fail "model $MODEL is not pulled in Ollama (ollama pull $MODEL)"
  fi
  log "ollama ok, model $MODEL present"
fi

# 2. onboard (non-interactive, no OS service)
if [[ ! -f "$REBORN_HOME/.onboard-completed.json" ]]; then
  run ironclaw onboard --no-service < /dev/null
else
  log "already onboarded"
fi

# 3. provider/model — set only if nothing is configured; never overwrite the owner's choice
CUR_PROVIDER="$(ironclaw models status 2>/dev/null | awk -F': ' '/^default.provider:/{print $2}' || true)"
CUR_MODEL="$(ironclaw models status 2>/dev/null | awk -F': ' '/^default.model:/{print $2}' || true)"
if [[ -z "$CUR_PROVIDER" || "$CUR_PROVIDER" == "(not set)" || "$CUR_PROVIDER" == "unconfigured" ]]; then
  run ironclaw models set-provider "$PROVIDER" --model "$MODEL"
else
  log "provider already configured: $CUR_PROVIDER / $CUR_MODEL (left as is; change with 'ironclaw models set-provider')"
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

# 5. remove a previous install FIRST, while the catalog copy still matches what IronClaw
#    registered. (IronClaw pins the manifest hash at install time; overwriting the catalog
#    manifest before `extension remove` wedges every lifecycle command with
#    "installation manifest hash does not match registered manifest hash". Recovery: put the
#    previously installed manifest back in $EXT_DIR, run `ironclaw extension remove jyotish`.)
if [[ "$MODE" != "--dry-run" ]]; then
  set +e
  SEARCH="$(ironclaw extension search jyotish 2>&1)"
  set -e
  if printf '%s' "$SEARCH" | grep -q 'manifest hash does not match'; then
    fail "IronClaw lifecycle state is wedged (catalog manifest differs from the registered one). Restore the previously installed manifest in $EXT_DIR, run 'ironclaw extension remove jyotish', then re-run this script."
  fi
  if printf '%s' "$SEARCH" | grep -q 'capability: jyotish'; then
    ironclaw extension remove jyotish >/dev/null 2>&1 || fail "could not remove the existing jyotish extension"
    log "removed previous install"
  fi
fi

# 6. extension package -> filesystem catalog dir, then install (= install + activate in 1.4.0;
#    there is no separate 'activate' subcommand)
run mkdir -p "$(dirname "$EXT_DIR")"
run rm -rf "$EXT_DIR"
run cp -R "$HERE/jyotish" "$EXT_DIR"
log "package copied to $EXT_DIR"
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
      fail "IronClaw rejected the endpoint scheme; the manifest must say https://127.0.0.1:7791/mcp"
    fi
    fail "extension install failed (rc=$RC); see output above"
  fi
  printf '%s' "$OUT" | grep -q '"phase":"active"' || fail "extension installed but not active; see output above"
fi

# 7. is jyotish-mcp up over TLS?
if [[ ! -s "$CA_BUNDLE" ]]; then
  log "no $CA_BUNDLE yet - run from the repo root: scripts/gen-cert.sh && scripts/make-ca-bundle.sh"
elif curl -fsS --max-time 3 --cacert "$CA_BUNDLE" "$MCP_HEALTH" >/dev/null 2>&1; then
  log "jyotish-mcp health OK over TLS at $MCP_HEALTH (CA bundle: $CA_BUNDLE)"
else
  log "jyotish-mcp is NOT answering over TLS at $MCP_HEALTH; start it from the repo root (services/jyotish-mcp/README.md)"
fi
log "the IronClaw process must trust the local CA:  export SSL_CERT_FILE=$CA_BUNDLE"
log "  (rustls-native-certs REPLACES the platform store with that file, which is why the bundle also"
log "   carries the macOS system roots; for the launchd service put the same variable under"
log "   EnvironmentVariables in ~/Library/LaunchAgents/com.ironclaw.reborn.plist)"
if [[ "$IRONCLAW_BIN" == "$HOME/.local/bin/ironclaw-jyotish" ]]; then
  if ironclaw extension search jyotish-local 2>/dev/null | grep -q 'capability: mcp-jyotish-local.catalog.list'; then
    log "callable extension: mcp-jyotish-local (registered hosted MCP, 10 tools discovered live)"
  else
    log "NEXT STEP: ./install.sh --register   (registers https://127.0.0.1:7791/mcp through IronClaw's"
    log "   builtin.extension_register_hosted_mcp; the 'jyotish' package above is persona/schemas only —"
    log "   its pinned tools are never callable on 1.4.0, README.md 'Phase 7')"
  fi
  log "and must opt in to loopback egress:   export IRONCLAW_EGRESS_ALLOW_LOOPBACK=1"
  log "  run the agent with:  IRONCLAW_EGRESS_ALLOW_LOOPBACK=1 SSL_CERT_FILE=$CA_BUNDLE $IRONCLAW_BIN run|repl|serve"
  log "  rollback: rm $IRONCLAW_BIN (the Homebrew ironclaw 1.4.0 is untouched; without the flag the patched"
  log "  binary behaves exactly like upstream)"
else
  log "KNOWN LIMIT: unpatched IronClaw 1.4.0 refuses loopback egress for hosted-MCP tool calls (network_denied);"
  log "   build the patched binary per README.md 'Phase 7' (patches/ironclaw-1.4.0-allow-loopback-egress.patch)"
fi
log "done"

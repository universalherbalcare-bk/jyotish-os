#!/usr/bin/env bash
# Phase 7 proof runner: drive ONE IronClaw agent turn with the stub LLM so that IronClaw's
# dispatcher issues exactly one extension tool call (or one builtin.http probe), and capture
# every piece of evidence the README cites:
#   <label>.turn.txt        stdout+stderr of `ironclaw run -m ...` (IRONCLAW_REBORN_LOG=debug)
#   <label>.stub.jsonl      the verbatim request/response transcript the stub LLM saw
#   <label>.server.log      lines jyotish-mcp appended to audit/server.log during the turn
#   <label>.audit.jsonl     lines appended to audit/jyotish-mcp.jsonl during the turn
#
# Usage:
#   p7_tool_call.sh <label> <flag 0|1> <tool-name|-> [tool-args-json] [--http-probe URL]
#     <flag>       1 -> export IRONCLAW_EGRESS_ALLOW_LOOPBACK=1 ; 0 -> leave it unset
#     <tool-name>  provider-facing name, e.g. jyotish__catalog__list ; '-' with --http-probe ;
#                  'script' with a JSON list of {"tool","arguments"} steps as the 4th argument
#   IRONCLAW_BIN   binary to run (default: ~/.local/bin/ironclaw-jyotish, falls back to ironclaw)
#
# The script switches IronClaw's default provider to ollama/jyotish-stub for the turn and
# ALWAYS restores the previous provider/model afterwards (trap), even on failure. It never
# touches credentials. Exit code = 0 when the turn ran; the evidence files decide pass/fail.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
LOGS="$HERE/../logs"
LABEL="${1:?label}"; FLAG="${2:?0|1}"; TOOL="${3:?tool-name or -}"; ARGS='{}'; PROBE=""
shift 3
if [[ -n "${1:-}" && "${1:-}" != "--http-probe" ]]; then ARGS="$1"; shift; fi
if [[ "${1:-}" == "--http-probe" ]]; then PROBE="${2:?url}"; fi

BIN="${IRONCLAW_BIN:-$HOME/.local/bin/ironclaw-jyotish}"
[[ -x "$BIN" ]] || BIN="$(command -v ironclaw)"
CA="$ROOT/certs/ironclaw-ca-bundle.pem"
PORT=11435
SERVER_LOG="$ROOT/audit/server.log"
AUDIT="$ROOT/audit/jyotish-mcp.jsonl"

log() { printf '[p7] %s\n' "$*"; }

# remember + restore the owner's provider selection
PREV_PROVIDER="$(ironclaw models status | awk -F': ' '/^default.provider:/{print $2}')"
PREV_MODEL="$(ironclaw models status | awk -F': ' '/^default.model:/{print $2}')"
STUB_PID=""
cleanup() {
  [[ -n "$STUB_PID" ]] && kill "$STUB_PID" 2>/dev/null || true
  # Always restore, including ollama/<real model>: leaving ollama/jyotish-stub configured would make
  # the next `serve` boot point at a model that does not exist.
  if [[ -n "$PREV_PROVIDER" && ( "$PREV_PROVIDER" != "ollama" || "$PREV_MODEL" != "jyotish-stub" ) ]]; then
    ironclaw models set-provider "$PREV_PROVIDER" --model "$PREV_MODEL" >/dev/null 2>&1 || true
  fi
  log "provider restored: $(ironclaw models status | awk -F': ' '/^default.provider:|^default.model:/{printf "%s ", $2}')"
}
trap cleanup EXIT

# Phase 8: when jyotish-mcp runs as the compose container (deploy/compose.yaml) its stderr is the
# docker json-file log, not audit/server.log. P7_DOCKER_SERVICE=jyotish-mcp (auto-detected when that
# container is running) makes the <label>.server.log slice come from `docker compose logs --since`.
DOCKER_SVC="${P7_DOCKER_SERVICE:-}"
if [[ -z "$DOCKER_SVC" ]] && docker compose -f "$ROOT/deploy/compose.yaml" ps -q jyotish-mcp 2>/dev/null | grep -q .; then
  DOCKER_SVC="jyotish-mcp"
fi

# baseline offsets so we only keep lines produced during this turn
S0=$(wc -l < "$SERVER_LOG" 2>/dev/null || echo 0)
A0=$(wc -l < "$AUDIT" 2>/dev/null || echo 0)

STUB_ARGS=(--port "$PORT" --log "$LOGS/$LABEL.stub.jsonl")
if [[ -n "$PROBE" ]]; then STUB_ARGS+=(--http-probe "$PROBE")
elif [[ "$TOOL" == "script" ]]; then STUB_ARGS+=(--script "$ARGS")
else STUB_ARGS+=(--tool-name "$TOOL" --tool-args "$ARGS"); fi
rm -f "$LOGS/$LABEL.stub.jsonl"
python3 "$HERE/stub_ollama.py" "${STUB_ARGS[@]}" 2>"$LOGS/$LABEL.stub.stderr" &
STUB_PID=$!
for _ in $(seq 1 30); do curl -fsS "http://127.0.0.1:$PORT/api/tags" >/dev/null 2>&1 && break; sleep 0.2; done

OLLAMA_BASE_URL="http://127.0.0.1:$PORT" ironclaw models set-provider ollama --model jyotish-stub >/dev/null

if [[ -n "$PROBE" ]]; then MSG="Fetch $PROBE with the http tool and report the status."
elif [[ "$TOOL" == "script" ]]; then MSG="Run the scripted tool steps and report the last result verbatim."
else MSG="Call the $TOOL tool with arguments $ARGS and report the result verbatim."; fi

log "bin=$BIN ($("$BIN" --version)) flag=$FLAG tool=${PROBE:-$TOOL} server-log-source=${DOCKER_SVC:-audit/server.log}"
START_TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
set +e
if [[ "$FLAG" == "1" ]]; then
  IRONCLAW_EGRESS_ALLOW_LOOPBACK=1 SSL_CERT_FILE="$CA" OLLAMA_BASE_URL="http://127.0.0.1:$PORT" \
    IRONCLAW_REBORN_LOG=debug "$BIN" run -m "$MSG" > "$LOGS/$LABEL.turn.txt" 2>&1
else
  env -u IRONCLAW_EGRESS_ALLOW_LOOPBACK SSL_CERT_FILE="$CA" OLLAMA_BASE_URL="http://127.0.0.1:$PORT" \
    IRONCLAW_REBORN_LOG=debug "$BIN" run -m "$MSG" > "$LOGS/$LABEL.turn.txt" 2>&1
fi
RC=$?
set -e
END_TS="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
printf '\n[p7] run rc=%s start=%s end=%s\n' "$RC" "$START_TS" "$END_TS" >> "$LOGS/$LABEL.turn.txt"

if [[ -n "$DOCKER_SVC" ]]; then
  docker compose -f "$ROOT/deploy/compose.yaml" logs --no-log-prefix --since "$START_TS" "$DOCKER_SVC" 2>/dev/null > "$LOGS/$LABEL.server.log" || true
else
  tail -n +"$((S0+1))" "$SERVER_LOG" 2>/dev/null > "$LOGS/$LABEL.server.log" || true
fi
tail -n +"$((A0+1))" "$AUDIT" 2>/dev/null > "$LOGS/$LABEL.audit.jsonl" || true

log "rc=$RC ; reply:"
grep -v '^\[' "$LOGS/$LABEL.turn.txt" | sed 's/\x1b\[[0-9;]*m//g' | grep -v 'DEBUG\|INFO\|WARN\|TRACE' | grep -v '^\s*$' | tail -n 5 | cut -c1-600
log "capability lines:"; sed 's/\x1b\[[0-9;]*m//g' "$LOGS/$LABEL.turn.txt" | grep -o 'capability invocation failed.*\|loopback (127.0.0.0/8, ::1) exempted.*' | sort -u | cut -c1-200 || true
log "server.log new lines: $(wc -l < "$LOGS/$LABEL.server.log")  audit new lines: $(wc -l < "$LOGS/$LABEL.audit.jsonl")"

#!/usr/bin/env bash
# launchd entrypoint for the patched IronClaw service.
# Boots `serve` with the configured default provider (the owner's choice: anthropic / claude-sonnet-5).
# If that boot dies immediately because no API key is stored yet, re-exec with the keyless local
# provider (Ollama) so the service — and its scheduled routines — stay alive instead of crash-looping.
# Nothing here reads, prints, or stores a key. To switch to Claude for real:
#   ironclaw config set anthropic.api_key        # hidden prompt
#   launchctl kickstart -k gui/$(id -u)/com.jyotish-os.ironclaw
set -uo pipefail
BIN="${IRONCLAW_BIN:-$HOME/.local/bin/ironclaw-jyotish}"
LOG="${HOME}/Library/Logs/jyotish-os/ironclaw.boot.log"
export IRONCLAW_EGRESS_ALLOW_LOOPBACK=1
export SSL_CERT_FILE="${SSL_CERT_FILE:-/Users/brijesh/Projects/jyotish-os/certs/ironclaw-ca-bundle.pem}"
log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" | tee -a "$LOG" >&2; }

"$BIN" serve &
PID=$!
# A healthy boot stays up well past this window; a key-less anthropic boot dies in < 2 s.
for _ in $(seq 1 20); do
  sleep 0.5
  kill -0 "$PID" 2>/dev/null || break
done
if kill -0 "$PID" 2>/dev/null; then
  log "serve up (pid $PID) with the configured provider"
  wait "$PID"; exit $?
fi
wait "$PID"; RC=$?
if tail -n 40 "${HOME}/Library/Logs/jyotish-os/ironclaw.stderr.log" 2>/dev/null | grep -q "requires API key env var"; then
  # 1.4.0 has no per-process provider override (LLM_BACKEND is ignored; config.toml wins), so the
  # only way to keep the service alive is to switch the configured provider to the keyless local one.
  # This is logged loudly and is reversed by the two commands in the header once a key is stored.
  PREV=$("$BIN" models status 2>/dev/null | awk '/default.provider:/{print $2}')
  log "configured provider '$PREV' has no stored key (exit $RC) — switching provider to keyless ollama/${OLLAMA_FALLBACK_MODEL:-qwen3.6:latest} so serve + routines stay up. To use Claude: ironclaw config set anthropic.api_key && ironclaw models set-provider anthropic --model claude-sonnet-5 && launchctl kickstart -k gui/\$(id -u)/com.jyotish-os.ironclaw"
  "$BIN" models set-provider ollama --model "${OLLAMA_FALLBACK_MODEL:-qwen3.6:latest}" >/dev/null 2>&1 || log "set-provider fallback failed"
  exec "$BIN" serve
fi
log "serve exited with $RC for a reason other than a missing key — letting launchd restart it"
exit "$RC"

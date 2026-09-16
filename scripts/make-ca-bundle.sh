#!/usr/bin/env bash
# Build certs/ironclaw-ca-bundle.pem = macOS system root CAs + certs/jyotish-ca.crt (Phase 6).
#
# Why: IronClaw's HTTP client is reqwest with `rustls-tls-native-roots`; rustls-native-certs
# honours SSL_CERT_FILE, and that file REPLACES the platform trust store rather than extending
# it. So the bundle must carry the public roots too, or every https call IronClaw makes to the
# outside world (Anthropic, Ollama registry, ...) would fail while jyotish-mcp works.
#
# Read-only export only: `security find-certificate -a -p` prints certificates; nothing is
# added to or removed from any keychain, no sudo, no `security add-trusted-cert`.
#
#   scripts/make-ca-bundle.sh            # (re)build and verify
#   scripts/make-ca-bundle.sh --verify   # verify only (exit 1 on any failure)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CERTS="$ROOT/certs"
CA_CRT="$CERTS/jyotish-ca.crt"
SRV_CRT="$CERTS/jyotish-mcp.crt"
BUNDLE="$CERTS/ironclaw-ca-bundle.pem"
SYS_ROOTS="/System/Library/Keychains/SystemRootCertificates.keychain"
HEALTH_URL="${JYOTISH_HEALTH_URL:-https://127.0.0.1:7791/health}"
PUBLIC_URL="${JYOTISH_PUBLIC_PROBE_URL:-https://www.anthropic.com}"
MODE="${1:-build}"

log() { printf '[ca-bundle] %s\n' "$*"; }
[[ -s "$CA_CRT" ]] || { echo "missing $CA_CRT — run scripts/gen-cert.sh first" >&2; exit 1; }

if [[ "$MODE" != "--verify" ]]; then
  TMP="$(mktemp)"
  if [[ "$(uname -s)" == "Darwin" ]]; then
    [[ -r "$SYS_ROOTS" ]] || { echo "cannot read $SYS_ROOTS" >&2; exit 1; }
    security find-certificate -a -p "$SYS_ROOTS" > "$TMP"
  elif [[ -r /etc/ssl/certs/ca-certificates.crt ]]; then
    cat /etc/ssl/certs/ca-certificates.crt > "$TMP"
  else
    echo "no system root store found (expected $SYS_ROOTS or /etc/ssl/certs/ca-certificates.crt)" >&2; exit 1
  fi
  SYS_N="$(grep -c 'BEGIN CERTIFICATE' "$TMP" || true)"
  [[ "$SYS_N" -gt 50 ]] || { echo "system root export looks wrong ($SYS_N certs)" >&2; rm -f "$TMP"; exit 1; }
  {
    printf '# jyotish-os IronClaw CA bundle — %s system roots + certs/jyotish-ca.crt; built %s by scripts/make-ca-bundle.sh\n' "$SYS_N" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    cat "$TMP"
    printf '\n# --- jyotish-os local CA (certs/jyotish-ca.crt) ---\n'
    cat "$CA_CRT"
  } > "$BUNDLE.tmp"
  rm -f "$TMP"
  mv "$BUNDLE.tmp" "$BUNDLE"
  chmod 644 "$BUNDLE"
  log "wrote $BUNDLE ($SYS_N system roots + 1 local CA, $(grep -c 'BEGIN CERTIFICATE' "$BUNDLE") certificates)"
fi

# --- verification (all three must pass) -----------------------------------------------------
fail=0
log "1/3 openssl verify -CAfile bundle jyotish-mcp.crt"
if openssl verify -CAfile "$BUNDLE" "$SRV_CRT"; then :; else fail=1; fi

log "2/3 curl --cacert bundle $HEALTH_URL"
if out="$(curl -sS --max-time 5 --cacert "$BUNDLE" "$HEALTH_URL")"; then
  printf '%s\n' "$out" | head -c 300; echo
else
  echo "   (jyotish-mcp not answering over TLS at $HEALTH_URL — start it with TLS on, see services/jyotish-mcp/README.md)"; fail=1
fi

log "3/3 curl --cacert bundle $PUBLIC_URL (public roots survived)"
code="$(curl -sS --max-time 15 --cacert "$BUNDLE" -o /dev/null -w '%{http_code}' "$PUBLIC_URL" || echo "000")"
echo "   HTTP $code"
case "$code" in 2*|3*) ;; *) fail=1;; esac

if [[ $fail -ne 0 ]]; then echo "[ca-bundle] VERIFY FAILED" >&2; exit 1; fi
log "OK — export SSL_CERT_FILE=$BUNDLE for the IronClaw process"

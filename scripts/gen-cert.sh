#!/usr/bin/env bash
# Local CA + server certificate for jyotish-mcp's loopback HTTPS listener (Phase 6).
#
#   scripts/gen-cert.sh            # create certs/ if missing; no-op when everything is present and valid
#   scripts/gen-cert.sh --force    # regenerate everything (new CA => re-run make-ca-bundle.sh afterwards)
#
# Produces (all under certs/, which is gitignored):
#   certs/jyotish-ca.key      CA private key, EC P-256, mode 0600
#   certs/jyotish-ca.crt      CA certificate, CN=jyotish-os local CA, 10 years, CA:TRUE, pathlen:0
#   certs/jyotish-mcp.key     server private key, EC P-256, mode 0600 (jyotish-mcp refuses anything else)
#   certs/jyotish-mcp.crt     server certificate, CN=jyotish-mcp, SAN = IP:127.0.0.1, DNS:localhost, 825 days
#
# Only `openssl` is used. Nothing is written to the macOS keychain or any system trust store;
# the CA is trusted per-process via SSL_CERT_FILE (see scripts/make-ca-bundle.sh).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CERTS="$ROOT/certs"
CA_KEY="$CERTS/jyotish-ca.key"
CA_CRT="$CERTS/jyotish-ca.crt"
SRV_KEY="$CERTS/jyotish-mcp.key"
SRV_CRT="$CERTS/jyotish-mcp.crt"
SRV_CSR="$CERTS/jyotish-mcp.csr"
CA_DAYS=3650      # 10 years
SRV_DAYS=825      # Apple / CA-B Forum ceiling for a leaf
FORCE="${1:-}"

command -v openssl >/dev/null 2>&1 || { echo "openssl not found" >&2; exit 1; }
mkdir -p "$CERTS"
chmod 700 "$CERTS"

log() { printf '[gen-cert] %s\n' "$*"; }

# --- CA ------------------------------------------------------------------------------------
if [[ "$FORCE" == "--force" || ! -s "$CA_KEY" || ! -s "$CA_CRT" ]]; then
  log "creating CA ($CA_DAYS days)"
  umask 077
  openssl ecparam -name prime256v1 -genkey -noout -out "$CA_KEY"
  chmod 600 "$CA_KEY"
  openssl req -x509 -new -key "$CA_KEY" -sha256 -days "$CA_DAYS" \
    -subj "/CN=jyotish-os local CA/O=jyotish-os" \
    -addext "basicConstraints=critical,CA:TRUE,pathlen:0" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -addext "subjectKeyIdentifier=hash" \
    -out "$CA_CRT"
  umask 022
  chmod 644 "$CA_CRT"
  # A new CA invalidates the old server cert.
  rm -f "$SRV_KEY" "$SRV_CRT" "$SRV_CSR"
else
  log "CA present: $CA_CRT"
fi

# --- server cert ---------------------------------------------------------------------------
need_server=0
if [[ ! -s "$SRV_KEY" || ! -s "$SRV_CRT" ]]; then
  need_server=1
elif ! openssl verify -CAfile "$CA_CRT" "$SRV_CRT" >/dev/null 2>&1; then
  log "server cert does not verify against the CA; regenerating"
  need_server=1
elif ! openssl x509 -in "$SRV_CRT" -noout -checkend 2592000 >/dev/null 2>&1; then
  log "server cert expires within 30 days; regenerating"
  need_server=1
elif ! openssl x509 -in "$SRV_CRT" -noout -ext subjectAltName 2>/dev/null | grep -q '127\.0\.0\.1'; then
  log "server cert SAN lacks IP:127.0.0.1; regenerating"
  need_server=1
fi

if [[ $need_server -eq 1 ]]; then
  log "creating server certificate ($SRV_DAYS days, SAN = IP:127.0.0.1, DNS:localhost)"
  umask 077
  openssl ecparam -name prime256v1 -genkey -noout -out "$SRV_KEY"
  chmod 600 "$SRV_KEY"
  umask 022
  openssl req -new -key "$SRV_KEY" -sha256 -subj "/CN=jyotish-mcp/O=jyotish-os" -out "$SRV_CSR"
  EXT="$(mktemp)"
  cat > "$EXT" <<'EOF'
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=IP:127.0.0.1,DNS:localhost
subjectKeyIdentifier=hash
authorityKeyIdentifier=keyid,issuer
EOF
  openssl x509 -req -in "$SRV_CSR" -CA "$CA_CRT" -CAkey "$CA_KEY" -CAcreateserial \
    -days "$SRV_DAYS" -sha256 -extfile "$EXT" -out "$SRV_CRT"
  rm -f "$EXT" "$SRV_CSR" "$CERTS/jyotish-ca.srl"
  chmod 644 "$SRV_CRT"
else
  log "server cert present and valid: $SRV_CRT"
fi

# --- invariants the server will re-check at boot -------------------------------------------
# portable mode read: GNU stat (-c) first, BSD/macOS stat (-f) second — `stat -f` on GNU means *filesystem* status.
mode_of() { stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1"; }
[[ "$(mode_of "$SRV_KEY")" == "600" ]] || { echo "server key is not mode 600 (got $(mode_of "$SRV_KEY"))" >&2; exit 1; }
[[ "$(mode_of "$CA_KEY")" == "600" ]] || { echo "CA key is not mode 600 (got $(mode_of "$CA_KEY"))" >&2; exit 1; }
openssl verify -CAfile "$CA_CRT" "$SRV_CRT" >/dev/null
openssl x509 -in "$SRV_CRT" -noout -ext subjectAltName | grep -q '127\.0\.0\.1'

log "CA      : $(openssl x509 -in "$CA_CRT"  -noout -subject | sed 's/^subject=//') — $(openssl x509 -in "$CA_CRT"  -noout -fingerprint -sha256)"
log "server  : $(openssl x509 -in "$SRV_CRT" -noout -subject | sed 's/^subject=//') — $(openssl x509 -in "$SRV_CRT" -noout -fingerprint -sha256)"
log "SAN     : $(openssl x509 -in "$SRV_CRT" -noout -ext subjectAltName | tail -n +2 | tr -d ' ')"
log "expires : $(openssl x509 -in "$SRV_CRT" -noout -enddate | sed 's/^notAfter=//')"
log "next    : scripts/make-ca-bundle.sh  (builds certs/ironclaw-ca-bundle.pem for SSL_CERT_FILE)"

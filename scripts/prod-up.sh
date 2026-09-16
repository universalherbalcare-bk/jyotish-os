#!/usr/bin/env bash
# Build + start the containerized stack (deploy/compose.yaml), replacing the native servers.
#   scripts/prod-up.sh            # build (cached), stop native servers, up -d, wait for 3x healthy, status
#   scripts/prod-up.sh --no-build # skip the build step
# Exit: 0 all three healthy; 1 health timeout (status + logs printed); 3 docker missing.
set -euo pipefail
. "$(dirname "$0")/prod-lib.sh"
BUILD=1
[ "${1:-}" = "--no-build" ] && BUILD=0
need_docker
cd "$ROOT"
for f in kernels/de440s.bsp kernels/de440s.sha256 certs/jyotish-mcp.crt certs/jyotish-mcp.key certs/jyotish-ca.crt; do
  [ -f "$f" ] || { echo "missing $f (kernel: services/jyotish-mcp/README.md; certs: scripts/gen-cert.sh)" >&2; exit 3; }
done
mkdir -p audit
if [ "$BUILD" = 1 ]; then
  echo "== building images"
  compose build
fi
echo "== native servers before:"
list_native || true
stop_native
echo "== native servers after stop:"
list_native || echo "(none)"
echo "== docker compose up -d"
compose up -d --remove-orphans
echo "== waiting for health (jhora-svc boots its worker pool + catalog smoke: up to ~3 min)"
if ! wait_healthy "${PROD_UP_TIMEOUT:-420}"; then
  status_table
  compose logs --tail 40
  exit 1
fi
status_table
echo "== edge /health (https://127.0.0.1:7791, local CA):"
edge_health | python3 -c 'import json,sys; h=json.load(sys.stdin); print(json.dumps({k:h[k] for k in ("ok","deployment","sidecars","golden") if k in h}))'
echo "== host listeners on 7791/7792/7793:"
lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null | grep -E ':(7791|7792|7793) ' || echo "(none)"

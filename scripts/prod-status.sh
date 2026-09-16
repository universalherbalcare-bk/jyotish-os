#!/usr/bin/env bash
# Status table for the containerized stack + edge health + host listeners. Exit 0 iff all three healthy.
set -euo pipefail
. "$(dirname "$0")/prod-lib.sh"
need_docker
status_table
rc=0
for s in "${SERVICES[@]}"; do [ "$(svc_health "$s")" = "healthy" ] || rc=1; done
if [ "$rc" = 0 ]; then
  echo "== edge /health:"
  edge_health | python3 -c 'import json,sys; h=json.load(sys.stdin); print(json.dumps({k:h[k] for k in ("ok","deployment","sidecars") if k in h}))' || rc=1
fi
echo "== host listeners on 7791/7792/7793:"
lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null | grep -E ':(7791|7792|7793) ' || echo "(none)"
echo "== native servers still running (should be none):"
list_native || echo "(none)"
exit "$rc"

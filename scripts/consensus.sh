#!/usr/bin/env bash
# Differential consensus corpus: XALEN-DE440 (jyotish-mcp engine) vs jhora-svc (PyJHora / Swiss).
#   scripts/consensus.sh            # n=500  (the CI gate)
#   scripts/consensus.sh --full     # n=10000 seed 42 (the docs/CONTRACT.md numbers)
#   scripts/consensus.sh --n 2000 --seed 7 --years 1972:2033   # any runner flag is passed through
#   scripts/consensus.sh --compose  # Phase 8: run INSIDE the compose sidecar network against the
#                                   # containerized jhora-svc (docker compose --profile corpus run);
#                                   # no port of jhora-svc is ever published. Output: validation/compose-*
# Native mode starts jhora-svc if it is not already listening on 127.0.0.1:7792 (and stops it again on exit).
# Exit code is the runner's: 0 all categories within tolerance, 1 a category over tolerance,
# 2 a chart could not be computed/fetched, 3 engine/sidecar boot failure, 64 bad arguments.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

N=500
ARGS=()
COMPOSE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --full) N=10000; shift ;;
    --n) N="$2"; shift 2 ;;
    --compose) COMPOSE=1; shift ;;
    *) ARGS+=("$1"); shift ;;
  esac
done

if [ "$COMPOSE" = 1 ]; then
  # In-network run: the consensus-corpus image (services/jyotish-mcp/Dockerfile target `consensus`)
  # joins the internal `sidecars` network and talks to http://jhora-svc:7792 (JYOTISH_DEPLOYMENT=compose
  # is the only mode in which the runner accepts that hostname). jhora-svc must be healthy (scripts/prod-up.sh).
  set +e
  docker compose -f deploy/compose.yaml --profile corpus run --rm consensus-corpus \
    --n "$N" --seed 42 --jhora-url http://jhora-svc:7792 \
    --kernel /kernels/de440s.bsp --sha /kernels/de440s.sha256 \
    --csv /out/compose-consensus-corpus.csv --summary /out/compose-consensus-summary.md ${ARGS[@]+"${ARGS[@]}"}
  RC=$?
  set -e
  echo "consensus-corpus (compose) exit code: $RC" >&2
  exit "$RC"
fi

test -f kernels/de440s.bsp || { echo "kernels/de440s.bsp missing (see services/jyotish-mcp/README.md)" >&2; exit 3; }
test -f kernels/de440s.sha256 || { echo "kernels/de440s.sha256 missing" >&2; exit 3; }

JHORA_URL="${JHORA_URL:-http://127.0.0.1:7792}"
STARTED_PID=""
cleanup() { if [ -n "$STARTED_PID" ]; then kill "$STARTED_PID" 2>/dev/null || true; fi; }
trap cleanup EXIT

if ! curl -sf -m 3 "$JHORA_URL/v1/health" >/dev/null 2>&1; then
  echo "jhora-svc not listening at $JHORA_URL — starting it" >&2
  if [ ! -x services/jhora-svc/.venv/bin/python ]; then
    (cd services/jhora-svc && ./setup_env.sh)
  fi
  (cd services/jhora-svc && ./run.sh > "${JHORA_LOG:-/tmp/jhora-svc-consensus.log}" 2>&1) &
  STARTED_PID=$!
  for _ in $(seq 1 180); do
    sleep 1
    if curl -sf -m 3 "$JHORA_URL/v1/health" >/dev/null 2>&1; then break; fi
    if ! kill -0 "$STARTED_PID" 2>/dev/null; then echo "jhora-svc exited during start-up; see ${JHORA_LOG:-/tmp/jhora-svc-consensus.log}" >&2; exit 3; fi
  done
  curl -sf -m 3 "$JHORA_URL/v1/health" >/dev/null || { echo "jhora-svc did not become healthy" >&2; exit 3; }
fi

(cd validation/consensus && cargo build --release --quiet)

set +e
validation/consensus/target/release/consensus-corpus --n "$N" --jhora-url "$JHORA_URL" ${ARGS[@]+"${ARGS[@]}"}
RC=$?
set -e
echo "consensus-corpus exit code: $RC" >&2
exit "$RC"

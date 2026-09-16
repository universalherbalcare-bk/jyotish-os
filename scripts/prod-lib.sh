#!/usr/bin/env bash
# Shared helpers for scripts/prod-*.sh (sourced, not executed).
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="$ROOT/deploy/compose.yaml"
SERVICES=(jhora-svc vedastro-svc jyotish-mcp)
CA="$ROOT/certs/jyotish-ca.crt"

compose() { docker compose -f "$COMPOSE_FILE" "$@"; }

need_docker() {
  command -v docker >/dev/null || { echo "docker not found" >&2; exit 3; }
  docker info >/dev/null 2>&1 || { echo "docker daemon not reachable (is Docker Desktop running?)" >&2; exit 3; }
}

# The three NATIVE servers are identified two ways at once — the process must (a) own the contract
# port on 127.0.0.1 and (b) match the exact command line of services/*/run.sh — so nothing else is
# ever touched (another jyotish-mcp on a different port, an unrelated python/dotnet, ...).
NATIVE_PORTS=(7791 7792 7793)
NATIVE_PATTERNS=(
  'services/jyotish-mcp/target/release/jyotish-mcp'
  'uvicorn app:app --host 127\.0\.0\.1 --port 7792'
  'VedAstroSvc/bin/Release/net10\.0/VedAstroSvc\.dll'
)

# pid of the process LISTENING on 127.0.0.1:<port> (empty if none)
port_owner() { lsof -nP -iTCP@127.0.0.1:"$1" -sTCP:LISTEN -t 2>/dev/null | head -n1; }

# "pid command" for each native server currently owning a contract port with the expected command line
list_native() {
  local i pid cmd
  for i in 0 1 2; do
    pid="$(port_owner "${NATIVE_PORTS[$i]}")"
    [ -z "$pid" ] && continue
    cmd="$(ps -p "$pid" -o command= 2>/dev/null || true)"
    if printf '%s' "$cmd" | grep -q -E "${NATIVE_PATTERNS[$i]}"; then
      echo "$pid ${NATIVE_PORTS[$i]} $cmd"
    else
      echo "NOTE: 127.0.0.1:${NATIVE_PORTS[$i]} is owned by pid $pid ($cmd) which is not the native server; left alone" >&2
    fi
  done
}

# Stop the native servers (SIGTERM, then SIGKILL after 10 s). Prints exactly what it kills.
stop_native() {
  local line pid pids=() i
  while read -r line; do
    [ -z "$line" ] && continue
    pid="${line%% *}"
    echo "stopping native server: $line"
    kill -TERM "$pid" 2>/dev/null || true
    pids+=("$pid")
  done < <(list_native 2>/dev/null)
  [ "${#pids[@]}" -eq 0 ] && return 0
  for i in $(seq 1 20); do
    local alive=0
    for pid in "${pids[@]}"; do kill -0 "$pid" 2>/dev/null && alive=1; done
    [ "$alive" = 0 ] && break
    sleep 0.5
  done
  for pid in "${pids[@]}"; do
    if kill -0 "$pid" 2>/dev/null; then echo "pid $pid still alive after 10 s, SIGKILL"; kill -KILL "$pid" 2>/dev/null || true; fi
  done
  # the vedastro pid file (services/vedastro-svc/run.sh --daemon) is stale now
  rm -f "$ROOT/services/vedastro-svc/data/vedastro-svc.pid"
}

# health of one compose service as docker sees it: healthy | starting | unhealthy | none | (missing)
svc_health() {
  local id
  id="$(compose ps -q "$1" 2>/dev/null || true)"
  [ -z "$id" ] && { echo "missing"; return; }
  docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}' "$id" 2>/dev/null || echo "missing"
}

wait_healthy() {
  local timeout="${1:-420}" t0 s all
  t0=$(date +%s)
  while :; do
    all=1
    for s in "${SERVICES[@]}"; do
      [ "$(svc_health "$s")" = "healthy" ] || all=0
    done
    [ "$all" = 1 ] && return 0
    if [ $(( $(date +%s) - t0 )) -ge "$timeout" ]; then
      echo "timeout after ${timeout}s waiting for health" >&2
      return 1
    fi
    sleep 3
  done
}

status_table() {
  printf '%-14s %-12s %-10s %-22s %-8s %s\n' SERVICE HEALTH RESTARTS STARTED_AT UID PORTS
  local s id health restarts started uid ports
  for s in "${SERVICES[@]}"; do
    id="$(compose ps -q "$s" 2>/dev/null || true)"
    if [ -z "$id" ]; then
      printf '%-14s %-12s %-10s %-22s %-8s %s\n' "$s" missing - - - -
      continue
    fi
    health="$(svc_health "$s")"
    restarts="$(docker inspect -f '{{.RestartCount}}' "$id")"
    started="$(docker inspect -f '{{.State.StartedAt}}' "$id" | cut -c1-19)"
    uid="$(docker inspect -f '{{.Config.User}}' "$id")"
    ports="$(docker inspect -f '{{range $p, $b := .NetworkSettings.Ports}}{{range $b}}{{.HostIp}}:{{.HostPort}}->{{$p}} {{end}}{{end}}' "$id")"
    printf '%-14s %-12s %-10s %-22s %-8s %s\n' "$s" "$health" "$restarts" "$started" "$uid" "${ports:-none}"
  done
}

edge_health() {
  [ -f "$CA" ] || { echo "certs/jyotish-ca.crt missing (scripts/gen-cert.sh)"; return 1; }
  curl -sf -m 5 --cacert "$CA" https://127.0.0.1:7791/health
}

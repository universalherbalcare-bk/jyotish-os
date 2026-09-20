#!/usr/bin/env bash
# Idempotent: make sure the Docker daemon is up (launch Docker Desktop if not) and the compose stack is running.
# Safe to run every 5 minutes from launchd; exits 0 when healthy, non-zero only if the daemon never came up.
set -uo pipefail
cd "$(dirname "$0")/.."
log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*"; }
# `docker info` can hang (not just fail) while Docker Desktop's VM is half-up, so every probe is capped.
dinfo() { ( docker info >/dev/null 2>&1 & local p=$!; local i=0; while kill -0 $p 2>/dev/null && [ $i -lt 30 ]; do sleep 0.5; i=$((i+1)); done; kill $p 2>/dev/null; wait $p 2>/dev/null ); }
wait_daemon() { local n=$1; local i=0; while [ $i -lt "$n" ]; do dinfo && return 0; sleep 5; i=$((i+5)); done; return 1; }
if ! dinfo; then
  log "docker daemon down — launching Docker Desktop"
  open -a Docker 2>/dev/null || true
  if ! wait_daemon 180; then
    # Observed twice (2026-09-17, 2026-09-20): the VM never comes up after a launch
    # (backend logs "no route to host 192.168.65.x:2376"); a clean app restart recovers it.
    log "docker daemon still down after 180s — clean restart of Docker Desktop"
    osascript -e 'quit app "Docker"' >/dev/null 2>&1 || true
    for _ in $(seq 1 20); do sleep 2; pgrep -f "com.docker.backend" >/dev/null || break; done
    pkill -f "com.docker.backend" 2>/dev/null || true; sleep 3
    open -a Docker 2>/dev/null || true
    wait_daemon 420 || { log "docker daemon still down after clean restart (7 min) — giving up this run"; exit 1; }
  fi
fi
docker compose -f deploy/compose.yaml up -d --no-build --remove-orphans >/dev/null 2>&1 || docker compose -f deploy/compose.yaml up -d --remove-orphans
st=$(docker compose -f deploy/compose.yaml ps --format '{{.Service}}={{.Health}}' | tr '\n' ' ')
log "stack: $st"

#!/usr/bin/env bash
# Idempotent: make sure the Docker daemon is up (launch Docker Desktop if not) and the compose stack is running.
# Safe to run every 5 minutes from launchd; exits 0 when healthy, non-zero only if the daemon never came up.
set -uo pipefail
cd "$(dirname "$0")/.."
log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*"; }
if ! docker info >/dev/null 2>&1; then
  log "docker daemon down — launching Docker Desktop"
  open -a Docker 2>/dev/null || true
  for _ in $(seq 1 60); do sleep 3; docker info >/dev/null 2>&1 && break; done
  docker info >/dev/null 2>&1 || { log "docker daemon still down after 180s"; exit 1; }
fi
docker compose -f deploy/compose.yaml up -d --no-build --remove-orphans >/dev/null 2>&1 || docker compose -f deploy/compose.yaml up -d --remove-orphans
st=$(docker compose -f deploy/compose.yaml ps --format '{{.Service}}={{.Health}}' | tr '\n' ' ')
log "stack: $st"

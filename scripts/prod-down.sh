#!/usr/bin/env bash
# Stop the containerized stack. Containers, images and the vedastro-data volume are kept
# (`scripts/prod-down.sh --purge` also removes containers + the volume; images stay).
set -euo pipefail
. "$(dirname "$0")/prod-lib.sh"
need_docker
case "${1:-}" in
  --purge) compose down --volumes --remove-orphans ;;
  *)       compose stop ;;
esac
status_table

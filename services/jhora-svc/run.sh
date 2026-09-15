#!/usr/bin/env bash
# jhora-svc launcher: loopback only, ONE uvicorn process, PyJHora work in an in-process spawn pool.
# Env: JHORA_SVC_WORKERS (default min(4, cpu)), JHORA_SVC_LOG_LEVEL (default INFO)
set -euo pipefail
cd "$(dirname "$0")"
if [ ! -x .venv/bin/python ]; then
  echo "missing .venv - create it with: uv venv --python 3.12 .venv && see README.md" >&2
  exit 1
fi
exec .venv/bin/python -m uvicorn app:app --host 127.0.0.1 --port 7792 --workers 1 --no-access-log "$@"

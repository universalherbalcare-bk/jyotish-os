#!/usr/bin/env bash
# JYOTISH-OS console on 127.0.0.1:7790 (plain HTTP, loopback only; proxies to the TLS MCP with the CA bundle).
set -euo pipefail
cd "$(dirname "$0")"
[ -x .venv/bin/python ] || { uv venv -q --python 3.12 .venv && VIRTUAL_ENV="$PWD/.venv" uv pip install -q fastapi uvicorn "pydantic>=2"; }
exec .venv/bin/python -m uvicorn app:app --host 127.0.0.1 --port 7790 --no-access-log

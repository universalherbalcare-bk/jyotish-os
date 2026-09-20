#!/usr/bin/env bash
# Creates services/jhora-svc/.venv (Python 3.12 via uv) with PyJHora's pinned requirements, the vendored
# PyJHora source on sys.path (a .pth entry; `uv pip install -e vendor/pyjhora` fails because its
# pyproject uses `license-expression`, which the setuptools resolved in the isolated build rejects),
# and the service/test dependencies. Idempotent. vendor/ is never modified.
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd ../.. && pwd)"
test -d "$ROOT/vendor/pyjhora/src/jhora" || { echo "vendor/pyjhora missing - run scripts/vendor.sh" >&2; exit 1; }
command -v uv >/dev/null || { echo "uv not found (https://docs.astral.sh/uv/)" >&2; exit 1; }
[ -x .venv/bin/python ] || uv venv --python 3.12 .venv
export VIRTUAL_ENV="$PWD/.venv"
uv pip install -q -r "$ROOT/vendor/pyjhora/requirements.txt"
# python-dateutil: imported by vendor/pyjhora/src/jhora/utils.py:44 but absent from PyJHora's own
# requirements.txt (upstream omission, surfaced by hosted CI 2026-09-20). Pinned to the lock's version.
uv pip install -q fastapi uvicorn "pydantic>=2" pytest httpx "python-dateutil==2.9.0.post0"
SP="$(.venv/bin/python -c 'import sysconfig; print(sysconfig.get_paths()["purelib"])')"
echo "$ROOT/vendor/pyjhora/src" > "$SP/pyjhora-vendor.pth"
.venv/bin/python - <<'PY'
import os
from jhora import const
import swisseph as swe
n = sum(1 for f in os.listdir(const._ephe_path) if f.endswith(".se1"))
print(f"jhora importable; ephe_path={const._ephe_path} se1_files={n} pyswisseph={swe.version}")
assert n > 0, "no ephemeris files"
PY
echo "jhora-svc env ready: $PWD/.venv"

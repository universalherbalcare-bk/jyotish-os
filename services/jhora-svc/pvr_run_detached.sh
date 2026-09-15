#!/usr/bin/env bash
# Runs run_pvr_tests.py fully detached (own session via python os.setsid) with stdout+stderr to a
# file; appends EXIT=<code> when done. macOS has no setsid(1), hence the python launcher.
# usage: pvr_run_detached.sh <mean|true> <outfile>
set -u
cd "$(dirname "$0")"
.venv/bin/python - "$1" "$2" <<'PY'
import os, subprocess, sys
nodes, out = sys.argv[1], os.path.abspath(sys.argv[2])
cmd = f".venv/bin/python -u run_pvr_tests.py --nodes {nodes} --baseline compare > '{out}' 2>&1; echo \"EXIT=$?\" >> '{out}'"
p = subprocess.Popen(["bash", "-c", cmd], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, start_new_session=True, cwd=os.getcwd())
print(f"launched nodes={nodes} pid={p.pid} -> {out}")
PY

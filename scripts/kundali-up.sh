#!/usr/bin/env bash
# Start the earlier "Vedic Astrology" product (kundali-webapp, Swiss Ephemeris + 22 engines) on 127.0.0.1:8000.
# It lives in its OWN repo (~/Projects/Vedic Astrology/kundali-webapp) and is never modified from here — only run.
set -uo pipefail
K="${KUNDALI_DIR:-$HOME/Projects/Vedic Astrology/kundali-webapp}"
[ -d "$K/backend" ] || { echo "kundali-webapp not found at $K (set KUNDALI_DIR)"; exit 1; }
if curl -s -m 2 http://127.0.0.1:8000/ >/dev/null; then echo "kundali-webapp already up on :8000"; exit 0; fi
[ -x "$K/venv/bin/python" ] || { echo "no venv in $K — run its start.command once"; exit 1; }
( cd "$K/backend" && nohup ../venv/bin/python -m uvicorn app:app --host 127.0.0.1 --port 8000 > "$HOME/Library/Logs/jyotish-os/kundali.log" 2>&1 & )
for _ in $(seq 1 30); do sleep 1; curl -s -m 2 http://127.0.0.1:8000/ >/dev/null && { echo "kundali-webapp up on :8000"; exit 0; }; done
echo "kundali-webapp did not come up (see ~/Library/Logs/jyotish-os/kundali.log)"; exit 1

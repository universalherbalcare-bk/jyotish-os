#!/usr/bin/env bash
# vedastro-svc — VedAstro rule/dataset sidecar for jyotish-os. Loopback only (127.0.0.1:7793).
# Usage: ./run.sh            (foreground)
#        ./run.sh --daemon   (background; pid in data/vedastro-svc.pid, log in data/vedastro-svc.log)
#        ./run.sh --stop
set -euo pipefail
cd "$(dirname "$0")"
export DOTNET_SYSTEM_NET_DISABLEIPV6=1          # NuGet restore hangs on IPv6 on this host (SYN_SENT to akamai); harmless otherwise
export DOTNET_CLI_TELEMETRY_OPTOUT=1
export DOTNET_NOLOGO=1
export VEDASTRO_SVC_URL="${VEDASTRO_SVC_URL:-http://127.0.0.1:7793}"
mkdir -p data
case "${1:-}" in
  --stop)
    if [[ -f data/vedastro-svc.pid ]]; then kill "$(cat data/vedastro-svc.pid)" 2>/dev/null || true; rm -f data/vedastro-svc.pid; echo "stopped"; else echo "not running"; fi ;;
  --daemon)
    dotnet build VedAstroSvc/VedAstroSvc.csproj -c Release -nologo -v q
    nohup dotnet VedAstroSvc/bin/Release/net10.0/VedAstroSvc.dll > data/vedastro-svc.log 2>&1 &
    echo $! > data/vedastro-svc.pid
    for i in $(seq 1 60); do curl -sf "$VEDASTRO_SVC_URL/v1/health" >/dev/null 2>&1 && { echo "vedastro-svc up on $VEDASTRO_SVC_URL (pid $(cat data/vedastro-svc.pid))"; exit 0; }; sleep 0.5; done
    echo "vedastro-svc did not become healthy; see data/vedastro-svc.log" >&2; exit 1 ;;
  *)
    exec dotnet run --project VedAstroSvc/VedAstroSvc.csproj -c Release ;;
esac

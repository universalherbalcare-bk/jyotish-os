#!/usr/bin/env python3
"""Bottleneck benchmark for the running JYOTISH-OS stack. Prints Markdown; numbers are measured now.

What it measures (all through the real TLS MCP on 127.0.0.1:7791):
  1. chart.compute cold (unique births → cache miss, includes the consensus round-trip to jhora-svc)
  2. chart.compute warm (same birth → cache hit)
  3. engine.consensus alone (the sidecar round-trip cost)
  4. catalog.list (pure in-process baseline)
  5. dasha.timeline depth 2 (proxied to the PyJHora process pool)
  6. muhurta.find one day (proxied to vedastro-svc rule evaluation)
  7. concurrency: 24 unique chart.compute in flight at once (throughput + p95)
"""

from __future__ import annotations

import concurrent.futures as cf
import json
import os
import ssl
import statistics as st
import time
import urllib.error
import urllib.request

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
CA = os.path.join(ROOT, "certs", "ironclaw-ca-bundle.pem")
MCP = "https://127.0.0.1:7791/mcp"
CTX = ssl.create_default_context(cafile=CA)
GOLDEN = {
    "utc": "1990-03-15T06:30:00Z",
    "lat": 28.6139,
    "lon": 77.2090,
    "tz_offset_hours": 5.5,
}


def call(name: str, args: dict, timeout: int = 60) -> tuple[float, dict | None]:
    body = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": args},
        }
    ).encode()
    req = urllib.request.Request(
        MCP, data=body, headers={"content-type": "application/json"}
    )
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=timeout, context=CTX) as r:
            d = json.loads(r.read().decode())
    except urllib.error.URLError, TimeoutError, ValueError:
        return time.perf_counter() - t0, None
    return time.perf_counter() - t0, d.get("result", {}).get("structuredContent")


def birth(i: int) -> dict:
    # deterministic unique births (different minute each) so every call is a cache miss
    return {
        "utc": f"19{70 + i % 30:02d}-{1 + i % 12:02d}-{1 + i % 27:02d}T{i % 24:02d}:{(i * 7) % 60:02d}:00Z",
        "lat": -60 + (i * 9.7) % 120,
        "lon": -180 + (i * 37.3) % 360,
        "tz_offset_hours": 0,
    }


def stats(xs: list[float]) -> str:
    xs = sorted(xs)
    if not xs:
        return "n=0"
    p95 = xs[min(len(xs) - 1, round(0.95 * (len(xs) - 1)))]
    return f"n={len(xs)} p50={st.median(xs) * 1000:.0f}ms p95={p95 * 1000:.0f}ms max={xs[-1] * 1000:.0f}ms"


def main() -> None:
    out = [f"# Bench (live, {time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())})\n"]
    # 1. cold
    cold, status = [], {}
    for i in range(12):
        dt_, r = call("chart.compute", {"birth": birth(100 + i)})
        cold.append(dt_)
        s = (r or {}).get("evidence", {}).get("consensus_status", "NONE")
        status[s] = status.get(s, 0) + 1
    out.append(
        f"| chart.compute cold (unique, consensus on) | {stats(cold)} | consensus {status} |"
    )
    # 2. warm
    call("chart.compute", {"birth": GOLDEN})
    warm = [call("chart.compute", {"birth": GOLDEN})[0] for _ in range(12)]
    out.append(f"| chart.compute warm (cache hit) | {stats(warm)} | |")
    # 3. consensus alone
    cons = [call("engine.consensus", {"birth": birth(200 + i)})[0] for i in range(8)]
    out.append(f"| engine.consensus (sidecar round-trip) | {stats(cons)} | |")
    # 4. baseline
    cat = [call("catalog.list", {})[0] for _ in range(12)]
    out.append(f"| catalog.list (in-process baseline) | {stats(cat)} | |")
    # 5. dasha
    das = [
        call(
            "dasha.timeline",
            {"birth": birth(300 + i), "system": "graha.vimsottari", "depth": 2},
        )[0]
        for i in range(6)
    ]
    out.append(f"| dasha.timeline depth 2 (PyJHora pool) | {stats(das)} | |")
    # 6. muhurta
    muh = [
        call(
            "muhurta.find",
            {
                "activity": "GoodLunarDayForTravel",
                "from_utc": f"2026-10-{1 + i:02d}T00:00:00Z",
                "to_utc": f"2026-10-{2 + i:02d}T00:00:00Z",
                "lat": 28.6139,
                "lon": 77.2090,
                "tz_offset_hours": 5.5,
                "step_minutes": 60,
            },
            timeout=120,
        )[0]
        for i in range(3)
    ]
    out.append(f"| muhurta.find 1 day @60 min (vedastro rules) | {stats(muh)} | |")
    # 7. concurrency
    t0 = time.perf_counter()
    with cf.ThreadPoolExecutor(max_workers=24) as ex:
        lat = list(
            ex.map(
                lambda i: call("chart.compute", {"birth": birth(400 + i)})[0], range(24)
            )
        )
    wall = time.perf_counter() - t0
    out.append(
        f"| 24 concurrent unique chart.compute | {stats(lat)} | wall {wall:.2f}s → {24 / wall:.1f} charts/s |"
    )
    header = "| Path | Latency | Notes |\n|---|---|---|"
    print("\n".join([out[0], header] + out[1:]))


if __name__ == "__main__":
    main()

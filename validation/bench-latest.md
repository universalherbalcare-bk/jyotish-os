# Bench (live, 2026-09-20T12:46:16Z)

| Path | Latency | Notes |
|---|---|---|
| chart.compute cold (unique, consensus on) | n=12 p50=6ms p95=6ms max=11ms | consensus {'PASS': 12} |
| chart.compute warm (cache hit) | n=12 p50=5ms p95=6ms max=6ms | |
| engine.consensus (sidecar round-trip) | n=8 p50=5ms p95=6ms max=6ms | |
| catalog.list (in-process baseline) | n=12 p50=3ms p95=3ms max=3ms | |
| dasha.timeline depth 2 (PyJHora pool) | n=6 p50=5ms p95=7ms max=7ms | |
| muhurta.find 1 day @60 min (vedastro rules) | n=3 p50=269ms p95=347ms max=347ms | |
| 24 concurrent unique chart.compute | n=24 p50=36ms p95=51ms max=54ms | wall 0.08s → 317.3 charts/s |

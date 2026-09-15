# validation/ — how every number here was produced (and how to reproduce it)

All commands run from the repo root unless stated. Nothing in this directory is hand-edited: each
file is the captured stdout/stderr of the command next to it, with the exit code appended or
printed. Prerequisites: `vendor/` restored (`scripts/vendor.sh`), `kernels/de440s.bsp` present and
matching `kernels/de440s.sha256`, Rust stable ≥ 1.88, `uv`, .NET 10 (vedastro-svc only).

| file | what | command |
|---|---|---|
| `xalen-cargo-test.txt` | XALEN's own test suite (all crates except the Python/Node bindings) | `cd vendor/xalen && cargo test --workspace --exclude xalen-python --exclude xalen-node 2>&1 \| tee ../../validation/xalen-cargo-test.txt` |
| `de440_bench.txt` | XALEN DE440 planetary-longitude precision benchmark against the pinned kernel | `cd vendor/xalen/validation && cargo run --release --bin de440_bench -- --kernel "$PWD/../../../kernels/de440s.bsp" 2>&1 \| tee ../../../validation/de440_bench.txt` |
| `pyjhora-pvr-tests.txt` | PyJHora's bundled ~10.4k-case book suite, LAHIRI, **mean** nodes (the configuration the suite's baseline was generated with) | `cd services/jhora-svc && .venv/bin/python run_pvr_tests.py --nodes mean --baseline compare 2>&1 \| tee ../../validation/pyjhora-pvr-tests.txt` — 10404 tests, 265 fail (257 float-repr noise < 1e-8°, 5 last-digit, 3 other); exit 1 is the suite's own verdict |
| `pyjhora-pvr-tests-truenodes.txt` | same suite in the jhora-svc configuration (**true** nodes) | `... --nodes true --baseline none` — +184 Rahu/Ketu-dependent cases differ from the mean-node baseline, as expected |
| `jyotish-mcp-cargo-test.txt`, `jyotish-mcp-clippy.txt` | jyotish-mcp unit + in-process integration tests, clippy `-D warnings` | `cd services/jyotish-mcp && cargo test 2>&1 \| tee ../../validation/jyotish-mcp-cargo-test.txt; cargo clippy --all-targets -- -D warnings` (the integration test needs the real kernel; a missing kernel is a failure, not a skip) |
| `jyotish-mcp-health.json`, `jyotish-mcp-chart-golden-live7791.json`, `jyotish-mcp-chart-golden-sidecar-down.json`, `jyotish-mcp-panchang-golden.json`, `jyotish-mcp-consensus-golden.json` | Phase-1/2 live captures from the running server (`GET /health`, `tools/call` on the golden chart) — the consensus one is the **pre-Phase-4 FAIL** kept for the record | `curl -s localhost:7791/mcp -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"engine.consensus","arguments":{"birth":{"utc":"1990-03-15T06:30:00Z","lat":28.6139,"lon":77.2090,"tz_offset_hours":5.5}}}}'` |
| `jyotish-mcp-consensus-golden-v2.json`, `jyotish-mcp-chart-golden-v2.json` | Phase-4b live captures: golden chart `engine.consensus` → `PASS` (same-instant comparison, DE440 node 0.014″) and `chart.compute` → `consensus_status: PASS` | same `tools/call` after rebuilding/restarting both services (see below) |
| `consensus-corpus.csv`, `consensus-summary.md`, `consensus-run.txt` | the 10,000-chart differential corpus (docs/CONTRACT.md "Consensus tolerances") | `scripts/consensus.sh --full` (≡ `validation/consensus/target/release/consensus-corpus --n 10000 --seed 42`); `consensus-run.txt` is its stdout+stderr with `time` |

## Service tests (Phase 2/3, re-run in Phase 4)

```bash
cd services/jhora-svc && ./setup_env.sh && .venv/bin/python -m pytest -q     # 19 passed (Phase 4: +2 for ayanamsa conventions / time scale)
cd services/vedastro-svc && dotnet build -warnaserror:CS8600,CS8602 && dotnet test
```

## The consensus corpus (`validation/consensus`, Rust, edition 2024)

Links the jyotish-mcp library (so the engine under test is byte-for-byte the server's: kernel pin,
DE440 provenance, coverage guard, `Instant` time-scale policy) and POSTs the same `BirthInput` to
jhora-svc `/v1/positions`. Deterministic sampling from `--seed` (SplitMix64): UTC uniform over
`--years` (default 1900-01-01..2050-12-31, whole seconds), lat uniform −66..66, lon −180..180,
`tz_offset_hours = round(lon/15)`. Concurrency `--concurrency` (default 6) against the 4-worker
sidecar; 5xx/429/transport errors are retried with exponential backoff (`--retries`, default 6);
4xx or exhausted retries are recorded as failures and make the exit code 2 — no chart is ever dropped.

Per chart it records (CSV): both engines' `jd_ut`/`jd_tt`, ayanamsa under both conventions, the
Ascendant with XALEN at Swiss's UT1 (gated) and at its own UT1 (diagnostic), and for each of the 9
grahas the tropical (sidereal + own true-equinox ayanamsa) and sidereal deltas at the aligned instant
(gated) and at XALEN's own instant (diagnostic), the elongation from the Sun, the per-chart tropical
tolerance (2.5″ inside the 1.0° solar-conjunction band), the node source, and retrograde flags. The summary gives mean (signed) / mean |Δ| / RMS / p50 / p99 / p99.9 / max per
category (a category passes when no chart exceeds its per-chart tolerance AND p99.9 ≤ the nominal
tolerance), the same split by time-scale era (pre-1972 · 1972..2033-09-16 · 2033-09-17..), the
era-gated `time_scale.*` categories, the ten worst epochs by |Δ|/tolerance, the conjunction-band charts,
and the jd_ut / jd_tt convention characterisation.

```bash
scripts/consensus.sh                 # n=500, the CI gate; starts jhora-svc if needed
scripts/consensus.sh --full          # n=10000 seed=42 → validation/consensus-corpus.csv + consensus-summary.md
scripts/consensus.sh --n 2000 --seed 7 --years 1972:2033 --csv /tmp/c.csv --summary /tmp/c.md
cd validation/consensus && cargo test --release && cargo clippy --all-targets -- -D warnings
```

Exit codes: 0 every gated category within tolerance · 1 a category over tolerance · 2 a chart failed ·
3 engine/sidecar boot failure · 64 bad arguments. Timing on this machine (Apple Silicon, 4 sidecar
workers): 10,000 charts in 6.8 s (≈1460 charts/s), 0 retries, 0 failures.

**Current verdict at N=10000: PASS (exit 0)** — Phase 4b: every gated category is evaluated at the
same instant (XALEN at jhora-svc's `jd_tt`/`jd_ut`), the time-scale conventions are gated separately per
era, Rahu/Ketu come from DE440's own lunar state vector (p99.9 1.41″ vs Swiss), and planets within 1.0°
of the Sun are held to a 2.5″ per-chart tropical tolerance (gravitational deflection is not modelled in
XALEN's DE440 chain — upstream item; the p99.9 gate stays at 1.0″). Own-instant deltas remain in the
summary as `diag.*`. All numbers and their attribution: docs/CONTRACT.md "Consensus tolerances".

## Rebuild + restart both services (what the v2 golden captures were taken from)

```bash
cd services/jyotish-mcp && cargo build --release && cd ../..
pkill -f target/release/jyotish-mcp; nohup services/jyotish-mcp/target/release/jyotish-mcp > audit/server.log 2>&1 &
cd services/jhora-svc && pkill -f 'uvicorn app:app'; nohup ./run.sh > server.log 2>&1 & cd ../..
curl -s localhost:7791/health; curl -s localhost:7792/v1/health
```

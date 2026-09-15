# jyotish-mcp

JYOTISH-OS L4 server: **MCP JSON-RPC 2.0 over HTTP**, with the XALEN ephemeris
crates linked natively (by path from `vendor/xalen`) and the **JPL DE440** kernel
mandatory at boot. PyJHora (`jhora-svc`, :7792) and VedAstro (`vedastro-svc`, :7793)
are proxied, never linked. Binding contract: `docs/CONTRACT.md`.

## Run

```bash
# one-time: kernel + pin (from the repo root)
curl -L -o kernels/de440s.bsp https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440s.bsp
shasum -a 256 kernels/de440s.bsp > kernels/de440s.sha256

cd services/jyotish-mcp
cargo build --release
cd ../..                                   # run from the repo root (default kernel path is ./kernels)
nohup services/jyotish-mcp/target/release/jyotish-mcp > audit/server.log 2>&1 &
curl -s localhost:7791/health
```

Boot guard (any failure aborts with a non-zero exit and a JSON fatal line):

1. `kernels/de440s.bsp` SHA-256 must equal the pinned value in `kernels/de440s.sha256` (exit 3).
2. The SPK must confirm `DE440` provenance (exit 3).
3. The GOLDEN chart (1990-03-15T06:30:00Z, New Delhi) must put the Moon in Swati
   (sidereal Lahiri longitude in [186.6667, 200.0)) (exit 3).
4. Bind and sidecar URLs must be loopback (exit 2).

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `JYOTISH_BIND` | `127.0.0.1:7791` | Listen address; non-loopback is refused |
| `JYOTISH_KERNEL` | `./kernels/de440s.bsp` | DE440 SPK kernel |
| `JYOTISH_KERNEL_SHA256` | `<kernel>.sha256` | Pinned hash file (`shasum -a 256` format) |
| `JHORA_URL` | `http://127.0.0.1:7792` | jhora-svc base URL (http, loopback only) |
| `VEDASTRO_URL` | `http://127.0.0.1:7793` | vedastro-svc base URL (http, loopback only) |
| `JYOTISH_AUDIT` | `./audit/jyotish-mcp.jsonl` | Append-only audit log (tool, request hash, ms, status; no payloads) |
| `JYOTISH_CACHE_CAP` | `4096` | Max entries in the content-addressed cache |
| `JYOTISH_SIDECAR_TIMEOUT_MS` | `5000` | Per-request sidecar timeout |

No secrets are read or stored.

## Endpoints

* `GET /health` — kernel id/sha, coverage, golden self-test result, sidecar liveness, tool list.
* `POST /mcp` — MCP 2025-06-18 JSON-RPC: `initialize`, `ping`, `tools/list`, `tools/call`.
  Notifications (no `id`) return `202`. Batches are not accepted (removed in 2025-06-18).
  Tool results: `content:[{type:"text", text:<json>}]` + `structuredContent` (same JSON);
  tool failures set `isError: true` with `structuredContent.error = {code, message, details}`.

Every result carries `evidence: { engine, kernel, kernel_id, kernel_sha256, ayanamsa:"LAHIRI",
nodes:"TRUE", consensus_status, delta_t_sigma_sec, cache_hit }`. `delta_t_sigma_sec` is the
1σ of the SMH2016 ΔT model at the request epoch from `xalen-time`; it is `null` for results
without an epoch (catalog, proxied tools).

## Tools

| Tool | Backed by | Notes |
|---|---|---|
| `chart.compute` | XALEN + DE440 | `{birth: BirthInput, varga?: D1…D60}`. Sidereal lon/speed/retrograde, rashi, nakshatra, pada, `boundary_distance_sec` {rashi, nakshatra, pada} (linear from speed; retrograde counts backward), Ascendant (GAST + true obliquity, Swiss-equivalent), Whole-Sign (sidereal sign boundaries) and Sripati cusps, per-body `source`. When jhora-svc is reachable the result is **cross-checked** and any out-of-tolerance body returns `CONSENSUS_FAIL` — never a number. |
| `panchang.day` | XALEN + DE440 | `{date, lat, lon, tz_offset_hours}`. Tithi/nakshatra/yoga/karana with start/end instants from sunrise to next sunrise, sunrise/sunset/moonrise/moonset, vara, rahu kala, abhijit. |
| `transit.window` | XALEN + DE440 | `{from, to, bodies?, birth?, natal_points?, step_hours?}` (≤ 10 years). Bisection to exact rashi ingress and exact conjunction instants, direct and retrograde. |
| `rectify.birth_time` | XALEN + DE440 | `{birth, window_min?, step_min?, events:[{date|utc, significators[], label?}], top_n?}`. **Heuristic** sweep scored by Vimshottari maha/antar lords vs supplied significators; explicitly labelled. |
| `engine.consensus` | XALEN vs jhora-svc | Per-body Δ in arcsec vs contract tolerances; `PASS` / `FAIL` / `SIDECAR_UNAVAILABLE`. Ayanamsa is compared under both the true-equinox (XALEN) and mean-equinox (`swe_get_ayanamsa_ut`) conventions and the matched one is reported. |
| `catalog.list` | static | Enabled ayanamsa (LAHIRI only), 21 house systems, 16 vargas, dasha systems. **Gauquelin, QiMenDunJia, PullenSinusoidalRatio are excluded** with reasons. |
| `dasha.timeline` | jhora-svc `/v1/dasha` | Proxied; `evidence.engine = "jhora-svc"`. |
| `match.kuta` | jhora-svc `/v1/kuta` | Proxied. |
| `muhurta.find` | vedastro-svc `/v1/muhurta/find` | Proxied. |
| `rule.validate` | vedastro-svc `/v1/rule/validate` | Proxied. |

A down sidecar yields `SIDECAR_UNAVAILABLE` (structured, `isError: true`); nothing is fabricated.

## Guarantees enforced in code (not prose)

* Kernel hash pin + DE440 provenance check at boot; the almanac is never built otherwise.
* Every instant is checked against the kernel's own coverage window (JD 2396752.5–2506352.5,
  with a 5-day margin) → `KERNEL_COVERAGE` error; XALEN's silent VSOP87 fallback is unreachable.
* Loopback-only bind and sidecar URLs, validated at boot.
* Content-addressed cache keyed by `sha256(tool ‖ canonical JSON ‖ kernel sha256)`.
* Append-only JSONL audit; request payloads (birth data) never logged.

## Conventions

* Ayanamsa `Ayanamsa::Lahiri` evaluated in TT (true-equinox value); nodes `Body::TrueNode`
  (XALEN's osculating node — analytic, tagged `source: "xalen-true-node (osculating, analytic)"`).
* UTC → TT is leap-second exact (`Epoch::from_utc`); UT1 ≈ UTC (|DUT1| < 0.9 s).
* Speeds are sidereal rates (tropical rate minus ayanamsa rate).

## Tests

```bash
cd services/jyotish-mcp
cargo test                      # unit (boundary math, mapping, cache, config, catalog) + in-process integration
cargo clippy --all-targets -- -D warnings
cargo build --release
```

The integration test boots the server on a random loopback port with the real kernel and
exercises `initialize`, `ping`, `tools/list`, `chart.compute`, `panchang.day`,
`transit.window`, `rectify.birth_time`, `engine.consensus`, `catalog.list`, the
sidecar-down path, invalid params, out-of-coverage dates, and the audit log.
A missing kernel is a test **failure**, not a skip.

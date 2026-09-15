# JYOTISH-OS Service Contract (binding for every phase)

## Ports (loopback only)
| Service | Bind | Purpose |
|---|---|---|
| jyotish-mcp | 127.0.0.1:7791 | MCP JSON-RPC 2.0 at `POST /mcp`; `GET /health` |
| jhora-svc | 127.0.0.1:7792 | PyJHora sidecar, JSON REST |
| vedastro-svc | 127.0.0.1:7793 | VedAstro rule/dataset sidecar, JSON REST |

## Canonical config (all engines MUST match; jyotish-mcp aborts at boot otherwise)
- ayanamsa: LAHIRI  (xalen `Ayanamsa::Lahiri`; PyJHora `const._DEFAULT_AYANAMSA_MODE='LAHIRI'`; VedAstro `Ayanamsa.LAHIRI=1`)
- nodes: TRUE (xalen TrueNode; PyJHora `const._use_true_nodes_for_rahu_ketu=True`)
- ephemeris: xalen = JPL DE440 via `kernels/de440s.bsp` (sha256 pinned in `kernels/de440s.sha256`); PyJHora = bundled Swiss `.se1`
- house: WholeSign for rasi; bhava = Sripati
- zodiac output: sidereal longitudes in degrees [0,360)
- no runtime geocoding anywhere; callers pass lat/lon/tz

## Shared input type
```json
BirthInput = { "utc": "1990-03-15T06:30:00Z", "lat": 28.6139, "lon": 77.2090, "tz_offset_hours": 5.5 }
```
`utc` is authoritative; `tz_offset_hours` is only for local rendering (sunrise-relative panchanga in PyJHora needs the place tz).

## GOLDEN chart (boot self-test + consensus smoke)
BirthInput above (1990-03-15 12:00 IST, New Delhi). Expected: Moon sidereal longitude in Swati (186°40′–200°00′) — from xalen README.

## Sidecar REST (jhora-svc, port 7792)
- `GET  /v1/health` → `{ "ok": true, "ayanamsa": "LAHIRI", "true_nodes": true, "pyswisseph": "2.10.x", "ephe_files": <count> }`
- `POST /v1/positions` BirthInput → `{ "bodies": { "Sun": {"lon": f, "speed": f, "retro": bool}, "Moon":…, "Mercury","Venus","Mars","Jupiter","Saturn","Rahu","Ketu" }, "ascendant": f, "ayanamsa_deg": f, "jd_ut": f }`
- `POST /v1/dasha` `{ "birth": BirthInput, "system": "<id>", "depth": 1..5 }` → `{ "system": id, "balance_at_birth_years": f, "periods": [ { "lord": str, "start_utc": iso, "end_utc": iso, "children": [...] } ] }`
- `GET  /v1/catalog/dasha` → list of every system id available (one per module in vendor/pyjhora/src/jhora/horoscope/dhasa/{graha,raasi,annual})
- `POST /v1/panchang` `{ "date": "YYYY-MM-DD", "lat", "lon", "tz_offset_hours" }` → tithi/nakshatra/yoga/karana with start/end UTC, sunrise/sunset UTC
- `POST /v1/kuta` `{ "a": BirthInput, "b": BirthInput }` → per-kuta scores + total

## Sidecar REST (vedastro-svc, port 7793)
- `GET  /v1/health` → `{ "ok": true, "rules_proved": n, "rules_quarantined": n, "dataset_rows": n }`
- `GET  /v1/rules?set=event|horoscope&status=proved|quarantined`
- `POST /v1/muhurta/find` `{ "activity": "<rule Name>", "from_utc", "to_utc", "lat", "lon", "tz_offset_hours", "step_minutes": 60 }` → ranked windows with `passed_rules[]`, `vetoed_by[]`
- `POST /v1/rule/validate` `{ "rule_id", "dataset": "marriage|person", "outcome_column" }` → `{ n, hits, hit_rate, base_rate, ci95:[lo,hi], verdict: "PROMOTE"|"KEEP_UNPROVED" }`

## jyotish-mcp tools (MCP `tools/list`)
chart.compute · panchang.day · dasha.timeline · transit.window · muhurta.find · match.kuta · rectify.birth_time · rule.validate · engine.consensus · catalog.list
Every result includes `"evidence": { "engine", "kernel", "ayanamsa", "consensus_status", "delta_t_sigma_sec" }`.

## Consensus tolerances (arcsec unless noted) — XALEN-DE440 vs jhora-svc(Swiss)
Sun/Mercury–Saturn 1.0 · Moon 1.5 (15 if analytic fallback) · Uranus/Neptune 3.0 · ayanamsa 1.0 · Ascendant 0.01° · True Node 5.0 · Vimshottari boundaries 1 day.
Out of tolerance ⇒ tool returns error, never a number.

## Test/attempt discipline
Before each test-run loop, append `{ "unit": "<name>", "attempt": n, "ts": iso }` to `.claude/execution_state.json`. Stop and report after 5 consecutive failures of the same unit.

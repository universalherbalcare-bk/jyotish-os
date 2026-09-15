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
- `POST /v1/positions` BirthInput → `{ "bodies": { "Sun": {"lon": f, "speed": f, "retro": bool}, "Moon":…, "Mercury","Venus","Mars","Jupiter","Saturn","Rahu","Ketu" }, "ascendant": f, "ayanamsa_deg": f (mean equinox, swe_get_ayanamsa_ut), "ayanamsa_true_equinox_deg": f (swe_get_ayanamsa_ex_ut FLG_SWIEPH; sidereal = tropical − this), "ayanamsa_mean_equinox_deg": f, "nutation_dpsi_arcsec": f, "jd_ut": f, "jd_tt": f (jd_ut + swe.deltat_ex), "delta_t_sec": f }`
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

## Consensus tolerances — XALEN-DE440 vs jhora-svc (Swiss), decomposed (Phase 4)
The comparison is decomposed so a disagreement is attributable, not smeared. Tropical = sidereal +
each engine's **own** true-equinox ayanamsa (Swiss: `swe_get_ayanamsa_ex_ut(jd, FLG_SWIEPH)`; XALEN:
`Ayanamsa::Lahiri.compute_deg(jd_tt)`), so (a) isolates ephemeris + reduction, (b) isolates the
ayanamsa constant, (c) is what the user sees. Out of tolerance ⇒ tool returns `CONSENSUS_FAIL`, never a number.

| # | category | tolerance | gate applies to | rationale |
|---|---|---|---|---|
| a | tropical longitude | 1.0″ (Moon 1.5″) | Sun, Moon, Mercury–Saturn | both read JPL-class data; XALEN doc ≤ 0.76″ |
| b | Lahiri ayanamsa, matching convention | 1.0″ (true-equinox and mean-equinox each) | always | XALEN's Lahiri sits a constant −0.731″ below Swiss's (1900–2050 range 0.7309–0.7355″); Swiss `swe_get_ayanamsa_ut` is mean-equinox, `_ex_ut` true-equinox — jhora-svc reports both |
| c | sidereal longitude, end-to-end | 2.0″ | Sun, Moon, Mercury–Saturn | (a) + (b): the −0.73″ ayanamsa offset is common to every body |
| d | Rahu/Ketu sidereal | 60″, `source: analytic` | nodes only | XALEN's true node is an analytic osculating node (finite difference on the analytic Moon); Swiss's is from the ephemeris state vector; Ketu ≡ Rahu + 180° in both (verified 0.0″) |
| e | Ascendant | 0.01° | always | XALEN doc: 19 house systems within 0.01°; UT1 ≈ UTC (|DUT1| < 0.9 s) |
| — | Uranus/Neptune 3.0″; Vimshottari boundaries 1 day | | not consensus-checked (jhora-svc `/v1/positions` carries the 9 grahas only) | unchanged from Phase 1 |

Sun–Saturn and the Moon must pass (a), (b) and (c); the nodes only (d); (b) and (e) always.
Implementation: `services/jyotish-mcp/src/tools/consensus.rs` (gates `engine.consensus` and `chart.compute`).

### Measured against the 10,000-chart corpus (`validation/consensus`, seed 42, 1900–2050, |lat| ≤ 66°; EXECUTED 2026-09-16, `validation/consensus-summary.md`)

| category | tolerance | p99.9 | max | n over / 10000 | status |
|---|---|---|---|---|---|
| tropical.Sun | 1.0″ | 0.578″ | 0.635″ | 0 | PASS |
| tropical.Moon | 1.5″ | 3.326″ | 3.560″ | 661 | **UNDER REVIEW** (time-scale convention, see below) |
| tropical.Mercury | 1.0″ | 0.695″ | 0.886″ | 0 | PASS |
| tropical.Venus | 1.0″ | 0.667″ | 1.635″ | 2 | **UNDER REVIEW** (light deflection at conjunction) |
| tropical.Mars | 1.0″ | 0.631″ | 1.337″ | 2 | **UNDER REVIEW** (light deflection at conjunction) |
| tropical.Jupiter | 1.0″ | 0.677″ | 1.711″ | 4 | **UNDER REVIEW** (light deflection at conjunction) |
| tropical.Saturn | 1.0″ | 0.598″ | 0.905″ | 0 | PASS |
| ayanamsa.true_equinox | 1.0″ | 0.735″ | 0.736″ | 0 | PASS |
| ayanamsa.mean_equinox | 1.0″ | 0.731″ | 0.731″ | 0 | PASS |
| sidereal.Sun | 2.0″ | 1.157″ | 1.164″ | 0 | PASS |
| sidereal.Moon | 2.0″ | 2.595″ | 2.828″ | 135 | **UNDER REVIEW** (time-scale convention) |
| sidereal.Mercury | 2.0″ | 1.257″ | 1.423″ | 0 | PASS |
| sidereal.Venus | 2.0″ | 1.311″ | 1.472″ | 0 | PASS |
| sidereal.Mars | 2.0″ | 1.246″ | 1.465″ | 0 | PASS |
| sidereal.Jupiter | 2.0″ | 1.345″ | 1.928″ | 0 | PASS |
| sidereal.Saturn | 2.0″ | 1.206″ | 1.533″ | 0 | PASS |
| nodes.Rahu = nodes.Ketu (analytic) | 60″ | 90.54″ | 105.20″ | 162 | **UNDER REVIEW** |
| ascendant | 0.01° | 0.0059° | 0.0102° | 1 | **UNDER REVIEW** (UT1 convention) |

Tolerances were **not** loosened to fit the data. Each UNDER REVIEW category has a measured, attributed cause:

1. **Moon (tropical and sidereal) — TT convention after 2033-09-17, not ephemeris.** With XALEN evaluated at
   Swiss's own TT, Moon tropical p99.9 = 0.0065″ / max 0.0068″ over all 10,000 charts. Both engines use
   leap-second-exact TT from 1972-01-01 to 2033-09-16 (TT identical to 0.000 s; Moon max 0.005″). Before
   1972 both derive TT = UT + ΔT(model); the two models differ ≤ 0.67 s (1954–56, 1962–66) → Moon ≤ 0.42″.
   From 2033-09-17 (bisected: the first instant where Swiss's ΔT(model) exceeds TAI−UTC+32.184 s by > 1 s)
   Swiss `swe_utc_to_jd` switches to `TT = UTC + ΔT(model)` (70.2 s → 74.7 s by 2050) while XALEN keeps the
   leap-second TT (69.184 s): up to 5.67 s → Moon 3.56″. First failing charts: tropical 2039-07-04, sidereal
   2046-09-29; the ten worst are all in 2050 (e.g. 2050-09-29T11:26:15Z, −63.46/+141.33: −3.560″).
   XALEN's own SMH2016 extrapolation (69.1 → 71.1 s) would still differ from Swiss's by 1.2–3.6 s there, so no
   XALEN-side convention closes this to 1.5″; it is a statement about unknown future leap seconds
   (CGPM 2022 intends none after 2035, which favours the leap-second TT). **Lead decision needed:** keep the
   full 1900–2050 corpus with this category red, or scope the Moon gate to epochs where TAI−UTC is known.
2. **Venus/Mars/Jupiter tropical max — gravitational light deflection by the Sun.** All 8 charts over 1.0″
   lie within 0.11°–0.54° of the Sun (superior conjunction: Venus 2024-06-05 ×2, Mars 1991-11-07 ×2, Jupiter
   1930-06-20, 1936-12-28, 2037-06-29, 2044-01-06). Swiss applies relativistic deflection (measured
   `calc_ut` − `FLG_NOGDEFL`: +1.37″ Venus, +1.52″ Mars, +1.36″ Jupiter); XALEN's DE440 apparent-place chain is
   light-time + precession + nutation + aberration with no deflection term (`vendor/xalen/.../de440.rs`).
   p99.9 is 0.63–0.70″ (PASS); only bodies inside ≈0.6° of the Sun exceed 1.0″ (≈0.1 % of charts). Vendor property.
3. **Rahu/Ketu — 60″ does not hold for XALEN's analytic node.** p50 16.7″, p90 40.8″, p99 65.6″, p99.9 90.5″,
   max 105.2″ (1929-06-02T13:02:39Z: +105.2″; 2025-04-22T03:48:49Z: +102.8″; 1927-10-11: +99.5″;
   1945-10-21: +99.4″; 1992-02-18: −97.6″); 162/10000 over 60″, 11 over 90″, 0 over 120″. Retrograde flag
   differs in 343/10000 charts (the node hovers near stationary). XALEN documents this node as "well under
   0.1°" (360″) vs Swiss; the corpus confirms ≤ 105″. **Lead decision needed** (120″ would pass the corpus;
   or source Rahu from the sidecar/ephemeris state vector).
4. **Ascendant 0.0102° (1/10000) — UT1 convention, not house math.** XALEN uses UT1 = UTC; Swiss uses
   UT1 = TT − ΔT(table) inside the leap-second era (XALEN − Swiss: RMS 0.215 s, max 0.998 s at
   2033-09-13). With XALEN evaluated at Swiss's UT1 the Ascendant max is 0.0023° (p99.9 0.0009°). The one
   failing chart (2033-08-19T02:55:43Z, −53.51/+87.13: 0.01016°) has ΔUT1 = 0.981 s, beyond the physical
   |DUT1| < 0.9 s bound, i.e. Swiss's extrapolated UT1 just before its own regime switch. 25 charts exceed
   0.005°, all at |lat| ≥ 31.8°.

Time-scale characterisation (`jd_ut_delta_sec` ≈ 0.194 s on the golden chart): XALEN − Swiss UT1 mean +0.003 s,
RMS 0.215 s, max 0.998 s; it affects only the Ascendant (≤ 0.00998° attributable, ≪ tolerance except the
single chart above) and nothing in longitude, because longitudes are evaluated in TT, which is identical
in both engines for 1972-01-01..2033-09-16.

Engine fix that fell out of the corpus (jyotish-mcp `Instant::from_utc_fields`): before 1972-01-01 xalen-time's
`Epoch::from_utc` applies a fixed TAI−UTC = 10 s floor (documented as out of scope), which placed TT 13–44 s
late for 1900–1971 (Moon 7–24″ off). The engine now treats pre-1972 civil time as UT1 and derives TT from the
ΔT model, matching Swiss (`services/jyotish-mcp/src/engine.rs`, unit-tested at 1900, 1950, 1972-01-01).

Reproduce: `scripts/consensus.sh --full` (or `--n 500` in CI); every number above is in
`validation/consensus-summary.md` and per-chart in `validation/consensus-corpus.csv`.

## Test/attempt discipline
Before each test-run loop, append `{ "unit": "<name>", "attempt": n, "ts": iso }` to `.claude/execution_state.json`. Stop and report after 5 consecutive failures of the same unit.

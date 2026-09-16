# JYOTISH-OS Service Contract (binding for every phase)

## Ports (loopback only)
| Service | Bind | Purpose |
|---|---|---|
| jyotish-mcp | 127.0.0.1:7791 | MCP JSON-RPC 2.0 at `POST /mcp`; `GET /health`. **HTTPS** (Phase 6): locally issued cert from `scripts/gen-cert.sh`, SAN `IP:127.0.0.1, DNS:localhost`; `JYOTISH_TLS=off` for plain HTTP (tests) |
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
- `POST /v1/rule/validate` `{ "rule_id", "dataset": "marriage|person", "outcome_column" }` → `{ n, hits, hit_rate, base_rate, ci95:[lo,hi], verdict: "PROMOTE"|"KEEP_UNPROVED", validation_status: "PROMOTED"|"PROMOTED_STILL_QUARANTINED"|"NOT_PROMOTED", promotion_status, promotion_entry_hash }`. Every call appends one hash-chained row to the append-only promotion log (blueprint §6); no row → no verdict (503 `promotion_log_unavailable`).
- `GET  /v1/rule/promotions?rule_id=` → `{ count, path, entries:[{ts, rule_id, dataset, dataset_sha256, outcome_column, n, fired, hits, hit_rate, base_rate, ci95:[lo,hi], verdict, prev_hash, entry_hash}] }`; `GET /v1/rule/promotions/verify` → `{ ok, entries, first_bad_index, head_hash }`. `GET /v1/rules` rows carry `promotion_status: PROMOTED|NOT_PROMOTED|NEVER_VALIDATED` derived from the log on every call. Promotion never changes `status` (proved/quarantined) or muhurta eligibility.

## jyotish-mcp tools (MCP `tools/list`)
chart.compute · panchang.day · dasha.timeline · transit.window · muhurta.find · match.kuta · rectify.birth_time · rule.validate · engine.consensus · catalog.list
Every result includes `"evidence": { "engine", "kernel", "ayanamsa", "consensus_status", "delta_t_sigma_sec" }`.

## Consensus tolerances — XALEN-DE440 vs jhora-svc (Swiss), decomposed (Phase 4 / 4b)
**Principle:** the gate measures *ephemeris* agreement, so both engines are compared at the **same
instant** — XALEN is evaluated at the sidecar's reported `jd_tt` (longitudes, ayanamsa) and `jd_ut`
(Ascendant). Time-scale conventions are gated separately (`time_scale`). Tropical = sidereal + each
engine's **own** true-equinox ayanamsa (Swiss: `swe_get_ayanamsa_ex_ut(jd, FLG_SWIEPH)`; XALEN:
`Ayanamsa::Lahiri.compute_deg(jd_tt)`), so (a) isolates ephemeris + reduction, (b) the ayanamsa
constant, (c) what the user sees. Out of tolerance ⇒ tool returns `CONSENSUS_FAIL`, never a number.

| # | category | tolerance | gate applies to | rationale |
|---|---|---|---|---|
| a | tropical longitude (aligned TT) | 1.0″ (Moon 1.5″); **2.5″ per chart inside the solar-conjunction band** (planet ≤ 1.0° from the Sun) | Sun, Moon, Mercury–Saturn | both read JPL-class data; Swiss applies gravitational light deflection by the Sun (≤ 1.7″ at the limb), XALEN's DE440 chain does not — upstream XALEN item; the corpus p99.9 gate stays at 1.0″ |
| b | Lahiri ayanamsa, matching convention | 1.0″ (true-equinox and mean-equinox each) | always | XALEN's Lahiri sits a constant −0.731″ below Swiss's (1900–2050: 0.7309–0.7355″); `swe_get_ayanamsa_ut` is mean-equinox, `_ex_ut` true-equinox — jhora-svc reports both |
| c | sidereal longitude, end-to-end (aligned TT) | 2.0″ | Sun, Moon, Mercury–Saturn | (a) + (b): the −0.73″ offset is common to every body |
| d | Rahu/Ketu sidereal | **5.0″**, `source: de440-osculating` (analytic fallback 120″, tagged) | nodes only | osculating node of DE440's own geocentric lunar state vector (r × v, ecliptic of date) = Swiss `SE_TRUE_NODE` definition; golden chart 0.014″; Ketu ≡ Rahu + 180° (verified 0.0″). The analytic XALEN node (finite difference on the analytic Moon) measured p99.9 90.5″ / max 105.2″ / 0 over 120″ and is now only a fallback if the kernel cannot serve the Moon |
| e | Ascendant (aligned UT1) | 0.01° | always | XALEN doc: 19 house systems within 0.01° |
| f | time_scale ΔTT / ΔUT1 (each engine's own instant) | 1900–1971: \|ΔTT\| ≤ 1.0 s · 1972-01-01..2033-09-16: \|ΔTT\| ≤ 0.01 s and \|ΔUT1\| ≤ 1.0 s · from 2033-09-17: not gated, `convention: "extrapolated"` | always | pre-1972 both engines use ΔT tables (this gate is what would have caught the 13–44 s pre-1972 floor bug); inside the leap-second era TT is known exactly; after Swiss's switch date the gap is a statement about unknown future leap seconds — XALEN assumes none (CGPM 2022), Swiss extrapolates ΔT |
| — | Uranus/Neptune 3.0″; Vimshottari boundaries 1 day | | not consensus-checked (`/v1/positions` carries the 9 grahas only) | unchanged from Phase 1 |

Sun–Saturn and the Moon must pass (a), (b), (c); the nodes only (d); (b), (e), (f) always.
Implementation: `services/jyotish-mcp/src/tools/consensus.rs` (gates `engine.consensus` and `chart.compute`);
node: `services/jyotish-mcp/src/engine.rs` `Engine::de440_osculating_node`.

### Measured against the 10,000-chart corpus (`validation/consensus`, seed 42, 1900–2050, |lat| ≤ 66°; EXECUTED 2026-09-16, `validation/consensus-summary.md`) — **verdict PASS**

| category | tolerance | n | p99.9 | max | over per-chart tol | status |
|---|---|---|---|---|---|---|
| tropical.Sun | 1.0″ | 10000 | 0.427″ | 0.430″ | 0 | PASS |
| tropical.Moon | 1.5″ | 10000 | 0.0065″ | 0.0068″ | 0 | PASS |
| tropical.Mercury | 1.0″ | 10000 | 0.479″ | 0.550″ | 0 (60 in the conjunction band) | PASS |
| tropical.Venus | 1.0″ | 10000 | 0.613″ | 1.635″ | 0 (44 in band; max is in band) | PASS |
| tropical.Mars | 1.0″ | 10000 | 0.551″ | 1.337″ | 0 (50 in band; max is in band) | PASS |
| tropical.Jupiter | 1.0″ | 10000 | 0.679″ | 1.693″ | 0 (35 in band; max is in band) | PASS |
| tropical.Saturn | 1.0″ | 10000 | 0.591″ | 0.905″ | 0 (16 in band) | PASS |
| ayanamsa.true_equinox | 1.0″ | 10000 | 0.735″ | 0.736″ | 0 | PASS |
| ayanamsa.mean_equinox | 1.0″ | 10000 | 0.731″ | 0.731″ | 0 | PASS |
| sidereal.Sun | 2.0″ | 10000 | 1.158″ | 1.164″ | 0 | PASS |
| sidereal.Moon | 2.0″ | 10000 | 0.734″ | 0.734″ | 0 | PASS |
| sidereal.Mercury | 2.0″ | 10000 | 1.183″ | 1.193″ | 0 | PASS |
| sidereal.Venus | 2.0″ | 10000 | 1.305″ | 1.364″ | 0 | PASS |
| sidereal.Mars | 2.0″ | 10000 | 1.245″ | 1.530″ | 0 | PASS |
| sidereal.Jupiter | 2.0″ | 10000 | 1.346″ | 1.928″ | 0 | PASS |
| sidereal.Saturn | 2.0″ | 10000 | 1.213″ | 1.533″ | 0 | PASS |
| nodes.Rahu = nodes.Ketu (de440-osculating) | 5.0″ | 10000 | 1.410″ | 1.442″ | 0 | PASS |
| ascendant | 0.01° | 10000 | 0.0009° | 0.0023° | 0 | PASS |
| time_scale.tt (pre-1972) | 1.0 s | 4779 | 0.666 s | 0.666 s | 0 | PASS |
| time_scale.tt (1972..2033-09-16) | 0.01 s | 4125 | 0.0001 s | 0.0001 s | 0 | PASS |
| time_scale.ut1 (1972..2033-09-16) | 1.0 s | 4125 | 0.979 s | 0.998 s | 0 | PASS (at the edge, see note 4) |
| diag.time_scale.tt (2033-09-17..) | not gated | 1096 | 5.669 s | 5.674 s | — | reported, `extrapolated` |

Attribution of everything that is not simply "both engines agree":

1. **Moon.** At the same TT the two ephemerides agree to 0.0068″ max over 10,000 charts. Evaluated at each
   engine's *own* TT (now `diag.tropical_own_instant.Moon`) the delta is 3.56″ max, entirely from the
   TT convention after Swiss's 2033-09-17 switch (`TT = UTC + ΔT(model)`, up to 5.67 s from XALEN's
   leap-second TT by 2050). Reported under `time_scale.convention = "extrapolated"`, not gated.
2. **Solar-conjunction band.** Of 10,000 charts, 205 had a planet within 1.0° of the Sun; the tropical
   maxima (Venus 1.635″ 2024-06-05 at 0.11°, Mars 1.337″ 1991-11-07 at 0.13°, Jupiter 1.693″
   2037-06-29 at 0.27°) are all in the band and all ≤ 2.5″. Cause verified: Swiss `calc_ut` −
   `FLG_NOGDEFL` = +1.37″ / +1.52″ / +1.36″ on those charts; XALEN's DE440 apparent-place chain is
   light-time + precession + nutation + aberration with no deflection term (`vendor/xalen/crates/xalen-ephem/src/de440.rs`);
   nothing in `vendor/xalen` implements deflection (grep: only a test comment). **Upstream XALEN item.**
   Outside the band every planet's p99.9 is ≤ 0.68″.
3. **Nodes.** DE440 osculating node vs Swiss true node: p50 0.727″, p99 1.322″, p99.9 1.410″, max 1.442″
   sidereal (this includes the 0.731″ ayanamsa constant; the tropical node agrees to ≈0.7″). Retrograde
   flag differs in 66/10000 charts (node near stationary; not gated). The analytic fallback was used by
   0/10000 charts.
4. **UT1.** XALEN uses UT1 = UTC; Swiss uses UT1 = TT − ΔT(table) inside the leap-second era: XALEN − Swiss
   mean +0.003 s, RMS 0.215 s, max 0.998 s. The maximum is reached in 2033-08/09 just before Swiss's own
   regime switch, where its extrapolated ΔT implies |UT1 − UTC| ≈ 1 s — beyond the physical |DUT1| < 0.9 s
   bound, i.e. a model artefact, not a real DUT1. The 1.0 s gate therefore passes by 2 ms at 2033-09-13;
   any later Swiss ΔT table revision could move it either way — that is exactly what this gate exists to
   flag. Ascendant at each engine's own UT1 (diagnostic): max 0.0102°; at the same UT1: max 0.0023°.
5. **Pre-1972 TT.** Both engines derive TT from ΔT tables there (SMH2016 vs Swiss's table): max 0.666 s
   (1954–56, 1962–66) → Moon ≤ 0.37″ at own instants; gated at 1.0 s. Before the Phase-4 fix XALEN's
   `Epoch::from_utc` applied a fixed TAI−UTC = 10 s floor pre-1972 (13–44 s of TT error, Moon 7–24″);
   jyotish-mcp `Instant::from_utc_fields` now treats pre-1972 civil time as UT1 (unit-tested at 1900,
   1950, 1972-01-01), matching Swiss.

Golden chart (`validation/jyotish-mcp-consensus-golden-v2.json`): `PASS`; tropical residuals Sun 0.156″,
Moon 0.002″, Mercury 0.193″, Venus 0.448″, Mars 0.363″, Jupiter 0.349″, Saturn 0.344″, Rahu/Ketu 0.014″;
ayanamsa 0.731″ (both conventions); Ascendant 0.00019° (aligned UT1); `time_scale.convention =
"known_tai_utc"`, ΔTT 0.000 s, ΔUT1 −0.194 s.

Reproduce: `scripts/consensus.sh --full` (or `--n 500` in CI); every number above is in
`validation/consensus-summary.md` and per-chart in `validation/consensus-corpus.csv`.

## Test/attempt discipline
Before each test-run loop, append `{ "unit": "<name>", "attempt": n, "ts": iso }` to `.claude/execution_state.json`. Stop and report after 5 consecutive failures of the same unit.

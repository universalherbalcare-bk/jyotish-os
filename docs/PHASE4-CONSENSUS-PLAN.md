# Phase 4 — consensus decomposition + differential corpus (plan)

Objective: make `engine.consensus` / `chart.compute` gate on a decomposed comparison
(tropical | ayanamsa | sidereal | nodes | ascendant) and prove the tolerances against a
10,000-chart seeded corpus (XALEN-DE440 vs jhora-svc/Swiss).

Findings that shape the plan (all EXECUTED this session, see validation/consensus-summary.md):
1. Swiss `swe_get_ayanamsa_ut` = mean-equinox value; `swe_get_ayanamsa_ex_ut(jd, FLG_SWIEPH)[1]` =
   true-equinox value; Swiss sidereal = tropical − true-equinox ayanamsa exactly (2e-10″).
2. XALEN `Epoch::from_utc` applies a fixed TAI−UTC = 10 s floor before 1972-01-01 (documented as
   out of scope in xalen-time), so jyotish-mcp's TT was 13–44 s late for 1900–1971 births
   (Moon 7–24″). Swiss treats pre-1972 UTC as UT and adds ΔT(model); the two ΔT models agree ≤ 0.2 s
   there (≤0.7 s in 1954–56/1962–66). Fix: `Instant::from_utc_fields` uses the ΔT-model path before 1972-01-01.
3. From 2033-09-17 Swiss `swe_utc_to_jd` abandons the leap-second TT (69.184 s) for `UTC + ΔT(model)`
   (70.2 → 74.7 s by 2050); XALEN keeps leap-second TT. Up to 5.5 s of TT divergence → Moon ≈ 3″.
   This is a convention, not an ephemeris error; measured and reported, not hidden.

Files
- services/jhora-svc/jhora_svc/compute.py, models.py, tests/test_service.py — add
  `ayanamsa_true_equinox_deg`, `jd_tt`, `delta_t_sec`, `nutation_dpsi_arcsec`; test sid+aya_true == tropical.
- services/jyotish-mcp/src/engine.rs — pre-1972 TT fix, `ayanamsa_mean_equinox_deg`, `Instant::from_parts`.
- services/jyotish-mcp/src/tools/consensus.rs — decomposed categories + gates; chart.rs unchanged gate.
- validation/consensus/ — Rust bin (edition 2024): corpus generator + runner → CSV, summary.md.
- scripts/consensus.sh, .github/workflows/ci.yml consensus job, docs/CONTRACT.md, validation/README.md.

Done criteria: cargo test/clippy/fmt clean; pytest clean; golden chart PASS via live tools/call;
corpus run at N=10000 with per-category p99.9 table; CONTRACT updated with measured values
(categories over tolerance marked UNDER REVIEW, never loosened).

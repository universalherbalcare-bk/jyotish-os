//! `engine.consensus` — XALEN-DE440 vs jhora-svc (Swiss) differential with
//! the docs/CONTRACT.md tolerances, decomposed so that a disagreement can be
//! attributed to its cause instead of being smeared into one number.
//!
//! **Principle (Phase 4b):** the gate measures *ephemeris* agreement, so both
//! engines are compared at the **same instant** — XALEN is evaluated at the
//! sidecar's reported `jd_tt` (longitudes, ayanamsa) and `jd_ut` (Ascendant).
//! The time-scale conventions themselves are gated separately (`time_scale`).
//!
//! | category   | what is compared                                        | gate |
//! |------------|---------------------------------------------------------|------|
//! | tropical   | apparent tropical longitude (sidereal + *own* true-equinox ayanamsa) at Swiss's TT | 1.0″ (Moon 1.5″); 2.5″ inside the solar-conjunction band |
//! | ayanamsa   | Lahiri under matching convention (true & mean equinox)  | 1.0″ each |
//! | sidereal   | end-to-end sidereal longitude at Swiss's TT             | 2.0″ |
//! | nodes      | Rahu/Ketu sidereal (DE440 osculating node vs Swiss true node) | 5.0″ (`source: de440-osculating`); analytic fallback 120″ |
//! | ascendant  | sidereal Ascendant at Swiss's UT1                       | 0.01° |
//! | time_scale | ΔTT / ΔUT1 between the engines' own instants            | era-dependent, see [`time_scale_gate`] |
//!
//! Sun–Saturn and the Moon must pass tropical AND sidereal; the nodes only the
//! node gate; ayanamsa, Ascendant and time_scale always. Disagreement is
//! surfaced, never averaged; an unreachable sidecar is reported, never faked.
//! The corpus that sizes these numbers lives in validation/consensus.

use serde_json::{Value, json};
use xalen_houses::HouseSystem;

use super::{Ctx, ToolOutput, args};
use crate::engine::{
    BodyId, ENGINE_NAME, Instant, NODE_SOURCE_DE440, UTC_LEAP_SECOND_ERA_START_JD,
};
use crate::sidecar::Sidecar;
use crate::types::{BirthInput, ToolError, parse_utc, signed_delta_deg};

pub const ASC_TOL_DEG: f64 = 0.01;
pub const AYANAMSA_TOL_ARCSEC: f64 = 1.0;
/// End-to-end sidereal tolerance for the physical bodies: the tropical
/// tolerance plus the ayanamsa tolerance (XALEN's Lahiri sits a constant
/// ≈0.73″ below Swiss's; see validation/consensus-summary.md).
pub const SIDEREAL_TOL_ARCSEC: f64 = 2.0;
/// Rahu/Ketu from the osculating node of DE440's own lunar state vector
/// (`source: de440-osculating`) vs Swiss `SE_TRUE_NODE`.
pub const NODE_TOL_ARCSEC: f64 = 5.0;
/// Rahu/Ketu when only XALEN's analytic node is available (tagged fallback):
/// measured 0/10000 over 120″ (p99.9 90.5″, max 105.2″); 120″ is 1 % of a pada.
pub const NODE_TOL_ANALYTIC_ARCSEC: f64 = 120.0;
/// Solar-conjunction band: Swiss applies gravitational light deflection by the
/// Sun (up to ≈1.7″ at the limb); XALEN's DE440 apparent-place chain does not
/// (upstream item, docs/CONTRACT.md). Inside this angular separation from the
/// Sun a planet's per-chart tropical tolerance is [`CONJUNCTION_TROPICAL_TOL_ARCSEC`].
pub const CONJUNCTION_BAND_DEG: f64 = 1.0;
pub const CONJUNCTION_TROPICAL_TOL_ARCSEC: f64 = 2.5;
pub const CONJUNCTION_NOTE: &str =
    "solar conjunction band — gravitational deflection not modelled in DE440 chain";
/// Swiss `swe_utc_to_jd` regime change (bisected against pyswisseph 2.10.03):
/// from 2033-09-17T00:00Z it abandons the leap-second TT for `UTC + ΔT(model)`.
pub const SWISS_EXTRAPOLATION_START_JD: f64 = 2463857.5;
/// `time_scale` gate inside the known-TAI−UTC era (1972-01-01..2033-09-16):
/// both engines must agree on TT to the millisecond and on UT1 to |DUT1|-class.
pub const TT_TOL_KNOWN_ERA_SEC: f64 = 0.01;
pub const UT1_TOL_KNOWN_ERA_SEC: f64 = 1.0;
/// `time_scale` gate before 1972: both engines derive TT from ΔT tables.
pub const TT_TOL_PRE_1972_SEC: f64 = 1.0;

/// Tropical-longitude tolerance (arcsec). `None` = not gated on this axis.
pub fn tropical_tolerance_arcsec(b: BodyId) -> Option<f64> {
    match b {
        BodyId::Moon => Some(1.5),
        BodyId::Rahu | BodyId::Ketu => None,
        BodyId::Uranus | BodyId::Neptune | BodyId::Pluto => Some(3.0),
        _ => Some(1.0),
    }
}

/// Per-chart tropical tolerance given the body's angular separation from the
/// Sun: the conjunction band applies to the planets only (the Moon and the
/// nodes are never behind the Sun). Returns `(tolerance, note)`.
pub fn tropical_tolerance_for(
    b: BodyId,
    elongation_deg: f64,
) -> (Option<f64>, Option<&'static str>) {
    match (b, tropical_tolerance_arcsec(b)) {
        (BodyId::Sun | BodyId::Moon | BodyId::Rahu | BodyId::Ketu, t) => (t, None),
        (_, Some(t)) if elongation_deg <= CONJUNCTION_BAND_DEG => (
            Some(t.max(CONJUNCTION_TROPICAL_TOL_ARCSEC)),
            Some(CONJUNCTION_NOTE),
        ),
        (_, t) => (t, None),
    }
}

/// End-to-end sidereal tolerance (arcsec) for a body from the given source.
pub fn sidereal_tolerance_arcsec_for(b: BodyId, source: &str) -> f64 {
    match b {
        BodyId::Rahu | BodyId::Ketu => {
            if source == NODE_SOURCE_DE440 {
                NODE_TOL_ARCSEC
            } else {
                NODE_TOL_ANALYTIC_ARCSEC
            }
        }
        BodyId::Uranus | BodyId::Neptune | BodyId::Pluto => 3.0 + AYANAMSA_TOL_ARCSEC,
        _ => SIDEREAL_TOL_ARCSEC,
    }
}

/// End-to-end sidereal tolerance (arcsec) with the kernel-backed node.
pub fn sidereal_tolerance_arcsec(b: BodyId) -> f64 {
    sidereal_tolerance_arcsec_for(b, NODE_SOURCE_DE440)
}

/// Backwards-compatible alias: the sidereal (end-to-end) tolerance.
pub fn tolerance_arcsec(b: BodyId) -> f64 {
    sidereal_tolerance_arcsec(b)
}

/// Angular separation on the sphere between two ecliptic places, degrees.
pub fn angular_separation_deg(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let (l1, b1, l2, b2) = (
        lon1.to_radians(),
        lat1.to_radians(),
        lon2.to_radians(),
        lat2.to_radians(),
    );
    let c = b1.sin() * b2.sin() + b1.cos() * b2.cos() * (l1 - l2).cos();
    c.clamp(-1.0, 1.0).acos().to_degrees()
}

/// Which time-scale regime a UTC instant falls in, and the gates that apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeScaleEra {
    /// Before 1972-01-01: no leap-second UTC; both engines use ΔT tables.
    Pre1972,
    /// 1972-01-01..2033-09-16: TAI−UTC known; leap-second-exact TT in both.
    KnownTaiUtc,
    /// From 2033-09-17: Swiss extrapolates ΔT, XALEN assumes no further leap
    /// seconds (CGPM 2022). ΔTT is reported, not gated.
    Extrapolated,
}

impl TimeScaleEra {
    pub fn of_jd_utc(jd_utc: f64) -> Self {
        if jd_utc < UTC_LEAP_SECOND_ERA_START_JD {
            TimeScaleEra::Pre1972
        } else if jd_utc < SWISS_EXTRAPOLATION_START_JD {
            TimeScaleEra::KnownTaiUtc
        } else {
            TimeScaleEra::Extrapolated
        }
    }
    pub fn convention(self) -> &'static str {
        match self {
            TimeScaleEra::Pre1972 => "delta_t_tables",
            TimeScaleEra::KnownTaiUtc => "known_tai_utc",
            TimeScaleEra::Extrapolated => "extrapolated",
        }
    }
    /// `(tt_tol_sec, ut1_tol_sec)`; `None` = reported, not gated.
    pub fn tolerances_sec(self) -> (Option<f64>, Option<f64>) {
        match self {
            TimeScaleEra::Pre1972 => (Some(TT_TOL_PRE_1972_SEC), None),
            TimeScaleEra::KnownTaiUtc => (Some(TT_TOL_KNOWN_ERA_SEC), Some(UT1_TOL_KNOWN_ERA_SEC)),
            TimeScaleEra::Extrapolated => (None, None),
        }
    }
    pub fn note(self) -> &'static str {
        match self {
            TimeScaleEra::Pre1972 => {
                "pre-1972: no leap-second UTC; both engines treat civil time as UT1 and derive TT from ΔT tables (SMH2016 vs Swiss's table, ≤ 0.7 s apart in 1900–1971)"
            }
            TimeScaleEra::KnownTaiUtc => {
                "TAI−UTC known: TT is leap-second exact in both engines; UT1 differs only by the DUT1 model (XALEN: UT1 = UTC; Swiss: UT1 = TT − ΔT(table))"
            }
            TimeScaleEra::Extrapolated => {
                "extrapolated: XALEN assumes no further leap seconds (CGPM 2022 resolution) so TT − UTC stays 69.184 s; Swiss swe_utc_to_jd extrapolates ΔT(model) instead (up to 5.7 s apart by 2050). Longitudes are still compared at the SAME instant (Swiss's jd_tt); only the convention gap is reported here"
            }
        }
    }
}

/// Evaluate the `time_scale` gate. `delta_tt_sec` / `delta_ut1_sec` are
/// XALEN − Swiss for the engines' own instants.
pub fn time_scale_gate(era: TimeScaleEra, delta_tt_sec: f64, delta_ut1_sec: f64) -> (bool, Value) {
    let (tt_tol, ut1_tol) = era.tolerances_sec();
    let tt_pass = tt_tol.map(|t| delta_tt_sec.abs() <= t);
    let ut1_pass = ut1_tol.map(|t| delta_ut1_sec.abs() <= t);
    let pass = tt_pass.unwrap_or(true) && ut1_pass.unwrap_or(true);
    (
        pass,
        json!({
            "convention": era.convention(),
            "delta_tt_sec": delta_tt_sec,
            "delta_ut1_sec": delta_ut1_sec,
            "tt_tolerance_sec": tt_tol,
            "ut1_tolerance_sec": ut1_tol,
            "tt_pass": tt_pass,
            "ut1_pass": ut1_pass,
            "pass": pass,
            "gated": tt_tol.is_some() || ut1_tol.is_some(),
            "note": era.note(),
        }),
    )
}

pub struct ConsensusReport {
    pub status: &'static str,
    pub report: Value,
}

fn num(v: &Value, path: &str) -> Result<f64, ToolError> {
    v.as_f64().ok_or_else(|| {
        ToolError::new(
            "SIDECAR_ERROR",
            format!("jhora-svc /v1/positions: field {path:?} missing or non-numeric"),
        )
    })
}

fn arcsec(a: f64, b: f64) -> f64 {
    signed_delta_deg(a, b).abs() * 3600.0
}

/// Fetch `/v1/positions` from jhora-svc and compare against XALEN evaluated
/// at the sidecar's own instant.
pub async fn compare(ctx: &Ctx, birth: &BirthInput) -> Result<ConsensusReport, ToolError> {
    let fields = parse_utc(&birth.utc)?;
    let own = Instant::from_utc_fields(&fields);
    let engine = &ctx.engine;
    engine.check_coverage(&own)?;

    let body = serde_json::to_value(birth).unwrap_or(Value::Null);
    let resp = ctx
        .sidecars
        .post_json(Sidecar::Jhora, "/v1/positions", &body)
        .await?;

    let jd_ut_j = num(&resp["jd_ut"], "jd_ut")?;
    let jd_tt_j = num(&resp["jd_tt"], "jd_tt")?;
    // The aligned instant: Swiss's UT1 and TT. Everything gated below is
    // evaluated here; the engines' own instants only feed `time_scale`.
    let at = Instant::from_parts(jd_ut_j, jd_tt_j);
    engine.check_coverage(&at)?;

    let mut xalen_bodies = Vec::with_capacity(9);
    for b in BodyId::GRAHAS {
        xalen_bodies.push(engine.body_state(b, &at)?);
    }
    let sun = xalen_bodies[0].clone();
    let xalen_asc = engine
        .houses(&at, birth.lat, birth.lon, HouseSystem::WholeSign)?
        .ascendant_deg;
    let xalen_aya_true = engine.ayanamsa_deg(&at);
    let xalen_aya_mean = engine.ayanamsa_mean_equinox_deg(&at);
    let xalen_dpsi_arcsec = engine.nutation_dpsi_deg(&at) * 3600.0;

    // Ayanamsa under both conventions. `ayanamsa_true_equinox_deg` is the
    // value Swiss actually subtracts to form sidereal longitudes; `ayanamsa_deg`
    // is `swe_get_ayanamsa_ut` (mean equinox, no nutation).
    let jaya_mean = num(&resp["ayanamsa_deg"], "ayanamsa_deg")?;
    let jaya_true = num(
        &resp["ayanamsa_true_equinox_deg"],
        "ayanamsa_true_equinox_deg",
    )?;
    let aya_true_delta = arcsec(xalen_aya_true, jaya_true);
    let aya_mean_delta = arcsec(xalen_aya_mean, jaya_mean);
    let aya_true_pass = aya_true_delta <= AYANAMSA_TOL_ARCSEC;
    let aya_mean_pass = aya_mean_delta <= AYANAMSA_TOL_ARCSEC;
    let aya_pass = aya_true_pass && aya_mean_pass;

    // Time-scale gate on the engines' own instants.
    let delta_tt_sec = (own.jd_tt.0 - jd_tt_j) * 86400.0;
    let delta_ut1_sec = (own.jd_ut1.0 - jd_ut_j) * 86400.0;
    let era = TimeScaleEra::of_jd_utc(own.jd_ut1.0);
    let (ts_pass, ts_report) = time_scale_gate(era, delta_tt_sec, delta_ut1_sec);

    let mut all_pass = aya_pass && ts_pass;
    let mut bodies = serde_json::Map::new();
    let mut worst_tropical: (f64, &str) = (0.0, "");
    let mut worst_sidereal: (f64, &str) = (0.0, "");
    let mut worst_node: (f64, &str) = (0.0, "");
    let mut tropical_all_pass = true;
    let mut node_source = NODE_SOURCE_DE440;
    let mut conjunction_bodies: Vec<&str> = Vec::new();
    for st in &xalen_bodies {
        let name = st.body.name();
        let j = &resp["bodies"][name];
        let j_sid = num(&j["lon"], &format!("bodies.{name}.lon"))?;
        let j_trop = (j_sid + jaya_true).rem_euclid(360.0);

        let trop_delta = arcsec(st.tropical_lon_deg, j_trop);
        let sid_delta = arcsec(st.sidereal_lon_deg, j_sid);
        let sid_tol = sidereal_tolerance_arcsec_for(st.body, st.source);
        let sid_pass = sid_delta <= sid_tol;
        let is_node = matches!(st.body, BodyId::Rahu | BodyId::Ketu);
        if is_node {
            node_source = st.source;
        }
        let elongation = angular_separation_deg(
            st.tropical_lon_deg,
            st.latitude_deg,
            sun.tropical_lon_deg,
            sun.latitude_deg,
        );
        let (trop_tol, trop_note) = tropical_tolerance_for(st.body, elongation);
        if trop_note.is_some() {
            conjunction_bodies.push(name);
        }
        let (trop_tol_json, trop_pass, gated_on): (Value, Option<bool>, Vec<&str>) = match trop_tol
        {
            Some(t) => (
                json!(t),
                Some(trop_delta <= t),
                vec!["tropical", "sidereal"],
            ),
            None => (Value::Null, None, vec!["nodes"]),
        };
        let pass = sid_pass && trop_pass.unwrap_or(true);
        tropical_all_pass &= trop_pass.unwrap_or(true);
        all_pass &= pass;
        if is_node {
            if sid_delta > worst_node.0 {
                worst_node = (sid_delta, name);
            }
        } else {
            if trop_delta > worst_tropical.0 {
                worst_tropical = (trop_delta, name);
            }
            if sid_delta > worst_sidereal.0 {
                worst_sidereal = (sid_delta, name);
            }
        }
        bodies.insert(
            name.to_string(),
            json!({
                "xalen_deg": st.sidereal_lon_deg,
                "jhora_deg": j_sid,
                "xalen_tropical_deg": st.tropical_lon_deg,
                "jhora_tropical_deg": j_trop,
                "elongation_from_sun_deg": elongation,
                "tropical": { "delta_arcsec": trop_delta, "tolerance_arcsec": trop_tol_json, "pass": trop_pass, "note": trop_note },
                "sidereal": { "delta_arcsec": sid_delta, "tolerance_arcsec": sid_tol, "pass": sid_pass },
                "delta_arcsec": sid_delta,
                "tolerance_arcsec": sid_tol,
                "pass": pass,
                "gated_on": gated_on,
                "source": st.source,
                "xalen_retrograde": st.retrograde,
                "jhora_retrograde": j["retro"].as_bool(),
            }),
        );
    }

    let jasc = num(&resp["ascendant"], "ascendant")?;
    let asc_delta = signed_delta_deg(xalen_asc, jasc).abs();
    let asc_pass = asc_delta <= ASC_TOL_DEG;
    all_pass &= asc_pass;

    let node_tol = if node_source == NODE_SOURCE_DE440 {
        NODE_TOL_ARCSEC
    } else {
        NODE_TOL_ANALYTIC_ARCSEC
    };
    let status = if all_pass { "PASS" } else { "FAIL" };
    Ok(ConsensusReport {
        status,
        report: json!({
            "consensus_status": status,
            "principle": "ephemeris agreement is measured at the SAME instant (XALEN evaluated at jhora-svc's jd_tt for longitudes/ayanamsa and jd_ut for the Ascendant); time-scale conventions are gated separately under time_scale",
            "reference": {
                "engine": "jhora-svc (PyJHora / Swiss Ephemeris)",
                "jd_ut": jd_ut_j,
                "jd_tt": jd_tt_j,
                "delta_t_sec": resp["delta_t_sec"],
                "flags": resp["flags"],
            },
            "candidate": {
                "engine": ENGINE_NAME,
                "own_jd_ut1": own.jd_ut1.0,
                "own_jd_tt": own.jd_tt.0,
                "own_delta_t_sec": (own.jd_tt.0 - own.jd_ut1.0) * 86400.0,
                "delta_t_sigma_sec": own.delta_t_sigma_sec,
                "evaluated_at": { "jd_ut1": at.jd_ut1.0, "jd_tt": at.jd_tt.0, "aligned_to": "jhora-svc" },
            },
            "time_scale": ts_report,
            "jd_ut_delta_sec": delta_ut1_sec,
            "jd_tt_delta_sec": delta_tt_sec,
            "categories": {
                "tropical":  { "tolerance_arcsec": 1.0, "moon_tolerance_arcsec": 1.5, "conjunction_band_deg": CONJUNCTION_BAND_DEG, "conjunction_tolerance_arcsec": CONJUNCTION_TROPICAL_TOL_ARCSEC, "bodies_in_conjunction_band": conjunction_bodies, "worst_body": worst_tropical.1, "worst_arcsec": worst_tropical.0, "pass": tropical_all_pass },
                "ayanamsa":  { "tolerance_arcsec": AYANAMSA_TOL_ARCSEC, "true_equinox_arcsec": aya_true_delta, "mean_equinox_arcsec": aya_mean_delta, "pass": aya_pass },
                "sidereal":  { "tolerance_arcsec": SIDEREAL_TOL_ARCSEC, "worst_body": worst_sidereal.1, "worst_arcsec": worst_sidereal.0, "pass": worst_sidereal.0 <= SIDEREAL_TOL_ARCSEC },
                "nodes":     { "tolerance_arcsec": node_tol, "source": node_source, "worst_body": worst_node.1, "worst_arcsec": worst_node.0, "pass": worst_node.0 <= node_tol },
                "ascendant": { "tolerance_deg": ASC_TOL_DEG, "delta_deg": asc_delta, "pass": asc_pass },
                "time_scale": { "convention": era.convention(), "pass": ts_pass },
            },
            "bodies": bodies,
            "ascendant": { "xalen_deg": xalen_asc, "jhora_deg": jasc, "delta_deg": asc_delta, "tolerance_deg": ASC_TOL_DEG, "pass": asc_pass },
            "ayanamsa": {
                "xalen_true_equinox_deg": xalen_aya_true,
                "xalen_mean_equinox_deg": xalen_aya_mean,
                "jhora_true_equinox_deg": jaya_true,
                "jhora_mean_equinox_deg": jaya_mean,
                "jhora_deg": jaya_mean,
                "nutation_dpsi_arcsec": xalen_dpsi_arcsec,
                "jhora_nutation_dpsi_arcsec": resp["nutation_dpsi_arcsec"],
                "true_equinox": { "delta_arcsec": aya_true_delta, "tolerance_arcsec": AYANAMSA_TOL_ARCSEC, "pass": aya_true_pass },
                "mean_equinox": { "delta_arcsec": aya_mean_delta, "tolerance_arcsec": AYANAMSA_TOL_ARCSEC, "pass": aya_mean_pass },
                "delta_arcsec": aya_true_delta,
                "delta_true_equinox_arcsec": aya_true_delta,
                "delta_mean_equinox_arcsec": aya_mean_delta,
                "matched_convention": "both (true_equinox gates sidereal longitudes; mean_equinox = swe_get_ayanamsa_ut)",
                "tolerance_arcsec": AYANAMSA_TOL_ARCSEC,
                "pass": aya_pass
            },
        }),
    })
}

pub async fn run(ctx: &Ctx, args: Value) -> Result<ToolOutput, ToolError> {
    let m = args::obj(&args, "arguments")?;
    let birth = args::birth(m, "birth")?;
    let fields = parse_utc(&birth.utc)?;
    let at = Instant::from_utc_fields(&fields);
    match compare(ctx, &birth).await {
        Ok(r) => Ok(ToolOutput {
            payload: r.report,
            engine: ENGINE_NAME,
            consensus_status: r.status.to_string(),
            delta_t_sigma_sec: Some(at.delta_t_sigma_sec),
            cache_hit: false,
        }),
        Err(e) if e.code == "SIDECAR_UNAVAILABLE" => Ok(ToolOutput {
            payload: json!({
                "consensus_status": "SIDECAR_UNAVAILABLE",
                "reason": e.message,
                "details": e.details,
                "bodies": {},
            }),
            engine: ENGINE_NAME,
            consensus_status: "SIDECAR_UNAVAILABLE".into(),
            delta_t_sigma_sec: Some(at.delta_t_sigma_sec),
            cache_hit: false,
        }),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::NODE_SOURCE_ANALYTIC;

    #[test]
    fn tolerances_match_contract() {
        // tropical
        assert_eq!(tropical_tolerance_arcsec(BodyId::Sun), Some(1.0));
        assert_eq!(tropical_tolerance_arcsec(BodyId::Saturn), Some(1.0));
        assert_eq!(tropical_tolerance_arcsec(BodyId::Moon), Some(1.5));
        assert_eq!(tropical_tolerance_arcsec(BodyId::Neptune), Some(3.0));
        assert_eq!(tropical_tolerance_arcsec(BodyId::Rahu), None);
        assert_eq!(tropical_tolerance_arcsec(BodyId::Ketu), None);
        // sidereal end-to-end
        assert_eq!(sidereal_tolerance_arcsec(BodyId::Sun), 2.0);
        assert_eq!(sidereal_tolerance_arcsec(BodyId::Moon), 2.0);
        assert_eq!(sidereal_tolerance_arcsec(BodyId::Rahu), 5.0);
        assert_eq!(sidereal_tolerance_arcsec(BodyId::Ketu), 5.0);
        assert_eq!(
            sidereal_tolerance_arcsec_for(BodyId::Rahu, NODE_SOURCE_ANALYTIC),
            120.0
        );
        assert_eq!(tolerance_arcsec(BodyId::Saturn), 2.0);
        assert_eq!(ASC_TOL_DEG, 0.01);
        assert_eq!(AYANAMSA_TOL_ARCSEC, 1.0);
        assert_eq!(SIDEREAL_TOL_ARCSEC, 2.0);
        assert_eq!(NODE_TOL_ARCSEC, 5.0);
        assert_eq!(NODE_TOL_ANALYTIC_ARCSEC, 120.0);
    }

    #[test]
    fn conjunction_band_applies_to_planets_only() {
        // Venus 0.11° from the Sun (2024-06-05): 2.5″ with the note.
        let (t, n) = tropical_tolerance_for(BodyId::Venus, 0.11);
        assert_eq!(t, Some(2.5));
        assert_eq!(n, Some(CONJUNCTION_NOTE));
        // exactly on the band edge is inside; just outside is nominal
        assert_eq!(tropical_tolerance_for(BodyId::Mars, 1.0).0, Some(2.5));
        assert_eq!(
            tropical_tolerance_for(BodyId::Mars, 1.0001),
            (Some(1.0), None)
        );
        // the Moon at new moon and the nodes are never widened
        assert_eq!(tropical_tolerance_for(BodyId::Moon, 0.0), (Some(1.5), None));
        assert_eq!(tropical_tolerance_for(BodyId::Rahu, 0.0), (None, None));
        assert_eq!(tropical_tolerance_for(BodyId::Sun, 0.0), (Some(1.0), None));
    }

    #[test]
    fn angular_separation_basic() {
        assert!((angular_separation_deg(10.0, 0.0, 12.0, 0.0) - 2.0).abs() < 1e-9);
        assert!((angular_separation_deg(359.5, 0.0, 0.5, 0.0) - 1.0).abs() < 1e-9);
        assert!((angular_separation_deg(0.0, 0.0, 0.0, 3.0) - 3.0).abs() < 1e-9);
        assert!((angular_separation_deg(0.0, 90.0, 180.0, 90.0)).abs() < 1e-6);
    }

    #[test]
    fn time_scale_eras_and_gates() {
        assert_eq!(TimeScaleEra::of_jd_utc(2441317.4), TimeScaleEra::Pre1972);
        assert_eq!(
            TimeScaleEra::of_jd_utc(2441317.5),
            TimeScaleEra::KnownTaiUtc
        );
        assert_eq!(
            TimeScaleEra::of_jd_utc(2463857.4),
            TimeScaleEra::KnownTaiUtc
        );
        assert_eq!(
            TimeScaleEra::of_jd_utc(2463857.5),
            TimeScaleEra::Extrapolated
        );
        // known era: TT to 10 ms, UT1 to 1 s
        let (p, r) = time_scale_gate(TimeScaleEra::KnownTaiUtc, 0.0, 0.2);
        assert!(p && r["gated"] == true && r["convention"] == "known_tai_utc");
        assert!(!time_scale_gate(TimeScaleEra::KnownTaiUtc, 0.02, 0.0).0);
        assert!(!time_scale_gate(TimeScaleEra::KnownTaiUtc, 0.0, 1.5).0);
        // pre-1972: TT to 1 s (13 s — the old floor bug — must fail), UT1 not gated
        assert!(time_scale_gate(TimeScaleEra::Pre1972, 0.7, 0.0).0);
        assert!(!time_scale_gate(TimeScaleEra::Pre1972, 13.08, 0.0).0);
        let (_, r) = time_scale_gate(TimeScaleEra::Pre1972, 0.0, 0.0);
        assert!(r["ut1_pass"].is_null());
        // extrapolated: reported, never gated
        let (p, r) = time_scale_gate(TimeScaleEra::Extrapolated, -5.67, 0.0);
        assert!(p && r["gated"] == false && r["convention"] == "extrapolated");
    }

    #[test]
    fn arcsec_is_wrap_safe() {
        assert!((arcsec(359.9999, 0.0001) - 0.72).abs() < 1e-6);
        assert!((arcsec(0.0001, 359.9999) - 0.72).abs() < 1e-6);
        assert_eq!(arcsec(10.0, 10.0), 0.0);
    }
}

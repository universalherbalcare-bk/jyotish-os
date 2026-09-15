//! `engine.consensus` — XALEN-DE440 vs jhora-svc (Swiss) differential with
//! the docs/CONTRACT.md tolerances, decomposed so that a disagreement can be
//! attributed to its cause instead of being smeared into one number:
//!
//! | category  | what is compared                                   | gate            |
//! |-----------|----------------------------------------------------|-----------------|
//! | tropical  | apparent tropical longitude (sidereal + *own* ayanamsa, true-equinox) | 1.0″ (Moon 1.5″) |
//! | ayanamsa  | Lahiri under matching convention (true & mean equinox) | 1.0″ each     |
//! | sidereal  | end-to-end sidereal longitude                      | 2.0″            |
//! | nodes     | Rahu/Ketu sidereal (analytic osculating node vs Swiss) | 60″ (source: analytic) |
//! | ascendant | sidereal Ascendant                                  | 0.01°           |
//!
//! Sun–Saturn and the Moon must pass tropical AND sidereal; the nodes only the
//! node gate; ayanamsa and Ascendant always. Disagreement is surfaced, never
//! averaged; an unreachable sidecar is reported, never faked. The corpus that
//! sizes these numbers lives in validation/consensus (validation/README.md).

use serde_json::{Value, json};
use xalen_houses::HouseSystem;

use super::{Ctx, ToolOutput, args};
use crate::engine::{BodyId, ENGINE_NAME, Instant};
use crate::sidecar::Sidecar;
use crate::types::{BirthInput, ToolError, parse_utc, signed_delta_deg};

pub const ASC_TOL_DEG: f64 = 0.01;
pub const AYANAMSA_TOL_ARCSEC: f64 = 1.0;
/// End-to-end sidereal tolerance for the physical bodies: the tropical
/// tolerance plus the ayanamsa tolerance (XALEN's Lahiri sits a constant
/// ≈0.73″ below Swiss's; see validation/consensus-summary.md).
pub const SIDEREAL_TOL_ARCSEC: f64 = 2.0;
/// Rahu/Ketu: XALEN's osculating node is analytic (finite-difference on the
/// analytic Moon), Swiss's is from the ephemeris state vector.
pub const NODE_TOL_ARCSEC: f64 = 60.0;

/// Tropical-longitude tolerance (arcsec). `None` = not gated on this axis.
pub fn tropical_tolerance_arcsec(b: BodyId) -> Option<f64> {
    match b {
        BodyId::Moon => Some(1.5),
        BodyId::Rahu | BodyId::Ketu => None,
        BodyId::Uranus | BodyId::Neptune | BodyId::Pluto => Some(3.0),
        _ => Some(1.0),
    }
}

/// End-to-end sidereal tolerance (arcsec).
pub fn sidereal_tolerance_arcsec(b: BodyId) -> f64 {
    match b {
        BodyId::Rahu | BodyId::Ketu => NODE_TOL_ARCSEC,
        BodyId::Uranus | BodyId::Neptune | BodyId::Pluto => 3.0 + AYANAMSA_TOL_ARCSEC,
        _ => SIDEREAL_TOL_ARCSEC,
    }
}

/// Backwards-compatible alias: the sidereal (end-to-end) tolerance.
pub fn tolerance_arcsec(b: BodyId) -> f64 {
    sidereal_tolerance_arcsec(b)
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

/// Fetch `/v1/positions` from jhora-svc and compare against XALEN.
pub async fn compare(ctx: &Ctx, birth: &BirthInput) -> Result<ConsensusReport, ToolError> {
    let fields = parse_utc(&birth.utc)?;
    let at = Instant::from_utc_fields(&fields);
    let engine = &ctx.engine;

    let mut xalen_bodies = Vec::with_capacity(9);
    for b in BodyId::GRAHAS {
        xalen_bodies.push(engine.body_state(b, &at)?);
    }
    let xalen_asc = engine
        .houses(&at, birth.lat, birth.lon, HouseSystem::WholeSign)?
        .ascendant_deg;
    let xalen_aya_true = engine.ayanamsa_deg(&at);
    let xalen_aya_mean = engine.ayanamsa_mean_equinox_deg(&at);
    let xalen_dpsi_arcsec = engine.nutation_dpsi_deg(&at) * 3600.0;

    let body = serde_json::to_value(birth).unwrap_or(Value::Null);
    let resp = ctx
        .sidecars
        .post_json(Sidecar::Jhora, "/v1/positions", &body)
        .await?;

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

    let mut all_pass = aya_pass;
    let mut bodies = serde_json::Map::new();
    let mut worst_tropical: (f64, &str) = (0.0, "");
    let mut worst_sidereal: (f64, &str) = (0.0, "");
    let mut worst_node: (f64, &str) = (0.0, "");
    let mut tropical_all_pass = true;
    for st in &xalen_bodies {
        let name = st.body.name();
        let j = &resp["bodies"][name];
        let j_sid = num(&j["lon"], &format!("bodies.{name}.lon"))?;
        let j_trop = (j_sid + jaya_true).rem_euclid(360.0);

        let trop_delta = arcsec(st.tropical_lon_deg, j_trop);
        let sid_delta = arcsec(st.sidereal_lon_deg, j_sid);
        let sid_tol = sidereal_tolerance_arcsec(st.body);
        let sid_pass = sid_delta <= sid_tol;
        let is_node = matches!(st.body, BodyId::Rahu | BodyId::Ketu);
        let (trop_tol, trop_pass, gated_on): (Value, Option<bool>, Vec<&str>) =
            match tropical_tolerance_arcsec(st.body) {
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
                "tropical": { "delta_arcsec": trop_delta, "tolerance_arcsec": trop_tol, "pass": trop_pass },
                "sidereal": { "delta_arcsec": sid_delta, "tolerance_arcsec": sid_tol, "pass": sid_pass },
                "delta_arcsec": sid_delta,
                "tolerance_arcsec": sid_tol,
                "pass": pass,
                "gated_on": gated_on,
                "source": if is_node { "analytic" } else { "jpl-de440" },
                "xalen_source": st.body.source(),
                "xalen_retrograde": st.retrograde,
                "jhora_retrograde": j["retro"].as_bool(),
            }),
        );
    }

    let jasc = num(&resp["ascendant"], "ascendant")?;
    let asc_delta = signed_delta_deg(xalen_asc, jasc).abs();
    let asc_pass = asc_delta <= ASC_TOL_DEG;
    all_pass &= asc_pass;

    let jd_ut_delta_sec = resp["jd_ut"].as_f64().map(|j| (j - at.jd_ut1.0) * 86400.0);
    let jd_tt_delta_sec = resp["jd_tt"].as_f64().map(|j| (j - at.jd_tt.0) * 86400.0);

    let status = if all_pass { "PASS" } else { "FAIL" };
    Ok(ConsensusReport {
        status,
        report: json!({
            "consensus_status": status,
            "reference": {
                "engine": "jhora-svc (PyJHora / Swiss Ephemeris)",
                "jd_ut": resp["jd_ut"],
                "jd_tt": resp["jd_tt"],
                "delta_t_sec": resp["delta_t_sec"],
                "flags": resp["flags"],
            },
            "candidate": {
                "engine": ENGINE_NAME,
                "jd_ut1": at.jd_ut1.0,
                "jd_tt": at.jd_tt.0,
                "delta_t_sec": (at.jd_tt.0 - at.jd_ut1.0) * 86400.0,
                "delta_t_sigma_sec": at.delta_t_sigma_sec,
            },
            "time_scale": {
                "jd_ut_delta_sec": jd_ut_delta_sec,
                "jd_tt_delta_sec": jd_tt_delta_sec,
                "note": "jd_ut: XALEN UT1≈UTC vs Swiss UT1=TT−ΔT(table) (|Δ|≤0.6 s, Ascendant ≤0.003°). jd_tt: identical (leap-second exact) 1972-01-01..2033-09-16; before 1972 both use ΔT models (≤0.7 s apart); from 2033-09-17 Swiss switches to UTC+ΔT(model) while XALEN keeps leap-second TT (convention gap, see docs/CONTRACT.md).",
            },
            "jd_ut_delta_sec": jd_ut_delta_sec,
            "categories": {
                "tropical":  { "tolerance_arcsec": 1.0, "moon_tolerance_arcsec": 1.5, "worst_body": worst_tropical.1, "worst_arcsec": worst_tropical.0, "pass": tropical_all_pass },
                "ayanamsa":  { "tolerance_arcsec": AYANAMSA_TOL_ARCSEC, "true_equinox_arcsec": aya_true_delta, "mean_equinox_arcsec": aya_mean_delta, "pass": aya_pass },
                "sidereal":  { "tolerance_arcsec": SIDEREAL_TOL_ARCSEC, "worst_body": worst_sidereal.1, "worst_arcsec": worst_sidereal.0, "pass": worst_sidereal.0 <= SIDEREAL_TOL_ARCSEC },
                "nodes":     { "tolerance_arcsec": NODE_TOL_ARCSEC, "source": "analytic", "worst_body": worst_node.1, "worst_arcsec": worst_node.0, "pass": worst_node.0 <= NODE_TOL_ARCSEC },
                "ascendant": { "tolerance_deg": ASC_TOL_DEG, "delta_deg": asc_delta, "pass": asc_pass },
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
        assert_eq!(sidereal_tolerance_arcsec(BodyId::Rahu), 60.0);
        assert_eq!(sidereal_tolerance_arcsec(BodyId::Ketu), 60.0);
        assert_eq!(tolerance_arcsec(BodyId::Saturn), 2.0);
        assert_eq!(ASC_TOL_DEG, 0.01);
        assert_eq!(AYANAMSA_TOL_ARCSEC, 1.0);
        assert_eq!(SIDEREAL_TOL_ARCSEC, 2.0);
        assert_eq!(NODE_TOL_ARCSEC, 60.0);
    }

    #[test]
    fn arcsec_is_wrap_safe() {
        assert!((arcsec(359.9999, 0.0001) - 0.72).abs() < 1e-6);
        assert!((arcsec(0.0001, 359.9999) - 0.72).abs() < 1e-6);
        assert_eq!(arcsec(10.0, 10.0), 0.0);
    }
}

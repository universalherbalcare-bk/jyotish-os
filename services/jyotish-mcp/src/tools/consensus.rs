//! `engine.consensus` — XALEN-DE440 vs jhora-svc (Swiss) per-body differential
//! with the docs/CONTRACT.md tolerances. Disagreement is surfaced, never
//! averaged; an unreachable sidecar is reported, never faked.

use serde_json::{Value, json};
use xalen_houses::HouseSystem;
use xalen_time::JulianDay;

use super::{Ctx, ToolOutput, args};
use crate::engine::{BodyId, ENGINE_NAME, Instant};
use crate::sidecar::Sidecar;
use crate::types::{BirthInput, ToolError, parse_utc, signed_delta_deg};

pub const ASC_TOL_DEG: f64 = 0.01;
pub const AYANAMSA_TOL_ARCSEC: f64 = 1.0;

/// Consensus tolerance in arcseconds (docs/CONTRACT.md).
pub fn tolerance_arcsec(b: BodyId) -> f64 {
    match b {
        BodyId::Moon => 1.5,
        BodyId::Rahu | BodyId::Ketu => 5.0,
        BodyId::Uranus | BodyId::Neptune | BodyId::Pluto => 3.0,
        _ => 1.0,
    }
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
    let xalen_aya = engine.ayanamsa_deg(&at);

    let body = serde_json::to_value(birth).unwrap_or(Value::Null);
    let resp = ctx
        .sidecars
        .post_json(Sidecar::Jhora, "/v1/positions", &body)
        .await?;

    let mut all_pass = true;
    let mut bodies = serde_json::Map::new();
    for st in &xalen_bodies {
        let name = st.body.name();
        let j = &resp["bodies"][name];
        let jl = num(&j["lon"], &format!("bodies.{name}.lon"))?;
        let delta = signed_delta_deg(st.sidereal_lon_deg, jl).abs() * 3600.0;
        let tol = tolerance_arcsec(st.body);
        let pass = delta <= tol;
        all_pass &= pass;
        bodies.insert(
            name.to_string(),
            json!({
                "xalen_deg": st.sidereal_lon_deg,
                "jhora_deg": jl,
                "delta_arcsec": delta,
                "tolerance_arcsec": tol,
                "pass": pass,
                "xalen_retrograde": st.retrograde,
                "jhora_retrograde": j["retro"].as_bool(),
            }),
        );
    }
    let jasc = num(&resp["ascendant"], "ascendant")?;
    let asc_delta = signed_delta_deg(xalen_asc, jasc).abs();
    let asc_pass = asc_delta <= ASC_TOL_DEG;
    all_pass &= asc_pass;
    // Ayanamsa: Swiss `swe_get_ayanamsa_ut` returns the MEAN-equinox value
    // (no nutation) while XALEN's `compute_deg` returns the TRUE-equinox
    // (with-nutation) value; both are legitimate definitions and yield the
    // same sidereal longitudes. Compare under both conventions and report
    // which one matched, rather than failing on a Δψ (≈ ±17″) convention gap.
    let jaya = num(&resp["ayanamsa_deg"], "ayanamsa_deg")?;
    let dpsi_deg = xalen_coords::nutation_2000b(at.jd_tt.julian_centuries_from_j2000())
        .delta_psi
        .to_degrees();
    let xalen_aya_mean = xalen_aya - dpsi_deg;
    let aya_delta_true = (xalen_aya - jaya).abs() * 3600.0;
    let aya_delta_mean = (xalen_aya_mean - jaya).abs() * 3600.0;
    let (aya_delta, aya_convention) = if aya_delta_true <= aya_delta_mean {
        (aya_delta_true, "true_equinox (with nutation)")
    } else {
        (aya_delta_mean, "mean_equinox (no nutation)")
    };
    let aya_pass = aya_delta <= AYANAMSA_TOL_ARCSEC;
    all_pass &= aya_pass;
    let jd_ut_delta_sec = resp["jd_ut"].as_f64().map(|j| (j - at.jd_ut1.0) * 86400.0);

    let status = if all_pass { "PASS" } else { "FAIL" };
    Ok(ConsensusReport {
        status,
        report: json!({
            "consensus_status": status,
            "reference": { "engine": "jhora-svc (PyJHora / Swiss Ephemeris)", "jd_ut": resp["jd_ut"], "flags": resp["flags"] },
            "jd_ut_delta_sec": jd_ut_delta_sec,
            "candidate": { "engine": ENGINE_NAME, "jd_ut1": at.jd_ut1.0, "jd_tt": at.jd_tt.0 },
            "bodies": bodies,
            "ascendant": { "xalen_deg": xalen_asc, "jhora_deg": jasc, "delta_deg": asc_delta, "tolerance_deg": ASC_TOL_DEG, "pass": asc_pass },
            "ayanamsa": {
                "xalen_true_equinox_deg": xalen_aya,
                "xalen_mean_equinox_deg": xalen_aya_mean,
                "nutation_dpsi_arcsec": dpsi_deg * 3600.0,
                "jhora_deg": jaya,
                "delta_arcsec": aya_delta,
                "matched_convention": aya_convention,
                "delta_true_equinox_arcsec": aya_delta_true,
                "delta_mean_equinox_arcsec": aya_delta_mean,
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
        assert_eq!(tolerance_arcsec(BodyId::Sun), 1.0);
        assert_eq!(tolerance_arcsec(BodyId::Saturn), 1.0);
        assert_eq!(tolerance_arcsec(BodyId::Moon), 1.5);
        assert_eq!(tolerance_arcsec(BodyId::Rahu), 5.0);
        assert_eq!(tolerance_arcsec(BodyId::Neptune), 3.0);
        assert_eq!(ASC_TOL_DEG, 0.01);
        assert_eq!(AYANAMSA_TOL_ARCSEC, 1.0);
    }
}

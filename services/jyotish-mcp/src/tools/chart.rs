//! `chart.compute` — sidereal positions, speeds, retrograde flags, rashi /
//! nakshatra / pada with `boundary_distance_sec`, Ascendant, Whole-Sign and
//! Sripati cusps, and the requested varga.

use serde_json::{Value, json};
use xalen_houses::HouseSystem;
use xalen_vedic::divisional::compute_varga_sign;

use super::{Ctx, ToolOutput, args, consensus};
use crate::engine::{
    AYANAMSA_NAME, BodyId, ENGINE_NAME, Engine, Instant, boundary_distance, placement,
};
use crate::types::{BirthInput, ToolError, Varga, jd_to_iso_local, jd_to_iso_utc, parse_utc};

fn varga_from_args(m: &serde_json::Map<String, Value>) -> Result<Varga, ToolError> {
    match m.get("varga") {
        None | Some(Value::Null) => Ok(Varga::D1),
        Some(v) => serde_json::from_value::<Varga>(v.clone()).map_err(|_| {
            ToolError::invalid(format!(
                "varga must be one of {:?}",
                Varga::ALL.iter().map(|v| v.code()).collect::<Vec<_>>()
            ))
        }),
    }
}

fn body_json(engine: &Engine, id: BodyId, at: &Instant, varga: Varga) -> Result<Value, ToolError> {
    let st = engine.body_state(id, at)?;
    let p = placement(st.sidereal_lon_deg);
    let b = boundary_distance(st.sidereal_lon_deg, st.speed_deg_per_day);
    let vr = compute_varga_sign(st.sidereal_lon_deg, varga.to_xalen());
    Ok(json!({
        "sidereal_lon_deg": st.sidereal_lon_deg,
        "tropical_lon_deg": st.tropical_lon_deg,
        "latitude_deg": st.latitude_deg,
        "distance_au": st.distance_au,
        "speed_deg_per_day": st.speed_deg_per_day,
        "retrograde": st.retrograde,
        "rashi": p.rashi,
        "rashi_index": p.rashi_index,
        "rashi_lord": p.rashi_lord,
        "degree_in_rashi": p.degree_in_rashi,
        "nakshatra": p.nakshatra,
        "nakshatra_index": p.nakshatra_index,
        "nakshatra_lord": p.nakshatra_lord,
        "pada": p.pada,
        "boundary_distance_sec": b,
        "varga": { "code": varga.code(), "rashi": format!("{vr:?}"), "rashi_index": vr.index() },
        "source": st.source,
    }))
}

/// Pure (cacheable) chart computation.
pub fn compute(engine: &Engine, birth: &BirthInput, varga: Varga) -> Result<Value, ToolError> {
    let fields = parse_utc(&birth.utc)?;
    let at = Instant::from_utc_fields(&fields);
    engine.check_coverage(&at)?;

    let mut bodies = serde_json::Map::new();
    let mut unavailable = serde_json::Map::new();
    for id in BodyId::GRAHAS {
        bodies.insert(id.name().into(), body_json(engine, id, &at, varga)?);
    }
    for id in BodyId::OUTER {
        match body_json(engine, id, &at, varga) {
            Ok(v) => {
                bodies.insert(id.name().into(), v);
            }
            Err(e) => {
                bodies.insert(id.name().into(), Value::Null);
                unavailable.insert(id.name().into(), json!(e.message));
            }
        }
    }

    let whole = engine.houses(&at, birth.lat, birth.lon, HouseSystem::WholeSign)?;
    let sripati = engine.houses(&at, birth.lat, birth.lon, HouseSystem::Sripati)?;
    let asc_rate = engine.ascendant_rate_deg_per_day(&at, birth.lat, birth.lon)?;
    let asc_p = placement(whole.ascendant_deg);
    let asc_b = boundary_distance(whole.ascendant_deg, asc_rate);
    let asc_v = compute_varga_sign(whole.ascendant_deg, varga.to_xalen());

    Ok(json!({
        "input": birth,
        "local_time": {
            "utc": jd_to_iso_utc(at.jd_ut1.0),
            "local": jd_to_iso_local(at.jd_ut1.0, birth.tz_offset_hours),
        },
        "jd_ut1": at.jd_ut1.0,
        "jd_tt": at.jd_tt.0,
        "ayanamsa": AYANAMSA_NAME,
        "ayanamsa_deg": engine.ayanamsa_deg(&at),
        "nodes": "TRUE",
        "varga": { "code": varga.code(), "name": varga.name() },
        "bodies": bodies,
        "unavailable": unavailable,
        "ascendant": {
            "sidereal_lon_deg": whole.ascendant_deg,
            "speed_deg_per_day": asc_rate,
            "rashi": asc_p.rashi,
            "rashi_index": asc_p.rashi_index,
            "rashi_lord": asc_p.rashi_lord,
            "degree_in_rashi": asc_p.degree_in_rashi,
            "nakshatra": asc_p.nakshatra,
            "nakshatra_index": asc_p.nakshatra_index,
            "nakshatra_lord": asc_p.nakshatra_lord,
            "pada": asc_p.pada,
            "boundary_distance_sec": asc_b,
            "varga": { "code": varga.code(), "rashi": format!("{asc_v:?}"), "rashi_index": asc_v.index() },
            "mc_sidereal_lon_deg": whole.mc_deg,
        },
        "houses": {
            "WholeSign": { "cusps_deg": whole.cusps, "ascendant_deg": whole.ascendant_deg, "mc_deg": whole.mc_deg, "fallback_used": whole.fallback_used },
            "Sripati": { "cusps_deg": sripati.cusps, "ascendant_deg": sripati.ascendant_deg, "mc_deg": sripati.mc_deg, "fallback_used": sripati.fallback_used },
        },
        "notes": [
            "Longitudes are sidereal (Lahiri) degrees in [0,360). speed_deg_per_day is the sidereal rate (tropical rate minus ayanamsa rate).",
            "boundary_distance_sec is a linear extrapolation from the current speed; near a station it is unreliable and a retrograde body counts backward.",
            "UT1 is approximated by UTC (|DUT1| < 0.9 s); TT is leap-second exact.",
        ]
    }))
}

pub async fn run(ctx: &Ctx, args: Value) -> Result<ToolOutput, ToolError> {
    let m = args::obj(&args, "arguments")?;
    // Accept either {birth: BirthInput, varga} or a flat BirthInput + varga.
    let birth = if m.contains_key("birth") {
        args::birth(m, "birth")?
    } else {
        let b: BirthInput = serde_json::from_value(args.clone())
            .map_err(|e| ToolError::invalid(format!("BirthInput: {e}")))?;
        b.validate()?;
        b
    };
    let varga = varga_from_args(m)?;
    let canonical = json!({ "birth": birth, "varga": varga.code() });
    let engine = ctx.engine.clone();
    let (payload, cache_hit) = ctx.cached("chart.compute", &canonical, || {
        compute(&engine, &birth, varga)
    })?;
    let sigma = payload["jd_ut1"]
        .as_f64()
        .map(|jd| Instant::from_jd_ut1(jd).delta_t_sigma_sec);

    // Cross-engine gate: when jhora-svc is reachable, any out-of-tolerance
    // body turns this call into an error — never a number (docs/CONTRACT.md).
    let consensus_status = if ctx.sidecars.jhora_is_up().await {
        match consensus::compare(ctx, &birth).await {
            Ok(r) if r.status == "PASS" => "PASS".to_string(),
            Ok(r) => {
                return Err(ToolError::new(
                    "CONSENSUS_FAIL",
                    "XALEN-DE440 and jhora-svc disagree beyond docs/CONTRACT.md tolerances; refusing to return positions",
                )
                .with_details(r.report));
            }
            Err(e) if e.code == "SIDECAR_UNAVAILABLE" => "SIDECAR_UNAVAILABLE".to_string(),
            Err(e) => return Err(e),
        }
    } else {
        "SIDECAR_UNAVAILABLE".to_string()
    };

    Ok(ToolOutput {
        payload,
        engine: ENGINE_NAME,
        consensus_status,
        delta_t_sigma_sec: sigma,
        cache_hit,
    })
}

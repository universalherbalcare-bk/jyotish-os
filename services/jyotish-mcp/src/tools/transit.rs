//! `transit.window` — sweep bodies over [from, to]; bisection to the exact
//! sidereal rashi ingress instants and exact conjunctions with natal points.

use serde_json::{Value, json};
use xalen_houses::HouseSystem;

use super::{Ctx, ToolOutput, args};
use crate::engine::{AYANAMSA_NAME, BodyId, ENGINE_NAME, Engine, Instant, placement};
use crate::types::{BirthInput, ToolError, jd_to_iso_utc, parse_utc, signed_delta_deg};

const MAX_SPAN_DAYS: f64 = 3660.0; // 10 years
const MIN_STEP_HOURS: f64 = 0.25;
const MAX_STEP_HOURS: f64 = 24.0;
const BISECT_ITERS: u32 = 50;
const MAX_EVENTS: usize = 20_000;

struct NatalPoint {
    name: String,
    lon: f64,
}

fn parse_natal_points(
    engine: &Engine,
    birth: Option<&BirthInput>,
    raw: Option<&Value>,
) -> Result<Vec<NatalPoint>, ToolError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let arr = raw
        .as_array()
        .ok_or_else(|| ToolError::invalid("natal_points must be an array"))?;
    if arr.len() > 32 {
        return Err(ToolError::invalid("natal_points: at most 32 entries"));
    }
    let natal_at = match birth {
        Some(b) => Some((Instant::from_utc_fields(&parse_utc(&b.utc)?), b.lat, b.lon)),
        None => None,
    };
    let mut out = Vec::new();
    for item in arr {
        match item {
            Value::String(name) => {
                let (at, lat, lon) = natal_at.ok_or_else(|| {
                    ToolError::invalid(format!("natal point {name:?} by name requires `birth`"))
                })?;
                let lon_deg = if name.eq_ignore_ascii_case("Ascendant")
                    || name.eq_ignore_ascii_case("Lagna")
                {
                    engine
                        .houses(&at, lat, lon, HouseSystem::WholeSign)?
                        .ascendant_deg
                } else {
                    let id = BodyId::parse(name).ok_or_else(|| {
                        ToolError::invalid(format!("unknown natal point {name:?}"))
                    })?;
                    engine.sidereal_lon(id, &at)?
                };
                out.push(NatalPoint {
                    name: name.clone(),
                    lon: lon_deg,
                });
            }
            Value::Object(o) => {
                let name = o
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("custom")
                    .to_string();
                let lon_deg = o
                    .get("sidereal_lon_deg")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        ToolError::invalid("natal point object needs numeric sidereal_lon_deg")
                    })?;
                if !lon_deg.is_finite() {
                    return Err(ToolError::invalid("sidereal_lon_deg must be finite"));
                }
                out.push(NatalPoint {
                    name,
                    lon: lon_deg.rem_euclid(360.0),
                });
            }
            _ => {
                return Err(ToolError::invalid(
                    "natal_points entries must be strings or objects",
                ));
            }
        }
    }
    Ok(out)
}

/// Bisect `f(t) = signed_delta(L(t), target)` on [lo, hi] where the sign
/// differs at the endpoints. Works for direct and retrograde motion and
/// across the 0°/360° wrap because the delta is taken modulo 360.
fn bisect_crossing<F: Fn(f64) -> Result<f64, ToolError>>(
    f: &F,
    target: f64,
    mut lo: f64,
    mut hi: f64,
) -> Result<f64, ToolError> {
    let mut flo = signed_delta_deg(f(lo)?, target);
    for _ in 0..BISECT_ITERS {
        let mid = 0.5 * (lo + hi);
        let fmid = signed_delta_deg(f(mid)?, target);
        if (flo < 0.0) == (fmid < 0.0) {
            lo = mid;
            flo = fmid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-9 {
            break;
        }
    }
    Ok(0.5 * (lo + hi))
}

pub fn compute(
    engine: &Engine,
    birth: Option<&BirthInput>,
    from: &str,
    to: &str,
    bodies: &[BodyId],
    natal_raw: Option<&Value>,
    step_hours: f64,
) -> Result<Value, ToolError> {
    let jd_from = Instant::from_utc_fields(&parse_utc(from)?).jd_ut1.0;
    let jd_to = Instant::from_utc_fields(&parse_utc(to)?).jd_ut1.0;
    if jd_to <= jd_from {
        return Err(ToolError::invalid("`to` must be after `from`"));
    }
    if jd_to - jd_from > MAX_SPAN_DAYS {
        return Err(ToolError::invalid(format!(
            "window exceeds {MAX_SPAN_DAYS} days"
        )));
    }
    if !(MIN_STEP_HOURS..=MAX_STEP_HOURS).contains(&step_hours) {
        return Err(ToolError::invalid(format!(
            "step_hours must be within [{MIN_STEP_HOURS}, {MAX_STEP_HOURS}]"
        )));
    }
    engine.check_coverage(&Instant::from_jd_ut1(jd_from))?;
    engine.check_coverage(&Instant::from_jd_ut1(jd_to))?;
    let natal = parse_natal_points(engine, birth, natal_raw)?;
    let step = step_hours / 24.0;

    let mut events: Vec<Value> = Vec::new();
    for &body in bodies {
        let lon_at = |jd: f64| -> Result<f64, ToolError> {
            Ok(engine.sidereal_lon(body, &Instant::from_jd_ut1(jd))?)
        };
        let mut t = jd_from;
        let mut prev = lon_at(t)?;
        while t < jd_to {
            let next_t = (t + step).min(jd_to);
            let cur = lon_at(next_t)?;
            let prev_idx = (prev.rem_euclid(360.0) / 30.0) as usize % 12;
            let cur_idx = (cur.rem_euclid(360.0) / 30.0) as usize % 12;
            if prev_idx != cur_idx {
                let forward = signed_delta_deg(cur, prev) > 0.0;
                let boundary = if forward {
                    cur_idx as f64 * 30.0
                } else {
                    prev_idx as f64 * 30.0
                };
                let jd = bisect_crossing(&lon_at, boundary, t, next_t)?;
                events.push(json!({
                    "type": "ingress",
                    "body": body.name(),
                    "jd_ut1": jd,
                    "utc": jd_to_iso_utc(jd),
                    "from_rashi": placement(prev).rashi,
                    "to_rashi": placement(cur).rashi,
                    "direction": if forward { "forward" } else { "retrograde" },
                    "sidereal_lon_deg": boundary,
                }));
            }
            for np in &natal {
                let d0 = signed_delta_deg(prev, np.lon);
                let d1 = signed_delta_deg(cur, np.lon);
                // Sign change with both deltas on the near side (< 90°) so the
                // antipodal wrap of signed_delta is never mistaken for a crossing.
                if (d0 < 0.0) != (d1 < 0.0) && d0.abs() < 90.0 && d1.abs() < 90.0 {
                    let jd = bisect_crossing(&lon_at, np.lon, t, next_t)?;
                    events.push(json!({
                        "type": "conjunction",
                        "body": body.name(),
                        "natal_point": np.name,
                        "natal_sidereal_lon_deg": np.lon,
                        "jd_ut1": jd,
                        "utc": jd_to_iso_utc(jd),
                        "direction": if d1 > d0 { "forward" } else { "retrograde" },
                    }));
                }
            }
            if events.len() > MAX_EVENTS {
                return Err(ToolError::new(
                    "TOO_MANY_EVENTS",
                    format!("more than {MAX_EVENTS} events; narrow the window"),
                ));
            }
            prev = cur;
            t = next_t;
        }
    }
    events.sort_by(|a, b| {
        a["jd_ut1"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&b["jd_ut1"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(json!({
        "from": from,
        "to": to,
        "bodies": bodies.iter().map(|b| b.name()).collect::<Vec<_>>(),
        "natal_points": natal.iter().map(|n| json!({"name": n.name, "sidereal_lon_deg": n.lon})).collect::<Vec<_>>(),
        "step_hours": step_hours,
        "ayanamsa": AYANAMSA_NAME,
        "event_count": events.len(),
        "events": events,
        "notes": [
            "Ingress = sidereal rashi boundary crossing; conjunction = exact same sidereal longitude as the natal point (0° orb).",
            "Instants are refined by bisection to < 1e-9 day; a station within one sampling step that reverses across a boundary and back could be missed — reduce step_hours if that matters."
        ]
    }))
}

pub fn run(ctx: &Ctx, args: Value) -> Result<ToolOutput, ToolError> {
    let m = args::obj(&args, "arguments")?;
    let birth = if m.contains_key("birth") && !m["birth"].is_null() {
        Some(args::birth(m, "birth")?)
    } else {
        None
    };
    let from = args::str_req(m, "from")?.to_string();
    let to = args::str_req(m, "to")?.to_string();
    let step_hours = args::f64_opt(m, "step_hours", 6.0)?;
    let bodies: Vec<BodyId> = match m.get("bodies") {
        None | Some(Value::Null) => BodyId::GRAHAS.to_vec(),
        Some(v) => {
            let arr = v
                .as_array()
                .ok_or_else(|| ToolError::invalid("bodies must be an array"))?;
            if arr.is_empty() {
                return Err(ToolError::invalid("bodies must not be empty"));
            }
            let mut out = Vec::new();
            for b in arr {
                let s = b
                    .as_str()
                    .ok_or_else(|| ToolError::invalid("bodies entries must be strings"))?;
                let id = BodyId::parse(s)
                    .ok_or_else(|| ToolError::invalid(format!("unknown body {s:?}")))?;
                if !out.contains(&id) {
                    out.push(id);
                }
            }
            out
        }
    };
    let natal_raw = m.get("natal_points").cloned();
    let canonical = json!({
        "birth": birth, "from": from, "to": to, "step_hours": step_hours,
        "bodies": bodies.iter().map(|b| b.name()).collect::<Vec<_>>(),
        "natal_points": natal_raw,
    });
    let engine = ctx.engine.clone();
    let (payload, cache_hit) = ctx.cached("transit.window", &canonical, || {
        compute(
            &engine,
            birth.as_ref(),
            &from,
            &to,
            &bodies,
            natal_raw.as_ref(),
            step_hours,
        )
    })?;
    let sigma = Some(Instant::from_utc_fields(&parse_utc(&from)?).delta_t_sigma_sec);
    Ok(ToolOutput {
        payload,
        engine: ENGINE_NAME,
        consensus_status: "NOT_CHECKED".into(),
        delta_t_sigma_sec: sigma,
        cache_hit,
    })
}

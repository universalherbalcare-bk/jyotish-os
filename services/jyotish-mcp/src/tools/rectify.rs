//! `rectify.birth_time` — HEURISTIC birth-time rectification.
//!
//! Sweep candidate birth instants in ±window_min at 1-minute steps. For each
//! candidate compute the Ascendant (rashi / nakshatra / pada) and the
//! Vimshottari maha + antar lords running at every supplied life event.
//! Score = number of (event, level) pairs whose running lord is among the
//! natural significators the caller supplied for that event (maha +1,
//! antar +1). Ties are broken by |offset from the stated time|. This is a
//! documented heuristic, not a proof of birth time.

use serde_json::{Value, json};
use xalen_houses::HouseSystem;
use xalen_vedic::nakshatra::DashaLord;

use super::{Ctx, ToolOutput, args};
use crate::engine::{
    AYANAMSA_NAME, BodyId, ENGINE_NAME, Engine, Instant, dasha_lord_name, dasha_lords_at, placement,
};
use crate::types::{BirthInput, ToolError, jd_to_iso_local, jd_to_iso_utc, parse_date, parse_utc};

const MAX_WINDOW_MIN: u64 = 720;
const MAX_EVENTS: usize = 50;
const MAX_TOP_N: u64 = 100;

pub struct Event {
    label: String,
    jd: f64,
    significators: Vec<DashaLord>,
}

fn parse_events(v: Option<&Value>) -> Result<Vec<Event>, ToolError> {
    let arr = v
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::invalid("events must be a non-empty array"))?;
    if arr.is_empty() || arr.len() > MAX_EVENTS {
        return Err(ToolError::invalid(format!(
            "events must contain 1..={MAX_EVENTS} entries"
        )));
    }
    let mut out = Vec::new();
    for (i, e) in arr.iter().enumerate() {
        let o = e
            .as_object()
            .ok_or_else(|| ToolError::invalid(format!("events[{i}] must be an object")))?;
        let jd = if let Some(d) = o.get("date").and_then(Value::as_str) {
            let (y, m, dd) = parse_date(d)?;
            xalen_time::calendar_to_jd(
                y,
                m,
                dd,
                12.0,
                xalen_time::CalendarSystem::ProlepticGregorian,
            )
            .0
        } else if let Some(u) = o.get("utc").and_then(Value::as_str) {
            Instant::from_utc_fields(&parse_utc(u)?).jd_ut1.0
        } else {
            return Err(ToolError::invalid(format!(
                "events[{i}] needs `date` (YYYY-MM-DD) or `utc`"
            )));
        };
        let sig_arr = o
            .get("significators")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ToolError::invalid(format!(
                    "events[{i}].significators must be an array of graha names"
                ))
            })?;
        if sig_arr.is_empty() || sig_arr.len() > 9 {
            return Err(ToolError::invalid(format!(
                "events[{i}].significators must have 1..=9 entries"
            )));
        }
        let mut significators = Vec::new();
        for s in sig_arr {
            let name = s
                .as_str()
                .ok_or_else(|| ToolError::invalid("significators must be strings"))?;
            let lord = BodyId::parse(name)
                .and_then(BodyId::to_dasha_lord)
                .ok_or_else(|| {
                    ToolError::invalid(format!(
                        "significator {name:?} is not one of the nine Vimshottari lords"
                    ))
                })?;
            if !significators.contains(&lord) {
                significators.push(lord);
            }
        }
        let label = o
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(80)
            .collect::<String>();
        out.push(Event {
            label,
            jd,
            significators,
        });
    }
    Ok(out)
}

pub fn compute(
    engine: &Engine,
    birth: &BirthInput,
    window_min: u64,
    step_min: u64,
    events: &[Event],
    top_n: u64,
) -> Result<Value, ToolError> {
    let base = Instant::from_utc_fields(&parse_utc(&birth.utc)?);
    let base_jd = base.jd_ut1.0;
    engine.check_coverage(&Instant::from_jd_ut1(base_jd - window_min as f64 / 1440.0))?;
    engine.check_coverage(&Instant::from_jd_ut1(base_jd + window_min as f64 / 1440.0))?;
    for e in events {
        engine.check_coverage(&Instant::from_jd_ut1(e.jd))?;
    }

    let mut candidates: Vec<Value> = Vec::new();
    let mut offset: i64 = -(window_min as i64);
    let max_score = (events.len() * 2) as u64;
    while offset <= window_min as i64 {
        let jd = base_jd + offset as f64 / 1440.0;
        let at = Instant::from_jd_ut1(jd);
        let moon = engine.sidereal_lon(BodyId::Moon, &at)?;
        let asc = engine
            .houses(&at, birth.lat, birth.lon, HouseSystem::WholeSign)?
            .ascendant_deg;
        let periods = engine.vimshottari(moon, jd);
        let mut score = 0u64;
        let mut per_event = Vec::with_capacity(events.len());
        for e in events {
            let (maha, antar) = match dasha_lords_at(&periods, e.jd) {
                Some(x) => x,
                None => {
                    per_event.push(json!({ "label": e.label, "maha": null, "antar": null, "hits": 0, "note": "event outside the 120-year cycle from birth" }));
                    continue;
                }
            };
            let mut hits = 0u64;
            if e.significators.contains(&maha) {
                hits += 1;
            }
            if let Some(a) = antar
                && e.significators.contains(&a)
            {
                hits += 1;
            }
            score += hits;
            per_event.push(json!({
                "label": e.label,
                "date_utc": jd_to_iso_utc(e.jd),
                "maha": dasha_lord_name(maha),
                "antar": antar.map(dasha_lord_name),
                "hits": hits,
            }));
        }
        let ap = placement(asc);
        let mp = placement(moon);
        candidates.push(json!({
            "offset_min": offset,
            "utc": jd_to_iso_utc(jd),
            "local": jd_to_iso_local(jd, birth.tz_offset_hours),
            "score": score,
            "ascendant": { "sidereal_lon_deg": asc, "rashi": ap.rashi, "nakshatra": ap.nakshatra, "pada": ap.pada },
            "moon": { "sidereal_lon_deg": moon, "nakshatra": mp.nakshatra, "pada": mp.pada, "dasha_lord_at_birth": mp.nakshatra_lord },
            "events": per_event,
        }));
        offset += step_min as i64;
    }
    let total = candidates.len();
    candidates.sort_by(|a, b| {
        let sa = a["score"].as_u64().unwrap_or(0);
        let sb = b["score"].as_u64().unwrap_or(0);
        sb.cmp(&sa).then_with(|| {
            let oa = a["offset_min"].as_i64().unwrap_or(0).abs();
            let ob = b["offset_min"].as_i64().unwrap_or(0).abs();
            oa.cmp(&ob)
        })
    });
    let best = candidates
        .first()
        .and_then(|c| c["score"].as_u64())
        .unwrap_or(0);
    let tied = candidates
        .iter()
        .filter(|c| c["score"].as_u64() == Some(best))
        .count();
    candidates.truncate(top_n as usize);

    Ok(json!({
        "heuristic": true,
        "note": "HEURISTIC ONLY. Score counts (event, level) pairs whose running Vimshottari maha/antar lord is among the caller-supplied significators. It does not validate birth time; ties are broken by proximity to the stated time and are otherwise unresolved. Use as a shortlist for a human astrologer.",
        "method": {
            "sweep": format!("±{window_min} min at {step_min}-min steps"),
            "score": "sum over events of [maha lord ∈ significators] + [antar lord ∈ significators]",
            "max_score": max_score,
            "dasha": "Vimshottari (xalen-vedic), 365.25-day years, TRUE-node-independent (Moon nakshatra based)",
            "ascendant": "WholeSign sidereal (Lahiri), GAST + true obliquity",
        },
        "input": birth,
        "ayanamsa": AYANAMSA_NAME,
        "candidates_evaluated": total,
        "best_score": best,
        "candidates_tied_at_best": tied,
        "candidates": candidates,
    }))
}

pub fn run(ctx: &Ctx, args: Value) -> Result<ToolOutput, ToolError> {
    let m = args::obj(&args, "arguments")?;
    let birth = args::birth(m, "birth")?;
    let window_min = args::u64_opt(m, "window_min", 120)?;
    if !(1..=MAX_WINDOW_MIN).contains(&window_min) {
        return Err(ToolError::invalid(format!(
            "window_min must be within [1, {MAX_WINDOW_MIN}]"
        )));
    }
    let step_min = args::u64_opt(m, "step_min", 1)?;
    if !(1..=60).contains(&step_min) {
        return Err(ToolError::invalid("step_min must be within [1, 60]"));
    }
    let top_n = args::u64_opt(m, "top_n", 10)?;
    if !(1..=MAX_TOP_N).contains(&top_n) {
        return Err(ToolError::invalid(format!(
            "top_n must be within [1, {MAX_TOP_N}]"
        )));
    }
    let events = parse_events(m.get("events"))?;
    let canonical = json!({
        "birth": birth, "window_min": window_min, "step_min": step_min, "top_n": top_n,
        "events": m.get("events"),
    });
    let engine = ctx.engine.clone();
    let (payload, cache_hit) = ctx.cached("rectify.birth_time", &canonical, || {
        compute(&engine, &birth, window_min, step_min, &events, top_n)
    })?;
    let sigma = Some(Instant::from_utc_fields(&parse_utc(&birth.utc)?).delta_t_sigma_sec);
    Ok(ToolOutput {
        payload,
        engine: ENGINE_NAME,
        consensus_status: "NOT_CHECKED".into(),
        delta_t_sigma_sec: sigma,
        cache_hit,
    })
}

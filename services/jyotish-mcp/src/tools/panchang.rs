//! `panchang.day` — tithi / nakshatra / yoga / karana with transition instants
//! for one civil day (sunrise to next sunrise), plus sunrise/sunset and
//! moonrise/moonset. Root-finding is `xalen_vedic::panchang_transitions`;
//! positions are DE440 through the engine closure.

use serde_json::{Value, json};
use xalen_time::{CalendarSystem, calendar_to_jd};
use xalen_vedic::nakshatra::Nakshatra;
use xalen_vedic::panchang::{
    Karana, Paksha, Tithi, Vara, Yoga, compute_panchang, karana_transition, nakshatra_transition,
    tithi_transition, yoga_transition,
};

use super::{Ctx, ToolOutput, args};
use crate::engine::{AYANAMSA_NAME, BodyId, ENGINE_NAME, Engine, Instant};
use crate::types::{
    ToolError, jd_to_iso_local, jd_to_iso_utc, parse_date, validate_lat_lon, validate_tz,
};

/// Upper bound on element intervals inside one day (a tithi lasts ≥ ~19 h,
/// so 6 is generous even at extreme latitudes with long solar days).
const MAX_INTERVALS: usize = 8;

fn iso_pair(jd: f64, tz: f64) -> Value {
    json!({ "utc": jd_to_iso_utc(jd), "local": jd_to_iso_local(jd, tz), "jd": jd })
}

fn tithi_json(t: &Tithi) -> Value {
    json!({
        "number": t.number,
        "name": t.name(),
        "paksha": match t.paksha { Paksha::Shukla => "Shukla", Paksha::Krishna => "Krishna" },
    })
}

fn nak_json(n: &Nakshatra) -> Value {
    json!({ "index": n.index(), "name": format!("{n:?}"), "lord": format!("{:?}", n.lord()) })
}

fn yoga_json(y: &Yoga) -> Value {
    json!({ "number": y.number, "name": y.name() })
}

fn karana_json(k: &Karana) -> Value {
    json!({ "number": k.number, "name": k.name(), "name_index": k.name_index })
}

/// Walk one element's intervals from `start` until the interval end passes
/// `end`. `next(jd, include_start)` returns (value_json, start_jd, end_jd).
fn walk<F>(start: f64, end: f64, tz: f64, mut next: F) -> Result<Vec<Value>, ToolError>
where
    F: FnMut(f64, bool) -> Result<(Value, Option<f64>, f64), ToolError>,
{
    let mut out = Vec::new();
    let mut t = start;
    for i in 0..MAX_INTERVALS {
        let (value, s, e) = next(t, i == 0)?;
        let mut item = json!({ "value": value, "end": iso_pair(e, tz) });
        if let Some(s) = s {
            item["start"] = iso_pair(s, tz);
        } else {
            item["start"] = iso_pair(t, tz);
        }
        out.push(item);
        if e >= end {
            break;
        }
        t = e + 1.0 / 86400.0;
    }
    Ok(out)
}

pub fn compute(
    engine: &Engine,
    date: &str,
    lat: f64,
    lon: f64,
    tz: f64,
) -> Result<Value, ToolError> {
    let (y, m, d) = parse_date(date)?;
    validate_lat_lon(lat, lon)?;
    validate_tz(tz)?;
    // Local civil midnight expressed in UT.
    let jd0 = calendar_to_jd(y, m, d, -tz, CalendarSystem::ProlepticGregorian).0;
    engine.check_coverage(&Instant::from_jd_ut1(jd0))?;

    let sun = engine.rise_set_next(BodyId::Sun, jd0, lat, lon)?;
    let sunrise = sun.rise.map(|j| j.0).ok_or_else(|| {
        ToolError::new(
            "NO_SUNRISE",
            format!("no sunrise within a day of {date} at lat {lat}: polar day/night; sunrise-anchored panchang undefined"),
        )
        .with_details(json!({ "always_above": sun.always_above, "always_below": sun.always_below }))
    })?;
    let after = engine.rise_set_next(BodyId::Sun, sunrise + 60.0 / 86400.0, lat, lon)?;
    let sunset = after
        .set
        .map(|j| j.0)
        .ok_or_else(|| ToolError::new("NO_SUNSET", "no sunset after sunrise (polar)"))?;
    let next_sunrise = after
        .rise
        .map(|j| j.0)
        .ok_or_else(|| ToolError::new("NO_SUNRISE", "no next sunrise (polar)"))?;
    let moon = engine.rise_set_next(BodyId::Moon, jd0, lat, lon)?;

    let pos = |jd: f64| -> (f64, f64) {
        // The transition finder wants an infallible closure; inside coverage
        // (checked above, with a 5-day margin) these cannot fail.
        engine.sun_moon_sidereal(jd).unwrap_or((f64::NAN, f64::NAN))
    };
    let (s0, m0) = pos(sunrise);
    if !s0.is_finite() || !m0.is_finite() {
        return Err(ToolError::new(
            "EPHEMERIS_ERROR",
            "Sun/Moon unavailable at sunrise",
        ));
    }
    let at_sunrise = compute_panchang(s0, m0, sunrise);
    let end = next_sunrise;
    let fail = |what: &str| {
        ToolError::new(
            "EPHEMERIS_ERROR",
            format!("{what} boundary search failed (non-monotone ephemeris)"),
        )
    };

    let tithis = walk(sunrise, end, tz, |t, inc| {
        let iv = tithi_transition(&pos, t, inc).ok_or_else(|| fail("tithi"))?;
        Ok((tithi_json(&iv.value), iv.start_jd, iv.end_jd))
    })?;
    let naks = walk(sunrise, end, tz, |t, inc| {
        let iv = nakshatra_transition(&pos, t, inc).ok_or_else(|| fail("nakshatra"))?;
        Ok((nak_json(&iv.value), iv.start_jd, iv.end_jd))
    })?;
    let yogas = walk(sunrise, end, tz, |t, inc| {
        let iv = yoga_transition(&pos, t, inc).ok_or_else(|| fail("yoga"))?;
        Ok((yoga_json(&iv.value), iv.start_jd, iv.end_jd))
    })?;
    let karanas = walk(sunrise, end, tz, |t, inc| {
        let iv = karana_transition(&pos, t, inc).ok_or_else(|| fail("karana"))?;
        Ok((karana_json(&iv.value), iv.start_jd, iv.end_jd))
    })?;

    // Weekday of the LOCAL civil date (local noon evaluated as if UT), so far-
    // eastern zones do not pick up the previous UT date's vara.
    let vara = Vara::from_jd(calendar_to_jd(y, m, d, 12.0, CalendarSystem::ProlepticGregorian).0);
    let weekday = match vara {
        Vara::Sunday => "Sunday",
        Vara::Monday => "Monday",
        Vara::Tuesday => "Tuesday",
        Vara::Wednesday => "Wednesday",
        Vara::Thursday => "Thursday",
        Vara::Friday => "Friday",
        Vara::Saturday => "Saturday",
    };
    let day_len = sunset - sunrise;
    let hora_len_day = day_len / 12.0;
    let night_len = next_sunrise - sunset;
    let rahu_seq = [8usize, 2, 7, 5, 6, 4, 3]; // Sun..Sat: 1-based eighth of the day
    let vara_idx = match vara {
        Vara::Sunday => 0,
        Vara::Monday => 1,
        Vara::Tuesday => 2,
        Vara::Wednesday => 3,
        Vara::Thursday => 4,
        Vara::Friday => 5,
        Vara::Saturday => 6,
    };
    let rahu_start = sunrise + day_len / 8.0 * (rahu_seq[vara_idx] - 1) as f64;
    let abhijit_mid = sunrise + day_len / 2.0;
    let muhurta = day_len / 15.0;

    Ok(json!({
        "date": date,
        "location": { "lat": lat, "lon": lon, "tz_offset_hours": tz },
        "ayanamsa": AYANAMSA_NAME,
        "ayanamsa_deg": engine.ayanamsa_deg(&Instant::from_jd_ut1(sunrise)),
        "sunrise": iso_pair(sunrise, tz),
        "sunset": iso_pair(sunset, tz),
        "next_sunrise": iso_pair(next_sunrise, tz),
        "moonrise": moon.rise.map(|j| iso_pair(j.0, tz)),
        "moonset": moon.set.map(|j| iso_pair(j.0, tz)),
        "vara": { "name": vara.name(), "weekday": weekday, "lord": vara.lord() },
        "at_sunrise": {
            "sun_sidereal_deg": s0,
            "moon_sidereal_deg": m0,
            "tithi": tithi_json(&at_sunrise.tithi),
            "nakshatra": nak_json(&at_sunrise.nakshatra),
            "yoga": yoga_json(&at_sunrise.yoga),
            "karana": karana_json(&at_sunrise.karana),
        },
        "tithi": tithis,
        "nakshatra": naks,
        "yoga": yogas,
        "karana": karanas,
        "rahu_kala": { "start": iso_pair(rahu_start, tz), "end": iso_pair(rahu_start + day_len / 8.0, tz) },
        "abhijit": { "start": iso_pair(abhijit_mid - muhurta / 2.0, tz), "end": iso_pair(abhijit_mid + muhurta / 2.0, tz) },
        "hora": {
            "day_hora_length_min": hora_len_day * 1440.0,
            "night_hora_length_min": night_len / 12.0 * 1440.0,
        },
        "notes": [
            "Day = local sunrise to next sunrise; elements listed with their start/end instants (UT and local).",
            "Sunrise/sunset: Sun upper limb with standard refraction at sea level (xalen-ephem rise_set), elevation 0 m.",
            "Rahu kala uses the classical eighth-of-day sequence (Sun 8, Mon 2, Tue 7, Wed 5, Thu 6, Fri 4, Sat 3); abhijit = middle muhurta of the day (day/15)."
        ]
    }))
}

pub fn run(ctx: &Ctx, args: Value) -> Result<ToolOutput, ToolError> {
    let m = args::obj(&args, "arguments")?;
    let date = args::str_req(m, "date")?.to_string();
    let lat = args::f64_req(m, "lat")?;
    let lon = args::f64_req(m, "lon")?;
    let tz = args::f64_opt(m, "tz_offset_hours", 0.0)?;
    let canonical = json!({ "date": date, "lat": lat, "lon": lon, "tz_offset_hours": tz });
    let engine = ctx.engine.clone();
    let (payload, cache_hit) = ctx.cached("panchang.day", &canonical, || {
        compute(&engine, &date, lat, lon, tz)
    })?;
    let sigma = payload["sunrise"]["jd"]
        .as_f64()
        .map(|jd| Instant::from_jd_ut1(jd).delta_t_sigma_sec);
    Ok(ToolOutput {
        payload,
        engine: ENGINE_NAME,
        consensus_status: "NOT_CHECKED".into(),
        delta_t_sigma_sec: sigma,
        cache_hit,
    })
}

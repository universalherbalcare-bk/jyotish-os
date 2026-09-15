//! Differential corpus: XALEN-DE440 (jyotish-mcp engine, linked) vs jhora-svc
//! (PyJHora / Swiss Ephemeris, over HTTP) on N seeded pseudo-random births.
//!
//! Every chart is compared per docs/CONTRACT.md "Consensus tolerances":
//! tropical (own ayanamsa), ayanamsa (true & mean equinox), sidereal end-to-end,
//! Rahu/Ketu (analytic node), Ascendant — plus ungated diagnostics that isolate
//! the time-scale conventions (XALEN at Swiss's UT1 / TT). Output: one CSV row
//! per chart, a Markdown summary with mean/RMS/p50/p99/p99.9/max per category,
//! the ten worst epochs, and pass/fail; exit 1 on any gate failure, 2 if any
//! chart could not be computed or fetched (nothing is dropped silently).

mod rng;
mod stats;

use chrono::{Duration, NaiveDate};
use jyotish_mcp::engine::{BodyId, Engine, Instant, NODE_SOURCE_DE440};
use jyotish_mcp::tools::consensus::{
    ASC_TOL_DEG, AYANAMSA_TOL_ARCSEC, NODE_TOL_ANALYTIC_ARCSEC, NODE_TOL_ARCSEC, TimeScaleEra,
    angular_separation_deg, sidereal_tolerance_arcsec_for, tropical_tolerance_arcsec,
    tropical_tolerance_for,
};
use jyotish_mcp::types::{BirthInput, parse_utc, signed_delta_deg};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant as StdInstant};
use tokio::sync::Semaphore;
use xalen_houses::HouseSystem;

use crate::rng::SplitMix64;
use crate::stats::{Summary, summarize};

const PLANETS: [BodyId; 7] = [
    BodyId::Sun,
    BodyId::Moon,
    BodyId::Mercury,
    BodyId::Venus,
    BodyId::Mars,
    BodyId::Jupiter,
    BodyId::Saturn,
];

#[derive(Debug, Clone)]
struct Args {
    n: usize,
    seed: u64,
    jhora_url: String,
    kernel: PathBuf,
    sha: PathBuf,
    concurrency: usize,
    csv: PathBuf,
    summary: PathBuf,
    timeout_ms: u64,
    retries: u32,
    year_from: i32,
    year_to: i32,
}

fn usage() -> ! {
    eprintln!(
        "consensus-corpus [--n N=10000] [--seed S=42] [--jhora-url http://127.0.0.1:7792] \
         [--kernel kernels/de440s.bsp] [--sha kernels/de440s.sha256] [--concurrency 6] \
         [--csv validation/consensus-corpus.csv] [--summary validation/consensus-summary.md] \
         [--timeout-ms 5000] [--retries 6] [--years 1900:2050]"
    );
    std::process::exit(64);
}

fn parse_args() -> Args {
    let mut a = Args {
        n: 10_000,
        seed: 42,
        jhora_url: "http://127.0.0.1:7792".into(),
        kernel: PathBuf::from("kernels/de440s.bsp"),
        sha: PathBuf::new(),
        concurrency: 6,
        csv: PathBuf::from("validation/consensus-corpus.csv"),
        summary: PathBuf::from("validation/consensus-summary.md"),
        timeout_ms: 5000,
        retries: 6,
        year_from: 1900,
        year_to: 2050,
    };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let key = argv[i].as_str();
        let val = argv.get(i + 1).cloned();
        let need = |v: Option<String>| v.unwrap_or_else(|| usage());
        match key {
            "--n" => a.n = need(val).parse().unwrap_or_else(|_| usage()),
            "--seed" => a.seed = need(val).parse().unwrap_or_else(|_| usage()),
            "--jhora-url" => a.jhora_url = need(val),
            "--kernel" => a.kernel = PathBuf::from(need(val)),
            "--sha" => a.sha = PathBuf::from(need(val)),
            "--concurrency" => a.concurrency = need(val).parse().unwrap_or_else(|_| usage()),
            "--csv" => a.csv = PathBuf::from(need(val)),
            "--summary" => a.summary = PathBuf::from(need(val)),
            "--timeout-ms" => a.timeout_ms = need(val).parse().unwrap_or_else(|_| usage()),
            "--retries" => a.retries = need(val).parse().unwrap_or_else(|_| usage()),
            "--years" => {
                let v = need(val);
                let (f, t) = v.split_once(':').unwrap_or_else(|| usage());
                a.year_from = f.parse().unwrap_or_else(|_| usage());
                a.year_to = t.parse().unwrap_or_else(|_| usage());
            }
            "-h" | "--help" => usage(),
            _ => usage(),
        }
        i += 2;
    }
    if a.sha.as_os_str().is_empty() {
        a.sha = a.kernel.with_extension("sha256");
    }
    if a.n == 0 || a.concurrency == 0 || a.year_from > a.year_to {
        usage();
    }
    let host_ok = a.jhora_url.starts_with("http://127.0.0.1")
        || a.jhora_url.starts_with("http://localhost")
        || a.jhora_url.starts_with("http://[::1]");
    if !host_ok {
        eprintln!("refusing non-loopback jhora-svc URL {:?}", a.jhora_url);
        std::process::exit(64);
    }
    a
}

/// One seeded birth. `utc` is the exact string both engines receive.
#[derive(Debug, Clone)]
struct Sample {
    idx: usize,
    birth: BirthInput,
}

fn generate(args: &Args) -> Vec<Sample> {
    let start = NaiveDate::from_ymd_opt(args.year_from, 1, 1)
        .expect("year_from")
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let end = NaiveDate::from_ymd_opt(args.year_to + 1, 1, 1)
        .expect("year_to")
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let span_sec = (end - start).num_seconds();
    let mut rng = SplitMix64::new(args.seed);
    (0..args.n)
        .map(|idx| {
            let sec = (rng.next_f64() * span_sec as f64).floor() as i64;
            let t = start + Duration::seconds(sec.min(span_sec - 1));
            let lat = rng.uniform(-66.0, 66.0);
            let lon = rng.uniform(-180.0, 180.0);
            let tz = (lon / 15.0).round();
            Sample {
                idx,
                birth: BirthInput {
                    utc: format!("{}Z", t.format("%Y-%m-%dT%H:%M:%S")),
                    lat,
                    lon,
                    tz_offset_hours: tz,
                },
            }
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
struct PerBody {
    /// XALEN at the ALIGNED instant (Swiss's jd_ut/jd_tt) — what the gate uses.
    xalen_trop: f64,
    xalen_sid: f64,
    jhora_sid: f64,
    /// Signed arcsec, XALEN(aligned) − jhora. Gated.
    trop_delta: f64,
    sid_delta: f64,
    /// Signed arcsec, XALEN at its OWN instant − jhora. Diagnostic only
    /// (folds the time-scale convention into the ephemeris comparison).
    trop_delta_own: f64,
    sid_delta_own: f64,
    /// Angular separation from the Sun (deg) at the aligned instant.
    elongation_deg: f64,
    /// Per-chart tropical tolerance (band-aware); None for the nodes.
    trop_tol: Option<f64>,
    in_conjunction_band: bool,
    source: &'static str,
    xalen_retro: bool,
    jhora_retro: bool,
}

#[derive(Debug, Clone)]
struct Record {
    idx: usize,
    birth: BirthInput,
    era: &'static str,
    jd_ut1_x: f64,
    jd_ut_j: f64,
    jd_tt_x: f64,
    jd_tt_j: f64,
    jd_ut_delta_sec: f64,
    jd_tt_delta_sec: f64,
    delta_t_sigma_sec: f64,
    aya_true_x: f64,
    aya_true_j: f64,
    aya_true_delta: f64,
    aya_mean_delta: f64,
    dpsi_delta_mas: f64,
    asc_x: f64,
    asc_j: f64,
    /// XALEN at Swiss's UT1 − jhora (gated).
    asc_delta_deg: f64,
    /// XALEN at its own UT1 − jhora (diagnostic).
    asc_delta_own_deg: f64,
    rahu_ketu_consistency_arcsec: f64,
    time_scale_era: TimeScaleEra,
    bodies: BTreeMap<&'static str, PerBody>,
    attempts: u32,
}

#[derive(Debug, Clone)]
struct Failure {
    idx: usize,
    birth: BirthInput,
    stage: &'static str,
    message: String,
    attempts: u32,
}

const ERAS: [&str; 3] = [
    "pre-1972 (both: UT=UTC, TT=UT+ΔT model)",
    "1972..2033-09-16 (both: leap-second TT)",
    "2033-09-17.. (Swiss: UTC+ΔT model; XALEN: leap-second TT)",
];

fn era_label(e: TimeScaleEra) -> &'static str {
    match e {
        TimeScaleEra::Pre1972 => ERAS[0],
        TimeScaleEra::KnownTaiUtc => ERAS[1],
        TimeScaleEra::Extrapolated => ERAS[2],
    }
}

fn num(v: &Value, path: &str) -> Result<f64, String> {
    v.as_f64()
        .ok_or_else(|| format!("jhora-svc field {path:?} missing or non-numeric"))
}

async fn fetch_positions(
    http: &reqwest::Client,
    url: &str,
    birth: &BirthInput,
    retries: u32,
) -> Result<(Value, u32), (String, u32)> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let resp = http.post(url).json(birth).send().await;
        match resp {
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                if status.is_success() {
                    match serde_json::from_str::<Value>(&text) {
                        Ok(v) => return Ok((v, attempt)),
                        Err(e) => {
                            if attempt > retries {
                                return Err((format!("non-JSON body: {e}"), attempt));
                            }
                        }
                    }
                } else if status.is_server_error() || status.as_u16() == 429 {
                    if attempt > retries {
                        let snippet: String = text.chars().take(300).collect();
                        return Err((
                            format!("HTTP {status} after {attempt} attempts: {snippet}"),
                            attempt,
                        ));
                    }
                } else {
                    // 4xx: deterministic rejection, retrying cannot help.
                    let snippet: String = text.chars().take(300).collect();
                    return Err((format!("HTTP {status}: {snippet}"), attempt));
                }
            }
            Err(e) => {
                if attempt > retries {
                    return Err((format!("transport: {e} after {attempt} attempts"), attempt));
                }
            }
        }
        let backoff = StdDuration::from_millis(100u64.saturating_mul(1u64 << (attempt - 1).min(6)));
        tokio::time::sleep(backoff).await;
    }
}

fn compare(engine: &Engine, s: &Sample, resp: &Value, attempts: u32) -> Result<Record, String> {
    let fields = parse_utc(&s.birth.utc).map_err(|e| e.message)?;
    let own = Instant::from_utc_fields(&fields);
    engine.check_coverage(&own).map_err(|e| e.to_string())?;

    let jd_ut_j = num(&resp["jd_ut"], "jd_ut")?;
    let jd_tt_j = num(&resp["jd_tt"], "jd_tt")?;
    let aya_true_j = num(
        &resp["ayanamsa_true_equinox_deg"],
        "ayanamsa_true_equinox_deg",
    )?;
    let aya_mean_j = num(&resp["ayanamsa_deg"], "ayanamsa_deg")?;
    let dpsi_j = num(&resp["nutation_dpsi_arcsec"], "nutation_dpsi_arcsec")?;
    let asc_j = num(&resp["ascendant"], "ascendant")?;

    // The gate compares ephemerides at the SAME instant: XALEN at Swiss's jd_ut/jd_tt.
    let at = Instant::from_parts(jd_ut_j, jd_tt_j);
    engine.check_coverage(&at).map_err(|e| e.to_string())?;

    let aya_true_x = engine.ayanamsa_deg(&at);
    let aya_mean_x = engine.ayanamsa_mean_equinox_deg(&at);
    let dpsi_x = engine.nutation_dpsi_deg(&at) * 3600.0;

    let asc_x = engine
        .houses(&at, s.birth.lat, s.birth.lon, HouseSystem::WholeSign)
        .map_err(|e| e.to_string())?
        .ascendant_deg;
    let asc_x_own = engine
        .houses(&own, s.birth.lat, s.birth.lon, HouseSystem::WholeSign)
        .map_err(|e| e.to_string())?
        .ascendant_deg;

    let sun = engine
        .body_state(BodyId::Sun, &at)
        .map_err(|e| e.to_string())?;
    let mut bodies = BTreeMap::new();
    for id in BodyId::GRAHAS {
        let name = id.name();
        let st = engine.body_state(id, &at).map_err(|e| e.to_string())?;
        let st_own = engine.body_state(id, &own).map_err(|e| e.to_string())?;
        let j = &resp["bodies"][name];
        let jhora_sid = num(&j["lon"], &format!("bodies.{name}.lon"))?;
        let jhora_trop = (jhora_sid + aya_true_j).rem_euclid(360.0);
        let elongation = angular_separation_deg(
            st.tropical_lon_deg,
            st.latitude_deg,
            sun.tropical_lon_deg,
            sun.latitude_deg,
        );
        let (trop_tol, note) = tropical_tolerance_for(id, elongation);
        bodies.insert(
            name,
            PerBody {
                xalen_trop: st.tropical_lon_deg,
                xalen_sid: st.sidereal_lon_deg,
                jhora_sid,
                trop_delta: signed_delta_deg(st.tropical_lon_deg, jhora_trop) * 3600.0,
                sid_delta: signed_delta_deg(st.sidereal_lon_deg, jhora_sid) * 3600.0,
                trop_delta_own: signed_delta_deg(st_own.tropical_lon_deg, jhora_trop) * 3600.0,
                sid_delta_own: signed_delta_deg(st_own.sidereal_lon_deg, jhora_sid) * 3600.0,
                elongation_deg: elongation,
                trop_tol,
                in_conjunction_band: note.is_some(),
                source: st.source,
                xalen_retro: st.retrograde,
                jhora_retro: j["retro"].as_bool().unwrap_or(false),
            },
        );
    }
    // Ketu = Rahu + 180° in both engines; record the largest deviation from that identity.
    let rk = |b: &BTreeMap<&str, PerBody>, f: fn(&PerBody) -> f64| {
        signed_delta_deg(f(&b["Ketu"]), f(&b["Rahu"]) + 180.0).abs() * 3600.0
    };
    let rahu_ketu_consistency_arcsec =
        rk(&bodies, |p| p.xalen_sid).max(rk(&bodies, |p| p.jhora_sid));

    Ok(Record {
        idx: s.idx,
        birth: s.birth.clone(),
        era: era_label(TimeScaleEra::of_jd_utc(own.jd_ut1.0)),
        jd_ut1_x: own.jd_ut1.0,
        jd_ut_j,
        jd_tt_x: own.jd_tt.0,
        jd_tt_j,
        jd_ut_delta_sec: (own.jd_ut1.0 - jd_ut_j) * 86400.0,
        jd_tt_delta_sec: (own.jd_tt.0 - jd_tt_j) * 86400.0,
        delta_t_sigma_sec: own.delta_t_sigma_sec,
        aya_true_x,
        aya_true_j,
        aya_true_delta: signed_delta_deg(aya_true_x, aya_true_j) * 3600.0,
        aya_mean_delta: signed_delta_deg(aya_mean_x, aya_mean_j) * 3600.0,
        dpsi_delta_mas: (dpsi_x - dpsi_j) * 1000.0,
        asc_x,
        asc_j,
        asc_delta_deg: signed_delta_deg(asc_x, asc_j),
        asc_delta_own_deg: signed_delta_deg(asc_x_own, asc_j),
        rahu_ketu_consistency_arcsec,
        time_scale_era: TimeScaleEra::of_jd_utc(own.jd_ut1.0),
        bodies,
        attempts,
    })
}

/// A category: how to read a signed delta off a record, its unit, tolerance
/// (None = diagnostic only), and whether it is part of the gate.
/// A category: how to read a signed delta off a record, its unit, the nominal
/// tolerance (gates p99.9 and, unless overridden, every chart; None =
/// diagnostic only), an optional per-chart tolerance override (the solar
/// conjunction band / node source), and an optional record filter (time-scale eras).
type Getter = Box<dyn Fn(&Record) -> f64 + Send + Sync>;
type ChartTol = Box<dyn Fn(&Record) -> Option<f64> + Send + Sync>;
type Filter = Box<dyn Fn(&Record) -> bool + Send + Sync>;

struct Category {
    name: String,
    unit: &'static str,
    tolerance: Option<f64>,
    get: Getter,
    per_chart_tol: Option<ChartTol>,
    filter: Option<Filter>,
}

impl Category {
    fn simple(
        name: impl Into<String>,
        unit: &'static str,
        tolerance: Option<f64>,
        get: impl Fn(&Record) -> f64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            unit,
            tolerance,
            get: Box::new(get),
            per_chart_tol: None,
            filter: None,
        }
    }
    fn applies(&self, r: &Record) -> bool {
        self.filter.as_ref().is_none_or(|f| f(r))
    }
    /// The tolerance this particular chart is held to.
    fn chart_tol(&self, r: &Record) -> Option<f64> {
        match &self.per_chart_tol {
            Some(f) => f(r),
            None => self.tolerance,
        }
    }
    fn is_gated(&self) -> bool {
        self.tolerance.is_some()
    }
}

fn categories() -> Vec<Category> {
    let mut v: Vec<Category> = Vec::new();
    // (a) tropical at the aligned instant; the per-chart tolerance widens to 2.5″ inside the
    //     solar-conjunction band while the p99.9 gate stays at the nominal value.
    for id in PLANETS {
        let n = id.name();
        v.push(Category {
            name: format!("tropical.{n}"),
            unit: "arcsec",
            tolerance: tropical_tolerance_arcsec(id),
            get: Box::new(move |r| r.bodies[n].trop_delta),
            per_chart_tol: Some(Box::new(move |r| r.bodies[n].trop_tol)),
            filter: None,
        });
    }
    // (b) ayanamsa under matching convention
    v.push(Category::simple(
        "ayanamsa.true_equinox",
        "arcsec",
        Some(AYANAMSA_TOL_ARCSEC),
        |r| r.aya_true_delta,
    ));
    v.push(Category::simple(
        "ayanamsa.mean_equinox",
        "arcsec",
        Some(AYANAMSA_TOL_ARCSEC),
        |r| r.aya_mean_delta,
    ));
    // (c) sidereal end-to-end at the aligned instant
    for id in PLANETS {
        let n = id.name();
        v.push(Category::simple(
            format!("sidereal.{n}"),
            "arcsec",
            Some(sidereal_tolerance_arcsec_for(id, NODE_SOURCE_DE440)),
            move |r| r.bodies[n].sid_delta,
        ));
    }
    // (d) nodes: the per-chart tolerance depends on which node source served the chart
    for n in ["Rahu", "Ketu"] {
        v.push(Category {
            name: format!("nodes.{n} (source: de440-osculating)"),
            unit: "arcsec",
            tolerance: Some(NODE_TOL_ARCSEC),
            get: Box::new(move |r| r.bodies[n].sid_delta),
            per_chart_tol: Some(Box::new(move |r| {
                Some(if r.bodies[n].source == NODE_SOURCE_DE440 {
                    NODE_TOL_ARCSEC
                } else {
                    NODE_TOL_ANALYTIC_ARCSEC
                })
            })),
            filter: None,
        });
    }
    // (e) Ascendant at Swiss's UT1
    v.push(Category::simple(
        "ascendant",
        "deg",
        Some(ASC_TOL_DEG),
        |r| r.asc_delta_deg,
    ));
    // (f) time-scale conventions, gated per era (docs/CONTRACT.md)
    let era_cat =
        |name: &str, tol: Option<f64>, era: TimeScaleEra, get: fn(&Record) -> f64| Category {
            name: name.into(),
            unit: "sec",
            tolerance: tol,
            get: Box::new(get),
            per_chart_tol: None,
            filter: Some(Box::new(move |r| r.time_scale_era == era)),
        };
    v.push(era_cat(
        "time_scale.tt (pre-1972, ΔT tables)",
        Some(1.0),
        TimeScaleEra::Pre1972,
        |r| r.jd_tt_delta_sec,
    ));
    v.push(era_cat(
        "time_scale.tt (1972..2033-09-16, known TAI−UTC)",
        Some(0.01),
        TimeScaleEra::KnownTaiUtc,
        |r| r.jd_tt_delta_sec,
    ));
    v.push(era_cat(
        "time_scale.ut1 (1972..2033-09-16, known TAI−UTC)",
        Some(1.0),
        TimeScaleEra::KnownTaiUtc,
        |r| r.jd_ut_delta_sec,
    ));
    v.push(era_cat(
        "diag.time_scale.tt (2033-09-17.., extrapolated — reported, not gated)",
        None,
        TimeScaleEra::Extrapolated,
        |r| r.jd_tt_delta_sec,
    ));
    // Diagnostics (never gated): each engine at its OWN instant = ephemeris + convention folded.
    for id in PLANETS {
        let n = id.name();
        v.push(Category::simple(
            format!("diag.tropical_own_instant.{n}"),
            "arcsec",
            None,
            move |r| r.bodies[n].trop_delta_own,
        ));
    }
    v.push(Category::simple(
        "diag.sidereal_own_instant.Moon",
        "arcsec",
        None,
        |r| r.bodies["Moon"].sid_delta_own,
    ));
    v.push(Category::simple(
        "diag.nodes_own_instant.Rahu",
        "arcsec",
        None,
        |r| r.bodies["Rahu"].sid_delta_own,
    ));
    v.push(Category::simple(
        "diag.ascendant_own_instant",
        "deg",
        None,
        |r| r.asc_delta_own_deg,
    ));
    v.push(Category::simple(
        "diag.jd_ut_delta (XALEN UT1 − Swiss UT1, all eras)",
        "sec",
        None,
        |r| r.jd_ut_delta_sec,
    ));
    v.push(Category::simple(
        "diag.jd_tt_delta (XALEN TT − Swiss TT, all eras)",
        "sec",
        None,
        |r| r.jd_tt_delta_sec,
    ));
    v.push(Category::simple(
        "diag.nutation_dpsi_delta",
        "mas",
        None,
        |r| r.dpsi_delta_mas,
    ));
    v.push(Category::simple(
        "diag.rahu_ketu_180_consistency",
        "arcsec",
        None,
        |r| r.rahu_ketu_consistency_arcsec,
    ));
    v
}

fn csv_header() -> String {
    let mut cols = vec![
        "idx",
        "utc",
        "lat",
        "lon",
        "tz_offset_hours",
        "era",
        "attempts",
        "jd_ut1_xalen",
        "jd_ut_jhora",
        "jd_tt_xalen",
        "jd_tt_jhora",
        "jd_ut_delta_sec",
        "jd_tt_delta_sec",
        "delta_t_sigma_sec",
        "ayanamsa_true_xalen_deg",
        "ayanamsa_true_jhora_deg",
        "ayanamsa_true_delta_arcsec",
        "ayanamsa_mean_delta_arcsec",
        "nutation_dpsi_delta_mas",
        "asc_xalen_aligned_deg",
        "asc_jhora_deg",
        "asc_delta_deg",
        "asc_delta_own_instant_deg",
        "rahu_ketu_180_consistency_arcsec",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    for id in BodyId::GRAHAS {
        let n = id.name();
        for suffix in [
            "tropical_xalen_aligned_deg",
            "sidereal_xalen_aligned_deg",
            "sidereal_jhora_deg",
            "tropical_delta_arcsec",
            "sidereal_delta_arcsec",
            "tropical_delta_own_instant_arcsec",
            "sidereal_delta_own_instant_arcsec",
            "elongation_from_sun_deg",
            "tropical_tolerance_arcsec",
            "source",
            "retro_xalen",
            "retro_jhora",
        ] {
            cols.push(format!("{n}_{suffix}"));
        }
    }
    cols.join(",")
}

fn csv_row(r: &Record) -> String {
    let mut f: Vec<String> = vec![
        r.idx.to_string(),
        r.birth.utc.clone(),
        format!("{:.6}", r.birth.lat),
        format!("{:.6}", r.birth.lon),
        format!("{}", r.birth.tz_offset_hours),
        format!("\"{}\"", r.era),
        r.attempts.to_string(),
        format!("{:.9}", r.jd_ut1_x),
        format!("{:.9}", r.jd_ut_j),
        format!("{:.9}", r.jd_tt_x),
        format!("{:.9}", r.jd_tt_j),
        format!("{:.6}", r.jd_ut_delta_sec),
        format!("{:.6}", r.jd_tt_delta_sec),
        format!("{:.4}", r.delta_t_sigma_sec),
        format!("{:.9}", r.aya_true_x),
        format!("{:.9}", r.aya_true_j),
        format!("{:.6}", r.aya_true_delta),
        format!("{:.6}", r.aya_mean_delta),
        format!("{:.4}", r.dpsi_delta_mas),
        format!("{:.9}", r.asc_x),
        format!("{:.9}", r.asc_j),
        format!("{:.8}", r.asc_delta_deg),
        format!("{:.8}", r.asc_delta_own_deg),
        format!("{:.6}", r.rahu_ketu_consistency_arcsec),
    ];
    for id in BodyId::GRAHAS {
        let b = &r.bodies[id.name()];
        f.push(format!("{:.9}", b.xalen_trop));
        f.push(format!("{:.9}", b.xalen_sid));
        f.push(format!("{:.9}", b.jhora_sid));
        f.push(format!("{:.6}", b.trop_delta));
        f.push(format!("{:.6}", b.sid_delta));
        f.push(format!("{:.6}", b.trop_delta_own));
        f.push(format!("{:.6}", b.sid_delta_own));
        f.push(format!("{:.4}", b.elongation_deg));
        f.push(b.trop_tol.map(|t| t.to_string()).unwrap_or_default());
        f.push(b.source.to_string());
        f.push(b.xalen_retro.to_string());
        f.push(b.jhora_retro.to_string());
    }
    f.join(",")
}

fn fmt_tol(c: &Category) -> String {
    match c.tolerance {
        Some(t) => format!("{t} {}", c.unit),
        None => "— (diagnostic)".into(),
    }
}

fn md_stats_row(
    name: &str,
    unit: &str,
    s: &Summary,
    tol: Option<f64>,
    over: usize,
    p999_ok: bool,
    worst: Option<&Record>,
) -> String {
    let verdict = match tol {
        Some(t) if s.n == 0 => format!("n/a ({t} {unit})"),
        Some(t) if over == 0 && p999_ok => format!("PASS (≤ {t} {unit})"),
        Some(t) if over == 0 => format!("**FAIL** (p99.9 > {t} {unit})"),
        Some(t) => format!("**FAIL** ({over} over per-chart tol, nominal {t} {unit})"),
        None => "diagnostic".into(),
    };
    let worst_s = worst
        .map(|r| format!("{} @ {:.2},{:.2}", r.birth.utc, r.birth.lat, r.birth.lon))
        .unwrap_or_default();
    format!(
        "| {name} | {} | {:+.4} | {:.4} | {:.4} | {:.4} | {:.4} | {:.4} | {:.4} | {verdict} | {worst_s} |",
        s.n, s.mean_signed, s.mean_abs, s.rms, s.p50, s.p99, s.p99_9, s.max
    )
}

/// Per-category evaluation over the applicable records.
struct CatResult {
    summary: Summary,
    /// Charts whose |Δ| exceeds their own (per-chart) tolerance.
    over: usize,
    /// Charts that were held to a widened per-chart tolerance (conjunction band / analytic node).
    widened: usize,
    p999_ok: bool,
    pass: bool,
}

fn evaluate(c: &Category, records: &[Record]) -> CatResult {
    let vals: Vec<(usize, f64)> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| c.applies(r))
        .map(|(i, r)| (i, (c.get)(r)))
        .collect();
    let summary = summarize(&vals);
    let mut over = 0usize;
    let mut widened = 0usize;
    if c.is_gated() {
        for (i, v) in &vals {
            let r = &records[*i];
            let tol = c.chart_tol(r);
            if tol != c.tolerance {
                widened += 1;
            }
            if let Some(t) = tol
                && v.abs() > t
            {
                over += 1;
            }
        }
    }
    let p999_ok = match c.tolerance {
        Some(t) => summary.n == 0 || summary.p99_9 <= t,
        None => true,
    };
    let pass = !c.is_gated() || (over == 0 && p999_ok);
    CatResult {
        summary,
        over,
        widened,
        p999_ok,
        pass,
    }
}

#[tokio::main]
async fn main() {
    let args = parse_args();
    let t0 = StdInstant::now();
    eprintln!(
        "consensus-corpus: n={} seed={} years={}:{} jhora={} kernel={} concurrency={}",
        args.n,
        args.seed,
        args.year_from,
        args.year_to,
        args.jhora_url,
        args.kernel.display(),
        args.concurrency
    );

    let engine = match Engine::load(&args.kernel, &args.sha) {
        Ok(e) => Arc::new(e),
        Err(e) => {
            eprintln!("engine boot failed: {e}");
            std::process::exit(3);
        }
    };
    if let Err(e) = engine.golden_self_test() {
        eprintln!("golden self-test failed: {e}");
        std::process::exit(3);
    }
    eprintln!(
        "engine: {} sha256={} coverage_jd={:?} (boot {:.2}s)",
        engine.kernel_id,
        engine.kernel_sha256,
        engine.coverage_jd,
        t0.elapsed().as_secs_f64()
    );

    let http = reqwest::Client::builder()
        .timeout(StdDuration::from_millis(args.timeout_ms.max(100)))
        .connect_timeout(StdDuration::from_millis(1000))
        .no_proxy()
        .build()
        .expect("http client");
    let health_url = format!("{}/v1/health", args.jhora_url);
    match http.get(&health_url).send().await {
        Ok(r) if r.status().is_success() => {
            let h: Value = r.json().await.unwrap_or(Value::Null);
            eprintln!(
                "jhora-svc: ok ayanamsa={} true_nodes={} pyswisseph={} workers={}",
                h["ayanamsa"], h["true_nodes"], h["pyswisseph"], h["workers"]
            );
            if h["ayanamsa"] != "LAHIRI" || h["true_nodes"] != true {
                eprintln!(
                    "jhora-svc is not in the canonical LAHIRI/true-node configuration; refusing"
                );
                std::process::exit(3);
            }
        }
        other => {
            eprintln!("jhora-svc health check failed at {health_url}: {other:?}");
            std::process::exit(3);
        }
    }

    let samples = generate(&args);
    let positions_url = format!("{}/v1/positions", args.jhora_url);
    let sem = Arc::new(Semaphore::new(args.concurrency));
    let mut tasks = Vec::with_capacity(samples.len());
    let t1 = StdInstant::now();
    for s in samples {
        let sem = sem.clone();
        let http = http.clone();
        let engine = engine.clone();
        let url = positions_url.clone();
        let retries = args.retries;
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("semaphore");
            match fetch_positions(&http, &url, &s.birth, retries).await {
                Ok((resp, attempts)) => match compare(&engine, &s, &resp, attempts) {
                    Ok(rec) => Ok(rec),
                    Err(m) => Err(Failure {
                        idx: s.idx,
                        birth: s.birth.clone(),
                        stage: "compare",
                        message: m,
                        attempts,
                    }),
                },
                Err((m, attempts)) => Err(Failure {
                    idx: s.idx,
                    birth: s.birth.clone(),
                    stage: "jhora-svc",
                    message: m,
                    attempts,
                }),
            }
        }));
    }
    let mut records: Vec<Record> = Vec::with_capacity(tasks.len());
    let mut failures: Vec<Failure> = Vec::new();
    let mut done = 0usize;
    for t in tasks {
        match t.await {
            Ok(Ok(r)) => records.push(r),
            Ok(Err(f)) => failures.push(f),
            Err(e) => failures.push(Failure {
                idx: usize::MAX,
                birth: BirthInput {
                    utc: String::new(),
                    lat: 0.0,
                    lon: 0.0,
                    tz_offset_hours: 0.0,
                },
                stage: "task",
                message: e.to_string(),
                attempts: 0,
            }),
        }
        done += 1;
        if done.is_multiple_of(1000) {
            eprintln!(
                "  {done}/{} charts ({:.1}s)",
                args.n,
                t1.elapsed().as_secs_f64()
            );
        }
    }
    records.sort_by_key(|r| r.idx);
    failures.sort_by_key(|f| f.idx);
    let elapsed = t1.elapsed().as_secs_f64();
    let retried = records.iter().filter(|r| r.attempts > 1).count();
    eprintln!(
        "computed {} charts, {} failures, {} needed retries, {:.1}s ({:.1} charts/s)",
        records.len(),
        failures.len(),
        retried,
        elapsed,
        records.len() as f64 / elapsed.max(1e-9)
    );

    // ---- CSV
    if let Some(p) = args.csv.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    {
        let mut w = std::io::BufWriter::new(std::fs::File::create(&args.csv).expect("csv"));
        writeln!(w, "{}", csv_header()).unwrap();
        for r in &records {
            writeln!(w, "{}", csv_row(r)).unwrap();
        }
    }

    // ---- stats
    let cats = categories();
    let mut all_pass = true;
    let mut table_rows: Vec<String> = Vec::new();
    let mut p999_rows: Vec<String> = Vec::new();
    let mut machine_cats: Vec<Value> = Vec::new();
    for c in &cats {
        let res = evaluate(c, &records);
        if !res.pass {
            all_pass = false;
        }
        let worst = if res.summary.argmax < records.len() {
            Some(&records[res.summary.argmax])
        } else {
            None
        };
        table_rows.push(md_stats_row(
            &c.name,
            c.unit,
            &res.summary,
            c.tolerance,
            res.over,
            res.p999_ok,
            worst,
        ));
        if let Some(t) = c.tolerance {
            let status = if res.summary.n == 0 {
                "n/a (no charts in era)"
            } else if res.pass {
                "PASS"
            } else {
                "UNDER REVIEW"
            };
            let widened = if res.widened > 0 {
                format!(" ({} held to a widened per-chart tol)", res.widened)
            } else {
                String::new()
            };
            p999_rows.push(format!(
                "| {} | {} {} | {} | {:.4} | {:.4} | {}{} | {} |",
                c.name,
                t,
                c.unit,
                res.summary.n,
                res.summary.p99_9,
                res.summary.max,
                res.over,
                widened,
                status
            ));
        }
        machine_cats.push(json!({
            "name": c.name, "unit": c.unit, "tolerance": c.tolerance, "n": res.summary.n,
            "p99_9": res.summary.p99_9, "max": res.summary.max, "over": res.over, "widened": res.widened, "pass": res.pass,
        }));
    }

    // ---- era split (gated categories + the own-instant diagnostics; p99.9 / max / over)
    let mut era_rows: Vec<String> = Vec::new();
    for c in cats.iter().filter(|c| {
        c.is_gated()
            || c.name.starts_with("diag.time_scale")
            || c.name.starts_with("diag.tropical_own_instant.Moon")
            || c.name.starts_with("diag.ascendant_own_instant")
    }) {
        let mut cells = vec![c.name.clone()];
        for era in ERAS {
            let subset: Vec<Record> = records.iter().filter(|r| r.era == era).cloned().collect();
            let res = evaluate(c, &subset);
            cells.push(if res.summary.n == 0 {
                "n=0".into()
            } else {
                format!(
                    "n={} p99.9={:.3} max={:.3} over={}",
                    res.summary.n, res.summary.p99_9, res.summary.max, res.over
                )
            });
        }
        era_rows.push(format!("| {} |", cells.join(" | ")));
    }

    // ---- worst epochs: rank by max(|delta|/per-chart tol) over gated categories
    let gated: Vec<&Category> = cats.iter().filter(|c| c.is_gated()).collect();
    let mut ranked: Vec<(f64, usize, String)> = records
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (ratio, name, val, tol) = gated
                .iter()
                .filter(|c| c.applies(r))
                .filter_map(|c| {
                    let v = (c.get)(r);
                    c.chart_tol(r).map(|t| (v.abs() / t, c.name.as_str(), v, t))
                })
                .fold(
                    (0.0, "", 0.0, 0.0),
                    |acc, x| if x.0 > acc.0 { x } else { acc },
                );
            (ratio, i, format!("{name} = {val:+.4} (tol {tol})"))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let worst_rows: Vec<String> = ranked
        .iter()
        .take(10)
        .map(|(ratio, i, what)| {
            let r = &records[*i];
            format!(
                "| {} | {} | {:.3},{:.3} | {} | {:.2}× | {} | Moon trop {:+.3}″ / sid {:+.3}″, Rahu {:+.3}″ ({}), asc {:+.5}°, ΔTT {:+.3}s |",
                r.idx, r.birth.utc, r.birth.lat, r.birth.lon, r.era, ratio, what,
                r.bodies["Moon"].trop_delta, r.bodies["Moon"].sid_delta, r.bodies["Rahu"].sid_delta, r.bodies["Rahu"].source, r.asc_delta_deg, r.jd_tt_delta_sec
            )
        })
        .collect();

    // ---- time-scale characterisation
    let stat = |f: &dyn Fn(&Record) -> f64| {
        summarize(
            &records
                .iter()
                .enumerate()
                .map(|(i, r)| (i, f(r)))
                .collect::<Vec<_>>(),
        )
    };
    let asc_al = stat(&|r| r.asc_delta_deg);
    let asc_own = stat(&|r| r.asc_delta_own_deg);
    let jdut = stat(&|r| r.jd_ut_delta_sec);
    let jdtt = stat(&|r| r.jd_tt_delta_sec);
    let moon_al = stat(&|r| r.bodies["Moon"].trop_delta);
    let moon_own = stat(&|r| r.bodies["Moon"].trop_delta_own);
    let rahu_al = stat(&|r| r.bodies["Rahu"].sid_delta);
    let asc_attrib_max = records
        .iter()
        .map(|r| (r.asc_delta_own_deg - r.asc_delta_deg).abs())
        .fold(0.0, f64::max);
    let in_band: Vec<&Record> = records
        .iter()
        .filter(|r| r.bodies.values().any(|b| b.in_conjunction_band))
        .collect();
    let analytic_nodes = records
        .iter()
        .filter(|r| r.bodies["Rahu"].source != NODE_SOURCE_DE440)
        .count();
    let node_retro_mismatch = records
        .iter()
        .filter(|r| r.bodies["Rahu"].xalen_retro != r.bodies["Rahu"].jhora_retro)
        .count();

    let verdict = if !failures.is_empty() {
        "FAIL (charts could not be computed)"
    } else if all_pass {
        "PASS"
    } else {
        "FAIL (category over tolerance)"
    };
    let mut md = String::new();
    md.push_str("# Consensus corpus — XALEN-DE440 vs jhora-svc (PyJHora / Swiss Ephemeris)\n\n");
    md.push_str(&format!(
        "Generated by `validation/consensus` — n={} seed={} years={}..{} lat∈[−66,66] lon∈[−180,180] tz=round(lon/15) h; \
         kernel {} sha256 {}; jhora-svc {}; {} charts computed in {:.1}s ({} retried, {} failed).\n\n",
        args.n, args.seed, args.year_from, args.year_to, engine.kernel_id, engine.kernel_sha256, args.jhora_url,
        records.len(), elapsed, retried, failures.len()
    ));
    md.push_str(&format!("**Verdict: {verdict}**\n\n"));
    md.push_str("**Principle:** the gate measures ephemeris agreement, so every gated category evaluates XALEN at jhora-svc's own instant (its `jd_tt` for longitudes/ayanamsa, its `jd_ut` for the Ascendant); the time-scale conventions are gated separately per era (`time_scale.*`). Deltas are XALEN − jhora (signed in the CSV; order statistics on |delta|). Tropical = sidereal + each engine's own true-equinox ayanamsa. A category passes when no chart exceeds its per-chart tolerance AND p99.9 ≤ the nominal tolerance; the per-chart tolerance is widened only inside the solar-conjunction band (planets within 1.0° of the Sun: 2.5″, gravitational deflection not modelled in XALEN's DE440 chain) and for a chart whose node had to fall back to the analytic model (120″). `diag.` categories are never gated.\n\n");
    md.push_str("## Per-category p99.9 vs tolerance (the numbers docs/CONTRACT.md cites)\n\n");
    md.push_str("| category | tolerance | n | p99.9 | max | n over per-chart tol | status |\n|---|---|---|---|---|---|---|\n");
    for r in &p999_rows {
        md.push_str(r);
        md.push('\n');
    }
    md.push_str("\n## Full statistics\n\n");
    md.push_str("| category | n | mean (signed) | mean |Δ| | RMS | p50 | p99 | p99.9 | max | verdict | worst epoch |\n|---|---|---|---|---|---|---|---|---|---|---|\n");
    for r in &table_rows {
        md.push_str(r);
        md.push('\n');
    }
    md.push_str("\n## By time-scale era (p99.9 / max / count over per-chart tolerance)\n\n");
    md.push_str(&format!(
        "| category | {} | {} | {} |\n|---|---|---|---|\n",
        ERAS[0], ERAS[1], ERAS[2]
    ));
    for r in &era_rows {
        md.push_str(r);
        md.push('\n');
    }
    md.push_str("\n## Worst epochs (top 10 by |Δ|/per-chart tolerance over gated categories)\n\n");
    md.push_str(
        "| idx | utc | lat,lon | era | ratio | driver | context |\n|---|---|---|---|---|---|---|\n",
    );
    for r in &worst_rows {
        md.push_str(r);
        md.push('\n');
    }
    md.push_str("\n## Time-scale conventions (jd_ut / jd_tt) and their effect\n\n");
    md.push_str(&format!(
        "* `jd_ut` (UT1): XALEN uses UT1 = UTC; Swiss uses UT1 = TT − ΔT(table) inside the leap-second era and UT1 = UTC outside it. \
         Measured XALEN − Swiss over all eras: mean {:+.4} s, RMS {:.4} s, max |Δ| {:.4} s.\n",
        jdut.mean_signed, jdut.rms, jdut.max
    ));
    md.push_str(&format!(
        "* Ascendant: gated (XALEN at Swiss's UT1) p99.9 {:.5}° / max {:.5}°; at each engine's own UT1 (diagnostic) p99.9 {:.5}° / max {:.5}°. \
         The largest change attributable to the UT1 convention is {:.5}° ({:.2}″) against the 0.01° tolerance.\n",
        asc_al.p99_9, asc_al.max, asc_own.p99_9, asc_own.max, asc_attrib_max, asc_attrib_max * 3600.0
    ));
    md.push_str(&format!(
        "* `jd_tt` (TT): XALEN − Swiss over all eras: mean {:+.4} s, RMS {:.4} s, max |Δ| {:.4} s. Identical (0.000 s) inside 1972-01-01..2033-09-16; ΔT-table differences before 1972; Swiss's ΔT extrapolation vs XALEN's no-further-leap-seconds assumption (CGPM 2022) from 2033-09-17.\n",
        jdtt.mean_signed, jdtt.rms, jdtt.max
    ));
    md.push_str(&format!(
        "* Moon tropical |Δ|: gated (same TT) p99.9 {:.4}″ / max {:.4}″ — pure ephemeris disagreement, DE440 vs Swiss's DE431-based files; at each engine's own TT (diagnostic) p99.9 {:.4}″ / max {:.4}″.\n",
        moon_al.p99_9, moon_al.max, moon_own.p99_9, moon_own.max
    ));
    md.push_str(&format!(
        "* Rahu (DE440 osculating node vs Swiss true node, sidereal, includes the 0.73″ ayanamsa constant): p50 {:.3}″ p99 {:.3}″ p99.9 {:.3}″ max {:.3}″; {} charts fell back to the analytic node; retrograde flag differs in {} charts.\n",
        rahu_al.p50, rahu_al.p99, rahu_al.p99_9, rahu_al.max, analytic_nodes, node_retro_mismatch
    ));
    md.push_str(&format!(
        "* Solar-conjunction band (planet within 1.0° of the Sun): {} charts had at least one planet in the band and were held to the 2.5″ per-chart tropical tolerance there.\n",
        in_band.len()
    ));
    if !in_band.is_empty() {
        md.push_str(
            "\n| utc | body | elongation | tropical Δ | own-instant Δ |\n|---|---|---|---|---|\n",
        );
        for r in &in_band {
            for (n, b) in &r.bodies {
                if b.in_conjunction_band {
                    md.push_str(&format!(
                        "| {} | {n} | {:.3}° | {:+.3}″ | {:+.3}″ |\n",
                        r.birth.utc, b.elongation_deg, b.trop_delta, b.trop_delta_own
                    ));
                }
            }
        }
    }
    if !failures.is_empty() {
        md.push_str("\n## Charts that could not be computed (never dropped silently)\n\n| idx | utc | lat,lon | stage | attempts | message |\n|---|---|---|---|---|---|\n");
        for f in &failures {
            md.push_str(&format!(
                "| {} | {} | {:.3},{:.3} | {} | {} | {} |\n",
                f.idx,
                f.birth.utc,
                f.birth.lat,
                f.birth.lon,
                f.stage,
                f.attempts,
                f.message.replace('|', "\\|")
            ));
        }
    }
    md.push_str(&format!(
        "\nTolerances: {}.\n",
        cats.iter()
            .filter(|c| c.is_gated())
            .map(|c| format!("{} {}", c.name, fmt_tol(c)))
            .collect::<Vec<_>>()
            .join("; ")
    ));
    if let Some(p) = args.summary.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::write(&args.summary, &md).expect("summary");

    // ---- stdout
    println!("{md}");
    println!(
        "CSV: {}  SUMMARY: {}",
        args.csv.display(),
        args.summary.display()
    );
    println!("CONSENSUS_CORPUS_VERDICT: {verdict}");
    let machine = json!({
        "n": args.n, "seed": args.seed, "computed": records.len(), "failed": failures.len(), "retried": retried,
        "elapsed_sec": elapsed, "verdict": verdict, "categories": machine_cats,
    });
    println!("CONSENSUS_CORPUS_JSON: {machine}");
    if !failures.is_empty() {
        std::process::exit(2);
    }
    if !all_pass {
        std::process::exit(1);
    }
}

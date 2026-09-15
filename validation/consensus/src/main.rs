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
use jyotish_mcp::engine::{BodyId, Engine, Instant, UTC_LEAP_SECOND_ERA_START_JD};
use jyotish_mcp::tools::consensus::{
    ASC_TOL_DEG, AYANAMSA_TOL_ARCSEC, NODE_TOL_ARCSEC, sidereal_tolerance_arcsec,
    tropical_tolerance_arcsec,
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

/// JD(UTC) of the Swiss `swe_utc_to_jd` regime change, bisected against
/// pyswisseph 2.10.03 this session: from 2033-09-17T00:00Z (the first instant
/// where ΔT(model) − (TAI−UTC + 32.184 s) exceeds 1 s) Swiss abandons the
/// leap-second TT for `UTC + ΔT(model)`.
const SWISS_FUTURE_CUTOFF_JD: f64 = 2463857.5;

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
    xalen_trop: f64,
    xalen_sid: f64,
    jhora_sid: f64,
    /// Signed arcsec, XALEN − jhora.
    trop_delta: f64,
    sid_delta: f64,
    /// XALEN evaluated at Swiss's TT (isolates ΔT convention from ephemeris).
    trop_delta_tt_aligned: f64,
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
    asc_delta_deg: f64,
    asc_ut1_aligned_delta_deg: f64,
    rahu_ketu_consistency_arcsec: f64,
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

fn era_of(jd_utc: f64) -> &'static str {
    if jd_utc < UTC_LEAP_SECOND_ERA_START_JD {
        "pre-1972 (both: UT=UTC, TT=UT+ΔT model)"
    } else if jd_utc < SWISS_FUTURE_CUTOFF_JD {
        "1972..2033-09-16 (both: leap-second TT)"
    } else {
        "2033-09-17.. (Swiss: UTC+ΔT model; XALEN: leap-second TT)"
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
    let at = Instant::from_utc_fields(&fields);
    engine.check_coverage(&at).map_err(|e| e.to_string())?;

    let jd_ut_j = num(&resp["jd_ut"], "jd_ut")?;
    let jd_tt_j = num(&resp["jd_tt"], "jd_tt")?;
    let aya_true_j = num(
        &resp["ayanamsa_true_equinox_deg"],
        "ayanamsa_true_equinox_deg",
    )?;
    let aya_mean_j = num(&resp["ayanamsa_deg"], "ayanamsa_deg")?;
    let dpsi_j = num(&resp["nutation_dpsi_arcsec"], "nutation_dpsi_arcsec")?;
    let asc_j = num(&resp["ascendant"], "ascendant")?;

    let aya_true_x = engine.ayanamsa_deg(&at);
    let aya_mean_x = engine.ayanamsa_mean_equinox_deg(&at);
    let dpsi_x = engine.nutation_dpsi_deg(&at) * 3600.0;

    // XALEN at Swiss's time scales: same UT1 as Swiss (Ascendant), same TT as Swiss (bodies).
    let at_ut1_aligned = Instant::from_parts(jd_ut_j, at.jd_tt.0);
    let at_tt_aligned = Instant::from_parts(at.jd_ut1.0, jd_tt_j);

    let asc_x = engine
        .houses(&at, s.birth.lat, s.birth.lon, HouseSystem::WholeSign)
        .map_err(|e| e.to_string())?
        .ascendant_deg;
    let asc_x_aligned = engine
        .houses(
            &at_ut1_aligned,
            s.birth.lat,
            s.birth.lon,
            HouseSystem::WholeSign,
        )
        .map_err(|e| e.to_string())?
        .ascendant_deg;

    let mut bodies = BTreeMap::new();
    for id in BodyId::GRAHAS {
        let name = id.name();
        let st = engine.body_state(id, &at).map_err(|e| e.to_string())?;
        let st_tt = engine
            .body_state(id, &at_tt_aligned)
            .map_err(|e| e.to_string())?;
        let j = &resp["bodies"][name];
        let jhora_sid = num(&j["lon"], &format!("bodies.{name}.lon"))?;
        let jhora_trop = (jhora_sid + aya_true_j).rem_euclid(360.0);
        bodies.insert(
            name,
            PerBody {
                xalen_trop: st.tropical_lon_deg,
                xalen_sid: st.sidereal_lon_deg,
                jhora_sid,
                trop_delta: signed_delta_deg(st.tropical_lon_deg, jhora_trop) * 3600.0,
                sid_delta: signed_delta_deg(st.sidereal_lon_deg, jhora_sid) * 3600.0,
                trop_delta_tt_aligned: signed_delta_deg(st_tt.tropical_lon_deg, jhora_trop)
                    * 3600.0,
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
        era: era_of(at.jd_ut1.0),
        jd_ut1_x: at.jd_ut1.0,
        jd_ut_j,
        jd_tt_x: at.jd_tt.0,
        jd_tt_j,
        jd_ut_delta_sec: (at.jd_ut1.0 - jd_ut_j) * 86400.0,
        jd_tt_delta_sec: (at.jd_tt.0 - jd_tt_j) * 86400.0,
        delta_t_sigma_sec: at.delta_t_sigma_sec,
        aya_true_x,
        aya_true_j,
        aya_true_delta: signed_delta_deg(aya_true_x, aya_true_j) * 3600.0,
        aya_mean_delta: signed_delta_deg(aya_mean_x, aya_mean_j) * 3600.0,
        dpsi_delta_mas: (dpsi_x - dpsi_j) * 1000.0,
        asc_x,
        asc_j,
        asc_delta_deg: signed_delta_deg(asc_x, asc_j),
        asc_ut1_aligned_delta_deg: signed_delta_deg(asc_x_aligned, asc_j),
        rahu_ketu_consistency_arcsec,
        bodies,
        attempts,
    })
}

/// A category: how to read a signed delta off a record, its unit, tolerance
/// (None = diagnostic only), and whether it is part of the gate.
struct Category {
    name: String,
    unit: &'static str,
    tolerance: Option<f64>,
    get: Box<dyn Fn(&Record) -> f64 + Send + Sync>,
}

fn categories() -> Vec<Category> {
    let mut v: Vec<Category> = Vec::new();
    for id in PLANETS {
        let n = id.name();
        v.push(Category {
            name: format!("tropical.{n}"),
            unit: "arcsec",
            tolerance: tropical_tolerance_arcsec(id),
            get: Box::new(move |r| r.bodies[n].trop_delta),
        });
    }
    v.push(Category {
        name: "ayanamsa.true_equinox".into(),
        unit: "arcsec",
        tolerance: Some(AYANAMSA_TOL_ARCSEC),
        get: Box::new(|r| r.aya_true_delta),
    });
    v.push(Category {
        name: "ayanamsa.mean_equinox".into(),
        unit: "arcsec",
        tolerance: Some(AYANAMSA_TOL_ARCSEC),
        get: Box::new(|r| r.aya_mean_delta),
    });
    for id in PLANETS {
        let n = id.name();
        v.push(Category {
            name: format!("sidereal.{n}"),
            unit: "arcsec",
            tolerance: Some(sidereal_tolerance_arcsec(id)),
            get: Box::new(move |r| r.bodies[n].sid_delta),
        });
    }
    v.push(Category {
        name: "nodes.Rahu (source: analytic)".into(),
        unit: "arcsec",
        tolerance: Some(NODE_TOL_ARCSEC),
        get: Box::new(|r| r.bodies["Rahu"].sid_delta),
    });
    v.push(Category {
        name: "nodes.Ketu (source: analytic)".into(),
        unit: "arcsec",
        tolerance: Some(NODE_TOL_ARCSEC),
        get: Box::new(|r| r.bodies["Ketu"].sid_delta),
    });
    v.push(Category {
        name: "ascendant".into(),
        unit: "deg",
        tolerance: Some(ASC_TOL_DEG),
        get: Box::new(|r| r.asc_delta_deg),
    });
    // Diagnostics (never gated): isolate time-scale conventions.
    for id in PLANETS {
        let n = id.name();
        v.push(Category {
            name: format!("diag.tropical_tt_aligned.{n}"),
            unit: "arcsec",
            tolerance: None,
            get: Box::new(move |r| r.bodies[n].trop_delta_tt_aligned),
        });
    }
    v.push(Category {
        name: "diag.ascendant_ut1_aligned".into(),
        unit: "deg",
        tolerance: None,
        get: Box::new(|r| r.asc_ut1_aligned_delta_deg),
    });
    v.push(Category {
        name: "diag.jd_ut_delta (XALEN UT1 − Swiss UT1)".into(),
        unit: "sec",
        tolerance: None,
        get: Box::new(|r| r.jd_ut_delta_sec),
    });
    v.push(Category {
        name: "diag.jd_tt_delta (XALEN TT − Swiss TT)".into(),
        unit: "sec",
        tolerance: None,
        get: Box::new(|r| r.jd_tt_delta_sec),
    });
    v.push(Category {
        name: "diag.nutation_dpsi_delta".into(),
        unit: "mas",
        tolerance: None,
        get: Box::new(|r| r.dpsi_delta_mas),
    });
    v.push(Category {
        name: "diag.rahu_ketu_180_consistency".into(),
        unit: "arcsec",
        tolerance: None,
        get: Box::new(|r| r.rahu_ketu_consistency_arcsec),
    });
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
        "asc_xalen_deg",
        "asc_jhora_deg",
        "asc_delta_deg",
        "asc_ut1_aligned_delta_deg",
        "rahu_ketu_180_consistency_arcsec",
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    for id in BodyId::GRAHAS {
        let n = id.name();
        for suffix in [
            "tropical_xalen_deg",
            "sidereal_xalen_deg",
            "sidereal_jhora_deg",
            "tropical_delta_arcsec",
            "sidereal_delta_arcsec",
            "tropical_tt_aligned_delta_arcsec",
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
        format!("{:.8}", r.asc_ut1_aligned_delta_deg),
        format!("{:.6}", r.rahu_ketu_consistency_arcsec),
    ];
    for id in BodyId::GRAHAS {
        let b = &r.bodies[id.name()];
        f.push(format!("{:.9}", b.xalen_trop));
        f.push(format!("{:.9}", b.xalen_sid));
        f.push(format!("{:.9}", b.jhora_sid));
        f.push(format!("{:.6}", b.trop_delta));
        f.push(format!("{:.6}", b.sid_delta));
        f.push(format!("{:.6}", b.trop_delta_tt_aligned));
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
    worst: Option<&Record>,
) -> String {
    let verdict = match tol {
        Some(t) if s.p99_9.is_nan() => format!("n/a ({t} {unit})"),
        Some(t) if s.max <= t => format!("PASS (≤ {t} {unit})"),
        Some(t) => format!("**FAIL** ({over} over {t} {unit})"),
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
    let mut cat_summaries: Vec<(String, Summary, Option<f64>, usize)> = Vec::new();
    for c in &cats {
        let vals: Vec<(usize, f64)> = records
            .iter()
            .enumerate()
            .map(|(i, r)| (i, (c.get)(r)))
            .collect();
        let s = summarize(&vals);
        let over = match c.tolerance {
            Some(t) => vals.iter().filter(|(_, v)| v.abs() > t).count(),
            None => 0,
        };
        if c.tolerance.is_some() && over > 0 {
            all_pass = false;
        }
        let worst = if s.argmax < records.len() {
            Some(&records[s.argmax])
        } else {
            None
        };
        table_rows.push(md_stats_row(&c.name, c.unit, &s, c.tolerance, over, worst));
        if let Some(t) = c.tolerance {
            let status = if over == 0 { "PASS" } else { "UNDER REVIEW" };
            p999_rows.push(format!(
                "| {} | {} {} | {:.4} | {:.4} | {} | {} |",
                c.name, t, c.unit, s.p99_9, s.max, over, status
            ));
        }
        cat_summaries.push((c.name.clone(), s, c.tolerance, over));
    }

    // ---- era split (gated categories only; p99.9 and max)
    let eras = [
        "pre-1972 (both: UT=UTC, TT=UT+ΔT model)",
        "1972..2033-09-16 (both: leap-second TT)",
        "2033-09-17.. (Swiss: UTC+ΔT model; XALEN: leap-second TT)",
    ];
    let mut era_rows: Vec<String> = Vec::new();
    for c in cats.iter().filter(|c| {
        c.tolerance.is_some()
            || c.name.starts_with("diag.jd_tt")
            || c.name.starts_with("diag.tropical_tt_aligned.Moon")
    }) {
        let mut cells = vec![c.name.clone()];
        for era in eras {
            let vals: Vec<(usize, f64)> = records
                .iter()
                .enumerate()
                .filter(|(_, r)| r.era == era)
                .map(|(i, r)| (i, (c.get)(r)))
                .collect();
            let s = summarize(&vals);
            let over = match c.tolerance {
                Some(t) => vals.iter().filter(|(_, v)| v.abs() > t).count(),
                None => 0,
            };
            cells.push(if s.n == 0 {
                "n=0".into()
            } else {
                format!(
                    "n={} p99.9={:.3} max={:.3} over={over}",
                    s.n, s.p99_9, s.max
                )
            });
        }
        era_rows.push(format!("| {} |", cells.join(" | ")));
    }

    // ---- worst epochs: rank by max(|delta|/tol) over gated categories
    let gated: Vec<&Category> = cats.iter().filter(|c| c.tolerance.is_some()).collect();
    let mut ranked: Vec<(f64, usize, String)> = records
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (ratio, name, val, tol) = gated
                .iter()
                .map(|c| {
                    let v = (c.get)(r);
                    (
                        v.abs() / c.tolerance.unwrap(),
                        c.name.as_str(),
                        v,
                        c.tolerance.unwrap(),
                    )
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
                "| {} | {} | {:.3},{:.3} | {} | {:.2}× | {} | Moon trop {:+.3}″ / sid {:+.3}″, Rahu {:+.2}″, asc {:+.5}°, ΔTT {:+.3}s |",
                r.idx, r.birth.utc, r.birth.lat, r.birth.lon, r.era, ratio, what,
                r.bodies["Moon"].trop_delta, r.bodies["Moon"].sid_delta, r.bodies["Rahu"].sid_delta, r.asc_delta_deg, r.jd_tt_delta_sec
            )
        })
        .collect();

    // ---- ΔUT1 effect characterisation
    let asc_own = summarize(
        &records
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.asc_delta_deg))
            .collect::<Vec<_>>(),
    );
    let asc_al = summarize(
        &records
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.asc_ut1_aligned_delta_deg))
            .collect::<Vec<_>>(),
    );
    let jdut = summarize(
        &records
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.jd_ut_delta_sec))
            .collect::<Vec<_>>(),
    );
    let jdtt = summarize(
        &records
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.jd_tt_delta_sec))
            .collect::<Vec<_>>(),
    );
    let moon_own = summarize(
        &records
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.bodies["Moon"].trop_delta))
            .collect::<Vec<_>>(),
    );
    let moon_al = summarize(
        &records
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.bodies["Moon"].trop_delta_tt_aligned))
            .collect::<Vec<_>>(),
    );
    let asc_attrib: Vec<f64> = records
        .iter()
        .map(|r| (r.asc_delta_deg - r.asc_ut1_aligned_delta_deg).abs())
        .collect();
    let asc_attrib_max = asc_attrib.iter().cloned().fold(0.0, f64::max);

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
    md.push_str("Deltas are XALEN − jhora (signed in the CSV; order statistics below are on |delta|). Tropical = sidereal + each engine's own true-equinox ayanamsa. Categories marked `diag.` are diagnostics, never gated.\n\n");
    md.push_str("## Per-category p99.9 vs tolerance (the numbers docs/CONTRACT.md cites)\n\n");
    md.push_str(
        "| category | tolerance | p99.9 | max | n over tol | status |\n|---|---|---|---|---|---|\n",
    );
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
    md.push_str("\n## By time-scale era (p99.9 / max / count over tolerance)\n\n");
    md.push_str(&format!(
        "| category | {} | {} | {} |\n|---|---|---|---|\n",
        eras[0], eras[1], eras[2]
    ));
    for r in &era_rows {
        md.push_str(r);
        md.push('\n');
    }
    md.push_str("\n## Worst epochs (top 10 by |Δ|/tolerance over gated categories)\n\n");
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
         Measured XALEN − Swiss: mean {:+.4} s, RMS {:.4} s, max |Δ| {:.4} s.\n",
        jdut.mean_signed, jdut.rms, jdut.max
    ));
    md.push_str(&format!(
        "* Effect on the Ascendant: |Δasc| at each engine's own UT1 p99.9 {:.5}° / max {:.5}°; with XALEN evaluated at Swiss's UT1 p99.9 {:.5}° / max {:.5}°. \
         The largest change attributable to the UT1 convention is {:.5}° ({:.2}″), against a 0.01° tolerance.\n",
        asc_own.p99_9, asc_own.max, asc_al.p99_9, asc_al.max, asc_attrib_max, asc_attrib_max * 3600.0
    ));
    md.push_str(&format!(
        "* `jd_tt` (TT): XALEN − Swiss mean {:+.4} s, RMS {:.4} s, max |Δ| {:.4} s. Longitudes are evaluated in TT, so this — not UT1 — is what moves the Moon (≈0.55″/s).\n",
        jdtt.mean_signed, jdtt.rms, jdtt.max
    ));
    md.push_str(&format!(
        "* Moon tropical |Δ| at own TT: p99.9 {:.4}″ / max {:.4}″; with XALEN evaluated at Swiss's TT: p99.9 {:.4}″ / max {:.4}″ (pure ephemeris disagreement, DE440 vs Swiss's DE431-based files).\n",
        moon_own.p99_9, moon_own.max, moon_al.p99_9, moon_al.max
    ));
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
            .filter(|c| c.tolerance.is_some())
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
        "elapsed_sec": elapsed, "verdict": verdict,
        "categories": cat_summaries.iter().map(|(n, s, t, over)| json!({"name": n, "tolerance": t, "p99_9": s.p99_9, "max": s.max, "over": over})).collect::<Vec<_>>(),
    });
    println!("CONSENSUS_CORPUS_JSON: {machine}");
    if !failures.is_empty() {
        std::process::exit(2);
    }
    if !all_pass {
        std::process::exit(1);
    }
}

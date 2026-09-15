//! Boots the server in-process on a random loopback port and exercises the
//! MCP surface on the docs/CONTRACT.md golden chart. Requires the real
//! DE440 kernel at ../../kernels/de440s.bsp (a missing kernel is a FAILURE,
//! not a skip — the kernel is a hard requirement of the service).

use jyotish_mcp::config::Config;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

async fn boot_test_server() -> (String, Arc<jyotish_mcp::AppState>) {
    boot_test_server_with("http://127.0.0.1:1").await
}

/// Same boot, but pointing the jhora sidecar at a caller-supplied URL.
async fn boot_test_server_with(jhora_url: &str) -> (String, Arc<jyotish_mcp::AppState>) {
    let root = repo_root();
    // One audit file per booted server: the integration tests run in parallel
    // inside one process and must not interleave their audit lines.
    static BOOTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let boot_no = BOOTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let audit = std::env::temp_dir().join(format!(
        "jyotish-mcp-test-{}-{boot_no}.jsonl",
        std::process::id()
    ));
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        kernel_path: root.join("kernels/de440s.bsp"),
        kernel_sha_path: root.join("kernels/de440s.sha256"),
        // Port 1 on loopback: nothing listens there, so sidecar paths are
        // exercised in their "unavailable" branch deterministically.
        jhora_url: jhora_url.into(),
        vedastro_url: "http://127.0.0.1:1".into(),
        audit_path: audit,
        cache_cap: 64,
        sidecar_timeout_ms: if jhora_url.ends_with(":1") {
            500
        } else {
            10_000
        },
    };
    let state =
        jyotish_mcp::boot(config.clone()).expect("boot guard must pass with the real kernel");
    let (addr, _handle) = jyotish_mcp::serve(state.clone(), config.bind)
        .await
        .expect("bind");
    (format!("http://{addr}"), state)
}

async fn rpc(base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base}/mcp"))
        .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
        .send()
        .await
        .expect("http");
    assert_eq!(resp.status(), 200, "method {method}");
    resp.json().await.expect("json")
}

fn golden() -> Value {
    json!({ "utc": "1990-03-15T06:30:00Z", "lat": 28.6139, "lon": 77.2090, "tz_offset_hours": 5.5 })
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_lifecycle_on_golden_chart() {
    let (base, state) = boot_test_server().await;

    // health
    let h: Value = reqwest::get(format!("{base}/health"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(h["ok"], true);
    assert_eq!(h["kernel_id"], "DE440");
    assert_eq!(h["ayanamsa"], "LAHIRI");
    assert_eq!(h["golden"]["moon_nakshatra"], "Swati");

    // initialize
    let init = rpc(&base, 1, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } })).await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "jyotish-mcp");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    // notifications/initialized -> 202, empty body
    let resp = reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);

    // ping
    let ping = rpc(&base, 2, "ping", json!({})).await;
    assert_eq!(ping["result"], json!({}));

    // tools/list
    let list = rpc(&base, 3, "tools/list", json!({})).await;
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "chart.compute",
            "panchang.day",
            "dasha.timeline",
            "transit.window",
            "muhurta.find",
            "match.kuta",
            "rectify.birth_time",
            "rule.validate",
            "engine.consensus",
            "catalog.list"
        ]
    );

    // chart.compute on the golden chart
    let call = rpc(
        &base,
        4,
        "tools/call",
        json!({ "name": "chart.compute", "arguments": { "birth": golden(), "varga": "D9" } }),
    )
    .await;
    let r = &call["result"];
    assert_eq!(r["isError"], false, "{call}");
    assert_eq!(r["content"][0]["type"], "text");
    let text: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        &text, &r["structuredContent"],
        "text content must be the JSON of structuredContent"
    );
    let sc = &r["structuredContent"];
    let moon = sc["bodies"]["Moon"]["sidereal_lon_deg"].as_f64().unwrap();
    assert!(
        (186.6667..200.0).contains(&moon),
        "Moon {moon} must be in Swati"
    );
    assert_eq!(sc["bodies"]["Moon"]["nakshatra"], "Swati");
    assert_eq!(sc["bodies"]["Moon"]["rashi"], "Tula");
    assert!(
        sc["bodies"]["Moon"]["boundary_distance_sec"]["pada_sec"]
            .as_f64()
            .unwrap()
            > 0.0
    );
    assert_eq!(sc["bodies"]["Moon"]["retrograde"], false);
    // Phase 4b: the node comes from DE440's own lunar state vector when the
    // kernel is loaded (0.014″ from Swiss's true node on this chart).
    assert_eq!(sc["bodies"]["Rahu"]["source"], "de440-osculating");
    assert_eq!(sc["bodies"]["Ketu"]["source"], "de440-osculating");
    let rahu = sc["bodies"]["Rahu"]["sidereal_lon_deg"].as_f64().unwrap();
    let ketu = sc["bodies"]["Ketu"]["sidereal_lon_deg"].as_f64().unwrap();
    assert!(((rahu + 180.0).rem_euclid(360.0) - ketu).abs() < 1e-9);
    // Swiss SE_TRUE_NODE sidereal 292.05962552° PLUS the 0.731″ by which XALEN's
    // Lahiri sits below Swiss's (sidereal = tropical − ayanamsa) → 292.05983°.
    assert!((rahu - 292.05983).abs() < 0.5 / 3600.0, "Rahu {rahu}");
    assert_eq!(sc["varga"]["code"], "D9");
    assert!(sc["ascendant"]["sidereal_lon_deg"].is_number());
    assert_eq!(
        sc["houses"]["WholeSign"]["cusps_deg"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    assert_eq!(
        sc["houses"]["Sripati"]["cusps_deg"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    // Whole-sign cusp 1 is the start of the ascendant's sign.
    let asc = sc["ascendant"]["sidereal_lon_deg"].as_f64().unwrap();
    let c1 = sc["houses"]["WholeSign"]["cusps_deg"][0].as_f64().unwrap();
    assert!(
        (c1 - (asc / 30.0).floor() * 30.0).abs() < 1e-9,
        "asc {asc} cusp1 {c1}"
    );
    // evidence block
    let ev = &sc["evidence"];
    assert_eq!(ev["engine"], "xalen-de440");
    assert_eq!(ev["ayanamsa"], "LAHIRI");
    assert_eq!(ev["kernel_sha256"], json!(state.engine.kernel_sha256));
    assert_eq!(ev["consensus_status"], "SIDECAR_UNAVAILABLE");
    assert!(ev["delta_t_sigma_sec"].is_number());
    assert_eq!(ev["cache_hit"], false);

    // second identical call is a cache hit
    let call2 = rpc(
        &base,
        5,
        "tools/call",
        json!({ "name": "chart.compute", "arguments": { "varga": "D9", "birth": golden() } }),
    )
    .await;
    assert_eq!(
        call2["result"]["structuredContent"]["evidence"]["cache_hit"],
        true
    );
    assert_eq!(call2["result"]["structuredContent"]["bodies"], sc["bodies"]);

    // catalog excludes the placeholder systems
    let cat = rpc(
        &base,
        6,
        "tools/call",
        json!({ "name": "catalog.list", "arguments": {} }),
    )
    .await;
    let hs = serde_json::to_string(&cat["result"]["structuredContent"]["house_systems"]).unwrap();
    assert!(
        !hs.contains("Gauquelin") && !hs.contains("PullenSinusoidalRatio") && !hs.contains("QiMen")
    );

    // proxied tool with the sidecar down -> structured SIDECAR_UNAVAILABLE, isError
    let d = rpc(&base, 7, "tools/call", json!({ "name": "dasha.timeline", "arguments": { "birth": golden(), "system": "graha.vimsottari", "depth": 2 } })).await;
    assert_eq!(d["result"]["isError"], true);
    assert_eq!(
        d["result"]["structuredContent"]["error"]["code"],
        "SIDECAR_UNAVAILABLE"
    );

    // unknown top-level and nested keys are rejected per the published schema
    let u1 = rpc(
        &base,
        71,
        "tools/call",
        json!({ "name": "chart.compute", "arguments": { "birth": golden(), "bogus": 1 } }),
    )
    .await;
    assert_eq!(u1["error"]["code"], -32602, "{u1}");
    assert!(
        u1["error"]["message"]
            .as_str()
            .unwrap()
            .contains("arguments.bogus")
    );
    let mut b = golden();
    b["name"] = json!("x");
    let u2 = rpc(
        &base,
        72,
        "tools/call",
        json!({ "name": "chart.compute", "arguments": { "birth": b } }),
    )
    .await;
    assert!(
        u2["error"]["message"]
            .as_str()
            .unwrap()
            .contains("arguments.birth.name"),
        "{u2}"
    );

    // dasha id grammar: bare `vimsottari` is INVALID_PARAMS locally (never reaches the sidecar)
    let d2 = rpc(&base, 73, "tools/call", json!({ "name": "dasha.timeline", "arguments": { "birth": golden(), "system": "vimsottari" } })).await;
    assert_eq!(d2["result"]["isError"], true);
    assert_eq!(
        d2["result"]["structuredContent"]["error"]["code"], "INVALID_PARAMS",
        "{d2}"
    );

    // engine.consensus with the sidecar down -> not an error, status reported
    let c = rpc(
        &base,
        8,
        "tools/call",
        json!({ "name": "engine.consensus", "arguments": { "birth": golden() } }),
    )
    .await;
    assert_eq!(c["result"]["isError"], false);
    assert_eq!(
        c["result"]["structuredContent"]["consensus_status"],
        "SIDECAR_UNAVAILABLE"
    );

    // invalid params -> isError with INVALID_PARAMS (never a number)
    let bad = rpc(&base, 9, "tools/call", json!({ "name": "chart.compute", "arguments": { "birth": { "utc": "1990-03-15", "lat": 28.6, "lon": 77.2, "tz_offset_hours": 5.5 } } })).await;
    assert_eq!(bad["result"]["isError"], true);
    assert_eq!(
        bad["result"]["structuredContent"]["error"]["code"],
        "INVALID_PARAMS"
    );

    // outside kernel coverage -> KERNEL_COVERAGE, never analytic fallback
    let far = rpc(&base, 10, "tools/call", json!({ "name": "chart.compute", "arguments": { "birth": { "utc": "1400-01-01T00:00:00Z", "lat": 28.6, "lon": 77.2, "tz_offset_hours": 5.5 } } })).await;
    assert_eq!(
        far["result"]["structuredContent"]["error"]["code"],
        "KERNEL_COVERAGE"
    );

    // unknown method
    let um = rpc(&base, 11, "resources/list", json!({})).await;
    assert_eq!(um["error"]["code"], -32601);

    // audit log has one line per tool call
    let audit_text = std::fs::read_to_string(&state.config.audit_path).unwrap();
    let lines: Vec<&str> = audit_text.lines().collect();
    assert!(lines.len() >= 7, "audit lines: {}", lines.len());
    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["tool"], "chart.compute");
    assert_eq!(first["status"], "ok");
    assert!(
        !audit_text.contains("1990-03-15"),
        "audit log must not contain birth data"
    );
    let _ = std::fs::remove_file(&state.config.audit_path);
}

#[tokio::test(flavor = "multi_thread")]
async fn panchang_transit_rectify_on_golden() {
    let (base, _state) = boot_test_server().await;

    let p = rpc(&base, 20, "tools/call", json!({ "name": "panchang.day", "arguments": { "date": "1990-03-15", "lat": 28.6139, "lon": 77.2090, "tz_offset_hours": 5.5 } })).await;
    assert_eq!(p["result"]["isError"], false, "{p}");
    let sc = &p["result"]["structuredContent"];
    assert_eq!(sc["vara"]["weekday"], "Thursday");
    assert_eq!(sc["vara"]["name"], "Guruvara");
    let sunrise = sc["sunrise"]["jd"].as_f64().unwrap();
    let sunset = sc["sunset"]["jd"].as_f64().unwrap();
    let next = sc["next_sunrise"]["jd"].as_f64().unwrap();
    assert!(sunrise < sunset && sunset < next);
    assert!((next - sunrise - 1.0).abs() < 0.01);
    assert!(
        sc["sunrise"]["local"]
            .as_str()
            .unwrap()
            .starts_with("1990-03-15T06:"),
        "{}",
        sc["sunrise"]
    );
    let tithis = sc["tithi"].as_array().unwrap();
    assert!(!tithis.is_empty() && tithis.len() <= 3);
    let e0 = tithis[0]["end"]["jd"].as_f64().unwrap();
    assert!(e0 > sunrise);
    let naks = sc["nakshatra"].as_array().unwrap();
    assert!(naks.iter().any(|n| n["value"]["name"] == "Swati"));

    // transit window: Sun over one sidereal year must produce ~12 ingresses.
    let t = rpc(
        &base,
        21,
        "tools/call",
        json!({ "name": "transit.window", "arguments": {
        "birth": golden(), "from": "2024-01-01T00:00:00Z", "to": "2025-01-01T00:00:00Z",
        "bodies": ["Sun"], "natal_points": ["Moon"], "step_hours": 12 } }),
    )
    .await;
    assert_eq!(t["result"]["isError"], false, "{t}");
    let events = t["result"]["structuredContent"]["events"]
        .as_array()
        .unwrap();
    let ingresses = events.iter().filter(|e| e["type"] == "ingress").count();
    let conj = events.iter().filter(|e| e["type"] == "conjunction").count();
    assert!((12..=13).contains(&ingresses), "sun ingresses {ingresses}");
    assert_eq!(conj, 1, "Sun conjoins natal Moon exactly once a year");
    for e in events {
        if e["type"] == "ingress" {
            let lon = e["sidereal_lon_deg"].as_f64().unwrap();
            assert!((lon % 30.0).abs() < 1e-9);
        }
    }

    // rectify: heuristic flag and ranking shape
    let r = rpc(&base, 22, "tools/call", json!({ "name": "rectify.birth_time", "arguments": {
        "birth": golden(), "window_min": 10, "step_min": 5, "top_n": 3,
        "events": [ { "label": "marriage", "date": "2016-11-20", "significators": ["Venus", "Jupiter"] } ] } })).await;
    assert_eq!(r["result"]["isError"], false, "{r}");
    let sc = &r["result"]["structuredContent"];
    assert_eq!(sc["heuristic"], true);
    assert_eq!(sc["candidates_evaluated"], 5);
    assert_eq!(sc["candidates"].as_array().unwrap().len(), 3);
    let scores: Vec<u64> = sc["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["score"].as_u64().unwrap())
        .collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]));
    assert!(sc["candidates"][0]["events"][0]["maha"].is_string());
}

/// Success path for the proxied dasha tool. Runs only when a real jhora-svc
/// answers on JHORA_URL (default 127.0.0.1:7792); otherwise it is skipped
/// loudly so CI without the sidecar still passes but says so.
#[tokio::test]
async fn dasha_timeline_success_path_with_live_sidecar() {
    let jhora = std::env::var("JHORA_URL").unwrap_or_else(|_| "http://127.0.0.1:7792".into());
    let probe = reqwest::Client::new()
        .get(format!("{jhora}/v1/health"))
        .send()
        .await;
    if !probe.map(|r| r.status().is_success()).unwrap_or(false) {
        eprintln!("SKIP dasha_timeline_success_path_with_live_sidecar: no jhora-svc at {jhora}");
        return;
    }
    let (base, _guard) = boot_test_server_with(&jhora).await;
    let d = rpc(&base, 90, "tools/call", json!({ "name": "dasha.timeline", "arguments": { "birth": golden(), "system": "graha.vimsottari", "depth": 2 } })).await;
    assert_eq!(d["result"]["isError"], false, "{d}");
    let sc = &d["result"]["structuredContent"];
    let periods = sc["periods"].as_array().expect("periods array");
    assert_eq!(periods.len(), 9, "nine maha-dashas");
    assert_eq!(periods[0]["lord"], "Rahu");
    let years: f64 = periods
        .iter()
        .map(|p| p["duration_years"].as_f64().unwrap_or(0.0))
        .sum();
    assert!((years - 120.0).abs() < 0.01, "maha periods sum {years}");
    assert!(
        periods[0]["children"]
            .as_array()
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    );
    assert_eq!(sc["evidence"]["engine"], "jhora-svc");
}

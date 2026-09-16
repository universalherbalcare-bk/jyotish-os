//! jyotish-mcp — JYOTISH-OS L4 consensus server.
//!
//! Boot sequence (any failure aborts the process):
//! 1. Read config from env (loopback-only bind and sidecar URLs).
//! 2. Verify `kernels/de440s.bsp` against the pinned SHA-256, parse it, confirm
//!    DE440 provenance, build the almanac.
//! 3. Compute the GOLDEN chart (docs/CONTRACT.md) and assert Moon ∈ Swati.
//! 4. Open the append-only audit log and serve MCP over HTTPS (loopback,
//!    locally issued certificate; `JYOTISH_TLS=off` for plain HTTP).

pub mod audit;
pub mod cache;
pub mod config;
pub mod engine;
pub mod mcp;
pub mod sidecar;
pub mod tls;
pub mod tools;
pub mod types;

use axum::Router;
use axum::routing::{get, post};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::audit::Audit;
use crate::cache::Cache;
use crate::config::Config;
use crate::engine::{Engine, GoldenReport};
use crate::sidecar::SidecarClient;

pub struct AppState {
    pub engine: Arc<Engine>,
    pub sidecars: Arc<SidecarClient>,
    pub cache: Arc<Cache>,
    pub audit: Arc<Audit>,
    pub golden: GoldenReport,
    pub config: Config,
}

pub(crate) fn log(level: &str, msg: &str, extra: serde_json::Value) {
    let mut v = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "level": level,
        "service": mcp::SERVER_NAME,
        "msg": msg,
    });
    if let (Some(a), Some(b)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in b {
            a.insert(k.clone(), val.clone());
        }
    }
    eprintln!("{v}");
}

/// Run the boot guard and build the shared state.
pub fn boot(config: Config) -> Result<Arc<AppState>, Box<dyn std::error::Error + Send + Sync>> {
    log(
        "info",
        "boot: verifying kernel",
        serde_json::json!({ "kernel": config.kernel_path.display().to_string(), "sha_file": config.kernel_sha_path.display().to_string() }),
    );
    let engine = Engine::load(&config.kernel_path, &config.kernel_sha_path)?;
    log(
        "info",
        "boot: kernel verified",
        serde_json::json!({ "kernel_id": engine.kernel_id, "sha256": engine.kernel_sha256, "coverage_jd": engine.coverage_jd }),
    );
    let golden = engine.golden_self_test()?;
    log(
        "info",
        "boot: golden chart self-test passed",
        serde_json::json!({ "moon_sidereal_lon_deg": golden.moon_sidereal_lon_deg, "nakshatra": golden.moon_nakshatra, "ayanamsa_deg": golden.ayanamsa_deg }),
    );
    let sidecars = SidecarClient::new(
        &config.jhora_url,
        &config.vedastro_url,
        config.sidecar_timeout_ms,
    )?;
    let audit = Audit::open(&config.audit_path)?;
    Ok(Arc::new(AppState {
        engine: Arc::new(engine),
        sidecars: Arc::new(sidecars),
        cache: Arc::new(Cache::new(config.cache_cap)),
        audit: Arc::new(audit),
        golden,
        config,
    }))
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(mcp::health))
        .route("/mcp", post(mcp::post_mcp).get(mcp::get_mcp))
        .with_state(state)
}

/// Bind and serve until the future is dropped or a shutdown signal fires.
/// Returns the bound address (useful when `bind` has port 0).
pub async fn serve(
    state: Arc<AppState>,
    bind: SocketAddr,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>), Box<dyn std::error::Error + Send + Sync>> {
    if !bind.ip().is_loopback() {
        return Err(format!("refusing to bind non-loopback address {bind}").into());
    }
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    let app = router(state);
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log(
                "error",
                "server exited",
                serde_json::json!({ "error": e.to_string() }),
            );
        }
    });
    log(
        "info",
        "listening",
        serde_json::json!({ "addr": addr.to_string() }),
    );
    Ok((addr, handle))
}

/// Bind and serve over TLS. Same loopback-only rule as [`serve`]; the TLS
/// boot checks (key mode 0600, SAN covers 127.0.0.1) run inside
/// [`tls::load`] before the socket is opened, so a bad certificate never
/// leaves a half-open listener behind.
pub async fn serve_tls(
    state: Arc<AppState>,
    bind: SocketAddr,
    paths: &tls::TlsPaths,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>), Box<dyn std::error::Error + Send + Sync>> {
    if !bind.ip().is_loopback() {
        return Err(format!("refusing to bind non-loopback address {bind}").into());
    }
    let rustls_config = tls::load(paths).await?;
    let fingerprint = tls::cert_fingerprint_sha256(&paths.cert)?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    let std_listener = listener.into_std()?;
    let app = router(state);
    let server =
        axum_server::from_tcp(std_listener)?.acceptor(tls::LoggingAcceptor::new(rustls_config));
    let handle = tokio::spawn(async move {
        if let Err(e) = server.serve(app.into_make_service()).await {
            log(
                "error",
                "server exited",
                serde_json::json!({ "error": e.to_string() }),
            );
        }
    });
    log(
        "info",
        "listening",
        serde_json::json!({
            "addr": addr.to_string(),
            "tls": true,
            "cert": paths.cert.display().to_string(),
            "cert_sha256": fingerprint,
        }),
    );
    Ok((addr, handle))
}

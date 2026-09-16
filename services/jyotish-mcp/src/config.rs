//! Process configuration. Everything comes from environment variables; there
//! are no secrets (the service holds none) and no hard-coded remote hosts.
//!
//! Deployment modes (`JYOTISH_DEPLOYMENT`, Phase 8):
//! * `native` (default) — every sidecar URL and the bind address must be loopback.
//! * `compose` — the process runs inside its own container network namespace
//!   (deploy/compose.yaml). The sidecars are reachable only by their compose
//!   service names on the `internal: true` network, so `JHORA_URL` may name
//!   exactly `jhora-svc` and `VEDASTRO_URL` exactly `vedastro-svc` (loopback stays
//!   accepted). The listener may bind the unspecified address (`0.0.0.0` / `::`)
//!   because the only route into the namespace is the host-side publish, which
//!   compose pins to `127.0.0.1:7791`; any other non-loopback bind is still refused.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

/// Canonical defaults from docs/CONTRACT.md.
pub const DEFAULT_BIND: &str = "127.0.0.1:7791";
pub const DEFAULT_KERNEL: &str = "./kernels/de440s.bsp";
pub const DEFAULT_JHORA_URL: &str = "http://127.0.0.1:7792";
pub const DEFAULT_VEDASTRO_URL: &str = "http://127.0.0.1:7793";
pub const DEFAULT_AUDIT: &str = "./audit/jyotish-mcp.jsonl";
pub const DEFAULT_CACHE_CAP: usize = 4096;

#[derive(Debug, Clone)]
pub struct Config {
    pub deployment: Deployment,
    pub bind: SocketAddr,
    pub kernel_path: PathBuf,
    pub kernel_sha_path: PathBuf,
    pub jhora_url: String,
    pub vedastro_url: String,
    pub audit_path: PathBuf,
    pub cache_cap: usize,
    /// Per-request timeout for sidecar calls, milliseconds.
    pub sidecar_timeout_ms: u64,
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config error: {}", self.0)
    }
}
impl std::error::Error for ConfigError {}

fn env_or(name: &str, default: &str) -> String {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    }
}

/// Where the process runs; decides which non-loopback names the boot guard tolerates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deployment {
    Native,
    Compose,
}

impl Deployment {
    /// Parse `JYOTISH_DEPLOYMENT` (`native` when unset/empty). Any other value is a
    /// config error: a typo must never silently fall back to the permissive mode.
    pub fn parse(raw: &str) -> Result<Self, ConfigError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "native" => Ok(Self::Native),
            "compose" => Ok(Self::Compose),
            other => Err(ConfigError(format!(
                "JYOTISH_DEPLOYMENT must be native|compose (got {other:?})"
            ))),
        }
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        Self::parse(&std::env::var("JYOTISH_DEPLOYMENT").unwrap_or_default())
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Compose => "compose",
        }
    }
}

/// The one compose service name each sidecar variable may point at (deploy/compose.yaml).
/// `JHORA_URL` may never name `vedastro-svc` and vice versa.
pub fn compose_service_host(name: &str) -> Option<&'static str> {
    match name {
        "JHORA_URL" => Some("jhora-svc"),
        "VEDASTRO_URL" => Some("vedastro-svc"),
        _ => None,
    }
}

/// Extract the host part of an `http://` URL (no scheme other than http is accepted).
fn http_host(name: &str, url: &str) -> Result<String, ConfigError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| ConfigError(format!("{name} must start with http:// (got {url:?})")))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority
            .rsplit_once(':')
            .map(|(h, _)| h)
            .unwrap_or(authority)
    };
    Ok(host.to_string())
}

/// Reject any sidecar URL whose host is not a loopback address. The topology
/// (blueprint §2) forbids the consensus server from ever talking to a
/// non-loopback engine, and a misconfigured URL must fail closed at boot.
pub fn validate_loopback_url(name: &str, url: &str) -> Result<(), ConfigError> {
    validate_sidecar_url(name, url, Deployment::Native)
}

/// Deployment-aware sidecar URL guard. Loopback is always accepted; under
/// `Deployment::Compose` the exact compose service name for `name` is accepted
/// as well (`jhora-svc` for `JHORA_URL`, `vedastro-svc` for `VEDASTRO_URL`).
/// Everything else — other hostnames, FQDNs, private IPs — is refused in every mode.
pub fn validate_sidecar_url(name: &str, url: &str, mode: Deployment) -> Result<(), ConfigError> {
    let host = http_host(name, url)?;
    let loopback = match host.parse::<IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => host.eq_ignore_ascii_case("localhost"),
    };
    if loopback {
        return Ok(());
    }
    if mode == Deployment::Compose
        && compose_service_host(name).is_some_and(|svc| host.eq_ignore_ascii_case(svc))
    {
        return Ok(());
    }
    Err(ConfigError(match mode {
        Deployment::Native => format!(
            "{name} host {host:?} is not loopback; sidecars must bind 127.0.0.1 (docs/CONTRACT.md)"
        ),
        Deployment::Compose => format!(
            "{name} host {host:?} is neither loopback nor the compose service {:?} (deploy/compose.yaml)",
            compose_service_host(name).unwrap_or("<none>")
        ),
    }))
}

/// Bind-address guard. Native: loopback only. Compose: loopback or the
/// unspecified address (the container namespace is the boundary; the host
/// publish is `127.0.0.1:7791`). A concrete non-loopback interface address is
/// refused in both modes.
pub fn validate_bind(bind: SocketAddr, mode: Deployment) -> Result<(), ConfigError> {
    let ip = bind.ip();
    if ip.is_loopback() || (mode == Deployment::Compose && ip.is_unspecified()) {
        Ok(())
    } else {
        Err(ConfigError(format!(
            "JYOTISH_BIND {bind} is not a loopback address; refusing to expose the service (JYOTISH_DEPLOYMENT={})",
            mode.as_str()
        )))
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let deployment = Deployment::from_env()?;
        let bind: SocketAddr = env_or("JYOTISH_BIND", DEFAULT_BIND)
            .parse()
            .map_err(|e| ConfigError(format!("JYOTISH_BIND: {e}")))?;
        validate_bind(bind, deployment)?;
        let kernel_path = PathBuf::from(env_or("JYOTISH_KERNEL", DEFAULT_KERNEL));
        let kernel_sha_path = match std::env::var("JYOTISH_KERNEL_SHA256") {
            Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
            _ => kernel_path.with_extension("sha256"),
        };
        let jhora_url = env_or("JHORA_URL", DEFAULT_JHORA_URL)
            .trim_end_matches('/')
            .to_string();
        let vedastro_url = env_or("VEDASTRO_URL", DEFAULT_VEDASTRO_URL)
            .trim_end_matches('/')
            .to_string();
        validate_sidecar_url("JHORA_URL", &jhora_url, deployment)?;
        validate_sidecar_url("VEDASTRO_URL", &vedastro_url, deployment)?;
        let audit_path = PathBuf::from(env_or("JYOTISH_AUDIT", DEFAULT_AUDIT));
        let cache_cap = env_or("JYOTISH_CACHE_CAP", &DEFAULT_CACHE_CAP.to_string())
            .parse::<usize>()
            .map_err(|e| ConfigError(format!("JYOTISH_CACHE_CAP: {e}")))?;
        let sidecar_timeout_ms = env_or("JYOTISH_SIDECAR_TIMEOUT_MS", "5000")
            .parse::<u64>()
            .map_err(|e| ConfigError(format!("JYOTISH_SIDECAR_TIMEOUT_MS: {e}")))?;
        Ok(Self {
            deployment,
            bind,
            kernel_path,
            kernel_sha_path,
            jhora_url,
            vedastro_url,
            audit_path,
            cache_cap,
            sidecar_timeout_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_urls_accepted() {
        assert!(validate_loopback_url("X", "http://127.0.0.1:7792").is_ok());
        assert!(validate_loopback_url("X", "http://localhost:7793/").is_ok());
        assert!(validate_loopback_url("X", "http://[::1]:7793").is_ok());
    }

    #[test]
    fn non_loopback_urls_rejected() {
        assert!(validate_loopback_url("X", "http://10.0.0.5:7792").is_err());
        assert!(validate_loopback_url("X", "http://example.com").is_err());
        assert!(validate_loopback_url("X", "https://127.0.0.1:7792").is_err());
        assert!(validate_loopback_url("X", "127.0.0.1:7792").is_err());
    }

    #[test]
    fn deployment_parse_is_strict() {
        assert_eq!(Deployment::parse("").unwrap(), Deployment::Native);
        assert_eq!(Deployment::parse("native").unwrap(), Deployment::Native);
        assert_eq!(Deployment::parse(" Compose ").unwrap(), Deployment::Compose);
        assert!(Deployment::parse("docker").is_err());
        assert!(Deployment::parse("compose-ish").is_err());
    }

    #[test]
    fn compose_accepts_only_the_matching_service_name() {
        let c = Deployment::Compose;
        assert!(validate_sidecar_url("JHORA_URL", "http://jhora-svc:7792", c).is_ok());
        assert!(validate_sidecar_url("VEDASTRO_URL", "http://vedastro-svc:7793", c).is_ok());
        // loopback stays accepted under compose
        assert!(validate_sidecar_url("JHORA_URL", "http://127.0.0.1:7792", c).is_ok());
        // cross-wired names are refused
        assert!(validate_sidecar_url("JHORA_URL", "http://vedastro-svc:7793", c).is_err());
        assert!(validate_sidecar_url("VEDASTRO_URL", "http://jhora-svc:7792", c).is_err());
        // look-alikes, FQDNs, private IPs, https are refused
        assert!(validate_sidecar_url("JHORA_URL", "http://jhora-svc.example.com:7792", c).is_err());
        assert!(validate_sidecar_url("JHORA_URL", "http://jhora-svc2:7792", c).is_err());
        assert!(validate_sidecar_url("JHORA_URL", "http://172.28.0.2:7792", c).is_err());
        assert!(validate_sidecar_url("JHORA_URL", "https://jhora-svc:7792", c).is_err());
        // an unknown variable name has no compose service and is refused
        assert!(validate_sidecar_url("OTHER_URL", "http://jhora-svc:7792", c).is_err());
    }

    #[test]
    fn native_rejects_compose_service_names() {
        let n = Deployment::Native;
        assert!(validate_sidecar_url("JHORA_URL", "http://jhora-svc:7792", n).is_err());
        assert!(validate_sidecar_url("VEDASTRO_URL", "http://vedastro-svc:7793", n).is_err());
    }

    #[test]
    fn bind_guard_per_mode() {
        let lo: SocketAddr = "127.0.0.1:7791".parse().unwrap();
        let any4: SocketAddr = "0.0.0.0:7791".parse().unwrap();
        let any6: SocketAddr = "[::]:7791".parse().unwrap();
        let lan: SocketAddr = "192.168.1.10:7791".parse().unwrap();
        assert!(validate_bind(lo, Deployment::Native).is_ok());
        assert!(validate_bind(any4, Deployment::Native).is_err());
        assert!(validate_bind(any6, Deployment::Native).is_err());
        assert!(validate_bind(lan, Deployment::Native).is_err());
        assert!(validate_bind(lo, Deployment::Compose).is_ok());
        assert!(validate_bind(any4, Deployment::Compose).is_ok());
        assert!(validate_bind(any6, Deployment::Compose).is_ok());
        assert!(validate_bind(lan, Deployment::Compose).is_err());
    }
}

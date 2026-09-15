//! Process configuration. Everything comes from environment variables; there
//! are no secrets (the service holds none) and no hard-coded remote hosts.

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

/// Reject any sidecar URL whose host is not a loopback address. The topology
/// (blueprint §2) forbids the consensus server from ever talking to a
/// non-loopback engine, and a misconfigured URL must fail closed at boot.
pub fn validate_loopback_url(name: &str, url: &str) -> Result<(), ConfigError> {
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
    let ok = match host.parse::<IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => host.eq_ignore_ascii_case("localhost"),
    };
    if ok {
        Ok(())
    } else {
        Err(ConfigError(format!(
            "{name} host {host:?} is not loopback; sidecars must bind 127.0.0.1 (docs/CONTRACT.md)"
        )))
    }
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let bind: SocketAddr = env_or("JYOTISH_BIND", DEFAULT_BIND)
            .parse()
            .map_err(|e| ConfigError(format!("JYOTISH_BIND: {e}")))?;
        if !bind.ip().is_loopback() {
            return Err(ConfigError(format!(
                "JYOTISH_BIND {bind} is not a loopback address; refusing to expose the service"
            )));
        }
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
        validate_loopback_url("JHORA_URL", &jhora_url)?;
        validate_loopback_url("VEDASTRO_URL", &vedastro_url)?;
        let audit_path = PathBuf::from(env_or("JYOTISH_AUDIT", DEFAULT_AUDIT));
        let cache_cap = env_or("JYOTISH_CACHE_CAP", &DEFAULT_CACHE_CAP.to_string())
            .parse::<usize>()
            .map_err(|e| ConfigError(format!("JYOTISH_CACHE_CAP: {e}")))?;
        let sidecar_timeout_ms = env_or("JYOTISH_SIDECAR_TIMEOUT_MS", "5000")
            .parse::<u64>()
            .map_err(|e| ConfigError(format!("JYOTISH_SIDECAR_TIMEOUT_MS: {e}")))?;
        Ok(Self {
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
}

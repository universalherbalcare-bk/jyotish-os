//! TLS for the loopback listener (Phase 6).
//!
//! IronClaw types a hosted-MCP endpoint as https-only, so `jyotish-mcp` serves
//! HTTPS on 127.0.0.1:7791 with a locally issued certificate
//! (`scripts/gen-cert.sh`). Nothing else changes: the same router, the same
//! loopback-only bind check, the same audit log.
//!
//! Boot guard (fails closed, exit 2 from `main`):
//! * the private key must not be readable by group/other (mode `0600`);
//! * the leaf certificate's SAN must contain `IP:127.0.0.1` — the address
//!   IronClaw connects to — otherwise every client handshake would fail
//!   verification and the server would be up for nothing.
//!
//! `JYOTISH_TLS=off` keeps plain HTTP (used by the in-process integration
//! tests, which never read this module).

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, ServerName};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

use axum_server::accept::Accept;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;

pub const DEFAULT_CERT: &str = "./certs/jyotish-mcp.crt";
pub const DEFAULT_KEY: &str = "./certs/jyotish-mcp.key";
/// The address IronClaw dials; the SAN must cover it.
pub const REQUIRED_SAN_IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

#[derive(Debug)]
pub struct TlsError(pub String);

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tls error: {}", self.0)
    }
}
impl std::error::Error for TlsError {}

/// Where the PEM files live. `None` means plain HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsPaths {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// Parse `JYOTISH_TLS` / `JYOTISH_TLS_CERT` / `JYOTISH_TLS_KEY`.
/// TLS is on unless `JYOTISH_TLS` is exactly `off` / `0` / `false` (case-insensitive).
pub fn tls_paths_from_env() -> Result<Option<TlsPaths>, TlsError> {
    let mode = std::env::var("JYOTISH_TLS").unwrap_or_default();
    match mode.trim().to_ascii_lowercase().as_str() {
        "" | "on" | "1" | "true" => {}
        "off" | "0" | "false" => return Ok(None),
        other => {
            return Err(TlsError(format!(
                "JYOTISH_TLS must be on|off (got {other:?})"
            )));
        }
    }
    let env_or = |name: &str, default: &str| match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    };
    Ok(Some(TlsPaths {
        cert: PathBuf::from(env_or("JYOTISH_TLS_CERT", DEFAULT_CERT)),
        key: PathBuf::from(env_or("JYOTISH_TLS_KEY", DEFAULT_KEY)),
    }))
}

/// Refuse a private key that group or other can read (or write).
/// `0600` is the only accepted shape; `0640`, `0644`, `0660` all fail.
pub fn check_key_permissions(key: &Path) -> Result<(), TlsError> {
    let meta = std::fs::metadata(key)
        .map_err(|e| TlsError(format!("private key {}: {e}", key.display())))?;
    if !meta.is_file() {
        return Err(TlsError(format!(
            "private key {} is not a regular file",
            key.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(TlsError(format!(
                "private key {} has mode {mode:04o}; it must be 0600 (chmod 600)",
                key.display()
            )));
        }
    }
    Ok(())
}

/// Read the leaf certificate and confirm its SAN covers `127.0.0.1`.
/// Uses webpki's own name matching, i.e. the same rule a rustls client applies.
pub fn check_cert_san(cert: &Path) -> Result<(), TlsError> {
    let pem = std::fs::read(cert)
        .map_err(|e| TlsError(format!("certificate {}: {e}", cert.display())))?;
    check_cert_san_pem(&pem).map_err(|e| TlsError(format!("{}: {}", cert.display(), e.0)))
}

pub fn check_cert_san_pem(pem: &[u8]) -> Result<(), TlsError> {
    let leaf: CertificateDer<'_> = CertificateDer::pem_slice_iter(pem)
        .next()
        .ok_or_else(|| TlsError("no CERTIFICATE block in PEM".into()))?
        .map_err(|e| TlsError(format!("certificate PEM parse: {e}")))?;
    let ee = webpki::EndEntityCert::try_from(&leaf)
        .map_err(|e| TlsError(format!("certificate DER parse: {e}")))?;
    let name = ServerName::IpAddress(REQUIRED_SAN_IP.into());
    ee.verify_is_valid_for_subject_name(&name)
        .map_err(|e| TlsError(format!("SAN does not cover IP:{REQUIRED_SAN_IP} ({e})")))
}

/// Run every boot check and build the rustls server config.
pub async fn load(paths: &TlsPaths) -> Result<RustlsConfig, TlsError> {
    check_key_permissions(&paths.key)?;
    check_cert_san(&paths.cert)?;
    RustlsConfig::from_pem_file(&paths.cert, &paths.key)
        .await
        .map_err(|e| TlsError(format!("loading cert/key: {e}")))
}

/// SHA-256 fingerprint of the leaf certificate (for logs / `openssl x509 -fingerprint`).
pub fn cert_fingerprint_sha256(cert: &Path) -> Result<String, TlsError> {
    use sha2::{Digest, Sha256};
    let pem = std::fs::read(cert)
        .map_err(|e| TlsError(format!("certificate {}: {e}", cert.display())))?;
    let leaf = CertificateDer::pem_slice_iter(&pem)
        .next()
        .ok_or_else(|| TlsError("no CERTIFICATE block in PEM".into()))?
        .map_err(|e| TlsError(format!("certificate PEM parse: {e}")))?;
    let digest = Sha256::digest(leaf.as_ref());
    Ok(digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":"))
}

/// Wraps the rustls acceptor so every completed handshake leaves one
/// structured line on stderr (peer, protocol, cipher suite, SNI). Failed
/// handshakes are logged too; the connection is dropped either way.
#[derive(Clone)]
pub struct LoggingAcceptor {
    inner: RustlsAcceptor,
}

impl LoggingAcceptor {
    pub fn new(config: RustlsConfig) -> Self {
        Self {
            inner: RustlsAcceptor::new(config),
        }
    }
}

impl<S> Accept<TcpStream, S> for LoggingAcceptor
where
    S: Send + 'static,
{
    type Stream = TlsStream<TcpStream>;
    type Service = S;
    type Future = std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::io::Result<(Self::Stream, Self::Service)>> + Send,
        >,
    >;

    fn accept(&self, stream: TcpStream, service: S) -> Self::Future {
        let peer = stream
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "?".into());
        let fut = self.inner.accept(stream, service);
        Box::pin(async move {
            match fut.await {
                Ok((tls, svc)) => {
                    let conn = tls.get_ref().1;
                    crate::log(
                        "info",
                        "tls handshake",
                        serde_json::json!({
                            "peer": peer,
                            "protocol": conn.protocol_version().map(|v| format!("{v:?}")),
                            "cipher": conn.negotiated_cipher_suite().map(|c| format!("{:?}", c.suite())),
                            "sni": conn.server_name(),
                        }),
                    );
                    Ok((tls, svc))
                }
                Err(e) => {
                    crate::log(
                        "warn",
                        "tls handshake failed",
                        serde_json::json!({ "peer": peer, "error": e.to_string() }),
                    );
                    Err(e)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Public leaf certificates only (no keys) generated once with openssl;
    /// see tests/fixtures/README.md.
    const WITH_IP_SAN: &str = include_str!("../tests/fixtures/san-ip-127.crt");
    const WITHOUT_IP_SAN: &str = include_str!("../tests/fixtures/san-dns-only.crt");

    #[test]
    fn san_with_loopback_ip_accepted() {
        check_cert_san_pem(WITH_IP_SAN.as_bytes()).expect("IP:127.0.0.1 present");
    }

    #[test]
    fn san_without_loopback_ip_rejected() {
        let err = check_cert_san_pem(WITHOUT_IP_SAN.as_bytes()).unwrap_err();
        assert!(err.0.contains("SAN does not cover IP:127.0.0.1"), "{err}");
    }

    #[test]
    fn garbage_pem_rejected() {
        assert!(check_cert_san_pem(b"not a certificate").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn key_permissions_enforced() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("jyotish-tls-perm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("k.pem");
        std::fs::write(&key, b"placeholder").unwrap();
        for bad in [0o644u32, 0o640, 0o660, 0o604, 0o601] {
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(bad)).unwrap();
            let err = check_key_permissions(&key).unwrap_err();
            assert!(err.0.contains("must be 0600"), "mode {bad:o}: {err}");
        }
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        check_key_permissions(&key).unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o400)).unwrap();
        check_key_permissions(&key).unwrap();
        assert!(
            check_key_permissions(&dir).is_err(),
            "directory is not a key"
        );
        assert!(check_key_permissions(&dir.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_parsing() {
        // Serialise env mutation: cargo runs tests in threads.
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap();
        // SAFETY: guarded by LOCK; no other test in this module touches these vars.
        unsafe {
            std::env::remove_var("JYOTISH_TLS_CERT");
            std::env::remove_var("JYOTISH_TLS_KEY");
            std::env::set_var("JYOTISH_TLS", "off");
        }
        assert_eq!(tls_paths_from_env().unwrap(), None);
        unsafe { std::env::set_var("JYOTISH_TLS", "on") };
        assert_eq!(
            tls_paths_from_env().unwrap(),
            Some(TlsPaths {
                cert: PathBuf::from(DEFAULT_CERT),
                key: PathBuf::from(DEFAULT_KEY)
            })
        );
        unsafe { std::env::set_var("JYOTISH_TLS", "maybe") };
        assert!(tls_paths_from_env().is_err());
        unsafe { std::env::remove_var("JYOTISH_TLS") };
    }

    #[test]
    fn fingerprint_is_sha256_colon_hex() {
        let dir = std::env::temp_dir().join(format!("jyotish-tls-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("c.crt");
        std::fs::write(&p, WITH_IP_SAN).unwrap();
        let fp = cert_fingerprint_sha256(&p).unwrap();
        assert_eq!(fp.len(), 32 * 3 - 1, "{fp}");
        assert!(
            fp.split(':')
                .all(|h| h.len() == 2 && u8::from_str_radix(h, 16).is_ok())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

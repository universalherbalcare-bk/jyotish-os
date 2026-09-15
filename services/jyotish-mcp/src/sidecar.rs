//! HTTP clients for the two loopback sidecars (jhora-svc on 7792,
//! vedastro-svc on 7793). A sidecar that is down yields a structured
//! `SIDECAR_UNAVAILABLE` error — never fabricated data.

use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::{Duration, Instant as StdInstant};

use crate::types::ToolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sidecar {
    Jhora,
    Vedastro,
}

impl Sidecar {
    pub fn name(self) -> &'static str {
        match self {
            Sidecar::Jhora => "jhora-svc",
            Sidecar::Vedastro => "vedastro-svc",
        }
    }
}

const MAX_ERROR_BODY: usize = 512;
const HEALTH_MEMO_TTL: Duration = Duration::from_secs(30);

struct HealthMemo {
    checked_at: StdInstant,
    up: bool,
}

pub struct SidecarClient {
    http: reqwest::Client,
    jhora_url: String,
    vedastro_url: String,
    jhora_health: Mutex<Option<HealthMemo>>,
}

impl SidecarClient {
    pub fn new(jhora_url: &str, vedastro_url: &str, timeout_ms: u64) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout_ms.max(100)))
            .connect_timeout(Duration::from_millis(1000))
            .no_proxy()
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self {
            http,
            jhora_url: jhora_url.to_string(),
            vedastro_url: vedastro_url.to_string(),
            jhora_health: Mutex::new(None),
        })
    }

    pub fn base_url(&self, s: Sidecar) -> &str {
        match s {
            Sidecar::Jhora => &self.jhora_url,
            Sidecar::Vedastro => &self.vedastro_url,
        }
    }

    fn unavailable(s: Sidecar, url: &str, detail: String) -> ToolError {
        ToolError::new(
            "SIDECAR_UNAVAILABLE",
            format!("{} unreachable: {detail}", s.name()),
        )
        .with_details(json!({ "sidecar": s.name(), "url": url }))
    }

    /// POST JSON to `<base>/<path>` and return the parsed JSON body.
    pub async fn post_json(
        &self,
        s: Sidecar,
        path: &str,
        body: &Value,
    ) -> Result<Value, ToolError> {
        let url = format!("{}{}", self.base_url(s), path);
        let resp = self
            .http
            .post(&url)
            .json(body)
            .send()
            .await
            .map_err(|e| Self::unavailable(s, &url, e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| Self::unavailable(s, &url, format!("body read: {e}")))?;
        if !status.is_success() {
            let snippet: String = text.chars().take(MAX_ERROR_BODY).collect();
            return Err(ToolError::new(
                "SIDECAR_ERROR",
                format!("{} returned HTTP {}", s.name(), status.as_u16()),
            )
            .with_details(json!({ "sidecar": s.name(), "url": url, "status": status.as_u16(), "body": snippet })));
        }
        serde_json::from_str::<Value>(&text).map_err(|e| {
            ToolError::new(
                "SIDECAR_ERROR",
                format!("{} returned non-JSON body: {e}", s.name()),
            )
            .with_details(json!({ "sidecar": s.name(), "url": url }))
        })
    }

    /// GET `<base>/<path>` as JSON.
    pub async fn get_json(&self, s: Sidecar, path: &str) -> Result<Value, ToolError> {
        let url = format!("{}{}", self.base_url(s), path);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| Self::unavailable(s, &url, e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ToolError::new(
                "SIDECAR_ERROR",
                format!("{} returned HTTP {}", s.name(), status.as_u16()),
            )
            .with_details(json!({ "sidecar": s.name(), "url": url, "status": status.as_u16() })));
        }
        resp.json::<Value>().await.map_err(|e| {
            ToolError::new("SIDECAR_ERROR", format!("{} non-JSON: {e}", s.name()))
                .with_details(json!({ "sidecar": s.name(), "url": url }))
        })
    }

    /// Memoised liveness of jhora-svc (30 s TTL) so chart tools do not pay a
    /// connect-refused round trip on every call while the sidecar is down.
    pub async fn jhora_is_up(&self) -> bool {
        if let Ok(g) = self.jhora_health.lock()
            && let Some(m) = g.as_ref()
            && m.checked_at.elapsed() < HEALTH_MEMO_TTL
        {
            return m.up;
        }
        let up = self.get_json(Sidecar::Jhora, "/v1/health").await.is_ok();
        if let Ok(mut g) = self.jhora_health.lock() {
            *g = Some(HealthMemo {
                checked_at: StdInstant::now(),
                up,
            });
        }
        up
    }

    /// Liveness of vedastro-svc, not memoised (only `/health` calls it).
    pub async fn vedastro_is_up(&self) -> bool {
        self.get_json(Sidecar::Vedastro, "/v1/health").await.is_ok()
    }
}

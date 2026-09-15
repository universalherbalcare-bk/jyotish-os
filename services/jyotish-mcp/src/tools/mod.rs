//! Tool implementations. Each tool takes validated JSON arguments and returns
//! a `ToolOutput`; the MCP layer wraps it in content + evidence.

pub mod catalog;
pub mod chart;
pub mod consensus;
pub mod panchang;
pub mod proxy;
pub mod rectify;
pub mod transit;

use serde_json::Value;
use std::sync::Arc;

use crate::cache::{Cache, request_key};
use crate::engine::Engine;
use crate::sidecar::SidecarClient;
use crate::types::ToolError;

pub struct Ctx {
    pub engine: Arc<Engine>,
    pub sidecars: Arc<SidecarClient>,
    pub cache: Arc<Cache>,
}

pub struct ToolOutput {
    pub payload: Value,
    /// Which engine produced the numbers in `payload`.
    pub engine: &'static str,
    pub consensus_status: String,
    pub delta_t_sigma_sec: Option<f64>,
    pub cache_hit: bool,
}

impl Ctx {
    /// Content-addressed memoisation for pure (deterministic, kernel-bound)
    /// computations. Returns `(value, cache_hit)`.
    pub fn cached<F>(
        &self,
        tool: &str,
        args: &Value,
        compute: F,
    ) -> Result<(Value, bool), ToolError>
    where
        F: FnOnce() -> Result<Value, ToolError>,
    {
        let key = request_key(tool, args, &self.engine.kernel_sha256);
        if let Some(v) = self.cache.get(&key) {
            return Ok((v, true));
        }
        let v = compute()?;
        self.cache.put(key, v.clone());
        Ok((v, false))
    }
}

pub const TOOL_NAMES: [&str; 10] = [
    "chart.compute",
    "panchang.day",
    "dasha.timeline",
    "transit.window",
    "muhurta.find",
    "match.kuta",
    "rectify.birth_time",
    "rule.validate",
    "engine.consensus",
    "catalog.list",
];

pub async fn dispatch(ctx: &Ctx, name: &str, args: Value) -> Result<ToolOutput, ToolError> {
    match name {
        "chart.compute" => chart::run(ctx, args).await,
        "panchang.day" => panchang::run(ctx, args),
        "transit.window" => transit::run(ctx, args),
        "catalog.list" => catalog::run(ctx, args),
        "engine.consensus" => consensus::run(ctx, args).await,
        "rectify.birth_time" => rectify::run(ctx, args),
        "dasha.timeline" => proxy::dasha_timeline(ctx, args).await,
        "match.kuta" => proxy::match_kuta(ctx, args).await,
        "muhurta.find" => proxy::muhurta_find(ctx, args).await,
        "rule.validate" => proxy::rule_validate(ctx, args).await,
        other => Err(ToolError::new(
            "UNKNOWN_TOOL",
            format!("unknown tool {other:?}"),
        )),
    }
}

/// Helpers for reading typed fields from JSON arguments with clear errors.
pub mod args {
    use super::ToolError;
    use serde_json::Value;

    pub fn obj<'a>(
        v: &'a Value,
        what: &str,
    ) -> Result<&'a serde_json::Map<String, Value>, ToolError> {
        v.as_object()
            .ok_or_else(|| ToolError::invalid(format!("{what} must be a JSON object")))
    }

    pub fn f64_req(m: &serde_json::Map<String, Value>, key: &str) -> Result<f64, ToolError> {
        m.get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| ToolError::invalid(format!("missing or non-numeric field {key:?}")))
    }

    pub fn f64_opt(
        m: &serde_json::Map<String, Value>,
        key: &str,
        default: f64,
    ) -> Result<f64, ToolError> {
        match m.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v
                .as_f64()
                .ok_or_else(|| ToolError::invalid(format!("field {key:?} must be numeric"))),
        }
    }

    pub fn str_req<'a>(
        m: &'a serde_json::Map<String, Value>,
        key: &str,
    ) -> Result<&'a str, ToolError> {
        m.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid(format!("missing or non-string field {key:?}")))
    }

    pub fn u64_opt(
        m: &serde_json::Map<String, Value>,
        key: &str,
        default: u64,
    ) -> Result<u64, ToolError> {
        match m.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v.as_u64().ok_or_else(|| {
                ToolError::invalid(format!("field {key:?} must be a non-negative integer"))
            }),
        }
    }

    pub fn birth(
        m: &serde_json::Map<String, Value>,
        key: &str,
    ) -> Result<crate::types::BirthInput, ToolError> {
        let v = m
            .get(key)
            .ok_or_else(|| ToolError::invalid(format!("missing field {key:?} (BirthInput)")))?;
        let b: crate::types::BirthInput = serde_json::from_value(v.clone())
            .map_err(|e| ToolError::invalid(format!("{key}: {e}")))?;
        b.validate()?;
        Ok(b)
    }
}

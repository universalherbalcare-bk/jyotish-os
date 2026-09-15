//! MCP (Model Context Protocol) JSON-RPC 2.0 over HTTP at `POST /mcp`.
//! Protocol revision 2025-06-18: single messages (no batches), `initialize`,
//! `ping`, `tools/list`, `tools/call`; notifications get `202 Accepted`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, extract::State};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant as StdInstant;

use crate::AppState;
use crate::cache::request_key;
use crate::engine::AYANAMSA_NAME;
use crate::tools::{self, TOOL_NAMES, ToolOutput};

pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SERVER_NAME: &str = "jyotish-mcp";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

fn birth_schema() -> Value {
    json!({
        "type": "object",
        "description": "BirthInput (docs/CONTRACT.md). `utc` is authoritative; tz_offset_hours is for local rendering only.",
        "properties": {
            "utc": { "type": "string", "format": "date-time", "description": "RFC 3339 UTC instant, e.g. 1990-03-15T06:30:00Z" },
            "lat": { "type": "number", "minimum": -90, "maximum": 90 },
            "lon": { "type": "number", "minimum": -180, "maximum": 180 },
            "tz_offset_hours": { "type": "number", "minimum": -14, "maximum": 14 }
        },
        "required": ["utc", "lat", "lon", "tz_offset_hours"],
        "additionalProperties": false
    })
}

fn evidence_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "engine": { "type": "string" },
            "kernel": { "type": "string" },
            "kernel_sha256": { "type": "string" },
            "ayanamsa": { "type": "string", "const": "LAHIRI" },
            "consensus_status": { "type": "string", "enum": ["PASS", "FAIL", "SIDECAR_UNAVAILABLE", "NOT_CHECKED", "NOT_APPLICABLE"] },
            "delta_t_sigma_sec": { "type": ["number", "null"] }
        },
        "required": ["engine", "kernel", "kernel_sha256", "ayanamsa", "consensus_status", "delta_t_sigma_sec"]
    })
}

pub fn tool_definitions() -> Vec<Value> {
    let bodies = [
        "Sun", "Moon", "Mercury", "Venus", "Mars", "Jupiter", "Saturn", "Rahu", "Ketu", "Uranus",
        "Neptune", "Pluto",
    ];
    let vargas = crate::types::Varga::ALL
        .iter()
        .map(|v| v.code())
        .collect::<Vec<_>>();
    let out = json!({ "type": "object", "properties": { "evidence": evidence_schema() }, "required": ["evidence"] });
    vec![
        json!({
            "name": "chart.compute",
            "title": "Natal / varga chart (XALEN + JPL DE440, Lahiri, true nodes)",
            "description": "Sidereal longitudes, speeds, retrograde flags, rashi/nakshatra/pada with boundary_distance_sec (seconds of clock time until each flips), Ascendant, Whole-Sign and Sripati cusps, and the requested varga. Positions are refused (CONSENSUS_FAIL) if jhora-svc is reachable and disagrees beyond contract tolerances.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "birth": birth_schema(),
                    "varga": { "type": "string", "enum": vargas, "default": "D1" }
                },
                "required": ["birth"],
                "additionalProperties": false
            },
            "outputSchema": out
        }),
        json!({
            "name": "panchang.day",
            "title": "Daily panchang with transition instants",
            "description": "Tithi, nakshatra, yoga, karana for a local civil day (sunrise→next sunrise) with exact start/end instants, sunrise/sunset, moonrise/moonset, vara, rahu kala, abhijit.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "date": { "type": "string", "pattern": "^\\d{4}-\\d{2}-\\d{2}$", "description": "Local civil date YYYY-MM-DD" },
                    "lat": { "type": "number", "minimum": -90, "maximum": 90 },
                    "lon": { "type": "number", "minimum": -180, "maximum": 180 },
                    "tz_offset_hours": { "type": "number", "minimum": -14, "maximum": 14, "default": 0 }
                },
                "required": ["date", "lat", "lon"],
                "additionalProperties": false
            },
            "outputSchema": out
        }),
        json!({
            "name": "dasha.timeline",
            "title": "Dasha timeline (PyJHora via jhora-svc)",
            "description": "Nested dasha periods for any of the ~55 PyJHora systems. Proxied to jhora-svc POST /v1/dasha; returns SIDECAR_UNAVAILABLE if the sidecar is down.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "birth": birth_schema(),
                    "system": { "type": "string", "description": "System id from jhora-svc GET /v1/catalog/dasha (e.g. vimsottari)" },
                    "depth": { "type": "integer", "minimum": 1, "maximum": 5, "default": 2 }
                },
                "required": ["birth", "system"],
                "additionalProperties": false
            },
            "outputSchema": out
        }),
        json!({
            "name": "transit.window",
            "title": "Transit sweep: exact ingresses and conjunctions",
            "description": "Sweep bodies over [from, to] (≤10 years). Bisection gives exact sidereal rashi ingress instants and exact conjunctions (0° orb) with natal points.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "birth": birth_schema(),
                    "from": { "type": "string", "format": "date-time" },
                    "to": { "type": "string", "format": "date-time" },
                    "bodies": { "type": "array", "items": { "type": "string", "enum": bodies }, "default": ["Sun","Moon","Mercury","Venus","Mars","Jupiter","Saturn","Rahu","Ketu"] },
                    "natal_points": {
                        "type": "array",
                        "items": { "oneOf": [
                            { "type": "string", "description": "Body name or \"Ascendant\" (requires birth)" },
                            { "type": "object", "properties": { "name": { "type": "string" }, "sidereal_lon_deg": { "type": "number" } }, "required": ["sidereal_lon_deg"] }
                        ] }
                    },
                    "step_hours": { "type": "number", "minimum": 0.25, "maximum": 24, "default": 6 }
                },
                "required": ["from", "to"],
                "additionalProperties": false
            },
            "outputSchema": out
        }),
        json!({
            "name": "muhurta.find",
            "title": "Muhurta windows (VedAstro rules via vedastro-svc)",
            "description": "Ranked windows for an activity with the rule ids that passed and vetoed. Proxied to vedastro-svc POST /v1/muhurta/find.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "activity": { "type": "string", "description": "VedAstro EventDataList rule Name, e.g. GoodLunarDayForTravel" },
                    "from_utc": { "type": "string", "format": "date-time" },
                    "to_utc": { "type": "string", "format": "date-time" },
                    "lat": { "type": "number" }, "lon": { "type": "number" },
                    "tz_offset_hours": { "type": "number", "default": 0 },
                    "step_minutes": { "type": "integer", "minimum": 1, "maximum": 1440, "default": 60 }
                },
                "required": ["activity", "from_utc", "to_utc", "lat", "lon"],
                "additionalProperties": false
            },
            "outputSchema": out
        }),
        json!({
            "name": "match.kuta",
            "title": "Kuta compatibility (PyJHora via jhora-svc)",
            "description": "Per-kuta scores and total for two births. Proxied to jhora-svc POST /v1/kuta.",
            "inputSchema": {
                "type": "object",
                "properties": { "a": birth_schema(), "b": birth_schema() },
                "required": ["a", "b"],
                "additionalProperties": false
            },
            "outputSchema": out
        }),
        json!({
            "name": "rectify.birth_time",
            "title": "Birth-time rectification sweep (HEURISTIC)",
            "description": "Sweeps ±window_min at step_min steps; scores each candidate by how many supplied life events fall under Vimshottari maha/antar lords that are among the event's significators. Returns a ranked shortlist with an explicit heuristic disclaimer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "birth": birth_schema(),
                    "window_min": { "type": "integer", "minimum": 1, "maximum": 720, "default": 120 },
                    "step_min": { "type": "integer", "minimum": 1, "maximum": 60, "default": 1 },
                    "top_n": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10 },
                    "events": {
                        "type": "array", "minItems": 1, "maxItems": 50,
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": { "type": "string" },
                                "date": { "type": "string", "description": "YYYY-MM-DD (noon UT)" },
                                "utc": { "type": "string", "format": "date-time" },
                                "significators": { "type": "array", "minItems": 1, "items": { "type": "string", "enum": ["Sun","Moon","Mercury","Venus","Mars","Jupiter","Saturn","Rahu","Ketu"] } }
                            },
                            "required": ["significators"]
                        }
                    }
                },
                "required": ["birth", "events"],
                "additionalProperties": false
            },
            "outputSchema": out
        }),
        json!({
            "name": "rule.validate",
            "title": "Validate a VedAstro rule against real outcomes (vedastro-svc)",
            "description": "Hit rate vs base rate with 95% CI and PROMOTE / KEEP_UNPROVED verdict. Proxied to vedastro-svc POST /v1/rule/validate.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "rule_id": { "type": "string" },
                    "dataset": { "type": "string", "enum": ["marriage", "person"] },
                    "outcome_column": { "type": "string" }
                },
                "required": ["rule_id", "dataset", "outcome_column"],
                "additionalProperties": false
            },
            "outputSchema": out
        }),
        json!({
            "name": "engine.consensus",
            "title": "Cross-engine differential (XALEN-DE440 vs PyJHora-Swiss)",
            "description": "Per-body deltas in arcseconds against jhora-svc /v1/positions with contract tolerances; PASS/FAIL, or SIDECAR_UNAVAILABLE.",
            "inputSchema": {
                "type": "object",
                "properties": { "birth": birth_schema() },
                "required": ["birth"],
                "additionalProperties": false
            },
            "outputSchema": out
        }),
        json!({
            "name": "catalog.list",
            "title": "Enabled ayanamsas / house systems / vargas / dasha systems",
            "description": "Exactly what this server serves. Gauquelin, Qi Men Dun Jia and PullenSinusoidalRatio are deliberately absent.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "outputSchema": out
        }),
    ]
}

fn rpc_error(id: Value, code: i64, message: impl Into<String>, data: Option<Value>) -> Value {
    let mut err = json!({ "code": code, "message": message.into() });
    if let Some(d) = data {
        err["data"] = d;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": err })
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn evidence(state: &AppState, out: &ToolOutput) -> Value {
    json!({
        "engine": out.engine,
        "kernel": "de440s.bsp",
        "kernel_id": state.engine.kernel_id,
        "kernel_sha256": state.engine.kernel_sha256,
        "ayanamsa": AYANAMSA_NAME,
        "nodes": "TRUE",
        "consensus_status": out.consensus_status,
        "delta_t_sigma_sec": out.delta_t_sigma_sec,
        "delta_t_sigma_note": if out.delta_t_sigma_sec.is_some() { "1σ of the SMH2016 ΔT model at the request epoch (xalen-time delta_t_with_uncertainty)" } else { "not applicable: no epoch in this result" },
        "cache_hit": out.cache_hit,
    })
}

fn tool_call_result(structured: Value, is_error: bool) -> Value {
    let text = serde_json::to_string(&structured).unwrap_or_else(|_| "{}".to_string());
    json!({
        "content": [ { "type": "text", "text": text } ],
        "structuredContent": structured,
        "isError": is_error,
    })
}

async fn handle_tools_call(state: &AppState, params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).ok_or((
        INVALID_PARAMS,
        "tools/call requires params.name".to_string(),
    ))?;
    if !TOOL_NAMES.contains(&name) {
        return Err((INVALID_PARAMS, format!("unknown tool {name:?}")));
    }
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !args.is_object() {
        return Err((
            INVALID_PARAMS,
            "params.arguments must be an object".to_string(),
        ));
    }
    let request_hash = request_key(name, &args, &state.engine.kernel_sha256);
    let started = StdInstant::now();
    let ctx = tools::Ctx {
        engine: state.engine.clone(),
        sidecars: state.sidecars.clone(),
        cache: state.cache.clone(),
    };
    let outcome = tools::dispatch(&ctx, name, args).await;
    let ms = started.elapsed().as_millis();
    match outcome {
        Ok(out) => {
            state
                .audit
                .record(name, &request_hash, ms, "ok", out.cache_hit);
            let mut structured = out.payload.clone();
            if let Value::Object(ref mut m) = structured {
                m.insert("evidence".into(), evidence(state, &out));
            } else {
                structured = json!({ "result": out.payload, "evidence": evidence(state, &out) });
            }
            Ok(tool_call_result(structured, false))
        }
        Err(e) => {
            state.audit.record(name, &request_hash, ms, e.code, false);
            let mut structured = e.to_value();
            structured["evidence"] = json!({
                "engine": crate::engine::ENGINE_NAME,
                "kernel": "de440s.bsp",
                "kernel_sha256": state.engine.kernel_sha256,
                "ayanamsa": AYANAMSA_NAME,
                "consensus_status": if e.code == "CONSENSUS_FAIL" { "FAIL" } else if e.code == "SIDECAR_UNAVAILABLE" { "SIDECAR_UNAVAILABLE" } else { "NOT_APPLICABLE" },
                "delta_t_sigma_sec": Value::Null,
            });
            Ok(tool_call_result(structured, true))
        }
    }
}

/// Handle one JSON-RPC message. Returns `None` for notifications.
pub async fn handle_message(state: &AppState, msg: Value) -> Option<Value> {
    let Some(obj) = msg.as_object() else {
        return Some(rpc_error(
            Value::Null,
            INVALID_REQUEST,
            "request must be a JSON object (batches are not supported in MCP 2025-06-18)",
            None,
        ));
    };
    let id = obj.get("id").cloned();
    let is_notification = id.is_none();
    let id = id.unwrap_or(Value::Null);
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(
            id,
            INVALID_REQUEST,
            "jsonrpc must be \"2.0\"",
            None,
        ));
    }
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Some(rpc_error(
            id,
            INVALID_REQUEST,
            "method must be a string",
            None,
        ));
    };
    let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));

    if is_notification {
        // notifications/initialized, notifications/cancelled, etc. — accepted, no body.
        return None;
    }

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION, "title": "JYOTISH-OS consensus engine (XALEN + JPL DE440)" },
            "instructions": "All longitudes are sidereal (Lahiri) degrees with TRUE nodes from JPL DE440. Every result carries an `evidence` block; treat consensus_status and boundary_distance_sec as first-class facts and never state a rashi/nakshatra/pada as certain when it flips within 120 s."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => handle_tools_call(state, &params).await,
        other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
    };
    Some(match result {
        Ok(r) => rpc_result(id, r),
        Err((code, message)) => rpc_error(id, code, message, None),
    })
}

pub async fn post_mcp(State(state): State<Arc<AppState>>, body: axum::body::Bytes) -> Response {
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(rpc_error(
                    Value::Null,
                    PARSE_ERROR,
                    format!("parse error: {e}"),
                    None,
                )),
            )
                .into_response();
        }
    };
    match handle_message(&state, msg).await {
        Some(resp) => (StatusCode::OK, Json(resp)).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// `GET /mcp` — this server does not implement the SSE stream; say so.
pub async fn get_mcp() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        "jyotish-mcp serves MCP over POST /mcp only (no SSE stream)",
    )
        .into_response()
}

pub async fn health(State(state): State<Arc<AppState>>) -> Response {
    let jhora_up = state.sidecars.jhora_is_up().await;
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "service": SERVER_NAME,
            "version": SERVER_VERSION,
            "protocolVersion": PROTOCOL_VERSION,
            "engine": crate::engine::ENGINE_NAME,
            "kernel_id": state.engine.kernel_id,
            "kernel_sha256": state.engine.kernel_sha256,
            "kernel_coverage_jd": state.engine.coverage_jd,
            "ayanamsa": AYANAMSA_NAME,
            "nodes": "TRUE",
            "golden": state.golden,
            "sidecars": { "jhora": { "url": state.sidecars.base_url(crate::sidecar::Sidecar::Jhora), "up": jhora_up }, "vedastro": { "url": state.sidecars.base_url(crate::sidecar::Sidecar::Vedastro) } },
            "cache_entries": state.cache.len(),
            "tools": TOOL_NAMES,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolError;

    #[test]
    fn tool_definitions_cover_contract_tool_list() {
        let defs = tool_definitions();
        let names: Vec<&str> = defs.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, TOOL_NAMES.to_vec());
        for d in &defs {
            assert_eq!(d["inputSchema"]["type"], "object");
            assert!(d["description"].as_str().unwrap().len() > 20);
        }
    }

    #[test]
    fn json_rpc_error_shape() {
        let e = rpc_error(json!(1), METHOD_NOT_FOUND, "nope", None);
        assert_eq!(e["jsonrpc"], "2.0");
        assert_eq!(e["error"]["code"], -32601);
        assert!(e.get("result").is_none());
    }

    #[test]
    fn tool_error_wrapping_sets_is_error() {
        let e = ToolError::new("X", "y");
        let r = tool_call_result(e.to_value(), true);
        assert_eq!(r["isError"], true);
        assert_eq!(r["content"][0]["type"], "text");
        assert_eq!(r["structuredContent"]["error"]["code"], "X");
    }
}

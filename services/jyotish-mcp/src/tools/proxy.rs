//! Tools proxied to the sidecars. Arguments are schema-checked locally, then
//! forwarded verbatim; a down sidecar yields `SIDECAR_UNAVAILABLE` — never
//! fabricated data. The `evidence.engine` names the sidecar that produced
//! the numbers, not XALEN.

use serde_json::{Value, json};

use super::{Ctx, ToolOutput, args};
use crate::sidecar::Sidecar;
use crate::types::ToolError;

const MAX_DASHA_DEPTH: u64 = 5;

fn out(payload: Value, engine: &'static str) -> ToolOutput {
    ToolOutput {
        payload,
        engine,
        consensus_status: "NOT_APPLICABLE".into(),
        delta_t_sigma_sec: None,
        cache_hit: false,
    }
}

/// `dasha.timeline` → jhora-svc `POST /v1/dasha`.
pub async fn dasha_timeline(ctx: &Ctx, args: Value) -> Result<ToolOutput, ToolError> {
    let m = args::obj(&args, "arguments")?;
    let birth = args::birth(m, "birth")?;
    let system = args::str_req(m, "system")?;
    // Must match jhora-svc's own id grammar `^[a-z]+\.[a-z_0-9]+$`
    // (e.g. `graha.vimsottari`, `raasi.narayana`, `annual.mudda`); anything
    // else is rejected here so the sidecar never sees an unparseable id.
    if !valid_dasha_system_id(system) {
        return Err(ToolError::invalid(
            "system must be `<family>.<module>` in lowercase, e.g. graha.vimsottari (see catalog.list / jhora-svc /v1/catalog/dasha)",
        ));
    }
    let depth = args::u64_opt(m, "depth", 2)?;
    if !(1..=MAX_DASHA_DEPTH).contains(&depth) {
        return Err(ToolError::invalid(format!(
            "depth must be within [1, {MAX_DASHA_DEPTH}]"
        )));
    }
    let body = json!({ "birth": birth, "system": system, "depth": depth });
    let resp = ctx
        .sidecars
        .post_json(Sidecar::Jhora, "/v1/dasha", &body)
        .await?;
    Ok(out(resp, "jhora-svc"))
}

/// `match.kuta` → jhora-svc `POST /v1/kuta`.
pub async fn match_kuta(ctx: &Ctx, args: Value) -> Result<ToolOutput, ToolError> {
    let m = args::obj(&args, "arguments")?;
    let a = args::birth(m, "a")?;
    let b = args::birth(m, "b")?;
    let body = json!({ "a": a, "b": b });
    let resp = ctx
        .sidecars
        .post_json(Sidecar::Jhora, "/v1/kuta", &body)
        .await?;
    Ok(out(resp, "jhora-svc"))
}

/// `muhurta.find` → vedastro-svc `POST /v1/muhurta/find`.
pub async fn muhurta_find(ctx: &Ctx, args: Value) -> Result<ToolOutput, ToolError> {
    let m = args::obj(&args, "arguments")?;
    let activity = args::str_req(m, "activity")?;
    if activity.is_empty() || activity.len() > 128 {
        return Err(ToolError::invalid(
            "activity must be a rule Name (1–128 chars)",
        ));
    }
    let from_utc = args::str_req(m, "from_utc")?;
    let to_utc = args::str_req(m, "to_utc")?;
    crate::types::parse_utc(from_utc)?;
    crate::types::parse_utc(to_utc)?;
    let lat = args::f64_req(m, "lat")?;
    let lon = args::f64_req(m, "lon")?;
    crate::types::validate_lat_lon(lat, lon)?;
    let tz = args::f64_opt(m, "tz_offset_hours", 0.0)?;
    crate::types::validate_tz(tz)?;
    let step = args::u64_opt(m, "step_minutes", 60)?;
    if !(1..=1440).contains(&step) {
        return Err(ToolError::invalid("step_minutes must be within [1, 1440]"));
    }
    let body = json!({
        "activity": activity, "from_utc": from_utc, "to_utc": to_utc,
        "lat": lat, "lon": lon, "tz_offset_hours": tz, "step_minutes": step,
    });
    let resp = ctx
        .sidecars
        .post_json(Sidecar::Vedastro, "/v1/muhurta/find", &body)
        .await?;
    Ok(out(resp, "vedastro-svc"))
}

/// `rule.validate` → vedastro-svc `POST /v1/rule/validate`.
pub async fn rule_validate(ctx: &Ctx, args: Value) -> Result<ToolOutput, ToolError> {
    let m = args::obj(&args, "arguments")?;
    let rule_id = args::str_req(m, "rule_id")?;
    if rule_id.is_empty() || rule_id.len() > 128 {
        return Err(ToolError::invalid("rule_id must be 1–128 chars"));
    }
    let dataset = args::str_req(m, "dataset")?;
    if dataset != "marriage" && dataset != "person" {
        return Err(ToolError::invalid(
            "dataset must be \"marriage\" or \"person\"",
        ));
    }
    let outcome = args::str_req(m, "outcome_column")?;
    if outcome.is_empty() || outcome.len() > 128 {
        return Err(ToolError::invalid("outcome_column must be 1–128 chars"));
    }
    let mut body = json!({ "rule_id": rule_id, "dataset": dataset, "outcome_column": outcome });
    // Optional sampling controls forwarded verbatim to vedastro-svc (its
    // defaults apply when absent). Bounded here so a caller cannot ask the
    // sidecar for an unbounded evaluation.
    if m.contains_key("max_rows") {
        let max_rows = args::u64_opt(m, "max_rows", 300)?;
        if !(1..=20_000).contains(&max_rows) {
            return Err(ToolError::invalid("max_rows must be within [1, 20000]"));
        }
        body["max_rows"] = json!(max_rows);
    }
    if m.contains_key("offset") {
        let offset = args::u64_opt(m, "offset", 0)?;
        if offset > 1_000_000 {
            return Err(ToolError::invalid("offset must be within [0, 1000000]"));
        }
        body["offset"] = json!(offset);
    }
    let resp = ctx
        .sidecars
        .post_json(Sidecar::Vedastro, "/v1/rule/validate", &body)
        .await?;
    Ok(out(resp, "vedastro-svc"))
}

/// jhora-svc accepts dasha ids matching `^[a-z]+\.[a-z_0-9]+$`.
pub fn valid_dasha_system_id(id: &str) -> bool {
    if id.len() > 64 {
        return false;
    }
    let Some((family, module)) = id.split_once('.') else {
        return false;
    };
    !family.is_empty()
        && family.bytes().all(|b| b.is_ascii_lowercase())
        && !module.is_empty()
        && module
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[cfg(test)]
mod id_tests {
    use super::valid_dasha_system_id;

    #[test]
    fn dasha_id_grammar_matches_sidecar() {
        for ok in [
            "graha.vimsottari",
            "raasi.narayana",
            "annual.mudda",
            "graha.yoga_vimsottari",
        ] {
            assert!(valid_dasha_system_id(ok), "{ok}");
        }
        for bad in [
            "vimsottari",
            "graha_vimsottari",
            "Graha.Vimsottari",
            "graha.",
            ".x",
            "a.b.c",
            "graha.vim-sottari",
            "",
        ] {
            assert!(!valid_dasha_system_id(bad), "{bad}");
        }
    }
}

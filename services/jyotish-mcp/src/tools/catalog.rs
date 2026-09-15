//! `catalog.list` — exactly which ayanamsas / house systems / vargas / dasha
//! systems this server will serve. Gauquelin, Qi Men Dun Jia and Pullen
//! Sinusoidal Ratio are deliberately absent (blueprint §4).

use serde_json::{Value, json};
use xalen_houses::HouseSystem;

use super::{Ctx, ToolOutput};
use crate::engine::{AYANAMSA_NAME, ENGINE_NAME};
use crate::types::{ToolError, Varga};

/// House systems exposed by this server. Everything in `xalen_houses::HouseSystem`
/// except the placeholders below.
pub const ENABLED_HOUSE_SYSTEMS: [HouseSystem; 21] = [
    HouseSystem::WholeSign,
    HouseSystem::Sripati,
    HouseSystem::Equal,
    HouseSystem::Placidus,
    HouseSystem::Koch,
    HouseSystem::Porphyry,
    HouseSystem::Regiomontanus,
    HouseSystem::Campanus,
    HouseSystem::Morinus,
    HouseSystem::Alcabitius,
    HouseSystem::Topocentric,
    HouseSystem::Meridian,
    HouseSystem::Vehlow,
    HouseSystem::KrusinskiPisa,
    HouseSystem::SunshineMakransky,
    HouseSystem::SunshineTreindl,
    HouseSystem::PullenSinusoidalDelta,
    HouseSystem::CarterPoliEquatorial,
    HouseSystem::APC,
    HouseSystem::Zariel,
    HouseSystem::AlcabitiusClassic,
];

/// Systems present in the vendored engines but refused here, with the reason.
pub const EXCLUDED: [(&str, &str); 3] = [
    (
        "Gauquelin",
        "xalen-houses placeholder: returns Placidus cusps, not a true 36-sector division",
    ),
    (
        "QiMenDunJia",
        "xalen-chinese experimental module; not linked by this server",
    ),
    (
        "PullenSinusoidalRatio",
        "~5° approximation per xalen docs/COMPARISON.md; untested",
    ),
];

pub fn house_system_name(h: HouseSystem) -> String {
    format!("{h:?}")
}

pub fn parse_house_system(s: &str) -> Option<HouseSystem> {
    ENABLED_HOUSE_SYSTEMS
        .iter()
        .copied()
        .find(|h| house_system_name(*h).eq_ignore_ascii_case(s))
}

pub fn catalog_value() -> Value {
    json!({
        "canonical": {
            "ayanamsa": AYANAMSA_NAME,
            "nodes": "TRUE",
            "ephemeris": "JPL DE440 (de440s.bsp)",
            "rasi_house_system": "WholeSign",
            "bhava_house_system": "Sripati",
            "zodiac_output": "sidereal degrees [0,360)"
        },
        "ayanamsas": {
            "enabled": [AYANAMSA_NAME],
            "note": "Only the canonical Lahiri ayanamsa is served; every engine in the topology is pinned to it (docs/CONTRACT.md)."
        },
        "house_systems": {
            "enabled": ENABLED_HOUSE_SYSTEMS.iter().map(|h| house_system_name(*h)).collect::<Vec<_>>(),
            "served_by_chart_compute": ["WholeSign", "Sripati"]
        },
        "vargas": Varga::ALL.iter().map(|v| json!({"code": v.code(), "name": v.name(), "divisions": v.to_xalen().divisions()})).collect::<Vec<_>>(),
        "dasha_systems": {
            "native": ["Vimshottari"],
            "via_sidecar": "jhora-svc GET /v1/catalog/dasha (dasha.timeline)"
        },
        "bodies": ["Sun","Moon","Mercury","Venus","Mars","Jupiter","Saturn","Rahu","Ketu","Uranus","Neptune","Pluto"],
        "excluded": EXCLUDED.iter().map(|(n, why)| json!({"name": n, "reason": why})).collect::<Vec<_>>(),
        "tools": super::TOOL_NAMES,
    })
}

pub fn run(_ctx: &Ctx, _args: Value) -> Result<ToolOutput, ToolError> {
    Ok(ToolOutput {
        payload: catalog_value(),
        engine: ENGINE_NAME,
        consensus_status: "NOT_APPLICABLE".into(),
        delta_t_sigma_sec: None,
        cache_hit: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_excludes_placeholder_systems() {
        let v = catalog_value();
        let text = serde_json::to_string(&v["house_systems"]).unwrap();
        assert!(!text.contains("Gauquelin"));
        assert!(!text.contains("PullenSinusoidalRatio"));
        assert!(!text.contains("QiMen"));
        assert!(text.contains("PullenSinusoidalDelta"));
        assert!(text.contains("WholeSign") && text.contains("Sripati"));
        let excluded: Vec<&str> = v["excluded"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            excluded,
            vec!["Gauquelin", "QiMenDunJia", "PullenSinusoidalRatio"]
        );
        assert_eq!(v["vargas"].as_array().unwrap().len(), 16);
        assert_eq!(v["ayanamsas"]["enabled"], serde_json::json!(["LAHIRI"]));
        assert!(parse_house_system("Gauquelin").is_none());
        assert_eq!(
            parse_house_system("wholesign"),
            Some(HouseSystem::WholeSign)
        );
    }
}

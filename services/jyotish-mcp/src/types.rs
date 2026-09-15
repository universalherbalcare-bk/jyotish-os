//! Shared wire types, validation, and time helpers. Every untrusted boundary
//! (tool arguments) is validated here before any astronomy runs.

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use xalen_time::{CalendarSystem, jd_to_calendar};

/// Structured tool error. Returned to MCP clients as `isError: true` with the
/// JSON form in `structuredContent.error`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("INVALID_PARAMS", message)
    }
    pub fn to_value(&self) -> Value {
        json!({ "error": self })
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// Shared input type from docs/CONTRACT.md. `utc` is authoritative;
/// `tz_offset_hours` is used only for local rendering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BirthInput {
    pub utc: String,
    pub lat: f64,
    pub lon: f64,
    pub tz_offset_hours: f64,
}

impl BirthInput {
    pub fn validate(&self) -> Result<(), ToolError> {
        parse_utc(&self.utc)?;
        validate_lat_lon(self.lat, self.lon)?;
        validate_tz(self.tz_offset_hours)?;
        Ok(())
    }
}

pub fn validate_lat_lon(lat: f64, lon: f64) -> Result<(), ToolError> {
    if !lat.is_finite() || !(-90.0..=90.0).contains(&lat) {
        return Err(ToolError::invalid(format!(
            "lat must be within [-90, 90], got {lat}"
        )));
    }
    if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
        return Err(ToolError::invalid(format!(
            "lon must be within [-180, 180], got {lon}"
        )));
    }
    Ok(())
}

pub fn validate_tz(tz: f64) -> Result<(), ToolError> {
    if !tz.is_finite() || !(-14.0..=14.0).contains(&tz) {
        return Err(ToolError::invalid(format!(
            "tz_offset_hours must be within [-14, 14], got {tz}"
        )));
    }
    Ok(())
}

/// Calendar fields of a UTC instant, ready for `xalen_time::Epoch::from_utc`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UtcFields {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: f64,
}

/// Parse an RFC 3339 / ISO-8601 timestamp (`Z` or numeric offset) into UTC
/// calendar fields. Any offset is folded into UTC; the tz offset supplied
/// elsewhere in the request is never applied here.
pub fn parse_utc(s: &str) -> Result<UtcFields, ToolError> {
    let dt = DateTime::parse_from_rfc3339(s.trim())
        .map_err(|e| ToolError::invalid(format!("utc {s:?} is not RFC 3339: {e}")))?;
    let dt = dt.with_timezone(&Utc);
    Ok(UtcFields {
        year: dt.year(),
        month: dt.month(),
        day: dt.day(),
        hour: dt.hour(),
        minute: dt.minute(),
        second: f64::from(dt.second()) + f64::from(dt.nanosecond()) / 1e9,
    })
}

/// Parse a `YYYY-MM-DD` civil date.
pub fn parse_date(s: &str) -> Result<(i32, u32, u32), ToolError> {
    let d = NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|e| ToolError::invalid(format!("date {s:?} is not YYYY-MM-DD: {e}")))?;
    Ok((d.year(), d.month(), d.day()))
}

/// Format a UT Julian Day as an RFC 3339 UTC string with millisecond precision.
pub fn jd_to_iso_utc(jd: f64) -> String {
    match jd_to_naive(jd) {
        Some(ndt) => format!("{}Z", ndt.format("%Y-%m-%dT%H:%M:%S%.3f")),
        None => "invalid".to_string(),
    }
}

/// Format a UT Julian Day as a local RFC 3339 string for the given offset.
pub fn jd_to_iso_local(jd: f64, tz_offset_hours: f64) -> String {
    let secs = (tz_offset_hours * 3600.0).round() as i32;
    let Some(off) = FixedOffset::east_opt(secs) else {
        return jd_to_iso_utc(jd);
    };
    match jd_to_naive(jd) {
        Some(ndt) => {
            let utc = ndt.and_utc();
            utc.with_timezone(&off)
                .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
                .to_string()
        }
        None => "invalid".to_string(),
    }
}

fn jd_to_naive(jd: f64) -> Option<chrono::NaiveDateTime> {
    if !jd.is_finite() {
        return None;
    }
    let (y, m, d, hour) = jd_to_calendar(jd, CalendarSystem::ProlepticGregorian);
    let ms = (hour * 3_600_000.0).round() as i64;
    let date = NaiveDate::from_ymd_opt(y, m, d)?;
    let base = date.and_hms_opt(0, 0, 0)?;
    base.checked_add_signed(Duration::milliseconds(ms))
}

/// Divisional chart enum from the blueprint (§3), mirrored 1:1 onto
/// `xalen_vedic::divisional::VargaChart`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[allow(clippy::upper_case_acronyms)]
pub enum Varga {
    D1,
    D2,
    D3,
    D4,
    D7,
    D9,
    D10,
    D12,
    D16,
    D20,
    D24,
    D27,
    D30,
    D40,
    D45,
    D60,
}

impl Varga {
    pub const ALL: [Varga; 16] = [
        Varga::D1,
        Varga::D2,
        Varga::D3,
        Varga::D4,
        Varga::D7,
        Varga::D9,
        Varga::D10,
        Varga::D12,
        Varga::D16,
        Varga::D20,
        Varga::D24,
        Varga::D27,
        Varga::D30,
        Varga::D40,
        Varga::D45,
        Varga::D60,
    ];

    pub fn to_xalen(self) -> xalen_vedic::divisional::VargaChart {
        use xalen_vedic::divisional::VargaChart as V;
        match self {
            Varga::D1 => V::D1,
            Varga::D2 => V::D2,
            Varga::D3 => V::D3,
            Varga::D4 => V::D4,
            Varga::D7 => V::D7,
            Varga::D9 => V::D9,
            Varga::D10 => V::D10,
            Varga::D12 => V::D12,
            Varga::D16 => V::D16,
            Varga::D20 => V::D20,
            Varga::D24 => V::D24,
            Varga::D27 => V::D27,
            Varga::D30 => V::D30,
            Varga::D40 => V::D40,
            Varga::D45 => V::D45,
            Varga::D60 => V::D60,
        }
    }

    pub fn name(self) -> &'static str {
        self.to_xalen().name()
    }

    pub fn code(self) -> &'static str {
        match self {
            Varga::D1 => "D1",
            Varga::D2 => "D2",
            Varga::D3 => "D3",
            Varga::D4 => "D4",
            Varga::D7 => "D7",
            Varga::D9 => "D9",
            Varga::D10 => "D10",
            Varga::D12 => "D12",
            Varga::D16 => "D16",
            Varga::D20 => "D20",
            Varga::D24 => "D24",
            Varga::D27 => "D27",
            Varga::D30 => "D30",
            Varga::D40 => "D40",
            Varga::D45 => "D45",
            Varga::D60 => "D60",
        }
    }
}

/// Signed angular difference `a − b` mapped to `[−180, 180)` degrees.
pub fn signed_delta_deg(a: f64, b: f64) -> f64 {
    ((a - b + 180.0).rem_euclid(360.0)) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_z_and_offsets_to_utc() {
        let a = parse_utc("1990-03-15T06:30:00Z").unwrap();
        let b = parse_utc("1990-03-15T12:00:00+05:30").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.hour, 6);
        assert_eq!(a.minute, 30);
        assert!(parse_utc("1990-03-15 06:30").is_err());
        assert!(parse_utc("").is_err());
    }

    #[test]
    fn jd_iso_roundtrip() {
        // 2000-01-01T12:00:00Z is JD 2451545.0
        assert_eq!(jd_to_iso_utc(2451545.0), "2000-01-01T12:00:00.000Z");
        assert_eq!(
            jd_to_iso_local(2451545.0, 5.5),
            "2000-01-01T17:30:00.000+05:30"
        );
        // rounding at a day boundary must roll over, not print 24:00
        let jd = 2451545.5 - 0.0001 / 86400.0;
        assert_eq!(jd_to_iso_utc(jd), "2000-01-02T00:00:00.000Z");
    }

    #[test]
    fn lat_lon_tz_validation() {
        assert!(validate_lat_lon(91.0, 0.0).is_err());
        assert!(validate_lat_lon(0.0, 181.0).is_err());
        assert!(validate_lat_lon(f64::NAN, 0.0).is_err());
        assert!(validate_lat_lon(28.6139, 77.2090).is_ok());
        assert!(validate_tz(15.0).is_err());
        assert!(validate_tz(5.5).is_ok());
    }

    #[test]
    fn signed_delta_wraps() {
        assert_eq!(signed_delta_deg(10.0, 350.0), 20.0);
        assert_eq!(signed_delta_deg(350.0, 10.0), -20.0);
        assert_eq!(signed_delta_deg(180.0, 0.0), -180.0);
        assert!((signed_delta_deg(179.999, 0.0) - 179.999).abs() < 1e-9);
    }

    #[test]
    fn varga_enum_matches_blueprint() {
        assert_eq!(Varga::ALL.len(), 16);
        assert_eq!(Varga::D9.name(), "Navamsa");
        assert_eq!(serde_json::to_string(&Varga::D60).unwrap(), "\"D60\"");
    }
}

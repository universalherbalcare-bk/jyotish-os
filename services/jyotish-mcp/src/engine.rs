//! L0 positions: XALEN crates linked natively, JPL DE440 kernel mandatory.
//!
//! Guarantees enforced here (not in prose):
//! * The kernel's SHA-256 must equal the pinned value in `kernels/de440s.sha256`.
//! * The parsed SPK must confirm `DE440` provenance (`is_de440_loaded`).
//! * Every instant is checked against the kernel's own coverage window; XALEN's
//!   silent VSOP87 fallback outside coverage is therefore unreachable.
//! * Ayanamsa is `Ayanamsa::Lahiri`, nodes are `Body::TrueNode` (docs/CONTRACT.md).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use xalen_ayanamsa::Ayanamsa;
use xalen_ephem::{Almanac, Body, De440Provider, De440Reader, EphemerisError, EphemerisProvider};
use xalen_houses::{GeoLocation, HouseSystem, compute_houses_from_ramc};
use xalen_time::{DeltaTModel, Epoch, JdTT, JdUT1, JulianDay, delta_t_with_uncertainty};
use xalen_vedic::dasha::{DashaLevel, DashaPeriod, vimshottari_dasha};
use xalen_vedic::nakshatra::{DashaLord, Nakshatra};
use xalen_vedic::panchang::Vara;
use xalen_vedic::rashi::Rashi;

use crate::cache::hex;
use crate::types::{ToolError, UtcFields, signed_delta_deg};

pub const ENGINE_NAME: &str = "xalen-de440";
pub const AYANAMSA_NAME: &str = "LAHIRI";
pub const NAKSHATRA_SPAN_DEG: f64 = 360.0 / 27.0;
pub const PADA_SPAN_DEG: f64 = NAKSHATRA_SPAN_DEG / 4.0;
pub const RASHI_SPAN_DEG: f64 = 30.0;
/// Margin (days) kept away from the kernel edges so ±0.5-day speed samples
/// and forward panchang scans never leave coverage.
const COVERAGE_MARGIN_DAYS: f64 = 5.0;

#[derive(Debug)]
pub enum EngineError {
    Kernel(String),
    Ephemeris(String),
    Coverage { jd_tt: f64, start: f64, end: f64 },
    Golden(String),
    Input(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Kernel(s) => write!(f, "kernel: {s}"),
            EngineError::Ephemeris(s) => write!(f, "ephemeris: {s}"),
            EngineError::Coverage { jd_tt, start, end } => write!(
                f,
                "JD(TT) {jd_tt:.3} is outside the loaded DE440 kernel coverage [{start:.1}, {end:.1}] (with {COVERAGE_MARGIN_DAYS}-day margin); refusing analytic fallback"
            ),
            EngineError::Golden(s) => write!(f, "golden-chart self-test failed: {s}"),
            EngineError::Input(s) => write!(f, "input: {s}"),
        }
    }
}
impl std::error::Error for EngineError {}

impl From<EphemerisError> for EngineError {
    fn from(e: EphemerisError) -> Self {
        EngineError::Ephemeris(e.to_string())
    }
}

impl From<EngineError> for ToolError {
    fn from(e: EngineError) -> Self {
        match &e {
            EngineError::Coverage { .. } => ToolError::new("KERNEL_COVERAGE", e.to_string()),
            EngineError::Input(_) => ToolError::invalid(e.to_string()),
            EngineError::Kernel(_) => ToolError::new("KERNEL_ERROR", e.to_string()),
            EngineError::Ephemeris(_) => ToolError::new("EPHEMERIS_ERROR", e.to_string()),
            EngineError::Golden(_) => ToolError::new("GOLDEN_CHECK_FAILED", e.to_string()),
        }
    }
}

/// The nine Vedic grahas plus the outer planets served by chart tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BodyId {
    Sun,
    Moon,
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Rahu,
    Ketu,
    Uranus,
    Neptune,
    Pluto,
}

impl BodyId {
    /// Contract bodies (`/v1/positions` shape) — consensus-checked.
    pub const GRAHAS: [BodyId; 9] = [
        BodyId::Sun,
        BodyId::Moon,
        BodyId::Mercury,
        BodyId::Venus,
        BodyId::Mars,
        BodyId::Jupiter,
        BodyId::Saturn,
        BodyId::Rahu,
        BodyId::Ketu,
    ];
    pub const OUTER: [BodyId; 3] = [BodyId::Uranus, BodyId::Neptune, BodyId::Pluto];

    pub fn name(self) -> &'static str {
        match self {
            BodyId::Sun => "Sun",
            BodyId::Moon => "Moon",
            BodyId::Mercury => "Mercury",
            BodyId::Venus => "Venus",
            BodyId::Mars => "Mars",
            BodyId::Jupiter => "Jupiter",
            BodyId::Saturn => "Saturn",
            BodyId::Rahu => "Rahu",
            BodyId::Ketu => "Ketu",
            BodyId::Uranus => "Uranus",
            BodyId::Neptune => "Neptune",
            BodyId::Pluto => "Pluto",
        }
    }

    pub fn parse(s: &str) -> Option<BodyId> {
        let all = [
            BodyId::Sun,
            BodyId::Moon,
            BodyId::Mercury,
            BodyId::Venus,
            BodyId::Mars,
            BodyId::Jupiter,
            BodyId::Saturn,
            BodyId::Rahu,
            BodyId::Ketu,
            BodyId::Uranus,
            BodyId::Neptune,
            BodyId::Pluto,
        ];
        all.into_iter().find(|b| b.name().eq_ignore_ascii_case(s))
    }

    fn xalen_body(self) -> Body {
        match self {
            BodyId::Sun => Body::Sun,
            BodyId::Moon => Body::Moon,
            BodyId::Mercury => Body::Mercury,
            BodyId::Venus => Body::Venus,
            BodyId::Mars => Body::Mars,
            BodyId::Jupiter => Body::Jupiter,
            BodyId::Saturn => Body::Saturn,
            BodyId::Rahu | BodyId::Ketu => Body::TrueNode,
            BodyId::Uranus => Body::Uranus,
            BodyId::Neptune => Body::Neptune,
            BodyId::Pluto => Body::Pluto,
        }
    }

    /// Where the number physically comes from. DE440 covers the physical
    /// bodies; the true node is XALEN's osculating-node computation (analytic,
    /// finite-difference on the analytic Moon), which is why the contract gates
    /// it separately at 60″ (`source: analytic`).
    pub fn source(self) -> &'static str {
        match self {
            BodyId::Rahu | BodyId::Ketu => "xalen-true-node (osculating, analytic)",
            _ => "jpl-de440",
        }
    }

    pub fn to_dasha_lord(self) -> Option<DashaLord> {
        match self {
            BodyId::Sun => Some(DashaLord::Sun),
            BodyId::Moon => Some(DashaLord::Moon),
            BodyId::Mercury => Some(DashaLord::Mercury),
            BodyId::Venus => Some(DashaLord::Venus),
            BodyId::Mars => Some(DashaLord::Mars),
            BodyId::Jupiter => Some(DashaLord::Jupiter),
            BodyId::Saturn => Some(DashaLord::Saturn),
            BodyId::Rahu => Some(DashaLord::Rahu),
            BodyId::Ketu => Some(DashaLord::Ketu),
            _ => None,
        }
    }
}

/// JD(UTC) of 1972-01-01T00:00:00Z — the first instant of leap-second UTC.
/// Before it, "UTC" was a rubber-second scale that xalen-time explicitly does
/// not model (`leap_seconds.rs`: a fixed TAI−UTC = 10 s floor is returned), so
/// a civil timestamp is treated as UT1 and TT comes from the ΔT model — the
/// same rule Swiss `swe_utc_to_jd` applies.
pub const UTC_LEAP_SECOND_ERA_START_JD: f64 = 2441317.5;

/// A fully resolved instant: UT1 (≈UTC, DUT1 not applied, |err| < 0.9 s),
/// TT, and the 1σ ΔT-model envelope at that epoch.
///
/// Time-scale policy (measured against Swiss in validation/consensus-summary.md):
/// * `UTC < 1972-01-01`: UT1 = civil time, TT = UT1 + ΔT(SMH2016). The
///   leap-second table does not apply; using it would place TT 13–44 s late
///   for 1900–1971 (Moon 7–24″).
/// * `UTC ≥ 1972-01-01`: TT is leap-second exact (`Epoch::from_utc`); the
///   sigma is the ΔT model's envelope for the epoch (0 inside the table would
///   overstate certainty about UT1, which is still ≈ UTC ± 0.9 s).
#[derive(Debug, Clone, Copy)]
pub struct Instant {
    pub jd_ut1: JdUT1,
    pub jd_tt: JdTT,
    /// 1σ of the SMH2016 ΔT model at this epoch, seconds (xalen-time).
    pub delta_t_sigma_sec: f64,
}

impl Instant {
    pub fn from_utc_fields(f: &UtcFields) -> Self {
        let e = Epoch::from_utc(f.year, f.month, f.day, f.hour, f.minute, f.second, 0.0);
        let jd_utc = e.jd_ut1.as_f64();
        if jd_utc < UTC_LEAP_SECOND_ERA_START_JD {
            return Self::from_jd_ut1(jd_utc);
        }
        let tt = e.jd_tt();
        let (_, sigma) =
            delta_t_with_uncertainty(jd_utc, &DeltaTModel::StephensonMorrisonHohenkerk2016);
        Self {
            jd_ut1: e.jd_ut1,
            jd_tt: tt,
            delta_t_sigma_sec: sigma,
        }
    }

    /// An instant with both scales supplied explicitly (differential tooling
    /// that wants to evaluate XALEN at another engine's UT1/TT pair).
    pub fn from_parts(jd_ut1: f64, jd_tt: f64) -> Self {
        let (_, sigma) =
            delta_t_with_uncertainty(jd_ut1, &DeltaTModel::StephensonMorrisonHohenkerk2016);
        Self {
            jd_ut1: JdUT1(jd_ut1),
            jd_tt: JdTT(jd_tt),
            delta_t_sigma_sec: sigma,
        }
    }

    /// From a UT JD (sweeps). TT via the ΔT model.
    pub fn from_jd_ut1(jd: f64) -> Self {
        let ut1 = JdUT1(jd);
        let (tt, sigma) = ut1.to_tt_with_sigma(&DeltaTModel::StephensonMorrisonHohenkerk2016);
        Self {
            jd_ut1: ut1,
            jd_tt: tt,
            delta_t_sigma_sec: sigma,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BodyState {
    pub body: BodyId,
    pub tropical_lon_deg: f64,
    pub sidereal_lon_deg: f64,
    pub latitude_deg: f64,
    pub distance_au: f64,
    /// Sidereal longitude rate, degrees/day (tropical rate minus ayanamsa rate).
    pub speed_deg_per_day: f64,
    pub retrograde: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Houses {
    pub system: HouseSystem,
    /// Sidereal cusps, degrees, index 0 = house 1.
    pub cusps: [f64; 12],
    pub ascendant_deg: f64,
    pub mc_deg: f64,
    pub fallback_used: bool,
}

/// Seconds of clock time until the value flips, from a linear extrapolation
/// of the body's current speed. `None` when the body is stationary.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BoundaryDistance {
    pub rashi_sec: Option<f64>,
    pub nakshatra_sec: Option<f64>,
    pub pada_sec: Option<f64>,
    pub direction: &'static str,
    pub method: &'static str,
}

pub fn boundary_distance(sidereal_lon_deg: f64, speed_deg_per_day: f64) -> BoundaryDistance {
    const STATIONARY: f64 = 1e-9;
    let lon = sidereal_lon_deg.rem_euclid(360.0);
    let direction = if speed_deg_per_day > STATIONARY {
        "forward"
    } else if speed_deg_per_day < -STATIONARY {
        "retrograde"
    } else {
        "stationary"
    };
    let secs = |span: f64| -> Option<f64> {
        if direction == "stationary" {
            return None;
        }
        let pos = lon.rem_euclid(span);
        let dist = if speed_deg_per_day > 0.0 {
            span - pos
        } else {
            pos
        };
        Some(dist / speed_deg_per_day.abs() * 86400.0)
    };
    BoundaryDistance {
        rashi_sec: secs(RASHI_SPAN_DEG),
        nakshatra_sec: secs(NAKSHATRA_SPAN_DEG),
        pada_sec: secs(PADA_SPAN_DEG),
        direction,
        method: "linear_from_speed",
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ZodiacPlacement {
    pub rashi: String,
    pub rashi_index: usize,
    pub rashi_lord: &'static str,
    pub degree_in_rashi: f64,
    pub nakshatra: String,
    pub nakshatra_index: usize,
    pub nakshatra_lord: String,
    pub pada: u8,
}

pub fn placement(sidereal_lon_deg: f64) -> ZodiacPlacement {
    let lon = sidereal_lon_deg.rem_euclid(360.0);
    let rashi = Rashi::from_longitude_deg(lon);
    let nak = Nakshatra::from_longitude_deg(lon);
    ZodiacPlacement {
        rashi: format!("{rashi:?}"),
        rashi_index: rashi.index(),
        rashi_lord: rashi.lord(),
        degree_in_rashi: lon.rem_euclid(RASHI_SPAN_DEG),
        nakshatra: format!("{nak:?}"),
        nakshatra_index: nak.index(),
        nakshatra_lord: format!("{:?}", nak.lord()),
        pada: Nakshatra::pada(lon),
    }
}

pub fn dasha_lord_name(l: DashaLord) -> String {
    format!("{l:?}")
}

pub fn vara_name(v: Vara) -> &'static str {
    v.name()
}

pub struct Engine {
    almanac: Almanac,
    pub kernel_sha256: String,
    pub kernel_path: String,
    pub kernel_id: String,
    /// TDB coverage of the loaded kernel, Julian Days.
    pub coverage_jd: (f64, f64),
}

/// Streaming SHA-256 of a file (the kernel is ~32 MB; never read into a String).
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// Read the pinned hash: the first 64-hex-char token of the `.sha256` file
/// (the `shasum -a 256` format is `<hex>  <path>`).
pub fn read_pinned_sha256(path: &Path) -> Result<String, EngineError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| EngineError::Kernel(format!("cannot read {}: {e}", path.display())))?;
    text.split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .find(|t| t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| {
            EngineError::Kernel(format!(
                "{} contains no 64-hex-digit SHA-256 token",
                path.display()
            ))
        })
}

impl Engine {
    /// Verify the pinned hash, parse the kernel, confirm DE440 provenance, and
    /// build the almanac. Any failure aborts boot.
    pub fn load(kernel: &Path, sha_file: &Path) -> Result<Self, EngineError> {
        let pinned = read_pinned_sha256(sha_file)?;
        let actual = sha256_file(kernel)
            .map_err(|e| EngineError::Kernel(format!("cannot hash {}: {e}", kernel.display())))?;
        if pinned != actual {
            return Err(EngineError::Kernel(format!(
                "SHA-256 mismatch for {}: pinned {pinned} != actual {actual}",
                kernel.display()
            )));
        }
        let reader = De440Reader::from_file(kernel)
            .map_err(|e| EngineError::Kernel(format!("cannot parse {}: {e}", kernel.display())))?;
        let header = reader.header();
        let coverage_jd = (header.jd_start, header.jd_end);
        let kernel_id = reader.kernel_id().unwrap_or("").to_string();
        let provider = De440Provider::with_reader(reader);
        if !provider.is_de440_loaded() {
            return Err(EngineError::Kernel(format!(
                "{} parsed as SPK but provenance is not confirmed DE440 (kernel_id={kernel_id:?}); refusing to serve analytical-grade positions",
                kernel.display()
            )));
        }
        let (pstart, pend) = provider.coverage();
        // The provider reports the kernel's window when data is loaded.
        let coverage_jd = (coverage_jd.0.max(pstart), coverage_jd.1.min(pend));
        let almanac = Almanac::default_vedic().with_provider(Arc::new(provider));
        Ok(Self {
            almanac,
            kernel_sha256: actual,
            kernel_path: kernel.display().to_string(),
            kernel_id,
            coverage_jd,
        })
    }

    pub fn check_coverage(&self, at: &Instant) -> Result<(), EngineError> {
        let jd = at.jd_tt.as_f64();
        let (s, e) = self.coverage_jd;
        if jd < s + COVERAGE_MARGIN_DAYS || jd > e - COVERAGE_MARGIN_DAYS {
            return Err(EngineError::Coverage {
                jd_tt: jd,
                start: s,
                end: e,
            });
        }
        Ok(())
    }

    /// Lahiri ayanamsa, TRUE-equinox convention (with nutation) — the value
    /// subtracted from apparent tropical longitudes to form sidereal ones
    /// (Swiss `swe_get_ayanamsa_ex_ut(jd, SEFLG_SWIEPH)` equivalent).
    pub fn ayanamsa_deg(&self, at: &Instant) -> f64 {
        Ayanamsa::Lahiri.compute_deg(at.jd_tt.as_f64())
    }

    /// Nutation in longitude Δψ (IAU 2000B), degrees, at the instant's TT.
    pub fn nutation_dpsi_deg(&self, at: &Instant) -> f64 {
        xalen_coords::nutation_2000b(at.jd_tt.julian_centuries_from_j2000())
            .delta_psi
            .to_degrees()
    }

    /// Lahiri ayanamsa, MEAN-equinox convention (no nutation) — Swiss
    /// `swe_get_ayanamsa_ut` equivalent. Equals `ayanamsa_deg − Δψ`.
    pub fn ayanamsa_mean_equinox_deg(&self, at: &Instant) -> f64 {
        self.ayanamsa_deg(at) - self.nutation_dpsi_deg(at)
    }

    /// Ayanamsa rate, degrees/day (≈ 50.29″/yr).
    pub fn ayanamsa_rate_deg_per_day(&self, at: &Instant) -> f64 {
        let t = at.jd_tt.as_f64();
        Ayanamsa::Lahiri.compute_deg(t + 0.5) - Ayanamsa::Lahiri.compute_deg(t - 0.5)
    }

    pub fn body_state(&self, id: BodyId, at: &Instant) -> Result<BodyState, EngineError> {
        self.check_coverage(at)?;
        let body = id.xalen_body();
        let pos = self.almanac.geocentric_ecliptic_tt(body, at.jd_tt)?;
        let speed = self.almanac.geocentric_speed_tt(body, at.jd_tt)?;
        let aya = self.ayanamsa_deg(at);
        let mut trop = pos.longitude.to_degrees().rem_euclid(360.0);
        let mut lat = pos.latitude.to_degrees();
        if id == BodyId::Ketu {
            trop = (trop + 180.0).rem_euclid(360.0);
            lat = -lat;
        }
        let sid = (trop - aya).rem_euclid(360.0);
        let rate = speed.longitude_deg_per_day() - self.ayanamsa_rate_deg_per_day(at);
        Ok(BodyState {
            body: id,
            tropical_lon_deg: trop,
            sidereal_lon_deg: sid,
            latitude_deg: lat,
            distance_au: pos.distance,
            speed_deg_per_day: rate,
            retrograde: rate < 0.0,
        })
    }

    pub fn sidereal_lon(&self, id: BodyId, at: &Instant) -> Result<f64, EngineError> {
        self.check_coverage(at)?;
        let body = id.xalen_body();
        let pos = self.almanac.geocentric_ecliptic_tt(body, at.jd_tt)?;
        let aya = self.ayanamsa_deg(at);
        let mut trop = pos.longitude.to_degrees();
        if id == BodyId::Ketu {
            trop += 180.0;
        }
        Ok((trop - aya).rem_euclid(360.0))
    }

    /// Sidereal Sun and Moon longitudes at a UT JD — the closure shape the
    /// xalen-vedic panchang transition finder expects.
    pub fn sun_moon_sidereal(&self, jd_ut1: f64) -> Result<(f64, f64), EngineError> {
        let at = Instant::from_jd_ut1(jd_ut1);
        Ok((
            self.sidereal_lon(BodyId::Sun, &at)?,
            self.sidereal_lon(BodyId::Moon, &at)?,
        ))
    }

    /// Sidereal houses on the true equinox of date (GAST + true obliquity),
    /// the same reduction Swiss `swe_houses_ex(SEFLG_SIDEREAL)` applies, so
    /// the Ascendant is comparable to PyJHora within the 0.01° tolerance.
    pub fn houses(
        &self,
        at: &Instant,
        lat: f64,
        lon: f64,
        system: HouseSystem,
    ) -> Result<Houses, EngineError> {
        self.check_coverage(at)?;
        let loc = GeoLocation::try_new(lat, lon)
            .ok_or_else(|| EngineError::Input(format!("invalid lat/lon {lat}/{lon}")))?;
        let t_tt = at.jd_tt.julian_centuries_from_j2000();
        let nut = xalen_coords::nutation_2000b(t_tt);
        let epsilon = xalen_coords::mean_obliquity(t_tt) + nut.delta_epsilon;
        let armc_deg = (xalen_coords::gast_deg(at.jd_ut1.as_f64(), t_tt) + lon).rem_euclid(360.0);
        let h = compute_houses_from_ramc(armc_deg.to_radians(), &loc, epsilon, system);
        let h = h.to_sidereal(self.ayanamsa_deg(at).to_radians());
        let ascendant_deg = h.ascendant.to_degrees().rem_euclid(360.0);
        let mut cusps = [0.0; 12];
        if system == HouseSystem::WholeSign {
            // xalen builds Whole-Sign cusps in the tropical frame and then
            // shifts them by the ayanamsa, which lands cusp 1 at (tropical sign
            // start − ayanamsa), not at the SIDEREAL sign start. A Vedic rasi
            // chart needs sidereal sign boundaries, so derive them from the
            // sidereal Ascendant directly.
            let first = (ascendant_deg / RASHI_SPAN_DEG).floor() * RASHI_SPAN_DEG;
            for (i, c) in cusps.iter_mut().enumerate() {
                *c = (first + RASHI_SPAN_DEG * i as f64).rem_euclid(360.0);
            }
        } else {
            for (i, c) in cusps.iter_mut().enumerate() {
                *c = h.cusp_deg(i);
            }
        }
        Ok(Houses {
            system,
            cusps,
            ascendant_deg,
            mc_deg: h.mc.to_degrees().rem_euclid(360.0),
            fallback_used: h.fallback_used,
        })
    }

    /// Sidereal Ascendant rate in degrees/day by a 60-second central difference.
    pub fn ascendant_rate_deg_per_day(
        &self,
        at: &Instant,
        lat: f64,
        lon: f64,
    ) -> Result<f64, EngineError> {
        let h = 30.0 / 86400.0;
        let a1 = self
            .houses(
                &Instant::from_jd_ut1(at.jd_ut1.as_f64() - h),
                lat,
                lon,
                HouseSystem::WholeSign,
            )?
            .ascendant_deg;
        let a2 = self
            .houses(
                &Instant::from_jd_ut1(at.jd_ut1.as_f64() + h),
                lat,
                lon,
                HouseSystem::WholeSign,
            )?
            .ascendant_deg;
        Ok(signed_delta_deg(a2, a1) / (2.0 * h))
    }

    /// Next rise/transit/set of a body after `jd_ut1` for an observer.
    pub fn rise_set_next(
        &self,
        id: BodyId,
        jd_ut1: f64,
        lat: f64,
        lon: f64,
    ) -> Result<xalen_ephem::RiseTransitSet, EngineError> {
        let at = Instant::from_jd_ut1(jd_ut1);
        self.check_coverage(&at)?;
        Ok(self
            .almanac
            .rise_transit_set_next(id.xalen_body(), JdUT1(jd_ut1), lat, lon, 0.0)?)
    }

    /// Vimshottari maha+antar periods from the sidereal Moon at birth.
    pub fn vimshottari(&self, moon_sidereal_deg: f64, birth_jd_ut1: f64) -> Vec<DashaPeriod> {
        vimshottari_dasha(moon_sidereal_deg, birth_jd_ut1, DashaLevel::Antardasha)
    }

    /// Run the docs/CONTRACT.md golden chart and assert Moon ∈ Swati.
    pub fn golden_self_test(&self) -> Result<GoldenReport, EngineError> {
        let fields = UtcFields {
            year: 1990,
            month: 3,
            day: 15,
            hour: 6,
            minute: 30,
            second: 0.0,
        };
        let at = Instant::from_utc_fields(&fields);
        let moon = self.body_state(BodyId::Moon, &at)?;
        let lon = moon.sidereal_lon_deg;
        let (lo, hi) = (186.6667, 200.0);
        if !(lo..hi).contains(&lon) {
            return Err(EngineError::Golden(format!(
                "Moon sidereal (Lahiri) longitude {lon:.6}° not in Swati [{lo}, {hi})"
            )));
        }
        Ok(GoldenReport {
            moon_sidereal_lon_deg: lon,
            moon_nakshatra: format!("{:?}", Nakshatra::from_longitude_deg(lon)),
            ayanamsa_deg: self.ayanamsa_deg(&at),
            jd_ut1: at.jd_ut1.as_f64(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GoldenReport {
    pub moon_sidereal_lon_deg: f64,
    pub moon_nakshatra: String,
    pub ayanamsa_deg: f64,
    pub jd_ut1: f64,
}

/// Locate the maha/antar lords active at `jd` in a Vimshottari sequence.
pub fn dasha_lords_at(periods: &[DashaPeriod], jd: f64) -> Option<(DashaLord, Option<DashaLord>)> {
    let maha = periods.iter().find(|p| jd >= p.start_jd && jd < p.end_jd)?;
    let antar = maha
        .sub_periods
        .iter()
        .find(|p| jd >= p.start_jd && jd < p.end_jd)
        .map(|p| p.lord);
    Some((maha.lord, antar))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_forward_moon_like() {
        // Moon at 189.0° sidereal, 13.2°/day forward.
        let b = boundary_distance(189.0, 13.2);
        assert_eq!(b.direction, "forward");
        // rashi Tula spans 180–210: 21° to go
        let expect_rashi = 21.0 / 13.2 * 86400.0;
        assert!((b.rashi_sec.unwrap() - expect_rashi).abs() < 1e-6);
        // nakshatra Swati spans 186.6667–200.0: 11.0° to go
        let expect_nak = (200.0 - 189.0) / 13.2 * 86400.0;
        assert!((b.nakshatra_sec.unwrap() - expect_nak).abs() < 1e-6);
        // pada: 189.0 is 2.3333 into Swati -> pada 1 ends at 190.0
        let expect_pada = 1.0 / 13.2 * 86400.0;
        assert!((b.pada_sec.unwrap() - expect_pada).abs() < 1e-6);
        assert_eq!(b.method, "linear_from_speed");
    }

    #[test]
    fn boundary_retrograde_counts_backward() {
        // Mars retrograde at 31.0°, -0.25°/day: 1° back to 30.0 (rashi flip)
        let b = boundary_distance(31.0, -0.25);
        assert_eq!(b.direction, "retrograde");
        assert!((b.rashi_sec.unwrap() - 1.0 / 0.25 * 86400.0).abs() < 1e-6);
        // 31.0° is in Krittika (26.667–40.0): 4.333° back to its start
        let expect = (31.0 - 2.0 * NAKSHATRA_SPAN_DEG) / 0.25 * 86400.0;
        assert!((b.nakshatra_sec.unwrap() - expect).abs() < 1e-6);
    }

    #[test]
    fn boundary_stationary_is_none() {
        let b = boundary_distance(100.0, 0.0);
        assert_eq!(b.direction, "stationary");
        assert!(b.rashi_sec.is_none() && b.nakshatra_sec.is_none() && b.pada_sec.is_none());
    }

    #[test]
    fn boundary_wraps_at_360() {
        let b = boundary_distance(359.5, 1.0);
        assert!((b.rashi_sec.unwrap() - 0.5 * 86400.0).abs() < 1e-6);
        let b = boundary_distance(0.25, -1.0);
        assert!((b.rashi_sec.unwrap() - 0.25 * 86400.0).abs() < 1e-6);
    }

    #[test]
    fn placement_maps_rashi_nakshatra_pada() {
        let p = placement(0.0);
        assert_eq!(p.rashi, "Mesha");
        assert_eq!(p.nakshatra, "Ashwini");
        assert_eq!(p.pada, 1);
        assert_eq!(p.nakshatra_lord, "Ketu");

        let p = placement(193.0); // Tula / Swati pada 2 (190.0–193.333)
        assert_eq!(p.rashi, "Tula");
        assert_eq!(p.rashi_index, 6);
        assert_eq!(p.nakshatra, "Swati");
        assert_eq!(p.nakshatra_index, 14);
        assert_eq!(p.pada, 2);
        assert_eq!(p.nakshatra_lord, "Rahu");
        assert!((p.degree_in_rashi - 13.0).abs() < 1e-9);

        let p = placement(359.999);
        assert_eq!(p.rashi, "Meena");
        assert_eq!(p.nakshatra, "Revati");
        assert_eq!(p.pada, 4);

        let p = placement(-1.0); // negative input normalises
        assert_eq!(p.rashi, "Meena");
    }

    #[test]
    fn pinned_sha_parser_accepts_shasum_format() {
        let dir = std::env::temp_dir().join(format!("jyotish-sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.sha256");
        std::fs::write(
            &p,
            "C1C7FEEAB882263FC493A9D5A5B2DDD71B54826CDF65D8D17A76126B260A49F2  kernels/de440s.bsp\n",
        )
        .unwrap();
        assert_eq!(
            read_pinned_sha256(&p).unwrap(),
            "c1c7feeab882263fc493a9d5a5b2ddd71b54826cdf65d8d17a76126b260a49f2"
        );
        std::fs::write(&p, "not a hash\n").unwrap();
        assert!(read_pinned_sha256(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instant_post_1972_is_leap_second_exact() {
        // 1990-03-15T06:30:00Z: TAI-UTC = 25 s → TT-UTC = 57.184 s (Swiss utc_to_jd agrees:
        // jd_et 2447965.7714951853 for the golden chart).
        let f = UtcFields {
            year: 1990,
            month: 3,
            day: 15,
            hour: 6,
            minute: 30,
            second: 0.0,
        };
        let at = Instant::from_utc_fields(&f);
        assert!((at.jd_ut1.as_f64() - 2447965.7708333335).abs() * 86400.0 < 1e-3);
        assert!((at.jd_tt.as_f64() - 2447965.7714951853).abs() * 86400.0 < 1e-3);
        assert!(((at.jd_tt.as_f64() - at.jd_ut1.as_f64()) * 86400.0 - 57.184).abs() < 1e-3);
    }

    #[test]
    fn instant_pre_1972_uses_delta_t_model_not_leap_floor() {
        // 1950-06-15T06:30:00Z: ΔT ≈ 29.1 s. The leap-second floor would give 42.184 s,
        // 13 s late (Moon 7″) — Swiss utc_to_jd gives jd_et 2433447.7711702073.
        let f = UtcFields {
            year: 1950,
            month: 6,
            day: 15,
            hour: 6,
            minute: 30,
            second: 0.0,
        };
        let at = Instant::from_utc_fields(&f);
        let tt_minus_ut = (at.jd_tt.as_f64() - at.jd_ut1.as_f64()) * 86400.0;
        assert!(
            (28.5..30.0).contains(&tt_minus_ut),
            "TT-UT1 = {tt_minus_ut}"
        );
        assert!((at.jd_tt.as_f64() - 2433447.7711702073).abs() * 86400.0 < 0.3);
        // 1900: ΔT is slightly negative; the floor would be +42 s.
        let f = UtcFields {
            year: 1900,
            month: 6,
            day: 15,
            hour: 6,
            minute: 30,
            second: 0.0,
        };
        let at = Instant::from_utc_fields(&f);
        let tt_minus_ut = (at.jd_tt.as_f64() - at.jd_ut1.as_f64()) * 86400.0;
        assert!((-3.0..1.0).contains(&tt_minus_ut), "TT-UT1 = {tt_minus_ut}");
        // The boundary instant itself is in the leap-second era (TAI-UTC = 10 s).
        let f = UtcFields {
            year: 1972,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0.0,
        };
        let at = Instant::from_utc_fields(&f);
        let tt_minus_ut = (at.jd_tt.as_f64() - at.jd_ut1.as_f64()) * 86400.0;
        assert!(
            (tt_minus_ut - 42.184).abs() < 1e-3,
            "TT-UT1 = {tt_minus_ut}"
        );
    }

    #[test]
    fn instant_from_parts_keeps_both_scales() {
        let at = Instant::from_parts(2447965.7708355812, 2447965.7714951853);
        assert_eq!(at.jd_ut1.as_f64(), 2447965.7708355812);
        assert_eq!(at.jd_tt.as_f64(), 2447965.7714951853);
        assert!(at.delta_t_sigma_sec > 0.0);
    }

    #[test]
    fn body_id_parse_roundtrip() {
        for b in BodyId::GRAHAS.iter().chain(BodyId::OUTER.iter()) {
            assert_eq!(BodyId::parse(b.name()), Some(*b));
        }
        assert_eq!(BodyId::parse("moon"), Some(BodyId::Moon));
        assert_eq!(BodyId::parse("Earth"), None);
    }
}

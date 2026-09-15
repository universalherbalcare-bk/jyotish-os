"""UTC <-> Julian Day <-> PyJHora local (Date, time-of-birth) conversions.

Conventions (from vendor/pyjhora/src/jhora/panchanga/drik.py and utils.py):
  * PyJHora "jd" is a *local-clock* Julian Day: swe.julday(y, m, d, local_hours).
  * PyJHora jd_utc = jd - place.timezone / 24.
  * PyJHora dasha rows carry start instants as (Y, M, D, fractional_local_hours).

CONTRACT: `utc` is authoritative; `tz_offset_hours` only renders local time for PyJHora.
"""

from __future__ import annotations

import math
from datetime import UTC, datetime

import swisseph as swe


def utc_datetime_to_jd_ut(dt: datetime) -> float:
    """Aware datetime -> JD(UT1) via pyswisseph's leap-second aware utc_to_jd."""
    if dt.tzinfo is None:
        raise ValueError("utc datetime must be timezone-aware")
    dt = dt.astimezone(UTC)
    seconds = dt.second + dt.microsecond / 1_000_000.0
    _jd_et, jd_ut = swe.utc_to_jd(
        dt.year, dt.month, dt.day, dt.hour, dt.minute, seconds, swe.GREG_CAL
    )
    return float(jd_ut)


def jd_ut_to_iso(jd_ut: float, precision: int = 3) -> str:
    """JD(UT1) -> ISO-8601 UTC string. Works outside datetime's year range."""
    y, m, d, h, mi, s = swe.jdut1_to_utc(jd_ut, swe.GREG_CAL)
    # jdut1_to_utc can return s == 60.0 at leap-second edges or rounding artefacts; normalize.
    s = round(float(s), precision)
    if s >= 60.0:
        # push forward through swe to keep calendar arithmetic correct
        return jd_ut_to_iso(jd_ut + (0.5 * 10**-precision) / 86400.0, precision)
    sec_int = math.floor(s)
    frac = s - sec_int
    frac_str = f"{frac:.{precision}f}"[1:] if precision > 0 else ""
    sign = "-" if y < 0 else ""
    return f"{sign}{abs(int(y)):04d}-{int(m):02d}-{int(d):02d}T{int(h):02d}:{int(mi):02d}:{sec_int:02d}{frac_str}Z"


def jd_ut_to_local_jd(jd_ut: float, tz_offset_hours: float) -> float:
    return jd_ut + tz_offset_hours / 24.0


def local_jd_to_jd_ut(jd_local: float, tz_offset_hours: float) -> float:
    return jd_local - tz_offset_hours / 24.0


def local_jd_to_ymd_hours(jd_local: float) -> tuple[int, int, int, float]:
    y, m, d, fh = swe.revjul(jd_local, swe.GREG_CAL)
    return int(y), int(m), int(d), float(fh)


def hours_to_hms(fh: float) -> tuple[int, int, float]:
    """Fractional hours -> (h, m, s_float); PyJHora's julian_day_number accepts float seconds."""
    h = math.floor(fh)
    rem = (fh - h) * 60.0
    mi = math.floor(rem)
    s = (rem - mi) * 60.0
    return h, mi, s


def local_ymd_hours_to_jd_ut(
    y: int, m: int, d: int, fh: float, tz_offset_hours: float
) -> float:
    """PyJHora (Y, M, D, local fractional hours) -> JD(UT)."""
    return (
        swe.julday(int(y), int(m), int(d), float(fh), swe.GREG_CAL)
        - tz_offset_hours / 24.0
    )


def local_date_midnight_jd_ut(y: int, m: int, d: int, tz_offset_hours: float) -> float:
    return (
        swe.julday(int(y), int(m), int(d), 0.0, swe.GREG_CAL) - tz_offset_hours / 24.0
    )


def parse_pyjhora_datetime_string(s: str) -> tuple[int, int, int, float]:
    """Parse PyJHora's 'YYYY-MM-DD HH:MM:SS AM/PM' strings.

    PyJHora emits 24-hour clock values with a redundant AM/PM suffix
    (observed: '2133-06-16 18:48:22 PM', '1996-03-15 01:13:39 AM'), so the suffix is ignored.
    """
    parts = s.strip().split()
    if len(parts) < 2:
        raise ValueError(f"unrecognized PyJHora datetime string: {s!r}")
    date_part, time_part = parts[0], parts[1]
    y, m, d = (int(x) for x in date_part.split("-"))
    hh, mm, ss = time_part.split(":")
    fh = int(hh) + int(mm) / 60.0 + float(ss) / 3600.0
    return y, m, d, fh

"""Worker-side computations (run inside the process pool; Swiss Ephemeris state is per-process).

Every function here takes plain scalars and returns JSON-serialisable dicts so results cross
the process boundary without pickling PyJHora objects.
"""

from __future__ import annotations

import contextlib
import io
from typing import Any

import swisseph as swe

from jhora_svc import bootstrap as _bootstrap
from jhora_svc import timeconv
from jhora_svc.names import (
    NAKSHATRAS,
    PLANETS,
    RASIS,
    TITHIS,
    YOGAS,
    karana_name,
    paksha_of_tithi,
)

# Apparent geocentric sidereal positions with speeds - the Swiss default used for the
# cross-engine consensus (CONTRACT tolerances). PyJHora's *internal* techniques keep their own
# PLANET_FLAGS (FLG_TRUEPOS geometric positions); the two differ by ~20 arcsec (aberration).
POSITION_FLAGS = swe.FLG_SWIEPH | swe.FLG_SIDEREAL | swe.FLG_SPEED

_BODY_IDS: tuple[tuple[str, int], ...] = (
    ("Sun", swe.SUN),
    ("Moon", swe.MOON),
    ("Mercury", swe.MERCURY),
    ("Venus", swe.VENUS),
    ("Mars", swe.MARS),
    ("Jupiter", swe.JUPITER),
    ("Saturn", swe.SATURN),
)

GOLDEN = {
    "utc": "1990-03-15T06:30:00Z",
    "lat": 28.6139,
    "lon": 77.2090,
    "tz_offset_hours": 5.5,
}
GOLDEN_JD_UT = float(swe.utc_to_jd(1990, 3, 15, 6, 30, 0.0, swe.GREG_CAL)[1])


def _ensure_lahiri() -> None:
    """Swiss sid mode is process-global; re-assert before every computation (cheap)."""
    _bootstrap.bootstrap()
    from jhora.panchanga import drik

    drik.set_ayanamsa_mode(_bootstrap.AYANAMSA)


def _nak_pada(lon: float) -> tuple[int, int]:
    from jhora.panchanga import drik

    nak, pada, _rem = drik.nakshatra_pada(lon)
    return int(nak), int(pada)


def _body(lon: float, lat: float, speed: float) -> dict[str, Any]:
    lon = lon % 360.0
    nak, pada = _nak_pada(lon)
    rasi_index = int(lon // 30)
    return {
        "lon": lon,
        "speed": speed,
        "retro": speed < 0.0,
        "lat": lat,
        "rasi": RASIS[rasi_index],
        "rasi_index": rasi_index,
        "nakshatra": NAKSHATRAS[nak - 1],
        "nakshatra_index": nak,
        "pada": pada,
    }


def positions(jd_ut: float, lat: float, lon: float, tz: float) -> dict[str, Any]:
    _ensure_lahiri()
    from jhora import const
    from jhora.panchanga import drik

    swe.set_sid_mode(swe.SIDM_LAHIRI)
    bodies: dict[str, Any] = {}
    for name, pid in _BODY_IDS:
        xx, _rf = swe.calc_ut(jd_ut, pid, POSITION_FLAGS)
        bodies[name] = _body(xx[0], xx[1], xx[3])
    rahu, _rf = swe.calc_ut(
        jd_ut, const._RAHU, POSITION_FLAGS
    )  # const._RAHU == swe.TRUE_NODE
    bodies["Rahu"] = _body(rahu[0], rahu[1], rahu[3])
    bodies["Ketu"] = _body(rahu[0] + 180.0, -rahu[1], rahu[3])
    ordered = {k: bodies[k] for k in PLANETS[:9]}

    place = drik.Place("svc", lat, lon, tz)
    jd_local = timeconv.jd_ut_to_local_jd(jd_ut, tz)
    asc_sign, asc_deg_in_sign, _nak, _pada = drik.ascendant(jd_local, place)
    asc = (asc_sign * 30.0 + asc_deg_in_sign) % 360.0
    ayan = float(swe.get_ayanamsa_ut(jd_ut))
    # leave the process in the state PyJHora expects
    drik.set_ayanamsa_mode(_bootstrap.AYANAMSA)
    return {
        "bodies": ordered,
        "ascendant": asc,
        "ascendant_rasi": RASIS[int(asc // 30)],
        "ayanamsa_deg": ayan,
        "jd_ut": jd_ut,
        "ayanamsa": "LAHIRI",
        "true_nodes": True,
        "flags": {
            "positions_swe_flags": int(POSITION_FLAGS),
            "positions_semantics": "apparent geocentric sidereal (FLG_SWIEPH|FLG_SIDEREAL|FLG_SPEED)",
            "pyjhora_internal_flags": int(drik.PLANET_FLAGS),
            "node": "TRUE_NODE",
            "ascendant_source": "drik.ascendant (swe.houses_ex sidereal)",
        },
    }


# ----------------------------------------------------------------------------- panchang

_DAY_SPAN = 2.0  # bracket (days) for transition bisection; Moon moves < 30 deg in it


def _crossing(fn, target: float, lo: float, hi: float) -> float:
    """First jd in (lo, hi] where the increasing angle fn(jd) mod 360 reaches `target`.

    fn must be monotonically increasing over the bracket (Moon longitude, Moon-Sun elongation and
    Moon+Sun sum all are: the Moon is never retrograde and outpaces the Sun). Signed difference
    keeps wrap-around safe as long as the bracket spans < 180 deg of motion.
    """

    def f(t: float) -> float:
        return ((fn(t) - target + 180.0) % 360.0) - 180.0

    if f(lo) > 0 or f(hi) < 0:
        raise ValueError("bisection bracket does not contain the transition")
    for _ in range(64):
        mid = 0.5 * (lo + hi)
        if f(mid) < 0:
            lo = mid
        else:
            hi = mid
    return 0.5 * (lo + hi)


def _element(
    idx: int,
    width: float,
    fn,
    jd_ref_ut: float,
    name: str,
    extra: dict[str, Any] | None = None,
) -> dict[str, Any]:
    start = _crossing(fn, (idx - 1) * width, jd_ref_ut - _DAY_SPAN, jd_ref_ut)
    end = _crossing(fn, idx * width, jd_ref_ut, jd_ref_ut + _DAY_SPAN)
    return {
        "index": int(idx),
        "name": name,
        "start_utc": timeconv.jd_ut_to_iso(start),
        "end_utc": timeconv.jd_ut_to_iso(end),
        "extra": {**(extra or {}), "_end_jd": end},
    }


def _strip(el: dict[str, Any]) -> dict[str, Any]:
    el["extra"].pop("_end_jd", None)
    return el


def panchang(
    y: int, m: int, d: int, lat: float, lon: float, tz: float
) -> dict[str, Any]:
    """Panchanga of the local calendar date, referenced at that date's local sunrise.

    Identification (which tithi/nakshatra/yoga/karana) is arithmetic on PyJHora's sidereal
    Moon/Sun longitudes at the reference instant and is cross-checked against drik.tithi /
    drik.nakshatra / drik.yogam / drik.karana (fail closed on disagreement). Transition instants
    are solved by bisection on the same longitude functions (< 0.1 s), because PyJHora's own
    end-time estimators are linear extrapolations (minutes of error) and its inverse-Lagrange path
    evaluates the phase at a local JD as if it were UTC.
    """
    _ensure_lahiri()
    from jhora.panchanga import drik

    place = drik.Place("svc", lat, lon, tz)
    day_jd_ut = timeconv.local_date_midnight_jd_ut(y, m, d, tz)
    jd_local_midnight = swe.julday(int(y), int(m), int(d), 0.0, swe.GREG_CAL)

    with contextlib.redirect_stdout(io.StringIO()):
        rise_h, _s1, _j1 = drik.sunrise(jd_local_midnight, place)
        set_h, _s2, _j2 = drik.sunset(jd_local_midnight, place)
        next_rise_h, _s3, _j3 = drik.sunrise(jd_local_midnight + 1.0, place)
        jd_ref_local = jd_local_midnight + rise_h / 24.0
        jd_ref_ut = timeconv.local_jd_to_jd_ut(jd_ref_local, tz)
        next_rise_ut = day_jd_ut + 1.0 + next_rise_h / 24.0
        # vedic weekday runs sunrise->sunrise; evaluate 1 s after sunrise so float rounding of
        # PyJHora's own sunrise (it re-rounds to whole seconds) cannot flip to the previous day
        vaara_idx = int(drik.vaara(jd_ref_local + 1.0 / 86400.0, place))
        drik_t = int(drik.tithi(jd_ref_local, place)[0])
        drik_n = int(drik.nakshatra(jd_ref_local, place)[0])
        drik_y = int(drik.yogam(jd_ref_local, place)[0])
        drik_k = int(drik.karana(jd_ref_local, place)[0])

    moon_fn = drik.lunar_longitude
    sun_fn = drik.solar_longitude

    def elong(t: float) -> float:
        return (moon_fn(t) - sun_fn(t)) % 360.0

    def yoga_sum(t: float) -> float:
        return (moon_fn(t) + sun_fn(t)) % 360.0

    moon = float(moon_fn(jd_ref_ut))
    sun = float(sun_fn(jd_ref_ut))
    e = elong(jd_ref_ut)
    tithi_idx = int(e // 12.0) + 1
    nak_idx = int(moon // (360.0 / 27.0)) + 1
    yoga_idx = int(yoga_sum(jd_ref_ut) // (360.0 / 27.0)) + 1
    karana_idx = int(e // 6.0) + 1

    mismatches = {
        k: v
        for k, v in {
            "tithi": (tithi_idx, drik_t),
            "nakshatra": (nak_idx, drik_n),
            "yoga": (yoga_idx, drik_y),
            "karana": (karana_idx, drik_k),
        }.items()
        if v[0] != v[1]
    }
    if mismatches:
        raise ValueError(
            f"panchang identification disagrees with drik at reference: {mismatches}"
        )

    tithi = _element(
        tithi_idx,
        12.0,
        elong,
        jd_ref_ut,
        TITHIS[tithi_idx - 1],
        {"paksha": paksha_of_tithi(tithi_idx)},
    )
    nak = _element(
        nak_idx,
        360.0 / 27.0,
        moon_fn,
        jd_ref_ut,
        NAKSHATRAS[nak_idx - 1],
        {"pada_at_reference": int(((moon % (360.0 / 27.0)) // (360.0 / 108.0)) + 1)},
    )
    yoga = _element(yoga_idx, 360.0 / 27.0, yoga_sum, jd_ref_ut, YOGAS[yoga_idx - 1])
    karana = _element(karana_idx, 6.0, elong, jd_ref_ut, karana_name(karana_idx))

    def following(
        el: dict[str, Any], width: float, fn, idx: int, count: int, namer
    ) -> dict[str, Any] | None:
        end_jd = el["extra"]["_end_jd"]
        if end_jd >= next_rise_ut:
            return None
        nxt = idx % count + 1
        nxt_end = _crossing(fn, nxt * width, end_jd, end_jd + _DAY_SPAN)
        return {
            "index": nxt,
            "name": namer(nxt),
            "start_utc": timeconv.jd_ut_to_iso(end_jd),
            "end_utc": timeconv.jd_ut_to_iso(nxt_end),
            "extra": {},
        }

    next_tithi = following(tithi, 12.0, elong, tithi_idx, 30, lambda i: TITHIS[i - 1])
    if next_tithi:
        next_tithi["extra"]["paksha"] = paksha_of_tithi(next_tithi["index"])
    next_nak = following(
        nak, 360.0 / 27.0, moon_fn, nak_idx, 27, lambda i: NAKSHATRAS[i - 1]
    )
    next_yoga = following(
        yoga, 360.0 / 27.0, yoga_sum, yoga_idx, 27, lambda i: YOGAS[i - 1]
    )
    next_karana = following(karana, 6.0, elong, karana_idx, 60, karana_name)

    return {
        "date": f"{int(y):04d}-{int(m):02d}-{int(d):02d}",
        "reference_utc": timeconv.jd_ut_to_iso(jd_ref_ut),
        "reference": "local_sunrise",
        "sunrise_utc": timeconv.jd_ut_to_iso(day_jd_ut + rise_h / 24.0),
        "sunset_utc": timeconv.jd_ut_to_iso(day_jd_ut + set_h / 24.0),
        "next_sunrise_utc": timeconv.jd_ut_to_iso(next_rise_ut),
        "tithi": _strip(tithi),
        "nakshatra": _strip(nak),
        "yoga": _strip(yoga),
        "karana": _strip(karana),
        "next_tithi": next_tithi,
        "next_nakshatra": next_nak,
        "next_yoga": next_yoga,
        "next_karana": next_karana,
        "vaara": [
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ][vaara_idx % 7],
        "moon_longitude_at_reference": moon,
        "sun_longitude_at_reference": sun,
        "longitude_swe_flags": int(drik.PLANET_FLAGS),
        "method": "identification at sunrise via drik sidereal longitudes (cross-checked with drik.tithi/"
        "nakshatra/yogam/karana); transitions by bisection on drik.lunar/solar_longitude",
    }


# ----------------------------------------------------------------------------- kuta

_ASHTAKOOTA_ORDER = (
    "varna",
    "vasya",
    "gana",
    "tara",
    "yoni",
    "graha_maitri",
    "bhakoot",
    "nadi",
)
_ASHTAKOOTA_MAX = (1.0, 2.0, 6.0, 3.0, 4.0, 5.0, 7.0, 8.0)
_SOUTH_ORDER = (
    "dina",
    "gana",
    "mahendra",
    "sthree_dheerga",
    "yoni",
    "raasi",
    "raasi_adhipathi",
    "vasiya",
    "rajju",
    "vedha",
)


def _moon_star(jd_ut: float) -> dict[str, Any]:
    from jhora import const
    from jhora.panchanga import drik

    moon = float(drik.sidereal_longitude(jd_ut, const._MOON))
    nak, pada = _nak_pada(moon)
    return {
        "moon_lon": moon,
        "nakshatra": NAKSHATRAS[nak - 1],
        "nakshatra_index": nak,
        "pada": pada,
        "rasi": RASIS[int(moon // 30)],
    }


def kuta(jd_a: float, jd_b: float) -> dict[str, Any]:
    """Ashtakoota (north, 36 points) + the 10 south-Indian poruthams from PyJHora compatibility.py.

    `a` is treated as the boy/first partner, `b` as the girl/second partner (PyJHora's parameter
    order). Only Moon nakshatra + pada enter the calculation, exactly as in the vendored code.
    """
    _ensure_lahiri()
    from jhora import const
    from jhora.horoscope.match import compatibility as C

    a = _moon_star(jd_a)
    b = _moon_star(jd_b)
    north = C.Ashtakoota(
        a["nakshatra_index"],
        a["pada"],
        b["nakshatra_index"],
        b["pada"],
        method=const.CHART_STYLE.NORTH_INDIAN,
    )
    with contextlib.redirect_stdout(io.StringIO()):
        res = north.compatibility_score()
    scores = {}
    for i, key in enumerate(_ASHTAKOOTA_ORDER):
        scores[key] = {"score": float(res[i]), "max": _ASHTAKOOTA_MAX[i]}
    total = float(res[8])
    if abs(total - sum(s["score"] for s in scores.values())) > 1e-9:
        raise ValueError("PyJHora ashtakoota total does not equal the sum of its kutas")
    dosha = {
        "mahendra": bool(res[9]),
        "vedha": bool(res[10]),
        "rajju": bool(res[11]),
        "sthree_dheerga": bool(res[12]),
    }

    south = C.Ashtakoota(
        a["nakshatra_index"],
        a["pada"],
        b["nakshatra_index"],
        b["pada"],
        method=const.CHART_STYLE.SOUTH_INDIAN_REGULAR,
    )
    with contextlib.redirect_stdout(io.StringIO()):
        sres = south.compatibility_score()
    # south list: [varna, vasiya, gana, dina, yoni, raasi_adhipathi, raasi, naadi, score, mahendra, vedha, rajju, sthree, minimum]
    smap = {
        "vasiya": sres[1],
        "gana": sres[2],
        "dina": sres[3],
        "yoni": sres[4],
        "raasi_adhipathi": sres[5],
        "raasi": sres[6],
        "mahendra": sres[9],
        "vedha": sres[10],
        "rajju": sres[11],
        "sthree_dheerga": sres[12],
    }
    poruthams = {k: bool(smap[k]) for k in _SOUTH_ORDER}
    return {
        "a": a,
        "b": b,
        "ashtakoota": scores,
        "total": total,
        "total_max": 36.0,
        "dosha_checks": dosha,
        "south_indian_poruthams": poruthams,
        "south_indian_total": int(sres[8]),
        "south_indian_max": 10,
        "south_indian_minimum_met": bool(sres[13]) if len(sres) > 13 else False,
        "method": "PyJHora horoscope.match.compatibility.Ashtakoota (NORTH_INDIAN + SOUTH_INDIAN_REGULAR)",
    }


# ----------------------------------------------------------------------------- health / warm-up


def worker_health() -> dict[str, Any]:
    st = _bootstrap.self_check(GOLDEN_JD_UT)
    st["pyswisseph"] = str(swe.version)
    return st


def golden_warmup() -> dict[str, Any]:
    """Compute the golden chart end-to-end; used by every worker at init and by /v1/health."""
    from jhora_svc import dasha_catalog

    pos = positions(
        GOLDEN_JD_UT, GOLDEN["lat"], GOLDEN["lon"], GOLDEN["tz_offset_hours"]
    )
    moon = pos["bodies"]["Moon"]["lon"]
    if not (186.0 + 40.0 / 60.0 <= moon < 200.0):
        raise RuntimeError(f"golden chart Moon {moon:.4f} not in Swati (186.6667..200)")
    vim = dasha_catalog.compute_dasha(
        "graha.vimsottari",
        GOLDEN_JD_UT,
        GOLDEN["lat"],
        GOLDEN["lon"],
        GOLDEN["tz_offset_hours"],
        1,
    )
    total = sum(p["duration_years"] for p in vim["periods"])
    if abs(total - 120.0) > 0.01:
        raise RuntimeError(
            f"golden Vimshottari maha-dashas sum to {total:.4f} years, expected 120"
        )
    return {
        "moon_lon": moon,
        "vimsottari_total_years": total,
        "vimsottari_balance_years": vim["balance_at_birth_years"],
    }

"""Service tests against the live ASGI app (httpx) plus direct pyswisseph cross-checks."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest
import swisseph as swe

ARCSEC = 1.0 / 3600.0
SWATI_LO = 186.0 + 40.0 / 60.0
SWATI_HI = 200.0


def _angle_diff(a: float, b: float) -> float:
    return abs(((a - b + 180.0) % 360.0) - 180.0)


# ------------------------------------------------------------------ health / config


def test_health_reports_lahiri_true_nodes(client):
    r = client.get("/v1/health")
    assert r.status_code == 200, r.text
    h = r.json()
    assert h["ok"] is True
    assert h["ayanamsa"] == "LAHIRI"
    assert h["true_nodes"] is True
    assert h["pyswisseph"].startswith("2.10")
    assert h["ephe_files"] > 0
    assert h["self_check"]["ayanamsa_delta_arcsec"] < 1.0
    assert h["self_check"]["dhasa_year_duration"] == "MEAN_SIDEREAL_YEAR"
    assert h["catalog"]["total"] >= 40


def test_geocoding_functions_raise():
    from jhora_svc import bootstrap

    bootstrap.bootstrap()
    from jhora import place_db, utils

    for fn in (
        utils.get_location_using_nominatim,
        utils.get_place_from_user_ip_address,
        utils.get_elevation,
        utils.get_location,
        utils.get_place_from_latitude_longitude,
    ):
        with pytest.raises(RuntimeError, match="network geocoding disabled"):
            fn("Chennai, IN")
    with pytest.raises(RuntimeError, match="network geocoding disabled"):
        utils.Nominatim(user_agent="x")
    with pytest.raises(RuntimeError, match="network geocoding disabled"):
        utils.geocoder.ip("me")
    with pytest.raises(RuntimeError, match="network geocoding disabled"):
        utils.requests.get("http://example.invalid")
    with pytest.raises(RuntimeError, match="network geocoding disabled"):
        place_db.urllib.request.urlopen("http://example.invalid")


def test_pyjhora_const_pinned_in_test_process():
    from jhora_svc import bootstrap

    bootstrap.bootstrap()
    import jhora.config as cfg
    from jhora import const

    assert const._DEFAULT_AYANAMSA_MODE == "LAHIRI"
    assert const._use_true_nodes_for_rahu_ketu is True
    assert const._RAHU == swe.TRUE_NODE
    assert (
        const.dhasa_year_duration_default
        == const.DHASA_YEAR_DURATION.MEAN_SIDEREAL_YEAR
    )
    assert cfg._SETTINGS_LOADED is True  # user_settings.json can never be applied


def test_non_loopback_client_rejected(app_ctx):
    import httpx

    app, loop = app_ctx
    transport = httpx.ASGITransport(app=app, client=("10.0.0.5", 12345))
    ac = httpx.AsyncClient(transport=transport, base_url="http://testserver")
    r = loop.run_until_complete(ac.get("/v1/health"))
    loop.run_until_complete(ac.aclose())
    assert r.status_code == 403
    assert r.json()["error"] == "loopback_only"


# ------------------------------------------------------------------ positions


def test_golden_moon_in_swati(client, golden):
    r = client.post("/v1/positions", json=golden)
    assert r.status_code == 200, r.text
    p = r.json()
    moon = p["bodies"]["Moon"]["lon"]
    assert SWATI_LO <= moon < SWATI_HI
    assert p["bodies"]["Moon"]["nakshatra"] == "Swati"
    assert p["ayanamsa"] == "LAHIRI" and p["true_nodes"] is True
    assert 23.0 < p["ayanamsa_deg"] < 24.5
    assert set(p["bodies"]) == {
        "Sun",
        "Moon",
        "Mercury",
        "Venus",
        "Mars",
        "Jupiter",
        "Saturn",
        "Rahu",
        "Ketu",
    }


def test_positions_match_pyswisseph_direct(client, golden):
    r = client.post("/v1/positions", json=golden)
    p = r.json()
    jd = p["jd_ut"]
    # jd_ut itself must be the leap-second aware conversion of the input instant
    dt = datetime(1990, 3, 15, 6, 30, tzinfo=UTC)
    _et, jd_direct = swe.utc_to_jd(
        dt.year, dt.month, dt.day, dt.hour, dt.minute, 0.0, swe.GREG_CAL
    )
    assert abs(jd - jd_direct) * 86400.0 < 0.001

    swe.set_sid_mode(swe.SIDM_LAHIRI)
    flags = swe.FLG_SWIEPH | swe.FLG_SIDEREAL | swe.FLG_SPEED
    ids = {
        "Sun": swe.SUN,
        "Moon": swe.MOON,
        "Mercury": swe.MERCURY,
        "Venus": swe.VENUS,
        "Mars": swe.MARS,
        "Jupiter": swe.JUPITER,
        "Saturn": swe.SATURN,
    }
    for name, pid in ids.items():
        xx, _ = swe.calc_ut(jd, pid, flags)
        body = p["bodies"][name]
        assert _angle_diff(body["lon"], xx[0]) < 0.5 * ARCSEC, name
        assert abs(body["speed"] - xx[3]) < 1e-6, name
        assert body["retro"] == (xx[3] < 0)
    rahu, _ = swe.calc_ut(jd, swe.TRUE_NODE, flags)
    assert _angle_diff(p["bodies"]["Rahu"]["lon"], rahu[0]) < 0.5 * ARCSEC
    assert _angle_diff(p["bodies"]["Ketu"]["lon"], rahu[0] + 180.0) < 0.5 * ARCSEC
    assert p["bodies"]["Rahu"]["retro"] is True
    assert abs(p["ayanamsa_deg"] - swe.get_ayanamsa_ut(jd)) < ARCSEC

    # ascendant vs swe.houses_ex sidereal (tolerance from CONTRACT: 0.01 deg)
    _cusps, ascmc = swe.houses_ex(
        jd, golden["lat"], golden["lon"], b"W", swe.FLG_SIDEREAL
    )
    assert _angle_diff(p["ascendant"], ascmc[0]) < 0.01


def test_positions_ayanamsa_conventions_and_time_scale(client, golden):
    """Both ayanamsa conventions are explicit and self-consistent with Swiss's own tropical
    longitudes; jd_tt is the TT swe.calc_ut evaluates at (docs/CONTRACT.md consensus categories)."""
    r = client.post("/v1/positions", json=golden)
    assert r.status_code == 200, r.text
    p = r.json()
    jd = p["jd_ut"]
    swe.set_sid_mode(swe.SIDM_LAHIRI)
    mean = swe.get_ayanamsa_ut(jd)
    _rf, true_eq = swe.get_ayanamsa_ex_ut(jd, swe.FLG_SWIEPH)
    assert abs(p["ayanamsa_mean_equinox_deg"] - mean) < 1e-9
    assert abs(p["ayanamsa_deg"] - mean) < 1e-9
    assert abs(p["ayanamsa_true_equinox_deg"] - true_eq) < 1e-9
    # true - mean == nutation in longitude (Δψ), reported in arcsec
    assert abs((true_eq - mean) * 3600.0 - p["nutation_dpsi_arcsec"]) < 1e-3, p[
        "nutation_dpsi_arcsec"
    ]
    assert 0.0 < abs(p["nutation_dpsi_arcsec"]) < 20.0  # |Δψ| < 17.2" always
    # sidereal + true-equinox ayanamsa reproduces Swiss's apparent tropical longitude
    flags = swe.FLG_SWIEPH | swe.FLG_SPEED
    for name, pid in (
        ("Sun", swe.SUN),
        ("Moon", swe.MOON),
        ("Mars", swe.MARS),
        ("Rahu", swe.TRUE_NODE),
    ):
        trop, _ = swe.calc_ut(jd, pid, flags)
        recon = p["bodies"][name]["lon"] + p["ayanamsa_true_equinox_deg"]
        assert _angle_diff(recon, trop[0]) < 1e-6 * ARCSEC, name
        # ... and the MEAN convention does NOT (it is off by exactly Δψ)
        recon_mean = p["bodies"][name]["lon"] + p["ayanamsa_deg"]
        assert (
            abs(
                _angle_diff(recon_mean, trop[0]) * 3600.0
                - abs(p["nutation_dpsi_arcsec"])
            )
            < 1e-3
        ), name
    # time scale: jd_tt = jd_ut + ΔT, ΔT within the physically plausible range
    assert abs(p["jd_tt"] - (jd + p["delta_t_sec"] / 86400.0)) < 1e-12
    assert 50.0 < p["delta_t_sec"] < 70.0  # 1990: TT-UT ≈ 57 s
    et, _ut = swe.utc_to_jd(1990, 3, 15, 6, 30, 0.0, swe.GREG_CAL)
    assert (
        abs(p["jd_tt"] - et) * 86400.0 < 1e-3
    )  # leap-second exact TT in the 1972..2033 era


def test_positions_pre_1972_treats_utc_as_ut(client, golden):
    """Swiss swe_utc_to_jd: before 1972 UTC is UT and TT = UT + ΔT(model). The service must echo
    that convention so the cross-engine consensus can align time scales explicitly."""
    r = client.post("/v1/positions", json=dict(golden, utc="1950-06-15T06:30:00Z"))
    assert r.status_code == 200, r.text
    p = r.json()
    naive = swe.julday(1950, 6, 15, 6.5, swe.GREG_CAL)
    assert abs(p["jd_ut"] - naive) * 86400.0 < 1e-3
    assert 28.0 < p["delta_t_sec"] < 30.5  # 1950: ΔT ≈ 29.1 s


def test_positions_reject_naive_and_out_of_range(client, golden):
    bad = dict(golden, utc="1990-03-15 06:30")
    r = client.post("/v1/positions", json=bad)
    assert r.status_code == 422 and r.json()["error"] == "validation_error"
    r = client.post("/v1/positions", json=dict(golden, lat=91))
    assert r.status_code == 422
    r = client.post("/v1/positions", json=dict(golden, extra=1))
    assert r.status_code == 422
    r = client.post("/v1/positions", json=dict(golden, utc="4000-01-01T00:00:00Z"))
    assert r.status_code == 422


# ------------------------------------------------------------------ dasha


def test_catalog_lists_at_least_40_systems(client):
    r = client.get("/v1/catalog/dasha")
    assert r.status_code == 200
    cat = r.json()
    ids = {s["id"] for s in cat["systems"]}
    assert len(ids) >= 40
    assert cat["supported"] + cat["unsupported"] == len(ids)
    assert {
        "graha.vimsottari",
        "raasi.narayana",
        "annual.mudda",
        "annual.patyayini",
    } <= ids
    assert all(s["status"] in ("supported", "unsupported") for s in cat["systems"])
    for s in cat["systems"]:
        assert s["id"] == f"{s['package']}.{s['module']}"


def test_vimsottari_maha_dashas_sum_to_120(client, golden):
    r = client.post(
        "/v1/dasha", json={"birth": golden, "system": "graha.vimsottari", "depth": 1}
    )
    assert r.status_code == 200, r.text
    d = r.json()
    assert d["periods"], "no periods"
    assert len(d["periods"]) == 9
    total = sum(p["duration_years"] for p in d["periods"])
    assert abs(total - 120.0) <= 0.01, total
    assert (
        d["balance_at_birth_years"] is not None and 0 < d["balance_at_birth_years"] < 18
    )
    assert d["balance_source"] == "module"
    # periods are contiguous and chronological
    for a, b in zip(d["periods"], d["periods"][1:], strict=False):
        assert a["end_utc"] == b["start_utc"]
    lords = [p["lord"] for p in d["periods"]]
    assert lords[0] == "Rahu"  # Moon in Swati -> Rahu maha dasha


def test_vimsottari_nested_depths_consistent(client, golden):
    d1 = client.post(
        "/v1/dasha", json={"birth": golden, "system": "graha.vimsottari", "depth": 1}
    ).json()
    d3 = client.post(
        "/v1/dasha", json={"birth": golden, "system": "graha.vimsottari", "depth": 3}
    ).json()
    assert d3["period_count_leaf"] == 729
    for p1, p3 in zip(d1["periods"], d3["periods"], strict=True):
        assert p1["lord"] == p3["lord"]
        assert p1["start_utc"] == p3["start_utc"]
        assert len(p3["children"]) == 9
        assert all(len(c["children"]) == 9 for c in p3["children"])
        # antardasha starts chain to the parent
        assert p3["children"][0]["start_utc"] == p3["start_utc"]
        assert p3["children"][-1]["end_utc"] == p3["end_utc"]
        # the bhukti of the same lord comes first (antardhasa_option=1)
        assert p3["children"][0]["lord"] == p3["lord"]


def test_every_supported_system_runs_on_golden_chart(client, golden):
    cat = client.get("/v1/catalog/dasha").json()["systems"]
    failures = {}
    for s in cat:
        if s["status"] != "supported" or s.get("golden_smoke") != "pass":
            continue
        r = client.post(
            "/v1/dasha", json={"birth": golden, "system": s["id"], "depth": 2}
        )
        if r.status_code != 200:
            failures[s["id"]] = r.text[:200]
            continue
        d = r.json()
        assert d["periods"], s["id"]
        for p in d["periods"]:
            assert p["start_utc"] < p["end_utc"] or p["start_utc"] == p["end_utc"], (
                s["id"],
                p,
            )
            assert p["children"], (s["id"], "depth 2 must have children")
    assert not failures, failures


def test_unsupported_and_unknown_systems_are_structured_422(client, golden):
    r = client.post(
        "/v1/dasha",
        json={"birth": golden, "system": "graha.does_not_exist", "depth": 1},
    )
    assert r.status_code == 422 and r.json()["error"] == "unknown_system"
    r = client.post("/v1/dasha", json={"birth": golden, "system": "BAD ID", "depth": 1})
    assert r.status_code == 422 and r.json()["error"] == "validation_error"
    r = client.post(
        "/v1/dasha", json={"birth": golden, "system": "graha.vimsottari", "depth": 6}
    )
    assert r.status_code == 422
    cat = client.get("/v1/catalog/dasha").json()["systems"]
    for s in cat:
        if s["status"] == "unsupported" or s.get("golden_smoke") != "pass":
            r = client.post(
                "/v1/dasha", json={"birth": golden, "system": s["id"], "depth": 1}
            )
            assert r.status_code == 422, (s["id"], r.text)
            assert r.json()["error"] in {
                "unsupported_system",
                "period_data_error",
                "unmappable_output",
                "engine_error",
            }


def test_dasha_depth_5_size_guard(client, golden):
    r = client.post(
        "/v1/dasha", json={"birth": golden, "system": "raasi.narayana", "depth": 5}
    )
    assert r.status_code == 422 and r.json()["error"] == "too_many_periods"


# ------------------------------------------------------------------ panchang


def _elong(jd: float, fl: int) -> float:
    swe.set_sid_mode(swe.SIDM_LAHIRI)
    return (
        swe.calc_ut(jd, swe.MOON, fl)[0][0] - swe.calc_ut(jd, swe.SUN, fl)[0][0]
    ) % 360.0


def _iso_to_jd(s: str) -> float:
    dt = datetime.fromisoformat(s)
    return swe.utc_to_jd(
        dt.year,
        dt.month,
        dt.day,
        dt.hour,
        dt.minute,
        dt.second + dt.microsecond / 1e6,
        swe.GREG_CAL,
    )[1]


def test_panchang_golden_day(client):
    r = client.post(
        "/v1/panchang",
        json={
            "date": "1990-03-15",
            "lat": 28.6139,
            "lon": 77.2090,
            "tz_offset_hours": 5.5,
        },
    )
    assert r.status_code == 200, r.text
    p = r.json()
    assert p["vaara"] == "Thursday"
    assert (
        p["sunrise_utc"] < p["reference_utc"] <= p["sunrise_utc"][:19] + ".999Z"
        or p["reference_utc"] == p["sunrise_utc"]
    )
    assert p["sunrise_utc"] < p["sunset_utc"] < p["next_sunrise_utc"]
    assert p["nakshatra"]["name"] == "Swati"
    for key in ("tithi", "nakshatra", "yoga", "karana"):
        el = p[key]
        assert el["start_utc"] <= p["reference_utc"] <= el["end_utc"], (key, el)
    # tithi boundaries are true elongation crossings under the longitude flags the service reports
    # (independent pyswisseph check, < 2 s of Moon-Sun motion)
    t = p["tithi"]["index"]
    fl = p["longitude_swe_flags"]
    for iso, target in (
        (p["tithi"]["start_utc"], (t - 1) * 12.0),
        (p["tithi"]["end_utc"], t * 12.0),
    ):
        assert (
            _angle_diff(_elong(_iso_to_jd(iso), fl), target) < 2.0 / 86400.0 * 13.0
        ), (iso, target)
    # and with Swiss default apparent flags the same instants are within ~1 minute (aberration)
    fl_app = swe.FLG_SWIEPH | swe.FLG_SIDEREAL
    assert (
        _angle_diff(_elong(_iso_to_jd(p["tithi"]["end_utc"]), fl_app), t * 12.0) < 0.01
    )
    if p["next_tithi"]:
        assert p["next_tithi"]["start_utc"] == p["tithi"]["end_utc"]
        assert p["next_tithi"]["index"] == t % 30 + 1


def test_polar_latitude_fails_closed_but_positions_work(client):
    r = client.post(
        "/v1/panchang",
        json={"date": "2026-06-21", "lat": 80.0, "lon": 20.0, "tz_offset_hours": 1.0},
    )
    assert r.status_code == 422 and "polar" in r.json()["detail"]
    r = client.post(
        "/v1/positions",
        json={
            "utc": "2026-06-21T12:00:00Z",
            "lat": 80.0,
            "lon": 20.0,
            "tz_offset_hours": 1.0,
        },
    )
    assert r.status_code == 200, r.text
    _c, ascmc = swe.houses_ex(r.json()["jd_ut"], 80.0, 20.0, b"W", swe.FLG_SIDEREAL)
    assert _angle_diff(r.json()["ascendant"], ascmc[0]) < 0.01


def test_panchang_high_latitude_and_bad_date(client):
    r = client.post(
        "/v1/panchang",
        json={"date": "2026-06-21", "lat": 60.17, "lon": 24.94, "tz_offset_hours": 3.0},
    )
    assert r.status_code == 200, r.text
    r = client.post(
        "/v1/panchang",
        json={"date": "2026-13-01", "lat": 0, "lon": 0, "tz_offset_hours": 0},
    )
    assert r.status_code == 422
    r = client.post(
        "/v1/panchang",
        json={"date": "2026-02-30", "lat": 0, "lon": 0, "tz_offset_hours": 0},
    )
    assert r.status_code in (
        200,
        422,
    )  # swe.julday normalises; either a normalised answer or refusal is honest
    if r.status_code == 200:
        assert r.json()["date"] == "2026-02-30"


# ------------------------------------------------------------------ kuta


def test_kuta_scores(client, golden):
    b = {
        "utc": "1992-08-01T12:00:00Z",
        "lat": 13.08,
        "lon": 80.27,
        "tz_offset_hours": 5.5,
    }
    r = client.post("/v1/kuta", json={"a": golden, "b": b})
    assert r.status_code == 200, r.text
    k = r.json()
    assert set(k["ashtakoota"]) == {
        "varna",
        "vasya",
        "gana",
        "tara",
        "yoni",
        "graha_maitri",
        "bhakoot",
        "nadi",
    }
    assert k["total_max"] == 36.0
    assert abs(sum(v["score"] for v in k["ashtakoota"].values()) - k["total"]) < 1e-9
    assert sum(v["max"] for v in k["ashtakoota"].values()) == 36.0
    for v in k["ashtakoota"].values():
        assert 0.0 <= v["score"] <= v["max"]
    assert 0 <= k["south_indian_total"] <= 10
    assert k["a"]["nakshatra"] == "Swati"
    # same chart with itself: nadi must be 0 (same nadi) and bhakoot full
    r = client.post("/v1/kuta", json={"a": golden, "b": golden})
    assert r.status_code == 200
    same = r.json()
    assert same["ashtakoota"]["nadi"]["score"] == 0.0
    assert same["ashtakoota"]["bhakoot"]["score"] == 7.0

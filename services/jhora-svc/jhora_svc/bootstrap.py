"""Import-time patching of the vendored PyJHora tree (vendor/ is never modified).

Everything the CONTRACT/BLUEPRINT requires of PyJHora is enforced here, in the
importing process, *before* `jhora.panchanga.drik` is imported:

  * `const._DEFAULT_AYANAMSA_MODE = 'LAHIRI'`  (shipped default is TRUE_PUSHYA)
  * `const._use_true_nodes_for_rahu_ketu = True` (already canonical; re-asserted)
  * `const.dhasa_year_duration_default = MEAN_SIDEREAL_YEAR` (the configuration PyJHora's own
    test-suite validates; the shipped TRUE_SIDEREAL_YEAR path breaks monotonicity at depth >= 4)
  * no runtime network: `geocoder`, `Nominatim`, `requests`, `urllib` and every
    geocoding / IP-lookup / elevation helper in `jhora.utils` and `jhora.place_db`
    are replaced with callables that raise RuntimeError("network geocoding disabled")
  * PyJHora's runtime-settings loader is frozen so `data/user_settings.json`
    (a mutable file the PyJHora UI writes) can never re-point const at boot or later
  * PyJHora's package `__init__` files append cwd-relative parents to `sys.path`
    and print about it; those additions are pruned again and the prints silenced.

Call `bootstrap()`; it is idempotent. `self_check()` proves the effective state.
"""

from __future__ import annotations

import contextlib
import io
import os
import sys
from typing import Any

AYANAMSA = "LAHIRI"
NETWORK_DISABLED_MSG = "network geocoding disabled"

_bootstrapped = False
_state: dict[str, Any] = {}


class GeocodingDisabledError(RuntimeError):
    """Raised by every neutralized PyJHora network path."""


def _disabled(*_args, **_kwargs):
    raise GeocodingDisabledError(NETWORK_DISABLED_MSG)


class _DisabledModule:
    """Stand-in for a network module: any attribute access yields a raising callable."""

    def __init__(self, name: str):
        self._name = name

    def __getattr__(self, item):
        if item.startswith("__"):
            raise AttributeError(item)
        # nested access (urllib.request.urlopen) stays disabled all the way down
        return _DisabledModule(f"{self._name}.{item}")

    def __call__(self, *a, **k):
        _disabled()

    def __repr__(self):
        return f"<{self._name} disabled: {NETWORK_DISABLED_MSG}>"


_UTILS_NETWORK_FUNCS = (
    "_get_place_from_ipinfo",
    "_get_place_from_user_ip_address_cached",
    "get_place_from_user_ip_address",
    "get_elevation",
    "scrap_google_map_for_latlongtz_from_city_with_country",
    "_scrap_google_map_for_latlongtz_from_city_with_country",
    "get_location_using_nominatim",
    "get_place_from_latitude_longitude",
    # local place DB lookups are also refused: callers must pass lat/lon/tz (CONTRACT)
    "get_location_record",
    "get_location",
    "get_place",
)
_PLACE_DB_NETWORK_FUNCS = ("_fallback_get_place_from_user_ip_address",)


def bootstrap() -> dict[str, Any]:
    """Apply all patches exactly once per process and return a state summary."""
    global _bootstrapped
    if _bootstrapped:
        return dict(_state)
    if "jhora.panchanga.drik" in sys.modules:
        raise RuntimeError(
            "jhora.panchanga.drik was imported before jhora_svc.bootstrap()"
        )

    path_before = list(sys.path)
    silenced = io.StringIO()
    with contextlib.redirect_stdout(silenced):
        import jhora.config as _cfg  # importing jhora.* runs jhora/__init__ (it does not apply user settings)
        from jhora import const

        # Freeze PyJHora's settings loader: nothing may re-apply data/user_settings.json.
        _cfg._SETTINGS_LOADED = True

        const._DEFAULT_AYANAMSA_MODE = AYANAMSA
        const.set_node_mode(True)
        const.get_place_elevation_from_internet = False
        const.use_internet_for_location_check = False
        # Dasha year length: MEAN sidereal year (365.256364 d, "From JHora" in const.py). PyJHora's
        # shipped default TRUE_SIDEREAL_YEAR produced non-monotonic sub-periods at depth >= 4 on the
        # golden chart (observed 2026-09-16) and PyJHora's own pvr_tests force MEAN_SIDEREAL_YEAR
        # ("all dhasa are tested only for mean_sidereal as year duration").
        const.dhasa_year_duration_default = const.DHASA_YEAR_DURATION.MEAN_SIDEREAL_YEAR

        from jhora import utils

        utils.geocoder = _DisabledModule("geocoder")
        utils.requests = _DisabledModule("requests")
        utils.Nominatim = _disabled
        for name in _UTILS_NETWORK_FUNCS:
            if not hasattr(utils, name):
                raise RuntimeError(
                    f"jhora.utils.{name} missing: vendored PyJHora layout changed"
                )
            setattr(utils, name, _disabled)

        from jhora import place_db

        place_db.urllib = _DisabledModule("urllib")
        for name in _PLACE_DB_NETWORK_FUNCS:
            if hasattr(place_db, name):
                setattr(place_db, name, _disabled)

        from jhora.panchanga import drik

        # drik already called set_ayanamsa_mode() at import using the patched const; re-assert.
        drik.set_ayanamsa_mode(AYANAMSA)
        drik.set_planet_list(
            set_rahu_ketu_as_true_nodes=True, include_western_planets=False
        )

    # Prune sys.path entries PyJHora's package __init__ files appended (cwd-relative parents).
    for p in list(sys.path):
        if p not in path_before:
            sys.path.remove(p)
    # Sub-packages imported later (horoscope.*, tests) append again; wrap import in prune too.
    _import_subpackages_quietly()

    _state.update(
        ayanamsa=const._DEFAULT_AYANAMSA_MODE,
        true_nodes=bool(const._use_true_nodes_for_rahu_ketu),
        ephe_path=const._ephe_path,
        planet_flags=int(drik.PLANET_FLAGS),
        dhasa_year_duration=const.dhasa_year_duration_default.name,
        settings_frozen=bool(_cfg._SETTINGS_LOADED),
        swallowed_import_output=silenced.getvalue().strip(),
    )
    _bootstrapped = True
    return dict(_state)


def _import_subpackages_quietly() -> None:
    """Import every PyJHora sub-package the service uses, silencing its prints and path edits."""
    path_before = list(sys.path)
    with contextlib.redirect_stdout(io.StringIO()):
        import importlib

        for mod in (
            "jhora.horoscope",
            "jhora.horoscope.chart",
            "jhora.horoscope.dhasa",
            "jhora.horoscope.dhasa.annual",
            "jhora.horoscope.dhasa.graha",
            "jhora.horoscope.dhasa.raasi",
            "jhora.horoscope.match",
            "jhora.horoscope.match.compatibility",
        ):
            importlib.import_module(mod)
    for p in list(sys.path):
        if p not in path_before:
            sys.path.remove(p)


def ephe_file_count() -> int:
    from jhora import const

    try:
        return sum(1 for f in os.listdir(const._ephe_path) if f.endswith(".se1"))
    except OSError:
        return 0


def self_check(golden_jd_ut: float, max_arcsec: float = 1.0) -> dict[str, Any]:
    """Prove that drik really runs Lahiri + true nodes in *this* process.

    Compares drik.get_ayanamsa_value (whatever sid mode drik left active) with a fresh
    pyswisseph SIDM_LAHIRI value for the golden JD; must agree within `max_arcsec`.
    Raises RuntimeError on any mismatch so the caller fails closed.
    """
    bootstrap()
    import swisseph as swe
    from jhora import const
    from jhora.panchanga import drik

    if const._DEFAULT_AYANAMSA_MODE != AYANAMSA:
        raise RuntimeError(
            f"const._DEFAULT_AYANAMSA_MODE is {const._DEFAULT_AYANAMSA_MODE!r}, expected {AYANAMSA!r}"
        )
    if not const._use_true_nodes_for_rahu_ketu or const._RAHU != swe.TRUE_NODE:
        raise RuntimeError("PyJHora is not using true nodes for Rahu/Ketu")

    drik_ayan = float(drik.get_ayanamsa_value(golden_jd_ut))
    swe.set_sid_mode(swe.SIDM_LAHIRI)
    direct_ayan = float(swe.get_ayanamsa_ut(golden_jd_ut))
    # restore whatever drik expects (LAHIRI) - identical, but keep the call explicit
    drik.set_ayanamsa_mode(AYANAMSA)
    delta_arcsec = abs(drik_ayan - direct_ayan) * 3600.0
    if delta_arcsec > max_arcsec:
        raise RuntimeError(
            f"ayanamsa self-check failed: drik={drik_ayan:.9f} lahiri_direct={direct_ayan:.9f} "
            f"delta={delta_arcsec:.4f} arcsec > {max_arcsec}"
        )

    # Network paths must raise.
    from jhora import utils

    for name in (
        "get_location_using_nominatim",
        "get_place_from_user_ip_address",
        "get_elevation",
        "get_location",
    ):
        try:
            getattr(utils, name)("Chennai, IN")
        except GeocodingDisabledError:
            pass
        else:
            raise RuntimeError(
                f"jhora.utils.{name} did not raise: network path not neutralized"
            )

    if ephe_file_count() == 0:
        raise RuntimeError(f"no Swiss ephemeris .se1 files under {const._ephe_path}")

    return {
        "ayanamsa": AYANAMSA,
        "ayanamsa_deg_drik": drik_ayan,
        "ayanamsa_deg_swe_lahiri": direct_ayan,
        "ayanamsa_delta_arcsec": delta_arcsec,
        "true_nodes": True,
        "ephe_files": ephe_file_count(),
        "planet_flags_pyjhora": int(drik.PLANET_FLAGS),
        "dhasa_year_duration": const.dhasa_year_duration_default.name,
        "dhasa_year_days": float(const.sidereal_year),
        "settings_loader_frozen": True,
        "pyswisseph": str(swe.version),
    }

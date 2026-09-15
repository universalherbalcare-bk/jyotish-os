"""Dasha system catalog: one system id per module under jhora/horoscope/dhasa/{graha,raasi,annual}.

Entry-function discovery (read from the modules, not guessed):
  * default convention: `get_dhasa_bhukthi(...)` or `get_dhasa_antardhasa(...)`
  * explicit overrides for the modules that name their entry differently (see _ENTRY_OVERRIDES)
  * argument conventions: (jd, place, ...) with PyJHora *local* jd, or (dob, tob, place, ...)
    with dob = drik.Date(y, m, d) and tob = (h, m, s); annual.mudda takes (jd, place, years)
  * depth is always the keyword `dhasa_level_index` (const.MAHA_DHASA_DEPTH: 1=Maha .. 6=Deha)

Row normalisation (observed formats on PyJHora 5.0):
  * `[lords, (Y, M, D, local_fh), duration]` where lords is a tuple/list of length == depth,
    or a bare lord at depth 1 (raasi.lagnamsaka, raasi.narayana)
  * `(lord, 'YYYY-MM-DD HH:MM:SS AM', duration)` / `(l1, l2, 'str', duration)` (raasi.moola, raasi.nirayana)
  * graha.karaka lords are ('atma_karaka', planet) pairs
  * graha.vimsottari / graha.yoga_vimsottari return ((y, m, d) balance, rows); a few others return
    (int_type, rows) whose first element is *not* a balance
  * durations are years except annual.mudda (days); the unit is inferred from consecutive starts

Anything that does not fit is reported as status "unsupported" with the reason; never guessed.
"""

from __future__ import annotations

import contextlib
import importlib
import inspect
import io
import os
import pkgutil
from dataclasses import dataclass, field
from typing import Any

from jhora_svc import bootstrap as _bootstrap
from jhora_svc import timeconv
from jhora_svc.names import NAKSHATRAS, RASIS, planet_name

PACKAGES = ("graha", "raasi", "annual")
SKIP_MODULES = {"__init__", "applicability"}

# module id -> (entry function name, lord kind)
_ENTRY_OVERRIDES: dict[str, str] = {
    "graha.aayu": "get_dhasa_antardhasa",
    "graha.ashtaka_varga": "get_ashtaka_varga_dhasa_bhukthi",
    "graha.rashmi": "get_rashmi_dhasa_bhukthi",
    "raasi.drig": "drig_dhasa_bhukthi",
    "raasi.karaka_kendraadhi": "karaka_kendradhi_rasi_dhasa",
    "raasi.kendradhi_rasi": "kendradhi_rasi_dhasa",
    "raasi.lagna_kendraadhi": "lagna_kendradhi_rasi_dhasa",
    "raasi.moola": "moola_dhasa",
    "raasi.narayana": "narayana_dhasa_for_rasi_chart",
    "raasi.nirayana": "nirayana_shoola_dhasa_bhukthi",
    "annual.mudda": "mudda_dhasa_bhukthi",
    "annual.patyayini": "get_dhasa_bhukthi",
}
_LORD_KIND_OVERRIDES: dict[str, str] = {
    "graha.karaka": "karaka",
    "graha.saptharishi_nakshathra": "nakshatra",
}
_DEFAULT_ENTRY_NAMES = ("get_dhasa_bhukthi", "get_dhasa_antardhasa")
MAX_DEPTH = 5


@dataclass
class SystemSpec:
    id: str
    package: str
    module: str
    entry_function: str | None
    lord_kind: str
    arg_style: str  # 'jd' | 'dob' | 'mudda' | 'jd_years' | 'unknown'
    supports_depth_kw: bool
    status: str = "supported"
    reason: str | None = None
    max_depth: int = MAX_DEPTH
    extra: dict[str, Any] = field(default_factory=dict)

    def catalog_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "package": self.package,
            "module": self.module,
            "entry_function": self.entry_function,
            "lord_kind": self.lord_kind,
            "status": self.status,
            "max_depth": self.max_depth if self.status == "supported" else 0,
            "reason": self.reason,
            **self.extra,
        }


_SPECS: dict[str, SystemSpec] | None = None


class MappingError(ValueError):
    """The module's output shape is not one this adapter understands (=> unsupported)."""


class PeriodDataError(ValueError):
    """The module's output is well-formed but physically invalid for this chart (=> 422)."""


class TooManyPeriodsError(ValueError):
    """The requested depth would produce more leaf periods than MAX_LEAF_PERIODS (=> 422)."""


MAX_LEAF_PERIODS = int(os.environ.get("JHORA_SVC_MAX_LEAF_PERIODS", "120000"))


def _lord_kind_for(package: str, module_id: str) -> str:
    if module_id in _LORD_KIND_OVERRIDES:
        return _LORD_KIND_OVERRIDES[module_id]
    return "planet" if package in ("graha", "annual") else "rasi"


def _discover_module(package: str, name: str) -> SystemSpec:
    module_id = f"{package}.{name}"
    lord_kind = _lord_kind_for(package, module_id)
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            mod = importlib.import_module(f"jhora.horoscope.dhasa.{package}.{name}")
    except Exception as exc:  # noqa: BLE001 - third-party import; any failure => unsupported
        return SystemSpec(
            module_id,
            package,
            name,
            None,
            lord_kind,
            "unknown",
            False,
            status="unsupported",
            reason=f"import failed: {type(exc).__name__}: {exc}",
        )

    entry = _ENTRY_OVERRIDES.get(module_id)
    if entry is None:
        for cand in _DEFAULT_ENTRY_NAMES:
            if callable(getattr(mod, cand, None)):
                entry = cand
                break
    fn = getattr(mod, entry, None) if entry else None
    if not callable(fn):
        return SystemSpec(
            module_id,
            package,
            name,
            None,
            lord_kind,
            "unknown",
            False,
            status="unsupported",
            reason="no recognised entry function (get_dhasa_bhukthi / get_dhasa_antardhasa / override)",
        )

    params = list(inspect.signature(fn).parameters)
    supports_depth = "dhasa_level_index" in params
    if module_id == "annual.mudda":
        style = "mudda"
    elif params[:3] == ["dob", "tob", "place"]:
        style = "dob"
    elif params[:2] == ["jd", "place"] or params[:2] == ["jd_years", "place"]:
        style = "jd"
    else:
        return SystemSpec(
            module_id,
            package,
            name,
            entry,
            lord_kind,
            "unknown",
            supports_depth,
            status="unsupported",
            reason=f"unrecognised signature {params[:3]}",
        )
    if not supports_depth:
        return SystemSpec(
            module_id,
            package,
            name,
            entry,
            lord_kind,
            style,
            False,
            status="unsupported",
            reason="entry function has no dhasa_level_index parameter",
        )
    return SystemSpec(module_id, package, name, entry, lord_kind, style, True)


def specs() -> dict[str, SystemSpec]:
    """Enumerate every module once per process (order: package, then module name)."""
    global _SPECS
    if _SPECS is not None:
        return _SPECS
    _bootstrap.bootstrap()
    out: dict[str, SystemSpec] = {}
    for package in PACKAGES:
        pkg = importlib.import_module(f"jhora.horoscope.dhasa.{package}")
        for m in sorted(pkgutil.iter_modules(pkg.__path__), key=lambda x: x.name):
            if m.name in SKIP_MODULES or m.ispkg:
                continue
            spec = _discover_module(package, m.name)
            out[spec.id] = spec
    _SPECS = out
    return out


# ----------------------------------------------------------------------------- normalisation


def _lord_label(kind: str, raw: Any) -> str:
    if kind == "karaka":
        if isinstance(raw, tuple | list) and len(raw) == 2:
            return f"{raw[0]}:{planet_name(raw[1])}"
        return str(raw)
    if kind == "rasi":
        return (
            RASIS[int(raw)]
            if isinstance(raw, int | float) and 0 <= int(raw) < 12
            else str(raw)
        )
    if kind == "nakshatra":
        return (
            NAKSHATRAS[int(raw)]
            if isinstance(raw, int | float) and 0 <= int(raw) < 27
            else str(raw)
        )
    return planet_name(raw)


def _jsonable_lord_id(raw: Any) -> Any:
    if isinstance(raw, tuple | list):
        return [_jsonable_lord_id(x) for x in raw]
    if hasattr(raw, "item"):  # numpy scalar
        return raw.item()
    return raw


def _split_row(
    row: Any, depth: int
) -> tuple[tuple[Any, ...], tuple[int, int, int, float], float]:
    """-> (lords tuple, (Y,M,D,fh local), duration). Raises ValueError for unknown shapes."""
    if not isinstance(row, tuple | list) or len(row) < 3:
        raise MappingError(f"row shape not understood: {row!r}")
    # shape A: [lords, start, duration]
    if len(row) == 3:
        lords, start, dur = row
        if isinstance(lords, tuple | list) and not (
            len(lords) == 2
            and isinstance(lords[0], str)
            and not isinstance(lords[1], tuple | list)
            and depth == 1
        ):
            lords_t = tuple(lords)
        else:
            lords_t = (lords,)
    else:
        # shape B: (l1, l2, ..., start, duration) flat lords (raasi.moola / raasi.nirayana)
        *lords_list, start, dur = row
        lords_t = tuple(lords_list)
    if isinstance(start, str):
        start_t = timeconv.parse_pyjhora_datetime_string(start)
    elif isinstance(start, tuple | list) and len(start) == 4:
        start_t = (int(start[0]), int(start[1]), int(start[2]), float(start[3]))
    else:
        raise MappingError(f"start instant not understood: {start!r}")
    if len(lords_t) != depth:
        raise MappingError(
            f"row has {len(lords_t)} lords, expected depth {depth}: {row!r}"
        )
    return lords_t, start_t, float(dur)


def _infer_duration_unit(
    starts_jd: list[float], durations: list[float], year_days: float
) -> str:
    """'years' or 'days', decided by which interpretation matches consecutive starts best."""
    votes_years = votes_days = 0
    for i in range(len(starts_jd) - 1):
        delta = starts_jd[i + 1] - starts_jd[i]
        if delta <= 0 or durations[i] <= 0:
            continue
        err_y = abs(delta - durations[i] * year_days) / delta
        err_d = abs(delta - durations[i]) / delta
        if err_y < err_d:
            votes_years += 1
        else:
            votes_days += 1
    if votes_years == 0 and votes_days == 0:
        return "years"
    return "years" if votes_years >= votes_days else "days"


def build_tree(
    rows: list[Any],
    depth: int,
    tz_offset_hours: float,
    lord_kind: str,
    year_days: float,
) -> tuple[list[dict[str, Any]], str]:
    """Flat deepest-level rows -> nested periods with UTC ISO instants. Returns (tree, duration_unit)."""
    parsed = [_split_row(r, depth) for r in rows]
    starts_jd = [
        timeconv.local_ymd_hours_to_jd_ut(*st, tz_offset_hours) for _, st, _ in parsed
    ]
    durations = [d for _, _, d in parsed]
    unit = _infer_duration_unit(starts_jd, durations, year_days)
    unit_days = year_days if unit == "years" else 1.0

    leaves: list[dict[str, Any]] = []
    for i, (lords, _st, dur) in enumerate(parsed):
        start = starts_jd[i]
        end = starts_jd[i + 1] if i + 1 < len(parsed) else start + dur * unit_days
        if dur < 0 or end < start:
            raise PeriodDataError(
                f"PyJHora returned a negative/non-monotonic period at row {i} "
                f"(duration={dur}, start_jd={start:.5f}, end_jd={end:.5f}) for this chart"
            )
        leaves.append({"lords": lords, "start_jd": start, "end_jd": end})

    def group(items: list[dict[str, Any]], level: int) -> list[dict[str, Any]]:
        out: list[dict[str, Any]] = []
        i = 0
        while i < len(items):
            key = items[i]["lords"][level]
            j = i
            while j < len(items) and items[j]["lords"][level] == key:
                j += 1
            run = items[i:j]
            node = {
                "lord": _lord_label(lord_kind, key),
                "lord_id": _jsonable_lord_id(key),
                "start_utc": timeconv.jd_ut_to_iso(run[0]["start_jd"]),
                "end_utc": timeconv.jd_ut_to_iso(run[-1]["end_jd"]),
                "duration_years": (run[-1]["end_jd"] - run[0]["start_jd"]) / year_days,
                "children": group(run, level + 1) if level + 1 < depth else [],
                "_start_jd": run[0]["start_jd"],
                "_end_jd": run[-1]["end_jd"],
            }
            out.append(node)
            i = j
        return out

    tree = group(leaves, 0)
    return tree, unit


def _strip_private(nodes: list[dict[str, Any]]) -> None:
    for n in nodes:
        n.pop("_start_jd", None)
        n.pop("_end_jd", None)
        _strip_private(n["children"])


# ----------------------------------------------------------------------------- invocation


def _call_entry(
    spec: SystemSpec, jd_ut: float, lat: float, lon: float, tz: float, depth: int
) -> Any:
    from jhora.panchanga import drik

    mod = importlib.import_module(f"jhora.horoscope.dhasa.{spec.package}.{spec.module}")
    fn = getattr(mod, spec.entry_function)
    place = drik.Place("svc", lat, lon, tz)
    jd_local = timeconv.jd_ut_to_local_jd(jd_ut, tz)
    kwargs: dict[str, Any] = {"dhasa_level_index": depth}
    if spec.arg_style == "dob":
        y, m, d, fh = timeconv.local_jd_to_ymd_hours(jd_local)
        args: tuple[Any, ...] = (drik.Date(y, m, d), timeconv.hours_to_hms(fh), place)
    elif spec.arg_style == "mudda":
        args = (jd_local, place, 1)
    else:
        args = (jd_local, place)
    with contextlib.redirect_stdout(io.StringIO()):
        return fn(*args, **kwargs)


def _unpack_result(result: Any) -> tuple[Any, list[Any]]:
    """-> (module_balance_or_None, rows)."""
    if (
        isinstance(result, tuple)
        and len(result) == 2
        and isinstance(result[1], list)
        and not isinstance(result[0], list | tuple)
        and not (isinstance(result[0], tuple) and len(result[0]) == 3)
    ):
        # (int, rows) - first element is a type index, not a balance
        return None, result[1]
    if (
        isinstance(result, tuple)
        and len(result) == 2
        and isinstance(result[1], list)
        and isinstance(result[0], tuple | list)
        and len(result[0]) == 3
        and all(isinstance(x, int | float) for x in result[0])
    ):
        return tuple(result[0]), result[1]
    if isinstance(result, list):
        return None, result
    raise MappingError(f"result shape not understood: {type(result).__name__}")


def compute_dasha(
    system: str, jd_ut: float, lat: float, lon: float, tz: float, depth: int
) -> dict[str, Any]:
    """Worker-side entry. Raises LookupError (unknown/unsupported) or ValueError (unmappable output)."""
    spec = specs().get(system)
    if spec is None:
        raise LookupError(f"unknown dasha system {system!r}")
    if spec.status != "supported":
        raise LookupError(f"dasha system {system!r} is unsupported: {spec.reason}")
    if not (1 <= depth <= spec.max_depth):
        raise LookupError(f"depth {depth} outside 1..{spec.max_depth} for {system!r}")

    from jhora import const
    from jhora.panchanga import drik

    place = drik.Place("svc", lat, lon, tz)
    jd_local = timeconv.jd_ut_to_local_jd(jd_ut, tz)
    year_days = float(drik.dhasa_year_duration(jd=jd_local, place=place))
    if not (300.0 < year_days < 400.0):
        year_days = float(const.sidereal_year)

    result = _call_entry(spec, jd_ut, lat, lon, tz, depth)
    module_balance, rows = _unpack_result(result)
    if not rows:
        raise PeriodDataError(f"{system}: module returned no periods")
    if len(rows) > MAX_LEAF_PERIODS:
        raise TooManyPeriodsError(
            f"{system} at depth {depth} yields {len(rows)} leaf periods > limit {MAX_LEAF_PERIODS}; lower the depth"
        )
    tree, unit = build_tree(rows, depth, tz, spec.lord_kind, year_days)

    first_end = tree[0]["_end_jd"]
    derived_balance = (first_end - jd_ut) / year_days
    if module_balance is not None:
        y, m, d = module_balance
        balance = float(y) + float(m) / 12.0 + float(d) / year_days
        source = "module"
    else:
        balance = derived_balance
        source = "derived"
    _strip_private(tree)
    return {
        "system": system,
        "package": spec.package,
        "entry_function": spec.entry_function,
        "depth": depth,
        "balance_at_birth_years": balance,
        "balance_source": source,
        "balance_at_birth_module": list(module_balance)
        if module_balance is not None
        else None,
        "first_period_remaining_years": derived_balance,
        "year_length_days": year_days,
        "duration_unit_reported_by_module": unit,
        "periods": tree,
        "period_count_leaf": len(rows),
    }


def smoke_all(
    jd_ut: float, lat: float, lon: float, tz: float, depths: tuple[int, ...] = (1, 2)
) -> list[dict[str, Any]]:
    """Run every mapped system on one chart at the given depths and record the evidence.

    Executed once at service start (in a worker). A *mapping* failure (signature/shape not
    understood) demotes the system to "unsupported". A *chart-specific* failure (e.g. PyJHora
    emitting a negative duration for this particular chart) keeps the mapping "supported" but is
    recorded in `golden_smoke`; such requests return a structured 422 at call time, never guessed data.
    """
    sp = specs()
    for spec in sp.values():
        if spec.status != "supported":
            continue
        spec.extra["golden_smoke"] = "pass"
        for depth in depths:
            try:
                res = compute_dasha(spec.id, jd_ut, lat, lon, tz, depth)
                if not res["periods"]:
                    raise ValueError("empty periods")
            except Exception as exc:  # noqa: BLE001 - third-party module; classify below
                msg = f"{type(exc).__name__}: {exc}"
                if isinstance(exc, MappingError | TypeError | AttributeError):
                    spec.status = "unsupported"
                    spec.reason = f"cannot map output at depth {depth}: {msg}"
                else:
                    spec.extra["golden_smoke"] = f"fail at depth {depth}: {msg}"
                break
    return [s.catalog_dict() for s in sp.values()]

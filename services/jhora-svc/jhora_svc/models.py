"""Pydantic request/response models (docs/CONTRACT.md is binding)."""

from __future__ import annotations

from datetime import datetime
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

_JD_MIN_YEAR = (
    -3000
)  # Swiss bundled se1 files cover roughly 5400 BC .. 5400 AD; keep a safe envelope
_JD_MAX_YEAR = 3000


class BirthInput(BaseModel):
    model_config = ConfigDict(extra="forbid")

    utc: datetime = Field(
        description="Authoritative instant, ISO-8601 with zone, e.g. 1990-03-15T06:30:00Z"
    )
    lat: float = Field(ge=-90.0, le=90.0)
    lon: float = Field(ge=-180.0, le=180.0)
    tz_offset_hours: float = Field(
        ge=-14.0, le=14.0, description="Only for local rendering (PyJHora place tz)"
    )

    @field_validator("utc")
    @classmethod
    def _aware_and_in_range(cls, v: datetime) -> datetime:
        if v.tzinfo is None or v.utcoffset() is None:
            raise ValueError("utc must carry a timezone designator (use 'Z')")
        if not (_JD_MIN_YEAR <= v.year <= _JD_MAX_YEAR):
            raise ValueError(
                f"utc year must be within [{_JD_MIN_YEAR}, {_JD_MAX_YEAR}]"
            )
        return v


class BodyPosition(BaseModel):
    lon: float
    speed: float
    retro: bool
    lat: float
    rasi: str
    rasi_index: int
    nakshatra: str
    nakshatra_index: int
    pada: int


class PositionsResponse(BaseModel):
    bodies: dict[str, BodyPosition]
    ascendant: float
    ascendant_rasi: str
    ayanamsa_deg: float = Field(
        description="Lahiri ayanamsa, MEAN-equinox convention (swe_get_ayanamsa_ut, no nutation)"
    )
    ayanamsa_true_equinox_deg: float = Field(
        description="Lahiri ayanamsa, TRUE-equinox convention (swe_get_ayanamsa_ex_ut FLG_SWIEPH); sidereal = tropical - this"
    )
    ayanamsa_mean_equinox_deg: float
    nutation_dpsi_arcsec: float
    jd_ut: float
    jd_tt: float = Field(
        description="jd_ut + swe.deltat_ex(jd_ut): the TT swe.calc_ut evaluates at"
    )
    delta_t_sec: float
    ayanamsa: Literal["LAHIRI"]
    true_nodes: Literal[True]
    flags: dict[str, Any]


class DashaRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    birth: BirthInput
    system: str = Field(min_length=3, max_length=64, pattern=r"^[a-z]+\.[a-z_0-9]+$")
    depth: int = Field(default=2, ge=1, le=5)


class DashaPeriod(BaseModel):
    lord: str
    lord_id: Any
    start_utc: str
    end_utc: str
    duration_years: float
    children: list[DashaPeriod] = Field(default_factory=list)


class DashaResponse(BaseModel):
    system: str
    package: str
    entry_function: str
    depth: int
    balance_at_birth_years: float | None
    balance_source: Literal["module", "derived", "none"]
    balance_at_birth_module: Any = None
    first_period_remaining_years: float
    year_length_days: float
    duration_unit_reported_by_module: Literal["years", "days"]
    periods: list[DashaPeriod]
    period_count_leaf: int


class CatalogEntry(BaseModel):
    id: str
    package: str
    module: str
    entry_function: str | None
    lord_kind: str
    status: Literal["supported", "unsupported"]
    max_depth: int
    reason: str | None = None
    golden_smoke: str | None = None


class CatalogResponse(BaseModel):
    systems: list[CatalogEntry]
    supported: int
    unsupported: int


class PanchangRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    date: str = Field(
        pattern=r"^-?\d{4}-\d{2}-\d{2}$", description="Local calendar date YYYY-MM-DD"
    )
    lat: float = Field(ge=-90.0, le=90.0)
    lon: float = Field(ge=-180.0, le=180.0)
    tz_offset_hours: float = Field(ge=-14.0, le=14.0)

    @field_validator("date")
    @classmethod
    def _valid_date(cls, v: str) -> str:
        neg = v.startswith("-")
        y, m, d = (int(x) for x in v.lstrip("-").split("-"))
        if neg:
            y = -y
        if (
            not (_JD_MIN_YEAR <= y <= _JD_MAX_YEAR)
            or not (1 <= m <= 12)
            or not (1 <= d <= 31)
        ):
            raise ValueError("date out of range")
        return v


class PanchangElement(BaseModel):
    index: int
    name: str
    start_utc: str
    end_utc: str
    extra: dict[str, Any] = Field(default_factory=dict)


class PanchangResponse(BaseModel):
    date: str
    reference_utc: str
    reference: Literal["local_sunrise"]
    sunrise_utc: str
    sunset_utc: str
    next_sunrise_utc: str
    tithi: PanchangElement
    nakshatra: PanchangElement
    yoga: PanchangElement
    karana: PanchangElement
    next_tithi: PanchangElement | None = None
    next_nakshatra: PanchangElement | None = None
    next_yoga: PanchangElement | None = None
    next_karana: PanchangElement | None = None
    vaara: str
    moon_longitude_at_reference: float
    sun_longitude_at_reference: float
    longitude_swe_flags: int
    method: str


class KutaRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    a: BirthInput = Field(description="Boy / partner A")
    b: BirthInput = Field(description="Girl / partner B")


class KutaScore(BaseModel):
    score: float
    max: float


class KutaResponse(BaseModel):
    a: dict[str, Any]
    b: dict[str, Any]
    ashtakoota: dict[str, KutaScore]
    total: float
    total_max: float
    dosha_checks: dict[str, bool]
    south_indian_poruthams: dict[str, bool]
    south_indian_total: int
    south_indian_max: int
    method: str


class ErrorBody(BaseModel):
    error: str
    detail: Any = None

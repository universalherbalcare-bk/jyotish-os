#!/usr/bin/env python3
"""Generate the jyotish extension's tool input schemas and prompt docs.

Single source of truth for the ten tools in docs/CONTRACT.md. Re-run after
editing; outputs are committed so IronClaw can read them without Python.

    python3 tools/gen_schemas.py            # write files
    python3 tools/gen_schemas.py --check    # exit 1 if files differ from generator
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "jyotish"
SCHEMA_DIR = ROOT / "schemas" / "jyotish"
PROMPT_DIR = ROOT / "prompts" / "jyotish"

# docs/CONTRACT.md "Shared input type" — verbatim field names.
BIRTH_INPUT = {
    "type": "object",
    "additionalProperties": False,
    "required": ["utc", "lat", "lon", "tz_offset_hours"],
    "properties": {
        "utc": {
            "type": "string",
            "format": "date-time",
            "pattern": r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$",
            "description": "Authoritative birth instant in UTC, RFC 3339 with trailing Z (e.g. 1990-03-15T06:30:00Z).",
        },
        "lat": {
            "type": "number",
            "minimum": -90,
            "maximum": 90,
            "description": "Geographic latitude in decimal degrees (north positive).",
        },
        "lon": {
            "type": "number",
            "minimum": -180,
            "maximum": 180,
            "description": "Geographic longitude in decimal degrees (east positive).",
        },
        "tz_offset_hours": {
            "type": "number",
            "minimum": -14,
            "maximum": 14,
            "description": "Local UTC offset in hours, only for local rendering (sunrise-relative panchanga). `utc` remains authoritative.",
        },
    },
}

UTC_INSTANT = {
    "type": "string",
    "format": "date-time",
    "pattern": r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$",
}

VARGAS = [
    "D1",
    "D2",
    "D3",
    "D4",
    "D7",
    "D9",
    "D10",
    "D12",
    "D16",
    "D20",
    "D24",
    "D27",
    "D30",
    "D40",
    "D45",
    "D60",
]
BODIES = [
    "Sun",
    "Moon",
    "Mercury",
    "Venus",
    "Mars",
    "Jupiter",
    "Saturn",
    "Rahu",
    "Ketu",
    "Uranus",
    "Neptune",
]
NATAL_POINTS = BODIES + ["Ascendant", "MC"]

EVIDENCE_NOTE = (
    "Every result carries `evidence` {engine, kernel, ayanamsa, consensus_status, delta_t_sigma_sec}. "
    "Quote `evidence.consensus_status` in your answer. If the tool returns an error, report it verbatim; never substitute a number."
)


def schema(
    title: str,
    properties: dict,
    required: list[str],
    description: str,
    defs: dict | None = None,
) -> dict:
    doc = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": title,
        "description": description,
        "type": "object",
        "additionalProperties": False,
        "properties": properties,
        "required": required,
    }
    if defs:
        doc["$defs"] = defs
    return doc


BIRTH_REF = {"$ref": "#/$defs/BirthInput"}
BIRTH_DEFS = {"BirthInput": BIRTH_INPUT}

TOOLS: dict[str, dict] = {
    "chart.compute": {
        "description": "Compute a sidereal (Lahiri, true nodes, JPL DE440) chart with rashi, nakshatra+pada, bhava cusps and boundary_distance_sec for every value.",
        "permission": "allow",
        "prompt": (
            "Use for any natal or divisional chart question. Pass the `birth` object exactly as the user supplied it (UTC instant + lat/lon + tz offset); "
            "never geocode or guess coordinates. Read `boundary_distance_sec` on every rashi/nakshatra/pada: when it is below 120 seconds, say so explicitly "
            "because a one-minute birth-time error would flip that value. "
            + EVIDENCE_NOTE
        ),
        "schema": schema(
            "jyotish.chart.compute input",
            {
                "birth": BIRTH_REF,
                "varga": {
                    "type": "string",
                    "enum": VARGAS,
                    "default": "D1",
                    "description": "Divisional chart to compute. D1 is the rasi chart.",
                },
            },
            ["birth"],
            "Input for chart.compute (docs/JYOTISH-OS-BLUEPRINT.md §3).",
            BIRTH_DEFS,
        ),
    },
    "panchang.day": {
        "description": "Daily panchanga for a place: tithi, nakshatra, yoga, karana with start/end instants, sunrise/sunset, hora table, rahu kala, abhijit.",
        "permission": "allow",
        "prompt": (
            "Use for 'today's panchang', tithi/nakshatra end times, hora, rahu kala or abhijit questions. Requires a calendar date plus lat/lon/tz offset. "
            "Report transition instants as returned (UTC) and convert to local time only using the supplied tz_offset_hours. "
            + EVIDENCE_NOTE
        ),
        "schema": schema(
            "jyotish.panchang.day input",
            {
                "date": {
                    "type": "string",
                    "pattern": r"^\d{4}-\d{2}-\d{2}$",
                    "description": "Civil date YYYY-MM-DD at the place.",
                },
                "lat": BIRTH_INPUT["properties"]["lat"],
                "lon": BIRTH_INPUT["properties"]["lon"],
                "tz_offset_hours": BIRTH_INPUT["properties"]["tz_offset_hours"],
            },
            ["date", "lat", "lon", "tz_offset_hours"],
            "Input for panchang.day.",
        ),
    },
    "dasha.timeline": {
        "description": "Nested dasha periods (55 systems from PyJHora; Vimshottari cross-checked against XALEN to <= 1 day) with UTC instants and balance at birth.",
        "permission": "allow",
        "prompt": (
            "Use for mahadasha/antardasha questions. `system` must be an id from catalog.list (call it first if unsure); Vimshottari is `vimshottari`. "
            "Keep `depth` at the smallest level the question needs (1 = mahadasha only). Vimshottari boundaries are consensus-checked; if consensus fails the tool errors — say so. "
            + EVIDENCE_NOTE
        ),
        "schema": schema(
            "jyotish.dasha.timeline input",
            {
                "birth": BIRTH_REF,
                "system": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64,
                    "pattern": "^[a-z0-9_]+$",
                    "description": "Dasha system id from catalog.list (e.g. vimshottari).",
                },
                "depth": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5,
                    "default": 2,
                    "description": "Nesting depth: 1 mahadasha, 2 antardasha, ... up to 5.",
                },
            },
            ["birth", "system"],
            "Input for dasha.timeline.",
            BIRTH_DEFS,
        ),
    },
    "transit.window": {
        "description": "Exact transit ingress/aspect instants over a window (XALEN bisection), Sade Sati windows, Ashtakavarga bindu at transit.",
        "permission": "allow",
        "prompt": (
            "Use for 'what transit changes this week/month' and Sade Sati questions. Provide from_utc/to_utc bounding the window (keep it <= 1 year). "
            "Instants are exact; present them with the user's tz offset. "
            + EVIDENCE_NOTE
        ),
        "schema": schema(
            "jyotish.transit.window input",
            {
                "birth": BIRTH_REF,
                "from_utc": {**UTC_INSTANT, "description": "Window start (UTC)."},
                "to_utc": {
                    **UTC_INSTANT,
                    "description": "Window end (UTC), after from_utc.",
                },
                "bodies": {
                    "type": "array",
                    "items": {"type": "string", "enum": BODIES},
                    "uniqueItems": True,
                    "minItems": 1,
                    "default": ["Saturn", "Jupiter", "Rahu", "Ketu"],
                    "description": "Transiting bodies to sweep.",
                },
                "natal_points": {
                    "type": "array",
                    "items": {"type": "string", "enum": NATAL_POINTS},
                    "uniqueItems": True,
                    "minItems": 1,
                    "default": ["Moon", "Ascendant"],
                    "description": "Natal reference points.",
                },
            },
            ["birth", "from_utc", "to_utc"],
            "Input for transit.window.",
            BIRTH_DEFS,
        ),
    },
    "muhurta.find": {
        "description": "Ranked muhurta windows for a VedAstro activity rule with the exact rule ids that passed and vetoed.",
        "permission": "allow",
        "prompt": (
            "Use for 'when should I ...' questions. `activity` is a VedAstro rule Name (e.g. GoodLunarDayForTravel); list options via catalog.list or config/activities.json. "
            "Always show the passed_rules and vetoed_by ids for each window so a human astrologer can audit it. "
            + EVIDENCE_NOTE
        ),
        "schema": schema(
            "jyotish.muhurta.find input",
            {
                "activity": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128,
                    "pattern": "^[A-Za-z0-9_]+$",
                    "description": "VedAstro EventDataList rule Name.",
                },
                "from_utc": {
                    **UTC_INSTANT,
                    "description": "Search window start (UTC).",
                },
                "to_utc": {**UTC_INSTANT, "description": "Search window end (UTC)."},
                "lat": BIRTH_INPUT["properties"]["lat"],
                "lon": BIRTH_INPUT["properties"]["lon"],
                "tz_offset_hours": BIRTH_INPUT["properties"]["tz_offset_hours"],
                "step_minutes": {
                    "type": "integer",
                    "minimum": 5,
                    "maximum": 1440,
                    "default": 60,
                    "description": "Evaluation step in minutes.",
                },
            },
            ["activity", "from_utc", "to_utc", "lat", "lon", "tz_offset_hours"],
            "Input for muhurta.find (mirrors vedastro-svc POST /v1/muhurta/find).",
        ),
    },
    "match.kuta": {
        "description": "Kuta compatibility for two births: per-kuta scores, dosha flags, PyJHora and VedAstro totals side by side.",
        "permission": "allow",
        "prompt": (
            "Use for compatibility/matching questions. Needs two complete BirthInput objects (`a`, `b`). Present both engines' totals; if they disagree, say so rather than averaging. "
            + EVIDENCE_NOTE
        ),
        "schema": schema(
            "jyotish.match.kuta input",
            {"a": BIRTH_REF, "b": BIRTH_REF},
            ["a", "b"],
            "Input for match.kuta.",
            BIRTH_DEFS,
        ),
    },
    "rectify.birth_time": {
        "description": "Heuristic birth-time rectification: sweeps candidate minutes around the stated time and ranks them by fit to dated life events.",
        "permission": "ask",
        "prompt": (
            "Use only when the user asks to rectify an uncertain birth time. Requires the nominal birth, a +/- window in minutes, and at least one dated life event. "
            "State plainly that the result is a heuristic ranking, not a determination, and quote each candidate's score. "
            + EVIDENCE_NOTE
        ),
        "schema": schema(
            "jyotish.rectify.birth_time input",
            {
                "birth": BIRTH_REF,
                "window_min": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 720,
                    "default": 30,
                    "description": "Half-width of the search window in minutes around birth.utc.",
                },
                "events": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 50,
                    "items": {
                        "type": "object",
                        "additionalProperties": False,
                        "required": ["date", "kind"],
                        "properties": {
                            "date": {
                                "type": "string",
                                "pattern": r"^\d{4}-\d{2}-\d{2}$",
                                "description": "Event date YYYY-MM-DD.",
                            },
                            "kind": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 64,
                                "description": "Event category (e.g. marriage, child_birth, career_change, relocation, bereavement).",
                            },
                            "note": {
                                "type": "string",
                                "maxLength": 280,
                                "description": "Optional short note.",
                            },
                        },
                    },
                    "description": "Dated life events used to score candidates.",
                },
            },
            ["birth", "window_min", "events"],
            "Input for rectify.birth_time.",
            BIRTH_DEFS,
        ),
    },
    "rule.validate": {
        "description": "Empirically validate a VedAstro horoscope rule against the local person/marriage datasets: hit rate vs base rate, n, 95% CI, PROMOTE or KEEP_UNPROVED.",
        "permission": "ask",
        "prompt": (
            "Use before relying on any yoga/dosha rule for a prediction. Quote n, hit_rate, base_rate, ci95 and the verdict verbatim; a KEEP_UNPROVED rule must be presented as unproved. "
            "This tool appends to the promotion log, so confirm with the user before running it in bulk. "
            + EVIDENCE_NOTE
        ),
        "schema": schema(
            "jyotish.rule.validate input",
            {
                "rule_id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 128,
                    "pattern": "^[A-Za-z0-9_.-]+$",
                    "description": "VedAstro HoroscopeDataList rule id.",
                },
                "dataset": {
                    "type": "string",
                    "enum": ["marriage", "person"],
                    "description": "Which local dataset to test against.",
                },
                "outcome_column": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64,
                    "pattern": "^[A-Za-z0-9_]+$",
                    "description": "Outcome column in the dataset.",
                },
            },
            ["rule_id", "dataset", "outcome_column"],
            "Input for rule.validate (mirrors vedastro-svc POST /v1/rule/validate).",
        ),
    },
    "engine.consensus": {
        "description": "Per-body delta in arcseconds between XALEN-DE440 and PyJHora-Swiss for a chart, with pass/fail against the contract tolerances.",
        "permission": "allow",
        "prompt": (
            "Use to audit engine agreement (nightly heartbeat runs it on the golden chart). Report every body's delta and the overall status; a FAIL means the tools are not to be trusted until fixed. "
            + EVIDENCE_NOTE
        ),
        "schema": schema(
            "jyotish.engine.consensus input",
            {"birth": BIRTH_REF},
            ["birth"],
            "Input for engine.consensus.",
            BIRTH_DEFS,
        ),
    },
    "catalog.list": {
        "description": "List exactly which ayanamsas, house systems, dasha systems and muhurta activity rules are enabled in this deployment.",
        "permission": "allow",
        "prompt": (
            "Call first whenever you need a valid `system` id for dasha.timeline or an `activity` name for muhurta.find. Takes no arguments."
        ),
        "schema": schema(
            "jyotish.catalog.list input",
            {},
            [],
            "Input for catalog.list (no parameters).",
        ),
    },
}


def render() -> dict[Path, str]:
    files: dict[Path, str] = {}
    for name, spec in TOOLS.items():
        rel = name.replace(".", "/")
        files[SCHEMA_DIR / f"{rel}.input.v1.json"] = (
            json.dumps(spec["schema"], indent=2, ensure_ascii=False) + "\n"
        )
        files[PROMPT_DIR / f"{rel}.md"] = spec["prompt"] + "\n"
    return files


def main(argv: list[str]) -> int:
    files = render()
    check = "--check" in argv
    drift = 0
    for path, content in files.items():
        if check:
            if not path.exists() or path.read_text(encoding="utf-8") != content:
                print(f"DRIFT {path}")
                drift += 1
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
            print(f"wrote {path.relative_to(ROOT.parent)}")
    if check:
        print("drift" if drift else "ok", drift)
        return 1 if drift else 0
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

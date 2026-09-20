"""JYOTISH-OS live console — one page, one birth, every engine side by side.

Plain-HTTP loopback UI (127.0.0.1:7790) that proxies to the three backends so the browser never
has to trust the self-signed MCP certificate itself:

  * jyotish-mcp  https://127.0.0.1:7791/mcp   (XALEN + DE440; consensus vs jhora-svc inside)
  * kundali-webapp http://127.0.0.1:8000/api  (the earlier "Vedic Astrology" product: Swiss + 22 engines)

Every number shown comes from a live call made when the user presses Run; nothing is cached or
pre-rendered. Deltas between engines are computed here in arcseconds and labelled with the known
convention differences (docs/CONTRACT.md § consensus). This service holds no secrets and stores
nothing.
"""

from __future__ import annotations

import datetime as dt
import json
import os
import ssl
import time
import urllib.error
import urllib.request
from pathlib import Path

from fastapi import FastAPI, HTTPException
from fastapi.responses import HTMLResponse, JSONResponse
from pydantic import BaseModel, Field

ROOT = Path(__file__).resolve().parents[2]
CA = os.environ.get("JYOTISH_CA", str(ROOT / "certs" / "ironclaw-ca-bundle.pem"))
MCP = os.environ.get("JYOTISH_MCP", "https://127.0.0.1:7791")
KUNDALI = os.environ.get("KUNDALI_URL", "http://127.0.0.1:8000")
CTX = ssl.create_default_context(cafile=CA) if os.path.exists(CA) else None
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
]
PROBE_ERRORS = (urllib.error.URLError, TimeoutError, ValueError, OSError)

app = FastAPI(title="JYOTISH-OS console", docs_url=None, redoc_url=None)


class Birth(BaseModel):
    utc: str = Field(..., description="RFC 3339 UTC instant, e.g. 1990-03-15T06:30:00Z")
    lat: float = Field(..., ge=-90, le=90)
    lon: float = Field(..., ge=-180, le=180)
    tz_offset_hours: float = Field(5.5, ge=-14, le=14)


def _post(url: str, body: dict, timeout: int = 60, ctx: ssl.SSLContext | None = None):
    req = urllib.request.Request(
        url,
        data=json.dumps(body).encode(),
        headers={"content-type": "application/json"},
    )
    t0 = time.perf_counter()
    with urllib.request.urlopen(req, timeout=timeout, context=ctx) as r:
        return json.loads(r.read().decode()), round((time.perf_counter() - t0) * 1000)


def mcp_call(name: str, args: dict, timeout: int = 60) -> dict:
    try:
        r, ms = _post(
            f"{MCP}/mcp",
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {"name": name, "arguments": args},
            },
            timeout,
            CTX,
        )
    except PROBE_ERRORS as e:
        return {"ok": False, "error": f"jyotish-mcp unreachable: {e}", "ms": None}
    if "error" in r:
        return {"ok": False, "error": r["error"], "ms": ms}
    res = r.get("result", {})
    return {
        "ok": not res.get("isError"),
        "data": res.get("structuredContent"),
        "ms": ms,
    }


def kundali_call(path: str, body: dict, timeout: int = 90) -> dict:
    try:
        r, ms = _post(f"{KUNDALI}{path}", body, timeout)
    except urllib.error.HTTPError as e:
        return {
            "ok": False,
            "error": f"kundali {e.code}: {e.read().decode()[:300]}",
            "ms": None,
        }
    except PROBE_ERRORS as e:
        return {"ok": False, "error": f"kundali-webapp unreachable: {e}", "ms": None}
    return {"ok": True, "data": r, "ms": ms}


def to_local(b: Birth) -> dict:
    t = dt.datetime.fromisoformat(b.utc).astimezone(
        dt.timezone(dt.timedelta(hours=b.tz_offset_hours))
    )
    return {
        "year": t.year,
        "month": t.month,
        "day": t.day,
        "hour": t.hour,
        "minute": t.minute,
    }


def arcsec(a: float | None, b: float | None) -> float | None:
    if a is None or b is None:
        return None
    d = (a - b + 180.0) % 360.0 - 180.0
    return round(d * 3600.0, 3)


@app.get("/", response_class=HTMLResponse)
def index() -> str:
    return (Path(__file__).parent / "index.html").read_text(encoding="utf-8")


@app.get("/api/health")
def health() -> JSONResponse:
    out = {"console": True}
    try:
        with urllib.request.urlopen(f"{MCP}/health", timeout=6, context=CTX) as r:
            h = json.loads(r.read().decode())
        out["jyotish_mcp"] = {
            "ok": h.get("ok"),
            "kernel": h.get("kernel_id"),
            "jhora_up": h.get("sidecars", {}).get("jhora", {}).get("up"),
            "vedastro_up": h.get("sidecars", {}).get("vedastro", {}).get("up"),
        }
    except PROBE_ERRORS as e:
        out["jyotish_mcp"] = {"ok": False, "error": str(e)}
    try:
        with urllib.request.urlopen(f"{KUNDALI}/", timeout=6) as r:
            out["kundali_webapp"] = {"ok": r.status == 200, "url": KUNDALI}
    except PROBE_ERRORS as e:
        out["kundali_webapp"] = {"ok": False, "error": str(e), "url": KUNDALI}
    return JSONResponse(out)


@app.post("/api/run")
def run(b: Birth) -> JSONResponse:
    birth = b.model_dump()
    t_all = time.perf_counter()
    chart = mcp_call("chart.compute", {"birth": birth})
    cons = mcp_call("engine.consensus", {"birth": birth})
    date_local = to_local(b)
    pan = mcp_call(
        "panchang.day",
        {
            "date": f"{date_local['year']:04d}-{date_local['month']:02d}-{date_local['day']:02d}",
            "lat": b.lat,
            "lon": b.lon,
            "tz_offset_hours": b.tz_offset_hours,
        },
    )
    dasha = mcp_call(
        "dasha.timeline", {"birth": birth, "system": "graha.vimsottari", "depth": 2}
    )
    kund = kundali_call(
        "/api/chart",
        {
            "name": "console",
            **date_local,
            "lat": b.lat,
            "lon": b.lon,
            "tz_offset_hours": b.tz_offset_hours,
        },
    )

    # --- tri-engine positions table (sidereal Lahiri longitudes, degrees) ---
    rows = []
    x = (chart.get("data") or {}).get("bodies", {}) if chart.get("ok") else {}
    cbodies = (cons.get("data") or {}).get("bodies", {}) if cons.get("ok") else {}
    kpos = {}
    if kund.get("ok"):
        # kundali returns {"sun": {...}, "moon": {...}} (dict keyed by lowercase body)
        raw = kund["data"].get("positions") or {}
        items = raw.values() if isinstance(raw, dict) else raw
        for p in items:
            if isinstance(p, dict):
                kpos[str(p.get("body", "")).capitalize()] = p.get("longitude")
    for name in BODIES:
        xl = (x.get(name) or {}).get("sidereal_lon_deg")
        jl = (cbodies.get(name) or {}).get("jhora_deg")
        kl = kpos.get(name)
        rows.append(
            {
                "body": name,
                "xalen_de440": xl,
                "jhora_swiss": jl,
                "kundali_swiss": kl,
                "d_xalen_jhora_arcsec": arcsec(xl, jl),
                "d_kundali_jhora_arcsec": arcsec(kl, jl),
                "nakshatra": (x.get(name) or {}).get("nakshatra"),
                "pada": (x.get(name) or {}).get("pada"),
                "pada_flip_sec": (
                    (x.get(name) or {}).get("boundary_distance_sec") or {}
                ).get("pada_sec"),
                "source": (x.get(name) or {}).get("source"),
            }
        )
    asc_x = (
        ((chart.get("data") or {}).get("ascendant") or {}).get("sidereal_lon_deg")
        if chart.get("ok")
        else None
    )
    asc_k = kund["data"].get("ascendant") if kund.get("ok") else None
    asc_j = (
        ((cons.get("data") or {}).get("ascendant") or {}).get("jhora_deg")
        if cons.get("ok")
        else None
    )

    return JSONResponse(
        {
            "birth": birth,
            "local": date_local,
            "elapsed_ms": round((time.perf_counter() - t_all) * 1000),
            "chart": chart,
            "consensus": cons,
            "panchang": pan,
            "dasha": dasha,
            "kundali": {
                "ok": kund.get("ok"),
                "ms": kund.get("ms"),
                "error": kund.get("error"),
                "ayanamsha": (kund.get("data") or {}).get("ayanamsha"),
                "provenance": (kund.get("data") or {}).get("provenance"),
                "warnings": (kund.get("data") or {}).get("warnings"),
                "panchanga": (kund.get("data") or {}).get("panchanga"),
                "yogas": (kund.get("data") or {}).get("yogas"),
                "shadbala": (kund.get("data") or {}).get("shadbala"),
                "birth_data_integrity": (kund.get("data") or {}).get(
                    "birth_data_integrity"
                ),
            },
            "positions": rows,
            "ascendant": {
                "xalen_de440": asc_x,
                "jhora_swiss": asc_j,
                "kundali_swiss": asc_k,
                "d_xalen_jhora_arcsec": arcsec(asc_x, asc_j),
                "d_kundali_jhora_arcsec": arcsec(asc_k, asc_j),
            },
            "notes": [
                "XALEN-DE440 and jhora-svc are compared at the same TT/UT1 inside engine.consensus (docs/CONTRACT.md).",
                'kundali-webapp is a separate product (Swiss Ephemeris, Lahiri mean-equinox value); a constant ~12" offset on all bodies is the true- vs mean-equinox ayanamsa convention, not an error.',
                "Deltas are shown, never averaged or hidden.",
            ],
        }
    )


@app.post("/api/tool/{name}")
def tool(name: str, args: dict) -> JSONResponse:
    if name not in {
        "chart.compute",
        "panchang.day",
        "transit.window",
        "dasha.timeline",
        "muhurta.find",
        "match.kuta",
        "rectify.birth_time",
        "rule.validate",
        "engine.consensus",
        "catalog.list",
    }:
        raise HTTPException(404, "unknown tool")
    return JSONResponse(mcp_call(name, args, timeout=120))


@app.post("/api/kundali/{path:path}")
def kundali_proxy(path: str, body: dict) -> JSONResponse:
    if not path.startswith("api/"):
        raise HTTPException(404, "only /api/* of kundali-webapp is proxied")
    return JSONResponse(kundali_call("/" + path, body))

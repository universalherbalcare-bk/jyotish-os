"""jhora-svc: PyJHora sidecar. JSON REST on 127.0.0.1:7792 (docs/CONTRACT.md).

Endpoints (exactly the contract's):
  GET  /v1/health        POST /v1/positions     POST /v1/dasha
  GET  /v1/catalog/dasha POST /v1/panchang      POST /v1/kuta

All PyJHora work runs in a spawn-based process pool (see jhora_svc/pool.py). The main process
also bootstraps PyJHora (patched, no network) but only to validate catalog ids; it never computes.
"""

from __future__ import annotations

import json
import logging
import os
import sys
import time
from contextlib import asynccontextmanager
from typing import Any

from fastapi import FastAPI, Request
from fastapi.encoders import jsonable_encoder
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from jhora_svc import bootstrap, compute, dasha_catalog, models, timeconv
from jhora_svc.pool import WorkerPool

BIND_HOST = "127.0.0.1"
BIND_PORT = 7792
LOOPBACK = {"127.0.0.1", "::1", "testclient"}

log = logging.getLogger("jhora_svc")


class _JsonFormatter(logging.Formatter):
    def format(self, record: logging.LogRecord) -> str:
        payload = {
            "ts": time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(record.created))
            + f".{int(record.msecs):03d}Z",
            "level": record.levelname,
            "logger": record.name,
            "msg": record.getMessage(),
        }
        if record.exc_info:
            payload["exc"] = self.formatException(record.exc_info)
        return json.dumps(payload)


def _configure_logging() -> None:
    handler = logging.StreamHandler(sys.stderr)
    handler.setFormatter(_JsonFormatter())
    root = logging.getLogger()
    root.handlers[:] = [handler]
    root.setLevel(os.environ.get("JHORA_SVC_LOG_LEVEL", "INFO").upper())


class ServiceError(Exception):
    def __init__(self, status: int, error: str, detail: Any = None):
        super().__init__(error)
        self.status = status
        self.error = error
        self.detail = detail


@asynccontextmanager
async def lifespan(app: FastAPI):
    _configure_logging()
    t0 = time.monotonic()
    bootstrap.bootstrap()  # main process: patched import only, no computation
    pool = WorkerPool()
    pool.start()
    # Fail closed: every worker must pass the self-check + golden warm-up (initializer), and one
    # worker proves it again here, returning the evidence stored for /v1/health.
    health = pool.run_sync(compute.worker_health, timeout=120)
    golden = pool.run_sync(compute.golden_warmup, timeout=120)
    catalog = pool.run_sync(
        dasha_catalog.smoke_all,
        compute.GOLDEN_JD_UT,
        compute.GOLDEN["lat"],
        compute.GOLDEN["lon"],
        compute.GOLDEN["tz_offset_hours"],
        (1, 2, 3),
        timeout=300,
    )
    app.state.pool = pool
    app.state.self_check = health
    app.state.golden = golden
    app.state.catalog = catalog
    app.state.catalog_index = {c["id"]: c for c in catalog}
    app.state.started_at = time.time()
    log.info(
        "jhora-svc ready: workers=%d ayanamsa=%s true_nodes=%s ephe_files=%d catalog=%d/%d supported boot=%.1fs",
        pool.workers,
        health["ayanamsa"],
        health["true_nodes"],
        health["ephe_files"],
        sum(c["status"] == "supported" for c in catalog),
        len(catalog),
        time.monotonic() - t0,
    )
    try:
        yield
    finally:
        pool.shutdown()
        log.info("jhora-svc stopped")


app = FastAPI(
    title="jhora-svc", version="0.1.0", lifespan=lifespan, docs_url=None, redoc_url=None
)


@app.middleware("http")
async def loopback_only(request: Request, call_next):
    host = request.client.host if request.client else None
    if host not in LOOPBACK:
        return JSONResponse(
            status_code=403, content={"error": "loopback_only", "detail": host}
        )
    started = time.monotonic()
    response = await call_next(request)
    log.info(
        "%s %s -> %d %.1fms",
        request.method,
        request.url.path,
        response.status_code,
        (time.monotonic() - started) * 1000.0,
    )
    return response


@app.exception_handler(ServiceError)
async def _service_error(_request: Request, exc: ServiceError):
    return JSONResponse(
        status_code=exc.status, content={"error": exc.error, "detail": exc.detail}
    )


@app.exception_handler(RequestValidationError)
async def _validation_error(_request: Request, exc: RequestValidationError):
    return JSONResponse(
        status_code=422,
        content={"error": "validation_error", "detail": jsonable_encoder(exc.errors())},
    )


@app.exception_handler(Exception)
async def _unhandled(_request: Request, exc: Exception):
    log.exception("unhandled error")
    return JSONResponse(
        status_code=500,
        content={"error": "internal_error", "detail": type(exc).__name__},
    )


async def _run(request: Request, fn, *args, timeout: float = 120.0):
    pool: WorkerPool = request.app.state.pool
    try:
        return await pool.run(fn, *args, timeout=timeout)
    except TimeoutError as exc:
        raise ServiceError(504, "computation_timeout", {"timeout_s": timeout}) from exc
    except dasha_catalog.MappingError as exc:
        raise ServiceError(422, "unmappable_output", str(exc)) from exc
    except dasha_catalog.PeriodDataError as exc:
        raise ServiceError(422, "period_data_error", str(exc)) from exc
    except dasha_catalog.TooManyPeriodsError as exc:
        raise ServiceError(422, "too_many_periods", str(exc)) from exc
    except LookupError as exc:
        raise ServiceError(422, "unsupported_system", str(exc)) from exc
    except bootstrap.GeocodingDisabledError as exc:
        raise ServiceError(422, "network_geocoding_disabled", str(exc)) from exc
    except (ValueError, ZeroDivisionError, IndexError, KeyError) as exc:
        # PyJHora raised on this input; surface it as a client-side 422, never a fabricated result
        raise ServiceError(422, "engine_error", f"{type(exc).__name__}: {exc}") from exc


def _jd(birth: models.BirthInput) -> float:
    return timeconv.utc_datetime_to_jd_ut(birth.utc)


@app.get("/v1/health")
async def health(request: Request):
    st = request.app.state
    live = await _run(request, compute.worker_health, timeout=30.0)
    return {
        "ok": True,
        "ayanamsa": live["ayanamsa"],
        "true_nodes": live["true_nodes"],
        "pyswisseph": live["pyswisseph"],
        "ephe_files": live["ephe_files"],
        "workers": st.pool.workers,
        "self_check": live,
        "golden": st.golden,
        "catalog": {
            "total": len(st.catalog),
            "supported": sum(c["status"] == "supported" for c in st.catalog),
            "unsupported": sum(c["status"] != "supported" for c in st.catalog),
        },
        "uptime_s": round(time.time() - st.started_at, 1),
    }


@app.post("/v1/positions", response_model=models.PositionsResponse)
async def positions(birth: models.BirthInput, request: Request):
    return await _run(
        request,
        compute.positions,
        _jd(birth),
        birth.lat,
        birth.lon,
        birth.tz_offset_hours,
    )


@app.get("/v1/catalog/dasha", response_model=models.CatalogResponse)
async def catalog_dasha(request: Request):
    cat = request.app.state.catalog
    return {
        "systems": cat,
        "supported": sum(c["status"] == "supported" for c in cat),
        "unsupported": sum(c["status"] != "supported" for c in cat),
    }


@app.post("/v1/dasha", response_model=models.DashaResponse)
async def dasha(req: models.DashaRequest, request: Request):
    entry = request.app.state.catalog_index.get(req.system)
    if entry is None:
        raise ServiceError(
            422,
            "unknown_system",
            {"system": req.system, "hint": "GET /v1/catalog/dasha"},
        )
    if entry["status"] != "supported":
        raise ServiceError(
            422,
            "unsupported_system",
            {"system": req.system, "reason": entry.get("reason")},
        )
    return await _run(
        request,
        dasha_catalog.compute_dasha,
        req.system,
        _jd(req.birth),
        req.birth.lat,
        req.birth.lon,
        req.birth.tz_offset_hours,
        req.depth,
    )


@app.post("/v1/panchang", response_model=models.PanchangResponse)
async def panchang(req: models.PanchangRequest, request: Request):
    neg = req.date.startswith("-")
    y, m, d = (int(x) for x in req.date.lstrip("-").split("-"))
    if neg:
        y = -y
    return await _run(
        request, compute.panchang, y, m, d, req.lat, req.lon, req.tz_offset_hours
    )


@app.post("/v1/kuta", response_model=models.KutaResponse)
async def kuta(req: models.KutaRequest, request: Request):
    return await _run(request, compute.kuta, _jd(req.a), _jd(req.b))


if __name__ == "__main__":
    import uvicorn

    uvicorn.run("app:app", host=BIND_HOST, port=BIND_PORT, workers=1, log_config=None)

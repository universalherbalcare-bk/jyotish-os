"""Shared fixtures: one live ASGI app (real spawn process pool) per test session."""

from __future__ import annotations

import asyncio
import sys
from pathlib import Path

import httpx
import pytest

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

GOLDEN = {
    "utc": "1990-03-15T06:30:00Z",
    "lat": 28.6139,
    "lon": 77.2090,
    "tz_offset_hours": 5.5,
}


@pytest.fixture(scope="session")
def event_loop_policy():
    return asyncio.DefaultEventLoopPolicy()


@pytest.fixture(scope="session")
def app_ctx():
    """Start the FastAPI lifespan once (boots the worker pool, self-check, catalog smoke)."""
    import app as app_module

    loop = asyncio.new_event_loop()
    cm = app_module.lifespan(app_module.app)
    loop.run_until_complete(cm.__aenter__())
    try:
        yield app_module.app, loop
    finally:
        loop.run_until_complete(cm.__aexit__(None, None, None))
        loop.close()


@pytest.fixture(scope="session")
def client(app_ctx):
    app, loop = app_ctx
    transport = httpx.ASGITransport(app=app)
    ac = httpx.AsyncClient(
        transport=transport, base_url="http://testserver", timeout=180.0
    )

    class SyncClient:
        def get(self, url, **kw):
            return loop.run_until_complete(ac.get(url, **kw))

        def post(self, url, **kw):
            return loop.run_until_complete(ac.post(url, **kw))

    try:
        yield SyncClient()
    finally:
        loop.run_until_complete(ac.aclose())


@pytest.fixture(scope="session")
def golden():
    return dict(GOLDEN)

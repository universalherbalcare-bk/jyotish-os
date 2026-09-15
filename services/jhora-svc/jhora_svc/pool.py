"""Process pool for PyJHora work. Swiss Ephemeris state is process-global, so: processes, never threads.

Each worker (spawned, never forked) runs `worker_init`: bootstrap (LAHIRI + true nodes + no
network), a self-check against pyswisseph SIDM_LAHIRI, and a golden-chart warm-up. Any failure
raises inside the initializer, which breaks the pool, which makes the service refuse to start.
"""

from __future__ import annotations

import asyncio
import concurrent.futures as cf
import logging
import multiprocessing as mp
import os
from collections.abc import Callable
from typing import Any

log = logging.getLogger("jhora_svc.pool")

DEFAULT_TIMEOUT_S = 120.0


def worker_init() -> None:
    from jhora_svc import bootstrap, compute

    bootstrap.bootstrap()
    compute.worker_health()
    compute.golden_warmup()


def default_workers() -> int:
    env = os.environ.get("JHORA_SVC_WORKERS")
    if env:
        n = int(env)
        if n < 1:
            raise ValueError("JHORA_SVC_WORKERS must be >= 1")
        return n
    return max(1, min(4, os.cpu_count() or 1))


class WorkerPool:
    def __init__(self, workers: int | None = None):
        self.workers = workers or default_workers()
        self._ctx = mp.get_context("spawn")
        self._executor: cf.ProcessPoolExecutor | None = None
        self._lock = asyncio.Lock()

    def start(self) -> None:
        self._executor = cf.ProcessPoolExecutor(
            max_workers=self.workers, mp_context=self._ctx, initializer=worker_init
        )

    def shutdown(self) -> None:
        if self._executor is not None:
            self._executor.shutdown(wait=True, cancel_futures=True)
            self._executor = None

    def run_sync(
        self, fn: Callable[..., Any], *args: Any, timeout: float = DEFAULT_TIMEOUT_S
    ) -> Any:
        """Blocking submit; used at startup before the event loop matters."""
        if self._executor is None:
            raise RuntimeError("pool not started")
        return self._executor.submit(fn, *args).result(timeout=timeout)

    async def run(
        self, fn: Callable[..., Any], *args: Any, timeout: float = DEFAULT_TIMEOUT_S
    ) -> Any:
        if self._executor is None:
            raise RuntimeError("pool not started")
        loop = asyncio.get_running_loop()
        fut = loop.run_in_executor(self._executor, fn, *args)
        try:
            return await asyncio.wait_for(fut, timeout=timeout)
        except cf.process.BrokenProcessPool:
            log.error("process pool broken; rebuilding")
            async with self._lock:
                self.shutdown()
                self.start()
            raise

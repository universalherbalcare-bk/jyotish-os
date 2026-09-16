# syntax=docker/dockerfile:1.7
# jhora-svc — PyJHora sidecar, containerized (Phase 8). Build context = repo root (deploy/compose.yaml).
#
# Hardening (compose supplies the rest: read_only rootfs, tmpfs /tmp, cap_drop ALL, no-new-privileges,
# internal-only network, memory limit): non-root uid 10002, no PyQt6/GUI packages, vendored PyJHora
# source + Swiss .se1 files copied read-only (root-owned 0644, the service user cannot modify them).
#
# The service binds 0.0.0.0:7792 INSIDE its network namespace only; compose never publishes it.

FROM python:3.12-slim AS builder
ENV PIP_NO_CACHE_DIR=1 PIP_DISABLE_PIP_VERSION_CHECK=1 PYTHONDONTWRITEBYTECODE=1
# gcc only in the builder: pyswisseph / h3 / cffi fall back to sdists on platforms without wheels.
RUN apt-get update && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*
RUN python -m venv /opt/venv
ENV PATH=/opt/venv/bin:$PATH
COPY services/jhora-svc/requirements.lock /tmp/requirements.lock
# --no-deps: requirements.lock is a complete transitive freeze of the native .venv (PyQt6/GUI/test
# packages removed); nothing outside the lock may be pulled in.
RUN pip install --no-deps -r /tmp/requirements.lock && pip check

FROM python:3.12-slim AS runtime
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 PATH=/opt/venv/bin:$PATH \
    JHORA_SVC_LOG_LEVEL=INFO
RUN groupadd -g 10002 jhora && useradd -u 10002 -g 10002 -M -d /nonexistent -s /usr/sbin/nologin jhora
COPY --from=builder /opt/venv /opt/venv
# Vendored PyJHora (AGPL-3.0, isolated in this container; never linked into jyotish-mcp).
# Includes src/jhora/data/ephe (Swiss .se1 files). Root-owned, world-readable, never writable by uid 10002.
COPY --chown=root:root vendor/pyjhora/src /opt/pyjhora/src
RUN echo /opt/pyjhora/src > "$(python -c 'import sysconfig; print(sysconfig.get_paths()["purelib"])')/pyjhora-vendor.pth" \
    && chmod -R a+rX,go-w /opt/pyjhora
WORKDIR /app
COPY --chown=root:root services/jhora-svc/app.py /app/app.py
COPY --chown=root:root services/jhora-svc/jhora_svc /app/jhora_svc
# Build-time proof (fails the build, never the runtime):
#  1. PyQt6 / pyqtgraph are NOT installed (the service must run headless without them);
#  2. the patched PyJHora imports and the Lahiri self-check passes in this image.
RUN python - <<'PY'
import importlib.util, sys
for gui in ("PyQt6", "pyqtgraph", "img2pdf"):
    assert importlib.util.find_spec(gui) is None, f"{gui} must not be installed in jhora-svc"
sys.path.insert(0, "/app")
from jhora_svc import bootstrap, compute
st = bootstrap.bootstrap()
assert st["ayanamsa"] == "LAHIRI" and st["true_nodes"] is True, st
chk = bootstrap.self_check(compute.GOLDEN_JD_UT)
print("jhora-svc image self-check:", chk)
print("ephe files:", bootstrap.ephe_file_count())
assert bootstrap.ephe_file_count() > 0
PY
USER 10002:10002
EXPOSE 7792
HEALTHCHECK --interval=10s --timeout=5s --start-period=180s --retries=3 \
  CMD ["python", "-c", "import sys,urllib.request; r=urllib.request.urlopen('http://127.0.0.1:7792/v1/health', timeout=4); sys.exit(0 if r.status==200 and b'\"ok\":true' in r.read() else 1)"]
CMD ["python", "-m", "uvicorn", "app:app", "--host", "0.0.0.0", "--port", "7792", "--workers", "1", "--no-access-log"]

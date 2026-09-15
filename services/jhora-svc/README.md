# jhora-svc — PyJHora sidecar (127.0.0.1:7792)

JSON REST wrapper around the vendored **PyJHora 5.0** tree (`vendor/pyjhora`, AGPL-3.0).
PyJHora stays an isolated process: nothing outside this directory imports it, and this
service is never linked into `jyotish-mcp`. Binding contract: `docs/CONTRACT.md`.

## Canonical configuration (enforced at import time, proven at boot)

| Item | Value | Where |
|---|---|---|
| Ayanamsa | `LAHIRI` (`const._DEFAULT_AYANAMSA_MODE`, shipped default is `TRUE_PUSHYA`) | `jhora_svc/bootstrap.py` |
| Rahu/Ketu | true node (`swe.TRUE_NODE`) | `bootstrap.py` |
| Dasha year | `MEAN_SIDEREAL_YEAR` (365.256364 d) — the configuration PyJHora's own test-suite validates; the shipped `TRUE_SIDEREAL_YEAR` path produced non-monotonic sub-periods at depth ≥ 4 on the golden chart | `bootstrap.py` |
| Network | `geocoder`, `Nominatim`, `requests`, `urllib`, every IP/elevation/geocode helper in `jhora.utils` / `jhora.place_db` → `RuntimeError("network geocoding disabled")` | `bootstrap.py` |
| Settings loader | `jhora.config._SETTINGS_LOADED = True` so `data/user_settings.json` can never re-point `const` | `bootstrap.py` |
| Ephemeris | bundled Swiss `.se1` files at `vendor/pyjhora/src/jhora/data/ephe` (100 files) | `.pth` entry keeps the source tree in place |
| Concurrency | `ProcessPoolExecutor` (spawn), `min(4, cpu)` workers; each worker self-checks + warms up on the golden chart; Swiss state is process-global so threads are never used | `jhora_svc/pool.py` |

Boot fails closed: every worker's initializer runs `bootstrap.self_check()` (drik ayanamsa vs
`swe.get_ayanamsa_ut` with `SIDM_LAHIRI` for the golden JD, must agree < 1″) and `golden_warmup()`
(Moon in Swati, Vimshottari maha-dashas sum to 120 y ± 0.01). Any failure aborts start-up.

## Setup / run / test

```bash
./setup_env.sh            # uv venv (py3.12) + pinned PyJHora deps + .pth to vendor/pyjhora/src + fastapi/uvicorn/pytest/httpx
./run.sh                  # uvicorn, 127.0.0.1:7792, 1 process + worker pool (env: JHORA_SVC_WORKERS, JHORA_SVC_LOG_LEVEL)
.venv/bin/python -m pytest -q          # service tests (real pool, httpx ASGI client, pyswisseph cross-checks)
.venv/bin/python run_pvr_tests.py --nodes mean   # PyJHora's own ~10k-test suite, LAHIRI, headless (see validation/)
```

`uv pip install -e ../../vendor/pyjhora` does not work (its `pyproject.toml` uses
`license-expression`, rejected by the setuptools chosen for the isolated build), hence the `.pth`.

## Endpoints

| Method/path | Input | Output |
|---|---|---|
| `GET /v1/health` | – | `{ok, ayanamsa:"LAHIRI", true_nodes:true, pyswisseph, ephe_files, workers, self_check{…}, golden{…}, catalog{…}}` |
| `POST /v1/positions` | `BirthInput` | `{bodies{Sun…Ketu:{lon,speed,retro,lat,rasi,nakshatra,pada}}, ascendant, ayanamsa_deg, jd_ut, flags}` |
| `GET /v1/catalog/dasha` | – | every module under `jhora/horoscope/dhasa/{graha,raasi,annual}` with `status`, `entry_function`, `golden_smoke` |
| `POST /v1/dasha` | `{birth, system:"graha.vimsottari", depth:1..5}` | nested periods with UTC ISO instants, `balance_at_birth_years` (+`balance_source`) |
| `POST /v1/panchang` | `{date:"YYYY-MM-DD", lat, lon, tz_offset_hours}` | tithi/nakshatra/yoga/karana (+ next ones in the day) with start/end UTC, sunrise/sunset/next sunrise UTC, vaara |
| `POST /v1/kuta` | `{a: BirthInput, b: BirthInput}` | Ashtakoota 8 kutas (score/max, total /36), 4 dosha checks, 10 south-Indian poruthams |

Errors are always structured: `{"error": "<code>", "detail": …}` with 422 for validation, unknown /
unsupported systems, oversized depth (`too_many_periods`, > 120 000 leaf periods) and PyJHora data
faults (`period_data_error`, e.g. `raasi.brahma` returns a −1-year period for the golden chart);
504 on computation timeout; 403 for non-loopback clients; 500 only for genuinely unexpected faults.

## Semantics worth knowing

* **`utc` is authoritative.** `jd_ut` comes from `swe.utc_to_jd` (leap-second aware). PyJHora
  needs local clock time, derived as `jd_ut + tz_offset_hours/24`; all outputs are converted back
  to UTC ISO strings (`jhora_svc/timeconv.py`).
* **Positions** are apparent geocentric sidereal Lahiri (`FLG_SWIEPH|FLG_SIDEREAL|FLG_SPEED`), the
  Swiss default used for cross-engine consensus. PyJHora's *internal* techniques (dasha seeds,
  panchang longitudes) use PyJHora's own `PLANET_FLAGS` (geometric `FLG_TRUEPOS`, ≈20″ different);
  both flag values are echoed in responses (`flags`, `longitude_swe_flags`).
* **Dasha** rows are PyJHora's deepest-level list `[lords, (Y,M,D,local h), duration]`, grouped into
  a tree by adjacent-prefix runs (cycling systems such as `graha.rashmi` repeat lords, so grouping
  is positional, not by key). `balance_at_birth_years` is the module's own (y,m,d) balance when it
  returns one (Vimshottari, Yoga-Vimshottari), otherwise the derived remaining length of the first
  period (`balance_source: "derived"`). `annual.mudda` reports durations in days; the unit is
  inferred from consecutive starts and echoed.
* **Panchang** is referenced at the local sunrise of the requested date (`drik.sunrise`). The
  element *identification* is arithmetic on `drik.lunar_longitude/solar_longitude` and is
  cross-checked against `drik.tithi/nakshatra/yogam/karana` (mismatch → 422, never a guess).
  Transition instants are solved by bisection on the same longitude functions (< 0.1 s) because
  PyJHora's end-time estimators are linear extrapolations (minutes of error) and its
  inverse-Lagrange path evaluates a local JD as UTC.
* **Kuta** uses only the two Moon nakshatra/pada values, exactly as
  `jhora.horoscope.match.compatibility.Ashtakoota` does; `a` is the boy/first partner.

## Files

`app.py` (FastAPI, endpoints, error mapping) · `jhora_svc/bootstrap.py` (PyJHora patches + self-check) ·
`jhora_svc/compute.py` (positions, panchang, kuta, warm-up) · `jhora_svc/dasha_catalog.py`
(module discovery, invocation, normalisation, smoke) · `jhora_svc/pool.py` · `jhora_svc/models.py` ·
`jhora_svc/timeconv.py` · `jhora_svc/names.py` · `tests/` · `run_pvr_tests.py` · `setup_env.sh` · `run.sh`.

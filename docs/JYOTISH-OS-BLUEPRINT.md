# JYOTISH-OS — Unified Orchestration Blueprint
### XALEN Ephemeris × PyJHora × VedAstro × IronClaw, compiled into one fail-closed system

**Status:** Design specification grounded in the four source archives inspected on 2026-09-16
(`VedAstro-master.zip`, `PyJHora-main.zip`, `ironclaw-main.zip`, `xalen-ephemeris-main.zip`).
Every file path, function name, config key, and limit cited below was read directly from
those archives (**OBSERVED**). Anything not read from source is labeled **ASSUMPTION** or
**EXPERT JUDGMENT**. Nothing in this document was compiled or executed this session.

---

## 0. The one decision that shapes everything

The four repos are not four competing astrology programs. They are four **incompatible
strengths** that, wired correctly, cancel each other's weaknesses:

| Layer | Component | Why it owns this layer (evidence) | What it must NEVER do |
|---|---|---|---|
| **L0 Positions** | XALEN (`crates/xalen-ephem`, `-time`, `-coords`, `-houses`, `-ayanamsa`) + JPL `de440s.bsp` | Pure Rust, `Send + Sync`, ~380 µs/chart, Apache-2.0, DE440 reader → sub-arcsecond on every body incl. Moon (`docs/ACCURACY.md`) | Never run without the DE440 kernel in production (analytic Moon max error 12″, Pluto only 1885–2099) |
| **L1 Vedic technique** | PyJHora (`src/jhora/horoscope/**`, `src/jhora/panchanga/**`) as an isolated HTTP service | 26 graha + 27 rasi + 2 annual dasha systems, Tajaka, Jaimini, Surya-Siddhanta panchanga, verified against PVR Rao's book + JHora (README: ~6,800 tests) | Never linked into your binary (AGPL-3.0); never called for raw positions (that is L0's job) |
| **L2 Interpretation + evidence** | VedAstro rule base (`Library/XMLData/EventDataList.xml` = 1,018 rules, `HoroscopeDataList.xml` = 491 rules) + `HuggingFace/PersonList-15k.csv`, `MarriageInfoDataset.csv` | Only repo with (a) machine-readable prediction rules and (b) a labeled real-world dataset to test them; MIT | Never exposed publicly (all its Azure Function routes are `AuthorizationLevel.Anonymous`); never trusted for `-not-proved.xml` rules without validation |
| **L3 Agent** | IronClaw v1.3.0, `local-dev` profile | Kernel-mediated capabilities, WASM sandbox, secret injection at egress with leak scan, prompt-injection policy layer, cron/heartbeat/reactive routines, Ollama for fully local inference | Never given raw engine access; only the typed MCP tool surface in §3 |
| **L4 Consensus (new)** | `jyotish-mcp` — a Rust HTTP/MCP server we write, embedding XALEN crates natively and proxying L1/L2 | Single choke point: canonical config, cross-engine differential check, schema validation, caching, audit | Never return a position that L0 and L1 disagree on beyond tolerance (fail closed, §7) |

**Result:** one binary you own (`jyotish-mcp`, Apache-2.0 + MIT dependencies only), two
sidecar services you run but do not distribute (PyJHora AGPL, VedAstro MIT), and one agent
runtime that turns it into daily real-life output.

---

## 1. Canonical Configuration Contract (kills the #1 silent failure: config drift)

Three engines, three defaults, three different answers for the same birth. This contract is
enforced at `jyotish-mcp` startup; a mismatch aborts the process.

| Parameter | Canonical value | XALEN setting | PyJHora setting (as shipped → required) | VedAstro setting |
|---|---|---|---|---|
| Ayanamsa | **Lahiri** | `Ayanamsa::Lahiri` (`crates/xalen-ayanamsa`) | `const._DEFAULT_AYANAMSA_MODE` is **`'TRUE_PUSHYA'`** at `src/jhora/const.py:303` — the README's 6,800 tests assume `'LAHIRI'`. **Must be set to `'LAHIRI'` before first `drik` call.** | `Ayanamsa.LAHIRI = 1` (`Library/Data/Enum/Ayanamsa.cs:18`) |
| Rahu/Ketu | **True node** | `Body::TrueNode` (README lists both mean and true) | `const._use_true_nodes_for_rahu_ketu = True` (`const.py:132`) — already canonical | Default not confirmed in source read — **UNVERIFIED**; set explicitly via its `Calculate` option or exclude node-dependent VedAstro rules until confirmed |
| Ephemeris backend | **JPL DE440** | `De440Provider::from_auto_cache()` (`crates/xalen-ephem/src/kernel_cache.rs`, feature `kernel-autodownload`), verified by `is_confirmed_de440()` | pyswisseph 2.10.3.2 + bundled `sepl_*`/`semo_*` `.se1` files (104 MB, `src/jhora/data/ephe`) — DE431-derived, Swiss-grade | SwissEphNet 2.8.0.2 (`Library/Library.csproj:83`) — older port; **positions from VedAstro are never used**, only its rule predicates |
| ΔT model | XALEN: Stephenson–Morrison–Hohenkerk 2016 with σ envelope; Swiss: its own table | Report XALEN's `σ(ΔT)` in every response for dates < 1900 | — | — |
| Time input | **UTC + explicit lat/long/tz** supplied by caller | `calendar_to_jd(..., CalendarSystem)` | `drik.Date(y,m,d)` + place tuple | ISO time + location string |
| Geocoding | **Forbidden at runtime** | `xalen-world` 130-city table for convenience only | `src/jhora/utils.py:89` calls `geocoder.ip('me')` and `utils.py:39` uses Nominatim — **both stripped/monkey-patched out in jhora-svc** | `LocationManager.cs` uses Google/Azure Maps keys — not invoked |
| House system (Vedic) | Whole Sign for rasi; Sripati for bhava chalit | `HouseSystem::WholeSign`, `HouseSystem::Sripati` | `charts.bhava_chart(jd, place, bhava_madhya_method=...)` (`charts.py:133`) | `Calculate` house functions |
| Kernel integrity | `de440s.bsp` SHA-256 pinned at first download; mismatch = refuse to start | XALEN `is_confirmed_de440()` header check + our hash pin | n/a | n/a |

---

## 2. Topology

```
                        ┌──────────────────────────────────────────────┐
                        │  IronClaw 1.3.0  (local-dev profile)          │
                        │  ├─ LLM: Ollama (local) │ Anthropic │ NEAR AI │
                        │  ├─ Channels: REPL · WebUI · Telegram · Slack │
                        │  ├─ Routines: cron · heartbeat · reactive     │
                        │  └─ Extension "jyotish" (kind: mcp_server,    │
                        │       url: http://127.0.0.1:7791/mcp)         │
                        └───────────────┬──────────────────────────────┘
                                        │ HTTP JSON-RPC 2.0 (MCP) — stdio is
                                        │ rejected by IronClaw (docs/extensions/mcp.mdx)
                        ┌───────────────▼──────────────────────────────┐
                        │  jyotish-mcp  (Rust, axum, single binary)     │
                        │  ├─ Canonical config guard (§1)               │
                        │  ├─ Schema-validated tool surface (§3)        │
                        │  ├─ L0: XALEN crates linked natively          │
                        │  │     Almanac::with_de440(de440s.bsp)        │
                        │  ├─ Consensus engine (§7)                     │
                        │  ├─ Content-addressed cache (§5)              │
                        │  └─ Audit log (append-only JSONL)             │
                        └──────┬──────────────────────────┬────────────┘
                 internal HTTP │ 127.0.0.1:7792           │ 127.0.0.1:7793
              ┌────────────────▼───────────┐   ┌──────────▼─────────────────┐
              │ jhora-svc (Python/FastAPI)  │   │ vedastro-svc (.NET 8)       │
              │ PyJHora 5.0, process pool   │   │ VedAstro.Library only       │
              │ LAHIRI forced, no PyQt, no  │   │ rule evaluation + datasets  │
              │ network, AGPL kept isolated │   │ Secrets via env, no Azure   │
              └────────────────────────────┘   └────────────────────────────┘
```

**Hard rules of the topology**
1. IronClaw talks only to `jyotish-mcp`. It never sees jhora-svc or vedastro-svc.
2. jhora-svc and vedastro-svc bind `127.0.0.1` only, run as separate OS users, no outbound network (enforced with an egress-deny firewall rule or a `network_mode: none` Docker network + one internal bridge).
3. `jyotish-mcp` is the only process holding the DE440 kernel, the cache, and the audit log.
4. All four processes are supervised (launchd/systemd); any one crashing does not take the others down; a missing sidecar degrades the tool list (tools that need it are unregistered), never returns fake data.

---

## 3. Tool Surface (`jyotish-mcp`) — typed, closed, no free-text pass-through

Every tool: JSON-schema validated input, typed output, `evidence` block on every response
(`engine`, `kernel`, `ayanamsa`, `delta_t_sigma_sec`, `consensus_status`).

| Tool | Backed by | Input (required) | Output highlights |
|---|---|---|---|
| `chart.compute` | XALEN `Almanac` + `xalen-houses` + `xalen-vedic` | `utc`, `lat`, `lon`, `tz`, `varga` ∈ {D1,D2,D3,D4,D7,D9,D10,D12,D16,D20,D24,D27,D30,D40,D45,D60} | Sidereal longitudes, rashi, nakshatra+pada, **`boundary_distance_sec`** for every nakshatra/pada/rashi (how many seconds of clock time until the value flips), retrograde flags, speeds, bhava cusps |
| `panchang.day` | XALEN `xalen-vedic::panchang` (transition times) cross-checked vs PyJHora `drik.tithi/nakshatra/yoga/karana` | `date`, `lat`, `lon`, `tz` | Tithi/nakshatra/yoga/karana with start/end instants, sunrise/sunset (`drik.sunrise`, `drik.py:507`), hora table, rahu kala, abhijit |
| `dasha.timeline` | PyJHora `horoscope/dhasa/graha/*` (26) and `raasi/*` (27), `annual/*` (2); XALEN Vimshottari as cross-check | `birth`, `system` ∈ 55-value enum generated from the module list, `depth` ≤ 5 | Nested periods with UTC instants, lord chain, **balance at birth**; Vimshottari must agree between XALEN and PyJHora to ≤ 1 day or consensus fails |
| `transit.window` | XALEN sweep (µs/step) | `birth`, `from`, `to`, `bodies`, `natal_points` | Exact ingress/aspect instants (bisection on XALEN), Sade Sati windows, Ashtakavarga bindu at transit (`PyJHora chart/ashtakavarga.py`) |
| `muhurta.find` | VedAstro `EventDataList.xml` rules (1,018) evaluated by vedastro-svc; positions injected from XALEN | `activity` (enum from rule `<Name>` tags, e.g. `GoodLunarDayForTravel`), `from`, `to`, `location` | Ranked windows with the exact rule IDs that fired and those that vetoed |
| `match.kuta` | PyJHora `horoscope/match/` + VedAstro 10-kuta | two births | Per-kuta score, dosha flags, both engines' totals side-by-side |
| `rectify.birth_time` | XALEN sweep + PyJHora dasha lords + VedAstro `BirthTimeFinderAPI` logic | `birth ± window_min`, `events[]` (dated life events) | Candidate times ranked by event-fit score; explicit statement that this is heuristic |
| `rule.validate` | VedAstro `HoroscopeDataList.xml` rule + `PersonList-15k.csv` / `MarriageInfoDataset.csv` | `rule_id`, `outcome_column` | Hit rate vs base rate, sample n, 95% CI, verdict `PROMOTE` / `KEEP_UNPROVED` |
| `engine.consensus` | all three | any chart request | Per-body Δ between XALEN-DE440 and PyJHora-Swiss in arcseconds; pass/fail vs §7 tolerances |
| `catalog.list` | static | — | Exposes exactly which ayanamsas / house systems / dasha systems are enabled (Gauquelin, Qi Men, PullenSinusoidalRatio deliberately absent — see §4) |

**Nothing returns prose.** Interpretation text is generated by the LLM inside IronClaw from
typed facts, so no engine output can carry an injected instruction.

---

## 4. Every limitation found → its removal (specific, sourced)

### XALEN
| Limitation (source) | Removal |
|---|---|
| Analytic Moon RMS 2.8″ / max 12″ (`docs/ACCURACY.md`) | DE440 mandatory in production → Moon sub-arcsecond (same doc, "DE440 Optional High-Precision Mode"). Analytic path allowed only in the WASM/mobile client, and every such result is tagged `engine: "xalen-analytic"` |
| Pluto valid 1885–2099 only | DE440 `de440s.bsp` spans 1550–2650; outside that, `chart.compute` returns Pluto as `null` with reason, never extrapolated |
| Chiron 1–2° (osculating elements, 1950–2050) | Neither XALEN nor PyJHora-as-shipped can do better (PyJHora bundles no `seas_*.se1`). Fix: jhora-svc downloads `seas_18.se1`/`seas_24.se1` from Astrodienst at build time → Swiss computes Chiron from its numerical integration; XALEN Chiron is demoted to `precision: "degree"` and excluded from consensus |
| Gauquelin sectors returns Placidus (README, explicit placeholder) | Not in `catalog.list`; request → `400 UNSUPPORTED`, not silent wrong data |
| Qi Men Dun Jia experimental; PullenSinusoidalRatio ~5° approximation (`docs/COMPARISON.md`) | Same: excluded from the enabled catalog until their own tests pin them |
| Crates/PyPI/npm **not published** (README) | Vendored as a git submodule pinned to a commit; `jyotish-mcp` links the crates by path — no registry dependency, reproducible build |
| Pre-1.0 (v0.6.0), API may change | Pin the submodule commit; the §7 differential suite is the upgrade gate — bump only when it stays green |
| ΔT uncertainty for ancient dates | Response carries `delta_t_sigma_sec` from XALEN's published envelope; UI shows "Ascendant ± X°" for pre-1600 charts |

### PyJHora
| Limitation (source) | Removal |
|---|---|
| AGPL-3.0 (`pyproject.toml`) | Runs as a separate service, separate process, never linked, never distributed with your product. Your code (jyotish-mcp) stays Apache/MIT. (**EXPERT JUDGMENT** on licensing boundary — confirm with counsel before commercial distribution) |
| Default ayanamsa `TRUE_PUSHYA` vs tests assuming LAHIRI (`const.py:303`, README) | jhora-svc sets `const._DEFAULT_AYANAMSA_MODE = 'LAHIRI'` on import and asserts it in a startup self-test that recomputes one known chart |
| Runtime geocoding + IP lookup (`utils.py:39`, `utils.py:89`) | Monkey-patched to raise; the service has no outbound network anyway |
| Swiss Ephemeris process-global state (`swe.set_sid_mode`, `drik.py:225–233`) | jhora-svc uses a **process pool** (one interpreter per worker), never threads; each worker sets ayanamsa once at start |
| Python speed | PyJHora is called only for techniques XALEN lacks; results cached by canonical request hash (§5); interactive latency target < 300 ms after warm cache |
| PyQt6 in `requirements.txt` | `drik.py`, `charts.py`, dasha modules import only `swisseph`, `numpy`, `utils`, `const` (`drik.py:47–58`); jhora-svc installs the package but never imports `jhora.ui.*`; runs headless |
| 104 MB `.se1` files not in the PyPI wheel (README) | Copied from the repo's `src/jhora/data/ephe` into the service image at build time; hash-verified |

### VedAstro
| Limitation / vulnerability (source) | Removal |
|---|---|
| Secrets as a hidden partial class in source (`Library/Secrets-HideMe-sample.cs`; `ChatAPI.cs:37,46,483,562,600` call `Secrets.Get(...)`) | vedastro-svc implements `Secrets.Get` as an env-var/secret-store lookup that **returns empty and disables the feature** for anything Azure/OpenAI/Cohere — those code paths (`ChatAPI.cs`, `LLMEmbeddingManager.cs`, `AzureCache.cs`) are never invoked |
| Every API route is `AuthorizationLevel.Anonymous` (`API/FrontDesk/*.cs`) | The `API/` Azure Functions project is **not deployed at all**. Only `Library/` is loaded, behind a loopback-only minimal host |
| SwissEphNet 2.8.0.2 (older port) | VedAstro positions are never surfaced; only rule predicates run. Where a rule needs a position, the rule is evaluated on XALEN-supplied longitudes (**ASSUMPTION**: the predicate functions in `Logic/Calculate/Core.cs` can be fed a pre-computed `Time`; if a predicate recomputes internally, the ≤ 1″ Swiss-2.8 vs DE440 difference is below any rule's sensitivity — EXPERT JUDGMENT) |
| `EventDataList-not-proved.xml`, `HoroscopeDataList-not-proved.xml` | Loaded into a quarantine set; only `rule.validate` can promote a rule, and only with n ≥ 200 and a CI that excludes the base rate |
| Thin test coverage (`LibraryTests`: 9 files) | Covered externally by the §7 differential suite; VedAstro is never the sole source of any number |
| Outdated deps (`Azure.AI.OpenAI 1.0.0-beta.17`, `Newtonsoft.Json 13.0.3`, `EPPlus 7.0.0-beta1`) | Build with `dotnet list package --vulnerable` gate; unused Azure/OpenAI packages removed from the trimmed `Library.csproj` fork used by vedastro-svc |
| Marketing "Perfect Predictions" claims (README) | Not a technical issue, but the product must never repeat them; every prediction carries `validation_status` from `rule.validate` |

### IronClaw
| Limitation (source) | Removal |
|---|---|
| PostgreSQL 15 + pgvector listed as prerequisite (README) | `docs/capabilities/database.mdx`: the `local-dev` profile uses embedded database files under `~/.ironclaw/reborn` — **no Postgres for single-user**. Postgres only if you serve multiple people |
| MCP over stdio rejected (`docs/extensions/mcp.mdx`) | `jyotish-mcp` is HTTP JSON-RPC 2.0 from day one |
| Cloud LLM sees birth data | Option A: `LLM_BACKEND=ollama` (`docs/capabilities/llm-providers.md:206–212`) → nothing leaves the machine. Option B: Anthropic provider, but the prompt receives only the typed tool output for the question asked, never the full dataset |
| Prompt injection via user-supplied names/questions/journal text | IronClaw safety layer (Block/Warn/Review/Sanitize) + the fact that engine responses are typed JSON with no external text fields |
| Rust 1.96 toolchain, ~1.5M LOC build | Use the released installer/Homebrew binary (README "Install"), not a source build, unless you patch it |

### Cross-cutting (the real accuracy ceiling)
| Limitation | Removal |
|---|---|
| **Birth-time uncertainty** — 1 min ≈ 0.25° of Ascendant, ~900× larger than any ephemeris error | `chart.compute` always returns `boundary_distance_sec`; `rectify.birth_time` is a first-class tool; charts with unknown seconds are computed at the minute's midpoint and flagged |
| Nakshatra/pada/rashi sandhi flips | Same `boundary_distance_sec`; the LLM is instructed (system prompt in IronClaw identity file) to state when a value is within 120 s of flipping |
| Three engines, three opinions | §7 consensus — disagreement is surfaced, never averaged |

---

## 5. Bottlenecks → removals

| Bottleneck | Where | Fix |
|---|---|---|
| Kernel download at runtime (~32 MB from NAIF) | XALEN `kernel-autodownload` | Pre-baked into the `jyotish-mcp` image; runtime download disabled; hash-pinned |
| Python GIL / Swiss global state | jhora-svc | Process pool sized to cores; warm-up computes one chart per worker at boot |
| .NET cold start | vedastro-svc | Kept resident under the supervisor; ReadyToRun compile; health endpoint |
| Repeated identical requests (daily panchang for the same place, natal chart re-reads) | all | Content-addressed cache keyed by `sha256(canonical_json(request) + config_version + kernel_hash)`; natal results are immutable → cache forever; panchang/transit results TTL = until the next transition instant they contain |
| Geocoder/timezone network calls | PyJHora, VedAstro | Removed; callers pass lat/lon/tz; `timezonefinder` (already in PyJHora deps) runs offline if a tz must be derived |
| LLM latency for daily briefs | IronClaw | Routine pre-computes facts via tools at 04:00 local, LLM writes the brief once, delivered to Telegram — no on-demand wait |
| Per-request `new SwissEph()` in `Core.cs:3529` | VedAstro | Irrelevant once positions come from XALEN; the object is cheap, but the ephemeris file open per call is avoided by never taking this path |
| Million-chart research sweeps | Python | Only XALEN runs sweeps (thread-safe `Almanac`, rayon); PyJHora/VedAstro touched only for the surviving candidates |

---

## 6. Security model (threat → control → where enforced)

| Threat | Control | Enforced in |
|---|---|---|
| Secret leakage (API keys, storage keys) | No secrets in any repo file; IronClaw encrypted store (AES-256-GCM per README) + env injection for sidecars; VedAstro `Secrets.Get` neutralized | IronClaw `ironclaw config set` (hidden prompt), jyotish-mcp reads env only |
| Birth data exfiltration by a tool or MCP server | IronClaw egress allowlist; jyotish-mcp is the only allowed MCP host; sidecars have no network | IronClaw `ironclaw_network` substrate; OS firewall |
| Prompt injection via journal/email content fed to the astrologer | IronClaw safety policies; engine outputs are typed; the LLM never receives raw engine text | `ironclaw_safety` crate |
| Wrong-config silent errors (ayanamsa, node type, kernel) | Startup self-test computes a golden chart on all three engines and aborts on mismatch | jyotish-mcp boot |
| Supply chain (pinned commits, vulnerable NuGet/crates) | `cargo deny` (IronClaw already ships `deny.toml`), `cargo audit`, `pip-audit`, `dotnet list package --vulnerable` in CI; submodule commits pinned | CI |
| Public exposure of anonymous VedAstro API | Not deployed; loopback only | Topology §2 |
| Data integrity of rule promotions | `rule.validate` writes an append-only promotion log with dataset hash, n, CI, timestamp | jyotish-mcp audit log |
| Fail-open on sidecar failure | Tools depending on a dead sidecar are unregistered from the MCP tool list; never fabricated | jyotish-mcp health loop |

---

## 7. Consensus & verification gates (the guarantee lives here, not in prose)

**Differential corpus:** 10,000 pseudo-random births, 1900–2050, global lat ±66°, seeded.
For each: XALEN-DE440 vs PyJHora-Swiss. Tolerances (EXPERT JUDGMENT, derived from the
documented accuracies of each path):

| Quantity | Tolerance | Rationale |
|---|---|---|
| Sun, Mercury–Saturn longitude | ≤ 1.0″ | Both paths read JPL-class data; XALEN doc: ≤ 0.76″ |
| Moon longitude | ≤ 1.5″ with DE440 (≤ 15″ analytic fallback) | XALEN DE440 Moon "sub-arcsecond"; analytic max 12″ |
| Uranus/Neptune | ≤ 3.0″ | XALEN doc: 1.8–2.5″ |
| Lahiri ayanamsa | ≤ 1.0″ | XALEN doc: 46/47 SE ayanamsas < 1″ |
| Ascendant / cusps | ≤ 0.01° | XALEN doc: 19 house systems within 0.01° |
| True Node | ≤ 5″ | Swiss true node vs XALEN true node; loosened for osculating differences — confirm empirically |
| Vimshottari period boundaries | ≤ 1 day | Balance-at-birth arithmetic identical; ΔT/ayanamsa jitter only |

Any chart outside tolerance is a **hard failure** of the release, not a warning.

**Gates that block, in order:**
1. `pre-commit`: `cargo fmt --check`, `cargo clippy -D warnings`, `ruff`, `dotnet format --verify-no-changes`, `gitleaks`
2. CI (GitHub Actions, ASSUMPTION: GitHub host): build all three services, run XALEN's own `cargo test --workspace` (doc claims 2,199 tests), PyJHora `jhora.tests.pvr_tests` with `LAHIRI`, the differential corpus above, `cargo run -p xalen-validation --release --bin de440_bench` (BENCHMARK.md) with regression thresholds
3. Boot-time golden-chart self-test in `jyotish-mcp` (§6)
4. Branch protection requiring the CI check (account-level; must be set in the repo settings)

**Residual honesty:** these are configuration-enforced gates, not mathematical proofs. A
disabled hook or force-merge bypasses them. And *no gate here validates astrology itself* —
only that the numbers are the right numbers for the chosen tradition. Predictive validity is
answered exclusively by `rule.validate` against real outcomes.

---

## 8. Build order (phased; each phase leaves a working system)

**Phase 1 — Positions you can trust (XALEN + DE440)**
```bash
git submodule add https://github.com/vedika-io/xalen-ephemeris vendor/xalen && git -C vendor/xalen checkout <pinned-commit>
curl -L -o kernels/de440s.bsp https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440s.bsp
shasum -a 256 kernels/de440s.bsp > kernels/de440s.sha256   # pin it
cargo new jyotish-mcp && # link vendor/xalen/crates/{xalen-ephem,xalen-time,xalen-coords,xalen-houses,xalen-ayanamsa,xalen-vedic,xalen-chart} by path
```
Deliver `chart.compute`, `panchang.day`, `transit.window`, `catalog.list` over HTTP MCP. Run
`cargo test --workspace` in `vendor/xalen` and the DE440 bench; record real output.

**Phase 2 — Vedic depth (jhora-svc)**
```bash
python -m venv .venv && . .venv/bin/activate && pip install -r vendor/pyjhora/requirements.txt fastapi uvicorn
cp -r vendor/pyjhora/src/jhora/data/ephe .venv/lib/python*/site-packages/jhora/data/ephe
```
Startup: set `const._DEFAULT_AYANAMSA_MODE='LAHIRI'`; patch `utils.geocoder`/`Nominatim` to
raise; run `jhora.tests.pvr_tests` once and store the pass count as a build artifact.
Deliver `dasha.timeline` (55 systems), `match.kuta`; wire the Vimshottari cross-check.

**Phase 3 — Knowledge + evidence (vedastro-svc)**
Fork `Library/` only; delete `API/`, `Website/`, `LLMCoder/`, Azure packages; implement
`Secrets.Get` → env; load `EventDataList.xml` + `HoroscopeDataList.xml` (proved) and quarantine
the `-not-proved` files. Import `HuggingFace/PersonList-15k.csv` and `MarriageInfoDataset.csv`
into a local SQLite. Deliver `muhurta.find`, `rule.validate`, `rectify.birth_time`.

**Phase 4 — Consensus + gates**
Differential corpus, tolerances, boot self-test, CI, pre-commit, audit log.

**Phase 5 — Agent (IronClaw)**
```bash
brew install ironclaw && ironclaw onboard        # choose local-dev profile
ironclaw models set-provider ollama              # or anthropic
# Package jyotish-mcp as an extension: manifest kind "mcp_server", url http://127.0.0.1:7791/mcp
ironclaw extension install ./extensions/jyotish && ironclaw extension activate jyotish
```
Identity file: astrologer persona + the "state boundary proximity" rule + "never claim
certainty; cite `validation_status`". Routines (created by asking the agent, which calls
`routine_create` per `docs/capabilities/routines/cron.mdx`):
- `0 4 * * *` — today's panchang + personal transits + running dasha → Telegram brief
- `0 6 * * 1` — week's muhurta windows for the activities you configured
- reactive — dasha/antardasha change within 7 days → alert
- heartbeat — nightly `engine.consensus` on the golden chart; any drift → alert, tools disabled

---

## 9. What this delivers in real life (concrete, not generic)

1. **A morning brief you did not have to compute** — tithi, nakshatra with its exact end
   time, your hora table, the transit that changes today, the antardasha you are in and when
   it ends — all from JPL-exact positions, in your language (PyJHora ships en/hi/ka/ml/ta/te
   string tables under `src/jhora/lang/`).
2. **Muhurta on demand, with the rule trail** — "when in the next 10 days for travel east"
   returns windows plus the exact VedAstro rule IDs that passed and vetoed, so a human
   astrologer can audit it.
3. **Birth-time rectification as a sweep, not a guess** — thousands of candidate minutes
   scored against your dated life events in seconds, thanks to XALEN's µs/chart speed.
4. **Evidence instead of belief** — before you act on any yoga/dosha rule, `rule.validate`
   tells you its hit rate on 15,000 real people vs. base rate. Rules that fail stay
   quarantined. This is the one capability none of the four repos delivers alone.
5. **Privacy by construction** — birth data, journal, and calendar never leave the machine
   when Ollama is the model; with a cloud model, only the typed facts for the current
   question are sent, through IronClaw's egress controls.
6. **A commercial-safe core** — everything you ship is Apache-2.0/MIT; AGPL PyJHora stays a
   service you operate.

---

## 10. Evidence ledger

| Claim class | Items |
|---|---|
| **OBSERVED** (read from the archives) | All file paths, line numbers, crate/module lists, license files, `const.py:303` TRUE_PUSHYA default, `const.py:132` true nodes, `utils.py:39/89` network calls, `Library.csproj` package versions, `Secrets.Get` call sites, `AuthorizationLevel.Anonymous` routes, rule counts (1,018 / 491), dataset filenames, IronClaw docs on storage/MCP transport/Ollama/cron, XALEN `docs/ACCURACY.md` figures, absence of `seas_*.se1` in PyJHora, XALEN test-only `unsafe` in `kernel_cache.rs` (core claim holds) |
| **UNVERIFIED** (not executed) | XALEN "2,199 tests", PyJHora "~6,800 tests", all arcsecond figures, IronClaw build |
| **ASSUMPTION** | GitHub Actions as CI host; VedAstro predicates can consume external positions; VedAstro node default |
| **EXPERT JUDGMENT** | Tolerance table in §7; AGPL isolation boundary (confirm with counsel); "birth-time error dominates" magnitude (0.25°/min is standard sidereal-rate arithmetic: 360°/24 h) |
| **NOT COVERED** | Nothing here has been built or run. Phase 1 is the first executable step. |

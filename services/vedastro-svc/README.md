# vedastro-svc — VedAstro rule / dataset sidecar (port 7793, loopback only)

Phase 3 of JYOTISH-OS. Binding contract: `docs/CONTRACT.md`; design: `docs/JYOTISH-OS-BLUEPRINT.md` §1, §3, §4 (VedAstro).

```
services/vedastro-svc/
├── Library.Trimmed/      VedAstro.Library fork (MIT) — see "What is trimmed" below
├── VedAstroSvc/          ASP.NET Core minimal API, net10.0, binds http://127.0.0.1:7793
├── VedAstroSvc.Tests/    xUnit (20 tests: catalog counts, quarantine, muhurta, validate, promotion log, HTTP)
├── vedastro-svc.slnx
├── run.sh                ./run.sh | ./run.sh --daemon | ./run.sh --stop
└── data/                 (git-ignored) vedastro-datasets.sqlite, rule-promotions.jsonl (append-only), pid, log
```

## Run

```bash
./run.sh --daemon            # builds Release, starts, waits for /v1/health
curl -s localhost:7793/v1/health
./run.sh --stop
```
`run.sh` exports `DOTNET_SYSTEM_NET_DISABLEIPV6=1` because on this host NuGet restore hangs in
`SYN_SENT` on IPv6 to api.nuget.org (curl silently falls back to IPv4; .NET does not). Env knobs:
`VEDASTRO_SVC_URL` (bind; default `http://127.0.0.1:7793`), `VEDASTRO_XML_DIR`, `VEDASTRO_DATASET_DIR`
(default `<repo>/vendor/vedastro/HuggingFace`), `VEDASTRO_DATA_DIR` (SQLite cache, default `./data`),
`VEDASTRO_PROMOTION_LOG` (promotion log path, default `<VEDASTRO_DATA_DIR>/rule-promotions.jsonl`).

## Endpoints (exactly CONTRACT.md)

| Route | Body / query | Returns |
|---|---|---|
| `GET /v1/health` | — | `ok, rules_proved (1508), rules_quarantined (128), dataset_rows (15807)` + per-set counts, predicate coverage, XML/CSV SHA-256, ayanamsa=LAHIRI, true_nodes=true |
| `GET /v1/rules?set=event\|horoscope&status=proved\|quarantined` | — | `{count, rules:[{id,name,description,tags,set,status,nature,has_predicate}]}` |
| `POST /v1/muhurta/find` | `{activity, from_utc, to_utc, lat, lon, tz_offset_hours, step_minutes=60, include_quarantined=false, birth?}` | ranked `windows[]` each with `passed_rules[]`, `vetoed_by[]`, `fired_neutral[]` (+ `_quarantined` variants), `rules_unevaluable[]`, `rule_errors{}`, `complete`, `evidence` |
| `POST /v1/rule/validate` | `{rule_id, dataset: marriage\|person, outcome_column, max_rows=300, offset=0}` | `n, fired, hits, hit_rate, base_rate, base_rate_full_dataset, ci95{lo,hi}, verdict PROMOTE\|KEEP_UNPROVED, verdict_reason, validation_status, promotion_status, promotion_scope, promotion_entry_hash, row_errors, evidence` — and appends one row to the promotion log |
| `GET /v1/rule/promotions?rule_id=` | — | `{count, path, entries[]}` — the hash chain, oldest first |
| `GET /v1/rule/promotions/verify` | — | `{ok, entries, first_bad_index, head_hash, path, detail}` — re-hashes the whole chain from disk |

Every error is `400 {error, detail}`; dataset failures are `503 dataset_unavailable`. Nothing returns prose.

### muhurta.find semantics
* The named `activity` must be an event-rule `Name`. Its **siblings** = every event rule sharing at least one
  XML `<Tag>` with it (e.g. `GoodLunarDayForTravel` → the 20 proved `Travel` rules).
* Each rule's C# predicate (`EventCalculatorMethods`, Muhurtha.cs) is evaluated at every `step_minutes`
  slice from `from_utc` to `to_utc` (max 4000 slices) at the given lat/lon; `tz_offset_hours` only sets the
  local day. Windows are maximal runs of slices with an identical fired-set.
* Effective nature = the predicate's `NatureOverride` if set, else the XML `<Nature>`. `Good` → `passed_rules`,
  `Bad` → `vetoed_by`, `Neutral`/unlabelled (486 of 1018 event rules have an empty `<Nature>`) → `fired_neutral`.
  Score = (good hits − bad hits)/slices. Rank: activity rule passed in every slice › no veto › score › duration.
* `birth` (BirthInput) is optional. Rules such as Tarabala/Chandrabala read the native's Moon; without `birth`
  they are evaluated against a placeholder born at `from_utc` and the response says `birth_supplied:false`.
* **Quarantine**: rules that exist only in `*-not-proved.xml` are `status:"quarantined"` (10 event, 118
  horoscope). They are never evaluated unless `include_quarantined:true`; even then they appear only in the
  `*_quarantined` trails and never affect score or rank. Naming a quarantined activity without the flag → 400.
* Fail-closed: a predicate whose upstream implementation is lost (see below) throws
  `RulePredicateUnavailableException` and the rule is listed in `rules_unevaluable`; any other predicate
  exception is recorded in `rule_errors` and `complete:false` — never silently dropped.

### rule.validate semantics
* Datasets are loaded on first use from `vendor/vedastro/HuggingFace/PersonList-15k.csv` (+
  `MarriageInfoDataset.csv`, joined by RowKey) into `data/vedastro-datasets.sqlite` via Microsoft.Data.Sqlite,
  keyed by the CSV SHA-256 (rebuilt only when the CSV changes). 15,807 persons, all with parseable birth data.
* Outcome columns (truthy = numeric &gt; 0):
  `person`: `male, female, rodden_aa`;
  `marriage`: `married, marriage_count, multiple_marriages, any_dissolution, dissolution_count, any_happiness,
  all_happiness, any_struggle_or_tragedy, love_count, arranged_count, any_arranged`.
  Column names are validated against this allow-list before they reach SQL (no string-built SQL from input).
* For the first `max_rows` persons (deterministic id order, `offset` to page): `n` = rows evaluated,
  `fired` = rows where the natal predicate is true, `hits` = fired ∧ outcome truthy, `hit_rate = hits/fired`,
  `base_rate` = outcome truthy over the same `n` rows (`base_rate_full_dataset` over all rows),
  `ci95` = Wilson score interval on `hits/fired`. **PROMOTE only if `fired ≥ 200` and `ci95.lo > base_rate`**,
  else `KEEP_UNPROVED`. The verdict is evidence for a human; the service never rewrites the XML.
* ~300 rows ≈ 0.4 s (Moshier ephemeris, per-request cache). Max `max_rows` = 15807.

### Promotion log (blueprint §6 "data integrity of rule promotions")
* Every `rule.validate` verdict is appended as one JSON line to `data/rule-promotions.jsonl`
  (`VEDASTRO_PROMOTION_LOG`): `{ts, rule_id, dataset, dataset_sha256, outcome_column, n, fired, hits, hit_rate,
  base_rate, ci95:[lo,hi], verdict, prev_hash, entry_hash}` with
  `entry_hash = sha256(prev_hash + canonical JSON of the row without entry_hash)` (canonical = System.Text.Json
  compact, properties in that order; genesis `prev_hash` = 64 zeros). The file is only ever opened with
  `FileMode.Append` and every write is fsync'd; there is no route or method that rewrites, truncates or deletes it
  (`DELETE/PUT/POST /v1/rule/promotions` → 405). No row → no verdict: an unwritable or already-broken chain makes
  `rule.validate` return `503 promotion_log_unavailable`, and a broken chain refuses boot.
* `promotion_status` (`PROMOTED` iff the newest row for that rule says `PROMOTE`; `NOT_PROMOTED`; `NEVER_VALIDATED`)
  is derived from the file on every `GET /v1/rules` / `rule.validate` — never from memory.
* `validation_status` on `rule.validate`: `PROMOTED`, `PROMOTED_STILL_QUARANTINED` (log says PROMOTE but the rule
  still lives only in the not-proved XML — it stays `status: quarantined`, excluded from `muhurta.find`, until a
  human moves it), or `NOT_PROMOTED`. Promotion affects natal confidence reporting only (`promotion_scope`).
* Editing or deleting an interior row breaks the chain (`verify` → `ok:false, first_bad_index`). Truncating the
  tail is only detectable against an external anchor: record `head_hash` (from `verify` or `/v1/health`) in the
  completion ledger / jyotish-mcp audit log when a promotion decision is taken.

## What is trimmed, and why

`vendor/vedastro/Library` **does not compile from source**: 698 errors. Root cause (OBSERVED via GitHub API):
upstream commit `319a610f` (2023-09-28, "folder structure improved") deleted `Library/managers/Calculate.cs`
(516,687 B) and `Library/managers/HoroscopeCalculatorMethods.cs` (146,699 B) and never re-added them; the
2025-02 `Logic/Calculate/Core.cs` restored only a subset. Upstream master (2026-08-13, same as the vendored
zip) still lacks ~70 `Calculate` members and the whole `CalculateHoroscope` class. `vendor/` was not edited.

`Library.Trimmed` = whitelist copy of vendor `Data/**` + `Logic/{EventManager,CacheManager,Format,Syntax,ListExtensions}`
+ `Logic/Calculate/{Core,Muhurtha,Ashtakavarga,VimshottariDasa,Vargas,PanchaPakshi}` + `XMLData/*.xml`, plus:

| File | Origin |
|---|---|
| `Restored/Calculate.Restored-2023-09-28.cs` | upstream `Library/managers/Calculate.cs` @ `dc7880af` (last public revision, MIT), sha256 `d6916a8f…8165db`, with the 86 members that `Core.cs` re-implements or that needed removed infrastructure (chart factories, AutoCalculator, geocoding) deleted (Core.cs wins) and enum renames applied (`Capricornus→Capricorn`, `LunarMonth.*`, `Ayanamsa.LAHIRI`) |
| `Restored/CalculateHoroscope.Restored-2023-09-28.cs` | upstream `HoroscopeCalculatorMethods.cs` @ `dc7880af`, sha256 `d1a1352c…fe963f8960`, class renamed to `CalculateHoroscope`; 6 predicates whose rule no longer exists dropped |
| `Restored/Calculate.Shims.cs` | 2026-name / argument-order forwarders, `Ayanamsa` (int, Swiss sid-mode, default LAHIRI=1), `UseMeanRahuKetu=false`, sidereal `PlanetNirayanaLongitude` via `SEFLG_SIDEREAL`, varga D9 helpers, `DivisionalLongitude`, `LongitudeToLMTOffset`, and fail-closed stubs |
| `Logic/Secrets.cs` | `Secrets.Get(key)` → env `VEDASTRO_SECRET_<KEY>` or `""`; never throws, never reads files |
| `Logic/Tools.cs` | 10 methods copied verbatim from vendor Tools.cs; `ParseTime` throws (geocoding) |
| `Logic/LibLogger.cs` | stderr-only replacement for upstream's HTTP logger; `LogManager` stub |

Removed entirely (with the files that needed them): `Azure.AI.OpenAI`, `Azure.Data.Tables`, `Azure.Storage.Blobs`,
`Microsoft.Azure.Functions.Worker.Core`, `Microsoft.Bing.Search.ImageSearch`, `EPPlus`, `ScottPlot`, `Svg`,
`HtmlAgilityPack`, `Mime-Detective`, `Microsoft.CodeAnalysis.CSharp`, `Microsoft.JSInterop` — i.e. `ChatAPI.cs`,
`LLMEmbeddingManager.cs`, `AzureCache.cs`, `LocationManager.cs`, `Tools.cs`, all `Data/AzureTable`, `Data/Statistic`,
chart factories. Only **SwissEphNet 2.8.0.2** and **Newtonsoft.Json 13.0.3** remain. A test asserts the trimmed
assembly references no `Azure*`, `*OpenAI*`, `*Bing*`, `System.Net.Http`, `System.Net.Requests`. At runtime the
process holds exactly one socket: the 127.0.0.1:7793 listener (`lsof` verified). `Calculate.AddressToGeoLocation`,
`GeoLocationToTimezone`, `Tools.ParseTime` throw `NotSupportedException` — callers pass lat/lon/tz.

Also changed in the trimmed copy: `CacheManager.ResetAll()` added (upstream caches grow for the life of the
process; the service resets after every request), the `MemoryCache.EntriesCollection` reflection hack removed,
`Person`/`LifeEvent` lost their Azure row converters, `EventManager`'s unused Azure storage constant removed.

### Positions: Swiss 2.8, predicates only
All longitudes inside this service come from **SwissEphNet 2.8.0.2 in sidereal mode (Lahiri, true node), Moshier
fallback (no `.se1` files shipped)**. Per blueprint §4 they are consumed only by rule predicates and are never
returned as chart data; XALEN-DE440 (jyotish-mcp) owns positions. The 2023 file's own ayanamsa formula
(year-of-coincidence × 50.33″/yr, Raman) was replaced by the Swiss sidereal mode so the whole library agrees with
CONTRACT §1. Boot self-test: GOLDEN chart Moon = 193.29° sidereal (Swati) or the process refuses to start.

### Known gaps (honest)
* **103 of 490 proved horoscope rules have no predicate** (`has_predicate:false`, mostly `*AshtakavargaYoga*`
  and `HouseNLordInHouseM` added upstream after 2023-09) → `rule.validate` returns 400 `predicate_missing`.
  All 118 quarantined horoscope rules lack predicates too.
* **Pancha Pakshi** (`BirdRuling/Eating/Walking/Sleeping/Dying`, quarantined) and **`Yama1..5`** (proved) call
  `Calculate.MainActivity`/`BirthYama`, whose upstream implementation was never public. Not reconstructed —
  they surface as `rules_unevaluable`.
* Upstream data quality: `HoroscopeDataList.xml` has one `<Event>` inside a comment (parsed count is 490, not
  491); `GoodPlanetsInLagnaForTravel` is labelled `<Nature>Bad</Nature>` although its text describes a good
  yoga. Data is served as upstream wrote it.
* `SolarYearTimeSpan` default 365.25 is an ASSUMPTION (upstream value not public; only affects VimshottariDasa).

## Adding rules
1. Add an `<Event>` (`Name`, `Nature` Good/Bad/Neutral, `Description`, `Tag`) to `Library.Trimmed/XMLData/EventDataList-not-proved.xml`
   (muhurta) or `HoroscopeDataList-not-proved.xml` (natal). It loads as **quarantined**.
2. Add the enum member to `Data/Enum/EventName.cs` / `HoroscopeName.cs` and a predicate
   `[EventCalculator(EventName.X)] public static CalculatorResult X(Time time, Person person)` in Muhurtha.cs
   or `[HoroscopeCalculator(HoroscopeName.X)] public static CalculatorResult X(Time birthTime)` in
   `Restored/CalculateHoroscope.Restored-2023-09-28.cs`. Use only `Calculate.*`; never network.
3. `dotnet test`; then `POST /v1/rule/validate` (natal rules) — promote by moving the `<Event>` to the proved
   file only with `PROMOTE` evidence (n ≥ 200 firing rows, CI excluding the base rate). Promotion is a human,
   reviewed edit; the service never writes XML.

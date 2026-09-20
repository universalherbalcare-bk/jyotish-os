# JYOTISH-OS — Completion Ledger (T3)

**Verdict (2026-09-19, commit HEAD after a943edc): READY WITH KNOWN RISKS — lead-verified re-execution.**
The last fully *independent* verdict is the verifier agent's READY WITH KNOWN RISKS at 771da8c (2026-09-17); the verifier
stalled twice on Docker-blocking calls during the post-ops re-run, so the checks below for phases 8–10 were re-executed by
the lead with hard time caps and are labelled as such. PRODUCTION-READY is blocked on exactly two owner actions (see end).

## Ops hardening + PRODUCTION-READY checklist (EXECUTED 2026-09-19 unless noted)
| Verifier requirement | Status | Evidence |
|---|---|---|
| Supervision (§2) | VERIFIED COMPLETE | compose `restart: always`; uvicorn crash inside jhora-svc → container healthy again in 6 s, RestartCount 1; full Docker daemon restart → all 3 back unaided (11:37:58Z 09-17); stack survived 2 days unattended incl. a daemon restart; `com.jyotish-os.stack` LaunchAgent re-asserts every 5 min; IronClaw `serve` under `com.jyotish-os.ironclaw`, kill → relaunch 5 s |
| OS-user separation (§2) | VERIFIED COMPLETE (container UIDs) | jhora uid 10002, vedastro 10003, jyotish-mcp 10001; read-only rootfs, cap_drop ALL, no-new-privileges. macOS user accounts NOT created (needs sudo — deliberately out of scope) |
| Egress-deny for sidecars (§2) | VERIFIED COMPLETE | sidecars on `internal` network: DNS fails, raw TCP to 1.1.1.1:443 → ENETUNREACH; host listeners: only 127.0.0.1:7791 (TLS) |
| Promotion log | VERIFIED COMPLETE | hash-chained append-only JSONL; 2 validate calls via MCP → `/v1/rule/promotions/verify` ok:true entries 4; vedastro-svc 20/20 tests |
| Routine observed firing | VERIFIED COMPLETE (stub LLM) | two consecutive scheduled fires from the supervised serve reached the containerized MCP (audit 02:29:49Z ×2, 02:30:19Z ×2 on 09-17), then removed, 2-min silence; routine JSONs rewritten to the 1.4.0 `execution_contract` schema the binary presents |
| Real-model turn | BLOCKED (owner: Claude key) | no key stored; serve runs keyless on ollama/qwen3.6 via the fallback launcher; switch = `ironclaw config set anthropic.api_key && ironclaw models set-provider anthropic --model claude-sonnet-5 && launchctl kickstart -k gui/$(id -u)/com.jyotish-os.ironclaw` |
| Hosted CI | VERIFIED COMPLETE — run 35518149781 on github.com (2026-09-20, commit 8416b05): all 6 jobs green incl. consensus 500 charts PASS on the runner with NAIF-downloaded, sha-verified DE440; first run surfaced 3 real defects (upstream PyJHora omits python-dateutil; macOS-only stat -f; shallow-clone gitleaks) — all fixed at the root |
| Branch protection | VERIFIED COMPLETE — owner chose PUBLIC (2026-09-20); API read-back: required checks gates/jyotish-mcp/jhora-svc/vedastro-svc/consensus/compose-build (strict), 1 approving review, enforce_admins true, linear history, force-push and deletion blocked. Note: repo has no LICENSE file yet (owner decision) | vendor pins exact-match and `--check` OK; actionlint 0 findings; `scripts/github-bootstrap.sh --dry-run` prints create/push + protection payload; no remote exists |
| Engine + MCP regression | VERIFIED COMPLETE | jyotish-mcp 44/0 (JYOTISH_TLS=off), jhora-svc 21/0, consensus runner 4/0, pre-commit 6/6 Passed; golden chart via TLS MCP → consensus PASS (cold); IronClaw turn → containerized MCP `success` + audit line |

Known risks accepted under this verdict: (1) IronClaw runs from a local fork build (patches/ironclaw-1.4.0-allow-loopback-egress.patch); `brew upgrade` will not carry it. (2) No real-model turn observed; with a cloud provider, typed content leaves the machine (owner's choice). (3) Docker Desktop is a user-session app: it stopped once during a soak and its VM hung once on relaunch (recovered by a clean app restart); mitigated by `restart: always` + `stack-ensure`, not eliminated. (4) Both LaunchAgents were once found unloaded with no log trace (cause unknown; re-bootstrapped; idempotent). (5) CI never executed on a host; no remote. (6) macOS-level user separation / pf firewall not applied (sudo).

## Evidence (all EXECUTED on this machine, re-run independently by the verifier)
| Suite | Result |
|---|---|
| XALEN `cargo test --workspace` (vendor) | 2,258 passed / 0 failed (builder run, all crates); 2,228 / 0 (verifier run, PyO3/napi cdylibs excluded) |
| DE440 benchmark (3,653 epochs) | Moon RMS 2.82″ max 11.5″ analytic; planets sub-arcsecond — reproduces BENCHMARK.md |
| jyotish-mcp | 31 unit + 3 integration passed; clippy `-D warnings` clean |
| jhora-svc | pytest 19 passed; Lahiri self-check 9e-5″; 54/54 dasha systems |
| PyJHora book suite | 10,404 tests; 265 fail = float-repr / last-digit noise (mean-node baseline) |
| vedastro-svc | dotnet test 20 passed (Phase 9: +5 promotion-log tests); `dotnet format --verify-no-changes` clean; 0 vulnerable packages; 1,508 proved / 128 quarantined rules |
| Promotion log (Phase 9) | live: 2 `rule.validate` calls → `/v1/rule/promotions/verify` `ok:true, entries:2` on the native 7793 build (head `63b09b32…`) and again inside the Phase-8 `vedastro-svc` container via `docker exec` (head `7d2b689c…`); chain independently re-hashed in Python (prev_ok/hash_ok on every row); `DELETE /v1/rule/promotions` → 405 |
| Vendor pins (Phase 9) | `scripts/vendor.sh --from-git` into an empty scratch tree: 31 s, sentinels OK, `diff -rq` vs the zip-restored vendor/ = 0 differing paths for all three trees; second run skips everything (idempotent) |
| CI file (Phase 9) | `python3 yaml.safe_load` ok; `actionlint 1.7.12` (with shellcheck) 0 findings; `scripts/github-bootstrap.sh --dry-run` prints the create/watch/protect commands and executes nothing outward |
| Consensus corpus (10,000 charts, XALEN-DE440 vs Swiss) | **PASS** — all categories; byte-identical on re-run |
| Third-path cross-check (pyswisseph direct) | golden chart ≤ 1.2″ all bodies |
| Adversarial probes | malformed JSON, lat 95, year 3500, D99, unknown tool, 5 MB body (413), 50 concurrent — all fail closed; audit log contains no birth data |
| Gates | pre-commit 6 hooks all blocking and Passed; gitleaks history clean |

## Requirement status
| Requirement | Status |
|---|---|
| §3 chart.compute, panchang.day, dasha.timeline, transit.window, muhurta.find, rule.validate, engine.consensus, catalog.list, evidence block | VERIFIED COMPLETE |
| §3 match.kuta (VedAstro side-by-side), rectify.birth_time (multi-engine scoring) | PARTIALLY COMPLETE — single-engine implementations, labelled |
| §4 XALEN: DE440 mandatory, placeholders excluded, vendored/pinned, ΔT σ reported, node from DE440 state vector | VERIFIED COMPLETE |
| §4 XALEN: Chiron via seas_*.se1; per-body Pluto null | EXPLICITLY DEFERRED (Chiron not served; out-of-coverage charts refused whole) |
| §4 PyJHora: Lahiri forced + self-test, true nodes, no network, process pool, headless, AGPL isolated | VERIFIED COMPLETE |
| §4 VedAstro: Secrets→env, Azure removed, loopback, quarantine + rule.validate gate | VERIFIED COMPLETE; 103 rules lack public predicates (known gap) |
| §6 promotion log: append-only, hash-chained, fsync'd; `promotion_status` derived from the file; quarantine unaffected by promotion | VERIFIED COMPLETE (Phase 9, see evidence rows) — tail-truncation is detectable only against an external anchor (`head_hash`), stated in the README |
| §4 IronClaw: HTTP MCP endpoint reachable from the agent (loopback, TLS), local-dev, extension registered (`mcp-jyotish-local`, 10 tools), egress policy intact for non-loopback | VERIFIED COMPLETE (verifier re-executed 2026-09-17: `cargo test -p ironclaw_network` 24+1+17+17 passed on the patched fork at tag ironclaw-v1.4.0; patch == working-tree diff; brew binary untouched — env-var string absent; `JYOTISH_TLS=off cargo test` 37+3 passed incl. key-mode 0600 and SAN guards; https health OK with `certs/ironclaw-ca-bundle.pem`, verify-fail without it, plain http refused; two positive P7 turns + three negative controls) |
| §4 IronClaw: real-model agent turn, persona behaviour, routines (cron/heartbeat/reactive), identity memory docs | COMPLETE BUT NOT FULLY VERIFIED — turns so far use a stub LLM emitting the exact `tool_call`; provider anthropic / claude-sonnet-5 configured, no key stored (`ironclaw config list` shows only `api_key_env ANTHROPIC_API_KEY`); needs the owner's key or a tool-capable local model |
| §2 ops hardening (supervisor, OS users, egress-deny firewall) | NOT DONE |
| §7 CI on a host + branch protection | COMPLETE BUT NOT FULLY VERIFIED — ci.yml now restores vendor/ from pinned upstream commits (vendor/UPSTREAM-PINS.txt, all three exact matches), caches by pin, `permissions: contents: read`, `concurrency` cancel-in-progress, `compose-build` job; validated locally (yaml + actionlint) but never executed on GitHub: no remote exists. `scripts/github-bootstrap.sh --confirm` (owner-only) creates the private repo, pushes, waits for the first run and applies protection on main (6 required checks + 1 review); dry-run executed |
| Upstream items | XALEN gravitational deflection at solar conjunction (2.5″ band rule in force); VedAstro Library unbuildable upstream since 2023-09-28 (restored in Library.Trimmed) |

## Exact missing evidence to move up
1. → PRODUCTION-READY: (a) one real-model IronClaw turn (owner's `ANTHROPIC_API_KEY`, or a tool-capable local model that answers inside IronClaw's 180 s budget) that selects `mcp-jyotish-local__chart__compute`, forms the BirthInput itself and reports the result, plus one created routine (`builtin.trigger_create`) observed firing; (b) launchd/systemd supervision with `IRONCLAW_EGRESS_ALLOW_LOOPBACK=1` + `SSL_CERT_FILE` in the plist, separate OS users + egress-deny for sidecars; (c) one green CI run on GitHub with branch protection — run `scripts/github-bootstrap.sh --confirm` (creates the remote and pushes; `gh` is authenticated).
2. → PRODUCTION-VERIFIED: the above running unattended for at least one 04:00 daily-brief cycle with the nightly `engine.consensus` heartbeat green.

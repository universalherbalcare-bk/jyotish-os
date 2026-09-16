# JYOTISH-OS — Completion Ledger (T3)

**Verdict (independent verifier, 2026-09-17, commit 771da8c): READY WITH KNOWN RISKS.**
The engine stack (jyotish-mcp + XALEN-DE440 + jhora-svc + vedastro-svc, all 10 contract tools) carries production-ready evidence. The L3 agent layer now attaches: the verifier re-executed IronClaw 1.4.0 (opt-in loopback-egress patch, `~/.local/bin/ironclaw-jyotish`) turns that called `catalog.list` and the GOLDEN `chart.compute` over loopback TLS 1.3 and received the real results (Moon Swati 193.2923°, `consensus_status: PASS`), with matching `tls handshake` and audit lines on the jyotish-mcp side; flag unset → `policy_denied` with no connection; `builtin.http` to 192.168.1.1 / 10.0.0.5 with the flag set → `policy_denied`. Those turns were driven by a stub LLM that emits the exact `tool_call`: the runtime, capability grant, egress policy, TLS and MCP path are real and EXECUTED; the model is not. This satisfies the verifier's connectivity criterion ("an IronClaw agent turn calling a jyotish tool"); it does not prove real-model tool selection, argument formation, persona behaviour or routines — those need the owner's Anthropic key (provider is set to anthropic / claude-sonnet-5, no key stored) or a tool-capable local model, and stay UNVERIFIED.

Named risks accepted under this verdict: (1) IronClaw runs from a local fork build (`patches/ironclaw-1.4.0-allow-loopback-egress.patch`, 4 files); `brew upgrade` will not carry it and the opt-in exempts 127.0.0.0/8 + ::1 from the SSRF guard for that process only. (2) No real-model turn has been observed; with the anthropic provider, conversation content (including birth data the user types) leaves the machine — blueprint Option B, owner's choice. (3) §2 operational hardening (supervisor, separate OS users, egress-deny firewall) is not built; sidecar network isolation is code-level only. (4) CI has never run on a host (no git remote; the workflow and bootstrap script are ready and locally validated — Phase 9).

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

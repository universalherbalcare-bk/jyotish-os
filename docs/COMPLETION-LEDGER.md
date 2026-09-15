# JYOTISH-OS — Completion Ledger (T3)

**Verdict (independent verifier, 2026-09-16, commit dd24bef): CONDITIONALLY READY.**
The engine stack (jyotish-mcp + XALEN-DE440 + jhora-svc + vedastro-svc, all 10 contract tools) carries production-ready evidence; the L3 agent layer cannot attach under IronClaw 1.4.0's https-only / private-IP-deny policy — an external dependency requiring an owner decision.

## Evidence (all EXECUTED on this machine, re-run independently by the verifier)
| Suite | Result |
|---|---|
| XALEN `cargo test --workspace` (vendor) | 2,258 passed / 0 failed (builder run, all crates); 2,228 / 0 (verifier run, PyO3/napi cdylibs excluded) |
| DE440 benchmark (3,653 epochs) | Moon RMS 2.82″ max 11.5″ analytic; planets sub-arcsecond — reproduces BENCHMARK.md |
| jyotish-mcp | 31 unit + 3 integration passed; clippy `-D warnings` clean |
| jhora-svc | pytest 19 passed; Lahiri self-check 9e-5″; 54/54 dasha systems |
| PyJHora book suite | 10,404 tests; 265 fail = float-repr / last-digit noise (mean-node baseline) |
| vedastro-svc | dotnet test 15 passed; 0 vulnerable packages; 1,508 proved / 128 quarantined rules |
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
| §4 VedAstro: Secrets→env, Azure removed, loopback, quarantine + rule.validate gate | VERIFIED COMPLETE; promotion log NOT DONE; 103 rules lack public predicates (known gap) |
| §4 IronClaw: installed, local-dev, Ollama, persona, extension package | COMPLETE BUT NOT FULLY VERIFIED — BLOCKED on https-only MCP endpoint; no agent turn observed (qwen3.6 too slow) |
| §2 ops hardening (supervisor, OS users, egress-deny firewall) | NOT DONE |
| §7 CI on a host + branch protection | BLOCKED — no git remote yet (`gh` authenticated) |
| Upstream items | XALEN gravitational deflection at solar conjunction (2.5″ band rule in force); VedAstro Library unbuildable upstream since 2023-09-28 (restored in Library.Trimmed) |

## Exact missing evidence to move up
1. → READY WITH KNOWN RISKS: one observed IronClaw agent turn calling a `jyotish` tool (owner chooses: self-signed https on loopback / patch IronClaw / non-loopback front) + a local model ≤ ~12 GB that answers inside IronClaw's timeout.
2. → PRODUCTION-READY: launchd/systemd supervision, separate OS users + egress-deny for sidecars, rule-promotion log, one green CI run on GitHub with branch protection.

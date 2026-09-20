# JYOTISH-OS

One orchestrated system compiled from four upstream projects — **XALEN Ephemeris** (positions, DE440), **PyJHora**
(classical Vedic technique), **VedAstro** (rule base + real-outcome datasets) and **IronClaw** (secure local agent) —
behind a single typed MCP surface, with a fail-closed cross-engine consensus gate and executed evidence for every claim.

```
IronClaw (launchd, loopback-egress patch) ──TLS──▶ jyotish-mcp :7791  (Rust, XALEN + DE440 kernel, 10 tools, audit, cache)
                                                   ├─internal net──▶ jhora-svc :7792    (PyJHora + Swiss .se1, 54 dasha systems)
                                                   └─internal net──▶ vedastro-svc :7793 (1,508 rules, 15,807-row datasets, promotion log)
```

## Control plane — start here
```bash
scripts/jyotish-os up        # docker daemon → compose stack → launchd agents → IronClaw ext → smoke
scripts/jyotish-os status    # every wire, probed live (OK / FAIL / UNKNOWN — never assumed)
scripts/jyotish-os verify    # decisive checks, exit 1 on any failure
scripts/jyotish-os map       # regenerate docs/SYSTEM-MAP.md from live introspection
scripts/jyotish-os bench     # latency / bottleneck measurements → validation/bench-latest.md
```

## Where everything is
| Layer | Path | What it is |
|---|---|---|
| Contract | [docs/CONTRACT.md](docs/CONTRACT.md) | binding config (Lahiri, true nodes, DE440), REST/MCP surface, consensus tolerances (measured) |
| Blueprint | [docs/JYOTISH-OS-BLUEPRINT.md](docs/JYOTISH-OS-BLUEPRINT.md) | the design, limitation-by-limitation removals |
| System map | [docs/SYSTEM-MAP.md](docs/SYSTEM-MAP.md) | feature → code → backend data → transport → live value (generated) |
| Ledger | [docs/COMPLETION-LEDGER.md](docs/COMPLETION-LEDGER.md) | requirement → status → evidence, verdict |
| MCP server | [services/jyotish-mcp](services/jyotish-mcp) | Rust; TLS; 10 tools; boot guards (kernel sha, cert SAN, key mode) |
| Vedic sidecar | [services/jhora-svc](services/jhora-svc) | PyJHora in a process pool; Lahiri forced + self-check; no network |
| Rules sidecar | [services/vedastro-svc](services/vedastro-svc) | VedAstro `Library.Trimmed` (upstream unbuildable since 2023 — restored), promotion log |
| Agent layer | [services/ironclaw-ext](services/ironclaw-ext) | IronClaw 1.4.0 extension, routines (`execution_contract`), persona, evidence logs |
| IronClaw patch | [patches/](patches) | opt-in loopback-egress exemption (127/8, ::1 only), built to `~/.local/bin/ironclaw-jyotish` |
| Deployment | [deploy/](deploy) | compose (uids 10001–10003, read-only, cap_drop ALL, internal net, `restart: always`), launchd units |
| Validation | [validation/](validation) | XALEN tests, DE440 bench, PyJHora suite, 10k-chart consensus corpus, runner |
| Gates | `.pre-commit-config.yaml`, `.github/workflows/ci.yml` | gitleaks, ruff, clippy -D warnings, dotnet format, vendor guard; CI restores vendor from exact upstream pins |

## Evidence in one line each (all executed; see the ledger for commands)
XALEN 2,258/0 tests · DE440 bench reproduced · jyotish-mcp 44/0 · jhora-svc 21/0 · vedastro-svc 20/0 · PyJHora 10,404 (precision-noise only)
· 10,000-chart XALEN-DE440-vs-Swiss consensus **PASS** (Moon 0.0065″, nodes 1.41″ via DE440 osculating node) · third-path pyswisseph ≤ 1.2″
· adversarial probes fail closed · sidecar egress `ENETUNREACH` · crash → restart 6 s · daemon restart → stack self-recovers
· IronClaw turn → containerized MCP over TLS `success` · scheduled routine fires observed.

## Owner-only actions (cannot be done by automation)
1. Claude as the model: `ironclaw config set anthropic.api_key && ironclaw models set-provider anthropic --model claude-sonnet-5 && launchctl kickstart -k gui/$(id -u)/com.jyotish-os.ironclaw`
2. GitHub + CI + branch protection: `scripts/github-bootstrap.sh --confirm`
3. macOS: allow the two `com.jyotish-os.*` agents under *System Settings → General → Login Items & Extensions → Allow in the Background*, and enable *Start Docker Desktop when you sign in* — otherwise nothing auto-starts after a reboot (observed 2026-09-20).

Licences: your code Apache-2.0/MIT-compatible; XALEN Apache-2.0; VedAstro MIT; IronClaw MIT/Apache-2.0; **PyJHora is AGPL-3.0 and runs only as an isolated service** (confirm with counsel before distribution).

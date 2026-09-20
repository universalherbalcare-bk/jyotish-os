# JYOTISH-OS — System Map (generated 2026-09-20T12:46:15Z by scripts/system-map.py from LIVE probes)

Regenerate with `scripts/jyotish-os map`. Cells reading UNKNOWN mean the probe did not answer at generation time — they are never filled from memory.

## Layers

```
IronClaw (launchd, loopback-egress patch) ──TLS──▶ jyotish-mcp :7791 (uid 10001, edge+sidecars nets)
                                                   ├─HTTP internal──▶ jhora-svc :7792 (uid 10002, no egress)  PyJHora + Swiss .se1
                                                   └─HTTP internal──▶ vedastro-svc :7793 (uid 10003, no egress) rules XML + datasets SQLite + promotion log
kernels/de440s.bsp (sha-pinned) ─ro─▶ jyotish-mcp     certs/ (CA + leaf, key 0600) ─ro─▶ jyotish-mcp     audit/ ◀─rw─ jyotish-mcp
```

## Wiring table (feature → code → backend data → transport → live value)

| Feature | Code | Backend data | Transport | Live now |
|---|---|---|---|---|
| chart.compute | services/jyotish-mcp/src/tools/chart.rs → engine.rs | kernels/de440s.bsp sha c1c7feeab882… (DE440, coverage JD [2396752.5, 2506352.5]) | in-process XALEN crates (vendor/xalen @ cc6edbec1f) | golden: PASS, Moon Swati, node de440-osculating |
| panchang.day | tools/panchang.rs | same kernel; sunrise/tithi/nakshatra/yoga/karana via xalen-vedic | in-process | served by chart engine (see bench) |
| transit.window | tools/transit.rs | same kernel; bisection on XALEN | in-process | — |
| rectify.birth_time | tools/rectify.rs | same kernel; Vimshottari from xalen-vedic; heuristic:true | in-process | — |
| catalog.list | tools/catalog.rs | static enum tables (placeholders excluded) | in-process | ayanamsa=['LAHIRI'] houses=21 vargas=16 excluded=['Gauquelin', 'QiMenDunJia', 'PullenSinusoidalRatio'] |
| engine.consensus | tools/consensus.rs | XALEN-DE440 vs jhora-svc /v1/positions at the sidecar's jd_tt/jd_ut; tolerances docs/CONTRACT.md | HTTP inside compose `sidecars` network | jhora up=True |
| dasha.timeline | tools/proxy.rs → jhora-svc /v1/dasha | vendor/pyjhora @ 48e57d29b4: horoscope/dhasa/{graha,raasi,annual} (54/54 systems); Swiss .se1 files (100 in container) | HTTP, internal network only | LAHIRI Δ=9.081104508368298e-05" true_nodes=True workers=4 |
| match.kuta | tools/proxy.rs → jhora-svc /v1/kuta | vendor/pyjhora horoscope/match/compatibility.py | HTTP, internal | — |
| muhurta.find | tools/proxy.rs → vedastro-svc /v1/muhurta/find | Library.Trimmed/XMLData/EventDataList.xml: proved 1018 (with predicate 1018), quarantined 10 | HTTP, internal | ok=True |
| rule.validate | tools/proxy.rs → vedastro-svc /v1/rule/validate | HoroscopeDataList.xml proved 490 (predicate 387); datasets rows=15807 (HuggingFace CSVs → SQLite); promotion log /data/rule-promotions.jsonl | HTTP, internal | promotion chain ok=True entries=4 |
| MCP transport | services/jyotish-mcp/src/{mcp,tls}.rs | certs/jyotish-mcp.crt (SAN IP:127.0.0.1) + certs/ironclaw-ca-bundle.pem | HTTPS 127.0.0.1:7791 only | tools advertised: 10/10 |
| audit | src/audit.rs | audit/jyotish-mcp.jsonl (host-mounted; hashes only, no birth data) | append-only file | 3 files |
| consensus corpus | validation/consensus (Rust) | validation/consensus-summary.md (10k charts) + compose profile `corpus` in-network | in-process + HTTP internal | see validation/README.md |
| IronClaw agent | ~/.local/bin/ironclaw-jyotish (fork patch patches/ironclaw-1.4.0-allow-loopback-egress.patch) | ~/.ironclaw/reborn (embedded libsql; extension jyotish-local → https://127.0.0.1:7791/mcp) | launchd com.jyotish-os.ironclaw → scripts/ironclaw-serve.sh | agent running; provider ollama/qwen3.6:latest |
| routines | services/ironclaw-ext/routines/*.json (execution_contract v1) | IronClaw trigger store (libsql) — fires call mcp-jyotish-local__* tools | scheduler inside serve | created on demand via builtin__trigger_create (see routines/README.md) |
| stack supervision | deploy/compose.yaml (restart: always) + scripts/stack-ensure.sh | Docker Desktop VM; images jyotish-os/*:local | launchd com.jyotish-os.stack (login + 5 min) | agent not running; compose: jhora-svc Up 2 minutes (healthy); jyotish-mcp Up 2 minutes (healthy); vedastro-svc Up 2 minutes (healthy) |
| CI / gates | .github/workflows/ci.yml, .pre-commit-config.yaml, scripts/vendor.sh --from-git | vendor/UPSTREAM-PINS.txt xalen@cc6edbec1f, pyjhora@48e57d29b4, vedastro@fcb4dede36 | GitHub Actions (no remote yet) | pre-commit local only |

## Reboot / login-load note (BTM)
On recent macOS, user LaunchAgents in `~/Library/LaunchAgents` load at login only if allowed under **System Settings → General → Login Items & Extensions → Allow in the Background**. On 2026-09-20 both agents (and unrelated Homebrew/other user agents) were found unloaded after a reboot; `sfltool dumpbtm` could not be read without privileges, so this cause is INFERRED, not confirmed. `scripts/jyotish-os up` re-asserts everything idempotently regardless.

## Owner-only actions still open
- Claude key: `ironclaw config set anthropic.api_key && ironclaw models set-provider anthropic --model claude-sonnet-5 && launchctl kickstart -k gui/$(id -u)/com.jyotish-os.ironclaw`
- GitHub: `scripts/github-bootstrap.sh --confirm`
- Docker Desktop → Settings → General → *Start Docker Desktop when you sign in* (user setting; needed for unattended reboots)


## Compiled from other astrology sessions (added 2026-09-20)
| Source | What it is | How it is wired | Evidence |
|---|---|---|---|
| `~/Projects/Vedic Astrology/kundali-webapp` (own git repo, 46 commits) | The earlier product: `kundali` engine package + FastAPI backend + Gujarati vanilla-JS UI; 22 engines; 6,908 backend + 216 frontend tests (per its README) | Started read-only by `scripts/kundali-up.sh` on 127.0.0.1:8000; never modified from this repo | Golden chart via `/api/chart`: Swiss Ephemeris, Lahiri 23.7202087°, true nodes, jd_ut identical to XALEN |
| `~/Projects/Vedic Astrology/claude-skill` (= `~/.claude/skills/vedic-astrology-supreme-intelligence-engine`) | The chat-side skill: 25 Python engines (11,769 LOC), golden charts GC001/GC002, Gujarati-only answer rule | Not executed here; its engines are the same code the webapp's `engines/` folder owns | listed, not re-verified |
| TrAgentic-Ai "Vedic time-of-day/lunar overlay" (trading project) | Astrology overlay embedded in a trading system | **Not pulled in** — belongs to a different project boundary; noted only | — |
| `services/console` (new) | Live console on 127.0.0.1:7790: one birth → XALEN-DE440, PyJHora/Swiss, Kundali/Swiss side by side with arcsecond deltas, consensus, panchang ×2, Vimshottari, raw JSON | `scripts/jyotish-os console` | Golden run: 190 ms total, consensus PASS; Kundali +12.2″ on every body = mean- vs true-equinox ayanamsa convention (labelled) |

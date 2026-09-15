# services/ironclaw-ext — Phase 5: IronClaw agent layer for Jyotish-OS

Everything here was produced against the **installed binary `ironclaw 1.4.0`** (Homebrew
bottle, 2026-09-16), not against the 1.3.0 docs tree. Where the binary and the docs differ,
the binary wins and the difference is called out. Raw command output for every claim is under
`logs/`.

## Contents

| path | purpose | status |
|---|---|---|
| `jyotish/` | extension package: `manifest.toml` (v3, `[mcp]` + 10 pinned tools), `schemas/`, `prompts/` | built; rejected by IronClaw (see Blocker) |
| `registry/jyotish.json` | registry-entry shape (`kind: mcp_server`, `auth: none`) mirroring `registry/mcp-servers/*.json` | built (no CLI consumes it in 1.4.0) |
| `identity/default-system.md` | the "Jyotish-OS astrologer" persona; installed as IronClaw's seeded system prompt | **installed** at `~/.ironclaw/reborn/local-dev/system/prompts/default-system.md` |
| `identity/{AGENTS,SOUL,IDENTITY,HEARTBEAT}.md` | memory-document identity files per `docs/capabilities/memory/identity.mdx` | written; must be loaded via `memory_write` (chat) — not done |
| `routines/*.json` + `routines/README.md` | daily 04:00 brief, Monday 06:00 muhurta scan, nightly consensus as 1.4.0 `trigger_create` records + exact prompts | written; not created (needs a live agent turn) |
| `config/activities.json` | activities for the weekly muhurta scan | written |
| `install.sh` | idempotent installer: onboard -> provider -> persona -> package -> `extension install` -> health probe; exits 2 at the IronClaw blocker | executed (`logs/install-sh-run.txt`) |
| `check.sh` + `tools/gen_schemas.py` | static gate: manifest/schemas/routines/persona consistency with `docs/CONTRACT.md` | executed, passes |
| `logs/` | verbatim outputs (tokens redacted) | — |

## Verified commands (all executed this session; exit codes in the logs)

```bash
brew install ironclaw                     # 1.4.0 bottle -> logs/brew-install.txt
ironclaw --version                        # ironclaw 1.4.0
ironclaw --help                           # logs/cli-surface.txt, logs/cli-surface-2.txt
ironclaw onboard --no-service < /dev/null # logs/onboard.txt  (non-interactive path from docs/onboard.mdx works)
ironclaw models set-provider ollama --model qwen3.6:latest   # logs/models.txt
ironclaw models status                    # default.provider: ollama, default.model: qwen3.6:latest
ironclaw status / config list / doctor    # logs/status-config.txt (doctor: 8 passed, 0 failed)
ironclaw extension search jyotish         # logs/ext-install-http.txt (rejected: https scheme)
ironclaw extension install jyotish --json # same rejection, exit 1
./install.sh                              # logs/install-sh-run.txt, exit 2 at the blocker
./check.sh                                # static gate, exit 0
```

### The real CLI surface (differs from the 1.3.0 docs)

- `ironclaw extension` has only `search | install | remove`. **There is no `activate`
  subcommand**: `install` installs *and* activates (observed: `"phase":"active"` in
  `logs/ext-install-https-variant.txt`). `install <ID>` takes an id from the local catalog,
  not a path.
- **There is no `routine` subcommand.** Scheduling is the builtin tool family
  `builtin.trigger_*` (see `routines/README.md`).
- `channels list`, `hooks list`, `logs` print "not implemented yet" (exit 1).
- `config set` routes secrets to the encrypted store and rejects positional secret values;
  `models set-provider` writes `[llm.default]` to `config.toml` (no key needed for Ollama).
- Onboarding non-interactively skips the provider prompt and the OS service; the master key
  is a cached dotfile under the profile dir (`.reborn-local-dev-secrets-master-key`), not a
  Keychain prompt in this session.

## Where things live on disk (binary-verified)

| what | path |
|---|---|
| IronClaw home | `~/.ironclaw/reborn` (`config.toml`, `providers.json`, `webui-token`) |
| local-dev storage root | `~/.ironclaw/reborn/local-dev/` — embedded libsql DB `reborn-local-dev.db` (+ `-wal`, `-shm`) |
| filesystem extension catalog | `~/.ironclaw/reborn/local-dev/system/extensions/<id>/manifest.toml` (scanned by `extension search`) |
| seeded system prompt (persona) | `~/.ironclaw/reborn/local-dev/system/prompts/default-system.md` (read on every turn; original kept as `.orig`) |
| identity memory docs (AGENTS/SOUL/IDENTITY/HEARTBEAT) | inside the DB-backed `/memory` mount — write with `builtin.memory_write` |
| workspace root for `run`/`repl` | `$IRONCLAW_REBORN_WORKSPACE_ROOT` or the current directory |

## Blocker: IronClaw 1.4.0 cannot reach `http://127.0.0.1:7791/mcp`

Three independent checks agree (source of 1.3.0 tree, strings of the 1.4.0 binary, and
execution):

1. **Manifest parse (EXECUTED).** `[mcp].server` is typed `HttpsEndpoint`
   (`crates/contracts/ironclaw_extension_contracts/src/recipe.rs`). The binary rejects the
   contract package with:
   `invalid https_endpoint id 'http://127.0.0.1:7791/mcp': must use the https scheme`
   (`logs/ext-install-http.txt`).
2. **Everything else in the package is valid (EXECUTED).** The same package with only the
   scheme changed to `https://127.0.0.1:7791/mcp` installs and activates with all ten tools
   model-visible (`logs/ext-install-https-variant.txt`, `"phase":"active"`). That variant was
   removed again (`logs/ext-restore-contract.txt`) because it cannot work either (next point)
   and is not the contract.
3. **Egress (OBSERVED in source, UNVERIFIED at runtime).** The hosted-MCP egress plan pins
   `scheme = https`, the manifest host, and `deny_private_ip_ranges: true`
   (`crates/extensions/ironclaw_extension_host/src/mcp.rs`), and the resolver rejects any
   target that resolves to a private or loopback IP
   (`crates/substrates/ironclaw_network/src/resolver.rs`: "network target resolves to a
   private or host-local IP"). TLS uses `rustls-tls-native-roots`, so a self-signed
   certificate would also need the system trust store. The WebUI's custom-MCP form rejects
   `localhost` client-side (`extensions.customMcpEndpointHttps` in the bundled JS). The
   `docs/extensions/mcp.mdx` note that stdio is rejected "because process-level egress
   controls have not landed yet" is the same design decision.

No `config.toml` key, profile (`local-dev-yolo` grants host filesystem/process access, not a
loopback-MCP exemption in the code read) or env var found in the binary lifts this.

**Options (decision for the project owner; none implemented here):**
- (a) Patch IronClaw: accept `http://` loopback in `HttpsEndpoint`/`HostedMcpEgressEndpoint`
  and exempt loopback from `deny_private_ip_ranges` for hosted MCP under `local-dev`. Source
  build needs Rust 1.96+ and Node 22 (README); the docs' rationale for rejecting stdio applies
  equally, so upstream may not accept it.
- (b) Front `jyotish-mcp` with a public `https://` name that resolves to a **non-private** IP
  (e.g. a tunnel) — this contradicts CONTRACT.md "loopback only" and Blueprint §2 rule 1/§6,
  and would send birth data through a third party. Not recommended.
- (c) Re-package the tool surface as a **WASM extension** (`docs/extensions/building-a-tool.md`)
  whose sandboxed HTTP allowlist targets the loopback server — needs checking whether the
  WASM egress path also denies private ranges (`crates/lanes/ironclaw_wasm`); not verified.
- (d) Keep IronClaw for chat/routines and call `jyotish-mcp` from a first-party tool executor
  (`crates/extensions/ironclaw_extension_support`) in a fork — same source-build cost as (a).

## Security check (item 6)

| claim | evidence | class |
|---|---|---|
| profile is `local-dev` | `ironclaw config get boot.profile` -> `local-dev`; `ironclaw status` | EXECUTED |
| embedded storage, no Postgres | `storage.backend (not set)`; DB files `~/.ironclaw/reborn/local-dev/reborn-local-dev.db{,-wal,-shm}`; `docs/capabilities/database.mdx` says `[storage]` is a startup failure under `local-dev` | EXECUTED + OBSERVED |
| LLM is local | `llm.default.provider_id ollama`, base URL `http://localhost:11434`, "no API key needed" (`models list ollama -v`) | EXECUTED |
| no telemetry / trace upload | `ironclaw traces status`: `enabled: false`, `endpoint: not configured`, `include message text: false`, `include tool payloads: false`, `queued envelopes: 0` | EXECUTED |
| nothing listening | `lsof` shows no ironclaw listener; OS service not installed (`status: service: not installed`) | EXECUTED |
| MCP calls go through host egress allowlisting | plan = manifest host only + `deny_private_ip_ranges` + 2 MiB body cap + 60 s timeout (`extension_host/src/mcp.rs`); credentials injected at the egress boundary (`docs/extensions/mcp.mdx`) | OBSERVED in source; **UNVERIFIED at runtime** (no successful agent turn) |
| secrets | none entered; no API keys anywhere in this directory; `webui-token` is IronClaw-generated and redacted in logs | EXECUTED |
| `channels`/`hooks`/`logs` inspection | "not implemented yet" in 1.4.0 | EXECUTED (limits what can be observed) |

## LLM turn timing (why nothing was exercised end-to-end)

`ironclaw run -m ...` with `qwen3.6:latest` failed: Ollama loads the model as **29 GB split
46 % CPU / 54 % GPU** on this 24 GiB machine (context 262144), a 16-token reply took **105 s**
(`logs/ollama-warmup.txt`), IronClaw's Ollama client retried after ~60 s per request and the
run aborted with `run did not reach a terminal state within 180s` (`logs/run-tool-call-test.txt`).
So: provider wiring is verified (requests reach `http://localhost:11434/api/chat`), but no
chat turn, tool call, trigger creation or persona behaviour was observed. A model that fits
in GPU memory (<= ~12 GB, e.g. an 8B quant) is needed for IronClaw's timeouts; changing the
model is the owner's call (`ironclaw models set --help`, or `JYOTISH_OLLAMA_MODEL=... ./install.sh`).

## What is NOT yet activated (honest list)

1. `jyotish` extension — **not installable** on IronClaw 1.4.0 with the contract endpoint (Blocker).
2. `jyotish-mcp` itself — not running (`curl http://127.0.0.1:7791/health` -> connection refused); built by another phase.
3. Routines — none created; needs a live agent turn (`routines/README.md`).
4. Identity memory docs (`AGENTS.md`, `SOUL.md`, `IDENTITY.md`, `HEARTBEAT.md`) — not written to memory; the persona *is* installed as the system prompt file, which is the path the binary actually reads.
5. OS service / WebUI — not installed, not started (`ironclaw service install`, `ironclaw serve`) — left to the owner; `serve` binds 127.0.0.1:3000 by default.
6. Telegram/Slack delivery — not installed (by design; REPL/WebUI only).
7. End-to-end agent behaviour with `qwen3.6:latest` — blocked by hardware/timeouts above.

## Remove

`./install.sh --remove` removes the package from the catalog dir, `ironclaw extension remove
jyotish` if installed, and restores `default-system.md.orig`. IronClaw config (`ollama` /
`qwen3.6:latest`) is left as is.

## Reproducing the binary-string evidence

`strings "$(readlink -f "$(command -v ironclaw)")" | grep -n 'must use the https scheme\|builtin.trigger_create\|customMcpEndpointHttps'`
(the full dump is 158k lines and is not kept in the repo).

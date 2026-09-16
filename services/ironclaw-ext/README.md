# services/ironclaw-ext — Phase 5/6: IronClaw agent layer for Jyotish-OS

Everything here was produced against the **installed binary `ironclaw 1.4.0`** (Homebrew
bottle, 2026-09-16), not against the 1.3.0 docs tree. Where the binary and the docs differ,
the binary wins and the difference is called out. Raw command output for every claim is under
`logs/` (Phase 5: original files; Phase 6: `logs/p6-*.txt`).

## Contents

| path | purpose | status |
|---|---|---|
| `jyotish/` | extension package: `manifest.toml` (v3, `[mcp]` at **`https://127.0.0.1:7791/mcp`** + 10 pinned tools), `schemas/`, `prompts/` | **installed and active** on 1.4.0 (`logs/p6-ext-install-https.txt`) |
| `registry/jyotish.json` | registry-entry shape (`kind: mcp_server`, `auth: none`, https URL) mirroring `registry/mcp-servers/*.json` | built (no CLI consumes it in 1.4.0) |
| `identity/default-system.md` | the "Jyotish-OS astrologer" persona; installed as IronClaw's seeded system prompt | **installed** at `~/.ironclaw/reborn/local-dev/system/prompts/default-system.md` |
| `identity/{AGENTS,SOUL,IDENTITY,HEARTBEAT}.md` | memory-document identity files per `docs/capabilities/memory/identity.mdx` | written; must be loaded via `memory_write` (chat) — not done |
| `routines/*.json` + `routines/README.md` | daily 04:00 brief, Monday 06:00 muhurta scan, nightly consensus as 1.4.0 `trigger_create` records + exact prompts | written; not created (needs a live agent turn) |
| `config/activities.json` | activities for the weekly muhurta scan | written |
| `install.sh` | idempotent installer: onboard -> provider (only if unset) -> persona -> remove-old -> package -> `extension install` -> TLS health probe | executed twice, active both times (`logs/p6-install-sh-run.txt`) |
| `check.sh` + `tools/gen_schemas.py` | static gate: manifest/schemas/routines/persona consistency with `docs/CONTRACT.md` | executed, passes |
| `tools/stub_ollama.py` | deterministic Ollama-shaped stub LLM (loopback) that issues one `tool_call` — used to drive IronClaw's dispatcher without a real model | executed (`logs/p6-run-attempt3*.txt`) |
| `logs/` | verbatim outputs (no secrets present; checked) | — |

## TLS on loopback (Phase 6) — what now works

IronClaw types `[mcp].server` as an https-only endpoint, so `jyotish-mcp` now serves HTTPS
(`services/jyotish-mcp/README.md` "TLS"). Steps, all executed 2026-09-16 from the repo root:

```bash
scripts/gen-cert.sh          # local CA (10 y) + server leaf CN=jyotish-mcp, SAN IP:127.0.0.1,DNS:localhost (825 d); keys 0600; idempotent
scripts/make-ca-bundle.sh    # certs/ironclaw-ca-bundle.pem = 158 macOS system roots (read-only export) + certs/jyotish-ca.crt, then 3 checks
nohup services/jyotish-mcp/target/release/jyotish-mcp > audit/server.log 2>&1 &   # TLS on by default
cd services/ironclaw-ext && ./install.sh                                          # extension active, 10 tools
```

`make-ca-bundle.sh` output (EXECUTED): `openssl verify -CAfile certs/ironclaw-ca-bundle.pem
certs/jyotish-mcp.crt: OK`; `curl --cacert bundle https://127.0.0.1:7791/health` returned the
health JSON; `curl --cacert bundle https://www.anthropic.com` returned **HTTP 200** — the public
roots survived, which matters because rustls-native-certs treats `SSL_CERT_FILE` as a
*replacement* for the platform store, not an addition.

### Where the IronClaw process gets `SSL_CERT_FILE`

IronClaw's HTTP client is reqwest with `rustls-tls-native-roots`; the 1.4.0 binary contains
the `SSL_CERT_FILE` / `SSL_CERT_DIR` loader strings (`strings` on the bottle, see below).
There is no `config.toml` key for a CA file (`ironclaw config set --help`: keys route to
config.toml, the secret store or the WebUI token only), so it is an environment variable of
the process:

| how IronClaw is started | where to set it | status |
|---|---|---|
| `ironclaw run` / `ironclaw repl` / `ironclaw serve` from a shell | `export SSL_CERT_FILE=/Users/brijesh/Projects/jyotish-os/certs/ironclaw-ca-bundle.pem` (install.sh prints this line) | used for every Phase-6 run |
| launchd service (`ironclaw service install` → `~/Library/LaunchAgents/com.ironclaw.reborn.plist`) | add `<key>EnvironmentVariables</key><dict><key>SSL_CERT_FILE</key><string>…/certs/ironclaw-ca-bundle.pem</string></dict>` to the plist, then `ironclaw service restart` | service not installed (owner's call); the plist path is what `ironclaw service status` reports |

`SSL_CERT_FILE` never carries a secret: the bundle is public certificates only.

## What was observed (EXECUTED, 2026-09-16)

1. **Manifest with `https://127.0.0.1:7791/mcp` installs and activates**: `"phase":"active"`,
   `visible_capability_ids` = all ten `jyotish.*` tools (`logs/p6-ext-install-https.txt`).
2. **TLS transport works for any client that trusts the local CA**: curl `tools/list` over
   https lists the ten contract tools; `audit/server.log` shows
   `{"msg":"tls handshake","protocol":"TLSv1_3","cipher":"TLS13_CHACHA20_POLY1305_SHA256",...}`
   and, for a client without the CA, `{"msg":"tls handshake failed","error":"received fatal
   alert: UnknownCA"}`.
3. **IronClaw dispatches the tool but its egress policy refuses loopback before connecting.**
   With the extension active, `SSL_CERT_FILE` set and a stub model that answers
   `tool_call(name="jyotish__catalog__list", arguments="{}")` (`tools/stub_ollama.py`, provider
   `ollama` pointed at it via `OLLAMA_BASE_URL=http://127.0.0.1:11435`), the turn completed in
   0.5 s and the tool result was:
   ```
   {"detail":{"detail":"The capability runtime did not provide additional diagnostic detail.",
    "failure_kind":"network_denied","kind":"generic_failure"},"recovery":{"recovery_hint":"revise_approach",
    "same_call_retry":"forbidden"},"schema_version":1,"status":"error","summary":"Capability failed with network_denied.",
    "trust":"untrusted_tool_output"}
   ```
   Debug log line: `ironclaw_host_runtime::production: capability invocation failed
   capability_id=jyotish.catalog.list error_kind="network_denied"`
   (`logs/p6-run-attempt3b-stub-debug.txt`). **No TLS handshake and no audit line** appeared
   on the jyotish-mcp side, i.e. the denial happens at IronClaw's resolver, before any socket
   is opened. Same result with `https://localhost:7791/mcp` as the manifest URL
   (`logs/p6-run-attempt4-localhost-stub-debug.txt`): hostname vs literal IP makes no
   difference — the 1.3.0 resolver denies after DNS resolution as well
   (`crates/substrates/ironclaw_network/src/resolver.rs`: "network target resolves to a
   private or host-local IP").
4. **`builtin.http` is denied the same way** (`logs/p6-run-attempt5-builtin-http-probe.txt`):
   `capability invocation failed capability_id=builtin.http error_kind="policy_denied"`, and the
   grant IronClaw 1.4.0 issued to the loop driver is printed verbatim in that debug log:
   `NetworkPolicy { allowed_targets: [NetworkTargetPattern { scheme: None, host_pattern: "*",
   port: None }], deny_private_ip_ranges: true, max_egress_bytes: None }`. That is the exact
   policy string: **`deny_private_ip_ranges: true`**, host-wildcard, no loopback exemption.
5. Nothing in 1.4.0 lifts it: no `config.toml` key, no `IRONCLAW_REBORN_*` env var among the 42
   the binary reads (`strings` dump; `IRONCLAW_REBORN_NETWORK_MODE` is the Docker-sandbox posture, not host egress), no profile switch
   (`local-dev-yolo` grants host filesystem/process access, not loopback egress).

So the https/cert half of the blocker is solved and verified; the private-IP egress half is an
IronClaw design decision that the manifest cannot change. **Per the Phase-6 brief, work stopped
here: IronClaw was not patched and jyotish-mcp was not bound to a non-loopback address.**

### Options (owner decision; none implemented)

- (a) **Patch IronClaw** (source build, Rust 1.96+/Node 22): exempt loopback from
  `deny_private_ip_ranges` for hosted-MCP + `builtin.http` under `local-dev`, or add a
  manifest/network-policy override. Everything else (https endpoint, cert trust, dispatch,
  tool naming `jyotish__catalog__list`) is proven to work up to that check.
- (b) A public https name resolving to a non-private IP (tunnel) — contradicts CONTRACT.md
  "loopback only" and sends birth data through a third party. Not recommended.
- (c) Re-package as a WASM extension whose sandboxed HTTP allowlist targets loopback — needs
  checking whether the WASM egress path shares the same resolver (it uses `ironclaw_network`
  too in 1.3.0; likely the same denial). Unverified.
- (d) Keep IronClaw for chat/routines; call jyotish-mcp from a first-party tool executor in a
  fork — same build cost as (a).

## Model: Anthropic Claude (set, key not entered)

```
$ ironclaw models set-provider anthropic --model claude-sonnet-5
Provider set to `anthropic`, model set to `claude-sonnet-5`
Saved to /Users/brijesh/.ironclaw/reborn/config.toml
Note: `anthropic` requires credentials. Set ANTHROPIC_API_KEY before running with this provider.
$ ironclaw models status
default.provider: anthropic
default.provider_known: yes
default.model: claude-sonnet-5
default.api_key_env: ANTHROPIC_API_KEY
```
(`logs/p6-models-set-anthropic.txt`; the catalog's own default `claude-sonnet-4-20250514` is
stale and was overridden by the explicit model id.) **No agent turn with Claude was run:** no
`ANTHROPIC_API_KEY` is set in this environment and the brief forbids entering or handling any
key. The provider is left on `anthropic` deliberately so the owner only has to add the key.

### Activate Claude (owner, one line, hidden prompt — never paste the key on the command line)

```bash
ironclaw config set anthropic.api_key      # prompts with input hidden; stored in IronClaw's encrypted secret store
ironclaw models status                     # confirm: default.provider anthropic / default.model claude-sonnet-5
```
`ironclaw config set` rejects a positional value for `<provider>.api_key` by design. For
maximum quality use `ironclaw models set claude-opus-5` (Opus is the stronger, slower and
pricier tier); `claude-sonnet-5` is the balanced default. Even with the key in place, the
first real `jyotish.*` call will hit the egress denial in "What was observed" item 3 until
option (a)/(c)/(d) is taken — chat, memory and routines work regardless.

## Why no real local model turn was observed

- `qwen2.5vl:7b` (present, 6 GB): Ollama answers `400 {"error":"registry.ollama.ai/library/
  qwen2.5vl:7b does not support tools"}` to IronClaw's first request
  (`logs/p6-run-attempt1.txt`) — a vision model without tool calling.
- `qwen3.6:latest` (present, 23 GB, tools-capable): on this 24 GiB machine Ollama loads it
  31–46 % on CPU. Measured warm: 19 prompt tokens in 18.9 s at the default 262144 context; with
  a temporary 8192-context alias (`ollama create`, no download, removed again) 1216 prompt
  tokens took 88 s (~14 tok/s prompt eval) — IronClaw's turn carries ~16 k tokens of tool
  schemas (`effective_catalog_schema_tokens=16459` in the debug log) and the CLI's completion
  poll is a hard-coded 180 s (`crates/app/ironclaw_cli/src/runtime/mod.rs`
  `PollSettings { max_total: Duration::from_secs(180) }` in 1.3.0; not configurable in 1.4.0).
  Not viable.
- Pulling a small tools-capable model (e.g. an ~2–5 GB Qwen3/Llama 3.x instruct) is a download
  and was left to the owner: `ollama pull <model> && ironclaw models set-provider ollama --model <model>`.
- The stub (`tools/stub_ollama.py`) was therefore used to drive the dispatcher; it is a test
  harness, not a model, and the README says so wherever its output is cited.

## Verified commands (all executed; exit codes in the logs)

```bash
ironclaw --version                                   # ironclaw 1.4.0
ironclaw extension install jyotish --json            # phase active, 10 tools (logs/p6-ext-install-https.txt)
./install.sh                                         # twice; idempotent (logs/p6-install-sh-run.txt)
./check.sh                                           # static gate, exit 0 (https URL asserted)
SSL_CERT_FILE=…/certs/ironclaw-ca-bundle.pem OLLAMA_BASE_URL=http://127.0.0.1:11435 \
  IRONCLAW_REBORN_LOG=debug ironclaw run -m "Call the jyotish.catalog.list tool …"   # network_denied (logs/p6-run-attempt3b-*.txt)
ironclaw models set-provider anthropic --model claude-sonnet-5 && ironclaw models status
```

### The real CLI surface (differs from the 1.3.0 docs)

- `ironclaw extension` has only `search | install | remove`; `install` installs *and* activates.
  `install <ID>` takes an id from the local catalog dir, not a path.
- IronClaw pins the manifest hash at install time. Overwriting the catalog copy before
  `extension remove` wedges every lifecycle command with "installation manifest hash does not
  match registered manifest hash"; `install.sh` therefore removes first, then copies. Recovery
  if it happens: put the previously installed manifest back, `ironclaw extension remove jyotish`.
- Extension tools are not in the model's direct tool list; the model sees the builtins plus
  `tool_search` / `tool_describe` / `tool_call`, and calls an extension capability as
  `tool_call(name="jyotish__catalog__list", arguments="{}")` (dots become `__`).
- **There is no `routine` subcommand.** Scheduling is the builtin tool family
  `builtin.trigger_*` (see `routines/README.md`).
- `channels list`, `hooks list`, `logs` print "not implemented yet" (exit 1).
- `config set` routes secrets to the encrypted store and rejects positional secret values;
  `models set-provider` writes `[llm.default]` to `config.toml`.
- `IRONCLAW_REBORN_LOG=debug` (or `RUST_LOG=debug`) prints the capability grants and the
  `capability invocation failed … error_kind=` line for every tool call.

## Where things live on disk (binary-verified)

| what | path |
|---|---|
| IronClaw home | `~/.ironclaw/reborn` (`config.toml`, `providers.json`, `webui-token`) |
| local-dev storage root | `~/.ironclaw/reborn/local-dev/` — embedded libsql DB `reborn-local-dev.db` (+ `-wal`, `-shm`) |
| filesystem extension catalog | `~/.ironclaw/reborn/local-dev/system/extensions/<id>/manifest.toml` (scanned by `extension search`) |
| seeded system prompt (persona) | `~/.ironclaw/reborn/local-dev/system/prompts/default-system.md` (read on every turn; original kept as `.orig`) |
| identity memory docs | inside the DB-backed `/memory` mount — write with `builtin.memory_write` |
| workspace root for `run`/`repl` | `$IRONCLAW_REBORN_WORKSPACE_ROOT` or the current directory |
| local CA + server cert + IronClaw bundle | `<repo>/certs/` (gitignored; `scripts/gen-cert.sh`, `scripts/make-ca-bundle.sh`) |

## Security check

| claim | evidence | class |
|---|---|---|
| profile is `local-dev` | `ironclaw config path` -> `profile: local-dev` | EXECUTED |
| embedded storage, no Postgres | `storage.backend (not set)`; DB files under `~/.ironclaw/reborn/local-dev/` | EXECUTED |
| jyotish-mcp reachable only over TLS on loopback | `curl http://127.0.0.1:7791/health` -> `000` (TLS-only socket); `curl https://…` without `--cacert` -> curl exit 60 (`UnknownCA` on the server) | EXECUTED |
| server key permissions enforced | boot with a 0644 key -> exit 2 `"private key … has mode 0644; it must be 0600"`; cert without IP SAN -> exit 2 | EXECUTED |
| no keychain / trust-store mutation | only `security find-certificate -a -p` (export) is used; no `add-trusted-cert`, no sudo | OBSERVED (scripts) |
| MCP calls go through host egress allowlisting | `NetworkPolicy { … deny_private_ip_ranges: true }` printed by the 1.4.0 runtime; loopback call refused before connect | EXECUTED |
| secrets | none entered; `logs/p6-*.txt` grepped for `Bearer`, `sk-ant-`, `webui-token`: none | EXECUTED |
| telemetry | unchanged from Phase 5 (`traces status`: disabled) | EXECUTED (Phase 5) |

## What is NOT yet activated (honest list)

1. **A `jyotish.*` tool call completing through IronClaw** — refused by IronClaw 1.4.0's
   `deny_private_ip_ranges` egress policy (both `127.0.0.1` and `localhost`); needs option (a)/(c)/(d).
2. **A Claude agent turn** — provider set to `anthropic` / `claude-sonnet-5`, key not entered (owner: "Activate Claude").
3. Routines — none created; needs a live agent turn (`routines/README.md`).
4. Identity memory docs — not written to memory; the persona *is* installed as the system prompt file.
5. OS service / WebUI — not installed, not started (`ironclaw service install`, `ironclaw serve`); when installed, add `SSL_CERT_FILE` to the plist as above.
6. Telegram/Slack delivery — not installed (by design; REPL/WebUI only).

## Remove

`./install.sh --remove` removes the package from the catalog dir, `ironclaw extension remove
jyotish` if installed, and restores `default-system.md.orig`. IronClaw config (provider/model)
and `certs/` are left as is.

## Reproducing the binary-string evidence

`strings "$(readlink -f "$(command -v ironclaw)")" | grep -n 'must use the https scheme\|SSL_CERT_FILE\|private or host-local IP\|deny_private_ip_ranges\|builtin.trigger_create'`
(the full dump is 158k lines and is not kept in the repo).

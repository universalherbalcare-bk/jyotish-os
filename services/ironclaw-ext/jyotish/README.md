# jyotish — IronClaw hosted-MCP extension package (data-only)

Wraps the Jyotish-OS MCP server (`docs/CONTRACT.md`, `POST http://127.0.0.1:7791/mcp`,
JSON-RPC 2.0) as an IronClaw `reborn.extension_manifest.v3` package, in the exact shape of
the bundled `notion-mcp` / `nearai-mcp` packages:

- `manifest.toml` — `[mcp]` connection (server, namespace `jyotish`, `max_tools`, effects,
  origin gate matrix) + ten statically pinned `[[tools]]`, one per contract tool, in contract
  order. Tool ids are `jyotish.<contract name>` (IronClaw requires `<extension>.<capability>`).
- `schemas/jyotish/<tool>/<name>.input.v1.json` — JSON Schema 2020-12 inputs. `BirthInput`
  is the contract's `{utc, lat, lon, tz_offset_hours}` verbatim, as `$defs.BirthInput`.
- `prompts/jyotish/<tool>/<name>.md` — model-visible usage notes carrying the persona rules
  (cite `evidence.consensus_status`, flag `boundary_distance_sec < 120`, never certainty).

Regenerate schemas/prompts from the single source of truth: `python3 ../tools/gen_schemas.py`
(`--check` verifies no drift). No credentials: the server is loopback and unauthenticated.

Install location IronClaw scans (verified): `~/.ironclaw/reborn/local-dev/system/extensions/jyotish/`.
`../install.sh` copies it there and runs `ironclaw extension install jyotish`.

**Status:** rejected by IronClaw 1.4.0 at manifest parse — `[mcp].server` must be `https://`
and hosted-MCP egress denies loopback. See `../README.md` "Blocker" and the logs.

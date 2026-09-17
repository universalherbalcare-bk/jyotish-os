# Routines (IronClaw 1.4.0 scheduled triggers)

**Verified against the installed binary on 2026-09-17, not the 1.3.0 docs.** IronClaw 1.4.0 has no `routine`
CLI subcommand. Scheduling is done by the agent through builtin tools. The model-visible names are
`builtin__trigger_create`, `builtin__trigger_list`, and — surfaced only after `tool_search` — `builtin__trigger_remove`,
`builtin__trigger_pause`, `builtin__trigger_resume`, `builtin__trigger_run`.

## Payload shape (execution_contract — the only shape 1.4.0 accepts)
The three JSON files here are exact `builtin__trigger_create` payloads:
```json
{ "name": "...", "schedule": {"kind":"cron","expression":"0 4 * * *","timezone":"Asia/Kolkata"},
  "execution_contract": { "version": 1, "goal": "...", "success_criteria": ["..."],
    "output_instructions": "...", "no_result_text": "...",
    "policy": {"required_skills": null, "result_delivery": "deliver|suppress_when_nothing_to_report"} } }
```
Quirk observed: deferred tools return their own schema on the FIRST call with a `note` ("call the tool again");
a second identical call succeeds. When driving creation programmatically, issue the call twice.

## How to create (chat)
Paste into the WebUI/REPL: `Create a scheduled trigger with builtin__trigger_create using exactly this JSON, do not change any field:` followed by the file contents. Confirm with `Call builtin__trigger_list`.

## Firing evidence (EXECUTED 2026-09-17, stub-driven LLM, real scheduler)
Two every-minute test triggers (`jyotish-p10-fire-test`, ids `01M2PJD8Y5GD68TJS740XHMSKG`, `01M2PJD8Z62SG27PBJQ22XRBGK`)
were created at 02:16:25Z inside the launchd-supervised `ironclaw-jyotish serve`. Observed firings:
- `audit/jyotish-mcp.jsonl` (containerized MCP): `catalog.list ok` at 02:29:49.588Z, 02:29:49.629Z, 02:30:19.555Z, 02:30:19.555Z
- firing-stub transcript: requests whose user message is the rendered `## Goal` of the routine (not an interactive turn)
- serve stderr at INFO level carries no per-fire line (evidence rests on the two sources above).
Both triggers then removed with `builtin__trigger_remove` (`removed: true` each; `trigger_list` → empty); no further
`catalog.list` calls in the following 2 minutes. The LLM in those fires was the stub harness (`tools/stub_ollama.py`),
so this proves the scheduler → turn → capability → egress → TLS → MCP path, not model behaviour.

## Prerequisite for real routines
`serve` refuses to boot when the default provider is `anthropic` and no key is stored (`llm provider anthropic requires
API key env var ANTHROPIC_API_KEY`). Store the key first (`ironclaw config set anthropic.api_key`, hidden prompt), then
`ironclaw models set-provider anthropic --model claude-sonnet-5` and `launchctl kickstart -k gui/$(id -u)/com.jyotish-os.ironclaw`.

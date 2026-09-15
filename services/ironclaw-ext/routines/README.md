# Routines (IronClaw 1.4.0 "scheduled triggers")

**Verified against the installed binary, not the 1.3.0 docs:** `ironclaw 1.4.0` has **no `routine`
CLI subcommand** and no `routine_create` tool. Scheduling is done by the agent through the
builtin tools `builtin.trigger_create` / `trigger_list` / `trigger_pause` / `trigger_resume` /
`trigger_remove` / `trigger_run` / `trigger_status` (names read from the binary), and the record
it stores has exactly these fields: `name`, `schedule_kind` (`cron` | `once`),
`schedule_expression`, `schedule_timezone` (default `UTC`), `schedule_at` (for `once`),
`prompt`, `delivery_target`. The three JSON files here are those records, ready to paste.

The heartbeat path (`HEARTBEAT.md` read on a fixed interval, `runner.heartbeat_interval_secs`
= 15 in `config.toml`) exists in addition; `identity/HEARTBEAT.md` carries the nightly
consensus checklist for it.

## Files

| file | schedule (Asia/Kolkata) | what |
|---|---|---|
| `daily-brief.json` | `0 4 * * *` | panchang.day + transit.window (24 h) + dasha.timeline (vimshottari, depth 3) -> brief to memory `jyotish/brief/<date>.md` and the conversation |
| `weekly-muhurta.json` | `0 6 * * 1` | muhurta.find for every enabled activity in `config/activities.json`, 7-day window, top 3 windows with passed/vetoed rule ids |
| `nightly-consensus.json` | `30 2 * * *` | engine.consensus on the GOLDEN chart; reply `PASS` or `jyotish consensus FAIL` + raw deltas |

`delivery_target` is `null` = deliver to the originating conversation (REPL / WebUI).
Telegram/Slack delivery requires that channel extension to be installed and connected first;
then set `delivery_target` to an id from `builtin.outbound_delivery_targets_list`.

## How to create them (there is no non-interactive path in 1.4.0)

Start the agent (`ironclaw repl`, or `ironclaw serve` + WebUI) and paste, one at a time:

```
Create a scheduled trigger with builtin.trigger_create using exactly this JSON, do not change any field:
<paste the contents of routines/daily-brief.json>
```

Repeat for `weekly-muhurta.json` and `nightly-consensus.json`. Then verify:

```
Call builtin.trigger_list and show every trigger's name, schedule_expression, schedule_timezone and next_run_at.
```

Run one immediately to test: `Call builtin.trigger_run for the trigger named jyotish-daily-brief.`

Before the weekly scan can read the activity list, store it in memory once:

```
Call builtin.memory_write with path "jyotish/activities.json" and this exact content:
<paste the contents of config/activities.json>
```

And the heartbeat checklist (optional if you created `nightly-consensus.json`):

```
Call builtin.memory_write with path "HEARTBEAT.md" and this exact content:
<paste the contents of identity/HEARTBEAT.md>
```

## Not verified in this session

- No trigger was created: creating one needs a live agent turn, and agent turns time out on this
  machine with `qwen3.6:latest` (see ../README.md "LLM turn timing"). The JSON shapes are taken
  from the binary's `trigger_records` schema and tool list, not from an executed `trigger_create`.
- Whether `automation`-origin tool calls to `jyotish.*` are granted without a prompt depends on
  the operator grant (`origin_gate_matrix.automation = "gated_unless_granted"` in the manifest);
  expect one approval per tool the first time a trigger runs, via the WebUI approvals list.

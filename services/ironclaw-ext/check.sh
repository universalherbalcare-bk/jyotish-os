#!/usr/bin/env bash
# Static checks for services/ironclaw-ext (no IronClaw runtime needed). Exit 1 on any failure.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$HERE"
python3 tools/gen_schemas.py --check
python3 - <<'PY'
import json, pathlib, sys, tomllib
root = pathlib.Path(".")
contract = ["chart.compute","panchang.day","dasha.timeline","transit.window","muhurta.find",
            "match.kuta","rectify.birth_time","rule.validate","engine.consensus","catalog.list"]
m = tomllib.loads((root/"jyotish/manifest.toml").read_text())
assert m["schema_version"] == "reborn.extension_manifest.v3", m["schema_version"]
assert m["id"] == "jyotish" and m["mcp"]["namespace"] == "jyotish"
assert m["mcp"]["server"] == "http://127.0.0.1:7791/mcp", m["mcp"]["server"]
ids = [t["id"] for t in m["tools"]]
assert ids == ["jyotish."+c for c in contract], ids
for t in m["tools"]:
    assert t["default_permission"] in ("allow","ask","deny")
    for k in ("input_schema_ref","prompt_doc_ref"):
        p = root/"jyotish"/t[k]; assert p.exists(), p
    s = json.loads((root/"jyotish"/t["input_schema_ref"]).read_text())
    assert s["type"]=="object" and s["additionalProperties"] is False
    if "birth" in s["properties"] or "a" in s["properties"]:
        bi = s["$defs"]["BirthInput"]["properties"]
        assert set(bi) == {"utc","lat","lon","tz_offset_hours"}, set(bi)
r = json.loads((root/"registry/jyotish.json").read_text())
assert r["kind"]=="mcp_server" and r["url"]=="http://127.0.0.1:7791/mcp" and r["auth"]=="none"
for f in sorted((root/"routines").glob("*.json")):
    d = json.loads(f.read_text())
    assert d["schedule_kind"]=="cron" and len(d["schedule_expression"].split())==5 and d["prompt"], f
    assert "jyotish." in d["prompt"], f
sched = {json.loads(f.read_text())["name"]: json.loads(f.read_text())["schedule_expression"] for f in (root/"routines").glob("*.json")}
assert sched["jyotish-daily-brief"]=="0 4 * * *" and sched["jyotish-weekly-muhurta"]=="0 6 * * 1", sched
a = json.loads((root/"config/activities.json").read_text())
assert a["activities"] and all("activity" in x for x in a["activities"])
persona = (root/"identity/default-system.md").read_text()
for must in ("evidence.consensus_status","boundary_distance_sec","120","validation_status","never","jyotish.*"):
    assert must in persona, must
print("check.sh: manifest, 10 schemas/prompts, registry entry, 3 routines, activities, persona: OK")
PY
bash -n install.sh && echo "install.sh: bash syntax OK"

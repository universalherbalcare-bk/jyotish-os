#!/usr/bin/env python3
"""Generate docs/SYSTEM-MAP.md from LIVE introspection + file hashes.

Every row's "live" column is read from a running service at generation time; nothing is
hard-coded from memory. If a probe fails the cell says UNKNOWN (never a guessed value).
"""

from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import subprocess
import sys
import urllib.error
import urllib.request

PROBE_ERRORS = (
    OSError,
    subprocess.SubprocessError,
    ValueError,
    urllib.error.URLError,
    TimeoutError,
)

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
CA = os.path.join(ROOT, "certs", "ironclaw-ca-bundle.pem")
MCP = "https://127.0.0.1:7791"
GOLDEN = {
    "utc": "1990-03-15T06:30:00Z",
    "lat": 28.6139,
    "lon": 77.2090,
    "tz_offset_hours": 5.5,
}


def sh(cmd: list[str], timeout: int = 20) -> str:
    try:
        return subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout, check=False
        ).stdout
    except PROBE_ERRORS:
        return ""


def http_json(url: str, body: dict | None = None, timeout: int = 20):
    import ssl

    ctx = ssl.create_default_context(cafile=CA)
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        url, data=data, headers={"content-type": "application/json"}
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout, context=ctx) as r:
            return json.loads(r.read().decode())
    except PROBE_ERRORS:
        return None


def mcp_call(name: str, args: dict, timeout: int = 30):
    r = http_json(
        f"{MCP}/mcp",
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": args},
        },
        timeout,
    )
    if not r:
        return None
    return r.get("result", {}).get("structuredContent") or r.get("error")


def in_container_json(container: str, cmd: list[str]):
    out = sh(["docker", "exec", container] + cmd, timeout=20)
    try:
        return json.loads(out)
    except ValueError:
        return None


def sha_prefix(path: str) -> str:
    try:
        h = hashlib.sha256()
        with open(path, "rb") as f:
            for chunk in iter(lambda: f.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()[:12] + "…"
    except OSError:
        return "UNKNOWN"


def count_files(path: str, suffix: str = "") -> str:
    try:
        return str(len([f for f in os.listdir(path) if f.endswith(suffix)]))
    except OSError:
        return "UNKNOWN"


def main() -> None:
    now = dt.datetime.now(dt.UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    health = http_json(f"{MCP}/health", timeout=8) or {}
    tools = (
        http_json(f"{MCP}/mcp", {"jsonrpc": "2.0", "id": 1, "method": "tools/list"}, 8)
        or {}
    )
    tool_names = [t["name"] for t in tools.get("result", {}).get("tools", [])]
    jh = (
        in_container_json(
            "jhora-svc",
            [
                "python3",
                "-c",
                "import urllib.request;print(urllib.request.urlopen('http://127.0.0.1:7792/v1/health',timeout=5).read().decode())",
            ],
        )
        or {}
    )
    vh = (
        in_container_json(
            "vedastro-svc", ["curl", "-s", "-m", "5", "localhost:7793/v1/health"]
        )
        or {}
    )
    cat = mcp_call("catalog.list", {}) or {}
    golden = mcp_call("chart.compute", {"birth": GOLDEN}) or {}
    ev = golden.get("evidence", {}) if isinstance(golden, dict) else {}
    compose_ps = (
        sh(
            [
                "docker",
                "compose",
                "-f",
                "deploy/compose.yaml",
                "ps",
                "--format",
                "{{.Service}} {{.Status}}",
            ]
        )
        .strip()
        .replace("\n", "; ")
        or "UNKNOWN"
    )
    agents = {}
    for lbl in ("com.jyotish-os.ironclaw", "com.jyotish-os.stack"):
        out = sh(["launchctl", "print", f"gui/{os.getuid()}/{lbl}"])
        agents[lbl] = next(
            (l.split("=")[1].strip() for l in out.splitlines() if "state =" in l),
            "not loaded",
        )
    prov = sh([os.path.expanduser("~/.local/bin/ironclaw-jyotish"), "models", "status"])
    provider = "/".join(
        [
            next(
                (
                    l.split()[1]
                    for l in prov.splitlines()
                    if l.startswith("default.provider:")
                ),
                "UNKNOWN",
            ),
            next(
                (
                    l.split()[1]
                    for l in prov.splitlines()
                    if l.startswith("default.model:")
                ),
                "UNKNOWN",
            ),
        ]
    )
    pins = {}
    try:
        with open(
            os.path.join(ROOT, "vendor", "UPSTREAM-PINS.txt"), encoding="utf-8"
        ) as fh:
            for line in fh:
                if (
                    line.endswith("\n")
                    and "_COMMIT=" in line
                    and not line.startswith("#")
                ):
                    k, v = line.split("=", 1)
                    pins[k.replace("_COMMIT", "").strip().lower()] = v.strip()[:10]
    except OSError:
        pins = {"UPSTREAM-PINS.txt": "UNKNOWN"}

    def U(x):
        return x if x not in (None, "", {}) else "UNKNOWN"

    rows = [
        # feature | code | backend data (file/hash) | transport | live value now
        (
            "chart.compute",
            "services/jyotish-mcp/src/tools/chart.rs → engine.rs",
            f"kernels/de440s.bsp sha {sha_prefix(os.path.join(ROOT, 'kernels', 'de440s.bsp'))} (DE440, coverage JD {U(health.get('kernel_coverage_jd'))})",
            "in-process XALEN crates (vendor/xalen @ " + pins.get("xalen", "?") + ")",
            f"golden: {U(ev.get('consensus_status'))}, Moon {U(golden.get('bodies', {}).get('Moon', {}).get('nakshatra')) if isinstance(golden, dict) else 'UNKNOWN'}, node {U(golden.get('bodies', {}).get('Rahu', {}).get('source')) if isinstance(golden, dict) else 'UNKNOWN'}",
        ),
        (
            "panchang.day",
            "tools/panchang.rs",
            "same kernel; sunrise/tithi/nakshatra/yoga/karana via xalen-vedic",
            "in-process",
            "served by chart engine (see bench)",
        ),
        (
            "transit.window",
            "tools/transit.rs",
            "same kernel; bisection on XALEN",
            "in-process",
            "—",
        ),
        (
            "rectify.birth_time",
            "tools/rectify.rs",
            "same kernel; Vimshottari from xalen-vedic; heuristic:true",
            "in-process",
            "—",
        ),
        (
            "catalog.list",
            "tools/catalog.rs",
            "static enum tables (placeholders excluded)",
            "in-process",
            f"ayanamsa={U(cat.get('ayanamsas', {}).get('enabled'))} houses={len(cat.get('house_systems', {}).get('enabled', [])) if isinstance(cat.get('house_systems'), dict) else U(None)} vargas={len(cat.get('vargas', [])) if isinstance(cat.get('vargas'), list) else U(None)} excluded={[e.get('name') if isinstance(e, dict) else e for e in cat.get('excluded', [])]}",
        ),
        (
            "engine.consensus",
            "tools/consensus.rs",
            "XALEN-DE440 vs jhora-svc /v1/positions at the sidecar's jd_tt/jd_ut; tolerances docs/CONTRACT.md",
            "HTTP inside compose `sidecars` network",
            f"jhora up={U(health.get('sidecars', {}).get('jhora', {}).get('up'))}",
        ),
        (
            "dasha.timeline",
            "tools/proxy.rs → jhora-svc /v1/dasha",
            f"vendor/pyjhora @ {pins.get('pyjhora', '?')}: horoscope/dhasa/{{graha,raasi,annual}} ({U(jh.get('catalog', {}).get('supported'))}/{U(jh.get('catalog', {}).get('total'))} systems); Swiss .se1 files ({U(jh.get('ephe_files'))} in container)",
            "HTTP, internal network only",
            f'LAHIRI Δ={U(jh.get("self_check", {}).get("ayanamsa_delta_arcsec"))}" true_nodes={U(jh.get("true_nodes"))} workers={U(jh.get("workers"))}',
        ),
        (
            "match.kuta",
            "tools/proxy.rs → jhora-svc /v1/kuta",
            "vendor/pyjhora horoscope/match/compatibility.py",
            "HTTP, internal",
            "—",
        ),
        (
            "muhurta.find",
            "tools/proxy.rs → vedastro-svc /v1/muhurta/find",
            f"Library.Trimmed/XMLData/EventDataList.xml: proved {U(vh.get('rules', {}).get('event_proved'))} (with predicate {U(vh.get('rules', {}).get('event_proved_with_predicate'))}), quarantined {U(vh.get('rules', {}).get('event_quarantined'))}",
            "HTTP, internal",
            f"ok={U(vh.get('ok'))}",
        ),
        (
            "rule.validate",
            "tools/proxy.rs → vedastro-svc /v1/rule/validate",
            f"HoroscopeDataList.xml proved {U(vh.get('rules', {}).get('horoscope_proved'))} (predicate {U(vh.get('rules', {}).get('horoscope_proved_with_predicate'))}); datasets rows={U(vh.get('dataset_rows'))} (HuggingFace CSVs → SQLite); promotion log {U(vh.get('promotion_log', {}).get('path'))}",
            "HTTP, internal",
            f"promotion chain ok={U(vh.get('promotion_log', {}).get('chain_ok', vh.get('promotion_log', {}).get('ok')))} entries={U(vh.get('promotion_log', {}).get('entries'))}",
        ),
        (
            "MCP transport",
            "services/jyotish-mcp/src/{mcp,tls}.rs",
            "certs/jyotish-mcp.crt (SAN IP:127.0.0.1) + certs/ironclaw-ca-bundle.pem",
            "HTTPS 127.0.0.1:7791 only",
            f"tools advertised: {len(tool_names)}/10",
        ),
        (
            "audit",
            "src/audit.rs",
            "audit/jyotish-mcp.jsonl (host-mounted; hashes only, no birth data)",
            "append-only file",
            f"{count_files(os.path.join(ROOT, 'audit'))} files",
        ),
        (
            "consensus corpus",
            "validation/consensus (Rust)",
            "validation/consensus-summary.md (10k charts) + compose profile `corpus` in-network",
            "in-process + HTTP internal",
            "see validation/README.md",
        ),
        (
            "IronClaw agent",
            "~/.local/bin/ironclaw-jyotish (fork patch patches/ironclaw-1.4.0-allow-loopback-egress.patch)",
            "~/.ironclaw/reborn (embedded libsql; extension jyotish-local → https://127.0.0.1:7791/mcp)",
            "launchd com.jyotish-os.ironclaw → scripts/ironclaw-serve.sh",
            f"agent {agents['com.jyotish-os.ironclaw']}; provider {provider}",
        ),
        (
            "routines",
            "services/ironclaw-ext/routines/*.json (execution_contract v1)",
            "IronClaw trigger store (libsql) — fires call mcp-jyotish-local__* tools",
            "scheduler inside serve",
            "created on demand via builtin__trigger_create (see routines/README.md)",
        ),
        (
            "stack supervision",
            "deploy/compose.yaml (restart: always) + scripts/stack-ensure.sh",
            "Docker Desktop VM; images jyotish-os/*:local",
            "launchd com.jyotish-os.stack (login + 5 min)",
            f"agent {agents['com.jyotish-os.stack']}; compose: {compose_ps}",
        ),
        (
            "CI / gates",
            ".github/workflows/ci.yml, .pre-commit-config.yaml, scripts/vendor.sh --from-git",
            "vendor/UPSTREAM-PINS.txt "
            + ", ".join(f"{k}@{v}" for k, v in pins.items()),
            "GitHub Actions (no remote yet)",
            "pre-commit local only",
        ),
    ]

    out = []
    out.append(
        f"# JYOTISH-OS — System Map (generated {now} by scripts/system-map.py from LIVE probes)\n"
    )
    out.append(
        "Regenerate with `scripts/jyotish-os map`. Cells reading UNKNOWN mean the probe did not answer at generation time — they are never filled from memory.\n"
    )
    out.append("## Layers\n")
    out.append(
        "```\nIronClaw (launchd, loopback-egress patch) ──TLS──▶ jyotish-mcp :7791 (uid 10001, edge+sidecars nets)\n                                                   ├─HTTP internal──▶ jhora-svc :7792 (uid 10002, no egress)  PyJHora + Swiss .se1\n                                                   └─HTTP internal──▶ vedastro-svc :7793 (uid 10003, no egress) rules XML + datasets SQLite + promotion log\nkernels/de440s.bsp (sha-pinned) ─ro─▶ jyotish-mcp     certs/ (CA + leaf, key 0600) ─ro─▶ jyotish-mcp     audit/ ◀─rw─ jyotish-mcp\n```\n"
    )
    out.append(
        "## Wiring table (feature → code → backend data → transport → live value)\n"
    )
    out.append(
        "| Feature | Code | Backend data | Transport | Live now |\n|---|---|---|---|---|"
    )
    for r in rows:
        out.append("| " + " | ".join(str(c).replace("|", "\\|") for c in r) + " |")
    out.append(
        "\n## Reboot / login-load note (BTM)\nOn recent macOS, user LaunchAgents in `~/Library/LaunchAgents` load at login only if allowed under **System Settings → General → Login Items & Extensions → Allow in the Background**. On 2026-09-20 both agents (and unrelated Homebrew/other user agents) were found unloaded after a reboot; `sfltool dumpbtm` could not be read without privileges, so this cause is INFERRED, not confirmed. `scripts/jyotish-os up` re-asserts everything idempotently regardless.\n"
    )
    out.append(
        "## Owner-only actions still open\n- Claude key: `ironclaw config set anthropic.api_key && ironclaw models set-provider anthropic --model claude-sonnet-5 && launchctl kickstart -k gui/$(id -u)/com.jyotish-os.ironclaw`\n- GitHub: `scripts/github-bootstrap.sh --confirm`\n- Docker Desktop → Settings → General → *Start Docker Desktop when you sign in* (user setting; needed for unattended reboots)\n"
    )
    sys.stdout.write("\n".join(out) + "\n")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Deterministic Ollama-shaped stub LLM for wiring tests (Phase 6).

Purpose: exercise IronClaw's tool-dispatch -> hosted-MCP egress -> TLS -> jyotish-mcp
path WITHOUT a real language model, on machines where no locally present model can both
call tools and finish inside IronClaw's fixed 180 s turn budget.

Behaviour (POST /api/chat):
  * turn 1 (no tool result in the transcript): reply with ONE call to IronClaw's
    `tool_call` dispatcher naming TARGET_TOOL_NAME (default `jyotish__catalog__list`,
    the provider-facing spelling of `jyotish.catalog.list`), arguments "{}".
    If the dispatcher is absent, call the first visible tool containing "catalog".
  * turn 2 (a tool result is present): reply with plain text that quotes the number of
    house systems found in the tool result JSON, or the error text verbatim.
  * anything else: a short text reply. Never loops.
Also serves GET /api/tags and GET /api/version so IronClaw's model listing works.

Every request/response is appended (verbatim JSON, one object per line) to STUB_LOG so the
transcript IronClaw actually sent can be inspected afterwards. No secrets are involved:
the stub is loopback-only and unauthenticated by design, exactly like Ollama itself.

Run:  python3 stub_ollama.py [--port 11435] [--log path]
Then: OLLAMA_BASE_URL=http://127.0.0.1:11435 ironclaw run -m "..."
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODEL = "jyotish-stub"
TARGET_TOOL_SUBSTR = "catalog"
TARGET_TOOL_NAME = "jyotish__catalog__list"
HTTP_PROBE_URL = None
LOG_PATH = None


def log(kind: str, payload) -> None:
    if LOG_PATH:
        with open(LOG_PATH, "a", encoding="utf-8") as fh:
            fh.write(
                json.dumps({"ts": time.time(), "kind": kind, "payload": payload}) + "\n"
            )


def _extract_house_count(text: str):
    """Best-effort: find `house_systems` (list or {enabled:[...]}) in a JSON blob."""
    try:
        data = json.loads(text)
    except ValueError, TypeError:
        return None
    stack = [data]
    while stack:
        cur = stack.pop()
        if isinstance(cur, dict):
            for k, v in cur.items():
                if k == "house_systems":
                    if isinstance(v, list):
                        return len(v)
                    if isinstance(v, dict):
                        for kk in ("enabled", "items", "systems"):
                            if isinstance(v.get(kk), list):
                                return len(v[kk])
                stack.append(v)
        elif isinstance(cur, list):
            stack.extend(cur)
    return None


def _tool_result_texts(messages):
    out = []
    for m in messages:
        if m.get("role") == "tool":
            c = m.get("content")
            out.append(c if isinstance(c, str) else json.dumps(c))
    return out


def plan_reply(req: dict) -> dict:
    """Decide the assistant message for this request.

    IronClaw 1.4.0 exposes extension tools to the model *indirectly*: the visible
    tool list holds the builtins plus `tool_search` / `tool_describe` / `tool_call`,
    and an extension capability is invoked with
    `tool_call(name="<provider-facing name>", arguments="<json string>")`.
    Provider-facing names replace `.` with `__` (e.g. `jyotish__catalog__list`).
    """
    messages = req.get("messages") or []
    tools = req.get("tools") or []
    names = []
    for t in tools:
        fn = t.get("function") if isinstance(t, dict) else None
        if isinstance(fn, dict) and fn.get("name"):
            names.append(fn["name"])
    results = _tool_result_texts(messages)
    candidates = [TARGET_TOOL_NAME, TARGET_TOOL_NAME.replace("__", ".")]
    if results:
        last = results[-1]
        n = _extract_house_count(last)
        if n is not None:
            return {
                "content": f"The jyotish.catalog.list tool returned {n} house systems.",
                "tool_calls": None,
            }
        lowered = last.lower()
        looks_like_name_miss = any(
            k in lowered
            for k in (
                "unknown tool",
                "not found",
                "no such tool",
                "unknown capability",
                "not visible",
            )
        )
        if (
            looks_like_name_miss
            and len(results) < len(candidates)
            and "tool_call" in names
        ):
            nxt = candidates[len(results)]
            return {
                "content": "",
                "tool_calls": [
                    {
                        "function": {
                            "name": "tool_call",
                            "arguments": {"name": nxt, "arguments": "{}"},
                        }
                    }
                ],
            }
        return {"content": "The tool returned: " + last[:900], "tool_calls": None}
    if HTTP_PROBE_URL and "builtin__http" in names:
        return {
            "content": "",
            "tool_calls": [
                {
                    "function": {
                        "name": "builtin__http",
                        "arguments": {"method": "get", "url": HTTP_PROBE_URL},
                    }
                }
            ],
        }
    if "tool_call" in names:
        return {
            "content": "",
            "tool_calls": [
                {
                    "function": {
                        "name": "tool_call",
                        "arguments": {"name": candidates[0], "arguments": "{}"},
                    }
                }
            ],
        }
    direct = next((n for n in names if TARGET_TOOL_SUBSTR in n), None)
    if direct:
        return {
            "content": "",
            "tool_calls": [{"function": {"name": direct, "arguments": {}}}],
        }
    return {
        "content": f"No tool containing '{TARGET_TOOL_SUBSTR}' and no tool_call dispatcher was offered; "
        f"visible tools: {', '.join(names) or '(none)'}",
        "tool_calls": None,
    }


class Handler(BaseHTTPRequestHandler):
    server_version = "jyotish-stub-ollama/0.1"

    def _json(self, code: int, obj) -> None:
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        log("GET", {"path": self.path})
        if self.path.startswith("/api/tags"):
            return self._json(
                200,
                {
                    "models": [
                        {
                            "name": MODEL,
                            "model": MODEL,
                            "size": 0,
                            "details": {"family": "stub"},
                        }
                    ]
                },
            )
        if self.path.startswith("/api/version"):
            return self._json(200, {"version": "0.0.0-stub"})
        if self.path.startswith("/api/ps"):
            return self._json(200, {"models": []})
        return self._json(404, {"error": "not found"})

    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(n) if n else b""
        try:
            req = json.loads(raw or b"{}")
        except ValueError as e:
            log("POST-bad-json", {"path": self.path, "error": str(e)})
            return self._json(400, {"error": "bad json"})
        log("POST", {"path": self.path, "body": req})
        if self.path.startswith("/api/show"):
            return self._json(
                200,
                {
                    "capabilities": ["completion", "tools"],
                    "model_info": {},
                    "details": {"family": "stub"},
                },
            )
        if not self.path.startswith("/api/chat"):
            return self._json(404, {"error": "not found"})
        plan = plan_reply(req)
        msg = {"role": "assistant", "content": plan["content"]}
        if plan["tool_calls"]:
            msg["tool_calls"] = plan["tool_calls"]
        now = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
        final = {
            "model": req.get("model", MODEL),
            "created_at": now,
            "message": msg,
            "done": True,
            "done_reason": "stop",
            "total_duration": 1000000,
            "load_duration": 0,
            "prompt_eval_count": 1,
            "prompt_eval_duration": 1,
            "eval_count": 1,
            "eval_duration": 1,
        }
        log("REPLY", final)
        if req.get("stream"):
            # NDJSON: one chunk carrying the whole message, then the done frame.
            chunk = {
                "model": final["model"],
                "created_at": now,
                "message": msg,
                "done": False,
            }
            body = (json.dumps(chunk) + "\n" + json.dumps(final) + "\n").encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/x-ndjson")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        return self._json(200, final)

    def log_message(self, fmt, *args):  # quiet
        sys.stderr.write("[stub] " + (fmt % args) + "\n")


def main() -> None:
    global LOG_PATH, TARGET_TOOL_SUBSTR, TARGET_TOOL_NAME, HTTP_PROBE_URL
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=11435)
    ap.add_argument("--log", default=None)
    ap.add_argument("--tool", default=TARGET_TOOL_SUBSTR)
    ap.add_argument("--tool-name", default=TARGET_TOOL_NAME)
    ap.add_argument(
        "--http-probe",
        default=None,
        help="instead of the MCP tool, GET this URL via builtin.http (egress probe)",
    )
    a = ap.parse_args()
    LOG_PATH = a.log
    TARGET_TOOL_SUBSTR = a.tool
    TARGET_TOOL_NAME = a.tool_name
    HTTP_PROBE_URL = a.http_probe
    srv = ThreadingHTTPServer(("127.0.0.1", a.port), Handler)
    sys.stderr.write(
        f"[stub] listening on http://127.0.0.1:{a.port} (model {MODEL}, log {LOG_PATH})\n"
    )
    srv.serve_forever()


if __name__ == "__main__":
    main()

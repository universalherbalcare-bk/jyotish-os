# JYOTISH-OS — session state
Objective: build all 5 blueprint phases and run real tests (2026-09-16).
Decisions: project at ~/Projects/jyotish-os (own git repo); vendor/ gitignored + restorable via scripts/vendor.sh; contract in docs/CONTRACT.md.
Phase status: P1 xalen+jyotish-mcp ▢ · P2 jhora-svc ▢ · P3 vedastro-svc ▢ · P4 consensus+gates ▢ · P5 ironclaw ◐ (ironclaw 1.4.0 installed+configured ollama/qwen3.6, persona installed, ext pkg built; BLOCKED: 1.4.0 requires https + non-loopback MCP — see services/ironclaw-ext/README.md)
Next: agents dispatched for P1/P2/P3/P5-prep in parallel; P4 after P1+P2.

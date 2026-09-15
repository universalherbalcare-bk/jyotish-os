# JYOTISH-OS — session state
Objective: build all 5 blueprint phases and run real tests (2026-09-16).
Decisions: project at ~/Projects/jyotish-os (own git repo); vendor/ gitignored + restorable via scripts/vendor.sh; contract in docs/CONTRACT.md.
Phase status: P1 jyotish-mcp ✅ (5caaf96) · P2 jhora-svc ✅ (8d869e8) · P3 vedastro-svc ✅ (8aed3bb) · P4 consensus+gates ◐ (built+executed; corpus verdict FAIL by design — 4 UNDER REVIEW categories need lead decision, see docs/CONTRACT.md) · P5 ironclaw ◐ (installed; https-only MCP blocker, owner decision)
Next: await P3 + P4 → verifier agent → final ledger. Open decision: IronClaw https-only endpoint (3 options in services/ironclaw-ext/README.md).

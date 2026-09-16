# JYOTISH-OS — session state
Objective: build all 5 blueprint phases and run real tests (2026-09-16).
Decisions: project at ~/Projects/jyotish-os (own git repo); vendor/ gitignored + restorable via scripts/vendor.sh; contract in docs/CONTRACT.md; jyotish-mcp serves HTTPS on loopback (Phase 6: scripts/gen-cert.sh + scripts/make-ca-bundle.sh, certs/ gitignored, JYOTISH_TLS=off for tests).
Phase status: P1–P4 ✅ · P5–P7 ✅ (IronClaw over loopback TLS, opt-in patch) · Verifier: READY WITH KNOWN RISKS @ 771da8c (docs/COMPLETION-LEDGER.md)
Next: owner — store Claude key (ironclaw config set anthropic.api_key) for a real-model turn; then launchd supervision + OS-user separation + egress-deny, promotion log, git remote → CI + branch protection.

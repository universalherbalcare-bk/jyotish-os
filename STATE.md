# JYOTISH-OS — session state
Objective: build all 5 blueprint phases and run real tests (2026-09-16).
Decisions: project at ~/Projects/jyotish-os (own git repo); vendor/ gitignored + restorable via scripts/vendor.sh; contract in docs/CONTRACT.md; jyotish-mcp serves HTTPS on loopback (Phase 6: scripts/gen-cert.sh + scripts/make-ca-bundle.sh, certs/ gitignored, JYOTISH_TLS=off for tests).
Phase status: P1–P10 ✅ · control plane scripts/jyotish-os (up|status|verify|map|bench) ✅ · verdict READY WITH KNOWN RISKS (owner: key, push, BTM approval)
Next: owner — (1) Claude key; (2) scripts/github-bootstrap.sh --confirm; (3) approve com.jyotish-os.* in Login Items & Extensions + Docker Desktop start-at-login. Then re-run scripts/jyotish-os verify after a reboot.

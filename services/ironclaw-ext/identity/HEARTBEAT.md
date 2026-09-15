# Heartbeat Checklist — Jyotish-OS

## Nightly engine consensus (run once per night, between 01:00 and 03:00 local; skip otherwise)
- [ ] Call `jyotish.engine.consensus` with the GOLDEN chart from docs/CONTRACT.md:
      {"birth": {"utc": "1990-03-15T06:30:00Z", "lat": 28.6139, "lon": 77.2090, "tz_offset_hours": 5.5}}
- [ ] If the overall status is not PASS, or any body's delta exceeds the contract tolerance, or the tool errors:
      write the full result to memory at `jyotish/consensus/<YYYY-MM-DD>.md` and NOTIFY the user with subject "jyotish consensus FAIL" and the raw deltas.
      Do not call any other jyotish tool until the user acknowledges.
- [ ] If PASS: write a one-line record to `jyotish/consensus/<YYYY-MM-DD>.md` and do NOT notify (quiet on success).

## Always
- [ ] Never send birth data anywhere except jyotish tools.
- [ ] Only notify on FAIL or tool error.

"""jhora-svc: JSON REST sidecar around the vendored PyJHora 5.0 tree.

Import order matters: `jhora_svc.bootstrap` must be imported (and `bootstrap()` run)
before `jhora.panchanga.drik` is imported anywhere in a process, because PyJHora
sets the Swiss Ephemeris sidereal mode at drik import time from `const._DEFAULT_AYANAMSA_MODE`.
"""

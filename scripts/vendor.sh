#!/usr/bin/env bash
# Restores vendor/ from the four source archives. Idempotent. Verifies archive hashes first.
set -euo pipefail
cd "$(dirname "$0")/.."
DL="${SOURCE_ZIP_DIR:-$HOME/Downloads}"
shasum -a 256 -c vendor/SOURCE-ARCHIVES.sha256 2>/dev/null || { echo "archive hash mismatch or archives missing in $DL"; exit 1; }
T=$(mktemp -d)
for f in xalen-ephemeris-main PyJHora-main VedAstro-master; do unzip -q -o "$DL/$f.zip" -d "$T"; done
rm -rf vendor/xalen vendor/pyjhora vendor/vedastro
cp -R "$T/xalen-ephemeris-main" vendor/xalen
mkdir -p vendor/pyjhora && cp -R "$T/PyJHora-main/src" "$T/PyJHora-main/pyproject.toml" "$T/PyJHora-main/requirements.txt" "$T/PyJHora-main/LICENSE" "$T/PyJHora-main/README.md" vendor/pyjhora/
mkdir -p vendor/vedastro && cp -R "$T/VedAstro-master/Library" "$T/VedAstro-master/LibraryTests" "$T/VedAstro-master/HuggingFace" "$T/VedAstro-master/LICENSE.md" vendor/vedastro/
rm -rf "$T"; echo "vendor/ restored"

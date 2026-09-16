#!/usr/bin/env bash
# scripts/vendor.sh — restore vendor/{xalen,pyjhora,vedastro}. Idempotent. Never edits anything else.
#
#   scripts/vendor.sh              from the local source zips (vendor/SOURCE-ARCHIVES.sha256; $SOURCE_ZIP_DIR, default ~/Downloads)
#   scripts/vendor.sh --from-git   from upstream GitHub at the commits pinned in vendor/UPSTREAM-PINS.txt (what CI uses)
#   scripts/vendor.sh --check      verify the sentinel hashes of the current vendor/ against the pins; restore nothing
#
# --from-git fetches each pinned commit with `git fetch --depth 1 --filter=blob:none` (falls back to a full
# blobless fetch if the host refuses fetch-by-SHA), sparse-checks-out only the pinned subtree, copies it into
# vendor/<name>, installs the pinned vendor/xalen.Cargo.lock, then re-hashes the sentinel files. A sentinel
# mismatch is a hard failure (exit 1): the pins are the contract, not a hint. Already-matching trees are skipped.
set -euo pipefail
cd "$(dirname "$0")/.."
PINS=vendor/UPSTREAM-PINS.txt

usage() { sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; }

MODE=zip
case "${1:-}" in
  "") MODE=zip ;;
  --from-git) MODE=git ;;
  --check) MODE=check ;;
  -h|--help) usage; exit 0 ;;
  *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
esac

# ---------- pins ----------
pin() { # pin KEY -> value (first match)
  local v
  v=$(grep -E "^$1=" "$PINS" | head -n1 | cut -d= -f2-) || true
  [[ -n "$v" ]] || { echo "pin $1 missing in $PINS" >&2; exit 1; }
  printf '%s' "$v"
}
pins_multi() { grep -E "^$1=" "$PINS" | cut -d= -f2-; } # every line for KEY

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# check_sentinels NAME DIR -> 0 if every pinned sentinel hashes as pinned, 1 otherwise (prints the first mismatch)
check_sentinels() {
  local name=$1 dir=$2 line hash rel actual
  while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    hash=${line%% *}; rel=${line#* }; rel=${rel# }
    if [[ ! -f "$dir/$rel" ]]; then echo "  sentinel missing: $dir/$rel"; return 1; fi
    actual=$(sha256_of "$dir/$rel")
    if [[ "$actual" != "$hash" ]]; then echo "  sentinel mismatch: $dir/$rel expected $hash got $actual"; return 1; fi
  done < <(pins_multi "${name}_SENTINEL")
  return 0
}

check_ephe() { # PyJHora ephemeris presence + count
  local dir=$1 ephe count want
  ephe=$(pin PYJHORA_EPHE_DIR); want=$(pin PYJHORA_EPHE_COUNT)
  [[ -d "$dir/$ephe" ]] || { echo "  ephemeris dir missing: $dir/$ephe"; return 1; }
  count=$(find "$dir/$ephe" -type f | wc -l | tr -d ' ')
  [[ "$count" == "$want" ]] || { echo "  ephemeris file count $count != pinned $want in $dir/$ephe"; return 1; }
  return 0
}

verify_all() {
  local ok=0
  for name in XALEN PYJHORA VEDASTRO; do
    local lc; lc=$(echo "$name" | tr '[:upper:]' '[:lower:]')
    if check_sentinels "$name" "vendor/$lc"; then echo "vendor/$lc: sentinels OK ($(pin "${name}_COMMIT"))"; else echo "vendor/$lc: NOT at pin"; ok=1; fi
  done
  if check_ephe vendor/pyjhora; then echo "vendor/pyjhora: ephemeris OK ($(pin PYJHORA_EPHE_COUNT) files in $(pin PYJHORA_EPHE_DIR))"; else ok=1; fi
  if [[ -f vendor/xalen.Cargo.lock ]] && cmp -s vendor/xalen.Cargo.lock vendor/xalen/Cargo.lock; then echo "vendor/xalen/Cargo.lock: matches pinned vendor/xalen.Cargo.lock"; else echo "vendor/xalen/Cargo.lock: NOT the pinned lock"; ok=1; fi
  return $ok
}

# ---------- zip mode (original behaviour) ----------
restore_from_zip() {
  local DL="${SOURCE_ZIP_DIR:-$HOME/Downloads}" T
  shasum -a 256 -c vendor/SOURCE-ARCHIVES.sha256 2>/dev/null || { echo "archive hash mismatch or archives missing in $DL"; exit 1; }
  T=$(mktemp -d)
  for f in xalen-ephemeris-main PyJHora-main VedAstro-master; do unzip -q -o "$DL/$f.zip" -d "$T"; done
  rm -rf vendor/xalen vendor/pyjhora vendor/vedastro
  cp -R "$T/xalen-ephemeris-main" vendor/xalen
  mkdir -p vendor/pyjhora && cp -R "$T/PyJHora-main/src" "$T/PyJHora-main/pyproject.toml" "$T/PyJHora-main/requirements.txt" "$T/PyJHora-main/LICENSE" "$T/PyJHora-main/README.md" vendor/pyjhora/
  mkdir -p vendor/vedastro && cp -R "$T/VedAstro-master/Library" "$T/VedAstro-master/LibraryTests" "$T/VedAstro-master/HuggingFace" "$T/VedAstro-master/LICENSE.md" vendor/vedastro/
  rm -rf "$T"; echo "vendor/ restored from zips"
}

# ---------- git mode ----------
# fetch_pinned NAME DEST: materialise the pinned subtree of upstream NAME into DEST (a fresh temp dir)
fetch_pinned() {
  local name=$1 dest=$2 url commit paths
  url=$(pin "${name}_REPO"); commit=$(pin "${name}_COMMIT"); paths=$(pin "${name}_PATHS")
  [[ "$commit" =~ ^[0-9a-f]{40}$ ]] || { echo "$name: pinned commit is not a full 40-hex SHA: $commit" >&2; exit 1; }
  git init -q "$dest"
  git -C "$dest" remote add origin "$url"
  if ! git -C "$dest" fetch -q --depth 1 --filter=blob:none origin "$commit" 2>/dev/null; then
    echo "  $name: fetch-by-SHA refused, falling back to a full blobless fetch"
    git -C "$dest" fetch -q --filter=blob:none origin
  fi
  if [[ "$paths" != "/" ]]; then
    # shellcheck disable=SC2086
    git -C "$dest" sparse-checkout set --no-cone $paths
  fi
  git -C "$dest" checkout -q "$commit"
  [[ "$(git -C "$dest" rev-parse HEAD)" == "$commit" ]] || { echo "$name: checkout is not at $commit" >&2; exit 1; }
}

# install_tree NAME SRC -> vendor/<name>, keeping a local xalen target/ build cache aside so re-runs stay fast
install_tree() {
  local name=$1 src=$2 lc paths keep=""
  lc=$(echo "$name" | tr '[:upper:]' '[:lower:]'); paths=$(pin "${name}_PATHS")
  if [[ -d "vendor/$lc/target" ]]; then keep=$(mktemp -d); mv "vendor/$lc/target" "$keep/target"; fi
  rm -rf "vendor/$lc"; mkdir -p "vendor/$lc"
  if [[ "$paths" == "/" ]]; then
    (cd "$src" && tar --exclude=.git -cf - .) | (cd "vendor/$lc" && tar -xf -)
  else
    local p
    for p in $paths; do p=${p#/}; p=${p%/}; cp -R "$src/$p" "vendor/$lc/$p"; done
  fi
  if [[ -n "$keep" ]]; then mv "$keep/target" "vendor/$lc/target"; rmdir "$keep"; fi
}

restore_from_git() {
  command -v git >/dev/null || { echo "git is required for --from-git" >&2; exit 1; }
  [[ -f "$PINS" ]] || { echo "$PINS missing" >&2; exit 1; }
  T=$(mktemp -d); trap 'rm -rf "$T"' EXIT   # global on purpose: the EXIT trap runs outside this function's scope
  for name in XALEN PYJHORA VEDASTRO; do
    local lc; lc=$(echo "$name" | tr '[:upper:]' '[:lower:]')
    if [[ -d "vendor/$lc" ]] && check_sentinels "$name" "vendor/$lc" >/dev/null && { [[ "$name" != PYJHORA ]] || check_ephe vendor/pyjhora >/dev/null; }; then
      echo "vendor/$lc: already at $(pin "${name}_COMMIT") — skipped"
    else
      echo "vendor/$lc: fetching $(pin "${name}_REPO") @ $(pin "${name}_COMMIT")"
      fetch_pinned "$name" "$T/$lc"
      install_tree "$name" "$T/$lc"
      rm -rf "$T/$lc"
    fi
  done
  # xalen's Cargo.lock is gitignored upstream; install the pinned one so CI resolves the same crate versions as the verified local build.
  [[ -f vendor/xalen.Cargo.lock ]] || { echo "vendor/xalen.Cargo.lock missing" >&2; exit 1; }
  cp vendor/xalen.Cargo.lock vendor/xalen/Cargo.lock
  echo "--- verifying sentinels against $PINS"
  verify_all || { echo "vendor/ does NOT match the pins after restore — refusing to continue" >&2; exit 1; }
  echo "vendor/ restored from git pins"
}

case "$MODE" in
  zip) restore_from_zip ;;
  git) restore_from_git ;;
  check) verify_all ;;
esac

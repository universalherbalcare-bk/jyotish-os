#!/usr/bin/env bash
# scripts/github-bootstrap.sh — create the private GitHub repo, push main, wait for the first CI run,
# then lock main behind the CI status checks + 1 review. Every outward action is gated:
#
#   scripts/github-bootstrap.sh --dry-run            print every command; execute nothing outward
#   scripts/github-bootstrap.sh --confirm            run for real, after an interactive y/N prompt
#   scripts/github-bootstrap.sh --confirm --skip-create   remote already exists: only wait + protect
#
# Outward actions (blueprint §6b): gh repo create --push, gh api PUT branches/main/protection.
# Nothing here is run without both the --confirm flag AND a typed "y" at the prompt.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO_NAME="${GH_REPO_NAME:-jyotish-os}"
BRANCH="${GH_BRANCH:-main}"
# Job ids in .github/workflows/ci.yml — these are the status-check contexts GitHub reports.
CHECKS=(gates jyotish-mcp jhora-svc vedastro-svc consensus compose-build)
REQUIRED_REVIEWS="${GH_REQUIRED_REVIEWS:-1}"

DRY=0; CONFIRM=0; SKIP_CREATE=0
for a in "$@"; do
  case "$a" in
    --dry-run) DRY=1 ;;
    --confirm) CONFIRM=1 ;;
    --skip-create) SKIP_CREATE=1 ;;
    -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $a" >&2; exit 2 ;;
  esac
done
if [[ $DRY -eq 0 && $CONFIRM -eq 0 ]]; then
  echo "refusing: pass --dry-run to preview, or --confirm to execute outward actions" >&2; exit 2
fi
if [[ $DRY -eq 1 && $CONFIRM -eq 1 ]]; then DRY=1; CONFIRM=0; fi   # dry-run always wins

run() { # run CMD... : print, and execute only when confirmed
  if [[ $DRY -eq 1 ]]; then printf '[dry-run]'; printf ' %q' "$@"; echo; else "$@"; fi
}

# ---------- preflight (read-only; safe in dry-run) ----------
command -v gh >/dev/null || { echo "gh CLI not installed" >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "gh is not authenticated (gh auth login)" >&2; exit 1; }
OWNER=$(gh api user --jq .login)
test "$(git rev-parse --abbrev-ref HEAD)" = "$BRANCH" || { echo "checkout $BRANCH first (on $(git rev-parse --abbrev-ref HEAD))" >&2; exit 1; }
if [[ -n "$(git status --porcelain)" ]]; then
  echo "working tree is not clean — commit or stash first; nothing was pushed" >&2
  [[ $DRY -eq 1 ]] || exit 1
fi
# Leak guard. The only tracked .jsonl allowed are the Phase-7 IronClaw evidence logs (audit lines carry
# request hashes only; stub transcripts are synthetic; grepped for sk-ant-/Bearer <token>/token values: 0 hits).
LEAKS=$(git ls-files | grep -Ei '(\.key|\.pem|\.bsp|\.jsonl|\.db)$' | grep -Ev '^services/ironclaw-ext/logs/[^/]+\.(audit|stub)\.jsonl$' || true)
if [[ -n "$LEAKS" ]]; then
  echo "tracked files match secret/kernel/log/db patterns — refusing:" >&2
  echo "$LEAKS" >&2
  exit 1
fi
echo "owner=$OWNER repo=$REPO_NAME branch=$BRANCH checks=${CHECKS[*]} reviews=$REQUIRED_REVIEWS mode=$([[ $DRY -eq 1 ]] && echo dry-run || echo EXECUTE)"

PROTECTION_JSON=$(python3 - "$REQUIRED_REVIEWS" "${CHECKS[@]}" <<'EOF'
import json, sys
reviews = int(sys.argv[1]); checks = sys.argv[2:]
print(json.dumps({
    "required_status_checks": {"strict": True, "contexts": checks},
    "enforce_admins": True,
    "required_pull_request_reviews": {"required_approving_review_count": reviews, "dismiss_stale_reviews": True},
    "restrictions": None,
    "required_linear_history": True,
    "allow_force_pushes": False,
    "allow_deletions": False,
    "required_conversation_resolution": True,
}, indent=2))
EOF
)

# ---------- confirm gate ----------
if [[ $CONFIRM -eq 1 ]]; then
  echo "Action  : gh repo create $OWNER/$REPO_NAME --private --push (skip=$SKIP_CREATE); then branch protection on $BRANCH"
  echo "Target  : github.com/$OWNER/$REPO_NAME"
  echo "Effect  : the whole tracked tree ($(git ls-files | wc -l | tr -d ' ') files) leaves this machine; main becomes PR-only"
  echo "Reverse : repo can be deleted (gh repo delete); protection can be removed (gh api -X DELETE .../protection)"
  read -r -p "proceed? [y/N] " ans
  [[ "$ans" == "y" || "$ans" == "Y" ]] || { echo "aborted; nothing outward was executed"; exit 1; }
fi

# ---------- (1) create + push ----------
if [[ $SKIP_CREATE -eq 0 ]]; then
  run gh repo create "$REPO_NAME" --private --source . --remote origin --push
else
  echo "skip-create: assuming origin already points at github.com/$OWNER/$REPO_NAME"
fi

# ---------- (2) wait for the first CI run ----------
if [[ $DRY -eq 1 ]]; then
  echo "[dry-run] gh run list --repo $OWNER/$REPO_NAME --branch $BRANCH --workflow ci --limit 1 --json databaseId --jq '.[0].databaseId'   # poll until a run id appears"
  echo "[dry-run] gh run watch --repo $OWNER/$REPO_NAME <run-id> --exit-status"
else
  RUN_ID=""
  for _ in $(seq 1 60); do
    RUN_ID=$(gh run list --repo "$OWNER/$REPO_NAME" --branch "$BRANCH" --workflow ci --limit 1 --json databaseId --jq '.[0].databaseId' 2>/dev/null || true)
    [[ -n "$RUN_ID" && "$RUN_ID" != "null" ]] && break
    sleep 5
  done
  [[ -n "$RUN_ID" && "$RUN_ID" != "null" ]] || { echo "no CI run appeared within 5 minutes; protection NOT applied (contexts would not exist yet)" >&2; exit 1; }
  gh run watch --repo "$OWNER/$REPO_NAME" "$RUN_ID" --exit-status || echo "WARNING: first CI run is red — protection is still applied so nothing can merge until it is green"
fi

# ---------- (3) branch protection ----------
echo "--- protection payload"; echo "$PROTECTION_JSON"
if [[ $DRY -eq 1 ]]; then
  echo "[dry-run] gh api -X PUT repos/$OWNER/$REPO_NAME/branches/$BRANCH/protection -H 'Accept: application/vnd.github+json' --input <payload above>"
  echo "[dry-run] gh api repos/$OWNER/$REPO_NAME/branches/$BRANCH/protection   # (4) print resulting protection JSON"
else
  echo "$PROTECTION_JSON" | gh api -X PUT "repos/$OWNER/$REPO_NAME/branches/$BRANCH/protection" -H 'Accept: application/vnd.github+json' --input - >/dev/null
  # ---------- (4) print the result ----------
  gh api "repos/$OWNER/$REPO_NAME/branches/$BRANCH/protection"
fi
echo "done ($([[ $DRY -eq 1 ]] && echo 'dry-run: nothing outward executed' || echo 'executed'))"

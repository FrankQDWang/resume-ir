#!/usr/bin/env sh
set -eu

OWNER="${1:-FrankQDWang}"
REPO="${2:-resume-ir}"
FULL_NAME="$OWNER/$REPO"

if ! command -v gh >/dev/null 2>&1; then
  echo "gh is required to configure GitHub" >&2
  exit 1
fi

gh auth status -h github.com >/dev/null

if ! gh repo view "$FULL_NAME" >/dev/null 2>&1; then
  gh repo create "$FULL_NAME" --public --source=. --remote=origin --description "Local-first resume search engine" --disable-wiki
fi

if ! git remote get-url origin >/dev/null 2>&1; then
  git remote add origin "https://github.com/$FULL_NAME.git"
fi

./scripts/ci/guard-public-repo.sh
git push -u origin main

gh repo edit "$FULL_NAME" \
  --enable-issues=true \
  --enable-projects=false \
  --enable-wiki=false \
  --default-branch main \
  --delete-branch-on-merge=true

main_sha="$(git rev-parse main)"
protection_payload=$(cat <<EOF
{
  "required_status_checks": {
    "strict": true,
    "contexts": [
      "rust workspace",
      "public repository guard",
      "license policy",
      "dependency tree"
    ]
  },
  "enforce_admins": true,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": true,
    "require_code_owner_reviews": true,
    "required_approving_review_count": 1
  },
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "required_linear_history": true
}
EOF
)

gh api \
  --method PUT \
  -H "Accept: application/vnd.github+json" \
  "/repos/$FULL_NAME/branches/main/protection" \
  --input - <<EOF
$protection_payload
EOF

printf 'configured %s at main %s\n' "$FULL_NAME" "$main_sha"

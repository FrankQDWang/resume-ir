#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required release publication file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

script="scripts/release/create-release-publication-evidence.sh"
publish_script="scripts/release/publish-github-release.sh"
workflow=".github/workflows/release.yml"
verify_script="scripts/ci/verify-local.sh"

require_file "$script"
require_file "$publish_script"
require_file "$workflow"
require_file "$verify_script"

if [ ! -x "$script" ]; then
  fail "release publication evidence script is not executable"
fi
if [ ! -x "$publish_script" ]; then
  fail "GitHub Release publication script is not executable"
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-release-publication-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

artifact_manifest="$tmpdir/release-artifacts.json"
out_dir="$tmpdir/out"
mkdir -p "$out_dir"
cat > "$artifact_manifest" <<'JSON'
{
  "schema_version": "release.artifacts.v1",
  "version": "v0.0.0",
  "packaging_status": "blocked",
  "artifacts": [
    {"name": "resume-cli", "file": "resume-cli", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "bytes": 101},
    {"name": "resume-daemon", "file": "resume-daemon", "sha256": "2222222222222222222222222222222222222222222222222222222222222222", "bytes": 202},
    {"name": "resume-benchmark", "file": "resume-benchmark", "sha256": "3333333333333333333333333333333333333333333333333333333333333333", "bytes": 303}
  ],
  "blocked_release_steps": ["packaging", "signing", "notarization", "github_release_upload"],
  "notes": "Synthetic dry-run fixture only."
}
JSON

"$script" --version v0.0.0 --artifact-manifest "$artifact_manifest" --out-dir "$out_dir"
manifest="$out_dir/release-publication-evidence.json"
require_file "$manifest"
require_text "$manifest" '"schema_version": "release.publication_evidence.v1"'
require_text "$manifest" '"publication_status": "blocked"'
require_text "$manifest" '"evidence_boundary": "dry_run_no_release_publication"'
require_text "$manifest" '"artifact_manifest_sha256": "'
require_text "$manifest" '"human_release_approval"'
require_text "$manifest" '"github_actions_release_token"'
require_text "$manifest" '"github_release_upload_evidence"'
require_text "$manifest" '"github_release_create"'
require_text "$manifest" '"github_release_upload"'
require_text "$manifest" '"prohibited_public_material": ['
require_text "$manifest" '"github_token"'
require_text "$manifest" '"local_paths"'

if grep -Fq "$tmpdir" "$manifest"; then
  fail "release publication evidence leaked an absolute temp path"
fi

if "$script" --version 0.0.0 --artifact-manifest "$artifact_manifest" --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "release publication evidence accepted an invalid version"
fi

artifact_manifest_unknown="$tmpdir/release-artifacts-unknown-field.json"
cat > "$artifact_manifest_unknown" <<JSON
{
  "schema_version": "release.artifacts.v1",
  "version": "v0.0.0",
  "packaging_status": "blocked",
  "artifacts": [
    {"name": "resume-cli", "file": "resume-cli", "local_probe_path": "$tmpdir/PRIVATE-release-cache", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "bytes": 101},
    {"name": "resume-daemon", "file": "resume-daemon", "sha256": "2222222222222222222222222222222222222222222222222222222222222222", "bytes": 202},
    {"name": "resume-benchmark", "file": "resume-benchmark", "sha256": "3333333333333333333333333333333333333333333333333333333333333333", "bytes": 303}
  ],
  "blocked_release_steps": ["packaging", "signing", "notarization", "github_release_upload"],
  "notes": "Synthetic dry-run fixture only."
}
JSON
if "$script" --version v0.0.0 --artifact-manifest "$artifact_manifest_unknown" --out-dir "$out_dir/unknown-artifact" >/dev/null 2>&1; then
  fail "release publication evidence accepted an unknown artifact manifest field"
fi

"$publish_script" \
  --dry-run \
  --version v0.0.0 \
  --repo FrankQDWang/resume-ir \
  --artifact-manifest "$artifact_manifest" \
  --publication-evidence "$manifest" \
  --out-dir "$out_dir"
gate="$out_dir/github-release-publication-gate.json"
require_file "$gate"
require_text "$gate" '"schema_version": "release.github_publication_gate.v1"'
require_text "$gate" '"execution_mode": "dry_run"'
require_text "$gate" '"publication_status": "blocked"'
require_text "$gate" '"approval_gate": "human_release_approval_required"'
require_text "$gate" '"secret_interface": "GITHUB_TOKEN_or_GH_TOKEN_required_for_execute"'
require_text "$gate" '"gh_release_create"'
require_text "$gate" '"gh_release_upload"'
require_text "$gate" '"gh_release_download_verify"'
if grep -Fq "$tmpdir" "$gate"; then
  fail "GitHub Release publication gate leaked an absolute temp path"
fi
publication_unknown="$tmpdir/release-publication-evidence-unknown-field.json"
python3 - "$manifest" "$publication_unknown" <<'PY'
import json
import sys
from pathlib import Path

source = Path(sys.argv[1])
target = Path(sys.argv[2])
document = json.loads(source.read_text(encoding="utf-8"))
document["artifacts"][0]["local_probe_path"] = "PRIVATE-release-cache"
target.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
PY
if "$publish_script" \
  --dry-run \
  --version v0.0.0 \
  --repo FrankQDWang/resume-ir \
  --artifact-manifest "$artifact_manifest" \
  --publication-evidence "$publication_unknown" \
  --out-dir "$out_dir/unknown-publication" >/dev/null 2>&1; then
  fail "GitHub Release publication gate accepted an unknown publication evidence field"
fi
if "$publish_script" --execute --version v0.0.0 --repo FrankQDWang/resume-ir --artifact-manifest "$artifact_manifest" --publication-evidence "$manifest" --out-dir "$out_dir/execute" >/dev/null 2>&1; then
  fail "GitHub Release publication execute mode passed without explicit approval"
fi

require_text "$workflow" "scripts/release/create-release-publication-evidence.sh"
require_text "$workflow" "scripts/release/publish-github-release.sh"
require_text "$workflow" "release-publication-evidence.json"
require_text "$workflow" "github-release-publication-gate.json"
require_text "$workflow" "release publication evidence manifest leaked a local path or runtime-data marker"
require_text "$workflow" "GitHub Release publication gate leaked a local path or runtime-data marker"
require_text "$verify_script" "./scripts/ci/check-release-publication-evidence.sh"

printf '%s\n' "release publication evidence check passed"

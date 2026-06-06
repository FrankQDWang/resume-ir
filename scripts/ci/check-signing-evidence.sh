#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required signing evidence file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  if grep -Fq -- "$text" "$file"; then
    fail "$file leaked signing evidence marker: $text"
  fi
}

artifact_script="scripts/release/create-artifact-manifest.sh"
signing_script="scripts/release/create-signing-evidence.sh"
verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"
release_workflow=".github/workflows/release.yml"
runbook="docs/runbooks/release-blockers.md"

for file in "$artifact_script" "$signing_script" "$verify_script" "$workflow_guard" "$release_workflow" "$runbook"; do
  require_file "$file"
done

if [ ! -x "$artifact_script" ]; then
  fail "release artifact manifest script is not executable"
fi
if [ ! -x "$signing_script" ]; then
  fail "signing evidence script is not executable"
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-signing-evidence-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

target_dir="$tmpdir/target/release"
out_dir="$tmpdir/out"
mkdir -p "$target_dir" "$out_dir"
for binary in resume-cli resume-daemon resume-benchmark; do
  printf 'synthetic signing evidence binary %s\n' "$binary" > "$target_dir/$binary"
  chmod 755 "$target_dir/$binary"
done

"$artifact_script" --version v0.0.0 --target-dir "$target_dir" --out-dir "$out_dir" >/dev/null
"$signing_script" \
  --version v0.0.0 \
  --artifact-manifest "$out_dir/release-artifacts.json" \
  --out-dir "$out_dir" >/dev/null

manifest="$out_dir/signing-evidence.json"
require_file "$manifest"
require_text "$manifest" '"schema_version": "release.signing_evidence.v1"'
require_text "$manifest" '"version": "v0.0.0"'
require_text "$manifest" '"signing_status": "blocked"'
require_text "$manifest" '"evidence_boundary": "dry_run_no_signing_material"'
require_text "$manifest" '"artifact_manifest_sha256": "'
require_text "$manifest" '"certificate_chain"'
require_text "$manifest" '"private_key_custody"'
require_text "$manifest" '"signature_verification_evidence"'
require_text "$manifest" '"artifact_signature_verification"'
require_text "$manifest" '"resume-cli"'
require_text "$manifest" '"resume-daemon"'
require_text "$manifest" '"resume-benchmark"'
require_text "$manifest" '"signature_status": "missing"'
require_text "$manifest" '"verification_status": "blocked"'

reject_text "$manifest" "$tmpdir"
reject_text "$manifest" "/Users/"
reject_text "$manifest" "target/release"
reject_text "$manifest" "local-data"
reject_text "$manifest" "diagnostics"
reject_text "$manifest" "model-cache"
reject_text "$manifest" "signing-token"

if "$signing_script" --version 0.0.0 --artifact-manifest "$out_dir/release-artifacts.json" --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "signing evidence script accepted an invalid version"
fi

if "$signing_script" --version v0.0.1 --artifact-manifest "$out_dir/release-artifacts.json" --out-dir "$out_dir/mismatch" >/dev/null 2>&1; then
  fail "signing evidence script accepted a mismatched artifact manifest version"
fi

require_text "$verify_script" "./scripts/ci/check-signing-evidence.sh"
require_text "$workflow_guard" "check-signing-evidence.sh"
require_text "$release_workflow" "scripts/release/create-signing-evidence.sh"
require_text "$release_workflow" "signing-evidence.json"
require_text "$runbook" "create-signing-evidence.sh"
require_text "$runbook" "release.signing_evidence.v1"

printf '%s\n' "signing evidence check passed"

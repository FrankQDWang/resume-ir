#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required workflow policy file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "workflow policy $file is missing required text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  if grep -Fq -- "$text" "$file"; then
    fail "workflow policy $file contains deprecated text: $text"
  fi
}

pr_workflow=".github/workflows/pr.yml"
nightly_workflow=".github/workflows/bench-nightly.yml"
platform_workflow=".github/workflows/ci-platform.yml"
release_workflow=".github/workflows/release.yml"
verify_script="scripts/ci/verify-local.sh"

for file in "$pr_workflow" "$nightly_workflow" "$platform_workflow" "$release_workflow" "$verify_script"; do
  require_file "$file"
done

require_text "$pr_workflow" "resume-benchmark --locked -- synthetic-query"
require_text "$pr_workflow" "resume-benchmark --locked -- gate"
require_text "$pr_workflow" "resume-benchmark --locked -- ocr-throughput"
require_text "$pr_workflow" "resume-benchmark --locked -- ocr-gate"
require_text "$pr_workflow" "resume-benchmark --locked -- vector-quality"
require_text "$pr_workflow" "resume-benchmark --locked -- vector-gate"
require_text "$pr_workflow" "vector-benchmark-smoke.json"
require_text "$pr_workflow" "--allow-synthetic"
require_text "$pr_workflow" "check-workflows.sh"
require_text "$pr_workflow" "actions/checkout@v6"

require_text "$nightly_workflow" "resume-benchmark --locked -- synthetic-query"
require_text "$nightly_workflow" "resume-benchmark --locked -- gate"
require_text "$nightly_workflow" "resume-benchmark --locked -- ocr-throughput"
require_text "$nightly_workflow" "resume-benchmark --locked -- ocr-gate"
require_text "$nightly_workflow" "ocr-benchmark-smoke.json"
require_text "$nightly_workflow" "resume-benchmark --locked -- vector-quality"
require_text "$nightly_workflow" "resume-benchmark --locked -- vector-gate"
require_text "$nightly_workflow" "vector-benchmark-smoke.json"
require_text "$nightly_workflow" "--allow-synthetic"
require_text "$nightly_workflow" "actions/checkout@v6"
require_text "$nightly_workflow" "actions/upload-artifact@v7"

require_text "$platform_workflow" "pull_request"
require_text "$platform_workflow" "macos-latest"
require_text "$platform_workflow" "windows-latest"
require_text "$platform_workflow" "cargo build --workspace --locked"
require_text "$platform_workflow" "cargo test --workspace --locked"
require_text "$platform_workflow" "actions/checkout@v6"

require_text "$verify_script" "./scripts/ci/check-workflows.sh"
require_text "$verify_script" "./scripts/ci/check-release-artifacts.sh"
require_text "$verify_script" "./scripts/ci/check-release-sbom.sh"

require_text "$release_workflow" "scripts/release/create-artifact-manifest.sh"
require_text "$release_workflow" "scripts/release/create-sbom.sh"
require_text "$release_workflow" "release-artifacts.json"
require_text "$release_workflow" "release-sbom.json"
require_text "$release_workflow" "actions/upload-artifact"
require_text "$release_workflow" "actions/checkout@v6"
require_text "$release_workflow" "actions/upload-artifact@v7"
require_text "$release_workflow" "Packaging, signing, notarization"

for file in "$pr_workflow" "$nightly_workflow" "$platform_workflow" "$release_workflow"; do
  reject_text "$file" "actions/checkout@v4"
  reject_text "$file" "actions/upload-artifact@v4"
done

printf '%s\n' "workflow check passed"

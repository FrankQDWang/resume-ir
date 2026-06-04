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

pr_workflow=".github/workflows/pr.yml"
nightly_workflow=".github/workflows/bench-nightly.yml"
verify_script="scripts/ci/verify-local.sh"

for file in "$pr_workflow" "$nightly_workflow" "$verify_script"; do
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

require_text "$nightly_workflow" "resume-benchmark --locked -- synthetic-query"
require_text "$nightly_workflow" "resume-benchmark --locked -- gate"
require_text "$nightly_workflow" "resume-benchmark --locked -- ocr-throughput"
require_text "$nightly_workflow" "resume-benchmark --locked -- ocr-gate"
require_text "$nightly_workflow" "ocr-benchmark-smoke.json"
require_text "$nightly_workflow" "resume-benchmark --locked -- vector-quality"
require_text "$nightly_workflow" "resume-benchmark --locked -- vector-gate"
require_text "$nightly_workflow" "vector-benchmark-smoke.json"
require_text "$nightly_workflow" "--allow-synthetic"

require_text "$verify_script" "./scripts/ci/check-workflows.sh"

printf '%s\n' "workflow check passed"

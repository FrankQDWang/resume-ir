#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required release-readiness file: $1"
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
    fail "$file leaked local release-readiness marker: $text"
  fi
}

CARGO_BIN="${CARGO:-cargo}"
if ! command -v "$CARGO_BIN" >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi

verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"
release_workflow=".github/workflows/release.yml"
runbook="docs/runbooks/release-blockers.md"

require_file "$verify_script"
require_file "$workflow_guard"
require_file "$release_workflow"
require_file "$runbook"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-release-readiness-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

data_dir="$tmpdir/PRIVATE-release-readiness-data"
stdout_file="$tmpdir/stdout.txt"
stderr_file="$tmpdir/stderr.txt"
mkdir -p "$data_dir"

set +e
"$CARGO_BIN" run --quiet -p resume-cli --locked -- \
  --data-dir "$data_dir" release-readiness --json \
  > "$stdout_file" 2> "$stderr_file"
status=$?
set -e

if [ "$status" -eq 0 ]; then
  fail "release-readiness command unexpectedly passed stable release"
fi

require_text "$stdout_file" '"schema_version": "release-readiness.v1"'
require_text "$stdout_file" '"stable_release": "blocked"'
require_text "$stdout_file" '"local_dry_run_artifacts": "evidence_only"'
require_text "$stdout_file" '"provided_evidence": []'
require_text "$stdout_file" '"blockers": ['
require_text "$stdout_file" '"label": "signing certificates"'
require_text "$stdout_file" "production signing certificates"
require_text "$stdout_file" "certificate chain"
require_text "$stdout_file" "private key custody"
require_text "$stdout_file" "signature verification evidence"
require_text "$stdout_file" '"label": "macOS notarization"'
require_text "$stdout_file" "Apple Developer ID"
require_text "$stdout_file" "notarization credentials"
require_text "$stdout_file" "notarization ticket"
require_text "$stdout_file" "Gatekeeper validation"
require_text "$stdout_file" '"label": "Windows installer lifecycle"'
require_text "$stdout_file" "MSI install"
require_text "$stdout_file" "upgrade"
require_text "$stdout_file" "uninstall"
require_text "$stdout_file" "rollback"
require_text "$stdout_file" "release Windows runner"
require_text "$stdout_file" '"label": "Windows service lifecycle"'
require_text "$stdout_file" "install/start/stop/status/uninstall/recovery"
require_text "$stdout_file" '"label": "macOS installer lifecycle"'
require_text "$stdout_file" "signed pkg/dmg"
require_text "$stdout_file" "install/upgrade/uninstall/rollback"
require_text "$stdout_file" '"label": "private real-corpus performance evidence"'
require_text "$stdout_file" "representative local private real-corpus hot-index hybrid performance evidence is not available"
require_text "$stdout_file" "available private corpus"
require_text "$stdout_file" "500 query samples"
require_text "$stdout_file" "external 100k/1M scale validation"
require_text "$stdout_file" '"label": "field extraction quality"'
require_text "$stdout_file" "private business labeled field-quality evidence is not available"
require_text "$stdout_file" "min-samples 1000"
require_text "$stdout_file" "precision/recall/F1 >= 0.93"
require_text "$stdout_file" '"label": "dedupe quality"'
require_text "$stdout_file" "private business labeled dedupe-quality evidence is not available"
require_text "$stdout_file" "min-pairs 1000"
require_text "$stdout_file" "min-positive-pairs 100"
require_text "$stdout_file" "precision/recall/F1 >= 0.90"
require_text "$stdout_file" '"label": "vector quality"'
require_text "$stdout_file" "private business labeled vector-quality evidence is not available"
require_text "$stdout_file" "recall@k >= 0.90"
require_text "$stdout_file" "MRR >= 0.85"
require_text "$stdout_file" "NDCG@k >= 0.90"
require_text "$stdout_file" '"label": "OCR throughput"'
require_text "$stdout_file" "private real-corpus OCR throughput evidence is not available"
require_text "$stdout_file" "min-pages 500"
require_text "$stdout_file" "OCR p95 <= 1000ms"
require_text "$stdout_file" "pages_per_second >= 1"
require_text "$stdout_file" '"label": "OCR engine license/distribution"'
require_text "$stdout_file" "reviewed OCR runtime manifest"
require_text "$stdout_file" "engine distribution license"
require_text "$stdout_file" "language-pack distribution license"
require_text "$stdout_file" "offline packaging evidence"
require_text "$stdout_file" '"label": "embedding model license/distribution"'
require_text "$stdout_file" "reviewed licensed embedding model"
require_text "$stdout_file" "model manifest"
require_text "$stdout_file" "offline distribution"
require_text "$stdout_file" "license review"
require_text "$stdout_file" '"label": "cross-platform release validation"'
require_text "$stdout_file" "Windows and macOS release platforms"
require_text "$stdout_file" "fresh release artifacts"
require_text "$stdout_file" "install/upgrade/uninstall"
require_text "$stdout_file" "service lifecycle"
require_text "$stdout_file" '"label": "hardware fault drills"'
require_text "$stdout_file" "actual ENOSPC"
require_text "$stdout_file" "service-level daemon kill"
require_text "$stdout_file" '"status": "blocked"'
require_text "$stdout_file" '"next_gate": "keep release blocked until every item has current local evidence"'
require_text "$stderr_file" "release readiness blocked: stable release criteria are not met"

reject_text "$stdout_file" "$tmpdir"
reject_text "$stderr_file" "$tmpdir"
reject_text "$stdout_file" "PRIVATE-release-readiness-data"
reject_text "$stderr_file" "PRIVATE-release-readiness-data"
reject_text "$stdout_file" "/Users/"
reject_text "$stderr_file" "/Users/"
reject_text "$stdout_file" "local-data"
reject_text "$stderr_file" "local-data"
reject_text "$stdout_file" "diagnostics"
reject_text "$stderr_file" "diagnostics"
reject_text "$stdout_file" "model-cache"
reject_text "$stderr_file" "model-cache"

require_text "$verify_script" "./scripts/ci/check-release-readiness.sh"
require_text "$workflow_guard" "check-release-readiness.sh"
require_text "$release_workflow" "./scripts/ci/check-release-readiness.sh"
require_text "$runbook" "resume-cli --data-dir <local-data-dir> release-readiness --json"
require_text "$runbook" "--benchmark-report private-benchmark-local.json"
require_text "$runbook" "--field-quality-report private-field-quality.json"
require_text "$runbook" "--dedupe-quality-report private-dedupe-quality.json"
require_text "$runbook" "--vector-quality-report private-vector-quality.json"
require_text "$runbook" "--ocr-throughput-report private-ocr-throughput.json"
require_text "$runbook" "provided_evidence"
require_text "$runbook" "hardware fault drills"
require_text "$runbook" "actual ENOSPC"
require_text "$runbook" "service-level daemon kill"
require_text "$runbook" "vector-gate --report private-vector-quality.json"
require_text "$runbook" "ocr-gate --report private-ocr-throughput.json"
require_text "$runbook" "--max-run-ms <release-budget-ms>"
require_text "$runbook" "failed_document_count"
require_text "$runbook" "run_budget_exhausted"

printf '%s\n' "release readiness check passed"

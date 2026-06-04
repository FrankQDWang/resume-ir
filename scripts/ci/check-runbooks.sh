#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required runbook: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq "$text" "$file"; then
    fail "runbook $file is missing required text: $text"
  fi
}

diagnostics_runbook="docs/runbooks/diagnostics-redaction.md"
fault_runbook="docs/runbooks/fault-injection.md"
worker_runbook="docs/runbooks/ocr-embedding-workers.md"
release_runbook="docs/runbooks/release-blockers.md"

for file in "$diagnostics_runbook" "$fault_runbook" "$worker_runbook" "$release_runbook"; do
  require_file "$file"
  require_text "$file" "Local-only"
  require_text "$file" "Do not upload"
  require_text "$file" "Synthetic fixtures"
done

require_text "$diagnostics_runbook" "resume-cli export-diagnostics --redact"
require_text "$diagnostics_runbook" "raw resume text"
require_text "$diagnostics_runbook" "complete paths"
require_text "$fault_runbook" "resume-cli fault-simulate --case disk-space-low"
require_text "$fault_runbook" "resume-cli fault-simulate --case permission-denied"
require_text "$fault_runbook" "resume-cli fault-simulate --case file-lock"
require_text "$fault_runbook" "resume-cli fault-simulate --case model-checksum"
require_text "$fault_runbook" "resume-cli fault-simulate --case daemon-kill"
require_text "$fault_runbook" "resume-cli fault-simulate --case ocr-crash"
require_text "$fault_runbook" "resume-cli fault-simulate --case battery-mode"
require_text "$fault_runbook" "resume-cli fault-simulate --case external-drive-disconnect"
require_text "$fault_runbook" "real hardware drill: blocked"
require_text "$worker_runbook" "resume-cli ocr-worker --once"
require_text "$worker_runbook" "resume-cli ocr validate-manifest --manifest"
require_text "$worker_runbook" "resume-cli model validate-manifest --manifest"
require_text "$worker_runbook" "resume-daemon"
require_text "$worker_runbook" "FailedRetryable"
require_text "$release_runbook" "BLOCKED"
require_text "$release_runbook" "resume-benchmark gate"
require_text "$release_runbook" "resume-cli --data-dir <local-data-dir> ocr validate-manifest"
require_text "$release_runbook" "resume-cli --data-dir <local-data-dir> model validate-manifest"
require_text "$release_runbook" "signing"
require_text "$release_runbook" "notarization"
require_text "$release_runbook" "Windows"
require_text "$release_runbook" "macOS"

printf '%s\n' "runbook check passed"

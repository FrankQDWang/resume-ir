#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "local diagnostics release-evidence check missing expected file"
  fi
}

require_text() {
  file="$1"
  text="$2"
  require_file "$file"
  if ! grep -Fq -- "$text" "$file"; then
    fail "local diagnostics release-evidence check missing expected text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  label="$3"
  require_file "$file"
  if [ -n "$text" ] && grep -Fq -- "$text" "$file"; then
    fail "local diagnostics release-evidence check leaked $label"
  fi
}

reject_regex() {
  file="$1"
  pattern="$2"
  label="$3"
  require_file "$file"
  if grep -Eq -- "$pattern" "$file"; then
    fail "local diagnostics release-evidence check leaked $label"
  fi
}

run_cli() {
  label="$1"
  stdout_file="$2"
  stderr_file="$stdout_file.stderr"
  shift 2
  if ! "$CARGO_BIN" run --quiet -p resume-cli --locked -- --data-dir "$data_dir" "$@" \
    > "$stdout_file" 2> "$stderr_file"; then
    if [ "${RESUME_IR_DIAGNOSTICS_EVIDENCE_DEBUG:-0}" = "1" ]; then
      sed -n '1,80p' "$stdout_file" >&2 || true
      sed -n '1,80p' "$stderr_file" >&2 || true
    fi
    fail "local diagnostics release-evidence check failed at step: $label"
  fi
  if [ -s "$stderr_file" ]; then
    if [ "${RESUME_IR_DIAGNOSTICS_EVIDENCE_DEBUG:-0}" = "1" ]; then
      sed -n '1,80p' "$stdout_file" >&2 || true
      sed -n '1,80p' "$stderr_file" >&2 || true
    fi
    fail "local diagnostics release-evidence command wrote stderr at step: $label"
  fi
}

reject_paths() {
  file="$1"
  reject_text "$file" "$tmpdir" "temporary root path"
  reject_text "$file" "$data_dir" "private data-dir path"
  reject_text "$file" "$fixture_root" "fixture root path"
  reject_text "$file" "$canonical_fixture_root" "canonical fixture root path"
  reject_regex "$file" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"
}

CARGO_BIN="${CARGO:-}"
if [ -z "$CARGO_BIN" ] && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi
if [ -z "$CARGO_BIN" ]; then
  CARGO_BIN=cargo
fi

fixture_root="tests/fixtures/resumes"
if [ ! -d "$fixture_root" ]; then
  fail "local diagnostics release-evidence fixture root is missing"
fi
canonical_fixture_root="$(cd "$fixture_root" && pwd -P)"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-diagnostics-evidence.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

data_dir="$tmpdir/PRIVATE_DIAGNOSTICS_EVIDENCE_DATA"

import_out="$tmpdir/import.out"
run_cli "import synthetic fixtures" "$import_out" import --root "$fixture_root"
require_text "$import_out" "status: completed"
require_text "$import_out" "files discovered: 3"
reject_paths "$import_out"

search_out="$tmpdir/search.out"
run_cli "search for query telemetry" "$search_out" search Java --top-k 20
require_text "$search_out" "results: 2"
reject_text "$search_out" "query:" "raw query label"
reject_paths "$search_out"

diagnostics_out="$tmpdir/redacted-diagnostics.json"
run_cli "export-diagnostics --redact" "$diagnostics_out" export-diagnostics --redact
python3 -m json.tool "$diagnostics_out" >/dev/null
require_text "$diagnostics_out" "\"schema_version\": \"diagnostics.v1\""
require_text "$diagnostics_out" "\"redacted\": true"
require_text "$diagnostics_out" "\"raw_paths\": \"<redacted>\""
require_text "$diagnostics_out" "\"raw_queries\": \"<redacted>\""
require_text "$diagnostics_out" "\"raw_resume_text\": \"<redacted>\""
require_text "$diagnostics_out" "\"evidence_level\": \"local_aggregate_only\""
require_text "$diagnostics_out" "\"diagnostic_scope\""
reject_text "$diagnostics_out" "Synthetic Java" "raw resume text"
reject_text "$diagnostics_out" "Java payment" "raw resume text"
reject_regex "$diagnostics_out" '[[:alnum:]_.+-]+@[[:alnum:]_.-]+\.[[:alpha:]]{2,}' "email address"
reject_regex "$diagnostics_out" '[0-9]{3}[- .][0-9]{3}[- .][0-9]{4}' "phone number"
reject_paths "$diagnostics_out"

release_stdout="$tmpdir/release-readiness.stdout.json"
release_stderr="$tmpdir/release-readiness.stderr.txt"
set +e
"$CARGO_BIN" run --quiet -p resume-cli --locked -- \
  --data-dir "$data_dir" release-readiness --json \
  --diagnostics-report "$diagnostics_out" \
  > "$release_stdout" 2> "$release_stderr"
release_status=$?
set -e
if [ "$release_status" -eq 0 ]; then
  fail "local diagnostics release-evidence unexpectedly passed stable release"
fi

require_text "$release_stdout" '"schema_version": "release-readiness.v1"'
require_text "$release_stdout" '"stable_release": "blocked"'
require_text "$release_stdout" '"label": "redacted diagnostics evidence"'
require_text "$release_stdout" '"status": "provided"'
require_text "$release_stdout" "diagnostics.v1 report passed local aggregate redaction and scope checks"
require_text "$release_stdout" '"label": "private real-corpus performance evidence"'
require_text "$release_stdout" '"label": "field extraction quality"'
require_text "$release_stdout" '"label": "OCR throughput"'
require_text "$release_stdout" '"label": "hardware fault drills"'
require_text "$release_stderr" "release readiness blocked: stable release criteria are not met"
reject_text "$release_stdout" "$tmpdir" "temporary root path"
reject_text "$release_stderr" "$tmpdir" "temporary root path"
reject_text "$release_stdout" "$data_dir" "private data-dir path"
reject_text "$release_stderr" "$data_dir" "private data-dir path"
reject_text "$release_stdout" "$diagnostics_out" "diagnostics report path"
reject_text "$release_stderr" "$diagnostics_out" "diagnostics report path"
reject_text "$release_stdout" "PRIVATE_DIAGNOSTICS_EVIDENCE_DATA" "private data marker"
reject_text "$release_stderr" "PRIVATE_DIAGNOSTICS_EVIDENCE_DATA" "private data marker"
reject_text "$release_stdout" "/Users/" "absolute local path"
reject_text "$release_stderr" "/Users/" "absolute local path"
reject_text "$release_stdout" "Synthetic Java" "raw resume text"
reject_text "$release_stdout" "Java payment" "raw resume text"

printf '%s\n' "local diagnostics release-evidence check passed"

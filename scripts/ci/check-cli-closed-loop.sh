#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "cli closed-loop missing expected output"
  fi
}

require_text() {
  file="$1"
  text="$2"
  require_file "$file"
  if ! grep -Fq -- "$text" "$file"; then
    fail "cli closed-loop output is missing required evidence: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  label="$3"
  require_file "$file"
  if [ -n "$text" ] && grep -Fq -- "$text" "$file"; then
    fail "cli closed-loop output leaked $label"
  fi
}

reject_regex() {
  file="$1"
  pattern="$2"
  label="$3"
  require_file "$file"
  if grep -Eq -- "$pattern" "$file"; then
    fail "cli closed-loop output leaked $label"
  fi
}

run_cli() {
  label="$1"
  stdout_file="$2"
  stderr_file="$stdout_file.stderr"
  shift 2
  if ! "$CARGO_BIN" run --quiet -p resume-cli --locked -- --data-dir "$data_dir" "$@" \
    >"$stdout_file" 2>"$stderr_file"; then
    fail "cli closed-loop failed at step: $label"
  fi
  if [ -s "$stderr_file" ]; then
    fail "cli closed-loop command wrote stderr at step: $label"
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

CARGO_BIN="${CARGO:-cargo}"
if ! command -v "$CARGO_BIN" >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi

fixture_root="tests/fixtures/resumes"
if [ ! -d "$fixture_root" ]; then
  fail "cli closed-loop fixture root is missing"
fi
canonical_fixture_root="$(cd "$fixture_root" && pwd -P)"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-cli-closed-loop.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

data_dir="$tmpdir/PRIVATE_CLI_CLOSED_LOOP_DATA"
ocr_command="$tmpdir/ocr-fixture.sh"

cat >"$ocr_command" <<'SH'
#!/usr/bin/env sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.97\n'
printf 'text:\n'
printf 'SUMMARY\nSynthetic OCR profile.\nEXPERIENCE\nBuilt CLIClosedLoopOCRToken systems on page %s.\nSKILLS\nSearch.\n' "$RESUME_IR_OCR_PAGE_NO"
SH
chmod 700 "$ocr_command"

import_out="$tmpdir/import.out"
run_cli "import" "$import_out" import --root "$fixture_root"
require_text "$import_out" "import task submitted"
require_text "$import_out" "status: completed"
require_text "$import_out" "files discovered: 3"
require_text "$import_out" "searchable documents: 2"
require_text "$import_out" "ocr required documents: 1"
reject_paths "$import_out"

status_after_import_out="$tmpdir/status-after-import.out"
run_cli "status after import" "$status_after_import_out" status
require_text "$status_after_import_out" "searchable documents: 2"
require_text "$status_after_import_out" "ocr queue: 1"
require_text "$status_after_import_out" "import tasks queued: 0"
require_text "$status_after_import_out" "index health: ready"
require_text "$status_after_import_out" "search index: available (database Ready full-text snapshot)"
reject_paths "$status_after_import_out"

fulltext_out="$tmpdir/fulltext-search.out"
run_cli "fulltext search" "$fulltext_out" search Java --top-k 20
require_text "$fulltext_out" "results: 2"
reject_text "$fulltext_out" "query:" "raw query label"
reject_paths "$fulltext_out"

field_out="$tmpdir/field-search.out"
run_cli "field search" "$field_out" search Java --degree bachelor --skills-any java --top-k 20
require_text "$field_out" "results: 2"
reject_text "$field_out" "query:" "raw query label"
reject_paths "$field_out"

ocr_out="$tmpdir/ocr-worker.out"
run_cli "ocr worker" "$ocr_out" ocr-worker --once --command "$ocr_command"
require_text "$ocr_out" "ocr worker: completed"
require_text "$ocr_out" "documents processed: 1"
require_text "$ocr_out" "cache writes: 1"
require_text "$ocr_out" "cache hits: 0"
reject_text "$ocr_out" "CLIClosedLoopOCRToken" "OCR text"
reject_paths "$ocr_out"

ocr_search_out="$tmpdir/ocr-search.out"
run_cli "ocr search" "$ocr_search_out" search CLIClosedLoopOCRToken --top-k 20
require_text "$ocr_search_out" "results: 1"
reject_text "$ocr_search_out" "query:" "raw query label"
reject_paths "$ocr_search_out"

doctor_out="$tmpdir/doctor.out"
run_cli "doctor" "$doctor_out" doctor
require_text "$doctor_out" "resume-ir doctor"
require_text "$doctor_out" "search index: available (database Ready full-text snapshot)"
require_text "$doctor_out" "metadata encryption: sqlcipher"
reject_text "$doctor_out" "CLIClosedLoopOCRToken" "OCR text"
reject_text "$doctor_out" "SemanticOnlyToken" "raw query"
reject_paths "$doctor_out"

diagnostics_out="$tmpdir/diagnostics.out"
run_cli "export diagnostics" "$diagnostics_out" export-diagnostics --redact
python3 -m json.tool "$diagnostics_out" >/dev/null
require_text "$diagnostics_out" "\"paths\": \"<redacted>\""
require_text "$diagnostics_out" "\"metadata_encryption\": \"sqlcipher\""
require_text "$diagnostics_out" "\"raw_queries\": \"<redacted>\""
reject_text "$diagnostics_out" "CLIClosedLoopOCRToken" "OCR text"
reject_text "$diagnostics_out" "SemanticOnlyToken" "raw query"
reject_text "$diagnostics_out" "Synthetic Java" "raw resume text"
reject_text "$diagnostics_out" "Java payment" "raw resume text"
reject_regex "$diagnostics_out" '[[:alnum:]_.+-]+@[[:alnum:]_.-]+\.[[:alpha:]]{2,}' "email address"
reject_regex "$diagnostics_out" '[0-9]{3}[- .][0-9]{3}[- .][0-9]{4}' "phone number"
reject_paths "$diagnostics_out"

printf '%s\n' "cli closed-loop check passed"

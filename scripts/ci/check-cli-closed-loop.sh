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
embedding_command="$tmpdir/embedding-fixture.sh"

cat >"$ocr_command" <<'SH'
#!/usr/bin/env sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.97\n'
printf 'text:\n'
printf 'CLIClosedLoopOCRToken page %s\n' "$RESUME_IR_OCR_PAGE_NO"
SH
chmod 700 "$ocr_command"

cat >"$embedding_command" <<'SH'
#!/usr/bin/env sh
if [ ! -s "$RESUME_IR_EMBEDDING_INPUT_PATH" ]; then
  exit 7
fi
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ {
  id=$1;
  sub(/^input=/, "", id);
  printf "vector=%s\t1,0,0,0\n", id
}' "$RESUME_IR_EMBEDDING_INPUT_PATH"
SH
chmod 700 "$embedding_command"

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
require_text "$status_after_import_out" "search index: available (full-text snapshot)"
reject_paths "$status_after_import_out"

fulltext_out="$tmpdir/fulltext-search.out"
run_cli "fulltext search" "$fulltext_out" search Java --top-k 20
require_text "$fulltext_out" "results: 2"
require_text "$fulltext_out" "synthetic-java-platform.pdf"
require_text "$fulltext_out" "synthetic-java-engineer.docx"
reject_text "$fulltext_out" "query:" "raw query label"
reject_paths "$fulltext_out"

field_out="$tmpdir/field-search.out"
run_cli "field search" "$field_out" search Java --degree bachelor --skills-any java --top-k 20
require_text "$field_out" "results: 2"
require_text "$field_out" "synthetic-java-platform.pdf"
require_text "$field_out" "synthetic-java-engineer.docx"
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
require_text "$ocr_search_out" "synthetic-scanned-resume.pdf"
reject_text "$ocr_search_out" "query:" "raw query label"
reject_paths "$ocr_search_out"

embed_out="$tmpdir/embed-worker.out"
run_cli "embed worker" "$embed_out" embed-worker --once --command "$embedding_command" --model-id fixture-local-model --dimension 4 --max-docs 8 --max-text-bytes 100000
require_text "$embed_out" "embedding worker: completed"
require_text "$embed_out" "model id: fixture-local-model"
require_text "$embed_out" "dimension: 4"
require_text "$embed_out" "documents considered: 3"
require_text "$embed_out" "documents embedded: 3"
require_text "$embed_out" "vector index: available (hnsw ann vector snapshot)"
reject_paths "$embed_out"

status_after_embed_out="$tmpdir/status-after-embed.out"
run_cli "status after embedding" "$status_after_embed_out" status
require_text "$status_after_embed_out" "searchable documents: 3"
require_text "$status_after_embed_out" "ocr queue: 0"
require_text "$status_after_embed_out" "vector index: available (hnsw ann vector snapshot)"
require_text "$status_after_embed_out" "search index: available (full-text snapshot)"
reject_paths "$status_after_embed_out"

semantic_out="$tmpdir/semantic-search.out"
run_cli "semantic search" "$semantic_out" search SemanticOnlyToken --mode semantic --embedding-command "$embedding_command" --model-id fixture-local-model --dimension 4 --top-k 20
require_text "$semantic_out" "results: 3"
require_text "$semantic_out" "synthetic-java-platform.pdf"
require_text "$semantic_out" "synthetic-java-engineer.docx"
require_text "$semantic_out" "synthetic-scanned-resume.pdf"
reject_text "$semantic_out" "SemanticOnlyToken" "semantic raw query"
reject_paths "$semantic_out"

hybrid_out="$tmpdir/hybrid-search.out"
run_cli "hybrid search" "$hybrid_out" search SemanticOnlyToken --mode hybrid --embedding-command "$embedding_command" --model-id fixture-local-model --dimension 4 --top-k 20
require_text "$hybrid_out" "results: 3"
require_text "$hybrid_out" "synthetic-java-platform.pdf"
require_text "$hybrid_out" "synthetic-java-engineer.docx"
require_text "$hybrid_out" "synthetic-scanned-resume.pdf"
reject_text "$hybrid_out" "SemanticOnlyToken" "hybrid raw query"
reject_paths "$hybrid_out"

doctor_out="$tmpdir/doctor.out"
run_cli "doctor" "$doctor_out" doctor
require_text "$doctor_out" "resume-ir doctor"
require_text "$doctor_out" "search index: available (full-text snapshot)"
require_text "$doctor_out" "vector index: available (hnsw ann vector snapshot)"
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

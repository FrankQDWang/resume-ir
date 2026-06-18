#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing expected text: $text"
  fi
}

expect_fail() {
  label="$1"
  expected="$2"
  shift 2
  stdout_file="$tmpdir/$label.stdout"
  stderr_file="$tmpdir/$label.stderr"
  if "$@" >"$stdout_file" 2>"$stderr_file"; then
    fail "$label unexpectedly passed"
  fi
  require_text "$stderr_file" "$expected"
}

command -v python3 >/dev/null 2>&1 || fail "python3 is required for current-stage observability validation"

tmpdir="${TMPDIR:-/tmp}/resume-ir-current-stage-observability.$$"
rm -rf "$tmpdir"
mkdir -p "$tmpdir"
trap 'rm -rf "$tmpdir"' EXIT INT TERM

valid_summary="$tmpdir/valid-summary.json"
cat >"$valid_summary" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 8123,
    "searchable_document_count": 7000,
    "vector_indexed_document_count": 6500,
    "hot_index_fully_covered": false,
    "document_status_counts": {
      "searchable": 7000,
      "ocr_required": 1123
    },
    "ingest_job_status_counts": {
      "completed": 7000,
      "queued": 1123
    },
    "ingest_job_kind_status_counts": {
      "update_index": {
        "completed": 7000
      },
      "ocr_document": {
        "queued": 1123
      }
    },
    "ingest_job_failure_counts": {}
  }
}
JSON

python3 scripts/ci/validate-current-stage-observability.py --summary "$valid_summary"

valid_full="$tmpdir/valid-full-evidence.json"
cat >"$valid_full" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-validation-evidence.v1",
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 8001,
    "searchable_document_count": 8001,
    "vector_indexed_document_count": 8001,
    "hot_index_fully_covered": true,
    "document_status_counts": {
      "searchable": 8001
    },
    "ingest_job_status_counts": {
      "completed": 8001
    },
    "ingest_job_kind_status_counts": {
      "update_index": {
        "completed": 8001
      }
    },
    "ingest_job_failure_counts": {}
  }
}
JSON

python3 scripts/ci/validate-current-stage-observability.py --full-evidence "$valid_full"

below_floor="$tmpdir/below-floor.json"
cat >"$below_floor" <<'JSON'
{
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 7999,
    "searchable_document_count": 7999,
    "vector_indexed_document_count": 7999,
    "hot_index_fully_covered": true,
    "document_status_counts": {},
    "ingest_job_status_counts": {},
    "ingest_job_kind_status_counts": {},
    "ingest_job_failure_counts": {}
  }
}
JSON

expect_fail below-floor "document_count below current-stage floor" \
  python3 scripts/ci/validate-current-stage-observability.py --summary "$below_floor"

bad_vector="$tmpdir/bad-vector.json"
cat >"$bad_vector" <<'JSON'
{
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 8123,
    "searchable_document_count": 100,
    "vector_indexed_document_count": 101,
    "hot_index_fully_covered": false,
    "document_status_counts": {},
    "ingest_job_status_counts": {},
    "ingest_job_kind_status_counts": {},
    "ingest_job_failure_counts": {}
  }
}
JSON

expect_fail bad-vector "vector_indexed_document_count is inconsistent" \
  python3 scripts/ci/validate-current-stage-observability.py --summary "$bad_vector"

private_field="$tmpdir/private-field.json"
cat >"$private_field" <<'JSON'
{
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 8123,
    "searchable_document_count": 8123,
    "vector_indexed_document_count": 8123,
    "hot_index_fully_covered": true,
    "document_status_counts": {},
    "ingest_job_status_counts": {},
    "ingest_job_kind_status_counts": {},
    "ingest_job_failure_counts": {},
    "data_dir": "/Users/example/private-runtime-data"
  }
}
JSON

expect_fail private-field "forbidden observability field" \
  python3 scripts/ci/validate-current-stage-observability.py --summary "$private_field"

full_without_hot_index="$tmpdir/full-without-hot-index.json"
cat >"$full_without_hot_index" <<'JSON'
{
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 8123,
    "searchable_document_count": 8123,
    "vector_indexed_document_count": 8000,
    "hot_index_fully_covered": false,
    "document_status_counts": {},
    "ingest_job_status_counts": {},
    "ingest_job_kind_status_counts": {},
    "ingest_job_failure_counts": {}
  }
}
JSON

expect_fail full-without-hot-index "hot index coverage must be true for full evidence" \
  python3 scripts/ci/validate-current-stage-observability.py --full-evidence "$full_without_hot_index"

printf '%s\n' "current-stage observability check passed"

#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required current-stage handoff file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  require_file "$file"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  if [ -n "$text" ] && grep -Fq -- "$text" "$file"; then
    fail "$file leaked current-stage handoff marker: $text"
  fi
}

reject_regex() {
  file="$1"
  pattern="$2"
  label="$3"
  require_file "$file"
  if grep -Eq -- "$pattern" "$file"; then
    fail "$file leaked $label"
  fi
}

require_handoff_observability() {
  file="$1"
  min_documents="$2"
  python3 scripts/ci/validate-current-stage-observability.py \
    --summary "$file" \
    --min-documents "$min_documents" \
    --field observability \
    || fail "$file current-stage handoff observability is invalid"
}

script="scripts/local/summarize-current-stage-validation.py"
require_file "$script"

command -v python3 >/dev/null 2>&1 || fail "python3 is required"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-current-stage-handoff.XXXXXX")"
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

smoke_summary="$tmpdir/PRIVATE-current-stage-smoke-summary.json"
blocked_summary="$tmpdir/PRIVATE-current-stage-blocked-summary.json"
full_evidence="$tmpdir/PRIVATE-current-stage-validation-evidence.json"
smoke_out="$tmpdir/smoke-handoff.json"
blocked_out="$tmpdir/blocked-handoff.json"
full_out="$tmpdir/full-handoff.json"
bad_summary="$tmpdir/PRIVATE-bad-summary.json"
bad_out="$tmpdir/bad-handoff.json"
bad_boundary_summary="$tmpdir/PRIVATE-bad-boundary-summary.json"
bad_boundary_out="$tmpdir/bad-boundary-handoff.json"
bad_missing_blocked_observability="$tmpdir/PRIVATE-bad-missing-blocked-observability.json"
bad_missing_full_observability="$tmpdir/PRIVATE-bad-missing-full-observability.json"

cat > "$smoke_summary" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-smoke-summary.v1",
  "privacy_boundary": "local_only_redacted_aggregate_summary",
  "validation_profile": "smoke",
  "current_stage_target": "local_real_corpus_smoke_chain",
  "smoke_satisfied": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 12,
    "searchable_document_count": 1,
    "vector_indexed_document_count": 1,
    "hot_index_fully_covered": false,
    "document_status_counts": {
      "ocr_required": 11,
      "searchable": 1
    },
    "ingest_job_status_counts": {
      "completed": 1,
      "queued": 11
    },
    "ingest_job_kind_status_counts": {
      "ocr_document": {
        "queued": 11
      },
      "update_index": {
        "completed": 1
      }
    },
    "ingest_job_failure_counts": {
      "ocr_page_budget_exceeded": 1
    },
    "contains_raw_resume_text": false,
    "contains_resume_paths": false,
    "contains_queries": false,
    "contains_sample_ids": false
  },
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "smoke_success"}
  ],
  "not_completed": [
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "model caches"
  ]
}
JSON

python3 "$script" --input "$smoke_summary" --out "$smoke_out"
python3 -m json.tool "$smoke_out" >/dev/null
require_text "$smoke_out" '"schema_version": "resume-ir.current-stage-handoff.v1"'
require_text "$smoke_out" '"privacy_boundary": "local_only_redacted_handoff"'
require_text "$smoke_out" '"source_schema": "resume-ir.current-stage-smoke-summary.v1"'
require_text "$smoke_out" '"current_stage_status": "smoke_satisfied"'
require_text "$smoke_out" '"validation_profile": "smoke"'
require_text "$smoke_out" '"complete_product": false'
require_text "$smoke_out" '"full_baseline_satisfied": false'
require_text "$smoke_out" '"release_readiness_evidence": false'
require_text "$smoke_out" '"ocr_runtime_probe": "passed"'
require_text "$smoke_out" '"embedding_protocol": "passed"'
require_handoff_observability "$smoke_out" 1
require_text "$smoke_out" '"ocr_page_budget_exceeded": 1'
require_text "$smoke_out" '"blocked_or_not_complete"'
require_text "$smoke_out" '"full 10k/8000-document current-stage baseline"'
require_text "$smoke_out" '"must_not_upload"'
reject_text "$smoke_out" "$tmpdir"
reject_text "$smoke_out" "PRIVATE-current-stage"
reject_regex "$smoke_out" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"

cat > "$bad_boundary_summary" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-smoke-summary.v1",
  "privacy_boundary": "unsafe_boundary",
  "validation_profile": "smoke",
  "current_stage_target": "local_real_corpus_smoke_chain",
  "smoke_satisfied": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "steps": [
    {"id": "corpus_summary", "status": "success"}
  ],
  "must_not_upload": [
    "raw resumes"
  ]
}
JSON
if python3 "$script" --input "$bad_boundary_summary" --out "$bad_boundary_out" 2>/dev/null; then
  fail "current-stage handoff accepted an invalid source privacy_boundary"
fi

cat > "$bad_missing_blocked_observability" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "full",
  "current_stage_target": "reproducible_local_10k_baseline",
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "import_private_corpus",
  "blocked_category": "import/parser",
  "blocked_reason": "import_private_corpus_failed",
  "blocked_exit": 7,
  "private_corpus_read": true,
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "steps": [
    {"id": "corpus_summary", "status": "success"}
  ],
  "must_not_upload": [
    "raw resumes"
  ]
}
JSON
if python3 "$script" --input "$bad_missing_blocked_observability" --out "$bad_out" 2>/dev/null; then
  fail "current-stage handoff accepted a private-corpus blocked summary without observability"
fi

cat > "$blocked_summary" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "full",
  "current_stage_target": "reproducible_local_10k_baseline",
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "import_private_corpus",
  "blocked_category": "import/parser",
  "blocked_reason": "import_private_corpus_failed",
  "blocked_exit": 7,
  "private_corpus_read": true,
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 8000,
    "searchable_document_count": 200,
    "vector_indexed_document_count": 200,
    "hot_index_fully_covered": false,
    "document_status_counts": {
      "ocr_required": 7800,
      "searchable": 200
    },
    "ingest_job_status_counts": {
      "completed": 200,
      "queued": 7800
    },
    "ingest_job_kind_status_counts": {
      "ocr_document": {
        "queued": 7800
      },
      "update_index": {
        "completed": 200
      }
    },
    "ingest_job_failure_counts": {},
    "contains_raw_resume_text": false,
    "contains_resume_paths": false,
    "contains_queries": false,
    "contains_sample_ids": false
  },
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "import_private_corpus", "status": "blocked"}
  ],
  "not_completed": [
    "full 10k/8000-document current-stage baseline"
  ],
  "must_not_upload": [
    "raw resumes",
    "indexes"
  ]
}
JSON

python3 "$script" --input "$blocked_summary" --out "$blocked_out"
python3 -m json.tool "$blocked_out" >/dev/null
require_text "$blocked_out" '"source_schema": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$blocked_out" '"current_stage_status": "blocked"'
require_text "$blocked_out" '"blocked_step": "import_private_corpus"'
require_text "$blocked_out" '"blocked_category": "import/parser"'
require_text "$blocked_out" '"blocked_reason": "import_private_corpus_failed"'
require_text "$blocked_out" '"private_corpus_read": true'
require_text "$blocked_out" '"next_action"'
require_text "$blocked_out" '"category": "import/parser"'
require_text "$blocked_out" '"recommended_next_step": "fix import/parser blocker and rerun current-stage validation"'
require_text "$blocked_out" '"do_not_do": "do not chase P95/P99 optimization or require million-resume validation in current stage"'
require_handoff_observability "$blocked_out" 8000
reject_text "$blocked_out" "$tmpdir"
reject_text "$blocked_out" "PRIVATE-current-stage"
reject_regex "$blocked_out" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"

cat > "$bad_missing_full_observability" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-validation-evidence.v1",
  "privacy_boundary": "local_only_redacted_evidence_manifest",
  "current_stage_target": "reproducible_local_10k_baseline",
  "full_baseline_satisfied": true,
  "release_readiness_evidence": true,
  "performance_optimization_deferred": true,
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "steps": [],
  "must_not_upload": [
    "raw resumes"
  ]
}
JSON
if python3 "$script" --input "$bad_missing_full_observability" --out "$bad_out" 2>/dev/null; then
  fail "current-stage handoff accepted full evidence without observability"
fi

cat > "$full_evidence" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-validation-evidence.v1",
  "privacy_boundary": "local_only_redacted_evidence_manifest",
  "current_stage_target": "reproducible_local_10k_baseline",
  "full_baseline_satisfied": true,
  "release_readiness_evidence": true,
  "performance_optimization_deferred": true,
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 8000,
    "searchable_document_count": 8000,
    "vector_indexed_document_count": 8000,
    "hot_index_fully_covered": true,
    "document_status_counts": {
      "searchable": 8000
    },
    "ingest_job_status_counts": {
      "completed": 8000
    },
    "ingest_job_kind_status_counts": {
      "update_index": {
        "completed": 8000
      }
    },
    "ingest_job_failure_counts": {},
    "contains_raw_resume_text": false,
    "contains_resume_paths": false,
    "contains_queries": false,
    "contains_sample_ids": false
  },
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "release_readiness_intake", "status": "expected_blocked"}
  ],
  "must_not_upload": [
    "raw resumes",
    "diagnostics",
    "indexes"
  ]
}
JSON

python3 "$script" --input "$full_evidence" --out "$full_out"
python3 -m json.tool "$full_out" >/dev/null
require_text "$full_out" '"source_schema": "resume-ir.current-stage-validation-evidence.v1"'
require_text "$full_out" '"current_stage_status": "full_evidence_ready"'
require_text "$full_out" '"validation_profile": "full"'
require_text "$full_out" '"complete_product": false'
require_text "$full_out" '"full_baseline_satisfied": true'
require_text "$full_out" '"release_readiness_evidence": true'
require_text "$full_out" '"release_readiness_intake"'
require_handoff_observability "$full_out" 8000
reject_text "$full_out" "$tmpdir"
reject_text "$full_out" "PRIVATE-current-stage"
reject_regex "$full_out" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"

cat > "$bad_summary" <<JSON
{
  "schema_version": "resume-ir.current-stage-smoke-summary.v1",
  "privacy_boundary": "local_only_redacted_aggregate_summary",
  "validation_profile": "smoke",
  "current_stage_target": "$tmpdir/PRIVATE-current-stage-resumes",
  "smoke_satisfied": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false
}
JSON
if python3 "$script" --input "$bad_summary" --out "$bad_out" 2>/dev/null; then
  fail "current-stage handoff accepted a private local path marker"
fi

printf '%s\n' "current-stage handoff check passed"

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
full_comment_out="$tmpdir/full-issue-comment.md"
query_set_index_blocked_summary="$tmpdir/PRIVATE-query-set-index-blocked-summary.json"
query_set_index_blocked_out="$tmpdir/query-set-index-blocked-handoff.json"
query_set_index_blocked_comment_out="$tmpdir/query-set-index-blocked-issue-comment.md"
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
    "document_count": 10000,
    "searchable_document_count": 200,
    "vector_indexed_document_count": 200,
    "hot_index_fully_covered": false,
    "document_status_counts": {
      "ocr_required": 9800,
      "searchable": 200
    },
    "ingest_job_status_counts": {
      "completed": 200,
      "queued": 9800
    },
    "ingest_job_kind_status_counts": {
      "ocr_document": {
        "queued": 9800
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
require_handoff_observability "$blocked_out" 10000
reject_text "$blocked_out" "$tmpdir"
reject_text "$blocked_out" "PRIVATE-current-stage"
reject_regex "$blocked_out" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"

cat > "$query_set_index_blocked_summary" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "full",
  "current_stage_target": "reproducible_local_10k_baseline",
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "query_set_prepare",
  "blocked_category": "query-set",
  "blocked_reason": "query_set_index_unavailable",
  "blocked_exit": 1,
  "private_corpus_read": true,
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": 10000,
    "searchable_document_count": 8000,
    "vector_indexed_document_count": 8000,
    "hot_index_fully_covered": true,
    "document_status_counts": {
      "ocr_required": 2000,
      "searchable": 8000
    },
    "ingest_job_status_counts": {
      "completed": 8000,
      "queued": 2000
    },
    "ingest_job_kind_status_counts": {
      "ocr_document": {
        "queued": 2000
      },
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
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "blocked"}
  ],
  "redacted_outputs": [
    {
      "file": "query-set-trace-preflight.local.json",
      "sha256": "4444444444444444444444444444444444444444444444444444444444444444"
    }
  ],
  "not_completed": [
    "local private query-set generation"
  ],
  "must_not_upload": [
    "raw queries",
    "indexes"
  ]
}
JSON

python3 "$script" \
  --input "$query_set_index_blocked_summary" \
  --out "$query_set_index_blocked_out" \
  --issue-comment-out "$query_set_index_blocked_comment_out"
python3 -m json.tool "$query_set_index_blocked_out" >/dev/null
require_text "$query_set_index_blocked_out" '"blocked_reason": "query_set_index_unavailable"'
require_text "$query_set_index_blocked_out" '"blocked_artifact_refs": ['
require_text "$query_set_index_blocked_out" '"file": "query-set-trace-preflight.local.json"'
require_text "$query_set_index_blocked_out" '"sha256": "4444444444444444444444444444444444444444444444444444444444444444"'
require_text "$query_set_index_blocked_out" '"recommended_next_step": "prepare or reuse an indexed local data-dir, then rerun current-stage validation with the static replay query-set freeze"'
require_text "$query_set_index_blocked_out" '"do_not_do": "do not run private-query baseline, D10K calibration, or P95/P99 optimization until query-set freeze succeeds"'
require_text "$query_set_index_blocked_comment_out" "#53 Current-Stage Blocked Handoff"
require_text "$query_set_index_blocked_comment_out" "blocked_step: query_set_prepare"
require_text "$query_set_index_blocked_comment_out" "blocked_reason: query_set_index_unavailable"
require_text "$query_set_index_blocked_comment_out" "redacted_artifact_hash: 4444444444444444444444444444444444444444444444444444444444444444 (query-set-trace-preflight.local.json)"
require_text "$query_set_index_blocked_comment_out" "do not run private-query baseline, D10K calibration, or P95/P99 optimization until query-set freeze succeeds"
require_text "$query_set_index_blocked_comment_out" "not goal_complete; not a profile optimization issue closure"
reject_text "$query_set_index_blocked_out" "$tmpdir"
reject_text "$query_set_index_blocked_out" "PRIVATE-current-stage"
reject_regex "$query_set_index_blocked_out" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"
reject_text "$query_set_index_blocked_comment_out" "$tmpdir"
reject_text "$query_set_index_blocked_comment_out" "PRIVATE-current-stage"
reject_regex "$query_set_index_blocked_comment_out" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"

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
    "document_count": 10000,
    "searchable_document_count": 8000,
    "vector_indexed_document_count": 8000,
    "hot_index_fully_covered": true,
    "document_status_counts": {
      "ocr_required": 2000,
      "searchable": 8000
    },
    "ingest_job_status_counts": {
      "completed": 8000,
      "queued": 2000
    },
    "ingest_job_kind_status_counts": {
      "ocr_document": {
        "queued": 2000
      },
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
  "private_query_observability": {
    "privacy_boundary": "redacted_local_aggregate",
    "dataset_kind": "private-real-corpus",
    "document_count": 10000,
    "searchable_document_count": 8000,
    "vector_indexed_document_count": 8000,
    "query_count": 500,
    "request_sample_count": 5000,
    "query_source": "trace_source_search_v1",
    "private_scale_gate": "D10K_private_calibration",
    "query_set_sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
    "tune_sha256": "2222222222222222222222222222222222222222222222222222222222222222",
    "holdout_sha256": "3333333333333333333333333333333333333333333333333333333333333333",
    "bucket_counts": {
      "single_term": 50,
      "and_2": 75,
      "and_3_5":150,
      "and_6_16": 50,
      "field_filter": 75,
      "hybrid": 75,
      "semantic": 25
    },
    "tune_bucket_counts": {
      "single_term": 40,
      "and_2": 60,
      "and_3_5":120,
      "and_6_16": 40,
      "field_filter": 60,
      "hybrid": 60,
      "semantic": 20
    },
    "holdout_bucket_counts": {
      "single_term": 10,
      "and_2": 15,
      "and_3_5":30,
      "and_6_16": 10,
      "field_filter": 15,
      "hybrid": 15,
      "semantic": 5
    },
    "samples_per_bucket": {
      "single_term": 500,
      "and_2": 625,
      "and_3_5":1500,
      "and_6_16": 500,
      "field_filter": 625,
      "hybrid": 625,
      "semantic": 625
    },
    "query_latency_ms": {"samples": 5000, "p50": 5.0, "p95": 42.0, "p99": 84.0},
    "query_latency_by_bucket": {
      "hybrid": {"samples": 625, "p50": 5.0, "p95": 42.0, "p99": 84.0}
    },
    "stage_latency_p95_ms": {
      "query_parse": 2.0,
      "prefilter": 4.0,
      "bm25": 42.0,
      "ann": 12.0,
      "fusion": 3.0,
      "bulk_hydrate": 9.0,
      "snippet": 6.0
    },
    "stage_latency_by_bucket_p95_ms": {
      "hybrid": {
        "query_parse": 2.0,
        "prefilter": 4.0,
        "bm25": 42.0,
        "ann": 12.0,
        "fusion": 3.0,
        "bulk_hydrate": 9.0,
        "snippet": 6.0
      }
    },
    "stage_histogram_ms": {
      "query_parse": {
        "samples": 5000,
        "bins": [
          {"le_ms": 1.0, "count": 1000},
          {"le_ms": 5.0, "count": 5000}
        ],
        "overflow_count": 0
      },
      "bm25": {
        "samples": 5000,
        "bins": [
          {"le_ms": 1.0, "count": 100},
          {"le_ms": 5.0, "count": 5000}
        ],
        "overflow_count": 0
      }
    },
    "stage_histogram_by_bucket_ms": {
      "hybrid": {
        "query_parse": {
          "samples": 625,
          "bins": [
            {"le_ms": 1.0, "count": 125},
            {"le_ms": 5.0, "count": 625}
          ],
          "overflow_count": 0
        },
        "bm25": {
          "samples": 625,
          "bins": [
            {"le_ms": 1.0, "count": 25},
            {"le_ms": 5.0, "count": 625}
          ],
          "overflow_count": 0
        }
      }
    },
    "rss_delta_mb": {"samples": 5000, "p50": 0.0, "p95": 0.0, "p99": 0.0},
    "rss_delta_mb_by_bucket": {
      "hybrid": {"samples": 625, "p50": 0.0, "p95": 0.0, "p99": 0.0}
    },
    "zero_result_queries": 0,
    "query_runner": "resident-batch-command",
    "query_mode": "hybrid",
    "retrieval_layers": "fulltext+field+vector+rrf",
    "warm_or_cold_definition": "current_stage_single_resident_batch_no_extra_warmup",
    "cache_state": "hot_index_fully_covered_resident_batch_os_cache_uncontrolled",
    "percentile_confidence": "sampled",
    "spawn_per_query": false,
    "hot_index": true,
    "hot_path_ocr": false,
    "hot_path_parsing": false,
    "hot_path_heavy_model_inference": false,
    "contains_raw_resume_text": false,
    "contains_resume_paths": false,
    "contains_queries": false
  },
  "redacted_outputs": [
    {
      "file": "private-query-set.local.jsonl",
      "sha256": "9999999999999999999999999999999999999999999999999999999999999999"
    },
    {
      "file": "private-query-set.summary.json",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    },
    {
      "file": "private-benchmark-local.json",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    }
  ],
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

python3 "$script" --input "$full_evidence" --out "$full_out" --issue-comment-out "$full_comment_out"
python3 -m json.tool "$full_out" >/dev/null
require_text "$full_out" '"source_schema": "resume-ir.current-stage-validation-evidence.v1"'
require_text "$full_out" '"current_stage_status": "full_evidence_ready"'
require_text "$full_out" '"validation_profile": "full"'
require_text "$full_out" '"complete_product": false'
require_text "$full_out" '"full_baseline_satisfied": true'
require_text "$full_out" '"release_readiness_evidence": true'
require_text "$full_out" '"release_readiness_intake"'
require_handoff_observability "$full_out" 10000
require_text "$full_out" '"private_query_baseline_summary": {'
require_text "$full_out" '"query_source": "trace_source_search_v1"'
require_text "$full_out" '"private_scale_gate": "D10K_private_calibration"'
require_text "$full_out" '"query_set_sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"'
require_text "$full_out" '"request_sample_count": 5000'
require_text "$full_out" '"query_runner": "resident-batch-command"'
require_text "$full_out" '"spawn_per_query": false'
require_text "$full_out" '"stage_histogram_summary": {'
require_text "$full_out" '"global_stage_count": 2'
require_text "$full_out" '"bucket_count": 1'
require_text "$full_out" '"histogram_bin_count": 2'
require_text "$full_out" '"overflow_included": true'
require_text "$full_out" '"baseline_artifact_refs": ['
require_text "$full_out" '"file": "private-benchmark-local.json"'
require_text "$full_out" '"sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"'
require_text "$full_out" '"file": "private-query-set.summary.json"'
require_text "$full_out" '"sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"'
reject_text "$full_out" "private-query-set.local.jsonl"
reject_text "$full_out" "$tmpdir"
reject_text "$full_out" "PRIVATE-current-stage"
reject_regex "$full_out" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"
require_text "$full_comment_out" "#53 Current-Stage Private Query Baseline Handoff"
require_text "$full_comment_out" "query_source: trace_source_search_v1"
require_text "$full_comment_out" "private_scale_gate: D10K_private_calibration"
require_text "$full_comment_out" "query_set_sha256: abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
require_text "$full_comment_out" "request_sample_count: 5000"
require_text "$full_comment_out" "query_runner: resident-batch-command"
require_text "$full_comment_out" "spawn_per_query: false"
require_text "$full_comment_out" "stage_histogram_shape: global_stages=2, buckets=1, bins=2, overflow=true"
require_text "$full_comment_out" "benchmark_report_hash: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb (private-benchmark-local.json)"
require_text "$full_comment_out" "query_set_summary_hash: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa (private-query-set.summary.json)"
require_text "$full_comment_out" "contains_queries: false"
require_text "$full_comment_out" "not goal_complete; not a profile optimization issue closure"
reject_text "$full_comment_out" "private-query-set.local.jsonl"
reject_text "$full_comment_out" "$tmpdir"
reject_text "$full_comment_out" "PRIVATE-current-stage"
reject_regex "$full_comment_out" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"

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

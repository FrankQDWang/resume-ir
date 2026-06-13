#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage: scripts/local/run-current-stage-validation.sh [--dry-run|--execute]
  --resume-root DIR --data-dir DIR --out-dir DIR [--query-set FILE]
  [--validation-profile full|smoke]
  --model-manifest FILE --ocr-runtime-manifest FILE
  --model-artifact FILE --embedding-command FILE
  --model-pack-id ID --model-id ID --model-format ID --dimension N --model-license ID
  --runtime-pack-id ID --tesseract-command FILE --pdftoppm-command FILE
  --language LANG --language-pack FILE|LANG=FILE [--language-pack LANG=FILE ...]
  --engine-license ID --renderer-license ID --language-license ID
  [--dataset-manifest-sha256 SHA256] [--query-set-sha256 SHA256]
  [--model-manifest-sha256 SHA256]
  [--ocr-runtime-manifest-sha256 SHA256]
  [--renderer-manifest-sha256 SHA256]
  [--language-pack-manifest-sha256 SHA256]
  [--resume-cli PATH] [--resume-daemon PATH] [--resume-benchmark PATH]
  [--reviewed-model] [--reviewed-ocr-runtime]
  [--max-files N] [--max-queries N] [--top-k N]
  [--worker-interval-ms N] [--ocr-worker-ticks N] [--embedding-worker-ticks N]
  [--ocr-throughput-max-documents N] [--ocr-throughput-max-pages N]
  [--ocr-throughput-pages-per-document N] [--ocr-throughput-max-run-ms N]
  [--ocr-throughput-min-pages N]

Default mode is --dry-run and default validation profile is full. Dry-run prints
a redacted JSON plan and never reads the private resume root. Execute mode runs
local-only commands and writes local evidence under --out-dir. The smoke profile
proves wiring only; it does not produce release-readiness evidence.
EOF
  exit 2
}

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

need_value() {
  [ "$#" -ge 2 ] || usage
  [ -n "$2" ] || usage
}

require_arg() {
  name="$1"
  value="$2"
  [ -n "$value" ] || fail "missing required argument: $name"
}

require_positive_int() {
  name="$1"
  value="$2"
  case "$value" in
    ''|*[!0-9]*) fail "$name must be a positive integer" ;;
    0) fail "$name must be a positive integer" ;;
  esac
}

require_sha256() {
  name="$1"
  value="$2"
  case "$value" in
    '') fail "missing required digest: $name" ;;
    *[!0123456789abcdefABCDEF]*)
      fail "$name must be a hex sha256 digest"
      ;;
    *)
      [ "${#value}" -eq 64 ] || fail "$name must be a 64-character sha256 digest"
      ;;
  esac
}

sha256_file() {
  path="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
  elif command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  else
    fail "sha256 tool is required"
  fi
}

sha256_file_json_or_null() {
  path="$1"
  if [ -e "$path" ]; then
    printf '"%s"' "$(sha256_file "$path")"
  else
    printf 'null'
  fi
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
handoff_summarizer="$script_dir/summarize-current-stage-validation.py"

write_current_stage_handoff() {
  source_json="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for current-stage handoff"
  [ -f "$handoff_summarizer" ] || fail "current-stage handoff summarizer is unavailable"
  python3 "$handoff_summarizer" \
    --input "$source_json" \
    --out "$out_dir/current-stage-handoff.json" \
    >/dev/null || fail "current-stage handoff generation failed"
}

corpus_summary_observability_json() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for redacted corpus summary observability"
  python3 - "$path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    report = json.load(handle)

if report.get("privacy_boundary") != "redacted_local_aggregate":
    raise SystemExit("corpus summary privacy boundary failed")

for sentinel in (
    "contains_raw_resume_text",
    "contains_resume_paths",
    "contains_queries",
    "contains_sample_ids",
):
    if report.get(sentinel) is not False:
        raise SystemExit("corpus summary privacy sentinel failed")


def integer_field(name):
    value = report.get(name)
    if not isinstance(value, int) or value < 0:
        raise SystemExit(f"corpus summary field failed: {name}")
    return value


def boolean_field(name):
    value = report.get(name)
    if not isinstance(value, bool):
        raise SystemExit(f"corpus summary field failed: {name}")
    return value


def object_field(name):
    value = report.get(name, {})
    if not isinstance(value, dict):
        raise SystemExit(f"corpus summary field failed: {name}")
    return value


observability = {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": integer_field("document_count"),
    "searchable_document_count": integer_field("searchable_document_count"),
    "vector_indexed_document_count": integer_field("vector_indexed_document_count"),
    "hot_index_fully_covered": boolean_field("hot_index_fully_covered"),
    "document_status_counts": object_field("document_status_counts"),
    "ingest_job_status_counts": object_field("ingest_job_status_counts"),
    "ingest_job_kind_status_counts": object_field("ingest_job_kind_status_counts"),
    "ingest_job_failure_counts": object_field("ingest_job_failure_counts"),
}

json.dump(observability, sys.stdout, ensure_ascii=True, sort_keys=True, indent=2)
sys.stdout.write("\n")
PY
}

corpus_summary_has_bounded_ocr_backlog() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for OCR backlog classification"
  set +e
  python3 - "$path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    report = json.load(handle)

if report.get("privacy_boundary") != "redacted_local_aggregate":
    raise SystemExit(2)

for sentinel in (
    "contains_raw_resume_text",
    "contains_resume_paths",
    "contains_queries",
    "contains_sample_ids",
):
    if report.get(sentinel) is not False:
        raise SystemExit(2)


def int_field(name):
    value = report.get(name)
    if not isinstance(value, int) or value < 0:
        raise SystemExit(2)
    return value


def dict_field(name):
    value = report.get(name, {})
    if not isinstance(value, dict):
        raise SystemExit(2)
    return value


def count_from(mapping, key):
    value = mapping.get(key, 0)
    if not isinstance(value, int) or value < 0:
        raise SystemExit(2)
    return value


hot_index_fully_covered = report.get("hot_index_fully_covered")
if not isinstance(hot_index_fully_covered, bool):
    raise SystemExit(2)

document_count = int_field("document_count")
searchable_document_count = int_field("searchable_document_count")
vector_indexed_document_count = int_field("vector_indexed_document_count")
document_status_counts = dict_field("document_status_counts")
ocr_required_count = count_from(document_status_counts, "ocr_required")

if (
    document_count > 0
    and ocr_required_count > 0
    and not hot_index_fully_covered
    and (
        searchable_document_count < document_count
        or vector_indexed_document_count < document_count
    )
):
    raise SystemExit(0)

raise SystemExit(1)
PY
  status=$?
  set -e
  if [ "$status" -eq 2 ]; then
    fail "current-stage corpus summary privacy/shape validation failed"
  fi
  [ "$status" -eq 0 ]
}

write_runtime_preflight_blocked_summary() {
  blocked_step="$1"
  blocked_category="$2"
  blocked_reason="$3"
  blocked_exit="$4"

  [ -e "$out_dir/ocr-preflight.json" ] || : > "$out_dir/ocr-preflight.json"

  ocr_preflight_sha256_json=$(sha256_file_json_or_null "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256_json=$(sha256_file_json_or_null "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256_json=$(sha256_file_json_or_null "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256_json=$(sha256_file_json_or_null "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256_json=$(sha256_file_json_or_null "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256_json=$(sha256_file_json_or_null "$out_dir/model-preflight.json")
  ocr_runtime_manifest_sha256_json=$(sha256_file_json_or_null "$ocr_runtime_manifest")
  model_manifest_sha256_json=$(sha256_file_json_or_null "$model_manifest")

  case "$blocked_step" in
    ocr_preflight)
      ocr_probe_status="blocked"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    ocr_manifest_draft)
      ocr_probe_status="passed"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    ocr_manifest_validate)
      ocr_probe_status="passed"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    model_manifest_draft)
      ocr_probe_status="passed"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    model_manifest_validate)
      ocr_probe_status="passed"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    model_preflight)
      ocr_probe_status="passed"
      embedding_protocol_status="blocked"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    *)
      ocr_probe_status="blocked"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "$blocked_step", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
  esac

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "private_corpus_read": false,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "$blocked_step",
  "blocked_category": "$blocked_category",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": null,
    "query_set_sha256": null,
    "model_manifest_sha256": $model_manifest_sha256_json,
    "ocr_runtime_manifest_sha256": $ocr_runtime_manifest_sha256_json
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "$ocr_probe_status",
    "embedding_protocol": "$embedding_protocol_status"
  },
  "steps": [
$steps_json
  ],
  "redacted_outputs": [
    {"file": "ocr-preflight.json", "sha256": $ocr_preflight_sha256_json},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": $ocr_draft_stdout_sha256_json},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": $ocr_validate_stdout_sha256_json},
    {"file": "ocr-runtime-manifest.local.json", "sha256": $ocr_runtime_manifest_sha256_json},
    {"file": "model-draft-manifest.stdout.txt", "sha256": $model_draft_stdout_sha256_json},
    {"file": "model-validate-manifest.stdout.txt", "sha256": $model_validate_stdout_sha256_json},
    {"file": "model-manifest.local.json", "sha256": $model_manifest_sha256_json},
    {"file": "model-preflight.json", "sha256": $model_preflight_sha256_json}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "private corpus read",
    "runtime preflight",
    "dataset manifest",
    "import/OCR/embedding workers",
    "current-stage validation evidence",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_import_parser_blocked_summary() {
  blocked_step="$1"
  blocked_reason="$2"
  blocked_exit="$3"

  [ -e "$out_dir/dataset-manifest.stdout.txt" ] || : > "$out_dir/dataset-manifest.stdout.txt"
  [ -e "$out_dir/import.stdout.txt" ] || : > "$out_dir/import.stdout.txt"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  dataset_manifest_sha256_json=$(sha256_file_json_or_null "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")

  if [ "$blocked_step" = "dataset_manifest" ]; then
    private_corpus_read="true"
    steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
  else
    private_corpus_read="true"
    steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
  fi

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "private_corpus_read": $private_corpus_read,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "$blocked_step",
  "blocked_category": "import/parser",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": $dataset_manifest_sha256_json,
    "query_set_sha256": null,
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "steps": [
$steps_json
  ],
  "redacted_outputs": [
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "dataset-manifest.local.json", "sha256": $dataset_manifest_sha256_json},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "successful private corpus import",
    "OCR worker bounded run",
    "embedding worker bounded run",
    "corpus summary",
    "query-set draft",
    "private query baseline",
    "redacted diagnostics",
    "release-readiness current-stage evidence",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_query_set_blocked_summary() {
  blocked_exit="$1"
  blocked_reason="$2"
  [ -e "$out_dir/query-set-draft.stdout.txt" ] || {
    printf '%s\n' "query set: blocked"
    printf '%s\n' "schema: resume-ir.query-set.jsonl.v1"
    printf '%s\n' "privacy boundary: local_only_private_query_set"
    printf '%s\n' "queries: <redacted>"
    printf '%s\n' "paths: <redacted>"
  } > "$out_dir/query-set-draft.stdout.txt"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_worker_stdout_sha256=$(sha256_file "$out_dir/ocr-worker.stdout.txt")
  embedding_worker_stdout_sha256=$(sha256_file "$out_dir/embedding-worker.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_draft_stdout_sha256=$(sha256_file "$out_dir/query-set-draft.stdout.txt")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "query_set_draft",
  "blocked_category": "query-set",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": null,
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "success"},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_draft", "status": "blocked", "exit_code": $blocked_exit}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-worker.stdout.txt", "sha256": "$ocr_worker_stdout_sha256"},
    {"file": "embedding-worker.stdout.txt", "sha256": "$embedding_worker_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "query-set-draft.stdout.txt", "sha256": "$query_set_draft_stdout_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "local private query-set generation",
    "private query baseline",
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "redacted diagnostics for this blocked run",
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_private_query_blocked_summary() {
  blocked_exit="$1"
  blocked_reason="$2"
  [ -e "$out_dir/private-benchmark-local.json" ] || : > "$out_dir/private-benchmark-local.json"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_worker_stdout_sha256=$(sha256_file "$out_dir/ocr-worker.stdout.txt")
  embedding_worker_stdout_sha256=$(sha256_file "$out_dir/embedding-worker.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_draft_stdout_sha256=$(sha256_file "$out_dir/query-set-draft.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "private_query_baseline",
  "blocked_category": "benchmark",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "success"},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_draft", "status": "success"},
    {"id": "private_query_baseline", "status": "blocked", "exit_code": $blocked_exit}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-worker.stdout.txt", "sha256": "$ocr_worker_stdout_sha256"},
    {"file": "embedding-worker.stdout.txt", "sha256": "$embedding_worker_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "query-set-draft.stdout.txt", "sha256": "$query_set_draft_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "private query baseline",
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "redacted diagnostics for this blocked run",
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_ocr_throughput_blocked_summary() {
  blocked_step="$1"
  blocked_reason="$2"
  blocked_exit="$3"
  [ -e "$out_dir/private-ocr-throughput.json" ] || : > "$out_dir/private-ocr-throughput.json"
  [ -e "$out_dir/ocr-throughput-gate.stdout.txt" ] || : > "$out_dir/ocr-throughput-gate.stdout.txt"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_worker_stdout_sha256=$(sha256_file "$out_dir/ocr-worker.stdout.txt")
  embedding_worker_stdout_sha256=$(sha256_file "$out_dir/embedding-worker.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_draft_stdout_sha256=$(sha256_file "$out_dir/query-set-draft.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  private_ocr_throughput_sha256=$(sha256_file "$out_dir/private-ocr-throughput.json")
  ocr_throughput_gate_sha256=$(sha256_file "$out_dir/ocr-throughput-gate.stdout.txt")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  if [ "$blocked_step" = "private_ocr_throughput_baseline" ]; then
    ocr_throughput_steps=$(cat <<EOF_STEPS
    {"id": "private_ocr_throughput_baseline", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
  else
    ocr_throughput_steps=$(cat <<EOF_STEPS
    {"id": "private_ocr_throughput_baseline", "status": "success"},
    {"id": "ocr_throughput_baseline_gate", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
  fi

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "$blocked_step",
  "blocked_category": "ocr",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries,
    "ocr_throughput_min_pages": $ocr_throughput_min_pages
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "success"},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_draft", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
$ocr_throughput_steps
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-worker.stdout.txt", "sha256": "$ocr_worker_stdout_sha256"},
    {"file": "embedding-worker.stdout.txt", "sha256": "$embedding_worker_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "query-set-draft.stdout.txt", "sha256": "$query_set_draft_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "private-ocr-throughput.json", "sha256": "$private_ocr_throughput_sha256"},
    {"file": "ocr-throughput-gate.stdout.txt", "sha256": "$ocr_throughput_gate_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "private real-corpus OCR throughput baseline",
    "redacted diagnostics for this blocked run",
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_ocr_backlog_blocked_summary() {
  blocked_exit=1
  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_worker_stdout_sha256=$(sha256_file "$out_dir/ocr-worker.stdout.txt")
  embedding_worker_stdout_sha256=$(sha256_file "$out_dir/embedding-worker.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "ocr_worker_bounded_loop",
  "blocked_category": "ocr",
  "blocked_reason": "ocr_backlog_exceeds_current_stage_budget",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": null,
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "blocked", "exit_code": $blocked_exit},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-worker.stdout.txt", "sha256": "$ocr_worker_stdout_sha256"},
    {"file": "embedding-worker.stdout.txt", "sha256": "$embedding_worker_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "full OCR backlog drain",
    "private query-set draft",
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "private real-corpus OCR throughput baseline",
    "redacted diagnostics for this blocked run",
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_redacted_diagnostics_blocked_summary() {
  blocked_exit="$1"
  blocked_reason="$2"
  [ -e "$out_dir/redacted-diagnostics.json" ] || : > "$out_dir/redacted-diagnostics.json"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_worker_stdout_sha256=$(sha256_file "$out_dir/ocr-worker.stdout.txt")
  embedding_worker_stdout_sha256=$(sha256_file "$out_dir/embedding-worker.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_draft_stdout_sha256=$(sha256_file "$out_dir/query-set-draft.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  private_ocr_throughput_sha256=$(sha256_file "$out_dir/private-ocr-throughput.json")
  ocr_throughput_gate_sha256=$(sha256_file "$out_dir/ocr-throughput-gate.stdout.txt")
  redacted_diagnostics_sha256=$(sha256_file "$out_dir/redacted-diagnostics.json")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "redacted_diagnostics",
  "blocked_category": "diagnostics",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "success"},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_draft", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
    {"id": "private_ocr_throughput_baseline", "status": "success"},
    {"id": "ocr_throughput_baseline_gate", "status": "success"},
    {"id": "redacted_diagnostics", "status": "blocked", "exit_code": $blocked_exit}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-worker.stdout.txt", "sha256": "$ocr_worker_stdout_sha256"},
    {"file": "embedding-worker.stdout.txt", "sha256": "$embedding_worker_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "query-set-draft.stdout.txt", "sha256": "$query_set_draft_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "private-ocr-throughput.json", "sha256": "$private_ocr_throughput_sha256"},
    {"file": "ocr-throughput-gate.stdout.txt", "sha256": "$ocr_throughput_gate_sha256"},
    {"file": "redacted-diagnostics.json", "sha256": "$redacted_diagnostics_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "redacted diagnostics for this run",
    "release-readiness current-stage evidence",
    "full 10k/8000-document current-stage baseline",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_release_readiness_blocked_summary() {
  blocked_exit="$1"
  blocked_reason="$2"
  [ -e "$out_dir/release-readiness.json" ] || : > "$out_dir/release-readiness.json"
  [ -e "$out_dir/release-readiness.stderr.txt" ] || : > "$out_dir/release-readiness.stderr.txt"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_worker_stdout_sha256=$(sha256_file "$out_dir/ocr-worker.stdout.txt")
  embedding_worker_stdout_sha256=$(sha256_file "$out_dir/embedding-worker.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_draft_stdout_sha256=$(sha256_file "$out_dir/query-set-draft.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  private_ocr_throughput_sha256=$(sha256_file "$out_dir/private-ocr-throughput.json")
  ocr_throughput_gate_sha256=$(sha256_file "$out_dir/ocr-throughput-gate.stdout.txt")
  redacted_diagnostics_sha256=$(sha256_file "$out_dir/redacted-diagnostics.json")
  release_readiness_sha256=$(sha256_file "$out_dir/release-readiness.json")
  release_readiness_stderr_sha256=$(sha256_file "$out_dir/release-readiness.stderr.txt")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "private_corpus_read": true,
  "full_baseline_satisfied": true,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "release_readiness_intake",
  "blocked_category": "release-readiness",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "success"},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_draft", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
    {"id": "private_ocr_throughput_baseline", "status": "success"},
    {"id": "ocr_throughput_baseline_gate", "status": "success"},
    {"id": "redacted_diagnostics", "status": "success"},
    {"id": "release_readiness_intake", "status": "blocked", "exit_code": $blocked_exit}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-worker.stdout.txt", "sha256": "$ocr_worker_stdout_sha256"},
    {"file": "embedding-worker.stdout.txt", "sha256": "$embedding_worker_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "query-set-draft.stdout.txt", "sha256": "$query_set_draft_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "private-ocr-throughput.json", "sha256": "$private_ocr_throughput_sha256"},
    {"file": "ocr-throughput-gate.stdout.txt", "sha256": "$ocr_throughput_gate_sha256"},
    {"file": "redacted-diagnostics.json", "sha256": "$redacted_diagnostics_sha256"},
    {"file": "release-readiness.json", "sha256": "$release_readiness_sha256"},
    {"file": "release-readiness.stderr.txt", "sha256": "$release_readiness_stderr_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "accepted release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

require_text_in_file() {
  path="$1"
  text="$2"
  message="$3"
  if ! grep -Fq -- "$text" "$path"; then
    fail "$message"
  fi
}

mode="dry-run"
resume_cli="${RESUME_CLI:-resume-cli}"
resume_daemon="${RESUME_DAEMON:-resume-daemon}"
resume_benchmark="${RESUME_BENCHMARK:-resume-benchmark}"
resume_root=""
data_dir=""
out_dir=""
query_set=""
validation_profile="full"
model_manifest=""
ocr_runtime_manifest=""
model_artifact=""
embedding_command=""
model_pack_id=""
model_id=""
model_format=""
dimension=""
model_license=""
runtime_pack_id=""
tesseract_command=""
pdftoppm_command=""
language=""
language_pack_args=""
language_pack_count=0
engine_license=""
renderer_license=""
language_license=""
dataset_manifest_sha256=""
query_set_sha256=""
model_manifest_sha256=""
ocr_runtime_manifest_sha256=""
renderer_manifest_sha256=""
language_pack_manifest_sha256=""
reviewed_model="false"
reviewed_ocr_runtime="false"
max_files="10000"
max_queries="500"
top_k="10"
worker_interval_ms="1"
ocr_worker_ticks="10000"
embedding_worker_ticks="10000"
ocr_max_pages_per_document="20"
ocr_page_timeout_ms="30000"
ocr_render_dpi="150"
embedding_max_docs="128"
embedding_max_text_bytes="1000000"
embedding_timeout_ms="30000"
ocr_throughput_max_documents="900"
ocr_throughput_max_pages="500"
ocr_throughput_pages_per_document="1"
ocr_throughput_max_run_ms="3600000"
ocr_throughput_min_pages="500"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --dry-run)
      mode="dry-run"
      shift
      ;;
    --execute)
      mode="execute"
      shift
      ;;
    --resume-cli)
      need_value "$@"; resume_cli="$2"; shift 2
      ;;
    --resume-daemon)
      need_value "$@"; resume_daemon="$2"; shift 2
      ;;
    --resume-benchmark)
      need_value "$@"; resume_benchmark="$2"; shift 2
      ;;
    --resume-root)
      need_value "$@"; resume_root="$2"; shift 2
      ;;
    --data-dir)
      need_value "$@"; data_dir="$2"; shift 2
      ;;
    --out-dir)
      need_value "$@"; out_dir="$2"; shift 2
      ;;
    --query-set)
      need_value "$@"; query_set="$2"; shift 2
      ;;
    --validation-profile)
      need_value "$@"; validation_profile="$2"; shift 2
      ;;
    --model-manifest)
      need_value "$@"; model_manifest="$2"; shift 2
      ;;
    --ocr-runtime-manifest)
      need_value "$@"; ocr_runtime_manifest="$2"; shift 2
      ;;
    --model-artifact)
      need_value "$@"; model_artifact="$2"; shift 2
      ;;
    --embedding-command)
      need_value "$@"; embedding_command="$2"; shift 2
      ;;
    --model-pack-id)
      need_value "$@"; model_pack_id="$2"; shift 2
      ;;
    --model-id)
      need_value "$@"; model_id="$2"; shift 2
      ;;
    --model-format)
      need_value "$@"; model_format="$2"; shift 2
      ;;
    --dimension)
      need_value "$@"; dimension="$2"; shift 2
      ;;
    --model-license)
      need_value "$@"; model_license="$2"; shift 2
      ;;
    --runtime-pack-id)
      need_value "$@"; runtime_pack_id="$2"; shift 2
      ;;
    --tesseract-command)
      need_value "$@"; tesseract_command="$2"; shift 2
      ;;
    --pdftoppm-command)
      need_value "$@"; pdftoppm_command="$2"; shift 2
      ;;
    --language)
      need_value "$@"; language="$2"; shift 2
      ;;
    --language-pack)
      need_value "$@"
      if [ "$language_pack_count" -eq 0 ]; then
        language_pack_args="$2"
      else
        language_pack_args="$language_pack_args
$2"
      fi
      language_pack_count=$((language_pack_count + 1))
      shift 2
      ;;
    --engine-license)
      need_value "$@"; engine_license="$2"; shift 2
      ;;
    --renderer-license)
      need_value "$@"; renderer_license="$2"; shift 2
      ;;
    --language-license)
      need_value "$@"; language_license="$2"; shift 2
      ;;
    --dataset-manifest-sha256)
      need_value "$@"; dataset_manifest_sha256="$2"; shift 2
      ;;
    --query-set-sha256)
      need_value "$@"; query_set_sha256="$2"; shift 2
      ;;
    --model-manifest-sha256)
      need_value "$@"; model_manifest_sha256="$2"; shift 2
      ;;
    --ocr-runtime-manifest-sha256)
      need_value "$@"; ocr_runtime_manifest_sha256="$2"; shift 2
      ;;
    --renderer-manifest-sha256)
      need_value "$@"; renderer_manifest_sha256="$2"; shift 2
      ;;
    --language-pack-manifest-sha256)
      need_value "$@"; language_pack_manifest_sha256="$2"; shift 2
      ;;
    --reviewed-model)
      reviewed_model="true"
      shift
      ;;
    --reviewed-ocr-runtime)
      reviewed_ocr_runtime="true"
      shift
      ;;
    --max-files)
      need_value "$@"; max_files="$2"; shift 2
      ;;
    --max-queries)
      need_value "$@"; max_queries="$2"; shift 2
      ;;
    --top-k)
      need_value "$@"; top_k="$2"; shift 2
      ;;
    --worker-interval-ms)
      need_value "$@"; worker_interval_ms="$2"; shift 2
      ;;
    --ocr-worker-ticks)
      need_value "$@"; ocr_worker_ticks="$2"; shift 2
      ;;
    --embedding-worker-ticks)
      need_value "$@"; embedding_worker_ticks="$2"; shift 2
      ;;
    --ocr-max-pages-per-document)
      need_value "$@"; ocr_max_pages_per_document="$2"; shift 2
      ;;
    --ocr-page-timeout-ms)
      need_value "$@"; ocr_page_timeout_ms="$2"; shift 2
      ;;
    --ocr-render-dpi)
      need_value "$@"; ocr_render_dpi="$2"; shift 2
      ;;
    --embedding-max-docs)
      need_value "$@"; embedding_max_docs="$2"; shift 2
      ;;
    --embedding-max-text-bytes)
      need_value "$@"; embedding_max_text_bytes="$2"; shift 2
      ;;
    --embedding-timeout-ms)
      need_value "$@"; embedding_timeout_ms="$2"; shift 2
      ;;
    --ocr-throughput-max-documents)
      need_value "$@"; ocr_throughput_max_documents="$2"; shift 2
      ;;
    --ocr-throughput-max-pages)
      need_value "$@"; ocr_throughput_max_pages="$2"; shift 2
      ;;
    --ocr-throughput-pages-per-document)
      need_value "$@"; ocr_throughput_pages_per_document="$2"; shift 2
      ;;
    --ocr-throughput-max-run-ms)
      need_value "$@"; ocr_throughput_max_run_ms="$2"; shift 2
      ;;
    --ocr-throughput-min-pages)
      need_value "$@"; ocr_throughput_min_pages="$2"; shift 2
      ;;
    -h|--help)
      usage
      ;;
    *)
      usage
      ;;
  esac
done

require_arg "--resume-root" "$resume_root"
require_arg "--data-dir" "$data_dir"
require_arg "--out-dir" "$out_dir"
require_arg "--model-manifest" "$model_manifest"
require_arg "--ocr-runtime-manifest" "$ocr_runtime_manifest"
require_arg "--model-artifact" "$model_artifact"
require_arg "--embedding-command" "$embedding_command"
require_arg "--model-pack-id" "$model_pack_id"
require_arg "--model-id" "$model_id"
require_arg "--model-format" "$model_format"
require_arg "--dimension" "$dimension"
require_arg "--model-license" "$model_license"
require_arg "--runtime-pack-id" "$runtime_pack_id"
require_arg "--tesseract-command" "$tesseract_command"
require_arg "--pdftoppm-command" "$pdftoppm_command"
require_arg "--language" "$language"
[ "$language_pack_count" -gt 0 ] || fail "missing required argument: --language-pack"
require_arg "--engine-license" "$engine_license"
require_arg "--renderer-license" "$renderer_license"
require_arg "--language-license" "$language_license"
require_positive_int "--dimension" "$dimension"
require_positive_int "--max-files" "$max_files"
require_positive_int "--max-queries" "$max_queries"
require_positive_int "--top-k" "$top_k"
require_positive_int "--worker-interval-ms" "$worker_interval_ms"
require_positive_int "--ocr-worker-ticks" "$ocr_worker_ticks"
require_positive_int "--embedding-worker-ticks" "$embedding_worker_ticks"
require_positive_int "--ocr-max-pages-per-document" "$ocr_max_pages_per_document"
require_positive_int "--ocr-page-timeout-ms" "$ocr_page_timeout_ms"
require_positive_int "--ocr-render-dpi" "$ocr_render_dpi"
require_positive_int "--embedding-max-docs" "$embedding_max_docs"
require_positive_int "--embedding-max-text-bytes" "$embedding_max_text_bytes"
require_positive_int "--embedding-timeout-ms" "$embedding_timeout_ms"
require_positive_int "--ocr-throughput-max-documents" "$ocr_throughput_max_documents"
require_positive_int "--ocr-throughput-max-pages" "$ocr_throughput_max_pages"
require_positive_int "--ocr-throughput-pages-per-document" "$ocr_throughput_pages_per_document"
require_positive_int "--ocr-throughput-max-run-ms" "$ocr_throughput_max_run_ms"
require_positive_int "--ocr-throughput-min-pages" "$ocr_throughput_min_pages"
[ -z "$dataset_manifest_sha256" ] || require_sha256 "--dataset-manifest-sha256" "$dataset_manifest_sha256"
[ -z "$query_set_sha256" ] || require_sha256 "--query-set-sha256" "$query_set_sha256"
[ -z "$model_manifest_sha256" ] || require_sha256 "--model-manifest-sha256" "$model_manifest_sha256"
[ -z "$ocr_runtime_manifest_sha256" ] || require_sha256 "--ocr-runtime-manifest-sha256" "$ocr_runtime_manifest_sha256"
[ -z "$renderer_manifest_sha256" ] || require_sha256 "--renderer-manifest-sha256" "$renderer_manifest_sha256"
[ -z "$language_pack_manifest_sha256" ] || require_sha256 "--language-pack-manifest-sha256" "$language_pack_manifest_sha256"

case "$validation_profile" in
  full)
    current_stage_target="reproducible_local_10k_baseline"
    query_set_min_queries="$max_queries"
    baseline_min_documents="8000"
    baseline_min_queries="500"
    benchmark_gate_smoke_arg=""
    benchmark_gate_smoke_plan=""
    query_set_keyword_fallback_arg=""
    query_set_keyword_fallback_plan=""
    private_query_partial_hot_index_arg=""
    private_query_partial_hot_index_plan=""
    full_baseline_satisfied="false"
    release_readiness_evidence="true"
    ocr_throughput_plan_steps=$(cat <<EOF_STEPS
    {
      "id": "private_ocr_throughput_baseline",
      "command": "resume-benchmark private-ocr-throughput --root <local-resume-root> --pdftoppm-command <local-pdftoppm-command> --tesseract-command <local-tesseract-command> --max-documents $ocr_throughput_max_documents --max-pages $ocr_throughput_max_pages --pages-per-document $ocr_throughput_pages_per_document --page-timeout-ms $ocr_page_timeout_ms --max-run-ms $ocr_throughput_max_run_ms --render-dpi $ocr_render_dpi --ocr-lang <ocr-language> --dataset-manifest-sha256 <dataset-manifest-sha256> --ocr-runtime-manifest-sha256 <ocr-runtime-manifest-sha256> --renderer-manifest-sha256 <renderer-manifest-sha256> --language-pack-manifest-sha256 <language-pack-manifest-sha256> --json > <local-evidence-dir>/private-ocr-throughput.json"
    },
    {
      "id": "ocr_throughput_baseline_gate",
      "command": "resume-benchmark ocr-gate --report <local-evidence-dir>/private-ocr-throughput.json --current-stage-baseline --require-private-real-corpus --min-pages $ocr_throughput_min_pages"
    },
EOF_STEPS
)
    terminal_plan_steps='    {
      "id": "release_readiness_intake",
      "command": "resume-cli --data-dir <local-data-dir> release-readiness --json --benchmark-report <local-evidence-dir>/private-benchmark-local.json --ocr-throughput-report <local-evidence-dir>/private-ocr-throughput.json --model-manifest <local-model-manifest> --ocr-runtime-manifest <local-ocr-runtime-manifest> --diagnostics-report <local-evidence-dir>/redacted-diagnostics.json > <local-evidence-dir>/release-readiness.json"
    },
    {
      "id": "redacted_evidence_manifest",
      "command": "write <local-evidence-dir>/current-stage-validation-evidence.json with schema resume-ir.current-stage-validation-evidence.v1, file digests, step statuses, and privacy sentinels"
    },
    {
      "id": "current_stage_handoff",
      "command": "write <local-evidence-dir>/current-stage-handoff.json with schema resume-ir.current-stage-handoff.v1 from redacted current-stage evidence"
    }'
    ;;
  smoke)
    current_stage_target="local_real_corpus_smoke_chain"
    query_set_min_queries="1"
    baseline_min_documents="1"
    baseline_min_queries="1"
    benchmark_gate_smoke_arg="--allow-smoke-confidence"
    benchmark_gate_smoke_plan=" --allow-smoke-confidence"
    query_set_keyword_fallback_arg="--allow-keyword-fallback"
    query_set_keyword_fallback_plan=" --allow-keyword-fallback"
    private_query_partial_hot_index_arg="--allow-partial-hot-index-for-smoke"
    private_query_partial_hot_index_plan=" --allow-partial-hot-index-for-smoke"
    full_baseline_satisfied="false"
    release_readiness_evidence="false"
    ocr_throughput_plan_steps=""
    terminal_plan_steps='    {
      "id": "redacted_smoke_summary",
      "command": "write <local-evidence-dir>/current-stage-smoke-summary.json with schema resume-ir.current-stage-smoke-summary.v1, file digests, step statuses, and explicit non-release-evidence blockers"
    },
    {
      "id": "current_stage_handoff",
      "command": "write <local-evidence-dir>/current-stage-handoff.json with schema resume-ir.current-stage-handoff.v1 from redacted smoke summary"
    }'
    ;;
  *)
    fail "--validation-profile must be full or smoke"
    ;;
esac

if [ "$mode" = "dry-run" ]; then
  cat <<EOF
{
  "schema_version": "resume-ir.current-stage-validation-plan.v1",
  "mode": "dry-run",
  "validation_profile": "$validation_profile",
  "privacy_boundary": "local_only_redacted_plan",
  "resume_root": "<local-resume-root>",
  "data_dir": "<local-data-dir>",
  "out_dir": "<local-evidence-dir>",
  "current_stage_target": "$current_stage_target",
  "full_baseline_satisfied": $full_baseline_satisfied,
  "release_readiness_evidence": $release_readiness_evidence,
  "performance_optimization_deferred": true,
  "actual_execution_requires": "operator_local_execute_mode",
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries,
    "ocr_throughput_max_documents": $ocr_throughput_max_documents,
    "ocr_throughput_max_pages": $ocr_throughput_max_pages,
    "ocr_throughput_pages_per_document": $ocr_throughput_pages_per_document,
    "ocr_throughput_max_run_ms": $ocr_throughput_max_run_ms,
    "ocr_throughput_min_pages": $ocr_throughput_min_pages
  },
  "ordered_steps": [
    {
      "id": "ocr_preflight",
      "command": "resume-cli --data-dir <local-data-dir> ocr preflight --json --ocr-lang <ocr-language> --tesseract-command <local-tesseract-command> --pdftoppm-command <local-pdftoppm-command>"
    },
    {
      "id": "ocr_manifest_draft",
      "command": "resume-cli --data-dir <local-data-dir> ocr draft-manifest --out <local-ocr-runtime-manifest> --runtime-pack-id <reviewed-runtime-pack-id> --tesseract-command <local-tesseract-command> --pdftoppm-command <local-pdftoppm-command> --language <ocr-language> --language-pack <local-language-pack-or-lang=path> [--language-pack <lang=path> ...] --engine-license <engine-license> --renderer-license <renderer-license> --language-license <language-license> [--reviewed]"
    },
    {
      "id": "ocr_manifest_validate",
      "command": "resume-cli --data-dir <local-data-dir> ocr validate-manifest --manifest <local-ocr-runtime-manifest>"
    },
    {
      "id": "model_manifest_draft",
      "command": "resume-cli --data-dir <local-data-dir> model draft-manifest --out <local-model-manifest> --model-pack-id <reviewed-model-pack-id> --model-id <reviewed-local-model-id> --model-type embedding --dimension <dimension> --format <model-format> --artifact <local-model-artifact> --license <model-license> [--reviewed]"
    },
    {
      "id": "model_manifest_validate",
      "command": "resume-cli --data-dir <local-data-dir> model validate-manifest --manifest <local-model-manifest>"
    },
    {
      "id": "model_preflight",
      "command": "resume-cli --data-dir <local-data-dir> model preflight --json --manifest <local-model-manifest> --embedding-command <local-embedding-command> --model-id <reviewed-local-model-id> --dimension <dimension>"
    },
    {
      "id": "dataset_manifest",
      "command": "resume-cli --data-dir <local-data-dir> privacy dataset-manifest --root <local-resume-root> --out <local-evidence-dir>/dataset-manifest.local.json --profile explicit --max-files $max_files"
    },
    {
      "id": "import_private_corpus",
      "command": "resume-cli --data-dir <local-data-dir> import --root <local-resume-root> --profile explicit --max-files $max_files"
    },
    {
      "id": "ocr_worker_once_primitive",
      "command": "resume-daemon --data-dir <local-data-dir> run --foreground --once --work-ocr-once --ocr-tesseract-command <local-tesseract-command> --ocr-pdftoppm-command <local-pdftoppm-command>"
    },
    {
      "id": "ocr_worker_bounded_loop",
      "command": "resume-daemon --data-dir <local-data-dir> run --foreground --work-ocr --ocr-tesseract-command <local-tesseract-command> --ocr-pdftoppm-command <local-pdftoppm-command> --worker-interval-ms <bounded-interval-ms> --max-worker-ticks <bounded-worker-ticks>"
    },
    {
      "id": "embedding_worker_once_primitive",
      "command": "resume-daemon --data-dir <local-data-dir> run --foreground --once --work-embeddings-once --embedding-command <local-embedding-command> --embedding-model-id <reviewed-local-model-id> --embedding-dimension <dimension>"
    },
    {
      "id": "embedding_worker_bounded_loop",
      "command": "resume-daemon --data-dir <local-data-dir> run --foreground --work-embeddings --embedding-command <local-embedding-command> --embedding-model-id <reviewed-local-model-id> --embedding-dimension <dimension> --worker-interval-ms <bounded-interval-ms> --max-worker-ticks <bounded-worker-ticks>"
    },
    {
      "id": "corpus_summary",
      "command": "resume-cli --data-dir <local-data-dir> benchmark-corpus-summary --json > <local-evidence-dir>/benchmark-corpus-summary.local.json"
    },
    {
      "id": "query_set_draft",
      "command": "resume-cli --data-dir <local-data-dir> benchmark-query-set draft --out <local-evidence-dir>/private-query-set.local.jsonl --max-queries $max_queries --min-queries $query_set_min_queries$query_set_keyword_fallback_plan"
    },
    {
      "id": "private_query_baseline",
      "command": "resume-benchmark private-query --query-set <local-query-set> --command resume-cli --command-arg --data-dir --command-arg <local-data-dir> --command-arg benchmark-query-protocol --command-arg --embedding-command --command-arg <local-embedding-command> --command-arg --model-id --command-arg <reviewed-local-model-id> --command-arg --dimension --command-arg <dimension> --corpus-summary <local-evidence-dir>/benchmark-corpus-summary.local.json$private_query_partial_hot_index_plan --max-queries $max_queries --top-k $top_k --dataset-manifest-sha256 <dataset-manifest-sha256> --query-set-sha256 <query-set-sha256> --model-manifest-sha256 <model-manifest-sha256> --json > <local-evidence-dir>/private-benchmark-local.json"
    },
    {
      "id": "baseline_shape_gate",
      "command": "resume-benchmark gate --report <local-evidence-dir>/private-benchmark-local.json --require-private-real-corpus$benchmark_gate_smoke_plan --min-documents $baseline_min_documents --min-queries $baseline_min_queries --max-p95-ms 86400000 --max-zero-result-queries 500"
    },
$ocr_throughput_plan_steps
    {
      "id": "redacted_diagnostics",
      "command": "resume-cli --data-dir <local-data-dir> export-diagnostics --redact > <local-evidence-dir>/redacted-diagnostics.json"
    },
$terminal_plan_steps
  ],
  "must_not_upload": [
    "raw resumes",
    "local paths",
    "query set",
    "diagnostic package",
    "model cache",
    "runtime binaries",
    "indexes",
    "SQLite databases"
  ],
  "notes": [
    "Dry-run does not read the private resume root.",
    "Execute mode validates OCR and embedding runtime manifests/preflight before reading the private resume root.",
    "After runtime preflight succeeds, execute mode writes resume-ir.dataset-manifest.v1 under <local-evidence-dir> with privacy boundary local_only_redacted_dataset_manifest, then uses its sha256 as the dataset digest unless --dataset-manifest-sha256 is provided for consistency checking.",
    "If --query-set is omitted, execute mode writes resume-ir.query-set.jsonl.v1 under <local-evidence-dir> with privacy boundary local_only_private_query_set, then uses its sha256 as the query-set digest.",
    "Execute mode writes resume-ir.current-stage-handoff.v1 under <local-evidence-dir> after writing a smoke summary, blocked summary, or full current-stage evidence manifest.",
    "Execute mode keeps all evidence local under <local-evidence-dir>.",
    "The smoke validation profile proves local command wiring and never produces release-readiness evidence.",
    "The baseline shape gate deliberately uses --max-p95-ms 86400000; P95/P99 reduction is deferred.",
    "Release-readiness is expected to remain blocked while signing, notarization, platform installer, and other private quality evidence are missing."
  ]
}
EOF
  exit 0
fi

[ "$mode" = "execute" ] || usage
[ -d "$resume_root" ] || fail "resume root must exist and be a directory"
mkdir -p "$data_dir" "$out_dir"
dataset_manifest="$out_dir/dataset-manifest.local.json"
query_set_generated="false"
provided_query_set=""
if [ -z "$query_set" ]; then
  query_set="$out_dir/private-query-set.local.jsonl"
  query_set_generated="true"
else
  provided_query_set="$query_set"
  query_set="$out_dir/private-query-set.local.jsonl"
fi

ocr_reviewed_arg=""
if [ "$reviewed_ocr_runtime" = "true" ]; then
  ocr_reviewed_arg="--reviewed"
fi
model_reviewed_arg=""
if [ "$reviewed_model" = "true" ]; then
  model_reviewed_arg="--reviewed"
fi

printf '%s\n' "current-stage validation: ocr preflight"
set +e
"$resume_cli" --data-dir "$data_dir" ocr preflight --json \
  --ocr-lang "$language" \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  > "$out_dir/ocr-preflight.json"
ocr_preflight_status=$?
set -e
if [ "$ocr_preflight_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "ocr_preflight" "ocr" "ocr_runtime_preflight_failed" "$ocr_preflight_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi
if ! grep -Fq '"runtime_probe": "passed"' "$out_dir/ocr-preflight.json"; then
  write_runtime_preflight_blocked_summary \
    "ocr_preflight" "ocr" "ocr_runtime_probe_not_passed" 1
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

printf '%s\n' "current-stage validation: ocr manifest draft"
set -- "$resume_cli" --data-dir "$data_dir" ocr draft-manifest \
  --out "$ocr_runtime_manifest" \
  --runtime-pack-id "$runtime_pack_id" \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language "$language"
old_ifs=$IFS
IFS='
'
for language_pack_arg in $language_pack_args; do
  set -- "$@" --language-pack "$language_pack_arg"
done
IFS=$old_ifs
set -- "$@" \
  --engine-license "$engine_license" \
  --renderer-license "$renderer_license" \
  --language-license "$language_license"
if [ -n "$ocr_reviewed_arg" ]; then
  set -- "$@" "$ocr_reviewed_arg"
fi
set +e
"$@" > "$out_dir/ocr-draft-manifest.stdout.txt"
ocr_manifest_draft_status=$?
set -e
if [ "$ocr_manifest_draft_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "ocr_manifest_draft" "ocr" "ocr_runtime_manifest_draft_failed" "$ocr_manifest_draft_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

printf '%s\n' "current-stage validation: ocr manifest validate"
set +e
"$resume_cli" --data-dir "$data_dir" ocr validate-manifest \
  --manifest "$ocr_runtime_manifest" \
  > "$out_dir/ocr-validate-manifest.stdout.txt"
ocr_manifest_validate_status=$?
set -e
if [ "$ocr_manifest_validate_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "ocr_manifest_validate" "ocr" "ocr_runtime_manifest_validate_failed" "$ocr_manifest_validate_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

ocr_runtime_manifest_sha256_output=$(sha256_file "$ocr_runtime_manifest")
if [ -n "$ocr_runtime_manifest_sha256" ] && [ "$ocr_runtime_manifest_sha256" != "$ocr_runtime_manifest_sha256_output" ]; then
  write_runtime_preflight_blocked_summary \
    "ocr_manifest_validate" "ocr" "ocr_runtime_manifest_digest_mismatch" 1
  fail "OCR runtime manifest digest mismatch"
fi
ocr_runtime_manifest_sha256="$ocr_runtime_manifest_sha256_output"

printf '%s\n' "current-stage validation: model manifest draft"
set +e
"$resume_cli" --data-dir "$data_dir" model draft-manifest \
  --out "$model_manifest" \
  --model-pack-id "$model_pack_id" \
  --model-id "$model_id" \
  --model-type embedding \
  --dimension "$dimension" \
  --format "$model_format" \
  --artifact "$model_artifact" \
  --license "$model_license" \
  $model_reviewed_arg \
  > "$out_dir/model-draft-manifest.stdout.txt"
model_manifest_draft_status=$?
set -e
if [ "$model_manifest_draft_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "model_manifest_draft" "embedding" "embedding_model_manifest_draft_failed" "$model_manifest_draft_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

printf '%s\n' "current-stage validation: model manifest validate"
set +e
"$resume_cli" --data-dir "$data_dir" model validate-manifest \
  --manifest "$model_manifest" \
  > "$out_dir/model-validate-manifest.stdout.txt"
model_manifest_validate_status=$?
set -e
if [ "$model_manifest_validate_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "model_manifest_validate" "embedding" "embedding_model_manifest_validate_failed" "$model_manifest_validate_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

printf '%s\n' "current-stage validation: model preflight"
set +e
"$resume_cli" --data-dir "$data_dir" model preflight --json \
  --manifest "$model_manifest" \
  --embedding-command "$embedding_command" \
  --model-id "$model_id" \
  --dimension "$dimension" \
  > "$out_dir/model-preflight.json"
model_preflight_status=$?
set -e
if [ "$model_preflight_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "model_preflight" "embedding" "embedding_runtime_preflight_failed" "$model_preflight_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi
if ! grep -Fq '"embedding_protocol": "passed"' "$out_dir/model-preflight.json"; then
  write_runtime_preflight_blocked_summary \
    "model_preflight" "embedding" "embedding_protocol_not_passed" 1
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

model_manifest_sha256_output=$(sha256_file "$model_manifest")
if [ -n "$model_manifest_sha256" ] && [ "$model_manifest_sha256" != "$model_manifest_sha256_output" ]; then
  write_runtime_preflight_blocked_summary \
    "model_manifest_validate" "embedding" "embedding_model_manifest_digest_mismatch" 1
  fail "model manifest digest mismatch"
fi
model_manifest_sha256="$model_manifest_sha256_output"

printf '%s\n' "current-stage validation: dataset manifest"
set +e
"$resume_cli" --data-dir "$data_dir" privacy dataset-manifest \
  --root "$resume_root" \
  --out "$dataset_manifest" \
  --profile explicit \
  --max-files "$max_files" \
  > "$out_dir/dataset-manifest.stdout.txt"
dataset_manifest_status=$?
set -e
if [ "$dataset_manifest_status" -ne 0 ]; then
  write_import_parser_blocked_summary \
    "dataset_manifest" "dataset_manifest_failed" "$dataset_manifest_status"
  fail "current-stage validation blocked: import/parser failed"
fi
generated_dataset_manifest_sha256=$(sha256_file "$dataset_manifest")
if [ -n "$dataset_manifest_sha256" ] && [ "$dataset_manifest_sha256" != "$generated_dataset_manifest_sha256" ]; then
  write_import_parser_blocked_summary \
    "dataset_manifest" "dataset_manifest_digest_mismatch" 1
  fail "dataset manifest digest mismatch"
fi
dataset_manifest_sha256="$generated_dataset_manifest_sha256"

printf '%s\n' "current-stage validation: import private corpus"
set +e
"$resume_cli" --data-dir "$data_dir" import \
  --root "$resume_root" \
  --profile explicit \
  --max-files "$max_files" \
  > "$out_dir/import.stdout.txt"
import_status=$?
set -e
if [ "$import_status" -ne 0 ]; then
  write_import_parser_blocked_summary \
    "import_private_corpus" "import_private_corpus_failed" "$import_status"
  fail "current-stage validation blocked: import/parser failed"
fi

printf '%s\n' "current-stage validation: bounded ocr worker"
"$resume_daemon" --data-dir "$data_dir" run --foreground \
  --work-ocr \
  --ocr-tesseract-command "$tesseract_command" \
  --ocr-pdftoppm-command "$pdftoppm_command" \
  --ocr-lang "$language" \
  --ocr-render-dpi "$ocr_render_dpi" \
  --ocr-page-timeout-ms "$ocr_page_timeout_ms" \
  --ocr-max-pages-per-document "$ocr_max_pages_per_document" \
  --worker-interval-ms "$worker_interval_ms" \
  --max-worker-ticks "$ocr_worker_ticks" \
  > "$out_dir/ocr-worker.stdout.txt"

printf '%s\n' "current-stage validation: bounded embedding worker"
"$resume_daemon" --data-dir "$data_dir" run --foreground \
  --work-embeddings \
  --embedding-command "$embedding_command" \
  --embedding-model-id "$model_id" \
  --embedding-dimension "$dimension" \
  --embedding-max-docs "$embedding_max_docs" \
  --embedding-max-text-bytes "$embedding_max_text_bytes" \
  --embedding-timeout-ms "$embedding_timeout_ms" \
  --worker-interval-ms "$worker_interval_ms" \
  --max-worker-ticks "$embedding_worker_ticks" \
  > "$out_dir/embedding-worker.stdout.txt"

printf '%s\n' "current-stage validation: corpus summary"
"$resume_cli" --data-dir "$data_dir" benchmark-corpus-summary --json \
  > "$out_dir/benchmark-corpus-summary.local.json"

if [ "$validation_profile" = "full" ] && corpus_summary_has_bounded_ocr_backlog "$out_dir/benchmark-corpus-summary.local.json"; then
  write_ocr_backlog_blocked_summary
  printf '%s\n' "current-stage validation blocked: bounded OCR backlog remains" >&2
  exit 1
fi

printf '%s\n' "current-stage validation: query set"
if [ "$query_set_generated" = "true" ]; then
  set +e
  "$resume_cli" --data-dir "$data_dir" benchmark-query-set draft \
    --out "$query_set" \
    --max-queries "$max_queries" \
    --min-queries "$query_set_min_queries" \
    $query_set_keyword_fallback_arg \
    > "$out_dir/query-set-draft.stdout.txt"
  query_set_draft_status=$?
  set -e
  if [ "$query_set_draft_status" -ne 0 ]; then
    write_query_set_blocked_summary "$query_set_draft_status" "query_set_draft_failed"
    printf '%s\n' "current-stage validation blocked: query-set draft failed" >&2
    exit "$query_set_draft_status"
  fi
else
  [ -f "$provided_query_set" ] || fail "query set must exist and stay local"
  if [ "$provided_query_set" != "$query_set" ]; then
    cp "$provided_query_set" "$query_set" || fail "query set must stay local and readable"
  fi
  {
    printf '%s\n' "query set: provided"
    printf '%s\n' "schema: resume-ir.query-set.jsonl.v1"
    printf '%s\n' "privacy boundary: local_only_private_query_set"
    printf '%s\n' "queries: <redacted>"
    printf '%s\n' "paths: <redacted>"
  } > "$out_dir/query-set-draft.stdout.txt"
fi
query_set_output_sha256=$(sha256_file "$query_set")
if [ -n "$query_set_sha256" ] && [ "$query_set_sha256" != "$query_set_output_sha256" ]; then
  fail "query set digest mismatch"
fi
query_set_sha256="$query_set_output_sha256"

printf '%s\n' "current-stage validation: private query baseline"
set +e
"$resume_benchmark" private-query \
  --query-set "$query_set" \
  --command "$resume_cli" \
  --command-arg --data-dir --command-arg "$data_dir" \
  --command-arg benchmark-query-protocol \
  --command-arg --embedding-command --command-arg "$embedding_command" \
  --command-arg --model-id --command-arg "$model_id" \
  --command-arg --dimension --command-arg "$dimension" \
  --corpus-summary "$out_dir/benchmark-corpus-summary.local.json" \
  $private_query_partial_hot_index_arg \
  --max-queries "$max_queries" \
  --top-k "$top_k" \
  --dataset-manifest-sha256 "$dataset_manifest_sha256" \
  --query-set-sha256 "$query_set_sha256" \
  --model-manifest-sha256 "$model_manifest_sha256" \
  --json \
  > "$out_dir/private-benchmark-local.json"
private_query_status=$?
set -e
if [ "$private_query_status" -ne 0 ]; then
  write_private_query_blocked_summary "$private_query_status" "private_query_baseline_failed"
  printf '%s\n' "current-stage validation blocked: private query baseline failed" >&2
  exit "$private_query_status"
fi

printf '%s\n' "current-stage validation: baseline shape gate"
set +e
"$resume_benchmark" gate \
  --report "$out_dir/private-benchmark-local.json" \
  --require-private-real-corpus \
  $benchmark_gate_smoke_arg \
  --min-documents "$baseline_min_documents" \
  --min-queries "$baseline_min_queries" \
  --max-p95-ms 86400000 \
  --max-zero-result-queries 500 \
  > "$out_dir/private-benchmark-gate.stdout.txt"
baseline_gate_status=$?
set -e
if [ "$baseline_gate_status" -ne 0 ] && [ "$validation_profile" = "full" ]; then
  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_worker_stdout_sha256=$(sha256_file "$out_dir/ocr-worker.stdout.txt")
  embedding_worker_stdout_sha256=$(sha256_file "$out_dir/embedding-worker.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_draft_stdout_sha256=$(sha256_file "$out_dir/query-set-draft.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v1",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "baseline_shape_gate",
  "blocked_category": "benchmark",
  "blocked_reason": "baseline_shape_gate_failed",
  "blocked_exit": $baseline_gate_status,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "success"},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_draft", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "blocked", "exit_code": $baseline_gate_status}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-worker.stdout.txt", "sha256": "$ocr_worker_stdout_sha256"},
    {"file": "embedding-worker.stdout.txt", "sha256": "$embedding_worker_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "query-set-draft.stdout.txt", "sha256": "$query_set_draft_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "redacted diagnostics for this blocked run",
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
  printf '%s\n' "current-stage validation blocked: baseline shape gate failed" >&2
  exit "$baseline_gate_status"
fi
if [ "$baseline_gate_status" -ne 0 ]; then
  exit "$baseline_gate_status"
fi

if [ "$validation_profile" = "full" ]; then
  if [ -z "$renderer_manifest_sha256" ]; then
    renderer_manifest_sha256="$ocr_runtime_manifest_sha256"
  fi
  if [ -z "$language_pack_manifest_sha256" ]; then
    language_pack_manifest_sha256="$ocr_runtime_manifest_sha256"
  fi

  printf '%s\n' "current-stage validation: private ocr throughput baseline"
  set +e
  "$resume_benchmark" private-ocr-throughput \
    --root "$resume_root" \
    --pdftoppm-command "$pdftoppm_command" \
    --tesseract-command "$tesseract_command" \
    --max-documents "$ocr_throughput_max_documents" \
    --max-pages "$ocr_throughput_max_pages" \
    --pages-per-document "$ocr_throughput_pages_per_document" \
    --page-timeout-ms "$ocr_page_timeout_ms" \
    --max-run-ms "$ocr_throughput_max_run_ms" \
    --render-dpi "$ocr_render_dpi" \
    --ocr-lang "$language" \
    --dataset-manifest-sha256 "$dataset_manifest_sha256" \
    --ocr-runtime-manifest-sha256 "$ocr_runtime_manifest_sha256" \
    --renderer-manifest-sha256 "$renderer_manifest_sha256" \
    --language-pack-manifest-sha256 "$language_pack_manifest_sha256" \
    --json \
    > "$out_dir/private-ocr-throughput.json"
  private_ocr_throughput_status=$?
  set -e
  if [ "$private_ocr_throughput_status" -ne 0 ]; then
    write_ocr_throughput_blocked_summary \
      "private_ocr_throughput_baseline" \
      "private_ocr_throughput_failed" \
      "$private_ocr_throughput_status"
    printf '%s\n' "current-stage validation blocked: private OCR throughput baseline failed" >&2
    exit "$private_ocr_throughput_status"
  fi

  printf '%s\n' "current-stage validation: ocr throughput baseline gate"
  set +e
  "$resume_benchmark" ocr-gate \
    --report "$out_dir/private-ocr-throughput.json" \
    --current-stage-baseline \
    --require-private-real-corpus \
    --min-pages "$ocr_throughput_min_pages" \
    > "$out_dir/ocr-throughput-gate.stdout.txt"
  ocr_throughput_gate_status=$?
  set -e
  if [ "$ocr_throughput_gate_status" -ne 0 ]; then
    write_ocr_throughput_blocked_summary \
      "ocr_throughput_baseline_gate" \
      "ocr_throughput_baseline_gate_failed" \
      "$ocr_throughput_gate_status"
    printf '%s\n' "current-stage validation blocked: OCR throughput baseline gate failed" >&2
    exit "$ocr_throughput_gate_status"
  fi
fi

printf '%s\n' "current-stage validation: redacted diagnostics"
set +e
"$resume_cli" --data-dir "$data_dir" export-diagnostics --redact \
  > "$out_dir/redacted-diagnostics.json"
redacted_diagnostics_status=$?
set -e
if [ "$redacted_diagnostics_status" -ne 0 ]; then
  write_redacted_diagnostics_blocked_summary "$redacted_diagnostics_status" "redacted_diagnostics_failed"
  printf '%s\n' "current-stage validation blocked: redacted diagnostics failed" >&2
  exit "$redacted_diagnostics_status"
fi

if [ "$validation_profile" = "smoke" ]; then
  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_worker_stdout_sha256=$(sha256_file "$out_dir/ocr-worker.stdout.txt")
  embedding_worker_stdout_sha256=$(sha256_file "$out_dir/embedding-worker.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_draft_stdout_sha256=$(sha256_file "$out_dir/query-set-draft.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  redacted_diagnostics_sha256=$(sha256_file "$out_dir/redacted-diagnostics.json")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-smoke-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-smoke-summary.v1",
  "privacy_boundary": "local_only_redacted_aggregate_summary",
  "validation_profile": "smoke",
  "current_stage_target": "local_real_corpus_smoke_chain",
  "smoke_satisfied": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "success"},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_draft", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "smoke_success"},
    {"id": "redacted_diagnostics", "status": "success"}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-worker.stdout.txt", "sha256": "$ocr_worker_stdout_sha256"},
    {"file": "embedding-worker.stdout.txt", "sha256": "$embedding_worker_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "query-set-draft.stdout.txt", "sha256": "$query_set_draft_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "redacted-diagnostics.json", "sha256": "$redacted_diagnostics_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-smoke-summary.json"
  printf '%s\n' "current-stage validation: smoke summary written under <local-evidence-dir>"
  printf '%s\n' "current-stage validation: handoff summary written under <local-evidence-dir>"
  printf '%s\n' "current-stage validation: local smoke evidence written under <local-evidence-dir>"
  exit 0
fi

printf '%s\n' "current-stage validation: release-readiness intake"
set +e
"$resume_cli" --data-dir "$data_dir" release-readiness --json \
  --benchmark-report "$out_dir/private-benchmark-local.json" \
  --ocr-throughput-report "$out_dir/private-ocr-throughput.json" \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_runtime_manifest" \
  --diagnostics-report "$out_dir/redacted-diagnostics.json" \
  > "$out_dir/release-readiness.json" \
  2> "$out_dir/release-readiness.stderr.txt"
release_status=$?
set -e
if [ "$release_status" -ne 0 ]; then
  if grep -Fq "release readiness evidence failed validation" "$out_dir/release-readiness.stderr.txt"; then
    write_release_readiness_blocked_summary \
      "$release_status" "release_readiness_evidence_failed_validation"
    printf '%s\n' \
      "current-stage validation blocked: release-readiness evidence failed validation" >&2
    exit "$release_status"
  fi
  if ! grep -Fq "release readiness blocked: stable release criteria are not met" "$out_dir/release-readiness.stderr.txt"; then
    write_release_readiness_blocked_summary \
      "$release_status" "release_readiness_unexpected_error"
    printf '%s\n' \
      "current-stage validation blocked: release-readiness returned an unexpected error" >&2
    exit "$release_status"
  fi
fi
if [ "$release_status" -eq 0 ]; then
  stable_release_expected_blocked="false"
  release_readiness_step_status="success"
else
  stable_release_expected_blocked="true"
  release_readiness_step_status="expected_blocked"
fi

ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
ocr_worker_stdout_sha256=$(sha256_file "$out_dir/ocr-worker.stdout.txt")
embedding_worker_stdout_sha256=$(sha256_file "$out_dir/embedding-worker.stdout.txt")
corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
query_set_draft_stdout_sha256=$(sha256_file "$out_dir/query-set-draft.stdout.txt")
private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
private_ocr_throughput_sha256=$(sha256_file "$out_dir/private-ocr-throughput.json")
ocr_throughput_gate_sha256=$(sha256_file "$out_dir/ocr-throughput-gate.stdout.txt")
redacted_diagnostics_sha256=$(sha256_file "$out_dir/redacted-diagnostics.json")
release_readiness_sha256=$(sha256_file "$out_dir/release-readiness.json")
release_readiness_stderr_sha256=$(sha256_file "$out_dir/release-readiness.stderr.txt")
dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

cat > "$out_dir/current-stage-validation-evidence.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-validation-evidence.v1",
  "privacy_boundary": "local_only_redacted_evidence_manifest",
  "current_stage_target": "reproducible_local_10k_baseline",
  "full_baseline_satisfied": true,
  "release_readiness_evidence": true,
  "performance_optimization_deferred": true,
  "release_readiness_exit": $release_status,
  "stable_release_expected_blocked": $stable_release_expected_blocked,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "success"},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_draft", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
    {"id": "private_ocr_throughput_baseline", "status": "success"},
    {"id": "ocr_throughput_baseline_gate", "status": "success"},
    {"id": "redacted_diagnostics", "status": "success"},
    {"id": "release_readiness_intake", "status": "$release_readiness_step_status", "exit_code": $release_status}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-worker.stdout.txt", "sha256": "$ocr_worker_stdout_sha256"},
    {"file": "embedding-worker.stdout.txt", "sha256": "$embedding_worker_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "query-set-draft.stdout.txt", "sha256": "$query_set_draft_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "private-ocr-throughput.json", "sha256": "$private_ocr_throughput_sha256"},
    {"file": "ocr-throughput-gate.stdout.txt", "sha256": "$ocr_throughput_gate_sha256"},
    {"file": "redacted-diagnostics.json", "sha256": "$redacted_diagnostics_sha256"},
    {"file": "release-readiness.json", "sha256": "$release_readiness_sha256"},
    {"file": "release-readiness.stderr.txt", "sha256": "$release_readiness_stderr_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
write_current_stage_handoff "$out_dir/current-stage-validation-evidence.json"
printf 'current-stage validation: release-readiness exit %s\n' "$release_status"
printf '%s\n' "current-stage validation: redacted evidence manifest written under <local-evidence-dir>"
printf '%s\n' "current-stage validation: handoff summary written under <local-evidence-dir>"
printf '%s\n' "current-stage validation: local evidence written under <local-evidence-dir>"

#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required current-stage validation file: $1"
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
    fail "$file leaked local current-stage validation marker: $text"
  fi
}

require_current_stage_handoff() {
  expected_status="$1"
  expected_source_schema="$2"
  handoff="$execute_out_dir/current-stage-handoff.json"
  if [ ! -s "$handoff" ]; then
    fail "current-stage execute did not write redacted handoff summary"
  fi
  if command -v python3 >/dev/null 2>&1; then
    python3 -m json.tool "$handoff" >/dev/null
  fi
  require_text "$handoff" '"schema_version": "resume-ir.current-stage-handoff.v1"'
  require_text "$handoff" '"privacy_boundary": "local_only_redacted_handoff"'
  require_text "$handoff" "\"source_schema\": \"$expected_source_schema\""
  require_text "$handoff" "\"current_stage_status\": \"$expected_status\""
  require_text "$handoff" '"complete_product": false'
  require_text "$handoff" '"performance_optimization_deferred": true'
  require_text "$handoff" '"derived_blockers"'
  require_text "$handoff" '"must_not_upload"'
  reject_text "$handoff" "$tmpdir"
  reject_text "$handoff" "PRIVATE-current-stage"
  reject_text "$handoff" "private fake query"
}

require_full_evidence_observability() {
  file="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for current-stage observability validation"
  python3 scripts/ci/validate-current-stage-observability.py --full-evidence "$file" \
    || fail "current-stage full evidence observability is invalid"
}

require_summary_observability() {
  file="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for current-stage observability validation"
  python3 scripts/ci/validate-current-stage-observability.py --summary "$file" \
    || fail "$file current-stage summary observability is invalid"
}

require_partial_hot_index_observability() {
  file="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for partial hot-index observability validation"
  python3 scripts/ci/validate-current-stage-observability.py --summary "$file" --min-documents 1 \
    || fail "$file partial hot-index observability shape is invalid"
  python3 - "$file" <<'PY' || fail "$file partial hot-index observability is invalid"
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    document = json.load(handle)

observability = document.get("corpus_summary_observability")
if not isinstance(observability, dict):
    raise SystemExit("missing corpus_summary_observability")

document_count = observability.get("document_count")
searchable = observability.get("searchable_document_count")
vector = observability.get("vector_indexed_document_count")
hot = observability.get("hot_index_fully_covered")

if not all(isinstance(value, int) for value in (document_count, searchable, vector)):
    raise SystemExit("observability counts must be integers")
if not (document_count > searchable > 0):
    raise SystemExit("partial hot-index summary must have nonzero partial searchable coverage")
if not (0 < vector <= searchable):
    raise SystemExit("partial hot-index summary must have nonzero vector coverage within searchable coverage")
if hot is not False:
    raise SystemExit("partial hot-index summary must not claim full hot-index coverage")
PY
}

require_reused_import_stdout() {
  file="$1"
  require_text "$file" "import: reused existing data-dir"
  require_text "$file" "searchable documents:"
  require_text "$file" "import tasks recoverable: 0"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for reused import stdout validation"
  python3 - "$file" <<'PY' || fail "$file reused import stdout is invalid"
import re
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    text = handle.read()

match = re.search(r"(?m)^searchable documents: ([0-9]+)$", text)
if match is None:
    raise SystemExit("missing searchable document count")
if int(match.group(1)) < 1:
    raise SystemExit("searchable document count must be positive")
PY
}

require_fault_suite_evidence() {
  file="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for current-stage fault-suite validation"
  python3 scripts/ci/validate-current-stage-fault-suite.py --local-safe-suite "$file" \
    || fail "current-stage local-safe fault-suite evidence is invalid"
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

script="scripts/local/run-current-stage-validation.sh"

require_file "$script"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-current-stage-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

observability_good="$tmpdir/observability-good.json"
observability_missing_redaction="$tmpdir/observability-missing-redaction.json"
observability_leaking_redaction="$tmpdir/observability-leaking-redaction.json"
cat > "$observability_good" <<'JSON'
{"corpus_summary_observability":{"privacy_boundary":"redacted_local_aggregate","document_count":10000,"searchable_document_count":8000,"vector_indexed_document_count":8000,"hot_index_fully_covered":true,"document_status_counts":{},"ingest_job_status_counts":{},"ingest_job_kind_status_counts":{},"ingest_job_failure_counts":{},"contains_raw_resume_text":false,"contains_resume_paths":false,"contains_queries":false,"contains_sample_ids":false}}
JSON
cat > "$observability_missing_redaction" <<'JSON'
{"corpus_summary_observability":{"privacy_boundary":"redacted_local_aggregate","document_count":10000,"searchable_document_count":8000,"vector_indexed_document_count":8000,"hot_index_fully_covered":true,"document_status_counts":{},"ingest_job_status_counts":{},"ingest_job_kind_status_counts":{},"ingest_job_failure_counts":{}}}
JSON
cat > "$observability_leaking_redaction" <<'JSON'
{"corpus_summary_observability":{"privacy_boundary":"redacted_local_aggregate","document_count":10000,"searchable_document_count":8000,"vector_indexed_document_count":8000,"hot_index_fully_covered":true,"document_status_counts":{},"ingest_job_status_counts":{},"ingest_job_kind_status_counts":{},"ingest_job_failure_counts":{},"contains_raw_resume_text":true,"contains_resume_paths":false,"contains_queries":false,"contains_sample_ids":false}}
JSON
python3 scripts/ci/validate-current-stage-observability.py --summary "$observability_good" \
  || fail "current-stage observability validator rejected redacted evidence"
if python3 scripts/ci/validate-current-stage-observability.py --summary "$observability_missing_redaction" >/dev/null 2>&1; then
  fail "current-stage observability validator accepted missing redaction sentinels"
fi
if python3 scripts/ci/validate-current-stage-observability.py --summary "$observability_leaking_redaction" >/dev/null 2>&1; then
  fail "current-stage observability validator accepted leaking redaction sentinel"
fi

plan="$tmpdir/current-stage-validation-plan.json"
resume_root="$tmpdir/PRIVATE-current-stage-resumes"
data_dir="$tmpdir/PRIVATE-current-stage-data"
out_dir="$tmpdir/PRIVATE-current-stage-evidence"
query_set="$tmpdir/PRIVATE-current-stage-query-set.jsonl"
query_set_trace_root="$tmpdir/PRIVATE-current-stage-query-traces"
embedding_runtime_bin_dir="$tmpdir/PRIVATE-current-stage-embedding-runtime-bin"
model_manifest="$tmpdir/PRIVATE-current-stage-model-manifest.json"
ocr_manifest="$tmpdir/PRIVATE-current-stage-ocr-manifest.json"
model_artifact="$tmpdir/PRIVATE-current-stage-model.onnx"
embedding_command="$tmpdir/PRIVATE-current-stage-embedding"
tesseract_command="$tmpdir/PRIVATE-current-stage-tesseract"
pdftoppm_command="$tmpdir/PRIVATE-current-stage-pdftoppm"
language_pack="$tmpdir/PRIVATE-current-stage-tessdata.traineddata"

mkdir -p "$resume_root" "$data_dir" "$out_dir" "$query_set_trace_root" "$embedding_runtime_bin_dir"

"$script" --dry-run \
  --resume-root "$resume_root" \
  --data-dir "$data_dir" \
  --out-dir "$out_dir" \
  --query-set-trace-root "$query_set_trace_root" \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_manifest" \
  --model-artifact "$model_artifact" \
  --embedding-command "$embedding_command" \
  --embedding-runtime-bin-dir "$embedding_runtime_bin_dir" \
  --model-pack-id reviewed-local-model-pack \
  --model-id reviewed-local-embedding-model \
  --model-format onnx \
  --dimension 384 \
  --model-license Apache-2.0 \
  --runtime-pack-id reviewed-local-ocr-pack \
  --runtime-distribution-mode bundled \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  --model-manifest-sha256 cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc \
  --ocr-runtime-manifest-sha256 dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd \
  --renderer-manifest-sha256 eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee \
  --language-pack-manifest-sha256 ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10 \
  > "$plan"

if [ ! -s "$plan" ]; then
  fail "current-stage validation dry-run plan is empty"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$plan" >/dev/null
  python3 - "$plan" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    plan = json.load(handle)
ids = [step.get("id") for step in plan.get("ordered_steps", [])]
required = [
    "ocr_preflight",
    "ocr_manifest_draft",
    "ocr_manifest_validate",
    "model_manifest_draft",
    "model_manifest_validate",
    "model_preflight",
    "dataset_manifest",
    "import_private_corpus",
]
missing = [step for step in required if step not in ids]
if missing:
    raise SystemExit(f"missing current-stage ordered steps: {missing}")
if not all(ids.index(step) < ids.index("dataset_manifest") for step in required[:6]):
    raise SystemExit("runtime preflight steps must precede dataset manifest")
if ids.index("dataset_manifest") > ids.index("import_private_corpus"):
    raise SystemExit("dataset manifest must precede private import")
PY
fi

require_text "$plan" '"schema_version": "resume-ir.current-stage-validation-plan.v1"'
require_text "$plan" '"mode": "dry-run"'
require_text "$plan" '"privacy_boundary": "local_only_redacted_plan"'
require_text "$plan" '"resume_root": "<local-resume-root>"'
require_text "$plan" '"data_dir": "<local-data-dir>"'
require_text "$plan" '"out_dir": "<local-evidence-dir>"'
require_text "$plan" '"current_stage_target": "reproducible_local_10k_baseline"'
require_text "$plan" '"runtime_distribution_mode": "bundled"'
require_text "$plan" '"runtime_package_binaries_included": true'
require_text "$plan" '"performance_optimization_deferred": true'
require_text "$plan" '"embedding_runtime_bin_dir_configured": true'
require_text "$plan" '"private_query_timeout_ms": 30000'
require_text "$plan" '"ocr_jobs_per_tick": 1'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> privacy dataset-manifest --root <local-resume-root> --out <local-evidence-dir>/dataset-manifest.local.json --profile explicit --max-files 10000'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr preflight --json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr draft-manifest'
require_text "$plan" '[--language-pack <lang=path> ...]'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr validate-manifest --manifest <local-ocr-runtime-manifest>'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> model draft-manifest'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> model preflight --json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> model validate-manifest --manifest <local-model-manifest>'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> import --root <local-resume-root> --profile explicit --max-files 10000'
require_text "$plan" 'RESUME_IR_LOCAL_EVIDENCE_DIR=<local-evidence-dir> RESUME_IR_QUERY_ARTIFACT_ROOT=$RESUME_IR_QUERY_ARTIFACT_ROOT resume-cli --data-dir <local-data-dir> benchmark-query-set freeze-agent-replay --max-queries 500 --min-queries 500'
require_text "$plan" 'resume-daemon --data-dir <local-data-dir> run --foreground --once --work-ocr-once'
require_text "$plan" '--ocr-jobs-per-tick <bounded-ocr-jobs-per-tick>'
require_text "$plan" 'resume-daemon --data-dir <local-data-dir> run --foreground --once --work-embeddings-once'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> benchmark-corpus-summary --json > <local-evidence-dir>/benchmark-corpus-summary.local.json'
require_text "$plan" 'resume-benchmark private-query'
require_text "$plan" '--resident-command-arg benchmark-query-protocol'
require_text "$plan" '--resident-command-arg --batch-jsonl'
require_text "$plan" '--max-queries 500 --request-sample-count 5000 --top-k 10 --timeout-ms 30000'
reject_text "$plan" '--min-samples-per-bucket 500'
require_text "$plan" 'resume-benchmark gate --report <local-evidence-dir>/private-benchmark-local.json --require-private-real-corpus --min-documents 10000 --min-queries 500 --max-p95-ms 86400000 --max-zero-result-queries 0'
require_text "$plan" 'resume-benchmark private-ocr-throughput'
require_text "$plan" 'resume-benchmark ocr-gate --report <local-evidence-dir>/private-ocr-throughput.json --current-stage-baseline --require-private-real-corpus --min-pages 500'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> export-diagnostics --redact > <local-evidence-dir>/redacted-diagnostics.json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> fault-simulate --case disk-space-low --scratch-dir <local-evidence-dir>/fault-simulation-scratch --required-bytes 4096 --available-bytes 1024 --json > <local-evidence-dir>/fault-simulation-storage-low.json'
require_text "$plan" 'fault-simulation.v1'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> fault-simulate --suite local-safe --scratch-dir <local-evidence-dir>/fault-simulation-suite-scratch --daemon-binary <local-resume-daemon> --ocr-command <local-ocr-crash-fixture> --json > <local-evidence-dir>/fault-simulation-suite-local-safe.json'
require_text "$plan" 'fault-simulation-suite.v1'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> release-readiness --json'
require_text "$plan" '--benchmark-report <local-evidence-dir>/private-benchmark-local.json'
require_text "$plan" '--ocr-throughput-report <local-evidence-dir>/private-ocr-throughput.json'
require_text "$plan" '--model-manifest <local-model-manifest>'
require_text "$plan" '--ocr-runtime-manifest <local-ocr-runtime-manifest>'
require_text "$plan" '--diagnostics-report <local-evidence-dir>/redacted-diagnostics.json'
require_text "$plan" 'current-stage-handoff.json'
require_text "$plan" 'resume-ir.current-stage-handoff.v1'
require_text "$plan" 'current-stage-issue-comment.md'
require_text "$plan" 'redacted #53 comment drafting'
require_text "$plan" '"must_not_upload": ['
require_text "$plan" '"raw resumes"'
require_text "$plan" '"local paths"'
require_text "$plan" '"query set"'
require_text "$plan" '"diagnostic package"'
require_text "$plan" '"model cache"'
require_text "$plan" '"actual_execution_requires": "operator_local_execute_mode"'

reject_text "$plan" "$tmpdir"
reject_text "$plan" "PRIVATE-current-stage"
reject_text "$plan" "$resume_root"
reject_text "$plan" "$data_dir"
reject_text "$plan" "$out_dir"
reject_text "$plan" "$query_set"
reject_text "$plan" "$model_manifest"
reject_text "$plan" "$ocr_manifest"
reject_text "$plan" "$embedding_runtime_bin_dir"
reject_text "$plan" "$embedding_command"
reject_text "$plan" "$tesseract_command"
reject_text "$plan" "$pdftoppm_command"
reject_text "$plan" "$language_pack"
reject_text "$plan" "/Users/"
reject_text "$plan" "--allow-keyword-fallback"
reject_text "$plan" "--allow-partial-hot-index-for-smoke"

missing_trace_stdout="$tmpdir/current-stage-validation-missing-trace-root.stdout"
missing_trace_stderr="$tmpdir/current-stage-validation-missing-trace-root.stderr"
set +e
"$script" --dry-run \
  --resume-root "$resume_root" \
  --data-dir "$data_dir" \
  --out-dir "$out_dir" \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_manifest" \
  --model-artifact "$model_artifact" \
  --embedding-command "$embedding_command" \
  --embedding-runtime-bin-dir "$embedding_runtime_bin_dir" \
  --model-pack-id reviewed-local-model-pack \
  --model-id reviewed-local-embedding-model \
  --model-format onnx \
  --dimension 384 \
  --model-license Apache-2.0 \
  --runtime-pack-id reviewed-local-ocr-pack \
  --runtime-distribution-mode bundled \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10 \
  > "$missing_trace_stdout" 2> "$missing_trace_stderr"
missing_trace_status=$?
set -e
if [ "$missing_trace_status" -eq 0 ]; then
  fail "current-stage full dry-run accepted missing trace root and planned non-static query-set generation"
fi
require_text "$missing_trace_stderr" "current-stage validation requires --query-set-trace-root or --query-set"
if [ -s "$missing_trace_stdout" ]; then
  fail "current-stage full dry-run wrote a plan after missing trace root"
fi
reject_text "$missing_trace_stderr" "$tmpdir"
reject_text "$missing_trace_stderr" "PRIVATE-current-stage"
reject_text "$missing_trace_stderr" "/Users/"

env_trace_plan="$tmpdir/current-stage-validation-env-trace-root-plan.json"
RESUME_IR_QUERY_ARTIFACT_ROOT="$query_set_trace_root" "$script" --dry-run \
  --resume-root "$resume_root" \
  --data-dir "$data_dir" \
  --out-dir "$out_dir" \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_manifest" \
  --model-artifact "$model_artifact" \
  --embedding-command "$embedding_command" \
  --embedding-runtime-bin-dir "$embedding_runtime_bin_dir" \
  --model-pack-id reviewed-local-model-pack \
  --model-id reviewed-local-embedding-model \
  --model-format onnx \
  --dimension 384 \
  --model-license Apache-2.0 \
  --runtime-pack-id reviewed-local-ocr-pack \
  --runtime-distribution-mode bundled \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10 \
  > "$env_trace_plan"
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$env_trace_plan" >/dev/null
fi
require_text "$env_trace_plan" 'RESUME_IR_LOCAL_EVIDENCE_DIR=<local-evidence-dir> RESUME_IR_QUERY_ARTIFACT_ROOT=$RESUME_IR_QUERY_ARTIFACT_ROOT resume-cli --data-dir <local-data-dir> benchmark-query-set freeze-agent-replay --max-queries 500 --min-queries 500'
reject_text "$env_trace_plan" "benchmark-query-set draft"
reject_text "$env_trace_plan" "$query_set_trace_root"
reject_text "$env_trace_plan" "$tmpdir"
reject_text "$env_trace_plan" "PRIVATE-current-stage"
reject_text "$env_trace_plan" "/Users/"

env_private_sources_plan="$tmpdir/current-stage-env-private-sources-plan.json"
RESUME_IR_PRIVATE_RESUME_ROOT="$resume_root" RESUME_IR_DATA_DIR="$data_dir" RESUME_IR_LOCAL_EVIDENCE_DIR="$out_dir" RESUME_IR_QUERY_ARTIFACT_ROOT="$query_set_trace_root" "$script" --dry-run \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_manifest" \
  --model-artifact "$model_artifact" \
  --embedding-command "$embedding_command" \
  --model-pack-id reviewed-local-model-pack \
  --model-id reviewed-local-embedding-model \
  --model-format onnx \
  --dimension 384 \
  --model-license Apache-2.0 \
  --runtime-pack-id reviewed-local-ocr-pack \
  --runtime-distribution-mode bundled \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10 \
  > "$env_private_sources_plan"
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$env_private_sources_plan" >/dev/null
fi
require_text "$env_private_sources_plan" '"resume_root": "<local-resume-root>"'
require_text "$env_private_sources_plan" '"out_dir": "<local-evidence-dir>"'
require_text "$env_private_sources_plan" 'RESUME_IR_LOCAL_EVIDENCE_DIR=<local-evidence-dir> RESUME_IR_QUERY_ARTIFACT_ROOT=$RESUME_IR_QUERY_ARTIFACT_ROOT resume-cli --data-dir <local-data-dir> benchmark-query-set freeze-agent-replay --max-queries 500 --min-queries 500'
reject_text "$env_private_sources_plan" "$resume_root"
reject_text "$env_private_sources_plan" "$data_dir"
reject_text "$env_private_sources_plan" "$out_dir"
reject_text "$env_private_sources_plan" "$query_set_trace_root"
reject_text "$env_private_sources_plan" "$tmpdir"
reject_text "$env_private_sources_plan" "PRIVATE-current-stage"
reject_text "$env_private_sources_plan" "/Users/"

provided_query_set_plan="$tmpdir/current-stage-validation-provided-query-set-plan.json"
"$script" --dry-run \
  --resume-root "$resume_root" \
  --data-dir "$data_dir" \
  --out-dir "$out_dir" \
  --query-set "$query_set" \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_manifest" \
  --model-artifact "$model_artifact" \
  --embedding-command "$embedding_command" \
  --embedding-runtime-bin-dir "$embedding_runtime_bin_dir" \
  --model-pack-id reviewed-local-model-pack \
  --model-id reviewed-local-embedding-model \
  --model-format onnx \
  --dimension 384 \
  --model-license Apache-2.0 \
  --runtime-pack-id reviewed-local-ocr-pack \
  --runtime-distribution-mode bundled \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10 \
  > "$provided_query_set_plan"
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$provided_query_set_plan" >/dev/null
fi
require_text "$provided_query_set_plan" '"query_set_prepare"'
require_text "$provided_query_set_plan" 'copy <local-query-set> and its paired summary to <local-evidence-dir>/private-query-set.local.jsonl'
require_text "$provided_query_set_plan" 'write <local-evidence-dir>/query-set-prepare.stdout.txt with query_source, query_count, query_set_sha256, tune_sha256, holdout_sha256, and redacted query/path markers'
reject_text "$provided_query_set_plan" "$query_set"
reject_text "$provided_query_set_plan" "$tmpdir"
reject_text "$provided_query_set_plan" "PRIVATE-current-stage"
reject_text "$provided_query_set_plan" "private fake query"
reject_text "$provided_query_set_plan" "/Users/"

auto_ocr_bin_dir="$tmpdir/PRIVATE-current-stage-auto-ocr-bin"
auto_ocr_plan="$tmpdir/current-stage-validation-auto-ocr-plan.json"
mkdir -p "$auto_ocr_bin_dir"
printf '%s\n' '#!/usr/bin/env sh' 'exit 0' > "$auto_ocr_bin_dir/tesseract"
printf '%s\n' '#!/usr/bin/env sh' 'exit 0' > "$auto_ocr_bin_dir/pdftoppm"
chmod 700 "$auto_ocr_bin_dir/tesseract" "$auto_ocr_bin_dir/pdftoppm"

PATH="$auto_ocr_bin_dir:$PATH" "$script" --dry-run \
  --resume-root "$resume_root" \
  --data-dir "$data_dir" \
  --out-dir "$out_dir" \
  --query-set-trace-root "$query_set_trace_root" \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_manifest" \
  --model-artifact "$model_artifact" \
  --embedding-command "$embedding_command" \
  --embedding-runtime-bin-dir "$embedding_runtime_bin_dir" \
  --model-pack-id reviewed-local-model-pack \
  --model-id reviewed-local-embedding-model \
  --model-format onnx \
  --dimension 384 \
  --model-license Apache-2.0 \
  --runtime-pack-id reviewed-local-ocr-pack \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10 \
  > "$auto_ocr_plan"

require_text "$auto_ocr_plan" '"schema_version": "resume-ir.current-stage-validation-plan.v1"'
require_text "$auto_ocr_plan" 'resume-cli --data-dir <local-data-dir> ocr preflight --json'
require_text "$auto_ocr_plan" '--tesseract-command <local-tesseract-command>'
require_text "$auto_ocr_plan" '--pdftoppm-command <local-pdftoppm-command>'
reject_text "$auto_ocr_plan" "$auto_ocr_bin_dir"
reject_text "$auto_ocr_plan" "PRIVATE-current-stage"
reject_text "$auto_ocr_plan" "/Users/"

smoke_plan="$tmpdir/current-stage-validation-smoke-plan.json"
"$script" --dry-run \
  --validation-profile smoke \
  --resume-root "$resume_root" \
  --data-dir "$data_dir" \
  --out-dir "$out_dir" \
  --query-set-trace-root "$query_set_trace_root" \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_manifest" \
  --model-artifact "$model_artifact" \
  --embedding-command "$embedding_command" \
  --embedding-runtime-bin-dir "$embedding_runtime_bin_dir" \
  --model-pack-id reviewed-local-model-pack \
  --model-id reviewed-local-embedding-model \
  --model-format onnx \
  --dimension 384 \
  --model-license Apache-2.0 \
  --runtime-pack-id reviewed-local-ocr-pack \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  --max-files 6 \
  --max-queries 3 \
  --top-k 5 \
  > "$smoke_plan"

if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$smoke_plan" >/dev/null
fi
require_text "$smoke_plan" '"validation_profile": "smoke"'
require_text "$smoke_plan" '"current_stage_target": "local_real_corpus_smoke_chain"'
require_text "$smoke_plan" '"full_baseline_satisfied": false'
require_text "$smoke_plan" '"release_readiness_evidence": false'
require_text "$smoke_plan" '"private_query_timeout_ms": 30000'
require_text "$smoke_plan" 'RESUME_IR_LOCAL_EVIDENCE_DIR=<local-evidence-dir> RESUME_IR_QUERY_ARTIFACT_ROOT=$RESUME_IR_QUERY_ARTIFACT_ROOT resume-cli --data-dir <local-data-dir> benchmark-query-set freeze-agent-replay --max-queries 3 --min-queries 1'
reject_text "$smoke_plan" '--allow-keyword-fallback'
require_text "$smoke_plan" '--corpus-summary <local-evidence-dir>/benchmark-corpus-summary.local.json --allow-partial-hot-index-for-smoke --max-queries 3 --request-sample-count 3 --top-k 5 --timeout-ms 30000'
require_text "$smoke_plan" 'resume-benchmark gate --report <local-evidence-dir>/private-benchmark-local.json --require-private-real-corpus --allow-smoke-confidence --min-documents 1 --min-queries 1 --max-p95-ms 86400000 --max-zero-result-queries 0'
require_text "$smoke_plan" 'resume-cli --data-dir <local-data-dir> fault-simulate --case disk-space-low --scratch-dir <local-evidence-dir>/fault-simulation-scratch --required-bytes 4096 --available-bytes 1024 --json > <local-evidence-dir>/fault-simulation-storage-low.json'
require_text "$smoke_plan" 'resume-cli --data-dir <local-data-dir> fault-simulate --suite local-safe --scratch-dir <local-evidence-dir>/fault-simulation-suite-scratch --daemon-binary <local-resume-daemon> --ocr-command <local-ocr-crash-fixture> --json > <local-evidence-dir>/fault-simulation-suite-local-safe.json'
require_text "$smoke_plan" 'write <local-evidence-dir>/current-stage-smoke-summary.json'
require_text "$smoke_plan" 'current-stage-handoff.json'
require_text "$smoke_plan" 'resume-ir.current-stage-handoff.v1'
reject_text "$smoke_plan" "release-readiness --json"
reject_text "$smoke_plan" "$tmpdir"
reject_text "$smoke_plan" "PRIVATE-current-stage"
reject_text "$smoke_plan" "/Users/"

fake_resume_cli="$tmpdir/fake-resume-cli"
fake_resume_daemon="$tmpdir/fake-resume-daemon"
fake_resume_benchmark="$tmpdir/fake-resume-benchmark"
execute_resume_root="$tmpdir/PRIVATE-current-stage-execute-resumes"
execute_data_dir="$tmpdir/PRIVATE-current-stage-execute-data"
execute_out_dir="$tmpdir/PRIVATE-current-stage-execute-evidence"
execute_query_set="$tmpdir/PRIVATE-current-stage-execute-query-set.jsonl"
execute_query_set_summary="$tmpdir/PRIVATE-current-stage-execute-query-set.jsonl.summary.json"
execute_query_set_trace_root="$tmpdir/PRIVATE-current-stage-execute-query-traces"
execute_model_manifest="$tmpdir/PRIVATE-current-stage-execute-model-manifest.json"
execute_ocr_manifest="$tmpdir/PRIVATE-current-stage-execute-ocr-manifest.json"
execute_model_artifact="$tmpdir/PRIVATE-current-stage-execute-model.onnx"
execute_embedding_command="$tmpdir/PRIVATE-current-stage-execute-embedding"
execute_tesseract_command="$tmpdir/PRIVATE-current-stage-execute-tesseract"
execute_pdftoppm_command="$tmpdir/PRIVATE-current-stage-execute-pdftoppm"
execute_language_pack="$tmpdir/PRIVATE-current-stage-execute-tessdata.traineddata"

mkdir -p "$execute_resume_root" "$execute_data_dir" "$execute_out_dir" "$execute_query_set_trace_root"
printf '%s\n' '{"query":"private fake query"}' > "$execute_query_set"
cat > "$execute_query_set_summary" <<'JSON'
{"schema_version":"resume-ir.query-set-summary.v2","privacy_boundary":"redacted_local_aggregate","query_source":"trace_source_search_v1","query_count":500,"tune_query_count":400,"holdout_query_count":100,"bucket_counts":{"single_term":50,"and_2":75,"and_3_5":150,"and_6_16":50,"field_filter":75,"hybrid":75,"semantic":25},"tune_bucket_counts":{"single_term":40,"and_2":60,"and_3_5":120,"and_6_16":40,"field_filter":60,"hybrid":60,"semantic":20},"holdout_bucket_counts":{"single_term":10,"and_2":15,"and_3_5":30,"and_6_16":10,"field_filter":15,"hybrid":15,"semantic":5},"candidate_queries_sampled":500,"zero_hit_queries_dropped":0,"query_set_sha256":"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789","tune_sha256":"2222222222222222222222222222222222222222222222222222222222222222","holdout_sha256":"3333333333333333333333333333333333333333333333333333333333333333","hmac_split":true,"contains_raw_query_text":false,"contains_raw_resume_text":false,"contains_candidate_results":false,"contains_local_paths":false}
JSON
printf '%s\n' 'fake model bytes' > "$execute_model_artifact"
printf '%s\n' 'fake language bytes' > "$execute_language_pack"
printf '%s\n' '#!/usr/bin/env sh' 'exit 0' > "$execute_embedding_command"
printf '%s\n' '#!/usr/bin/env sh' 'exit 0' > "$execute_tesseract_command"
printf '%s\n' '#!/usr/bin/env sh' 'exit 0' > "$execute_pdftoppm_command"
chmod 700 "$execute_embedding_command" "$execute_tesseract_command" "$execute_pdftoppm_command"

cat > "$fake_resume_cli" <<'SH'
#!/usr/bin/env sh
set -eu
if [ "${1:-}" = "--data-dir" ]; then
  shift 2
fi
cmd="${1:-}"
sub="${2:-}"
out_arg() {
  out=""
  while [ "$#" -gt 0 ]; do
    if [ "$1" = "--out" ]; then
      shift
      out="${1:-}"
      break
    fi
    shift
  done
  printf '%s\n' "$out"
}
write_out_arg() {
  out=$(out_arg "$@")
  [ -n "$out" ] || exit 64
  printf '{"schema_version":"fake-local-manifest.v1"}\n' > "$out"
}
has_arg() {
  expected="$1"
  shift
  while [ "$#" -gt 0 ]; do
    [ "$1" = "$expected" ] && return 0
    shift
  done
  return 1
}
query_set_out_path() {
  out=$(out_arg "$@")
  if [ -n "$out" ]; then
    printf '%s\n' "$out"
    return
  fi
  [ -n "${RESUME_IR_LOCAL_EVIDENCE_DIR:-}" ] || exit 64
  case "$sub" in
    preflight-agent-replay)
      printf '%s/query-set-trace-preflight.local.json\n' "$RESUME_IR_LOCAL_EVIDENCE_DIR"
      ;;
    freeze-agent-replay)
      printf '%s/private-query-set.local.jsonl\n' "$RESUME_IR_LOCAL_EVIDENCE_DIR"
      ;;
    *)
      exit 64
      ;;
  esac
}
write_query_set_out() {
  out=$(query_set_out_path "$@")
  printf '{"schema_version":"fake-local-manifest.v1"}\n' > "$out"
}
query_set_summary_path() {
  file_name=$(basename "$1")
  dir_name=$(dirname "$1")
  case "$file_name" in
    *.local.jsonl) base_name=${file_name%.local.jsonl} ;;
    *) base_name=$file_name ;;
  esac
  printf '%s/%s.summary.json\n' "$dir_name" "$base_name"
}
write_query_set_summary_arg() {
  out=$(query_set_out_path "$@")
  summary=$(query_set_summary_path "$out")
  query_source="trace_source_search_v1"
  if [ "${FAKE_QUERY_SET_MODE:-ready}" = "local-field-source" ]; then
    query_source="local_field"
  fi
  cat > "$summary" <<EOF_QUERY_SET_SUMMARY
{"schema_version":"resume-ir.query-set-summary.v2","privacy_boundary":"redacted_local_aggregate","query_source":"$query_source","query_count":500,"tune_query_count":400,"holdout_query_count":100,"bucket_counts":{"single_term":50,"and_2":75,"and_3_5":150,"and_6_16":50,"field_filter":75,"hybrid":75,"semantic":25},"tune_bucket_counts":{"single_term":40,"and_2":60,"and_3_5":120,"and_6_16":40,"field_filter":60,"hybrid":60,"semantic":20},"holdout_bucket_counts":{"single_term":10,"and_2":15,"and_3_5":30,"and_6_16":10,"field_filter":15,"hybrid":15,"semantic":5},"candidate_queries_sampled":500,"zero_hit_queries_dropped":0,"query_set_sha256":"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789","tune_sha256":"2222222222222222222222222222222222222222222222222222222222222222","holdout_sha256":"3333333333333333333333333333333333333333333333333333333333333333","hmac_split":true,"contains_raw_query_text":false,"contains_raw_resume_text":false,"contains_candidate_results":false,"contains_local_paths":false}
EOF_QUERY_SET_SUMMARY
}
case "$cmd:$sub" in
  privacy:dataset-manifest)
    if [ "${FAKE_IMPORT_MODE:-ready}" = "forbid-scan" ]; then
      printf 'dataset manifest scan was unexpectedly invoked during reuse\n' >&2
      exit 65
    fi
    write_out_arg "$@"
    printf 'dataset manifest: written\n'
    printf 'schema: resume-ir.dataset-manifest.v1\n'
    printf 'privacy boundary: local_only_redacted_dataset_manifest\n'
    ;;
  benchmark-query-set:preflight-agent-replay)
    if has_arg --out "$@" || has_arg --trace-root "$@"; then
      printf 'query set preflight must use env defaults\n' >&2
      exit 1
    fi
    if [ -n "${FAKE_REQUIRED_QUERY_SET_TRACE_ROOT:-}" ] && [ "${RESUME_IR_QUERY_ARTIFACT_ROOT:-}" != "$FAKE_REQUIRED_QUERY_SET_TRACE_ROOT" ]; then
      printf 'query set trace root env missing\n' >&2
      exit 1
    fi
    out=$(query_set_out_path "$@")
    query_index_available=true
  if [ "${FAKE_QUERY_SET_MODE:-ready}" = "index-unavailable" ]; then
    query_index_available=false
  fi
  if [ "${FAKE_QUERY_SET_MODE:-ready}" = "d10k-corpus-not-ready" ]; then
    cat > "$out" <<EOF_QUERY_SET_PREFLIGHT_D10K_CORPUS
{
  "schema_version": "resume-ir.query-set-trace-preflight.v1",
  "privacy_boundary": "redacted_local_aggregate",
  "query_source": "trace_source_search_v1",
  "query_index_available": true,
  "target_query_count": 500,
  "document_count": 9999,
  "searchable_document_count": 7999,
  "vector_indexed_document_count": 8000,
  "d10k_min_document_count": 10000,
  "d10k_min_searchable_document_count": 8000,
  "d10k_min_vector_indexed_document_count": 8000,
  "d10k_corpus_ready": false,
  "d10k_corpus_deficits": {
    "document_count": 1,
    "searchable_document_count": 1,
    "vector_indexed_document_count": 0
  },
  "trace_logs": 7,
  "trace_lines": 700,
  "source_search_lines": 500,
  "extracted_queries": 500,
  "normalization_rejected": 0,
  "duplicate_queries_dropped": 0,
  "candidate_queries_sampled": 500,
  "zero_hit_queries_dropped": 0,
  "corpus_valid_queries": 500,
  "candidate_bucket_counts": {
    "single_term": 50,
    "and_2": 75,
    "and_3_5": 150,
    "and_6_16": 50,
    "field_filter": 75,
    "hybrid": 75,
    "semantic": 25
  },
  "corpus_valid_bucket_counts": {
    "single_term": 50,
    "and_2": 75,
    "and_3_5": 150,
    "and_6_16": 50,
    "field_filter": 75,
    "hybrid": 75,
    "semantic": 25
  },
  "required_bucket_counts": {
    "single_term": 50,
    "and_2": 75,
    "and_3_5": 150,
    "and_6_16": 50,
    "field_filter": 75,
    "hybrid": 75,
    "semantic": 25
  },
  "candidate_bucket_deficits": {
    "single_term": 0,
    "and_2": 0,
    "and_3_5": 0,
    "and_6_16": 0,
    "field_filter": 0,
    "hybrid": 0,
    "semantic": 0
  },
  "corpus_valid_bucket_deficits": {
    "single_term": 0,
    "and_2": 0,
    "and_3_5": 0,
    "and_6_16": 0,
    "field_filter": 0,
    "hybrid": 0,
    "semantic": 0
  },
  "contains_raw_query_text": false,
  "contains_raw_resume_text": false,
  "contains_candidate_results": false,
  "contains_local_paths": false
}
EOF_QUERY_SET_PREFLIGHT_D10K_CORPUS
    printf 'query set trace preflight: written\n'
    printf 'schema: resume-ir.query-set-trace-preflight.v1\n'
    printf 'privacy boundary: redacted_local_aggregate\n'
    printf 'queries: <redacted>\n'
    exit 0
  fi
  cat > "$out" <<EOF_QUERY_SET_PREFLIGHT
{
  "schema_version": "resume-ir.query-set-trace-preflight.v1",
  "privacy_boundary": "redacted_local_aggregate",
  "query_source": "trace_source_search_v1",
  "query_index_available": $query_index_available,
  "target_query_count": 500,
  "document_count": 3,
  "searchable_document_count": 2,
  "vector_indexed_document_count": 0,
  "d10k_min_document_count": 10000,
  "d10k_min_searchable_document_count": 8000,
  "d10k_min_vector_indexed_document_count": 8000,
  "d10k_corpus_ready": false,
  "d10k_corpus_deficits": {
    "document_count": 9997,
    "searchable_document_count": 7998,
    "vector_indexed_document_count": 8000
  },
  "trace_logs": 1,
  "trace_lines": 7,
  "source_search_lines": 6,
  "extracted_queries": 5,
  "normalization_rejected": 1,
  "duplicate_queries_dropped": 1,
  "candidate_queries_sampled": 3,
  "zero_hit_queries_dropped": 1,
  "corpus_valid_queries": 2,
  "candidate_bucket_counts": {
    "single_term": 0,
    "and_2": 2,
    "and_3_5": 0,
    "and_6_16": 0,
    "field_filter": 0,
    "hybrid": 1,
    "semantic": 0
  },
  "corpus_valid_bucket_counts": {
    "single_term": 0,
    "and_2": 1,
    "and_3_5": 0,
    "and_6_16": 0,
    "field_filter": 0,
    "hybrid": 1,
    "semantic": 0
  },
  "required_bucket_counts": {
    "single_term": 50,
    "and_2": 75,
    "and_3_5":150,
    "and_6_16": 50,
    "field_filter": 75,
    "hybrid": 75,
    "semantic": 25
  },
  "candidate_bucket_deficits": {
    "single_term": 50,
    "and_2": 73,
    "and_3_5":150,
    "and_6_16": 50,
    "field_filter": 75,
    "hybrid": 74,
    "semantic": 25
  },
  "corpus_valid_bucket_deficits": {
    "single_term": 50,
    "and_2": 74,
    "and_3_5":150,
    "and_6_16": 50,
    "field_filter": 75,
    "hybrid": 74,
    "semantic": 25
  },
  "contains_raw_query_text": false,
  "contains_raw_resume_text": false,
  "contains_candidate_results": false,
  "contains_local_paths": false
}
EOF_QUERY_SET_PREFLIGHT
    printf 'query set trace preflight: written\n'
    printf 'schema: resume-ir.query-set-trace-preflight.v1\n'
    printf 'privacy boundary: redacted_local_aggregate\n'
    printf 'queries: <redacted>\n'
    ;;
  benchmark-query-set:freeze-agent-replay)
    if [ "${FAKE_QUERY_SET_MODE:-ready}" = "prepare-failed" ]; then
      printf 'query set blocked: insufficient field-backed queries\n'
      exit 1
    fi
    if [ "${FAKE_QUERY_SET_MODE:-ready}" = "d10k-corpus-not-ready" ]; then
      printf 'resume-cli: query set blocked: D10K agent replay freeze requires a D10K-shaped indexed corpus; corpus deficits: document_count=1,searchable_document_count=1,vector_indexed_document_count=0\n' >&2
      exit 1
    fi
    if [ "${FAKE_QUERY_SET_MODE:-ready}" = "index-unavailable" ]; then
      printf 'resume-cli: query set blocked: local search index is unavailable\n' >&2
      exit 1
    fi
    if has_arg --out "$@" || has_arg --trace-root "$@"; then
      printf 'query set freeze must use env defaults\n' >&2
      exit 1
    fi
    if [ -n "${FAKE_REQUIRED_QUERY_SET_TRACE_ROOT:-}" ] && [ "${RESUME_IR_QUERY_ARTIFACT_ROOT:-}" != "$FAKE_REQUIRED_QUERY_SET_TRACE_ROOT" ]; then
      printf 'query set trace root env missing\n' >&2
      exit 1
    fi
    write_query_set_out "$@"
    if [ "${FAKE_QUERY_SET_MODE:-ready}" != "missing-summary" ]; then
      write_query_set_summary_arg "$@"
    fi
    if [ "$sub" = "freeze-agent-replay" ]; then
      printf 'query set: frozen\n'
    else
      printf 'query set: written\n'
    fi
    printf 'schema: resume-ir.query-set.jsonl.v2\n'
    printf 'privacy boundary: local_only_private_query_set\n'
    ;;
  ocr:preflight)
    if [ "${FAKE_RUNTIME_PREFLIGHT_MODE:-ready}" = "ocr-failed" ]; then
      printf 'fake OCR preflight failed\n' >&2
      exit 1
    fi
    printf '{"schema_version":"ocr-runtime-preflight.v1","runtime_probe": "passed","ready":true}\n'
    ;;
  ocr:draft-manifest)
    write_out_arg "$@"
    printf 'ocr manifest drafted\n'
    ;;
  ocr:validate-manifest)
    printf 'ocr manifest valid\n'
    ;;
  model:draft-manifest)
    write_out_arg "$@"
    printf 'model manifest drafted\n'
    ;;
  model:validate-manifest)
    printf 'model manifest valid\n'
    ;;
  model:preflight)
    if [ -n "${FAKE_REQUIRED_EMBEDDING_RUNTIME_BIN_DIR:-}" ]; then
      case ":$PATH:" in
        *":$FAKE_REQUIRED_EMBEDDING_RUNTIME_BIN_DIR:"*) ;;
        *)
          printf 'embedding runtime PATH prefix missing\n' >&2
          exit 1
          ;;
      esac
    fi
    if [ "${FAKE_RUNTIME_PREFLIGHT_MODE:-ready}" = "model-failed" ]; then
      printf 'fake model preflight failed\n' >&2
      exit 1
    fi
    printf '{"schema_version":"embedding-runtime-preflight.v1","embedding_protocol": "passed","ready":true}\n'
    ;;
  import:*)
    if [ "${FAKE_IMPORT_MODE:-ready}" = "forbid-scan" ]; then
      printf 'private corpus import was unexpectedly invoked during reuse\n' >&2
      exit 65
    fi
    if [ "${FAKE_IMPORT_MODE:-ready}" = "failed" ]; then
      printf 'import blocked: fake parser failure\n' >&2
      exit 1
    fi
    printf 'import task submitted\nstatus: completed\n'
    ;;
  status:*)
    recoverable_import_tasks='0'
    if [ "${FAKE_STATUS_MODE:-ready}" = "recoverable" ]; then
      recoverable_import_tasks='1'
    fi
    printf 'indexed documents: 10000\n'
    printf 'searchable documents: 8000\n'
    printf 'ocr queue: 0\n'
    printf 'embedding queue: 0\n'
    printf 'import tasks recoverable: %s\n' "$recoverable_import_tasks"
    if [ "$recoverable_import_tasks" = "1" ]; then
      printf 'latest import files discovered: 8720\n'
      printf 'latest import searchable documents: 312\n'
    fi
    printf 'paths: <redacted>\n'
    ;;
  benchmark-corpus-summary:*)
    if [ "${FAKE_CORPUS_SUMMARY_MODE:-hot}" = "smoke-low-hot" ]; then
      printf '{"schema_version":"benchmark-corpus-summary.v1","privacy_boundary":"redacted_local_aggregate","document_count":9133,"searchable_document_count":41,"vector_indexed_document_count":29,"hot_index_fully_covered":false,"document_status_counts":{"failed_permanent":20,"ocr_required":8951,"searchable":41,"text_cleaned":121},"ingest_job_status_counts":{"completed":226,"queued":8870},"ingest_job_kind_status_counts":{"ocr_document":{"completed":89,"queued":8870},"update_index":{"completed":137}},"ingest_job_failure_counts":{},"contains_raw_resume_text":false,"contains_resume_paths":false,"contains_queries":false,"contains_sample_ids":false}\n'
      exit 0
    fi
    if [ "${FAKE_CORPUS_SUMMARY_MODE:-hot}" = "ocr-backlog" ]; then
      printf '{"schema_version":"benchmark-corpus-summary.v1","privacy_boundary":"redacted_local_aggregate","document_count":8720,"searchable_document_count":162,"vector_indexed_document_count":0,"hot_index_fully_covered":false,"document_status_counts":{"failed_permanent":20,"ocr_required":8538,"searchable":162},"ingest_job_status_counts":{"completed":16,"queued":8537,"running":1},"ingest_job_kind_status_counts":{"ocr_document":{"completed":16,"queued":8537,"running":1}},"ingest_job_failure_counts":{},"contains_raw_resume_text":false,"contains_resume_paths":false,"contains_queries":false,"contains_sample_ids":false}\n'
      exit 0
    fi
    printf '{"schema_version":"benchmark-corpus-summary.v1","privacy_boundary":"redacted_local_aggregate","document_count":10000,"searchable_document_count":8000,"vector_indexed_document_count":8000,"hot_index_fully_covered":true,"document_status_counts":{"searchable":8000},"ingest_job_status_counts":{"completed":10000},"ingest_job_kind_status_counts":{"update_index":{"completed":8000}},"ingest_job_failure_counts":{},"contains_raw_resume_text":false,"contains_resume_paths":false,"contains_queries":false,"contains_sample_ids":false}\n'
    ;;
  export-diagnostics:*)
    if [ "${FAKE_DIAGNOSTICS_MODE:-ready}" = "failed" ]; then
      printf 'redacted diagnostics blocked: fake diagnostics failure\n' >&2
      exit 1
    fi
    if [ "${FAKE_DIAGNOSTICS_MODE:-ready}" = "invalid" ]; then
      printf '{"schema_version":"diagnostics.v1","redacted":true,"evidence_level":"local_aggregate_only"}\n'
      exit 0
    fi
    printf '{"schema_version":"diagnostics.v1","redacted":true,"raw_paths":"<redacted>","raw_queries":"<redacted>","raw_resume_text":"<redacted>","metadata":{"indexed_documents":10000,"searchable_documents":8000},"search_index_state":"available","vector_index_state":"available","query_latency":{"sample_count":500,"raw_queries":"<redacted>"},"resource_telemetry":{"status":"available","paths":"<redacted>"},"ocr_runtime":{"paths":"<redacted>","pdftoppm":"available","tesseract":"available","requested_language":"eng","requested_language_status":"available"},"diagnostic_scope":{"metadata":"aggregate_counts","search_index":"state_and_snapshot_health","vector_index":"state_backend_and_counts","query_latency":"aggregate_observations","runtime_dependencies":"presence_only","fault_simulations":"available_cases_only"},"evidence_level":"local_aggregate_only"}\n'
    ;;
  doctor:*)
    printf 'resume-ir doctor\n'
    printf 'search index: available (full-text snapshot)\n'
    printf 'vector index: available (hnsw ann vector snapshot)\n'
    printf 'metadata encryption: sqlcipher\n'
    printf 'paths: <redacted>\n'
    ;;
  fault-simulate:*)
    if [ "${FAKE_FAULT_SIMULATION_MODE:-ready}" = "failed" ]; then
      printf 'fault simulation blocked: fake fault probe failure\n' >&2
      exit 1
    fi
    case " $* " in
      *" --suite local-safe "*)
        if [ "${FAKE_FAULT_SIMULATION_MODE:-ready}" = "invalid-suite" ]; then
          printf '{"schema_version":"fault-simulation-suite.v1","redacted":true,"suite":"local_safe","paths":"<redacted>","evidence_level":"local_synthetic_fault_suite","release_hardware_drills":"blocked","summary":{"total_cases":10,"reproduced_cases":2,"blocked_by_host_cases":0,"failed_cases":0,"release_blockers_cleared":false},"cases":[{"fault":"daemon_kill","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"restart_check":"passed"}},{"fault":"ocr_crash","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"ocr_command":"failed"}}]}\n'
          exit 0
        fi
        cat <<'EOF_FAULT_SUITE'
{"schema_version":"fault-simulation-suite.v1","redacted":true,"suite":"local_safe","paths":"<redacted>","evidence_level":"local_synthetic_fault_suite","release_hardware_drills":"blocked","summary":{"total_cases":10,"reproduced_cases":10,"blocked_by_host_cases":0,"failed_cases":0,"release_blockers_cleared":false},"cases":[{"fault":"disk_space_low","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"required_bytes":4096,"available_bytes":1024,"probe_writes":"skipped"}},{"fault":"permission_denied","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"write_check":"denied"}},{"fault":"file_lock","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"lock_conflict":"detected"}},{"fault":"index_snapshot_corrupt","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"snapshot_validation":"failed"}},{"fault":"metadata_migration","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"rollback_check":"passed"}},{"fault":"model_checksum","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"checksum":"mismatch"}},{"fault":"daemon_kill","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"daemon_ready":"yes","terminated_daemon":"yes","restart_check":"passed"}},{"fault":"ocr_crash","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"ocr_command":"failed","probe_bytes":31}},{"fault":"battery_mode","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"battery_state":"battery"}},{"fault":"external_drive_disconnect","status":"reproduced","redacted":true,"paths":"<redacted>","details":{"drive_state":"disconnected"}}]}
EOF_FAULT_SUITE
        ;;
      *)
        printf '{"schema_version":"fault-simulation.v1","redacted":true,"fault":"disk_space_low","status":"reproduced","paths":"<redacted>","details":{"required_bytes":4096,"available_bytes":1024,"probe_writes":"skipped"},"evidence_level":"local_synthetic_fault_probe"}\n'
        ;;
    esac
    ;;
  release-readiness:*)
    printf '{"schema_version":"release-readiness.v1","stable_release":"blocked"}\n'
    if [ "${FAKE_RELEASE_READINESS_MODE:-blocked}" = "evidence-failed" ]; then
      printf 'release readiness evidence failed validation: fake evidence rejected\n' >&2
      exit 1
    fi
    printf 'release readiness blocked: stable release criteria are not met\n' >&2
    exit 1
    ;;
  *)
    printf 'unexpected fake resume-cli command\n' >&2
    exit 64
    ;;
esac
SH
chmod 700 "$fake_resume_cli"

cat > "$fake_resume_daemon" <<'SH'
#!/usr/bin/env sh
set -eu
if [ -n "${FAKE_REQUIRED_EMBEDDING_RUNTIME_BIN_DIR:-}" ]; then
  case " $* " in
    *" --work-embeddings "*)
      case ":$PATH:" in
        *":$FAKE_REQUIRED_EMBEDDING_RUNTIME_BIN_DIR:"*) ;;
        *)
          printf 'embedding worker PATH prefix missing\n' >&2
          exit 1
          ;;
      esac
      ;;
  esac
fi
printf 'fake worker completed\n'
SH
chmod 700 "$fake_resume_daemon"

cat > "$fake_resume_benchmark" <<'SH'
#!/usr/bin/env sh
set -eu
stage_latency_json() {
  samples="$1"
  first="true"
  printf '"stage_latency_ms":{'
  for stage in query_parse prefilter bm25 ann fusion bulk_hydrate snippet; do
    if [ "$first" = "true" ]; then
      first="false"
    else
      printf ','
    fi
    printf '"%s":{"samples":%s,"min":1.0,"mean":5.0,"p50":5.0,"p95":42.0,"p99":84.0,"max":100.0}' "$stage" "$samples"
  done
  printf '},'
}
stage_histogram_json() {
  samples="$1"
  first="true"
  printf '"stage_histogram_ms":{'
  for stage in query_parse prefilter bm25 ann fusion bulk_hydrate snippet; do
    if [ "$first" = "true" ]; then
      first="false"
    else
      printf ','
    fi
    printf '"%s":{"samples":%s,"bins":[{"le_ms":1.0,"count":%s},{"le_ms":5.0,"count":%s},{"le_ms":10.0,"count":%s},{"le_ms":25.0,"count":%s},{"le_ms":50.0,"count":%s},{"le_ms":100.0,"count":%s},{"le_ms":250.0,"count":%s},{"le_ms":500.0,"count":%s},{"le_ms":1000.0,"count":%s},{"le_ms":2500.0,"count":%s},{"le_ms":5000.0,"count":%s},{"le_ms":10000.0,"count":%s},{"le_ms":60000.0,"count":%s}],"overflow_count":0}' "$stage" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples"
  done
  printf '},'
}
query_latency_by_bucket_json() {
  first="true"
  printf '"query_latency_by_bucket":{'
  while [ "$#" -gt 0 ]; do
    bucket="$1"
    samples="$2"
    shift 2
    if [ "$samples" = "0" ]; then
      continue
    fi
    if [ "$first" = "true" ]; then
      first="false"
    else
      printf ','
    fi
    printf '"%s":{"samples":%s,"min":1.0,"mean":5.0,"p50":5.0,"p95":42.0,"p99":84.0,"max":100.0}' "$bucket" "$samples"
  done
  printf '},'
}
rss_delta_json() {
  samples="$1"
  printf '"rss_delta_mb":{"samples":%s,"min":0.0,"mean":0.0,"p50":0.0,"p95":0.0,"p99":0.0,"max":0.0},' "$samples"
}
rss_delta_by_bucket_json() {
  first="true"
  printf '"rss_delta_mb_by_bucket":{'
  while [ "$#" -gt 0 ]; do
    bucket="$1"
    samples="$2"
    shift 2
    if [ "$samples" = "0" ]; then
      continue
    fi
    if [ "$first" = "true" ]; then
      first="false"
    else
      printf ','
    fi
    printf '"%s":{"samples":%s,"min":0.0,"mean":0.0,"p50":0.0,"p95":0.0,"p99":0.0,"max":0.0}' "$bucket" "$samples"
  done
  printf '},'
}
stage_latency_by_bucket_json() {
  first_bucket="true"
  printf '"stage_latency_by_bucket_ms":{'
  while [ "$#" -gt 0 ]; do
    bucket="$1"
    samples="$2"
    shift 2
    if [ "$samples" = "0" ]; then
      continue
    fi
    if [ "$first_bucket" = "true" ]; then
      first_bucket="false"
    else
      printf ','
    fi
    printf '"%s":{' "$bucket"
    first_stage="true"
    for stage in query_parse prefilter bm25 ann fusion bulk_hydrate snippet; do
      if [ "$first_stage" = "true" ]; then
        first_stage="false"
      else
        printf ','
      fi
      printf '"%s":{"samples":%s,"min":1.0,"mean":5.0,"p50":5.0,"p95":42.0,"p99":84.0,"max":100.0}' "$stage" "$samples"
    done
    printf '}'
  done
  printf '},'
}
stage_histogram_by_bucket_json() {
  first_bucket="true"
  printf '"stage_histogram_by_bucket_ms":{'
  while [ "$#" -gt 0 ]; do
    bucket="$1"
    samples="$2"
    shift 2
    if [ "$samples" = "0" ]; then
      continue
    fi
    if [ "$first_bucket" = "true" ]; then
      first_bucket="false"
    else
      printf ','
    fi
    printf '"%s":{' "$bucket"
    first_stage="true"
    for stage in query_parse prefilter bm25 ann fusion bulk_hydrate snippet; do
      if [ "$first_stage" = "true" ]; then
        first_stage="false"
      else
        printf ','
      fi
      printf '"%s":{"samples":%s,"bins":[{"le_ms":1.0,"count":%s},{"le_ms":5.0,"count":%s},{"le_ms":10.0,"count":%s},{"le_ms":25.0,"count":%s},{"le_ms":50.0,"count":%s},{"le_ms":100.0,"count":%s},{"le_ms":250.0,"count":%s},{"le_ms":500.0,"count":%s},{"le_ms":1000.0,"count":%s},{"le_ms":2500.0,"count":%s},{"le_ms":5000.0,"count":%s},{"le_ms":10000.0,"count":%s},{"le_ms":60000.0,"count":%s}],"overflow_count":0}' "$stage" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples" "$samples"
    done
    printf '}'
  done
  printf '},'
}
private_query_split_low_hot_json() {
  printf '"tune_sha256":"2222222222222222222222222222222222222222222222222222222222222222",'
  printf '"holdout_sha256":"3333333333333333333333333333333333333333333333333333333333333333",'
  printf '"tune_bucket_counts":{"single_term":0,"and_2":0,"and_3_5":2,"and_6_16":0,"field_filter":0,"hybrid":0,"semantic":0},'
  printf '"holdout_bucket_counts":{"single_term":0,"and_2":0,"and_3_5":1,"and_6_16":0,"field_filter":0,"hybrid":0,"semantic":0},'
}
private_query_split_low_bucket_json() {
  printf '"tune_sha256":"2222222222222222222222222222222222222222222222222222222222222222",'
  printf '"holdout_sha256":"3333333333333333333333333333333333333333333333333333333333333333",'
  printf '"tune_bucket_counts":{"single_term":0,"and_2":0,"and_3_5":400,"and_6_16":0,"field_filter":0,"hybrid":0,"semantic":0},'
  printf '"holdout_bucket_counts":{"single_term":0,"and_2":0,"and_3_5":100,"and_6_16":0,"field_filter":0,"hybrid":0,"semantic":0},'
}
private_query_split_full_json() {
  printf '"tune_sha256":"2222222222222222222222222222222222222222222222222222222222222222",'
  printf '"holdout_sha256":"3333333333333333333333333333333333333333333333333333333333333333",'
  printf '"tune_bucket_counts":{"single_term":40,"and_2":60,"and_3_5":120,"and_6_16":40,"field_filter":60,"hybrid":60,"semantic":20},'
  printf '"holdout_bucket_counts":{"single_term":10,"and_2":15,"and_3_5":30,"and_6_16":10,"field_filter":15,"hybrid":15,"semantic":5},'
}
case "${1:-}" in
  private-query)
    query_set_report_sha=abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "private-query-sha-mismatch" ]; then
      query_set_report_sha=4444444444444444444444444444444444444444444444444444444444444444
    fi
    if [ -n "${FAKE_REQUIRED_EMBEDDING_RUNTIME_BIN_DIR:-}" ]; then
      case ":$PATH:" in
        *":$FAKE_REQUIRED_EMBEDDING_RUNTIME_BIN_DIR:"*) ;;
        *)
          printf 'private query PATH prefix missing\n' >&2
          exit 1
          ;;
      esac
    fi
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "private-query-failed" ]; then
      printf 'private query baseline blocked: query protocol failed\n' >&2
      exit 1
    fi
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "private-query-invalid" ]; then
      printf '{"schema_version":"benchmark.private-query.v1","dataset_kind":"private-real-corpus","target_claim":"benchmark_baseline_observed"}\n'
      exit 0
    fi
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "smoke-low-hot" ]; then
      printf '{"schema_version":"benchmark.v1","run_id":"current_stage_fake","platform":"ci/fake","dataset_kind":"private-real-corpus","document_count":9133,"searchable_document_count":41,"vector_indexed_document_count":29,"query_count":3,"request_sample_count":3,"bucket_counts":{"single_term":0,"and_2":0,"and_3_5":3,"and_6_16":0,"field_filter":0,"hybrid":0,"semantic":0},'
      private_query_split_low_hot_json
      printf '"samples_per_bucket":{"single_term":0,"and_2":0,"and_3_5":3,"and_6_16":0,"field_filter":0,"hybrid":0,"semantic":0},"top_k":5,"build_ms":1.0,"query_total_ms":300.0,"qps":10.0,"index_size_bytes":1000,"query_latency_ms":{"samples":3,"min":1.0,"mean":5.0,"p50":5.0,"p95":42.0,"p99":84.0,"max":100.0},'
      query_latency_by_bucket_json single_term 0 and_2 0 and_3_5 3 and_6_16 0 field_filter 0 hybrid 0 semantic 0
      stage_latency_json 3
      stage_latency_by_bucket_json single_term 0 and_2 0 and_3_5 3 and_6_16 0 field_filter 0 hybrid 0 semantic 0
      stage_histogram_json 3
      stage_histogram_by_bucket_json single_term 0 and_2 0 and_3_5 3 and_6_16 0 field_filter 0 hybrid 0 semantic 0
      rss_delta_json 3
      rss_delta_by_bucket_json single_term 0 and_2 0 and_3_5 3 and_6_16 0 field_filter 0 hybrid 0 semantic 0
      printf '"zero_result_queries":0,"total_hits":30,"million_scale_verified":false,"percentile_confidence":"smoke","target_claim":"benchmark_baseline_observed","scope":"private local real-corpus query benchmark; aggregate redacted report only","corpus_origin":"private_local","privacy_boundary":"redacted_local_aggregate","query_protocol":"resume-ir-query-v2","query_source":"trace_source_search_v1","private_scale_gate":null,"query_runner":"resident-batch-command","spawn_per_query":false,"query_mode":"hybrid","retrieval_layers":"fulltext+field+vector+rrf","warm_or_cold_definition":"current_stage_single_resident_batch_no_extra_warmup","cache_state":"hot_index_fully_covered_resident_batch_os_cache_uncontrolled","query_embedding_runtime":"local-command","query_embedding_command_invocations":3,"hot_index":true,"hot_path_ocr":false,"hot_path_parsing":false,"hot_path_heavy_model_inference":false,"contains_raw_resume_text":false,"contains_resume_paths":false,"contains_queries":false,"dataset_manifest_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","query_set_sha256":"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789","model_manifest_sha256":"1111111111111111111111111111111111111111111111111111111111111111","corpus_summary_sha256":"2222222222222222222222222222222222222222222222222222222222222222"}\n'
      exit 0
    fi
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "private-query-low-bucket" ]; then
      printf '{"schema_version":"benchmark.v1","run_id":"current_stage_fake","platform":"ci/fake","dataset_kind":"private-real-corpus","document_count":10000,"searchable_document_count":8000,"vector_indexed_document_count":8000,"query_count":500,"request_sample_count":5000,"bucket_counts":{"single_term":0,"and_2":0,"and_3_5":500,"and_6_16":0,"field_filter":0,"hybrid":0,"semantic":0},'
      private_query_split_low_bucket_json
      printf '"samples_per_bucket":{"single_term":0,"and_2":0,"and_3_5":5000,"and_6_16":0,"field_filter":0,"hybrid":0,"semantic":0},"top_k":10,"build_ms":1.0,"query_total_ms":5000.0,"qps":1000.0,"index_size_bytes":1000,"query_latency_ms":{"samples":5000,"min":1.0,"mean":5.0,"p50":5.0,"p95":42.0,"p99":84.0,"max":100.0},'
      query_latency_by_bucket_json single_term 0 and_2 0 and_3_5 5000 and_6_16 0 field_filter 0 hybrid 0 semantic 0
      stage_latency_json 5000
      stage_latency_by_bucket_json single_term 0 and_2 0 and_3_5 5000 and_6_16 0 field_filter 0 hybrid 0 semantic 0
      stage_histogram_json 5000
      stage_histogram_by_bucket_json single_term 0 and_2 0 and_3_5 5000 and_6_16 0 field_filter 0 hybrid 0 semantic 0
      rss_delta_json 5000
      rss_delta_by_bucket_json single_term 0 and_2 0 and_3_5 5000 and_6_16 0 field_filter 0 hybrid 0 semantic 0
      printf '"zero_result_queries":0,"total_hits":50000,"million_scale_verified":false,"percentile_confidence":"sampled","target_claim":"benchmark_baseline_observed","scope":"private local real-corpus query benchmark; aggregate redacted report only","corpus_origin":"private_local","privacy_boundary":"redacted_local_aggregate","query_protocol":"resume-ir-query-v2","query_source":"trace_source_search_v1","private_scale_gate":"D10K_private_calibration","query_runner":"resident-batch-command","spawn_per_query":false,"query_mode":"hybrid","retrieval_layers":"fulltext+field+vector+rrf","warm_or_cold_definition":"current_stage_single_resident_batch_no_extra_warmup","cache_state":"hot_index_fully_covered_resident_batch_os_cache_uncontrolled","query_embedding_runtime":"local-command","query_embedding_command_invocations":5000,"hot_index":true,"hot_path_ocr":false,"hot_path_parsing":false,"hot_path_heavy_model_inference":false,"contains_raw_resume_text":false,"contains_resume_paths":false,"contains_queries":false,"dataset_manifest_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","query_set_sha256":"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789","model_manifest_sha256":"1111111111111111111111111111111111111111111111111111111111111111","corpus_summary_sha256":"2222222222222222222222222222222222222222222222222222222222222222"}\n'
      exit 0
    fi
    private_scale_gate_json='"D10K_private_calibration"'
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "private-query-missing-scale-gate" ]; then
      private_scale_gate_json='null'
    fi
    printf '{"schema_version":"benchmark.v1","run_id":"current_stage_fake","platform":"ci/fake","dataset_kind":"private-real-corpus","document_count":10000,"searchable_document_count":8000,"vector_indexed_document_count":8000,"query_count":500,"request_sample_count":5000,"bucket_counts":{"single_term":50,"and_2":75,"and_3_5":150,"and_6_16":50,"field_filter":75,"hybrid":75,"semantic":25},'
    private_query_split_full_json
    printf '"samples_per_bucket":{"single_term":500,"and_2":625,"and_3_5":1500,"and_6_16":500,"field_filter":625,"hybrid":625,"semantic":625},"top_k":10,"build_ms":1.0,"query_total_ms":5000.0,"qps":1000.0,"index_size_bytes":1000,"query_latency_ms":{"samples":5000,"min":1.0,"mean":5.0,"p50":5.0,"p95":42.0,"p99":84.0,"max":100.0},'
    query_latency_by_bucket_json single_term 500 and_2 625 and_3_5 1500 and_6_16 500 field_filter 625 hybrid 625 semantic 625
    stage_latency_json 5000
    stage_latency_by_bucket_json single_term 500 and_2 625 and_3_5 1500 and_6_16 500 field_filter 625 hybrid 625 semantic 625
    stage_histogram_json 5000
    stage_histogram_by_bucket_json single_term 500 and_2 625 and_3_5 1500 and_6_16 500 field_filter 625 hybrid 625 semantic 625
    rss_delta_json 5000
    rss_delta_by_bucket_json single_term 500 and_2 625 and_3_5 1500 and_6_16 500 field_filter 625 hybrid 625 semantic 625
    printf '"zero_result_queries":0,"total_hits":50000,"million_scale_verified":false,"percentile_confidence":"sampled","target_claim":"benchmark_baseline_observed","scope":"private local real-corpus query benchmark; aggregate redacted report only","corpus_origin":"private_local","privacy_boundary":"redacted_local_aggregate","query_protocol":"resume-ir-query-v2","query_source":"trace_source_search_v1","private_scale_gate":%s,"query_runner":"resident-batch-command","spawn_per_query":false,"query_mode":"hybrid","retrieval_layers":"fulltext+field+vector+rrf","warm_or_cold_definition":"current_stage_single_resident_batch_no_extra_warmup","cache_state":"hot_index_fully_covered_resident_batch_os_cache_uncontrolled","query_embedding_runtime":"local-command","query_embedding_command_invocations":5000,"hot_index":true,"hot_path_ocr":false,"hot_path_parsing":false,"hot_path_heavy_model_inference":false,"contains_raw_resume_text":false,"contains_resume_paths":false,"contains_queries":false,"dataset_manifest_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","query_set_sha256":"%s","model_manifest_sha256":"1111111111111111111111111111111111111111111111111111111111111111","corpus_summary_sha256":"2222222222222222222222222222222222222222222222222222222222222222"}\n' "$private_scale_gate_json" "$query_set_report_sha"
    ;;
  gate)
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "gate-failed" ]; then
      printf 'benchmark gate blocked: private real-corpus hot-index coverage floor not met\n' >&2
      exit 1
    fi
    report_path=""
    allow_smoke="false"
    max_zero_result_queries=""
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --report)
          shift
          report_path="${1:-}"
          ;;
        --allow-smoke-confidence)
          allow_smoke="true"
          ;;
        --max-zero-result-queries)
          shift
          max_zero_result_queries="${1:-}"
          ;;
      esac
      shift || true
    done
    if [ "$max_zero_result_queries" != "0" ]; then
      printf 'benchmark gate blocked: current-stage query set must not allow zero-result queries\n' >&2
      exit 1
    fi
    if [ -z "$report_path" ] || [ ! -s "$report_path" ]; then
      printf 'benchmark gate blocked: missing benchmark report\n' >&2
      exit 1
    fi
    for required in \
      '"schema_version":"benchmark.v1"' \
      '"dataset_kind":"private-real-corpus"' \
      '"privacy_boundary":"redacted_local_aggregate"' \
      '"contains_raw_resume_text":false' \
      '"contains_resume_paths":false' \
      '"contains_queries":false' \
      '"query_latency_ms":' \
      '"query_latency_by_bucket":' \
      '"stage_latency_ms":' \
      '"stage_latency_by_bucket_ms":' \
      '"rss_delta_mb":' \
      '"rss_delta_mb_by_bucket":' \
      '"tune_sha256":' \
      '"holdout_sha256":' \
      '"tune_bucket_counts":' \
      '"holdout_bucket_counts":'
    do
      if ! grep -Fq "$required" "$report_path"; then
        printf 'benchmark gate blocked: invalid private benchmark report\n' >&2
        exit 1
      fi
    done
    if [ "$allow_smoke" = "true" ]; then
      if ! grep -Fq '"samples":' "$report_path"; then
        printf 'benchmark gate blocked: invalid private benchmark report\n' >&2
        exit 1
      fi
    elif ! grep -Fq '"samples":500' "$report_path"; then
      printf 'benchmark gate blocked: invalid private benchmark report\n' >&2
      exit 1
    fi
    printf 'benchmark gate passed\n'
    ;;
  private-ocr-throughput)
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "private-ocr-throughput-failed" ]; then
      printf 'private OCR throughput baseline blocked: fake OCR runtime failure\n' >&2
      exit 1
    fi
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "private-ocr-throughput-invalid" ]; then
      printf '{"schema_version":"ocr-throughput.v1","dataset_kind":"private-real-corpus","target_claim":"ocr_throughput_baseline_observed"}\n'
      exit 0
    fi
    printf '{"schema_version":"ocr-throughput.v1","run_id":"current_stage_ocr_fake","platform":"ci/fake","engine_kind":"tesseract","dataset_kind":"private-real-corpus","target_claim":"ocr_throughput_baseline_observed","scope":"private real-corpus OCR throughput benchmark; aggregate redacted report only","corpus_origin":"private_local","privacy_boundary":"redacted_local_aggregate","contains_raw_ocr_text":false,"contains_page_images":false,"contains_resume_paths":false,"contains_document_ids":false,"contains_page_ids":false,"contains_command_paths":false,"document_count":8720,"scanned_document_count":500,"page_count":500,"failed_document_count":0,"render_failure_count":0,"ocr_failure_count":0,"total_ms":1000,"pages_per_second":500.0,"run_budget_exhausted":false,"page_latency_ms":{"samples":500,"p50":250.0,"p95":450.0,"p99":800.0},"dataset_manifest_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","ocr_runtime_manifest_sha256":"3333333333333333333333333333333333333333333333333333333333333333","renderer_manifest_sha256":"4444444444444444444444444444444444444444444444444444444444444444","language_pack_manifest_sha256":"5555555555555555555555555555555555555555555555555555555555555555"}\n'
    ;;
  ocr-gate)
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "ocr-gate-failed" ]; then
      printf 'OCR throughput gate blocked: private real-corpus page floor not met\n' >&2
      exit 1
    fi
    printf 'ocr throughput gate passed\n'
    ;;
  *)
    printf 'unexpected fake resume-benchmark command\n' >&2
    exit 64
    ;;
esac
SH
chmod 700 "$fake_resume_benchmark"

run_execute_smoke() {
  mode="$1"
  shift
  stdout_file="$tmpdir/execute-$mode-stdout.txt"
  stderr_file="$tmpdir/execute-$mode-stderr.txt"
  rm -rf "$execute_data_dir" "$execute_out_dir"
  mkdir -p "$execute_data_dir" "$execute_out_dir"
  set +e
  benchmark_mode="pass"
  if [ "$mode" = "benchmark-gate-failed" ]; then
    benchmark_mode="gate-failed"
  fi
  if [ "$mode" = "private-query-failed" ]; then
    benchmark_mode="private-query-failed"
  fi
  if [ "$mode" = "private-query-invalid" ]; then
    benchmark_mode="private-query-invalid"
  fi
  if [ "$mode" = "private-query-low-bucket" ]; then
    benchmark_mode="private-query-low-bucket"
  fi
  if [ "$mode" = "private-query-missing-scale-gate" ]; then
    benchmark_mode="private-query-missing-scale-gate"
  fi
  if [ "$mode" = "private-query-sha-mismatch" ]; then
    benchmark_mode="private-query-sha-mismatch"
  fi
  if [ "$mode" = "smoke-low-hot" ]; then
    benchmark_mode="smoke-low-hot"
  fi
  if [ "$mode" = "private-ocr-throughput-failed" ]; then
    benchmark_mode="private-ocr-throughput-failed"
  fi
  if [ "$mode" = "private-ocr-throughput-invalid" ]; then
    benchmark_mode="private-ocr-throughput-invalid"
  fi
  if [ "$mode" = "ocr-gate-failed" ]; then
    benchmark_mode="ocr-gate-failed"
  fi
  diagnostics_mode="ready"
  if [ "$mode" = "diagnostics-failed" ]; then
    diagnostics_mode="failed"
  fi
  if [ "$mode" = "diagnostics-invalid" ]; then
    diagnostics_mode="invalid"
  fi
  fault_simulation_mode="ready"
  if [ "$mode" = "fault-simulation-failed" ]; then
    fault_simulation_mode="failed"
  fi
  if [ "$mode" = "fault-simulation-invalid" ]; then
    fault_simulation_mode="invalid-suite"
  fi
  import_mode="ready"
  if [ "$mode" = "import-failed" ]; then
    import_mode="failed"
  fi
  if [ "$mode" = "reuse-imported-corpus" ]; then
    import_mode="forbid-scan"
  fi
  status_mode="ready"
  if [ "$mode" = "reuse-imported-corpus-recoverable" ]; then
    import_mode="forbid-scan"
    status_mode="recoverable"
  fi
  query_set_mode="ready"
  if [ "$mode" = "query-set-prepare-failed" ]; then
    query_set_mode="prepare-failed"
  fi
  if [ "$mode" = "query-set-index-unavailable" ]; then
    query_set_mode="index-unavailable"
  fi
  if [ "$mode" = "query-set-d10k-corpus-not-ready" ]; then
    query_set_mode="d10k-corpus-not-ready"
  fi
  if [ "$mode" = "query-set-summary-missing" ]; then
    query_set_mode="missing-summary"
  fi
  if [ "$mode" = "query-set-local-field-source" ]; then
    query_set_mode="local-field-source"
  fi
  query_set_arg_name="--query-set-trace-root"
  query_set_arg_value="$execute_query_set_trace_root"
  if [ "$mode" = "provided-query-set" ]; then
    query_set_arg_name="--query-set"
    query_set_arg_value="$execute_query_set"
  fi
  corpus_summary_mode="hot"
  if [ "$mode" = "ocr-backlog" ]; then
    corpus_summary_mode="ocr-backlog"
  fi
  if [ "$mode" = "smoke-low-hot" ]; then
    corpus_summary_mode="smoke-low-hot"
  fi
  FAKE_BENCHMARK_MODE="$benchmark_mode" FAKE_CORPUS_SUMMARY_MODE="$corpus_summary_mode" FAKE_DIAGNOSTICS_MODE="$diagnostics_mode" FAKE_FAULT_SIMULATION_MODE="$fault_simulation_mode" FAKE_IMPORT_MODE="$import_mode" FAKE_STATUS_MODE="$status_mode" FAKE_QUERY_SET_MODE="$query_set_mode" FAKE_RELEASE_READINESS_MODE="$mode" FAKE_REQUIRED_EMBEDDING_RUNTIME_BIN_DIR="$embedding_runtime_bin_dir" FAKE_REQUIRED_QUERY_SET_TRACE_ROOT="$execute_query_set_trace_root" FAKE_RUNTIME_PREFLIGHT_MODE="$mode" "$script" --execute \
    --resume-cli "$fake_resume_cli" \
    --resume-daemon "$fake_resume_daemon" \
    --resume-benchmark "$fake_resume_benchmark" \
    --resume-root "$execute_resume_root" \
    --data-dir "$execute_data_dir" \
    --out-dir "$execute_out_dir" \
    "$query_set_arg_name" "$query_set_arg_value" \
    --model-manifest "$execute_model_manifest" \
    --ocr-runtime-manifest "$execute_ocr_manifest" \
    --model-artifact "$execute_model_artifact" \
    --embedding-command "$execute_embedding_command" \
    --embedding-runtime-bin-dir "$embedding_runtime_bin_dir" \
    --model-pack-id reviewed-local-model-pack \
    --model-id reviewed-local-embedding-model \
    --model-format onnx \
    --dimension 384 \
    --model-license Apache-2.0 \
    --runtime-pack-id reviewed-local-ocr-pack \
    --runtime-distribution-mode bundled \
    --tesseract-command "$execute_tesseract_command" \
    --pdftoppm-command "$execute_pdftoppm_command" \
    --language eng \
    --language-pack "$execute_language_pack" \
    --engine-license Apache-2.0 \
    --renderer-license GPL-2.0-or-later \
    --language-license Apache-2.0 \
    --max-files 10000 \
    --max-queries 500 \
    --top-k 10 \
    "$@" \
    > "$stdout_file" 2> "$stderr_file"
  status=$?
  set -e
  printf '%s' "$status" > "$tmpdir/execute-$mode-status.txt"
}

run_execute_smoke blocked
blocked_status=$(cat "$tmpdir/execute-blocked-status.txt")
if [ "$blocked_status" -ne 0 ]; then
  fail "current-stage execute rejected expected blocked release-readiness status"
fi
evidence_manifest="$execute_out_dir/current-stage-validation-evidence.json"
if [ ! -s "$evidence_manifest" ]; then
  fail "current-stage execute did not write redacted evidence manifest"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$evidence_manifest" >/dev/null
fi
require_text "$evidence_manifest" '"schema_version": "resume-ir.current-stage-validation-evidence.v1"'
require_text "$evidence_manifest" '"privacy_boundary": "local_only_redacted_evidence_manifest"'
require_text "$evidence_manifest" '"current_stage_target": "reproducible_local_10k_baseline"'
require_text "$evidence_manifest" '"runtime_distribution_mode": "bundled"'
require_text "$evidence_manifest" '"runtime_package_binaries_included": true'
require_text "$evidence_manifest" '"private_query_timeout_ms": 30000'
require_text "$evidence_manifest" '"performance_optimization_deferred": true'
require_text "$evidence_manifest" '"release_readiness_exit": 1'
require_text "$evidence_manifest" '"stable_release_expected_blocked": true'
expected_dataset_manifest_sha256=$(sha256_file "$execute_out_dir/dataset-manifest.local.json")
require_text "$evidence_manifest" "\"dataset_manifest_sha256\": \"$expected_dataset_manifest_sha256\""
expected_query_set_sha256=abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789
require_text "$evidence_manifest" "\"query_set_sha256\": \"$expected_query_set_sha256\""
expected_model_manifest_sha256=$(sha256_file "$execute_model_manifest")
require_text "$evidence_manifest" "\"model_manifest_sha256\": \"$expected_model_manifest_sha256\""
expected_ocr_manifest_sha256=$(sha256_file "$execute_ocr_manifest")
require_text "$evidence_manifest" "\"ocr_runtime_manifest_sha256\": \"$expected_ocr_manifest_sha256\""
require_text "$evidence_manifest" '"preflight_probes": {'
require_text "$evidence_manifest" '"ocr_runtime_probe": "passed"'
require_text "$evidence_manifest" '"embedding_protocol": "passed"'
require_full_evidence_observability "$evidence_manifest"
require_text "$evidence_manifest" '"private_query_observability": {'
require_text "$evidence_manifest" '"query_source": "trace_source_search_v1"'
require_text "$evidence_manifest" '"private_scale_gate": "D10K_private_calibration"'
require_text "$evidence_manifest" '"query_runner": "resident-batch-command"'
require_text "$evidence_manifest" '"spawn_per_query": false'
require_text "$evidence_manifest" '"request_sample_count": 5000'
require_text "$evidence_manifest" '"query_latency_by_bucket": {'
require_text "$evidence_manifest" '"stage_latency_p95_ms": {'
require_text "$evidence_manifest" '"stage_latency_by_bucket_p95_ms": {'
require_text "$evidence_manifest" '"stage_histogram_ms": {'
require_text "$evidence_manifest" '"stage_histogram_by_bucket_ms": {'
require_text "$evidence_manifest" '"rss_delta_mb": {'
require_text "$evidence_manifest" '"rss_delta_mb_by_bucket": {'
require_text "$evidence_manifest" '"dataset-manifest.local.json"'
require_text "$evidence_manifest" '"dataset-manifest.stdout.txt"'
require_text "$evidence_manifest" '"model-manifest.local.json"'
require_text "$evidence_manifest" '"ocr-runtime-manifest.local.json"'
require_text "$evidence_manifest" '"private-query-set.local.jsonl"'
require_text "$evidence_manifest" '"private-query-set.summary.json"'
require_text "$evidence_manifest" '"query-set-prepare.stdout.txt"'
require_text "$evidence_manifest" '"benchmark-corpus-summary.local.json"'
require_text "$evidence_manifest" '"private-benchmark-local.json"'
require_text "$evidence_manifest" '"private-ocr-throughput.json"'
require_text "$evidence_manifest" '"ocr-throughput-gate.stdout.txt"'
require_text "$evidence_manifest" '"redacted-diagnostics.json"'
require_text "$evidence_manifest" '"doctor", "status": "success"'
require_text "$evidence_manifest" '"doctor.out"'
require_text "$evidence_manifest" '"fault_simulation_smoke"'
require_text "$evidence_manifest" '"fault-simulation-storage-low.json"'
require_text "$evidence_manifest" '"fault_simulation_suite"'
require_text "$evidence_manifest" '"fault-simulation-suite-local-safe.json"'
require_fault_suite_evidence "$execute_out_dir/fault-simulation-suite-local-safe.json"
require_text "$evidence_manifest" '"release-readiness.json"'
require_text "$evidence_manifest" '"local_paths_included": false'
require_text "$evidence_manifest" '"raw_resume_text_included": false'
require_text "$evidence_manifest" '"raw_query_text_included": false'
require_text "$evidence_manifest" '"model_bytes_included": false'
require_text "$evidence_manifest" '"runtime_binaries_included": false'
require_current_stage_handoff \
  "full_evidence_ready" \
  "resume-ir.current-stage-validation-evidence.v1"
issue_comment="$execute_out_dir/current-stage-issue-comment.md"
require_text "$issue_comment" "#53 Current-Stage Private Query Baseline Handoff"
require_text "$issue_comment" "query_source: trace_source_search_v1"
require_text "$issue_comment" "private_scale_gate: D10K_private_calibration"
require_text "$issue_comment" "query_set_sha256: $expected_query_set_sha256"
require_text "$issue_comment" "request_sample_count: 5000"
expected_private_benchmark_sha256=$(sha256_file "$execute_out_dir/private-benchmark-local.json")
expected_query_set_summary_sha256=$(sha256_file "$execute_out_dir/private-query-set.summary.json")
require_text "$issue_comment" "benchmark_report_hash: $expected_private_benchmark_sha256 (private-benchmark-local.json)"
require_text "$issue_comment" "query_set_summary_hash: $expected_query_set_summary_sha256 (private-query-set.summary.json)"
require_text "$issue_comment" "not goal_complete; not a profile optimization issue closure"
reject_text "$issue_comment" "$tmpdir"
reject_text "$issue_comment" "PRIVATE-current-stage"
reject_text "$issue_comment" "private-query-set.local.jsonl"
reject_text "$issue_comment" "private fake query"
reject_text "$evidence_manifest" "$tmpdir"
reject_text "$evidence_manifest" "PRIVATE-current-stage"
reject_text "$evidence_manifest" "private fake query"
require_text "$tmpdir/execute-blocked-stdout.txt" "current-stage validation: release-readiness exit 1"
require_text "$tmpdir/execute-blocked-stdout.txt" "current-stage validation: local evidence written under <local-evidence-dir>"
reject_text "$tmpdir/execute-blocked-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-blocked-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-blocked-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-blocked-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke provided-query-set
provided_query_set_status=$(cat "$tmpdir/execute-provided-query-set-status.txt")
if [ "$provided_query_set_status" -ne 0 ]; then
  fail "current-stage execute rejected provided static query set"
fi
provided_query_set_stdout="$execute_out_dir/query-set-prepare.stdout.txt"
require_text "$provided_query_set_stdout" "query set: provided"
require_text "$provided_query_set_stdout" "schema: resume-ir.query-set.jsonl.v2"
require_text "$provided_query_set_stdout" "privacy boundary: local_only_private_query_set"
require_text "$provided_query_set_stdout" "queries: <redacted>"
require_text "$provided_query_set_stdout" "paths: <redacted>"
require_text "$provided_query_set_stdout" "query source: trace_source_search_v1"
require_text "$provided_query_set_stdout" "query set sha256: abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
require_text "$provided_query_set_stdout" "tune sha256: 2222222222222222222222222222222222222222222222222222222222222222"
require_text "$provided_query_set_stdout" "holdout sha256: 3333333333333333333333333333333333333333333333333333333333333333"
reject_text "$provided_query_set_stdout" "$tmpdir"
reject_text "$provided_query_set_stdout" "PRIVATE-current-stage"
reject_text "$provided_query_set_stdout" "private fake query"

run_execute_smoke smoke-profile \
  --validation-profile smoke \
  --max-files 6 \
  --max-queries 3 \
  --top-k 5
smoke_status=$(cat "$tmpdir/execute-smoke-profile-status.txt")
if [ "$smoke_status" -ne 0 ]; then
  fail "current-stage smoke profile execute failed"
fi
smoke_summary="$execute_out_dir/current-stage-smoke-summary.json"
if [ ! -s "$smoke_summary" ]; then
  fail "current-stage smoke profile did not write redacted smoke summary"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage smoke profile wrote full current-stage release evidence"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage smoke profile ran release-readiness intake"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$smoke_summary" >/dev/null
fi
require_text "$smoke_summary" '"schema_version": "resume-ir.current-stage-smoke-summary.v1"'
require_text "$smoke_summary" '"privacy_boundary": "local_only_redacted_aggregate_summary"'
require_text "$smoke_summary" '"validation_profile": "smoke"'
require_text "$smoke_summary" '"current_stage_target": "local_real_corpus_smoke_chain"'
require_text "$smoke_summary" '"runtime_distribution_mode": "bundled"'
require_text "$smoke_summary" '"runtime_package_binaries_included": true'
require_text "$smoke_summary" '"private_query_timeout_ms": 30000'
require_text "$smoke_summary" '"smoke_satisfied": true'
require_text "$smoke_summary" '"full_baseline_satisfied": false'
require_text "$smoke_summary" '"release_readiness_evidence": false'
require_text "$smoke_summary" '"ocr_runtime_probe": "passed"'
require_text "$smoke_summary" '"embedding_protocol": "passed"'
require_summary_observability "$smoke_summary"
require_text "$smoke_summary" '"document_status_counts": {'
require_text "$smoke_summary" '"ingest_job_kind_status_counts": {'
require_text "$smoke_summary" '"private_query_baseline"'
require_text "$smoke_summary" '"redacted_diagnostics"'
require_text "$smoke_summary" '"doctor", "status": "success"'
require_text "$smoke_summary" '"doctor.out"'
require_text "$smoke_summary" '"fault_simulation_smoke"'
require_text "$smoke_summary" '"fault-simulation-storage-low.json"'
require_text "$smoke_summary" '"fault_simulation_suite"'
require_text "$smoke_summary" '"fault-simulation-suite-local-safe.json"'
require_fault_suite_evidence "$execute_out_dir/fault-simulation-suite-local-safe.json"
require_text "$smoke_summary" '"full 10k/8000-document current-stage baseline"'
require_current_stage_handoff \
  "smoke_satisfied" \
  "resume-ir.current-stage-smoke-summary.v1"
reject_text "$smoke_summary" "$tmpdir"
reject_text "$smoke_summary" "PRIVATE-current-stage"
reject_text "$smoke_summary" "private fake query"
require_text "$tmpdir/execute-smoke-profile-stdout.txt" "current-stage validation: smoke summary written under <local-evidence-dir>"
reject_text "$tmpdir/execute-smoke-profile-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-smoke-profile-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-smoke-profile-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-smoke-profile-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke smoke-low-hot \
  --validation-profile smoke \
  --max-files 6 \
  --max-queries 3 \
  --top-k 5
smoke_low_hot_status=$(cat "$tmpdir/execute-smoke-low-hot-status.txt")
if [ "$smoke_low_hot_status" -ne 0 ]; then
  fail "current-stage smoke profile rejected partial hot-index benchmark evidence"
fi
smoke_low_hot_summary="$execute_out_dir/current-stage-smoke-summary.json"
require_text "$smoke_low_hot_summary" '"smoke_satisfied": true'
require_text "$smoke_low_hot_summary" '"private_query_baseline"'
require_partial_hot_index_observability "$smoke_low_hot_summary"
require_current_stage_handoff \
  "smoke_satisfied" \
  "resume-ir.current-stage-smoke-summary.v1"
handoff="$execute_out_dir/current-stage-handoff.json"
require_text "$handoff" '"kind": "derived_blocker"'
require_text "$handoff" '"category": "ocr"'
require_text "$handoff" '"reason": "ocr_backlog_present"'
require_text "$handoff" '"category": "import/parser"'
require_text "$handoff" '"reason": "failed_permanent_documents_present"'
reject_text "$smoke_low_hot_summary" "$tmpdir"
reject_text "$smoke_low_hot_summary" "PRIVATE-current-stage"
reject_text "$smoke_low_hot_summary" "private fake query"

reuse_dataset_manifest="$tmpdir/reuse-dataset-manifest.local.json"
cat > "$reuse_dataset_manifest" <<'JSON'
{
  "schema_version": "resume-ir.dataset-manifest.v1",
  "privacy_boundary": "local_only_redacted_dataset_manifest",
  "dataset_kind": "private-local-corpus",
  "scan_profile": "explicit",
  "file_count": 8720,
  "extension_counts": {
    "docx": 720,
    "pdf": 8000
  },
  "contains_paths": false,
  "contains_file_names": false,
  "contains_raw_resume_text": false,
  "contains_file_hashes": false,
  "corpus_fingerprint_sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}
JSON
run_execute_smoke reuse-imported-corpus \
  --validation-profile smoke \
  --reuse-imported-corpus \
  --reuse-dataset-manifest "$reuse_dataset_manifest" \
  --max-files 6 \
  --max-queries 3 \
  --top-k 5
reuse_status=$(cat "$tmpdir/execute-reuse-imported-corpus-status.txt")
if [ "$reuse_status" -ne 0 ]; then
  fail "current-stage reuse-imported-corpus smoke execute failed"
fi
reuse_summary="$execute_out_dir/current-stage-smoke-summary.json"
if [ ! -s "$reuse_summary" ]; then
  fail "current-stage reuse-imported-corpus did not write smoke summary"
fi
require_text "$reuse_summary" '"reuse_imported_corpus": true'
require_text "$reuse_summary" '"private_query_timeout_ms": 30000'
expected_reuse_dataset_manifest_sha256=$(sha256_file "$reuse_dataset_manifest")
require_text "$reuse_summary" "\"dataset_manifest_sha256\": \"$expected_reuse_dataset_manifest_sha256\""
require_text "$execute_out_dir/dataset-manifest.stdout.txt" "dataset manifest: reused"
require_text "$execute_out_dir/dataset-manifest.stdout.txt" "privacy boundary: local_only_redacted_dataset_manifest"
require_reused_import_stdout "$execute_out_dir/import.stdout.txt"
require_current_stage_handoff \
  "smoke_satisfied" \
  "resume-ir.current-stage-smoke-summary.v1"
reject_text "$reuse_summary" "$tmpdir"
reject_text "$reuse_summary" "PRIVATE-current-stage"
reject_text "$reuse_summary" "private fake query"
reject_text "$execute_out_dir/dataset-manifest.stdout.txt" "$tmpdir"
reject_text "$execute_out_dir/import.stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-reuse-imported-corpus-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-reuse-imported-corpus-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-reuse-imported-corpus-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-reuse-imported-corpus-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke reuse-imported-corpus-recoverable \
  --validation-profile smoke \
  --reuse-imported-corpus \
  --reuse-dataset-manifest "$reuse_dataset_manifest" \
  --max-files 6 \
  --max-queries 3 \
  --top-k 5
reuse_recoverable_status=$(cat "$tmpdir/execute-reuse-imported-corpus-recoverable-status.txt")
if [ "$reuse_recoverable_status" -eq 0 ]; then
  fail "current-stage reuse-imported-corpus accepted recoverable import work"
fi
reuse_recoverable_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$reuse_recoverable_summary" ]; then
  fail "current-stage reuse-imported-corpus recoverable case did not write blocked summary"
fi
if [ -e "$execute_out_dir/current-stage-smoke-summary.json" ]; then
  fail "current-stage reuse-imported-corpus recoverable case wrote smoke summary"
fi
require_text "$reuse_recoverable_summary" '"blocked_step": "import_private_corpus"'
require_text "$reuse_recoverable_summary" '"blocked_category": "import/parser"'
require_text "$reuse_recoverable_summary" '"blocked_reason": "reuse_imported_corpus_recoverable_task_present"'
require_text "$reuse_recoverable_summary" '"private_corpus_read": false'
require_text "$execute_out_dir/import.stdout.txt" "import tasks recoverable: 1"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
require_text "$tmpdir/execute-reuse-imported-corpus-recoverable-stderr.txt" "current-stage validation blocked: reusable data-dir still has recoverable import work"
reject_text "$reuse_recoverable_summary" "$tmpdir"
reject_text "$reuse_recoverable_summary" "PRIVATE-current-stage"
reject_text "$reuse_recoverable_summary" "private fake query"
reject_text "$execute_out_dir/import.stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-reuse-imported-corpus-recoverable-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-reuse-imported-corpus-recoverable-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-reuse-imported-corpus-recoverable-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-reuse-imported-corpus-recoverable-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke external-runtime-smoke \
  --validation-profile smoke \
  --runtime-distribution-mode external \
  --max-files 6 \
  --max-queries 3 \
  --top-k 5
external_smoke_status=$(cat "$tmpdir/execute-external-runtime-smoke-status.txt")
if [ "$external_smoke_status" -ne 0 ]; then
  fail "current-stage external runtime smoke profile execute failed"
fi
external_smoke_summary="$execute_out_dir/current-stage-smoke-summary.json"
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$external_smoke_summary" >/dev/null
fi
require_text "$external_smoke_summary" '"runtime_distribution_mode": "external"'
require_text "$external_smoke_summary" '"runtime_package_binaries_included": false'
require_text "$external_smoke_summary" '"runtime_binaries_included": false'
require_current_stage_handoff \
  "smoke_satisfied" \
  "resume-ir.current-stage-smoke-summary.v1"
reject_text "$external_smoke_summary" "$tmpdir"
reject_text "$external_smoke_summary" "PRIVATE-current-stage"

run_execute_smoke ocr-backlog
ocr_backlog_status=$(cat "$tmpdir/execute-ocr-backlog-status.txt")
if [ "$ocr_backlog_status" -eq 0 ]; then
  fail "current-stage full profile accepted bounded OCR backlog as full evidence"
fi
ocr_backlog_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$ocr_backlog_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary for bounded OCR backlog"
fi
if [ -e "$execute_out_dir/query-set-prepare.stdout.txt" ]; then
  fail "current-stage execute drafted private queries after bounded OCR backlog"
fi
if [ -e "$execute_out_dir/private-benchmark-local.json" ]; then
  fail "current-stage execute benchmarked private queries after bounded OCR backlog"
fi
ocr_backlog_diagnostics="$execute_out_dir/redacted-diagnostics.json"
if [ ! -s "$ocr_backlog_diagnostics" ]; then
  fail "current-stage execute did not write redacted diagnostics before bounded OCR backlog handoff"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after bounded OCR backlog"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after bounded OCR backlog"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$ocr_backlog_summary" >/dev/null
  python3 -m json.tool "$ocr_backlog_diagnostics" >/dev/null
fi
require_text "$ocr_backlog_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$ocr_backlog_summary" '"privacy_boundary": "local_only_redacted_blocked_summary"'
require_text "$ocr_backlog_summary" '"validation_profile": "full"'
require_text "$ocr_backlog_summary" '"current_stage_target": "reproducible_local_10k_baseline"'
require_text "$ocr_backlog_summary" '"full_baseline_satisfied": false'
require_text "$ocr_backlog_summary" '"release_readiness_evidence": false'
require_text "$ocr_backlog_summary" '"blocked_step": "ocr_worker_bounded_loop"'
require_text "$ocr_backlog_summary" '"blocked_category": "ocr"'
require_text "$ocr_backlog_summary" '"blocked_reason": "ocr_backlog_exceeds_current_stage_budget"'
require_text "$ocr_backlog_summary" '"ocr_runtime_probe": "passed"'
require_text "$ocr_backlog_summary" '"embedding_protocol": "passed"'
require_summary_observability "$ocr_backlog_summary"
require_text "$ocr_backlog_summary" '"hot_index_fully_covered": false'
require_text "$ocr_backlog_summary" '"redacted_diagnostics", "status": "success"'
require_text "$ocr_backlog_summary" '"doctor", "status": "success"'
require_text "$ocr_backlog_summary" '"benchmark-corpus-summary.local.json"'
require_text "$ocr_backlog_summary" '"redacted-diagnostics.json"'
require_text "$ocr_backlog_summary" '"doctor.out"'
require_text "$ocr_backlog_summary" '"full 10k/8000-document current-stage baseline"'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
require_text "$ocr_backlog_diagnostics" '"schema_version":"diagnostics.v1"'
require_text "$ocr_backlog_diagnostics" '"redacted":true'
reject_text "$ocr_backlog_summary" "$tmpdir"
reject_text "$ocr_backlog_summary" "PRIVATE-current-stage"
reject_text "$ocr_backlog_summary" "private fake query"
reject_text "$ocr_backlog_diagnostics" "$tmpdir"
reject_text "$ocr_backlog_diagnostics" "PRIVATE-current-stage"
reject_text "$ocr_backlog_diagnostics" "private fake query"
require_text "$tmpdir/execute-ocr-backlog-stderr.txt" "current-stage validation blocked: bounded OCR backlog remains"
reject_text "$tmpdir/execute-ocr-backlog-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-backlog-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-backlog-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-ocr-backlog-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke benchmark-gate-failed
benchmark_gate_failed_status=$(cat "$tmpdir/execute-benchmark-gate-failed-status.txt")
if [ "$benchmark_gate_failed_status" -eq 0 ]; then
  fail "current-stage full profile accepted failed benchmark gate"
fi
blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$blocked_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on benchmark gate failure"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage full profile wrote full evidence after benchmark gate failure"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage full profile ran release-readiness after benchmark gate failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$blocked_summary" >/dev/null
fi
require_text "$blocked_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$blocked_summary" '"privacy_boundary": "local_only_redacted_blocked_summary"'
require_text "$blocked_summary" '"validation_profile": "full"'
require_text "$blocked_summary" '"current_stage_target": "reproducible_local_10k_baseline"'
require_text "$blocked_summary" '"full_baseline_satisfied": false'
require_text "$blocked_summary" '"release_readiness_evidence": false'
require_text "$blocked_summary" '"blocked_step": "baseline_shape_gate"'
require_text "$blocked_summary" '"blocked_category": "benchmark"'
require_text "$blocked_summary" '"blocked_reason": "baseline_shape_gate_failed"'
require_text "$blocked_summary" '"ocr_runtime_probe": "passed"'
require_text "$blocked_summary" '"embedding_protocol": "passed"'
require_summary_observability "$blocked_summary"
require_text "$blocked_summary" '"document_status_counts": {'
require_text "$blocked_summary" '"ingest_job_kind_status_counts": {'
require_text "$blocked_summary" '"private-benchmark-local.json"'
require_text "$blocked_summary" '"private-benchmark-gate.stdout.txt"'
require_text "$blocked_summary" '"full 10k/8000-document current-stage baseline"'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$blocked_summary" "$tmpdir"
reject_text "$blocked_summary" "PRIVATE-current-stage"
reject_text "$blocked_summary" "private fake query"
require_text "$tmpdir/execute-benchmark-gate-failed-stderr.txt" "current-stage validation blocked: baseline shape gate failed"
reject_text "$tmpdir/execute-benchmark-gate-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-benchmark-gate-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-benchmark-gate-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-benchmark-gate-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke private-ocr-throughput-failed
private_ocr_throughput_failed_status=$(cat "$tmpdir/execute-private-ocr-throughput-failed-status.txt")
if [ "$private_ocr_throughput_failed_status" -eq 0 ]; then
  fail "current-stage full profile accepted failed private OCR throughput baseline"
fi
private_ocr_blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$private_ocr_blocked_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on private OCR throughput failure"
fi
if [ -e "$execute_out_dir/redacted-diagnostics.json" ]; then
  fail "current-stage execute ran diagnostics after private OCR throughput failure"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after private OCR throughput failure"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after private OCR throughput failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$private_ocr_blocked_summary" >/dev/null
fi
require_text "$private_ocr_blocked_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$private_ocr_blocked_summary" '"blocked_step": "private_ocr_throughput_baseline"'
require_text "$private_ocr_blocked_summary" '"blocked_category": "ocr"'
require_text "$private_ocr_blocked_summary" '"blocked_reason": "private_ocr_throughput_failed"'
require_text "$private_ocr_blocked_summary" '"ocr_throughput_min_pages": 500'
require_text "$private_ocr_blocked_summary" '"private-ocr-throughput.json"'
require_text "$private_ocr_blocked_summary" '"ocr-throughput-gate.stdout.txt"'
require_summary_observability "$private_ocr_blocked_summary"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$private_ocr_blocked_summary" "$tmpdir"
reject_text "$private_ocr_blocked_summary" "PRIVATE-current-stage"
reject_text "$private_ocr_blocked_summary" "private fake query"
require_text "$tmpdir/execute-private-ocr-throughput-failed-stderr.txt" "current-stage validation blocked: private OCR throughput baseline failed"
reject_text "$tmpdir/execute-private-ocr-throughput-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-private-ocr-throughput-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-private-ocr-throughput-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-private-ocr-throughput-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke private-ocr-throughput-invalid
private_ocr_throughput_invalid_status=$(cat "$tmpdir/execute-private-ocr-throughput-invalid-status.txt")
if [ "$private_ocr_throughput_invalid_status" -eq 0 ]; then
  fail "current-stage full profile accepted invalid private OCR throughput report"
fi
private_ocr_invalid_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$private_ocr_invalid_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on invalid private OCR throughput report"
fi
if [ -s "$execute_out_dir/ocr-throughput-gate.stdout.txt" ]; then
  fail "current-stage execute ran OCR throughput gate after invalid private OCR throughput report"
fi
if [ -e "$execute_out_dir/redacted-diagnostics.json" ]; then
  fail "current-stage execute ran diagnostics after invalid private OCR throughput report"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after invalid private OCR throughput report"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after invalid private OCR throughput report"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$private_ocr_invalid_summary" >/dev/null
fi
require_text "$private_ocr_invalid_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$private_ocr_invalid_summary" '"blocked_step": "private_ocr_throughput_baseline"'
require_text "$private_ocr_invalid_summary" '"blocked_category": "ocr"'
require_text "$private_ocr_invalid_summary" '"blocked_reason": "private_ocr_throughput_invalid"'
require_text "$private_ocr_invalid_summary" '"private-ocr-throughput.json"'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$private_ocr_invalid_summary" "$tmpdir"
reject_text "$private_ocr_invalid_summary" "PRIVATE-current-stage"
reject_text "$private_ocr_invalid_summary" "private fake query"
require_text "$tmpdir/execute-private-ocr-throughput-invalid-stderr.txt" "current-stage validation blocked: private OCR throughput evidence failed validation"
reject_text "$tmpdir/execute-private-ocr-throughput-invalid-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-private-ocr-throughput-invalid-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-private-ocr-throughput-invalid-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-private-ocr-throughput-invalid-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke ocr-gate-failed
ocr_gate_failed_status=$(cat "$tmpdir/execute-ocr-gate-failed-status.txt")
if [ "$ocr_gate_failed_status" -eq 0 ]; then
  fail "current-stage full profile accepted failed OCR throughput baseline gate"
fi
ocr_gate_blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$ocr_gate_blocked_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on OCR throughput gate failure"
fi
if [ -e "$execute_out_dir/redacted-diagnostics.json" ]; then
  fail "current-stage execute ran diagnostics after OCR throughput gate failure"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after OCR throughput gate failure"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after OCR throughput gate failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$ocr_gate_blocked_summary" >/dev/null
fi
require_text "$ocr_gate_blocked_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$ocr_gate_blocked_summary" '"blocked_step": "ocr_throughput_baseline_gate"'
require_text "$ocr_gate_blocked_summary" '"blocked_category": "ocr"'
require_text "$ocr_gate_blocked_summary" '"blocked_reason": "ocr_throughput_baseline_gate_failed"'
require_text "$ocr_gate_blocked_summary" '"private-ocr-throughput.json"'
require_text "$ocr_gate_blocked_summary" '"ocr-throughput-gate.stdout.txt"'
require_summary_observability "$ocr_gate_blocked_summary"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$ocr_gate_blocked_summary" "$tmpdir"
reject_text "$ocr_gate_blocked_summary" "PRIVATE-current-stage"
reject_text "$ocr_gate_blocked_summary" "private fake query"
require_text "$tmpdir/execute-ocr-gate-failed-stderr.txt" "current-stage validation blocked: OCR throughput baseline gate failed"
reject_text "$tmpdir/execute-ocr-gate-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-gate-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-gate-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-ocr-gate-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke query-set-prepare-failed
query_set_prepare_failed_status=$(cat "$tmpdir/execute-query-set-prepare-failed-status.txt")
if [ "$query_set_prepare_failed_status" -eq 0 ]; then
  fail "current-stage full profile accepted failed query-set prepare"
fi
query_set_blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$query_set_blocked_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on query-set prepare failure"
fi
if [ -e "$execute_out_dir/private-benchmark-local.json" ]; then
  fail "current-stage execute benchmarked private queries after query-set prepare failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$query_set_blocked_summary" >/dev/null
fi
require_text "$query_set_blocked_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$query_set_blocked_summary" '"blocked_step": "query_set_prepare"'
require_text "$query_set_blocked_summary" '"blocked_category": "query-set"'
require_text "$query_set_blocked_summary" '"blocked_reason": "query_set_corpus_or_trace_coverage_insufficient"'
require_text "$query_set_blocked_summary" '"query_set_trace_preflight"'
require_text "$query_set_blocked_summary" '"d10k_corpus_ready": false'
require_text "$query_set_blocked_summary" '"d10k_corpus_deficits"'
require_text "$query_set_blocked_summary" '"candidate_bucket_counts"'
require_text "$query_set_blocked_summary" '"corpus_valid_bucket_deficits"'
require_text "$query_set_blocked_summary" '"query-set-trace-preflight.local.json"'
require_text "$query_set_blocked_summary" '"query-set-prepare.stdout.txt"'
require_summary_observability "$query_set_blocked_summary"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
require_text "$execute_out_dir/current-stage-handoff.json" '"file": "query-set-trace-preflight.local.json"'
require_text "$execute_out_dir/current-stage-handoff.json" '"query_set_trace_preflight"'
require_text "$execute_out_dir/current-stage-handoff.json" '"d10k_corpus_ready": false'
require_text "$execute_out_dir/current-stage-handoff.json" '"d10k_corpus_deficits"'
require_text "$execute_out_dir/current-stage-handoff.json" '"candidate_bucket_deficits"'
require_text "$execute_out_dir/current-stage-handoff.json" '"corpus_valid_bucket_deficits"'
require_text "$execute_out_dir/current-stage-handoff.json" '"recommended_next_step": "prepare a D10K-shaped indexed local corpus and collect more trace-derived source_search workload for deficient buckets, then rerun current-stage validation with the static replay query-set freeze"'
require_text "$execute_out_dir/current-stage-issue-comment.md" "blocked_reason: query_set_corpus_or_trace_coverage_insufficient"
require_text "$execute_out_dir/current-stage-issue-comment.md" "d10k_corpus_ready: false"
require_text "$execute_out_dir/current-stage-issue-comment.md" "d10k_corpus_deficits: document_count=9997"
require_text "$execute_out_dir/current-stage-issue-comment.md" "candidate_bucket_deficits: and_2=73"
require_text "$execute_out_dir/current-stage-issue-comment.md" "corpus_valid_bucket_deficits: and_2=74"
require_text "$execute_out_dir/current-stage-issue-comment.md" "prepare a D10K-shaped indexed local corpus"
query_set_trace_preflight="$execute_out_dir/query-set-trace-preflight.local.json"
if [ ! -s "$query_set_trace_preflight" ]; then
  fail "current-stage execute did not write redacted query trace preflight before query-set prepare failure"
fi
python3 -m json.tool "$query_set_trace_preflight" >/dev/null
require_text "$query_set_trace_preflight" '"schema_version": "resume-ir.query-set-trace-preflight.v1"'
require_text "$query_set_trace_preflight" '"privacy_boundary": "redacted_local_aggregate"'
require_text "$query_set_trace_preflight" '"query_index_available": true'
require_text "$query_set_trace_preflight" '"d10k_corpus_ready": false'
require_text "$query_set_trace_preflight" '"d10k_corpus_deficits"'
require_text "$query_set_trace_preflight" '"candidate_bucket_counts"'
require_text "$query_set_trace_preflight" '"corpus_valid_bucket_deficits"'
reject_text "$query_set_blocked_summary" "$tmpdir"
reject_text "$query_set_blocked_summary" "PRIVATE-current-stage"
reject_text "$query_set_blocked_summary" "private fake query"
reject_text "$query_set_trace_preflight" "$tmpdir"
reject_text "$query_set_trace_preflight" "PRIVATE-current-stage"
reject_text "$query_set_trace_preflight" "private fake query"
require_text "$tmpdir/execute-query-set-prepare-failed-stderr.txt" "current-stage validation blocked: query-set corpus or trace coverage insufficient"
reject_text "$tmpdir/execute-query-set-prepare-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-prepare-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-prepare-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-query-set-prepare-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke query-set-d10k-corpus-not-ready
query_set_corpus_not_ready_status=$(cat "$tmpdir/execute-query-set-d10k-corpus-not-ready-status.txt")
if [ "$query_set_corpus_not_ready_status" -eq 0 ]; then
  fail "current-stage full profile accepted D10K freeze on non-D10K corpus"
fi
query_set_corpus_not_ready_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$query_set_corpus_not_ready_summary" ]; then
  fail "current-stage full profile did not write blocked summary on non-D10K corpus freeze"
fi
if [ -e "$execute_out_dir/private-benchmark-local.json" ]; then
  fail "current-stage execute benchmarked private queries after non-D10K corpus freeze"
fi
require_text "$query_set_corpus_not_ready_summary" '"blocked_step": "query_set_prepare"'
require_text "$query_set_corpus_not_ready_summary" '"blocked_category": "query-set"'
require_text "$query_set_corpus_not_ready_summary" '"blocked_reason": "query_set_corpus_or_trace_coverage_insufficient"'
require_text "$query_set_corpus_not_ready_summary" '"d10k_corpus_ready": false'
require_text "$query_set_corpus_not_ready_summary" '"d10k_corpus_deficits"'
require_text "$query_set_corpus_not_ready_summary" '"document_count": 1'
require_text "$query_set_corpus_not_ready_summary" '"corpus_valid_bucket_deficits"'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
require_text "$execute_out_dir/current-stage-handoff.json" '"d10k_corpus_ready": false'
require_text "$execute_out_dir/current-stage-issue-comment.md" "blocked_reason: query_set_corpus_or_trace_coverage_insufficient"
require_text "$execute_out_dir/current-stage-issue-comment.md" "d10k_corpus_deficits: document_count=1"
require_text "$execute_out_dir/current-stage-issue-comment.md" "prepare a D10K-shaped indexed local corpus"
require_text "$tmpdir/execute-query-set-d10k-corpus-not-ready-stderr.txt" "current-stage validation blocked: query-set corpus or trace coverage insufficient"
reject_text "$query_set_corpus_not_ready_summary" "$tmpdir"
reject_text "$query_set_corpus_not_ready_summary" "PRIVATE-current-stage"
reject_text "$query_set_corpus_not_ready_summary" "private fake query"
reject_text "$tmpdir/execute-query-set-d10k-corpus-not-ready-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-d10k-corpus-not-ready-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-d10k-corpus-not-ready-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-query-set-d10k-corpus-not-ready-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke query-set-index-unavailable
query_set_index_unavailable_status=$(cat "$tmpdir/execute-query-set-index-unavailable-status.txt")
if [ "$query_set_index_unavailable_status" -eq 0 ]; then
  fail "current-stage full profile accepted query-set freeze without a local search index"
fi
query_set_index_blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$query_set_index_blocked_summary" ]; then
  fail "current-stage full profile did not write blocked summary on query-set index unavailable"
fi
if [ -e "$execute_out_dir/private-benchmark-local.json" ]; then
  fail "current-stage execute benchmarked private queries after query-set index unavailable"
fi
python3 -m json.tool "$query_set_index_blocked_summary" >/dev/null
require_text "$query_set_index_blocked_summary" '"blocked_step": "query_set_prepare"'
require_text "$query_set_index_blocked_summary" '"blocked_category": "query-set"'
require_text "$query_set_index_blocked_summary" '"blocked_reason": "query_set_index_unavailable"'
require_text "$query_set_index_blocked_summary" '"query_set_trace_preflight"'
require_text "$query_set_index_blocked_summary" '"query_index_available": false'
require_text "$query_set_index_blocked_summary" '"candidate_bucket_counts"'
require_text "$query_set_index_blocked_summary" '"query-set-trace-preflight.local.json"'
require_text "$query_set_index_blocked_summary" '"query-set-prepare.stderr.txt"'
require_summary_observability "$query_set_index_blocked_summary"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
query_set_index_issue_comment="$execute_out_dir/current-stage-issue-comment.md"
require_text "$query_set_index_issue_comment" "#53 Current-Stage Blocked Handoff"
require_text "$query_set_index_issue_comment" "blocked_step: query_set_prepare"
require_text "$query_set_index_issue_comment" "blocked_reason: query_set_index_unavailable"
require_text "$query_set_index_issue_comment" "query_index_available: false"
require_text "$query_set_index_issue_comment" "not goal_complete; not a profile optimization issue closure"
reject_text "$query_set_index_issue_comment" "$tmpdir"
reject_text "$query_set_index_issue_comment" "PRIVATE-current-stage"
reject_text "$query_set_index_issue_comment" "private fake query"
query_set_index_trace_preflight="$execute_out_dir/query-set-trace-preflight.local.json"
if [ ! -s "$query_set_index_trace_preflight" ]; then
  fail "current-stage execute did not write redacted query trace preflight after index-unavailable freeze"
fi
python3 -m json.tool "$query_set_index_trace_preflight" >/dev/null
require_text "$query_set_index_trace_preflight" '"query_index_available": false'
reject_text "$query_set_index_trace_preflight" "$tmpdir"
reject_text "$query_set_index_trace_preflight" "PRIVATE-current-stage"
reject_text "$query_set_index_trace_preflight" "private fake query"
require_text "$tmpdir/execute-query-set-index-unavailable-stderr.txt" "current-stage validation blocked: query-set index unavailable"
reject_text "$query_set_index_blocked_summary" "$tmpdir"
reject_text "$query_set_index_blocked_summary" "PRIVATE-current-stage"
reject_text "$query_set_index_blocked_summary" "private fake query"
reject_text "$tmpdir/execute-query-set-index-unavailable-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-index-unavailable-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-index-unavailable-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-query-set-index-unavailable-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke private-query-failed
private_query_failed_status=$(cat "$tmpdir/execute-private-query-failed-status.txt")
if [ "$private_query_failed_status" -eq 0 ]; then
  fail "current-stage full profile accepted failed private query baseline"
fi
private_query_blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$private_query_blocked_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on private query baseline failure"
fi
if [ -e "$execute_out_dir/private-benchmark-gate.stdout.txt" ]; then
  fail "current-stage execute ran benchmark gate after private query baseline failure"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after private query baseline failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$private_query_blocked_summary" >/dev/null
fi
require_text "$private_query_blocked_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$private_query_blocked_summary" '"blocked_step": "private_query_baseline"'
require_text "$private_query_blocked_summary" '"blocked_category": "benchmark"'
require_text "$private_query_blocked_summary" '"blocked_reason": "private_query_baseline_failed"'
require_text "$private_query_blocked_summary" '"private-benchmark-local.json"'
require_summary_observability "$private_query_blocked_summary"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$private_query_blocked_summary" "$tmpdir"
reject_text "$private_query_blocked_summary" "PRIVATE-current-stage"
reject_text "$private_query_blocked_summary" "private fake query"
require_text "$tmpdir/execute-private-query-failed-stderr.txt" "current-stage validation blocked: private query baseline failed"
reject_text "$tmpdir/execute-private-query-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-private-query-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-private-query-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-private-query-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke private-query-invalid
private_query_invalid_status=$(cat "$tmpdir/execute-private-query-invalid-status.txt")
if [ "$private_query_invalid_status" -eq 0 ]; then
  fail "current-stage full profile accepted invalid private query benchmark report"
fi
private_query_invalid_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$private_query_invalid_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on invalid private query benchmark report"
fi
if [ -e "$execute_out_dir/private-benchmark-gate.stdout.txt" ]; then
  fail "current-stage execute ran benchmark gate after invalid private query benchmark report"
fi
if [ -e "$execute_out_dir/redacted-diagnostics.json" ]; then
  fail "current-stage execute ran diagnostics after invalid private query benchmark report"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after invalid private query benchmark report"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after invalid private query benchmark report"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$private_query_invalid_summary" >/dev/null
fi
require_text "$private_query_invalid_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$private_query_invalid_summary" '"blocked_step": "private_query_baseline"'
require_text "$private_query_invalid_summary" '"blocked_category": "benchmark"'
require_text "$private_query_invalid_summary" '"blocked_reason": "private_query_baseline_invalid"'
require_text "$private_query_invalid_summary" '"private-benchmark-local.json"'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$private_query_invalid_summary" "$tmpdir"
reject_text "$private_query_invalid_summary" "PRIVATE-current-stage"
reject_text "$private_query_invalid_summary" "private fake query"
require_text "$tmpdir/execute-private-query-invalid-stderr.txt" "current-stage validation blocked: private query baseline evidence failed validation"
reject_text "$tmpdir/execute-private-query-invalid-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-private-query-invalid-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-private-query-invalid-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-private-query-invalid-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke private-query-low-bucket
private_query_low_bucket_status=$(cat "$tmpdir/execute-private-query-low-bucket-status.txt")
if [ "$private_query_low_bucket_status" -eq 0 ]; then
  fail "current-stage full profile accepted private query samples below per-bucket floor"
fi
private_query_low_bucket_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$private_query_low_bucket_summary" ]; then
  fail "current-stage full profile did not write blocked summary for low-bucket private query benchmark report"
fi
if [ -e "$execute_out_dir/private-benchmark-gate.stdout.txt" ]; then
  fail "current-stage execute ran benchmark gate after low-bucket private query benchmark report"
fi
require_text "$private_query_low_bucket_summary" '"blocked_step": "private_query_baseline"'
require_text "$private_query_low_bucket_summary" '"blocked_reason": "private_query_baseline_invalid"'
require_text "$tmpdir/execute-private-query-low-bucket-stderr.txt" "current-stage validation blocked: private query baseline evidence failed validation"
reject_text "$private_query_low_bucket_summary" "$tmpdir"
reject_text "$private_query_low_bucket_summary" "PRIVATE-current-stage"
reject_text "$private_query_low_bucket_summary" "private fake query"

run_execute_smoke private-query-missing-scale-gate
private_query_missing_scale_gate_status=$(cat "$tmpdir/execute-private-query-missing-scale-gate-status.txt")
if [ "$private_query_missing_scale_gate_status" -eq 0 ]; then
  fail "current-stage full profile accepted private query report without D10K scale gate"
fi
private_query_missing_scale_gate_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$private_query_missing_scale_gate_summary" ]; then
  fail "current-stage full profile did not write blocked summary for missing D10K scale gate"
fi
if [ -e "$execute_out_dir/private-benchmark-gate.stdout.txt" ]; then
  fail "current-stage execute ran benchmark gate after missing D10K scale gate"
fi
require_text "$private_query_missing_scale_gate_summary" '"blocked_step": "private_query_baseline"'
require_text "$private_query_missing_scale_gate_summary" '"blocked_reason": "private_query_baseline_invalid"'
require_text "$tmpdir/execute-private-query-missing-scale-gate-stderr.txt" "current-stage validation blocked: private query baseline evidence failed validation"
reject_text "$private_query_missing_scale_gate_summary" "$tmpdir"
reject_text "$private_query_missing_scale_gate_summary" "PRIVATE-current-stage"
reject_text "$private_query_missing_scale_gate_summary" "private fake query"

run_execute_smoke private-query-sha-mismatch
private_query_sha_mismatch_status=$(cat "$tmpdir/execute-private-query-sha-mismatch-status.txt")
if [ "$private_query_sha_mismatch_status" -eq 0 ]; then
  fail "current-stage full profile accepted private query report with mismatched query_set_sha256"
fi
private_query_sha_mismatch_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$private_query_sha_mismatch_summary" ]; then
  fail "current-stage full profile did not write blocked summary for mismatched private query digest"
fi
if [ -e "$execute_out_dir/private-benchmark-gate.stdout.txt" ]; then
  fail "current-stage execute ran benchmark gate after mismatched private query digest"
fi
require_text "$private_query_sha_mismatch_summary" '"blocked_step": "private_query_baseline"'
require_text "$private_query_sha_mismatch_summary" '"blocked_reason": "private_query_baseline_query_set_mismatch"'
require_text "$tmpdir/execute-private-query-sha-mismatch-stderr.txt" "current-stage validation blocked: private query report query_set_sha256 mismatch"
reject_text "$private_query_sha_mismatch_summary" "$tmpdir"
reject_text "$private_query_sha_mismatch_summary" "PRIVATE-current-stage"
reject_text "$private_query_sha_mismatch_summary" "private fake query"

run_execute_smoke diagnostics-failed
diagnostics_failed_status=$(cat "$tmpdir/execute-diagnostics-failed-status.txt")
if [ "$diagnostics_failed_status" -eq 0 ]; then
  fail "current-stage full profile accepted failed redacted diagnostics"
fi
diagnostics_blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$diagnostics_blocked_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on diagnostics failure"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after diagnostics failure"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after diagnostics failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$diagnostics_blocked_summary" >/dev/null
fi
require_text "$diagnostics_blocked_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$diagnostics_blocked_summary" '"blocked_step": "redacted_diagnostics"'
require_text "$diagnostics_blocked_summary" '"blocked_category": "diagnostics"'
require_text "$diagnostics_blocked_summary" '"blocked_reason": "redacted_diagnostics_failed"'
require_text "$diagnostics_blocked_summary" '"redacted-diagnostics.json"'
require_text "$diagnostics_blocked_summary" '"private-benchmark-gate.stdout.txt"'
require_summary_observability "$diagnostics_blocked_summary"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$diagnostics_blocked_summary" "$tmpdir"
reject_text "$diagnostics_blocked_summary" "PRIVATE-current-stage"
reject_text "$diagnostics_blocked_summary" "private fake query"
require_text "$tmpdir/execute-diagnostics-failed-stderr.txt" "current-stage validation blocked: redacted diagnostics failed"
reject_text "$tmpdir/execute-diagnostics-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-diagnostics-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-diagnostics-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-diagnostics-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke diagnostics-invalid
diagnostics_invalid_status=$(cat "$tmpdir/execute-diagnostics-invalid-status.txt")
if [ "$diagnostics_invalid_status" -eq 0 ]; then
  fail "current-stage full profile accepted invalid redacted diagnostics schema"
fi
diagnostics_invalid_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$diagnostics_invalid_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on invalid diagnostics evidence"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after invalid diagnostics evidence"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after invalid diagnostics evidence"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$diagnostics_invalid_summary" >/dev/null
fi
require_text "$diagnostics_invalid_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$diagnostics_invalid_summary" '"blocked_step": "redacted_diagnostics"'
require_text "$diagnostics_invalid_summary" '"blocked_category": "diagnostics"'
require_text "$diagnostics_invalid_summary" '"blocked_reason": "redacted_diagnostics_invalid"'
require_text "$diagnostics_invalid_summary" '"redacted-diagnostics.json"'
require_text "$diagnostics_invalid_summary" '"private-benchmark-gate.stdout.txt"'
require_summary_observability "$diagnostics_invalid_summary"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$diagnostics_invalid_summary" "$tmpdir"
reject_text "$diagnostics_invalid_summary" "PRIVATE-current-stage"
reject_text "$diagnostics_invalid_summary" "private fake query"
require_text "$tmpdir/execute-diagnostics-invalid-stderr.txt" "current-stage validation blocked: redacted diagnostics evidence failed validation"
reject_text "$tmpdir/execute-diagnostics-invalid-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-diagnostics-invalid-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-diagnostics-invalid-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-diagnostics-invalid-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke fault-simulation-failed
fault_simulation_failed_status=$(cat "$tmpdir/execute-fault-simulation-failed-status.txt")
if [ "$fault_simulation_failed_status" -eq 0 ]; then
  fail "current-stage full profile accepted failed fault simulation smoke"
fi
fault_simulation_blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$fault_simulation_blocked_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on fault simulation failure"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after fault simulation failure"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after fault simulation failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$fault_simulation_blocked_summary" >/dev/null
fi
require_text "$fault_simulation_blocked_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$fault_simulation_blocked_summary" '"blocked_step": "fault_simulation_smoke"'
require_text "$fault_simulation_blocked_summary" '"blocked_category": "fault-injection"'
require_text "$fault_simulation_blocked_summary" '"blocked_reason": "fault_simulation_smoke_failed"'
require_text "$fault_simulation_blocked_summary" '"redacted-diagnostics.json"'
require_text "$fault_simulation_blocked_summary" '"fault-simulation-storage-low.json"'
require_summary_observability "$fault_simulation_blocked_summary"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$fault_simulation_blocked_summary" "$tmpdir"
reject_text "$fault_simulation_blocked_summary" "PRIVATE-current-stage"
reject_text "$fault_simulation_blocked_summary" "private fake query"
require_text "$tmpdir/execute-fault-simulation-failed-stderr.txt" "current-stage validation blocked: fault simulation smoke failed"
reject_text "$tmpdir/execute-fault-simulation-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-fault-simulation-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-fault-simulation-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-fault-simulation-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke fault-simulation-invalid
fault_simulation_invalid_status=$(cat "$tmpdir/execute-fault-simulation-invalid-status.txt")
if [ "$fault_simulation_invalid_status" -eq 0 ]; then
  fail "current-stage full profile accepted invalid fault simulation suite evidence"
fi
fault_simulation_invalid_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$fault_simulation_invalid_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on invalid fault simulation suite"
fi
if [ -e "$execute_out_dir/release-readiness.json" ]; then
  fail "current-stage execute ran release-readiness after invalid fault simulation suite"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after invalid fault simulation suite"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$fault_simulation_invalid_summary" >/dev/null
fi
require_text "$fault_simulation_invalid_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$fault_simulation_invalid_summary" '"blocked_step": "fault_simulation_suite"'
require_text "$fault_simulation_invalid_summary" '"blocked_category": "fault-injection"'
require_text "$fault_simulation_invalid_summary" '"blocked_reason": "fault_simulation_suite_invalid"'
require_text "$fault_simulation_invalid_summary" '"fault-simulation-suite-local-safe.json"'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$fault_simulation_invalid_summary" "$tmpdir"
reject_text "$fault_simulation_invalid_summary" "PRIVATE-current-stage"
reject_text "$fault_simulation_invalid_summary" "private fake query"
require_text "$tmpdir/execute-fault-simulation-invalid-stderr.txt" "current-stage validation blocked: fault simulation suite evidence failed validation"
reject_text "$tmpdir/execute-fault-simulation-invalid-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-fault-simulation-invalid-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-fault-simulation-invalid-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-fault-simulation-invalid-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke ocr-digest-mismatch \
  --ocr-runtime-manifest-sha256 0000000000000000000000000000000000000000000000000000000000000000
ocr_digest_mismatch_status=$(cat "$tmpdir/execute-ocr-digest-mismatch-status.txt")
if [ "$ocr_digest_mismatch_status" -eq 0 ]; then
  fail "current-stage execute accepted mismatched OCR runtime manifest digest"
fi
if [ -e "$execute_out_dir/dataset-manifest.local.json" ]; then
  fail "current-stage execute read private corpus before OCR runtime digest mismatch was rejected"
fi
require_text "$tmpdir/execute-ocr-digest-mismatch-stderr.txt" "OCR runtime manifest digest mismatch"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$tmpdir/execute-ocr-digest-mismatch-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-digest-mismatch-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-digest-mismatch-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-ocr-digest-mismatch-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke ocr-failed
ocr_failed_status=$(cat "$tmpdir/execute-ocr-failed-status.txt")
if [ "$ocr_failed_status" -eq 0 ]; then
  fail "current-stage execute accepted failed OCR runtime preflight"
fi
ocr_failed_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$ocr_failed_summary" ]; then
  fail "current-stage execute did not write pre-corpus blocked summary for OCR runtime preflight failure"
fi
if [ -e "$execute_out_dir/dataset-manifest.local.json" ]; then
  fail "current-stage execute read private corpus after OCR runtime preflight failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$ocr_failed_summary" >/dev/null
fi
require_text "$ocr_failed_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$ocr_failed_summary" '"blocked_step": "ocr_preflight"'
require_text "$ocr_failed_summary" '"blocked_category": "ocr"'
require_text "$ocr_failed_summary" '"blocked_reason": "ocr_runtime_preflight_failed"'
require_text "$ocr_failed_summary" '"private_corpus_read": false'
require_text "$ocr_failed_summary" '"ocr-preflight.json"'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$ocr_failed_summary" "$tmpdir"
reject_text "$ocr_failed_summary" "PRIVATE-current-stage"
require_text "$tmpdir/execute-ocr-failed-stderr.txt" "current-stage validation blocked: runtime preflight failed before reading private corpus"
reject_text "$tmpdir/execute-ocr-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-ocr-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke model-failed
model_failed_status=$(cat "$tmpdir/execute-model-failed-status.txt")
if [ "$model_failed_status" -eq 0 ]; then
  fail "current-stage execute accepted failed model preflight"
fi
model_failed_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$model_failed_summary" ]; then
  fail "current-stage execute did not write pre-corpus blocked summary for embedding runtime preflight failure"
fi
if [ -e "$execute_out_dir/dataset-manifest.local.json" ]; then
  fail "current-stage execute read private corpus before runtime preflight passed"
fi
if [ -e "$execute_out_dir/dataset-manifest.stdout.txt" ]; then
  fail "current-stage execute wrote dataset manifest stdout before runtime preflight passed"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$model_failed_summary" >/dev/null
fi
require_text "$model_failed_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$model_failed_summary" '"blocked_step": "model_preflight"'
require_text "$model_failed_summary" '"blocked_category": "embedding"'
require_text "$model_failed_summary" '"blocked_reason": "embedding_runtime_preflight_failed"'
require_text "$model_failed_summary" '"private_corpus_read": false'
require_text "$model_failed_summary" '"model-preflight.json"'
require_text "$model_failed_summary" '"ocr-runtime-manifest.local.json"'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$model_failed_summary" "$tmpdir"
reject_text "$model_failed_summary" "PRIVATE-current-stage"
require_text "$tmpdir/execute-model-failed-stderr.txt" "current-stage validation blocked: runtime preflight failed before reading private corpus"
reject_text "$tmpdir/execute-model-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-model-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-model-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-model-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke import-failed
import_failed_status=$(cat "$tmpdir/execute-import-failed-status.txt")
if [ "$import_failed_status" -eq 0 ]; then
  fail "current-stage execute accepted failed private corpus import"
fi
import_failed_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$import_failed_summary" ]; then
  fail "current-stage execute did not write redacted blocked summary on private corpus import failure"
fi
if [ -e "$execute_out_dir/ocr-worker.stdout.txt" ]; then
  fail "current-stage execute ran OCR worker after private corpus import failure"
fi
if [ -e "$execute_out_dir/private-query-set.local.jsonl" ]; then
  fail "current-stage execute drafted private query set after private corpus import failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$import_failed_summary" >/dev/null
fi
require_text "$import_failed_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$import_failed_summary" '"blocked_step": "import_private_corpus"'
require_text "$import_failed_summary" '"blocked_category": "import/parser"'
require_text "$import_failed_summary" '"blocked_reason": "import_private_corpus_failed"'
require_text "$import_failed_summary" '"private_corpus_read": true'
require_text "$import_failed_summary" '"dataset-manifest.local.json"'
require_text "$import_failed_summary" '"import.stdout.txt"'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$import_failed_summary" "$tmpdir"
reject_text "$import_failed_summary" "PRIVATE-current-stage"
reject_text "$import_failed_summary" "private fake query"
require_text "$tmpdir/execute-import-failed-stderr.txt" "current-stage validation blocked: import/parser failed"
reject_text "$tmpdir/execute-import-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-import-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-import-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-import-failed-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke query-set-summary-missing
query_set_summary_missing_status=$(cat "$tmpdir/execute-query-set-summary-missing-status.txt")
if [ "$query_set_summary_missing_status" -eq 0 ]; then
  fail "current-stage execute accepted private query-set without redacted summary"
fi
if [ -e "$execute_out_dir/private-benchmark-local.json" ]; then
  fail "current-stage execute benchmarked private queries before query-set summary was validated"
fi
require_text "$tmpdir/execute-query-set-summary-missing-stderr.txt" "query set summary must exist"
reject_text "$tmpdir/execute-query-set-summary-missing-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-summary-missing-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-summary-missing-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-query-set-summary-missing-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke query-set-local-field-source
query_set_local_field_source_status=$(cat "$tmpdir/execute-query-set-local-field-source-status.txt")
if [ "$query_set_local_field_source_status" -eq 0 ]; then
  fail "current-stage full profile accepted non-trace query-set source"
fi
query_set_local_field_source_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$query_set_local_field_source_summary" ]; then
  fail "current-stage full profile did not write blocked summary for non-trace query-set source"
fi
if [ -e "$execute_out_dir/private-benchmark-local.json" ]; then
  fail "current-stage execute benchmarked private queries after non-trace query-set source"
fi
require_text "$query_set_local_field_source_summary" '"blocked_step": "query_set_prepare"'
require_text "$query_set_local_field_source_summary" '"blocked_category": "query-set"'
require_text "$query_set_local_field_source_summary" '"blocked_reason": "query_set_source_invalid"'
require_text "$tmpdir/execute-query-set-local-field-source-stderr.txt" "current-stage validation blocked: query-set source invalid"
reject_text "$query_set_local_field_source_summary" "$tmpdir"
reject_text "$query_set_local_field_source_summary" "PRIVATE-current-stage"
reject_text "$query_set_local_field_source_summary" "private fake query"
reject_text "$tmpdir/execute-query-set-local-field-source-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-local-field-source-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-local-field-source-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-query-set-local-field-source-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke evidence-failed
failed_status=$(cat "$tmpdir/execute-evidence-failed-status.txt")
if [ "$failed_status" -eq 0 ]; then
  fail "current-stage execute accepted invalid release-readiness evidence"
fi
release_readiness_blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$release_readiness_blocked_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on release-readiness evidence failure"
fi
if [ -e "$execute_out_dir/current-stage-validation-evidence.json" ]; then
  fail "current-stage execute wrote full evidence after release-readiness evidence failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$release_readiness_blocked_summary" >/dev/null
fi
require_text "$release_readiness_blocked_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$release_readiness_blocked_summary" '"blocked_step": "release_readiness_intake"'
require_text "$release_readiness_blocked_summary" '"blocked_category": "release-readiness"'
require_text "$release_readiness_blocked_summary" '"blocked_reason": "release_readiness_evidence_failed_validation"'
require_text "$release_readiness_blocked_summary" '"release-readiness.json"'
require_text "$release_readiness_blocked_summary" '"release-readiness.stderr.txt"'
require_text "$release_readiness_blocked_summary" '"redacted-diagnostics.json"'
require_summary_observability "$release_readiness_blocked_summary"
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$release_readiness_blocked_summary" "$tmpdir"
reject_text "$release_readiness_blocked_summary" "PRIVATE-current-stage"
reject_text "$release_readiness_blocked_summary" "private fake query"
require_text "$tmpdir/execute-evidence-failed-stderr.txt" "current-stage validation blocked: release-readiness evidence failed validation"
reject_text "$tmpdir/execute-evidence-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-evidence-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-evidence-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-evidence-failed-stderr.txt" "PRIVATE-current-stage"

printf '%s\n' "current-stage validation check passed"

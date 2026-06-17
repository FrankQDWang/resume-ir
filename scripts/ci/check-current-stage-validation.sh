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
  require_text "$handoff" '"must_not_upload"'
  reject_text "$handoff" "$tmpdir"
  reject_text "$handoff" "PRIVATE-current-stage"
  reject_text "$handoff" "private fake query"
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
runbook="docs/runbooks/release-blockers.md"
worker_runbook="docs/runbooks/ocr-embedding-workers.md"
verify_script="scripts/ci/verify-local.sh"

require_file "$script"
require_file "$runbook"
require_file "$worker_runbook"
require_file "$verify_script"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-current-stage-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

plan="$tmpdir/current-stage-validation-plan.json"
resume_root="$tmpdir/PRIVATE-current-stage-resumes"
data_dir="$tmpdir/PRIVATE-current-stage-data"
out_dir="$tmpdir/PRIVATE-current-stage-evidence"
query_set="$tmpdir/PRIVATE-current-stage-query-set.jsonl"
embedding_runtime_bin_dir="$tmpdir/PRIVATE-current-stage-embedding-runtime-bin"
model_manifest="$tmpdir/PRIVATE-current-stage-model-manifest.json"
ocr_manifest="$tmpdir/PRIVATE-current-stage-ocr-manifest.json"
model_artifact="$tmpdir/PRIVATE-current-stage-model.onnx"
embedding_command="$tmpdir/PRIVATE-current-stage-embedding"
tesseract_command="$tmpdir/PRIVATE-current-stage-tesseract"
pdftoppm_command="$tmpdir/PRIVATE-current-stage-pdftoppm"
language_pack="$tmpdir/PRIVATE-current-stage-tessdata.traineddata"

mkdir -p "$resume_root" "$data_dir" "$out_dir" "$embedding_runtime_bin_dir"

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
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  --query-set-sha256 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
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
require_text "$plan" '"performance_optimization_deferred": true'
require_text "$plan" '"embedding_runtime_bin_dir_configured": true'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> privacy dataset-manifest --root <local-resume-root> --out <local-evidence-dir>/dataset-manifest.local.json --profile explicit --max-files 10000'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr preflight --json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr draft-manifest'
require_text "$plan" '[--language-pack <lang=path> ...]'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr validate-manifest --manifest <local-ocr-runtime-manifest>'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> model draft-manifest'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> model preflight --json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> model validate-manifest --manifest <local-model-manifest>'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> import --root <local-resume-root> --profile explicit --max-files 10000'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> benchmark-query-set draft --out <local-evidence-dir>/private-query-set.local.jsonl --max-queries 500 --min-queries 500'
require_text "$plan" 'resume-daemon --data-dir <local-data-dir> run --foreground --once --work-ocr-once'
require_text "$plan" 'resume-daemon --data-dir <local-data-dir> run --foreground --once --work-embeddings-once'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> benchmark-corpus-summary --json > <local-evidence-dir>/benchmark-corpus-summary.local.json'
require_text "$plan" 'resume-benchmark private-query'
require_text "$plan" '--command-arg benchmark-query-protocol'
require_text "$plan" '--max-queries 500 --top-k 10'
require_text "$plan" 'resume-benchmark gate --report <local-evidence-dir>/private-benchmark-local.json --require-private-real-corpus --min-documents 8000 --min-queries 500 --max-p95-ms 86400000'
require_text "$plan" 'resume-benchmark private-ocr-throughput'
require_text "$plan" 'resume-benchmark ocr-gate --report <local-evidence-dir>/private-ocr-throughput.json --current-stage-baseline --require-private-real-corpus --min-pages 500'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> export-diagnostics --redact > <local-evidence-dir>/redacted-diagnostics.json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> fault-simulate --case disk-space-low --scratch-dir <local-evidence-dir>/fault-simulation-scratch --required-bytes 4096 --available-bytes 1024 --json > <local-evidence-dir>/fault-simulation-storage-low.json'
require_text "$plan" 'fault-simulation.v1'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> fault-simulate --suite local-safe --scratch-dir <local-evidence-dir>/fault-simulation-suite-scratch --json > <local-evidence-dir>/fault-simulation-suite-local-safe.json'
require_text "$plan" 'fault-simulation-suite.v1'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> release-readiness --json'
require_text "$plan" '--benchmark-report <local-evidence-dir>/private-benchmark-local.json'
require_text "$plan" '--ocr-throughput-report <local-evidence-dir>/private-ocr-throughput.json'
require_text "$plan" '--model-manifest <local-model-manifest>'
require_text "$plan" '--ocr-runtime-manifest <local-ocr-runtime-manifest>'
require_text "$plan" '--diagnostics-report <local-evidence-dir>/redacted-diagnostics.json'
require_text "$plan" 'current-stage-handoff.json'
require_text "$plan" 'resume-ir.current-stage-handoff.v1'
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
require_text "$smoke_plan" 'benchmark-query-set draft --out <local-evidence-dir>/private-query-set.local.jsonl --max-queries 3 --min-queries 1 --allow-keyword-fallback'
require_text "$smoke_plan" '--corpus-summary <local-evidence-dir>/benchmark-corpus-summary.local.json --allow-partial-hot-index-for-smoke --max-queries 3 --top-k 5'
require_text "$smoke_plan" 'resume-benchmark gate --report <local-evidence-dir>/private-benchmark-local.json --require-private-real-corpus --allow-smoke-confidence --min-documents 1 --min-queries 1'
require_text "$smoke_plan" 'resume-cli --data-dir <local-data-dir> fault-simulate --case disk-space-low --scratch-dir <local-evidence-dir>/fault-simulation-scratch --required-bytes 4096 --available-bytes 1024 --json > <local-evidence-dir>/fault-simulation-storage-low.json'
require_text "$smoke_plan" 'resume-cli --data-dir <local-data-dir> fault-simulate --suite local-safe --scratch-dir <local-evidence-dir>/fault-simulation-suite-scratch --json > <local-evidence-dir>/fault-simulation-suite-local-safe.json'
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
execute_model_manifest="$tmpdir/PRIVATE-current-stage-execute-model-manifest.json"
execute_ocr_manifest="$tmpdir/PRIVATE-current-stage-execute-ocr-manifest.json"
execute_model_artifact="$tmpdir/PRIVATE-current-stage-execute-model.onnx"
execute_embedding_command="$tmpdir/PRIVATE-current-stage-execute-embedding"
execute_tesseract_command="$tmpdir/PRIVATE-current-stage-execute-tesseract"
execute_pdftoppm_command="$tmpdir/PRIVATE-current-stage-execute-pdftoppm"
execute_language_pack="$tmpdir/PRIVATE-current-stage-execute-tessdata.traineddata"

mkdir -p "$execute_resume_root" "$execute_data_dir" "$execute_out_dir"
printf '%s\n' '{"query":"private fake query"}' > "$execute_query_set"
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
write_out_arg() {
  out=""
  while [ "$#" -gt 0 ]; do
    if [ "$1" = "--out" ]; then
      shift
      out="${1:-}"
      break
    fi
    shift
  done
  [ -n "$out" ] || exit 64
  printf '{"schema_version":"fake-local-manifest.v1"}\n' > "$out"
}
case "$cmd:$sub" in
  privacy:dataset-manifest)
    write_out_arg "$@"
    printf 'dataset manifest: written\n'
    printf 'schema: resume-ir.dataset-manifest.v1\n'
    printf 'privacy boundary: local_only_redacted_dataset_manifest\n'
    ;;
  benchmark-query-set:draft)
    if [ "${FAKE_QUERY_SET_MODE:-ready}" = "draft-failed" ]; then
      printf 'query set blocked: insufficient field-backed queries\n'
      exit 1
    fi
    write_out_arg "$@"
    printf 'query set: written\n'
    printf 'schema: resume-ir.query-set.jsonl.v1\n'
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
    if [ "${FAKE_IMPORT_MODE:-ready}" = "failed" ]; then
      printf 'import blocked: fake parser failure\n' >&2
      exit 1
    fi
    printf 'import task submitted\nstatus: completed\n'
    ;;
  benchmark-corpus-summary:*)
    if [ "${FAKE_CORPUS_SUMMARY_MODE:-hot}" = "ocr-backlog" ]; then
      printf '{"schema_version":"benchmark-corpus-summary.v1","privacy_boundary":"redacted_local_aggregate","document_count":8720,"searchable_document_count":162,"vector_indexed_document_count":0,"hot_index_fully_covered":false,"document_status_counts":{"failed_permanent":20,"ocr_required":8538,"searchable":162},"ingest_job_status_counts":{"completed":16,"queued":8537,"running":1},"ingest_job_kind_status_counts":{"ocr_document":{"completed":16,"queued":8537,"running":1}},"ingest_job_failure_counts":{},"contains_raw_resume_text":false,"contains_resume_paths":false,"contains_queries":false,"contains_sample_ids":false}\n'
      exit 0
    fi
    printf '{"schema_version":"benchmark-corpus-summary.v1","privacy_boundary":"redacted_local_aggregate","document_count":8720,"searchable_document_count":8720,"vector_indexed_document_count":8720,"hot_index_fully_covered":true,"document_status_counts":{"searchable":8720},"ingest_job_status_counts":{"completed":8720},"ingest_job_kind_status_counts":{"update_index":{"completed":8720}},"ingest_job_failure_counts":{},"contains_raw_resume_text":false,"contains_resume_paths":false,"contains_queries":false,"contains_sample_ids":false}\n'
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
    printf '{"schema_version":"diagnostics.v1","redacted":true,"raw_paths":"<redacted>","raw_queries":"<redacted>","raw_resume_text":"<redacted>","metadata":{"indexed_documents":8720,"searchable_documents":8720},"search_index_state":"available","vector_index_state":"available","query_latency":{"sample_count":500,"raw_queries":"<redacted>"},"resource_telemetry":{"status":"available","paths":"<redacted>"},"ocr_runtime":{"paths":"<redacted>","pdftoppm":"available","tesseract":"available","requested_language":"eng","requested_language_status":"available"},"diagnostic_scope":{"metadata":"aggregate_counts","search_index":"state_and_snapshot_health","vector_index":"state_backend_and_counts","query_latency":"aggregate_observations","runtime_dependencies":"presence_only","fault_simulations":"available_cases_only"},"evidence_level":"local_aggregate_only"}\n'
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
        printf '{"schema_version":"fault-simulation-suite.v1","redacted":true,"suite":"local_safe","paths":"<redacted>","evidence_level":"local_synthetic_fault_suite","release_hardware_drills":"blocked","summary":{"total_cases":10,"failed_cases":0,"release_blockers_cleared":false},"cases":[]}\n'
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
case "${1:-}" in
  private-query)
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
    printf '{"schema_version":"benchmark.private-query.v1","dataset_kind":"private-real-corpus","target_claim":"benchmark_baseline_observed"}\n'
    ;;
  gate)
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "gate-failed" ]; then
      printf 'benchmark gate blocked: private real-corpus hot-index coverage floor not met\n' >&2
      exit 1
    fi
    printf 'benchmark gate passed\n'
    ;;
  private-ocr-throughput)
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "private-ocr-throughput-failed" ]; then
      printf 'private OCR throughput baseline blocked: fake OCR runtime failure\n' >&2
      exit 1
    fi
    printf '{"schema_version":"ocr-throughput.v1","dataset_kind":"private-real-corpus","target_claim":"ocr_throughput_baseline_observed","corpus_origin":"private_local","privacy_boundary":"redacted_local_aggregate","contains_raw_ocr_text":false,"contains_page_images":false,"contains_resume_paths":false,"contains_document_ids":false,"contains_page_ids":false,"contains_command_paths":false,"document_count":8720,"scanned_document_count":500,"page_count":500,"failed_document_count":0,"render_failure_count":0,"ocr_failure_count":0,"total_ms":1000,"pages_per_second":500.0,"run_budget_exhausted":false,"page_latency_ms":{"samples":500}}\n'
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
  if [ "$mode" = "private-ocr-throughput-failed" ]; then
    benchmark_mode="private-ocr-throughput-failed"
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
  import_mode="ready"
  if [ "$mode" = "import-failed" ]; then
    import_mode="failed"
  fi
  query_set_mode="ready"
  if [ "$mode" = "query-set-draft-failed" ]; then
    query_set_mode="draft-failed"
  fi
  corpus_summary_mode="hot"
  if [ "$mode" = "ocr-backlog" ]; then
    corpus_summary_mode="ocr-backlog"
  fi
  FAKE_BENCHMARK_MODE="$benchmark_mode" FAKE_CORPUS_SUMMARY_MODE="$corpus_summary_mode" FAKE_DIAGNOSTICS_MODE="$diagnostics_mode" FAKE_FAULT_SIMULATION_MODE="$fault_simulation_mode" FAKE_IMPORT_MODE="$import_mode" FAKE_QUERY_SET_MODE="$query_set_mode" FAKE_RELEASE_READINESS_MODE="$mode" FAKE_REQUIRED_EMBEDDING_RUNTIME_BIN_DIR="$embedding_runtime_bin_dir" FAKE_RUNTIME_PREFLIGHT_MODE="$mode" "$script" --execute \
    --resume-cli "$fake_resume_cli" \
    --resume-daemon "$fake_resume_daemon" \
    --resume-benchmark "$fake_resume_benchmark" \
    --resume-root "$execute_resume_root" \
    --data-dir "$execute_data_dir" \
    --out-dir "$execute_out_dir" \
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
require_text "$evidence_manifest" '"performance_optimization_deferred": true'
require_text "$evidence_manifest" '"release_readiness_exit": 1'
require_text "$evidence_manifest" '"stable_release_expected_blocked": true'
expected_dataset_sha256=$(sha256_file "$execute_out_dir/dataset-manifest.local.json")
require_text "$evidence_manifest" "\"dataset_manifest_sha256\": \"$expected_dataset_sha256\""
expected_query_set_sha256=$(sha256_file "$execute_out_dir/private-query-set.local.jsonl")
require_text "$evidence_manifest" "\"query_set_sha256\": \"$expected_query_set_sha256\""
expected_model_manifest_sha256=$(sha256_file "$execute_model_manifest")
require_text "$evidence_manifest" "\"model_manifest_sha256\": \"$expected_model_manifest_sha256\""
expected_ocr_manifest_sha256=$(sha256_file "$execute_ocr_manifest")
require_text "$evidence_manifest" "\"ocr_runtime_manifest_sha256\": \"$expected_ocr_manifest_sha256\""
require_text "$evidence_manifest" '"preflight_probes": {'
require_text "$evidence_manifest" '"ocr_runtime_probe": "passed"'
require_text "$evidence_manifest" '"embedding_protocol": "passed"'
require_text "$evidence_manifest" '"dataset-manifest.local.json"'
require_text "$evidence_manifest" '"dataset-manifest.stdout.txt"'
require_text "$evidence_manifest" '"model-manifest.local.json"'
require_text "$evidence_manifest" '"ocr-runtime-manifest.local.json"'
require_text "$evidence_manifest" '"private-query-set.local.jsonl"'
require_text "$evidence_manifest" '"query-set-draft.stdout.txt"'
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
require_text "$evidence_manifest" '"release-readiness.json"'
require_text "$evidence_manifest" '"local_paths_included": false'
require_text "$evidence_manifest" '"raw_resume_text_included": false'
require_text "$evidence_manifest" '"raw_query_text_included": false'
require_text "$evidence_manifest" '"model_bytes_included": false'
require_text "$evidence_manifest" '"runtime_binaries_included": false'
require_current_stage_handoff \
  "full_evidence_ready" \
  "resume-ir.current-stage-validation-evidence.v1"
reject_text "$evidence_manifest" "$tmpdir"
reject_text "$evidence_manifest" "PRIVATE-current-stage"
reject_text "$evidence_manifest" "private fake query"
require_text "$tmpdir/execute-blocked-stdout.txt" "current-stage validation: release-readiness exit 1"
require_text "$tmpdir/execute-blocked-stdout.txt" "current-stage validation: local evidence written under <local-evidence-dir>"
reject_text "$tmpdir/execute-blocked-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-blocked-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-blocked-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-blocked-stderr.txt" "PRIVATE-current-stage"

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
require_text "$smoke_summary" '"smoke_satisfied": true'
require_text "$smoke_summary" '"full_baseline_satisfied": false'
require_text "$smoke_summary" '"release_readiness_evidence": false'
require_text "$smoke_summary" '"ocr_runtime_probe": "passed"'
require_text "$smoke_summary" '"embedding_protocol": "passed"'
require_text "$smoke_summary" '"corpus_summary_observability": {'
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

run_execute_smoke ocr-backlog
ocr_backlog_status=$(cat "$tmpdir/execute-ocr-backlog-status.txt")
if [ "$ocr_backlog_status" -eq 0 ]; then
  fail "current-stage full profile accepted bounded OCR backlog as full evidence"
fi
ocr_backlog_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$ocr_backlog_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary for bounded OCR backlog"
fi
if [ -e "$execute_out_dir/query-set-draft.stdout.txt" ]; then
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
require_text "$ocr_backlog_summary" '"corpus_summary_observability": {'
require_text "$ocr_backlog_summary" '"ocr_required": 8538'
require_text "$ocr_backlog_summary" '"vector_indexed_document_count": 0'
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
require_text "$blocked_summary" '"corpus_summary_observability": {'
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
require_text "$private_ocr_blocked_summary" '"corpus_summary_observability": {'
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
require_text "$ocr_gate_blocked_summary" '"corpus_summary_observability": {'
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

run_execute_smoke query-set-draft-failed
query_set_draft_failed_status=$(cat "$tmpdir/execute-query-set-draft-failed-status.txt")
if [ "$query_set_draft_failed_status" -eq 0 ]; then
  fail "current-stage full profile accepted failed query-set draft"
fi
query_set_blocked_summary="$execute_out_dir/current-stage-blocked-summary.json"
if [ ! -s "$query_set_blocked_summary" ]; then
  fail "current-stage full profile did not write redacted blocked summary on query-set draft failure"
fi
if [ -e "$execute_out_dir/private-benchmark-local.json" ]; then
  fail "current-stage execute benchmarked private queries after query-set draft failure"
fi
if command -v python3 >/dev/null 2>&1; then
  python3 -m json.tool "$query_set_blocked_summary" >/dev/null
fi
require_text "$query_set_blocked_summary" '"schema_version": "resume-ir.current-stage-blocked-summary.v1"'
require_text "$query_set_blocked_summary" '"blocked_step": "query_set_draft"'
require_text "$query_set_blocked_summary" '"blocked_category": "query-set"'
require_text "$query_set_blocked_summary" '"blocked_reason": "query_set_draft_failed"'
require_text "$query_set_blocked_summary" '"query-set-draft.stdout.txt"'
require_text "$query_set_blocked_summary" '"corpus_summary_observability": {'
require_current_stage_handoff \
  "blocked" \
  "resume-ir.current-stage-blocked-summary.v1"
reject_text "$query_set_blocked_summary" "$tmpdir"
reject_text "$query_set_blocked_summary" "PRIVATE-current-stage"
reject_text "$query_set_blocked_summary" "private fake query"
require_text "$tmpdir/execute-query-set-draft-failed-stderr.txt" "current-stage validation blocked: query-set draft failed"
reject_text "$tmpdir/execute-query-set-draft-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-draft-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-set-draft-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-query-set-draft-failed-stderr.txt" "PRIVATE-current-stage"

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
require_text "$private_query_blocked_summary" '"corpus_summary_observability": {'
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
require_text "$diagnostics_blocked_summary" '"corpus_summary_observability": {'
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
require_text "$diagnostics_invalid_summary" '"corpus_summary_observability": {'
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
require_text "$fault_simulation_blocked_summary" '"corpus_summary_observability": {'
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

run_execute_smoke query-digest-mismatch \
  --query-set-sha256 0000000000000000000000000000000000000000000000000000000000000000
query_digest_mismatch_status=$(cat "$tmpdir/execute-query-digest-mismatch-status.txt")
if [ "$query_digest_mismatch_status" -eq 0 ]; then
  fail "current-stage execute accepted mismatched private query-set digest"
fi
if [ -e "$execute_out_dir/private-benchmark-local.json" ]; then
  fail "current-stage execute benchmarked private queries before query-set digest mismatch was rejected"
fi
require_text "$tmpdir/execute-query-digest-mismatch-stderr.txt" "query set digest mismatch"
reject_text "$tmpdir/execute-query-digest-mismatch-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-digest-mismatch-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-query-digest-mismatch-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-query-digest-mismatch-stderr.txt" "PRIVATE-current-stage"

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
require_text "$release_readiness_blocked_summary" '"corpus_summary_observability": {'
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

require_text "$script" "--execute"
require_text "$script" "--embedding-runtime-bin-dir"
require_text "$script" "resume-ir.current-stage-validation-plan.v1"
require_text "$script" "resume-ir.current-stage-validation-evidence.v1"
require_text "$script" "resume-ir.dataset-manifest.v1"
require_text "$script" "resume-ir.query-set.jsonl.v1"
require_text "$script" "local_only_redacted_plan"
require_text "$script" "local_only_redacted_evidence_manifest"
require_text "$script" "local_only_redacted_dataset_manifest"
require_text "$script" "local_only_private_query_set"
require_text "$script" "runtime preflight failed before reading private corpus"
require_text "$script" "preflight_probes"
require_text "$script" '"runtime_probe": "passed"'
require_text "$script" '"embedding_protocol": "passed"'
require_text "$script" "current-stage-handoff.json"
require_text "$script" "resume-ir.current-stage-handoff.v1"
require_text "$script" "performance_optimization_deferred"
require_text "$script" "ocr_backlog_exceeds_current_stage_budget"
require_text "$runbook" "scripts/local/run-current-stage-validation.sh --dry-run"
require_text "$runbook" "scripts/local/run-current-stage-validation.sh --execute"
require_text "$runbook" "--embedding-runtime-bin-dir <local-runtime-bin-dir>"
require_text "$runbook" "resume-ir.current-stage-validation-plan.v1"
require_text "$runbook" "resume-ir.current-stage-validation-evidence.v1"
require_text "$runbook" "resume-ir.current-stage-blocked-summary.v1"
require_text "$runbook" "resume-ir.dataset-manifest.v1"
require_text "$runbook" "resume-ir.query-set.jsonl.v1"
require_text "$runbook" "local_only_redacted_plan"
require_text "$runbook" "local_only_redacted_evidence_manifest"
require_text "$runbook" "local_only_redacted_dataset_manifest"
require_text "$runbook" "local_only_private_query_set"
require_text "$runbook" "before reading the"
require_text "$runbook" "private resume root"
require_text "$runbook" "stops before scanning the private corpus"
require_text "$runbook" "privacy dataset-manifest"
require_text "$runbook" "benchmark-query-set draft"
require_text "$runbook" "baseline shape gate fails"
require_text "$runbook" 'blocked_reason: "ocr_backlog_exceeds_current_stage_budget"'
require_text "$runbook" "current-stage-blocked-summary.json"
require_text "$runbook" "--current-stage-evidence current-stage-validation-evidence.json"
require_text "$runbook" "--max-p95-ms 86400000"
require_text "$runbook" "performance_optimization_deferred"
require_text "$worker_runbook" "run-current-stage-validation.sh"
require_text "$verify_script" "./scripts/ci/check-current-stage-validation.sh"

printf '%s\n' "current-stage validation check passed"

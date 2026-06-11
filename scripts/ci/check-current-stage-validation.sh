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
model_manifest="$tmpdir/PRIVATE-current-stage-model-manifest.json"
ocr_manifest="$tmpdir/PRIVATE-current-stage-ocr-manifest.json"
model_artifact="$tmpdir/PRIVATE-current-stage-model.onnx"
embedding_command="$tmpdir/PRIVATE-current-stage-embedding"
tesseract_command="$tmpdir/PRIVATE-current-stage-tesseract"
pdftoppm_command="$tmpdir/PRIVATE-current-stage-pdftoppm"
language_pack="$tmpdir/PRIVATE-current-stage-tessdata.traineddata"

mkdir -p "$resume_root" "$data_dir" "$out_dir"

"$script" --dry-run \
  --resume-root "$resume_root" \
  --data-dir "$data_dir" \
  --out-dir "$out_dir" \
  --query-set "$query_set" \
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
fi

require_text "$plan" '"schema_version": "resume-ir.current-stage-validation-plan.v1"'
require_text "$plan" '"mode": "dry-run"'
require_text "$plan" '"privacy_boundary": "local_only_redacted_plan"'
require_text "$plan" '"resume_root": "<local-resume-root>"'
require_text "$plan" '"data_dir": "<local-data-dir>"'
require_text "$plan" '"out_dir": "<local-evidence-dir>"'
require_text "$plan" '"current_stage_target": "reproducible_local_10k_baseline"'
require_text "$plan" '"performance_optimization_deferred": true'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> privacy dataset-manifest --root <local-resume-root> --out <local-evidence-dir>/dataset-manifest.local.json --profile explicit --max-files 10000'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr preflight --json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr draft-manifest'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr validate-manifest --manifest <local-ocr-runtime-manifest>'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> model draft-manifest'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> model preflight --json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> model validate-manifest --manifest <local-model-manifest>'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> import --root <local-resume-root> --profile explicit --max-files 10000'
require_text "$plan" 'resume-daemon --data-dir <local-data-dir> run --foreground --once --work-ocr-once'
require_text "$plan" 'resume-daemon --data-dir <local-data-dir> run --foreground --once --work-embeddings-once'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> benchmark-corpus-summary --json > <local-evidence-dir>/benchmark-corpus-summary.local.json'
require_text "$plan" 'resume-benchmark private-query'
require_text "$plan" '--command-arg benchmark-query-protocol'
require_text "$plan" '--max-queries 500 --top-k 10'
require_text "$plan" 'resume-benchmark gate --report <local-evidence-dir>/private-benchmark-local.json --require-private-real-corpus --min-documents 8000 --min-queries 500 --max-p95-ms 86400000'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> export-diagnostics --redact > <local-evidence-dir>/redacted-diagnostics.json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> release-readiness --json'
require_text "$plan" '--benchmark-report <local-evidence-dir>/private-benchmark-local.json'
require_text "$plan" '--model-manifest <local-model-manifest>'
require_text "$plan" '--ocr-runtime-manifest <local-ocr-runtime-manifest>'
require_text "$plan" '--diagnostics-report <local-evidence-dir>/redacted-diagnostics.json'
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
reject_text "$plan" "$embedding_command"
reject_text "$plan" "$tesseract_command"
reject_text "$plan" "$pdftoppm_command"
reject_text "$plan" "$language_pack"
reject_text "$plan" "/Users/"

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
  ocr:preflight)
    printf '{"schema_version":"ocr-runtime-preflight.v1","ready":true}\n'
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
    printf '{"schema_version":"embedding-runtime-preflight.v1","ready":true}\n'
    ;;
  import:*)
    printf 'import task submitted\nstatus: completed\n'
    ;;
  benchmark-corpus-summary:*)
    printf '{"schema_version":"benchmark-corpus-summary.v1","document_count":8720,"searchable_document_count":8720,"vector_indexed_document_count":8720,"hot_index_fully_covered":true}\n'
    ;;
  export-diagnostics:*)
    printf '{"schema_version":"diagnostics.v1","redacted":true,"evidence_level":"local_aggregate_only"}\n'
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
printf 'fake worker completed\n'
SH
chmod 700 "$fake_resume_daemon"

cat > "$fake_resume_benchmark" <<'SH'
#!/usr/bin/env sh
set -eu
case "${1:-}" in
  private-query)
    printf '{"schema_version":"benchmark.private-query.v1","dataset_kind":"private-real-corpus","target_claim":"benchmark_baseline_observed"}\n'
    ;;
  gate)
    printf 'benchmark gate passed\n'
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
  stdout_file="$tmpdir/execute-$mode-stdout.txt"
  stderr_file="$tmpdir/execute-$mode-stderr.txt"
  rm -rf "$execute_data_dir" "$execute_out_dir"
  mkdir -p "$execute_data_dir" "$execute_out_dir"
  set +e
  FAKE_RELEASE_READINESS_MODE="$mode" "$script" --execute \
    --resume-cli "$fake_resume_cli" \
    --resume-daemon "$fake_resume_daemon" \
    --resume-benchmark "$fake_resume_benchmark" \
    --resume-root "$execute_resume_root" \
    --data-dir "$execute_data_dir" \
    --out-dir "$execute_out_dir" \
    --query-set "$execute_query_set" \
    --model-manifest "$execute_model_manifest" \
    --ocr-runtime-manifest "$execute_ocr_manifest" \
    --model-artifact "$execute_model_artifact" \
    --embedding-command "$execute_embedding_command" \
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
    --query-set-sha256 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \
    --model-manifest-sha256 cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc \
    --max-files 10000 \
    --max-queries 500 \
    --top-k 10 \
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
require_text "$evidence_manifest" '"query_set_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"'
require_text "$evidence_manifest" '"model_manifest_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"'
require_text "$evidence_manifest" '"dataset-manifest.local.json"'
require_text "$evidence_manifest" '"dataset-manifest.stdout.txt"'
require_text "$evidence_manifest" '"benchmark-corpus-summary.local.json"'
require_text "$evidence_manifest" '"private-benchmark-local.json"'
require_text "$evidence_manifest" '"redacted-diagnostics.json"'
require_text "$evidence_manifest" '"release-readiness.json"'
require_text "$evidence_manifest" '"local_paths_included": false'
require_text "$evidence_manifest" '"raw_resume_text_included": false'
require_text "$evidence_manifest" '"raw_query_text_included": false'
require_text "$evidence_manifest" '"model_bytes_included": false'
require_text "$evidence_manifest" '"runtime_binaries_included": false'
reject_text "$evidence_manifest" "$tmpdir"
reject_text "$evidence_manifest" "PRIVATE-current-stage"
reject_text "$evidence_manifest" "private fake query"
require_text "$tmpdir/execute-blocked-stdout.txt" "current-stage validation: release-readiness exit 1"
require_text "$tmpdir/execute-blocked-stdout.txt" "current-stage validation: local evidence written under <local-evidence-dir>"
reject_text "$tmpdir/execute-blocked-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-blocked-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-blocked-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-blocked-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke evidence-failed
failed_status=$(cat "$tmpdir/execute-evidence-failed-status.txt")
if [ "$failed_status" -eq 0 ]; then
  fail "current-stage execute accepted invalid release-readiness evidence"
fi
require_text "$tmpdir/execute-evidence-failed-stderr.txt" "current-stage validation blocked: release-readiness evidence failed validation"
reject_text "$tmpdir/execute-evidence-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-evidence-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-evidence-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-evidence-failed-stderr.txt" "PRIVATE-current-stage"

require_text "$script" "--execute"
require_text "$script" "resume-ir.current-stage-validation-plan.v1"
require_text "$script" "resume-ir.current-stage-validation-evidence.v1"
require_text "$script" "resume-ir.dataset-manifest.v1"
require_text "$script" "local_only_redacted_plan"
require_text "$script" "local_only_redacted_evidence_manifest"
require_text "$script" "local_only_redacted_dataset_manifest"
require_text "$script" "performance_optimization_deferred"
require_text "$runbook" "scripts/local/run-current-stage-validation.sh --dry-run"
require_text "$runbook" "scripts/local/run-current-stage-validation.sh --execute"
require_text "$runbook" "resume-ir.current-stage-validation-plan.v1"
require_text "$runbook" "resume-ir.current-stage-validation-evidence.v1"
require_text "$runbook" "resume-ir.dataset-manifest.v1"
require_text "$runbook" "local_only_redacted_plan"
require_text "$runbook" "local_only_redacted_evidence_manifest"
require_text "$runbook" "local_only_redacted_dataset_manifest"
require_text "$runbook" "privacy dataset-manifest"
require_text "$runbook" "--current-stage-evidence current-stage-validation-evidence.json"
require_text "$runbook" "--max-p95-ms 86400000"
require_text "$runbook" "performance_optimization_deferred"
require_text "$worker_runbook" "run-current-stage-validation.sh"
require_text "$verify_script" "./scripts/ci/check-current-stage-validation.sh"

printf '%s\n' "current-stage validation check passed"

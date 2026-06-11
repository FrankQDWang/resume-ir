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
  --dataset-manifest-sha256 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
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

require_text "$script" "--execute"
require_text "$script" "resume-ir.current-stage-validation-plan.v1"
require_text "$script" "local_only_redacted_plan"
require_text "$script" "performance_optimization_deferred"
require_text "$runbook" "scripts/local/run-current-stage-validation.sh --dry-run"
require_text "$runbook" "scripts/local/run-current-stage-validation.sh --execute"
require_text "$runbook" "resume-ir.current-stage-validation-plan.v1"
require_text "$runbook" "local_only_redacted_plan"
require_text "$runbook" "--max-p95-ms 86400000"
require_text "$runbook" "performance_optimization_deferred"
require_text "$worker_runbook" "run-current-stage-validation.sh"
require_text "$verify_script" "./scripts/ci/check-current-stage-validation.sh"

printf '%s\n' "current-stage validation check passed"

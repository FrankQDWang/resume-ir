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
require_text "$plan" 'resume-cli --data-dir <local-data-dir> privacy dataset-manifest --root <local-resume-root> --out <local-evidence-dir>/dataset-manifest.local.json --profile explicit --max-files 10000'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr preflight --json'
require_text "$plan" 'resume-cli --data-dir <local-data-dir> ocr draft-manifest'
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
require_text "$smoke_plan" 'resume-benchmark gate --report <local-evidence-dir>/private-benchmark-local.json --require-private-real-corpus --allow-smoke-confidence --min-documents 1 --min-queries 1'
require_text "$smoke_plan" 'write <local-evidence-dir>/current-stage-smoke-summary.json'
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
    write_out_arg "$@"
    printf 'query set: written\n'
    printf 'schema: resume-ir.query-set.jsonl.v1\n'
    printf 'privacy boundary: local_only_private_query_set\n'
    ;;
  ocr:preflight)
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
    if [ "${FAKE_RUNTIME_PREFLIGHT_MODE:-ready}" = "model-failed" ]; then
      printf 'fake model preflight failed\n' >&2
      exit 1
    fi
    printf '{"schema_version":"embedding-runtime-preflight.v1","embedding_protocol": "passed","ready":true}\n'
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
    if [ "${FAKE_BENCHMARK_MODE:-pass}" = "gate-failed" ]; then
      printf 'benchmark gate blocked: private real-corpus hot-index coverage floor not met\n' >&2
      exit 1
    fi
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
  FAKE_BENCHMARK_MODE="$benchmark_mode" FAKE_RELEASE_READINESS_MODE="$mode" FAKE_RUNTIME_PREFLIGHT_MODE="$mode" "$script" --execute \
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
require_text "$smoke_summary" '"current_stage_target": "local_real_corpus_smoke_chain"'
require_text "$smoke_summary" '"full_baseline_satisfied": false'
require_text "$smoke_summary" '"release_readiness_evidence": false'
require_text "$smoke_summary" '"ocr_runtime_probe": "passed"'
require_text "$smoke_summary" '"embedding_protocol": "passed"'
require_text "$smoke_summary" '"private_query_baseline"'
require_text "$smoke_summary" '"redacted_diagnostics"'
require_text "$smoke_summary" '"full 10k/8000-document current-stage baseline"'
reject_text "$smoke_summary" "$tmpdir"
reject_text "$smoke_summary" "PRIVATE-current-stage"
reject_text "$smoke_summary" "private fake query"
require_text "$tmpdir/execute-smoke-profile-stdout.txt" "current-stage validation: smoke summary written under <local-evidence-dir>"
reject_text "$tmpdir/execute-smoke-profile-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-smoke-profile-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-smoke-profile-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-smoke-profile-stderr.txt" "PRIVATE-current-stage"

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
require_text "$blocked_summary" '"private-benchmark-local.json"'
require_text "$blocked_summary" '"private-benchmark-gate.stdout.txt"'
require_text "$blocked_summary" '"full 10k/8000-document current-stage baseline"'
reject_text "$blocked_summary" "$tmpdir"
reject_text "$blocked_summary" "PRIVATE-current-stage"
reject_text "$blocked_summary" "private fake query"
require_text "$tmpdir/execute-benchmark-gate-failed-stderr.txt" "current-stage validation blocked: baseline shape gate failed"
reject_text "$tmpdir/execute-benchmark-gate-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-benchmark-gate-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-benchmark-gate-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-benchmark-gate-failed-stderr.txt" "PRIVATE-current-stage"

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
reject_text "$tmpdir/execute-ocr-digest-mismatch-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-digest-mismatch-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-ocr-digest-mismatch-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-ocr-digest-mismatch-stderr.txt" "PRIVATE-current-stage"

run_execute_smoke model-failed
model_failed_status=$(cat "$tmpdir/execute-model-failed-status.txt")
if [ "$model_failed_status" -eq 0 ]; then
  fail "current-stage execute accepted failed model preflight"
fi
if [ -e "$execute_out_dir/dataset-manifest.local.json" ]; then
  fail "current-stage execute read private corpus before runtime preflight passed"
fi
if [ -e "$execute_out_dir/dataset-manifest.stdout.txt" ]; then
  fail "current-stage execute wrote dataset manifest stdout before runtime preflight passed"
fi
require_text "$tmpdir/execute-model-failed-stderr.txt" "current-stage validation blocked: runtime preflight failed before reading private corpus"
reject_text "$tmpdir/execute-model-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-model-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-model-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-model-failed-stderr.txt" "PRIVATE-current-stage"

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
require_text "$tmpdir/execute-evidence-failed-stderr.txt" "current-stage validation blocked: release-readiness evidence failed validation"
reject_text "$tmpdir/execute-evidence-failed-stdout.txt" "$tmpdir"
reject_text "$tmpdir/execute-evidence-failed-stderr.txt" "$tmpdir"
reject_text "$tmpdir/execute-evidence-failed-stdout.txt" "PRIVATE-current-stage"
reject_text "$tmpdir/execute-evidence-failed-stderr.txt" "PRIVATE-current-stage"

require_text "$script" "--execute"
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
require_text "$script" "performance_optimization_deferred"
require_text "$runbook" "scripts/local/run-current-stage-validation.sh --dry-run"
require_text "$runbook" "scripts/local/run-current-stage-validation.sh --execute"
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
require_text "$runbook" "current-stage-blocked-summary.json"
require_text "$runbook" "--current-stage-evidence current-stage-validation-evidence.json"
require_text "$runbook" "--max-p95-ms 86400000"
require_text "$runbook" "performance_optimization_deferred"
require_text "$worker_runbook" "run-current-stage-validation.sh"
require_text "$verify_script" "./scripts/ci/check-current-stage-validation.sh"

printf '%s\n' "current-stage validation check passed"

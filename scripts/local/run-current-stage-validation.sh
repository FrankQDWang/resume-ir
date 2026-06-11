#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage: scripts/local/run-current-stage-validation.sh [--dry-run|--execute]
  --resume-root DIR --data-dir DIR --out-dir DIR --query-set FILE
  --model-manifest FILE --ocr-runtime-manifest FILE
  --model-artifact FILE --embedding-command FILE
  --model-pack-id ID --model-id ID --model-format ID --dimension N --model-license ID
  --runtime-pack-id ID --tesseract-command FILE --pdftoppm-command FILE
  --language LANG --language-pack FILE
  --engine-license ID --renderer-license ID --language-license ID
  --dataset-manifest-sha256 SHA256 [--query-set-sha256 SHA256]
  [--model-manifest-sha256 SHA256]
  [--ocr-runtime-manifest-sha256 SHA256]
  [--renderer-manifest-sha256 SHA256]
  [--language-pack-manifest-sha256 SHA256]
  [--resume-cli PATH] [--resume-daemon PATH] [--resume-benchmark PATH]
  [--reviewed-model] [--reviewed-ocr-runtime]
  [--max-files N] [--max-queries N] [--top-k N]
  [--worker-interval-ms N] [--ocr-worker-ticks N] [--embedding-worker-ticks N]

Default mode is --dry-run. Dry-run prints a redacted JSON plan and never reads
the private resume root. Execute mode runs local-only commands and writes local
evidence under --out-dir.
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

mode="dry-run"
resume_cli="${RESUME_CLI:-resume-cli}"
resume_daemon="${RESUME_DAEMON:-resume-daemon}"
resume_benchmark="${RESUME_BENCHMARK:-resume-benchmark}"
resume_root=""
data_dir=""
out_dir=""
query_set=""
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
language_pack=""
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
      need_value "$@"; language_pack="$2"; shift 2
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
require_arg "--query-set" "$query_set"
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
require_arg "--language-pack" "$language_pack"
require_arg "--engine-license" "$engine_license"
require_arg "--renderer-license" "$renderer_license"
require_arg "--language-license" "$language_license"
require_arg "--dataset-manifest-sha256" "$dataset_manifest_sha256"

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
require_sha256 "--dataset-manifest-sha256" "$dataset_manifest_sha256"
[ -z "$query_set_sha256" ] || require_sha256 "--query-set-sha256" "$query_set_sha256"
[ -z "$model_manifest_sha256" ] || require_sha256 "--model-manifest-sha256" "$model_manifest_sha256"
[ -z "$ocr_runtime_manifest_sha256" ] || require_sha256 "--ocr-runtime-manifest-sha256" "$ocr_runtime_manifest_sha256"
[ -z "$renderer_manifest_sha256" ] || require_sha256 "--renderer-manifest-sha256" "$renderer_manifest_sha256"
[ -z "$language_pack_manifest_sha256" ] || require_sha256 "--language-pack-manifest-sha256" "$language_pack_manifest_sha256"

if [ "$mode" = "dry-run" ]; then
  cat <<EOF
{
  "schema_version": "resume-ir.current-stage-validation-plan.v1",
  "mode": "dry-run",
  "privacy_boundary": "local_only_redacted_plan",
  "resume_root": "<local-resume-root>",
  "data_dir": "<local-data-dir>",
  "out_dir": "<local-evidence-dir>",
  "current_stage_target": "reproducible_local_10k_baseline",
  "performance_optimization_deferred": true,
  "actual_execution_requires": "operator_local_execute_mode",
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "embedding_dimension": $dimension,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "embedding_worker_ticks": $embedding_worker_ticks
  },
  "ordered_steps": [
    {
      "id": "ocr_preflight",
      "command": "resume-cli --data-dir <local-data-dir> ocr preflight --json --ocr-lang <ocr-language> --tesseract-command <local-tesseract-command> --pdftoppm-command <local-pdftoppm-command>"
    },
    {
      "id": "ocr_manifest_draft",
      "command": "resume-cli --data-dir <local-data-dir> ocr draft-manifest --out <local-ocr-runtime-manifest> --runtime-pack-id <reviewed-runtime-pack-id> --tesseract-command <local-tesseract-command> --pdftoppm-command <local-pdftoppm-command> --language <ocr-language> --language-pack <local-language-pack> --engine-license <engine-license> --renderer-license <renderer-license> --language-license <language-license> [--reviewed]"
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
      "id": "private_query_baseline",
      "command": "resume-benchmark private-query --query-set <local-query-set> --command resume-cli --command-arg --data-dir --command-arg <local-data-dir> --command-arg benchmark-query-protocol --command-arg --embedding-command --command-arg <local-embedding-command> --command-arg --model-id --command-arg <reviewed-local-model-id> --command-arg --dimension --command-arg <dimension> --corpus-summary <local-evidence-dir>/benchmark-corpus-summary.local.json --max-queries $max_queries --top-k $top_k --dataset-manifest-sha256 <dataset-manifest-sha256> --query-set-sha256 <query-set-sha256> --model-manifest-sha256 <model-manifest-sha256> --json > <local-evidence-dir>/private-benchmark-local.json"
    },
    {
      "id": "baseline_shape_gate",
      "command": "resume-benchmark gate --report <local-evidence-dir>/private-benchmark-local.json --require-private-real-corpus --min-documents 8000 --min-queries 500 --max-p95-ms 86400000 --max-zero-result-queries 500"
    },
    {
      "id": "redacted_diagnostics",
      "command": "resume-cli --data-dir <local-data-dir> export-diagnostics --redact > <local-evidence-dir>/redacted-diagnostics.json"
    },
    {
      "id": "release_readiness_intake",
      "command": "resume-cli --data-dir <local-data-dir> release-readiness --json --benchmark-report <local-evidence-dir>/private-benchmark-local.json --model-manifest <local-model-manifest> --ocr-runtime-manifest <local-ocr-runtime-manifest> --diagnostics-report <local-evidence-dir>/redacted-diagnostics.json > <local-evidence-dir>/release-readiness.json"
    }
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
    "Execute mode keeps all evidence local under <local-evidence-dir>.",
    "The baseline shape gate deliberately uses --max-p95-ms 86400000; P95/P99 reduction is deferred.",
    "Release-readiness is expected to remain blocked while signing, notarization, platform installer, and other private quality evidence are missing."
  ]
}
EOF
  exit 0
fi

[ "$mode" = "execute" ] || usage
[ -d "$resume_root" ] || fail "resume root must exist and be a directory"
[ -f "$query_set" ] || fail "query set must exist and stay local"
mkdir -p "$data_dir" "$out_dir"

ocr_reviewed_arg=""
if [ "$reviewed_ocr_runtime" = "true" ]; then
  ocr_reviewed_arg="--reviewed"
fi
model_reviewed_arg=""
if [ "$reviewed_model" = "true" ]; then
  model_reviewed_arg="--reviewed"
fi

printf '%s\n' "current-stage validation: ocr preflight"
"$resume_cli" --data-dir "$data_dir" ocr preflight --json \
  --ocr-lang "$language" \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  > "$out_dir/ocr-preflight.json"

printf '%s\n' "current-stage validation: ocr manifest draft"
"$resume_cli" --data-dir "$data_dir" ocr draft-manifest \
  --out "$ocr_runtime_manifest" \
  --runtime-pack-id "$runtime_pack_id" \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language "$language" \
  --language-pack "$language_pack" \
  --engine-license "$engine_license" \
  --renderer-license "$renderer_license" \
  --language-license "$language_license" \
  $ocr_reviewed_arg \
  > "$out_dir/ocr-draft-manifest.stdout.txt"

printf '%s\n' "current-stage validation: ocr manifest validate"
"$resume_cli" --data-dir "$data_dir" ocr validate-manifest \
  --manifest "$ocr_runtime_manifest" \
  > "$out_dir/ocr-validate-manifest.stdout.txt"

printf '%s\n' "current-stage validation: model manifest draft"
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

printf '%s\n' "current-stage validation: model manifest validate"
"$resume_cli" --data-dir "$data_dir" model validate-manifest \
  --manifest "$model_manifest" \
  > "$out_dir/model-validate-manifest.stdout.txt"

printf '%s\n' "current-stage validation: model preflight"
"$resume_cli" --data-dir "$data_dir" model preflight --json \
  --manifest "$model_manifest" \
  --embedding-command "$embedding_command" \
  --model-id "$model_id" \
  --dimension "$dimension" \
  > "$out_dir/model-preflight.json"

if [ -z "$query_set_sha256" ]; then
  query_set_sha256=$(sha256_file "$query_set")
fi
if [ -z "$model_manifest_sha256" ]; then
  model_manifest_sha256=$(sha256_file "$model_manifest")
fi

printf '%s\n' "current-stage validation: import private corpus"
"$resume_cli" --data-dir "$data_dir" import \
  --root "$resume_root" \
  --profile explicit \
  --max-files "$max_files" \
  > "$out_dir/import.stdout.txt"

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

printf '%s\n' "current-stage validation: private query baseline"
"$resume_benchmark" private-query \
  --query-set "$query_set" \
  --command "$resume_cli" \
  --command-arg --data-dir --command-arg "$data_dir" \
  --command-arg benchmark-query-protocol \
  --command-arg --embedding-command --command-arg "$embedding_command" \
  --command-arg --model-id --command-arg "$model_id" \
  --command-arg --dimension --command-arg "$dimension" \
  --corpus-summary "$out_dir/benchmark-corpus-summary.local.json" \
  --max-queries "$max_queries" \
  --top-k "$top_k" \
  --dataset-manifest-sha256 "$dataset_manifest_sha256" \
  --query-set-sha256 "$query_set_sha256" \
  --model-manifest-sha256 "$model_manifest_sha256" \
  --json \
  > "$out_dir/private-benchmark-local.json"

printf '%s\n' "current-stage validation: baseline shape gate"
"$resume_benchmark" gate \
  --report "$out_dir/private-benchmark-local.json" \
  --require-private-real-corpus \
  --min-documents 8000 \
  --min-queries 500 \
  --max-p95-ms 86400000 \
  --max-zero-result-queries 500 \
  > "$out_dir/private-benchmark-gate.stdout.txt"

printf '%s\n' "current-stage validation: redacted diagnostics"
"$resume_cli" --data-dir "$data_dir" export-diagnostics --redact \
  > "$out_dir/redacted-diagnostics.json"

printf '%s\n' "current-stage validation: release-readiness intake"
set +e
"$resume_cli" --data-dir "$data_dir" release-readiness --json \
  --benchmark-report "$out_dir/private-benchmark-local.json" \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_runtime_manifest" \
  --diagnostics-report "$out_dir/redacted-diagnostics.json" \
  > "$out_dir/release-readiness.json" \
  2> "$out_dir/release-readiness.stderr.txt"
release_status=$?
set -e
if [ "$release_status" -ne 0 ]; then
  if grep -Fq "release readiness evidence failed validation" "$out_dir/release-readiness.stderr.txt"; then
    printf '%s\n' \
      "current-stage validation blocked: release-readiness evidence failed validation" >&2
    exit 1
  fi
  if ! grep -Fq "release readiness blocked: stable release criteria are not met" "$out_dir/release-readiness.stderr.txt"; then
    printf '%s\n' \
      "current-stage validation blocked: release-readiness returned an unexpected error" >&2
    exit 1
  fi
fi
printf 'current-stage validation: release-readiness exit %s\n' "$release_status"
printf '%s\n' "current-stage validation: local evidence written under <local-evidence-dir>"

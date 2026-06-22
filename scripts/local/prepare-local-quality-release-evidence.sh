#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage: scripts/local/prepare-local-quality-release-evidence.sh --reviewed
  --out-dir DIR
  --field-dataset FILE --field-dataset-manifest-sha256 SHA256 --field-annotation-manifest-sha256 SHA256
  --dedupe-dataset FILE --dedupe-dataset-manifest-sha256 SHA256 --dedupe-annotation-manifest-sha256 SHA256
  --vector-dataset FILE --vector-dataset-manifest-sha256 SHA256 --vector-annotation-manifest-sha256 SHA256
  --embedding-command PATH --model-id ID --dimension N --model-manifest-sha256 SHA256
  [--resume-benchmark PATH] [--resume-cli PATH --data-dir DIR]

Creates local aggregate field/dedupe/vector quality reports from reviewed
private business labeled datasets, runs release quality gates, and optionally
passes those aggregate reports to release-readiness. The command never uploads
private datasets or reports and prints only redacted status output.
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
  [ -n "$value" ] || fail "local quality evidence blocked: missing $name"
}

require_file_arg() {
  name="$1"
  value="$2"
  require_arg "$name" "$value"
  [ -f "$value" ] || fail "local quality evidence blocked: $name is unavailable"
}

require_sha256() {
  name="$1"
  value="$2"
  require_arg "$name" "$value"
  case "$value" in
    *[!0123456789abcdefABCDEF]*)
      fail "local quality evidence blocked: invalid $name"
      ;;
  esac
  [ "${#value}" -eq 64 ] || fail "local quality evidence blocked: invalid $name"
}

require_positive_int() {
  name="$1"
  value="$2"
  require_arg "$name" "$value"
  case "$value" in
    *[!0-9]*)
      fail "local quality evidence blocked: invalid $name"
      ;;
    0)
      fail "local quality evidence blocked: invalid $name"
      ;;
  esac
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "local quality evidence blocked: release-readiness did not accept quality evidence"
  fi
}

resolve_command() {
  configured="$1"
  label="$2"
  case "$configured" in
    */*)
      [ -x "$configured" ] || fail "local quality evidence blocked: $label is unavailable"
      printf '%s' "$configured"
      ;;
    *)
      resolved="$(command -v "$configured" 2>/dev/null || true)"
      [ -n "$resolved" ] || fail "local quality evidence blocked: $label is unavailable"
      printf '%s' "$resolved"
      ;;
  esac
}

run_step() {
  label="$1"
  shift
  stdout_file="$tmpdir/$label.stdout"
  stderr_file="$tmpdir/$label.stderr"
  set +e
  "$@" > "$stdout_file" 2> "$stderr_file"
  status=$?
  set -e
  if [ "$status" -ne 0 ]; then
    fail "local quality evidence blocked: $label failed"
  fi
}

field_dataset=""
field_dataset_sha=""
field_annotation_sha=""
dedupe_dataset=""
dedupe_dataset_sha=""
dedupe_annotation_sha=""
vector_dataset=""
vector_dataset_sha=""
vector_annotation_sha=""
embedding_command=""
model_id=""
dimension=""
model_manifest_sha=""
out_dir=""
resume_benchmark="resume-benchmark"
resume_cli=""
data_dir=""
reviewed=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --out-dir)
      need_value "$@"; out_dir="$2"; shift 2
      ;;
    --field-dataset)
      need_value "$@"; field_dataset="$2"; shift 2
      ;;
    --field-dataset-manifest-sha256)
      need_value "$@"; field_dataset_sha="$2"; shift 2
      ;;
    --field-annotation-manifest-sha256)
      need_value "$@"; field_annotation_sha="$2"; shift 2
      ;;
    --dedupe-dataset)
      need_value "$@"; dedupe_dataset="$2"; shift 2
      ;;
    --dedupe-dataset-manifest-sha256)
      need_value "$@"; dedupe_dataset_sha="$2"; shift 2
      ;;
    --dedupe-annotation-manifest-sha256)
      need_value "$@"; dedupe_annotation_sha="$2"; shift 2
      ;;
    --vector-dataset)
      need_value "$@"; vector_dataset="$2"; shift 2
      ;;
    --vector-dataset-manifest-sha256)
      need_value "$@"; vector_dataset_sha="$2"; shift 2
      ;;
    --vector-annotation-manifest-sha256)
      need_value "$@"; vector_annotation_sha="$2"; shift 2
      ;;
    --embedding-command)
      need_value "$@"; embedding_command="$2"; shift 2
      ;;
    --model-id)
      need_value "$@"; model_id="$2"; shift 2
      ;;
    --dimension)
      need_value "$@"; dimension="$2"; shift 2
      ;;
    --model-manifest-sha256)
      need_value "$@"; model_manifest_sha="$2"; shift 2
      ;;
    --resume-benchmark)
      need_value "$@"; resume_benchmark="$2"; shift 2
      ;;
    --resume-cli)
      need_value "$@"; resume_cli="$2"; shift 2
      ;;
    --data-dir)
      need_value "$@"; data_dir="$2"; shift 2
      ;;
    --reviewed)
      reviewed=1; shift
      ;;
    -h|--help)
      usage
      ;;
    *)
      usage
      ;;
  esac
done

require_arg "--out-dir" "$out_dir"
require_file_arg "--field-dataset" "$field_dataset"
require_file_arg "--dedupe-dataset" "$dedupe_dataset"
require_file_arg "--vector-dataset" "$vector_dataset"
require_file_arg "--embedding-command" "$embedding_command"
require_arg "--model-id" "$model_id"
require_positive_int "--dimension" "$dimension"
require_sha256 "--field-dataset-manifest-sha256" "$field_dataset_sha"
require_sha256 "--field-annotation-manifest-sha256" "$field_annotation_sha"
require_sha256 "--dedupe-dataset-manifest-sha256" "$dedupe_dataset_sha"
require_sha256 "--dedupe-annotation-manifest-sha256" "$dedupe_annotation_sha"
require_sha256 "--vector-dataset-manifest-sha256" "$vector_dataset_sha"
require_sha256 "--vector-annotation-manifest-sha256" "$vector_annotation_sha"
require_sha256 "--model-manifest-sha256" "$model_manifest_sha"

case "$model_id" in
  *[!A-Za-z0-9._~:/-]*|'')
    fail "local quality evidence blocked: invalid --model-id"
    ;;
esac

if [ "$reviewed" -ne 1 ]; then
  fail "local quality evidence blocked: quality review is incomplete"
fi

if { [ -n "$resume_cli" ] && [ -z "$data_dir" ]; } || { [ -z "$resume_cli" ] && [ -n "$data_dir" ]; }; then
  fail "local quality evidence blocked: --resume-cli and --data-dir must be supplied together"
fi

benchmark_cmd="$(resolve_command "$resume_benchmark" "resume-benchmark")"
embedding_cmd="$(resolve_command "$embedding_command" "embedding command")"
release_cli_cmd=""
if [ -n "$resume_cli" ]; then
  release_cli_cmd="$(resolve_command "$resume_cli" "resume-cli")"
fi

mkdir -p "$out_dir" || fail "local quality evidence blocked: output directory is unavailable"
tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-quality-evidence.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

field_report="$out_dir/private-field-quality.json"
dedupe_report="$out_dir/private-dedupe-quality.json"
vector_report="$out_dir/private-vector-quality.json"
release_report="$out_dir/release-readiness-quality.json"
release_stderr="$out_dir/release-readiness-quality.stderr.txt"

run_step field-quality "$benchmark_cmd" field-quality \
  --dataset "$field_dataset" \
  --private-business-labeled \
  --dataset-manifest-sha256 "$field_dataset_sha" \
  --annotation-manifest-sha256 "$field_annotation_sha" \
  --json
mv "$tmpdir/field-quality.stdout" "$field_report"

run_step field-gate "$benchmark_cmd" field-gate \
  --report "$field_report" \
  --require-private-business-labeled \
  --min-samples 1000 \
  --min-precision 0.93 \
  --min-recall 0.93 \
  --min-f1 0.93

run_step dedupe-quality "$benchmark_cmd" dedupe-quality \
  --dataset "$dedupe_dataset" \
  --private-business-labeled \
  --dataset-manifest-sha256 "$dedupe_dataset_sha" \
  --annotation-manifest-sha256 "$dedupe_annotation_sha" \
  --json
mv "$tmpdir/dedupe-quality.stdout" "$dedupe_report"

run_step dedupe-gate "$benchmark_cmd" dedupe-gate \
  --report "$dedupe_report" \
  --require-private-business-labeled \
  --min-pairs 1000 \
  --min-positive-pairs 100 \
  --min-precision 0.90 \
  --min-recall 0.90 \
  --min-f1 0.90

run_step vector-quality "$benchmark_cmd" vector-quality \
  --dataset "$vector_dataset" \
  --command "$embedding_cmd" \
  --model-id "$model_id" \
  --dimension "$dimension" \
  --private-business-labeled \
  --dataset-manifest-sha256 "$vector_dataset_sha" \
  --annotation-manifest-sha256 "$vector_annotation_sha" \
  --model-manifest-sha256 "$model_manifest_sha" \
  --top-k 10 \
  --json
mv "$tmpdir/vector-quality.stdout" "$vector_report"

run_step vector-gate "$benchmark_cmd" vector-gate \
  --report "$vector_report" \
  --require-private-business-labeled \
  --min-samples 1000 \
  --min-recall-at-k 0.90 \
  --min-mrr 0.85 \
  --min-ndcg-at-k 0.90 \
  --max-zero-recall-queries 0

release_readiness_status="skipped"
if [ -n "$release_cli_cmd" ]; then
  set +e
  "$release_cli_cmd" --data-dir "$data_dir" release-readiness --json \
    --field-quality-report "$field_report" \
    --dedupe-quality-report "$dedupe_report" \
    --vector-quality-report "$vector_report" \
    > "$release_report" 2> "$release_stderr"
  release_status=$?
  set -e
  if [ "$release_status" -eq 0 ]; then
    fail "local quality evidence blocked: release-readiness unexpectedly passed stable release"
  fi
  if grep -Fq "release readiness evidence failed validation" "$release_stderr"; then
    fail "local quality evidence blocked: release-readiness rejected quality evidence"
  fi
  require_text "$release_report" '"label": "field extraction quality"'
  require_text "$release_report" '"label": "dedupe quality"'
  require_text "$release_report" '"label": "vector quality"'
  require_text "$release_report" '"status": "provided"'
  release_readiness_status="quality evidence accepted; stable release still blocked"
fi

printf '%s\n' "local quality release evidence: written"
printf '%s\n' "field report: private-field-quality.json"
printf '%s\n' "dedupe report: private-dedupe-quality.json"
printf '%s\n' "vector report: private-vector-quality.json"
printf 'release readiness: %s\n' "$release_readiness_status"
printf '%s\n' "paths: <redacted>"

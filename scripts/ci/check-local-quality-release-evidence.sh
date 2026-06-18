#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "local quality release-evidence check missing expected file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  require_file "$file"
  if ! grep -Fq -- "$text" "$file"; then
    fail "local quality release-evidence check missing expected text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  label="$3"
  require_file "$file"
  if [ -n "$text" ] && grep -Fq -- "$text" "$file"; then
    fail "local quality release-evidence check leaked $label"
  fi
}

reject_regex() {
  file="$1"
  pattern="$2"
  label="$3"
  require_file "$file"
  if grep -Eq -- "$pattern" "$file"; then
    fail "local quality release-evidence check leaked $label"
  fi
}

quality_script="scripts/local/prepare-local-quality-release-evidence.sh"
if [ ! -f "$quality_script" ]; then
  fail "missing local quality release evidence preparation script"
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-local-quality-evidence.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

field_dataset="$tmpdir/PRIVATE-field-quality.jsonl"
dedupe_dataset="$tmpdir/PRIVATE-dedupe-quality.jsonl"
vector_dataset="$tmpdir/PRIVATE-vector-quality.jsonl"
embedding_command="$tmpdir/PRIVATE-embedding-command"
data_dir="$tmpdir/PRIVATE-quality-data"
out_dir="$tmpdir/PRIVATE-quality-out"
mkdir -p "$data_dir" "$out_dir"
printf '%s\n' '{"private":"field labels stay local"}' > "$field_dataset"
printf '%s\n' '{"private":"dedupe labels stay local"}' > "$dedupe_dataset"
printf '%s\n' '{"private":"vector labels stay local"}' > "$vector_dataset"
cat > "$embedding_command" <<'SH'
#!/usr/bin/env sh
printf '%s\n' "resume-ir-embedding-v1"
printf '%s\n' "model_id=reviewed-local-model"
printf '%s\n' "dimension=3"
printf '%s\n' "vector=doc-1	1,0,0"
SH
chmod 700 "$embedding_command"

fake_resume_benchmark="$tmpdir/fake-resume-benchmark"
fake_resume_benchmark_args="$tmpdir/fake-resume-benchmark-args.txt"
cat > "$fake_resume_benchmark" <<'SH'
#!/usr/bin/env sh
set -eu
printf '%s\n' "$*" >> "$FAKE_RESUME_BENCHMARK_ARGS"
case "${1:-}" in
  field-quality)
    printf '%s\n' '{"schema_version":"field-quality.v1","dataset_kind":"private-business-labeled","target_claim":"field_quality_target_met"}'
    ;;
  field-gate)
    printf '%s\n' "field gate passed"
    ;;
  dedupe-quality)
    printf '%s\n' '{"schema_version":"dedupe-quality.v1","dataset_kind":"private-business-labeled","target_claim":"dedupe_quality_target_met"}'
    ;;
  dedupe-gate)
    printf '%s\n' "dedupe gate passed"
    ;;
  vector-quality)
    printf '%s\n' '{"schema_version":"vector-quality.v1","dataset_kind":"private-business-labeled","target_claim":"vector_quality_target_met"}'
    ;;
  vector-gate)
    printf '%s\n' "vector gate passed"
    ;;
  *)
    printf 'unexpected fake resume-benchmark command: %s\n' "${1:-}" >&2
    exit 64
    ;;
esac
SH
chmod 700 "$fake_resume_benchmark"

fake_resume_cli="$tmpdir/fake-resume-cli"
fake_resume_cli_args="$tmpdir/fake-resume-cli-args.txt"
cat > "$fake_resume_cli" <<'SH'
#!/usr/bin/env sh
set -eu
printf '%s\n' "$*" >> "$FAKE_RESUME_CLI_ARGS"
cat <<'JSON'
{
  "schema_version": "release-readiness.v1",
  "stable_release": "blocked",
  "provided_evidence": [
    {
      "label": "field extraction quality",
      "status": "provided"
    },
    {
      "label": "dedupe quality",
      "status": "provided"
    },
    {
      "label": "vector quality",
      "status": "provided"
    }
  ],
  "blockers": [
    {
      "label": "private real-corpus performance evidence",
      "status": "blocked"
    }
  ]
}
JSON
printf '%s\n' "resume-cli: release readiness blocked: stable release criteria are not met" >&2
exit 1
SH
chmod 700 "$fake_resume_cli"

sha_a="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
sha_b="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
sha_c="cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
sha_d="dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
sha_e="eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
sha_f="ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
sha_1="1111111111111111111111111111111111111111111111111111111111111111"

run_common_args() {
  set -- \
    --out-dir "$out_dir" \
    --field-dataset "$field_dataset" \
    --field-dataset-manifest-sha256 "$sha_a" \
    --field-annotation-manifest-sha256 "$sha_b" \
    --dedupe-dataset "$dedupe_dataset" \
    --dedupe-dataset-manifest-sha256 "$sha_c" \
    --dedupe-annotation-manifest-sha256 "$sha_d" \
    --vector-dataset "$vector_dataset" \
    --vector-dataset-manifest-sha256 "$sha_e" \
    --vector-annotation-manifest-sha256 "$sha_f" \
    --embedding-command "$embedding_command" \
    --model-id reviewed-local-model \
    --dimension 3 \
    --model-manifest-sha256 "$sha_1" \
    --resume-benchmark "$fake_resume_benchmark" \
    --resume-cli "$fake_resume_cli" \
    --data-dir "$data_dir"
  printf '%s\n' "$@"
}

unreviewed_stdout="$tmpdir/unreviewed.stdout"
unreviewed_stderr="$tmpdir/unreviewed.stderr"
set +e
FAKE_RESUME_BENCHMARK_ARGS="$fake_resume_benchmark_args" \
FAKE_RESUME_CLI_ARGS="$fake_resume_cli_args" \
"$quality_script" $(run_common_args) > "$unreviewed_stdout" 2> "$unreviewed_stderr"
unreviewed_status=$?
set -e
if [ "$unreviewed_status" -eq 0 ]; then
  fail "local quality evidence preparation accepted unreviewed quality datasets"
fi
require_text "$unreviewed_stderr" "quality review is incomplete"
reject_text "$unreviewed_stdout" "$tmpdir" "temporary local path"
reject_text "$unreviewed_stderr" "$tmpdir" "temporary local path"

stdout_file="$tmpdir/stdout.txt"
stderr_file="$tmpdir/stderr.txt"
FAKE_RESUME_BENCHMARK_ARGS="$fake_resume_benchmark_args" \
FAKE_RESUME_CLI_ARGS="$fake_resume_cli_args" \
"$quality_script" --reviewed $(run_common_args) > "$stdout_file" 2> "$stderr_file"

if [ -s "$stderr_file" ]; then
  fail "local quality evidence preparation wrote stderr on success"
fi

require_text "$stdout_file" "local quality release evidence: written"
require_text "$stdout_file" "field report: private-field-quality.json"
require_text "$stdout_file" "dedupe report: private-dedupe-quality.json"
require_text "$stdout_file" "vector report: private-vector-quality.json"
require_text "$stdout_file" "release readiness: quality evidence accepted; stable release still blocked"
require_text "$stdout_file" "paths: <redacted>"

require_file "$out_dir/private-field-quality.json"
require_file "$out_dir/private-dedupe-quality.json"
require_file "$out_dir/private-vector-quality.json"
require_file "$out_dir/release-readiness-quality.json"
require_text "$out_dir/release-readiness-quality.json" '"label": "field extraction quality"'
require_text "$out_dir/release-readiness-quality.json" '"label": "dedupe quality"'
require_text "$out_dir/release-readiness-quality.json" '"label": "vector quality"'

require_text "$fake_resume_benchmark_args" "field-quality"
require_text "$fake_resume_benchmark_args" "field-gate"
require_text "$fake_resume_benchmark_args" "dedupe-quality"
require_text "$fake_resume_benchmark_args" "dedupe-gate"
require_text "$fake_resume_benchmark_args" "vector-quality"
require_text "$fake_resume_benchmark_args" "vector-gate"
require_text "$fake_resume_benchmark_args" "--private-business-labeled"
require_text "$fake_resume_benchmark_args" "--require-private-business-labeled"
require_text "$fake_resume_benchmark_args" "--min-samples 1000"
require_text "$fake_resume_benchmark_args" "--min-pairs 1000"
require_text "$fake_resume_benchmark_args" "--min-positive-pairs 100"
require_text "$fake_resume_benchmark_args" "--min-recall-at-k 0.90"

require_text "$fake_resume_cli_args" "release-readiness --json"
require_text "$fake_resume_cli_args" "--field-quality-report"
require_text "$fake_resume_cli_args" "--dedupe-quality-report"
require_text "$fake_resume_cli_args" "--vector-quality-report"

reject_text "$stdout_file" "$tmpdir" "temporary local path"
reject_text "$stderr_file" "$tmpdir" "temporary local path"
reject_text "$stdout_file" "PRIVATE-quality" "private quality marker"
reject_text "$stderr_file" "PRIVATE-quality" "private quality marker"
reject_text "$stdout_file" "field labels stay local" "raw field labels"
reject_text "$stdout_file" "dedupe labels stay local" "raw dedupe labels"
reject_text "$stdout_file" "vector labels stay local" "raw vector labels"
reject_regex "$stdout_file" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"

printf '%s\n' "local quality release-evidence check passed"

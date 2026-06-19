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

resolve_cargo() {
  configured="${CARGO:-cargo}"
  if command -v "$configured" >/dev/null 2>&1; then
    printf '%s' "$configured"
    return
  fi
  if [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
    printf '%s' /Users/frankqdwang/.cargo/bin/cargo
    return
  fi
  fail "local quality release-evidence check requires cargo"
}

run_real_quality_benchmark_smoke() {
  cargo_bin="$(resolve_cargo)"
  smoke_dir="$tmpdir/real-quality-smoke"
  mkdir -p "$smoke_dir"

  field_dataset="$smoke_dir/private-field-quality.jsonl"
  dedupe_dataset="$smoke_dir/private-dedupe-quality.jsonl"
  vector_dataset="$smoke_dir/private-vector-quality.jsonl"
  embedding_command="$smoke_dir/embedding-command.sh"
  field_report="$smoke_dir/private-field-quality.json"
  dedupe_report="$smoke_dir/private-dedupe-quality.json"
  vector_report="$smoke_dir/private-vector-quality.json"

  cat > "$field_dataset" <<'JSONL'
{"sample_id":"private-field-smoke-001","text":"Name: Synthetic Field Candidate\nSummary: REDACTION_SENTINEL_FIELD_VALUE\nEmail: field-candidate@example.test\nPhone: +1 (415) 555-0132\nWeChat: Candidate_2026\nEducation\nSchool: Synthetic 985 University (985/211/双一流)\nDegree: Bachelor of Engineering\nMajor: Computer Science\nLocation: Shanghai\nExperience\nCompany: Synthetic Commerce Inc.\nTitle: Product Manager\n2020年1月 - 2024年3月\nCertifications\nPMP\nSkills: Rust, Java","expected":[{"type":"name","normalized":"synthetic field candidate"},{"type":"email","normalized":"field-candidate@example.test"},{"type":"phone","normalized":"+14155550132"},{"type":"wechat","normalized":"candidate_2026"},{"type":"school","normalized":"synthetic 985 university (985/211/双一流)"},{"type":"school_tier","normalized":"985"},{"type":"school_tier","normalized":"211"},{"type":"school_tier","normalized":"double_first_class"},{"type":"degree","normalized":"bachelor"},{"type":"major","normalized":"computer_science"},{"type":"location","normalized":"shanghai"},{"type":"company","normalized":"synthetic commerce"},{"type":"title","normalized":"product_manager"},{"type":"date_range","normalized":"2020-01/2024-03"},{"type":"years_experience","normalized":"4.2"},{"type":"certificate","normalized":"pmp"},{"type":"skill","normalized":"Rust"},{"type":"skill","normalized":"Java"}]}
JSONL

  cat > "$dedupe_dataset" <<'JSONL'
{"sample_id":"private-dedupe-smoke-001","left":{"id":"private-left-doc-001","name":"Synthetic Duplicate Candidate","schools":["Synthetic University"],"companies":["Synthetic Commerce"],"skills":["Rust","Payments","REDACTION_SENTINEL_DEDUPE_VALUE"]},"right":{"id":"private-right-doc-001","name":"synthetic duplicate candidate","schools":["synthetic university"],"companies":["Synthetic Commerce"],"skills":["Rust","Search"]},"duplicate":true}
{"sample_id":"private-dedupe-smoke-002","left":{"id":"private-left-doc-002","name":"Synthetic Duplicate Candidate","schools":["Synthetic University"],"companies":["Synthetic Commerce"],"skills":["Rust"]},"right":{"id":"private-right-doc-002","name":"Different Synthetic Candidate","schools":["Synthetic University"],"companies":["Synthetic Commerce"],"skills":["Rust"]},"duplicate":false}
JSONL

  cat > "$vector_dataset" <<'JSONL'
{"sample_id":"private-vector-smoke-001","query":"REDACTION_SENTINEL_VECTOR_QUERY backend java payment","candidates":[{"id":"private-vector-candidate-001","text":"REDACTION_SENTINEL_VECTOR_CANDIDATE Java payment backend search engineer","relevant":true},{"id":"private-vector-candidate-002","text":"Synthetic sales operations","relevant":false}]}
{"sample_id":"private-vector-smoke-002","query":"REDACTION_SENTINEL_VECTOR_QUERY rust indexing","candidates":[{"id":"private-vector-candidate-003","text":"REDACTION_SENTINEL_VECTOR_CANDIDATE Rust indexing platform engineer","relevant":true},{"id":"private-vector-candidate-004","text":"Synthetic HR partner","relevant":false}]}
JSONL

  cat > "$embedding_command" <<'SH'
#!/usr/bin/env sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=%s\n' "$RESUME_IR_EMBEDDING_MODEL_ID"
printf 'dimension=%s\n' "$RESUME_IR_EMBEDDING_DIMENSION"
awk '
  /^input=/ {
    split(substr($0, 7), parts, "\t");
    id = parts[1];
    if (id ~ /^query-000000/ || id ~ /^candidate-000000-000000/) {
      vector = "1.0,0.0,0.0";
    } else if (id ~ /^query-000001/ || id ~ /^candidate-000001-000000/) {
      vector = "0.0,1.0,0.0";
    } else {
      vector = "0.0,0.0,1.0";
    }
    printf "vector=%s\t%s\n", id, vector;
  }
' "$RESUME_IR_EMBEDDING_INPUT_PATH"
SH
  chmod 700 "$embedding_command"

  sha_a="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  sha_b="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  sha_c="cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"

  "$cargo_bin" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- field-quality \
    --dataset "$field_dataset" \
    --private-business-labeled \
    --dataset-manifest-sha256 "$sha_a" \
    --annotation-manifest-sha256 "$sha_b" \
    --json > "$field_report"
  "$cargo_bin" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- field-gate \
    --report "$field_report" \
    --require-private-business-labeled \
    --min-samples 1 \
    --min-precision 0.93 \
    --min-recall 0.93 \
    --min-f1 0.93 >/dev/null

  "$cargo_bin" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- dedupe-quality \
    --dataset "$dedupe_dataset" \
    --private-business-labeled \
    --dataset-manifest-sha256 "$sha_a" \
    --annotation-manifest-sha256 "$sha_b" \
    --json > "$dedupe_report"
  "$cargo_bin" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- dedupe-gate \
    --report "$dedupe_report" \
    --require-private-business-labeled \
    --min-pairs 2 \
    --min-positive-pairs 1 \
    --min-precision 0.90 \
    --min-recall 0.90 \
    --min-f1 0.90 >/dev/null

  "$cargo_bin" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- vector-quality \
    --dataset "$vector_dataset" \
    --command "$embedding_command" \
    --model-id smoke-local-model \
    --dimension 3 \
    --private-business-labeled \
    --dataset-manifest-sha256 "$sha_a" \
    --annotation-manifest-sha256 "$sha_b" \
    --model-manifest-sha256 "$sha_c" \
    --top-k 1 \
    --json > "$vector_report"
  "$cargo_bin" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- vector-gate \
    --report "$vector_report" \
    --require-private-business-labeled \
    --min-samples 2 \
    --min-recall-at-k 0.90 \
    --min-mrr 0.90 \
    --min-ndcg-at-k 0.90 \
    --max-zero-recall-queries 0 >/dev/null

  require_text "$field_report" '"schema_version":"field-quality.v1"'
  require_text "$dedupe_report" '"schema_version":"dedupe-quality.v1"'
  require_text "$vector_report" '"schema_version":"vector-quality.v1"'
  require_text "$field_report" '"dataset_kind":"private-business-labeled"'
  require_text "$dedupe_report" '"dataset_kind":"private-business-labeled"'
  require_text "$vector_report" '"dataset_kind":"private-business-labeled"'

  for report in "$field_report" "$dedupe_report" "$vector_report"; do
    reject_text "$report" "$smoke_dir" "temporary quality smoke path"
    reject_text "$report" "REDACTION_SENTINEL" "raw quality smoke payload"
    reject_regex "$report" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"
  done
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

run_real_quality_benchmark_smoke
printf '%s\n' "real benchmark smoke: passed"
printf '%s\n' "local quality release-evidence check passed"

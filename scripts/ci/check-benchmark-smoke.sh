#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

CARGO_BIN="${CARGO:-cargo}"
if ! command -v "$CARGO_BIN" >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi
if ! command -v "$CARGO_BIN" >/dev/null 2>&1; then
  fail "cargo is required for benchmark smoke check"
fi

umask 077
tmpdir=$(mktemp -d)
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

query_report="$tmpdir/benchmark-smoke.json"
ocr_report="$tmpdir/ocr-benchmark-smoke.json"
vector_report="$tmpdir/vector-benchmark-smoke.json"

validate_json() {
  report="$1"
  if [ ! -s "$report" ]; then
    fail "benchmark smoke report is missing or empty: $(basename "$report")"
  fi
  if command -v python3 >/dev/null 2>&1; then
    python3 -m json.tool "$report" >/dev/null
  fi
}

assert_report_boundary() {
  report="$1"
  label="$2"
  validate_json "$report"
  if grep -Fq -- "$tmpdir" "$report"; then
    fail "$label leaked a temporary path"
  fi
  if grep -Eq '/Users/|/home/|/private/|/var/folders|[A-Za-z]:\\|local-data|diagnostics|model-cache|ocr-fixture|embedding-fixture|vector-dataset|RESUME_IR_|Synthetic OCR smoke|resume-ir-ocr-v1|Backend Java payment search|Java payment backend search engineer|Sales operations recruiter|Rust indexing platform|HR business partner' "$report"; then
    fail "$label leaked a local path, runtime-data marker, command marker, or fixture payload"
  fi
}

"$CARGO_BIN" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- synthetic-query \
  --index-dir "$tmpdir/query-index" \
  --documents 24 \
  --queries 6 \
  --top-k 5 \
  --json > "$query_report"
"$CARGO_BIN" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- gate \
  --report "$query_report" \
  --allow-synthetic \
  --min-documents 24 \
  --min-queries 6 \
  --max-p95-ms 1000 \
  --max-zero-result-queries 0
assert_report_boundary "$query_report" "query benchmark smoke report"

printf '%s\n' \
  '#!/usr/bin/env sh' \
  'printf "resume-ir-ocr-v1\nconfidence=0.97\ntext:\nSynthetic OCR smoke page %s\n" "$RESUME_IR_OCR_PAGE_NO"' \
  > "$tmpdir/ocr-fixture.sh"
chmod 700 "$tmpdir/ocr-fixture.sh"

"$CARGO_BIN" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- ocr-throughput \
  --command "$tmpdir/ocr-fixture.sh" \
  --pages 3 \
  --page-timeout-ms 5000 \
  --json > "$ocr_report"
"$CARGO_BIN" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- ocr-gate \
  --report "$ocr_report" \
  --allow-synthetic \
  --min-pages 3 \
  --max-p95-ms 5000 \
  --min-pages-per-second 0.001
assert_report_boundary "$ocr_report" "OCR benchmark smoke report"

cat > "$tmpdir/embedding-fixture.sh" <<'SH'
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
chmod 700 "$tmpdir/embedding-fixture.sh"

cat > "$tmpdir/vector-dataset.jsonl" <<'JSONL'
{"query":"Backend Java payment search","candidates":[{"id":"java-match","text":"Java payment backend search engineer","relevant":true},{"id":"sales-miss","text":"Sales operations recruiter","relevant":false}]}
{"query":"Rust indexing platform","candidates":[{"id":"rust-match","text":"Rust indexing platform engineer","relevant":true},{"id":"hr-miss","text":"HR business partner","relevant":false}]}
JSONL

"$CARGO_BIN" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- vector-quality \
  --dataset "$tmpdir/vector-dataset.jsonl" \
  --command "$tmpdir/embedding-fixture.sh" \
  --model-id fixture-local-model \
  --dimension 3 \
  --top-k 1 \
  --json > "$vector_report"
"$CARGO_BIN" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- vector-gate \
  --report "$vector_report" \
  --min-samples 2 \
  --min-recall-at-k 0.99 \
  --min-mrr 0.99 \
  --min-ndcg-at-k 0.99 \
  --max-zero-recall-queries 0
assert_report_boundary "$vector_report" "vector benchmark smoke report"

printf '%s\n' "benchmark smoke check passed"

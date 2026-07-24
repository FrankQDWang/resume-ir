#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

debug_file() {
  [ "${RESUME_IR_CLOSED_LOOP_DEBUG:-0}" = "1" ] || return 0
  benchmark_debug_path="$1"
  [ -s "$benchmark_debug_path" ] || return 0
  sed -n '1,120p' "$benchmark_debug_path" >&2 || true
}

CARGO_BIN="${CARGO:-cargo}"
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
protocol_report="$tmpdir/query-protocol-smoke.txt"
private_query_report="$tmpdir/private-query-smoke.json"
private_query_set="$tmpdir/private-query-smoke.local.jsonl"
private_query_summary="$tmpdir/private-query-smoke.summary.json"
query_set_preflight_report="$tmpdir/query-set-preflight-smoke.json"
query_set_preflight_stdout="$tmpdir/query-set-preflight-smoke.stdout"
query_set_preflight_stderr="$tmpdir/query-set-preflight-smoke.stderr"
smoke_report_out="${RESUME_IR_BENCHMARK_SMOKE_REPORT_OUT:-}"
smoke_manifest_out="${RESUME_IR_BENCHMARK_SMOKE_MANIFEST_OUT:-}"

if [ -n "$smoke_manifest_out" ] && [ -z "$smoke_report_out" ]; then
  fail "RESUME_IR_BENCHMARK_SMOKE_MANIFEST_OUT requires RESUME_IR_BENCHMARK_SMOKE_REPORT_OUT"
fi
if [ -n "$smoke_report_out" ] && [ -z "$smoke_manifest_out" ]; then
  fail "RESUME_IR_BENCHMARK_SMOKE_REPORT_OUT requires RESUME_IR_BENCHMARK_SMOKE_MANIFEST_OUT"
fi
if [ -n "$smoke_report_out" ] && [ "$smoke_report_out" = "$smoke_manifest_out" ]; then
  fail "synthetic smoke report and manifest outputs must use distinct paths"
fi
if [ -n "$smoke_report_out" ] && { [ -L "$smoke_report_out" ] || [ -L "$smoke_manifest_out" ]; }; then
  fail "synthetic smoke report and manifest outputs must not be symlinks"
fi
if [ -n "$smoke_report_out" ] && { [ -d "$smoke_report_out" ] || [ -d "$smoke_manifest_out" ]; }; then
  fail "synthetic smoke report and manifest outputs must be file paths, not directories"
fi
canonical_output_path() {
  output_path="$1"
  output_dir=$(dirname "$output_path")
  output_base=$(basename "$output_path")
  mkdir -p "$output_dir"
  (
    cd "$output_dir"
    printf '%s/%s\n' "$(pwd -P)" "$output_base"
  )
}
if [ -n "$smoke_report_out" ]; then
  smoke_report_canonical=$(canonical_output_path "$smoke_report_out")
  smoke_manifest_canonical=$(canonical_output_path "$smoke_manifest_out")
  if [ "$smoke_report_canonical" = "$smoke_manifest_canonical" ]; then
    fail "synthetic smoke report and manifest outputs must resolve to distinct paths"
  fi
fi

validate_json() {
  report="$1"
  if [ ! -s "$report" ]; then
    fail "benchmark smoke report is missing or empty: $(basename "$report")"
  fi
  if command -v python3 >/dev/null 2>&1; then
    python3 -m json.tool "$report" >/dev/null
  fi
}

report_boundary_deny='/Users/|/home/|/private/|/var/folders|[A-Za-z]:\\|local-data|diagnostics|model-cache|ocr-fixture|embedding-fixture|vector-dataset|RESUME_IR_|Synthetic OCR smoke|resume-ir-ocr-v1|Backend Java payment search|Java payment backend search engineer|Sales operations recruiter|Rust indexing platform|HR business partner'
evidence_boundary_deny='/Users/|/home/|/private/|/var/folders|[A-Za-z]:\\|local-data|model-cache|ocr-fixture|embedding-fixture|vector-dataset|RESUME_IR_|Synthetic OCR smoke|resume-ir-ocr-v1|Backend Java payment search|Java payment backend search engineer|Sales operations recruiter|Rust indexing platform|HR business partner'
text_boundary_deny='/Users/|/home/|/private/|/var/folders|[A-Za-z]:\\|local-data|model-cache|ocr-fixture|embedding-fixture|vector-dataset|RESUME_IR_|SemanticOnlyToken|synthetic-java-platform|synthetic-java-engineer|Synthetic Candidate|payment gateway|local search'

assert_no_boundary_leak() {
  report="$1"
  label="$2"
  deny_pattern="$3"
  message="$4"
  if grep -Fq -- "$tmpdir" "$report"; then
    fail "$label leaked a temporary path"
  fi
  if grep -Eq "$deny_pattern" "$report"; then
    fail "$label $message"
  fi
}

assert_report_boundary() {
  report="$1"
  label="$2"
  validate_json "$report"
  assert_no_boundary_leak "$report" "$label" "$report_boundary_deny" "leaked a local path, runtime-data marker, command marker, or fixture payload"
}

assert_evidence_boundary() {
  report="$1"
  label="$2"
  validate_json "$report"
  assert_no_boundary_leak "$report" "$label" "$evidence_boundary_deny" "leaked a local path, runtime-data marker, command marker, or fixture payload"
}

assert_text_boundary() {
  report="$1"
  label="$2"
  if [ ! -s "$report" ]; then
    fail "$label is missing or empty"
  fi
  assert_no_boundary_leak "$report" "$label" "$text_boundary_deny" "leaked a local path, runtime-data marker, command marker, query text, or fixture payload"
}

require_text() {
  report="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$report"; then
    fail "$(basename "$report") is missing required text: $text"
  fi
}

require_stage_timing() {
  report="$1"
  stage="$2"
  if ! grep -Eq "^${stage}=[0-9]+(\\.[0-9]+)?$" "$report"; then
    fail "$(basename "$report") is missing numeric $stage"
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

cat > "$tmpdir/protocol-resident-fixture.sh" <<'SH'
#!/usr/bin/env sh
set -eu

: "${RESUME_IR_QUERY_BATCH_INPUT_PATH:?}"
: "${RESUME_IR_QUERY_MODE:?}"
: "${RESUME_IR_QUERY_TOP_K:?}"
while IFS= read -r protocol_request; do
  protocol_request_id=$(printf '%s\n' "$protocol_request" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  [ -n "$protocol_request_id" ] || exit 2
  printf 'resume-ir-query-v2\nrequest_id=%s\nmode=%s\nlayers=fulltext+field+vector+rrf\ntop_k=%s\nquery_embedding_runtime=local-command\nquery_embedding_invocations=1\nstage_query_parse_ms=1.0\nstage_prefilter_ms=1.0\nstage_bm25_ms=1.0\nstage_ann_ms=1.0\nstage_fusion_ms=1.0\nstage_bulk_hydrate_ms=1.0\nstage_snippet_ms=1.0\nrss_delta_mb=0.0\nelapsed_ms=1.0\nhits=2\nresume-ir-query-end\n' "$protocol_request_id" "$RESUME_IR_QUERY_MODE" "$RESUME_IR_QUERY_TOP_K"
done < "$RESUME_IR_QUERY_BATCH_INPUT_PATH"
SH
chmod 700 "$tmpdir/protocol-resident-fixture.sh"

mkdir -p "$tmpdir/query-protocol-private-input"
printf '%s\n' \
  '{"schema_version":"resume-ir.query-batch-request.v2","request_id":"synthetic-smoke-1","query":"SemanticOnlyToken"}' \
  '{"schema_version":"resume-ir.query-batch-request.v2","request_id":"synthetic-smoke-2","query":"SemanticOnlyToken"}' \
  > "$tmpdir/query-protocol-private-input/queries.jsonl"

# A temporary shell process cannot satisfy the daemon's production executable
# attestation. The native, attested atomic-publication path is owned by the
# exact daemon integration test; this portable smoke owns only the redacted
# resident batch-protocol and evidence aggregation contract.
set +e
env \
  RESUME_IR_QUERY_BATCH_INPUT_PATH="$tmpdir/query-protocol-private-input/queries.jsonl" \
  RESUME_IR_QUERY_TOP_K=20 \
  RESUME_IR_QUERY_MODE=hybrid \
  "$tmpdir/protocol-resident-fixture.sh" \
  > "$protocol_report" \
  2> "$tmpdir/query-protocol-smoke.stderr"
protocol_status=$?
set -e
if [ "$protocol_status" -ne 0 ]; then
  debug_file "$protocol_report"
  debug_file "$tmpdir/query-protocol-smoke.stderr"
  fail "benchmark query protocol fixture failed"
fi
assert_text_boundary "$protocol_report" "query protocol smoke report"
require_text "$protocol_report" "resume-ir-query-v2"
require_text "$protocol_report" "request_id=synthetic-smoke-1"
require_text "$protocol_report" "request_id=synthetic-smoke-2"
require_text "$protocol_report" "mode=hybrid"
require_text "$protocol_report" "layers=fulltext+field+vector+rrf"
require_text "$protocol_report" "top_k=20"
require_text "$protocol_report" "query_embedding_runtime=local-command"
require_text "$protocol_report" "query_embedding_invocations=1"
require_text "$protocol_report" "hits=2"
require_text "$protocol_report" "resume-ir-query-end"
for stage in \
  stage_query_parse_ms \
  stage_prefilter_ms \
  stage_bm25_ms \
  stage_ann_ms \
  stage_fusion_ms \
  stage_bulk_hydrate_ms \
  stage_snippet_ms \
  elapsed_ms
do
  require_stage_timing "$protocol_report" "$stage"
done
require_stage_timing "$protocol_report" "rss_delta_mb"
mkdir -p "$tmpdir/query-set-trace/run/runtime"
printf '%s\n' \
  '[2026-06-05T12:09:20+08:00] | tool_called | round=1 | tool=source_search | Java' \
  '[2026-06-05T12:09:21+08:00] | tool_called | round=1 | tool=source_search | search indexing' \
  '[2026-06-05T12:09:22+08:00] | tool_called | round=1 | tool=source_search | payment reconciliation' \
  > "$tmpdir/query-set-trace/run/runtime/trace.log"
"$CARGO_BIN" run --quiet -p resume-cli --bin resume-cli --locked -- \
  --data-dir "$tmpdir/query-protocol-data" \
  benchmark-query-set \
  preflight-agent-replay \
  --out "$query_set_preflight_report" \
  --trace-root "$tmpdir/query-set-trace" \
  --max-queries 1 \
  > "$query_set_preflight_stdout" \
  2> "$query_set_preflight_stderr"
if [ -s "$query_set_preflight_stderr" ]; then
  fail "synthetic query-set preflight wrote stderr"
fi
assert_text_boundary "$query_set_preflight_stdout" "query set preflight smoke stdout"
assert_evidence_boundary "$query_set_preflight_report" "query set preflight smoke report"
require_text "$query_set_preflight_stdout" "query set trace preflight: written"
require_text "$query_set_preflight_stdout" "schema: resume-ir.query-set-trace-preflight.v1"
require_text "$query_set_preflight_stdout" "privacy boundary: redacted_local_aggregate"
if grep -Fq 'Java' "$query_set_preflight_stdout" || grep -Fq 'search indexing' "$query_set_preflight_stdout" || grep -Fq 'payment reconciliation' "$query_set_preflight_stdout"; then
  fail "query set preflight smoke stdout leaked raw query text"
fi

# The portable smoke owns the redacted batch-runner envelope. Its local-hit
# selection and production publication are covered by their focused CLI and
# attested daemon integration tests, respectively.
printf '%s\n' \
  '{"schema_version":"resume-ir.query-set.jsonl.v2","sample_id":"synthetic-smoke-000001","bucket":"single_term","query":"Java","source_kind":"trace_source_search_v1","query_shape":{"term_count":1,"has_boolean":false,"has_location":false,"has_years":false,"has_degree":false,"has_skill":true,"has_phrase":false}}' \
  > "$private_query_set"
cat > "$private_query_summary" <<'JSON'
{
  "schema_version": "resume-ir.query-set-summary.v2",
  "privacy_boundary": "redacted_local_aggregate",
  "query_source": "trace_source_search_v1",
  "query_count": 1,
  "tune_query_count": 1,
  "holdout_query_count": 0,
  "bucket_counts": {"single_term": 1, "and_2": 0, "and_3_5": 0, "and_6_16": 0, "field_filter": 0, "hybrid": 0, "semantic": 0},
  "tune_bucket_counts": {"single_term": 1, "and_2": 0, "and_3_5": 0, "and_6_16": 0, "field_filter": 0, "hybrid": 0, "semantic": 0},
  "holdout_bucket_counts": {"single_term": 0, "and_2": 0, "and_3_5": 0, "and_6_16": 0, "field_filter": 0, "hybrid": 0, "semantic": 0},
  "candidate_queries_sampled": 1,
  "zero_hit_queries_dropped": 0,
  "query_set_sha256": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
  "tune_sha256": "2222222222222222222222222222222222222222222222222222222222222222",
  "holdout_sha256": "3333333333333333333333333333333333333333333333333333333333333333",
  "hmac_split": true,
  "contains_raw_query_text": false,
  "contains_raw_resume_text": false,
  "contains_candidate_results": false,
  "contains_local_paths": false
}
JSON
assert_evidence_boundary "$private_query_summary" "synthetic query-set smoke summary"
cat > "$tmpdir/private-query-corpus-summary.json" <<'JSON'
{
  "schema_version": "benchmark-corpus-summary.v1",
  "privacy_boundary": "redacted_local_aggregate",
  "document_count": 2,
  "searchable_document_count": 2,
  "vector_indexed_document_count": 2,
  "active_vector_document_count": 2,
  "vector_count": 2,
  "vector_deleted_count": 0,
  "vector_index_state": "available",
  "vector_search_backend": "hnsw_ann",
  "hot_index_fully_covered": true,
  "contains_raw_resume_text": false,
  "contains_resume_paths": false,
  "contains_queries": false,
  "contains_sample_ids": false
}
JSON
"$CARGO_BIN" run --quiet -p benchmark-runner --bin resume-benchmark --locked -- private-query \
  --query-set "$private_query_set" \
  --resident-command "$tmpdir/protocol-resident-fixture.sh" \
  --corpus-summary "$tmpdir/private-query-corpus-summary.json" \
  --synthetic-smoke-evidence \
  --max-queries 1 \
  --request-sample-count 2 \
  --top-k 20 \
  --timeout-ms 10000 \
  --index-size-bytes 0 \
  --dataset-manifest-sha256 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
  --model-manifest-sha256 1111111111111111111111111111111111111111111111111111111111111111 \
  --json > "$private_query_report"
assert_report_boundary "$private_query_report" "private query runner smoke report"
require_text "$private_query_report" '"dataset_kind":"synthetic-smoke"'
require_text "$private_query_report" '"target_claim":"not_evaluated"'
require_text "$private_query_report" '"corpus_origin":"synthetic_public_fixture"'
require_text "$private_query_report" '"query_runner":"resident-batch-command"'
require_text "$private_query_report" '"spawn_per_query":false'
require_text "$private_query_report" '"query_protocol":"resume-ir-query-v2"'
require_text "$private_query_report" '"query_source":"trace_source_search_v1"'
require_text "$private_query_report" '"request_sample_count":2'
require_text "$private_query_report" '"query_embedding_runtime":"local-command"'
require_text "$private_query_report" '"query_embedding_command_invocations":2'
require_text "$private_query_report" '"hot_path_ocr":false'
require_text "$private_query_report" '"hot_path_parsing":false'
require_text "$private_query_report" '"hot_path_heavy_model_inference":false'
require_text "$private_query_report" '"rss_delta_mb":'
if grep -Fq '"target_claim":"benchmark_baseline_observed"' "$private_query_report" || grep -Fq '"dataset_kind":"private-real-corpus"' "$private_query_report"; then
  fail "private query runner smoke report claimed private baseline evidence"
fi
if grep -Fq 'SemanticOnlyToken' "$private_query_report" || grep -Fq 'synthetic-private-query-smoke' "$private_query_report"; then
  fail "private query runner smoke report leaked raw query text or sample id"
fi

if [ -n "$smoke_report_out" ]; then
  command -v python3 >/dev/null 2>&1 || fail "python3 is required to write synthetic smoke evidence"
  mkdir -p "$(dirname "$smoke_report_out")"
  mkdir -p "$(dirname "$smoke_manifest_out")"
  python3 - "$query_report" "$ocr_report" "$vector_report" "$private_query_report" "$protocol_report" "$smoke_report_out" "$smoke_manifest_out" <<'PY'
import hashlib
import json
import pathlib
import subprocess
import sys

root = pathlib.Path.cwd()
query_path, ocr_path, vector_path, private_query_path, protocol_path, report_path, manifest_path = [
    pathlib.Path(arg) for arg in sys.argv[1:]
]

def load_json(path):
    with path.open("rb") as handle:
        return json.load(handle)

def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def contract_pins():
    files = {
        "active_goal_sha256": root / "ACTIVE_GOAL.toml",
        "acceptance_matrix_sha256": root / "perf" / "acceptance-matrix.toml",
        "loop_state_schema_sha256": root / "perf" / "loop-state.schema.json",
        "experiment_report_schema_sha256": root / "perf" / "experiment-report.schema.json",
        "synthetic_smoke_artifact_manifest_schema_sha256": root / "perf" / "synthetic-smoke-artifact-manifest.schema.json",
    }
    pins = {key: sha256_file(path) for key, path in files.items()}
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    pins["git_head_sha"] = head
    return pins

def component(name, path, payload):
    return {
        "component": name,
        "schema_version": payload["schema_version"],
        "report_sha256": sha256_file(path),
        "report_size_bytes": path.stat().st_size,
        "target_claim": "not_evaluated",
    }

def protocol_observation(path):
    keys = {
        "stage_query_parse_ms": "query_parse",
        "stage_prefilter_ms": "prefilter",
        "stage_bm25_ms": "bm25",
        "stage_ann_ms": "ann",
        "stage_fusion_ms": "fusion",
        "stage_bulk_hydrate_ms": "bulk_hydrate",
        "stage_snippet_ms": "snippet",
        "elapsed_ms": "elapsed",
    }
    observed = {}
    request_ids = set()
    rss_delta_mb = None
    for line in path.read_text(encoding="utf-8").splitlines():
        key, separator, raw_value = line.partition("=")
        if not separator:
            continue
        if key == "request_id":
            request_ids.add(raw_value)
            continue
        if key == "rss_delta_mb":
            value = float(raw_value)
            if value < 0:
                raise SystemExit("negative protocol rss delta")
            rss_delta_mb = max(rss_delta_mb or 0.0, value)
            continue
        if key not in keys:
            continue
        value = float(raw_value)
        if value < 0:
            raise SystemExit(f"negative protocol stage latency: {key}")
        observed[keys[key]] = max(observed.get(keys[key], 0.0), value)
    if not request_ids:
        raise SystemExit("missing protocol request ids")
    if rss_delta_mb is None:
        raise SystemExit("missing protocol rss delta")
    missing = sorted(set(keys.values()) - set(observed))
    if missing:
        raise SystemExit(f"missing protocol stage latency: {','.join(missing)}")
    return {
        "request_count": len(request_ids),
        "stage_latency": observed,
        "rss_delta_mb": rss_delta_mb,
    }

query = load_json(query_path)
ocr = load_json(ocr_path)
vector = load_json(vector_path)
private_query = load_json(private_query_path)
protocol = protocol_observation(protocol_path)
components = [
    component("synthetic_query", query_path, query),
    component("ocr_throughput", ocr_path, ocr),
    component("vector_quality", vector_path, vector),
    component("private_query_runner", private_query_path, private_query),
]
privacy = {
    "contains_raw_resume_text": False,
    "contains_raw_query_text": False,
    "contains_candidate_results": False,
    "contains_local_paths": False,
    "contains_tokens": False,
    "contains_diagnostics_package": False,
    "trace_summary_redacted": True,
}
report = {
    "schema_version": "resume-ir.experiment-report.v2",
    "goal_id": "resume-ir.performance-gui-loop.2026-06",
    "report_kind": "redacted_evidence",
    "claim": "no_claim",
    "evidence_lane": "smoke",
    "contract_pins": contract_pins(),
    "synthetic_smoke": {
        "smoke_schema_version": "resume-ir.synthetic-smoke-baseline.v1",
        "source": "synthetic_public_fixture",
        "benchmark_command": (
            "resume-benchmark synthetic-query --index-dir <redacted-temp-index> "
            f"--documents {query['document_count']} --queries {query['query_count']} "
            f"--top-k {query['top_k']} --json"
        ),
        "document_count": query["document_count"],
        "query_count": query["query_count"],
        "top_k": query["top_k"],
        "percentile_confidence": "smoke",
        "batch_protocol_request_count": protocol["request_count"],
        "component_reports": components,
        "harness_observations": {
            "uses_private_resume_root": False,
            "uses_query_artifact_root": False,
            "uses_synthetic_public_fixtures": True,
            "resident_daemon_required": False,
            "resident_daemon_observed": False,
            "batch_protocol_observed": True,
            "private_query_runner_query_protocol": private_query["query_protocol"],
            "private_query_runner_request_sample_count": private_query["request_sample_count"],
            "private_query_runner_query_embedding_command_invocations": private_query[
                "query_embedding_command_invocations"
            ],
            "spawn_per_query": False,
        },
        "latency_ms": {
            "query_p95": query["query_latency_ms"]["p95"],
            "ocr_p95": ocr["page_latency_ms"]["p95"],
            "batch_protocol_stage": protocol["stage_latency"],
        },
        "resource_observations": {
            "batch_protocol_rss_delta_mb": protocol["rss_delta_mb"],
            "private_query_runner_rss_delta_mb": private_query["rss_delta_mb"],
        },
        "quality": {
            "vector_recall_at_k": vector["recall_at_k"],
            "vector_mrr": vector["mrr"],
            "vector_ndcg_at_k": vector["ndcg_at_k"],
            "zero_result_queries": query["zero_result_queries"],
            "zero_recall_queries": vector["zero_recall_queries"],
        },
    },
    "thresholds": {
        "matrix": "perf/acceptance-matrix.toml",
        "matrix_schema_version": "resume-ir.perf.acceptance-matrix.v2",
        "passed": True,
        "failed_redlines": [],
    },
    "privacy": privacy,
}
with report_path.open("w", encoding="utf-8") as handle:
    json.dump(report, handle, indent=2, sort_keys=True)
    handle.write("\n")

artifacts = report["synthetic_smoke"]["component_reports"]
report_sha256 = sha256_file(report_path)
print(f"redacted_smoke_report_sha256={report_sha256}")
manifest = {
    "schema_version": "resume-ir.synthetic-smoke-artifact-manifest.v1",
    "goal_id": "resume-ir.performance-gui-loop.2026-06",
    "manifest_kind": "synthetic_smoke_baseline",
    "report_schema_version": report["schema_version"],
    "report_kind": report["report_kind"],
    "evidence_lane": report["evidence_lane"],
    "claim": report["claim"],
    "contract_pins": report["contract_pins"],
    "privacy": report["privacy"],
    "report_sha256": report_sha256,
    "report_size_bytes": report_path.stat().st_size,
    "artifacts": artifacts,
}
with manifest_path.open("w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
  assert_evidence_boundary "$smoke_report_out" "synthetic smoke baseline report"
  printf '%s\n' "benchmark smoke redacted evidence report written"
  assert_evidence_boundary "$smoke_manifest_out" "synthetic smoke artifact manifest"
  python3 scripts/ci/check-experiment-report.py "$smoke_report_out" "$smoke_manifest_out"
  printf '%s\n' "benchmark smoke artifact manifest written"
fi

printf '%s\n' "benchmark smoke check passed"

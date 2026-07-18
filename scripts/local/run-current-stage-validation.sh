#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage: scripts/local/run-current-stage-validation.sh [--dry-run|--execute]
  [--resume-root DIR] [--data-dir DIR] [--out-dir DIR] [--query-set FILE]
  [--query-set-trace-root DIR]
  [--validation-profile full|smoke]
  --model-manifest FILE --ocr-runtime-manifest FILE
  --model-artifact FILE --embedding-command FILE
  [--embedding-runtime-bin-dir DIR]
  --model-pack-id ID --model-id ID --model-format ID --dimension N --model-license ID
  --runtime-pack-id ID [--tesseract-command FILE] [--pdftoppm-command FILE]
  [--runtime-distribution-mode bundled|external]
  --language LANG --language-pack FILE|LANG=FILE [--language-pack LANG=FILE ...]
  --engine-license ID --renderer-license ID --language-license ID
  [--dataset-manifest-sha256 SHA256]
  [--reuse-imported-corpus --reuse-dataset-manifest FILE]
  [--model-manifest-sha256 SHA256]
  [--ocr-runtime-manifest-sha256 SHA256]
  [--renderer-manifest-sha256 SHA256]
  [--language-pack-manifest-sha256 SHA256]
  [--resume-cli PATH] [--resume-daemon PATH] [--resume-benchmark PATH]
  [--reviewed-model] [--reviewed-ocr-runtime]
  [--max-files N] [--max-queries N] [--top-k N]
  [--private-query-request-sample-count N]
  [--private-query-timeout-ms N]
  [--worker-interval-ms N] [--ocr-worker-ticks N] [--ocr-jobs-per-tick N]
  [--ocr-throughput-max-documents N] [--ocr-throughput-max-pages N]
  [--ocr-throughput-pages-per-document N] [--ocr-throughput-max-run-ms N]
  [--ocr-throughput-min-pages N]

Default mode is --dry-run and default validation profile is full. Dry-run prints
a redacted JSON plan and never reads the private resume root. Execute mode runs
local-only commands and writes local evidence under --out-dir. --resume-root
defaults to RESUME_IR_PRIVATE_RESUME_ROOT, --data-dir defaults to
RESUME_IR_DATA_DIR, and --out-dir defaults to RESUME_IR_LOCAL_EVIDENCE_DIR when
omitted. The smoke profile proves wiring only; it does not produce
release-readiness evidence. Execute mode may explicitly reuse an already
imported data-dir and a prior redacted dataset manifest to continue bounded
local validation without rescanning the private corpus.
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

detect_command_path() {
  name="$1"
  value="$2"
  if [ -n "$value" ]; then
    printf '%s\n' "$value"
    return 0
  fi
  command -v "$name" 2>/dev/null || fail "missing required runtime command: $name; install it or pass --${name}-command"
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

query_set_summary_path() {
  file_name=$(basename "$1")
  dir_name=$(dirname "$1")
  case "$file_name" in
    *.local.jsonl) base_name=${file_name%.local.jsonl} ;;
    *) base_name=$file_name ;;
  esac
  printf '%s/%s.summary.json\n' "$dir_name" "$base_name"
}

query_set_summary_sha256() {
  path="$1"
  expected_query_source="${2:-}"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for query-set summary validation"
python3 - "$path" "$expected_query_source" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    summary = json.load(handle)

query_source = summary.get("query_source")
if (
    summary.get("schema_version") != "resume-ir.query-set-summary.v2"
    or summary.get("privacy_boundary") != "redacted_local_aggregate"
    or query_source != "trace_source_search_v1"
    or summary.get("hmac_split") is not True
    or summary.get("contains_raw_query_text") is not False
    or summary.get("contains_raw_resume_text") is not False
    or summary.get("contains_candidate_results") is not False
    or summary.get("contains_local_paths") is not False
):
    raise SystemExit("query set summary boundary invalid")

expected_query_source = sys.argv[2]
if expected_query_source and query_source != expected_query_source:
    raise SystemExit("query set summary source invalid")

digest = summary.get("query_set_sha256")
if not isinstance(digest, str) or not digest:
    raise SystemExit("query set summary digest missing")

print(digest.lower())
PY
}

append_query_set_prepare_summary_stdout() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for query-set summary output"
python3 - "$path" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    summary = json.load(handle)

for label, key in (
    ("query source", "query_source"),
    ("queries", "query_count"),
    ("query set sha256", "query_set_sha256"),
    ("tune sha256", "tune_sha256"),
    ("holdout sha256", "holdout_sha256"),
):
    value = summary.get(key)
    if not isinstance(value, (str, int)) or value == "":
        raise SystemExit(f"query set summary output missing {key}")
    print(f"{label}: {value}")
print("hmac split: true")
PY
}

sha256_file_json_or_null() {
  path="$1"
  if [ -e "$path" ]; then
    printf '"%s"' "$(sha256_file "$path")"
  else
    printf 'null'
  fi
}

query_set_prepare_blocked_reason() {
  stderr_path="$out_dir/query-set-prepare.stderr.txt"
  if [ -f "$stderr_path" ] && grep -Fq "query set blocked: local search index is unavailable" "$stderr_path"; then
    printf '%s\n' "query_set_index_unavailable"
    return
  fi
  if [ -s "$out_dir/query-set-trace-preflight.local.json" ]; then
    python3 - "$out_dir/query-set-trace-preflight.local.json" <<'PY' && return
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    report = json.load(handle)

def positive_int(value: object) -> bool:
    return isinstance(value, int) and value > 0

target_query_count = report.get("target_query_count")
d10k_corpus_deficits = report.get("d10k_corpus_deficits")
if (
    target_query_count == 500
    and report.get("d10k_corpus_ready") is False
    and isinstance(d10k_corpus_deficits, dict)
    and any(positive_int(value) for value in d10k_corpus_deficits.values())
):
    print("query_set_corpus_or_trace_coverage_insufficient")
    raise SystemExit(0)

deficits = report.get("corpus_valid_bucket_deficits")
if isinstance(deficits, dict) and any(positive_int(value) for value in deficits.values()):
    print("query_set_corpus_or_trace_coverage_insufficient")
    raise SystemExit(0)
raise SystemExit(1)
PY
  fi
  printf '%s\n' "query_set_prepare_failed"
}

query_set_prepare_blocked_message() {
  reason="$1"
  case "$reason" in
    query_set_index_unavailable)
      printf '%s\n' "current-stage validation blocked: query-set index unavailable"
      ;;
    query_set_corpus_or_trace_coverage_insufficient)
      printf '%s\n' "current-stage validation blocked: query-set corpus or trace coverage insufficient"
      ;;
    *)
      printf '%s\n' "current-stage validation blocked: query-set prepare failed"
      ;;
  esac
}

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
handoff_summarizer="$script_dir/summarize-current-stage-validation.py"

write_current_stage_handoff() {
  source_json="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for current-stage handoff"
  [ -f "$handoff_summarizer" ] || fail "current-stage handoff summarizer is unavailable"
  case "$(basename "$source_json")" in
    current-stage-validation-evidence.json|current-stage-blocked-summary.json)
      python3 "$handoff_summarizer" \
        --input "$source_json" \
        --out "$out_dir/current-stage-handoff.json" \
        --issue-comment-out "$out_dir/current-stage-issue-comment.md" \
        >/dev/null || fail "current-stage handoff generation failed"
      ;;
    *)
      python3 "$handoff_summarizer" \
        --input "$source_json" \
        --out "$out_dir/current-stage-handoff.json" \
        >/dev/null || fail "current-stage handoff generation failed"
      ;;
  esac
}

corpus_summary_observability_json() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for redacted corpus summary observability"
python3 - "$path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    report = json.load(handle)

if report.get("privacy_boundary") != "redacted_local_aggregate":
    raise SystemExit("corpus summary privacy boundary failed")

for sentinel in (
    "contains_raw_resume_text",
    "contains_resume_paths",
    "contains_queries",
    "contains_sample_ids",
):
    if report.get(sentinel) is not False:
        raise SystemExit("corpus summary privacy sentinel failed")


def integer_field(name):
    value = report.get(name)
    if not isinstance(value, int) or value < 0:
        raise SystemExit(f"corpus summary field failed: {name}")
    return value


def boolean_field(name):
    value = report.get(name)
    if not isinstance(value, bool):
        raise SystemExit(f"corpus summary field failed: {name}")
    return value


def object_field(name):
    value = report.get(name, {})
    if not isinstance(value, dict):
        raise SystemExit(f"corpus summary field failed: {name}")
    return value


observability = {
    "privacy_boundary": "redacted_local_aggregate",
    "document_count": integer_field("document_count"),
    "searchable_document_count": integer_field("searchable_document_count"),
    "vector_indexed_document_count": integer_field("vector_indexed_document_count"),
    "hot_index_fully_covered": boolean_field("hot_index_fully_covered"),
    "document_status_counts": object_field("document_status_counts"),
    "ingest_job_status_counts": object_field("ingest_job_status_counts"),
    "ingest_job_kind_status_counts": object_field("ingest_job_kind_status_counts"),
    "ingest_job_failure_counts": object_field("ingest_job_failure_counts"),
    "contains_raw_resume_text": False,
    "contains_resume_paths": False,
    "contains_queries": False,
    "contains_sample_ids": False,
}

json.dump(observability, sys.stdout, ensure_ascii=True, sort_keys=True, indent=2)
sys.stdout.write("\n")
PY
}

private_query_observability_json() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for redacted private query observability"
python3 - "$path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    report = json.load(handle)

STAGE_LATENCY_FIELDS = (
    "query_parse",
    "prefilter",
    "bm25",
    "ann",
    "fusion",
    "bulk_hydrate",
    "snippet",
)

COPY_FIELDS = (
    "privacy_boundary",
    "dataset_kind",
    "document_count",
    "searchable_document_count",
    "vector_indexed_document_count",
    "query_count",
    "request_sample_count",
    "query_set_sha256",
    "tune_sha256",
    "holdout_sha256",
    "query_source",
    "private_scale_gate",
    "bucket_counts",
    "tune_bucket_counts",
    "holdout_bucket_counts",
    "samples_per_bucket",
    "query_latency_ms",
    "query_latency_by_bucket",
    "rss_delta_mb",
    "rss_delta_mb_by_bucket",
    "zero_result_queries",
    "query_runner",
    "query_mode",
    "retrieval_layers",
    "warm_or_cold_definition",
    "cache_state",
    "percentile_confidence",
    "spawn_per_query",
    "hot_index",
    "hot_path_ocr",
    "hot_path_parsing",
    "hot_path_heavy_model_inference",
    "contains_raw_resume_text",
    "contains_resume_paths",
    "contains_queries",
)


def field(mapping, name):
    try:
        return mapping[name]
    except KeyError as error:
        raise SystemExit(f"private query observability missing field: {name}") from error


def stage_p95(stages):
    return {
        stage: field(field(stages, stage), "p95")
        for stage in STAGE_LATENCY_FIELDS
    }


observability = {
    key: field(report, key)
    for key in COPY_FIELDS
}
observability["stage_latency_p95_ms"] = stage_p95(field(report, "stage_latency_ms"))
observability["stage_latency_by_bucket_p95_ms"] = {
    bucket: stage_p95(stages)
    for bucket, stages in field(report, "stage_latency_by_bucket_ms").items()
}
observability["stage_histogram_ms"] = field(report, "stage_histogram_ms")
observability["stage_histogram_by_bucket_ms"] = field(report, "stage_histogram_by_bucket_ms")

json.dump(observability, sys.stdout, ensure_ascii=True, sort_keys=True, indent=2)
sys.stdout.write("\n")
PY
}

private_benchmark_query_set_sha256() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for private benchmark digest extraction"
python3 - "$path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    report = json.load(handle)

digest = report.get("query_set_sha256")
if not isinstance(digest, str):
    raise SystemExit("private benchmark missing query_set_sha256")

print(digest.lower())
PY
}

corpus_summary_has_bounded_ocr_backlog() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for OCR backlog classification"
  set +e
  python3 - "$path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    report = json.load(handle)

if report.get("privacy_boundary") != "redacted_local_aggregate":
    raise SystemExit(2)

for sentinel in (
    "contains_raw_resume_text",
    "contains_resume_paths",
    "contains_queries",
    "contains_sample_ids",
):
    if report.get(sentinel) is not False:
        raise SystemExit(2)


def int_field(name):
    value = report.get(name)
    if not isinstance(value, int) or value < 0:
        raise SystemExit(2)
    return value


def dict_field(name):
    value = report.get(name, {})
    if not isinstance(value, dict):
        raise SystemExit(2)
    return value


def count_from(mapping, key):
    value = mapping.get(key, 0)
    if not isinstance(value, int) or value < 0:
        raise SystemExit(2)
    return value


hot_index_fully_covered = report.get("hot_index_fully_covered")
if not isinstance(hot_index_fully_covered, bool):
    raise SystemExit(2)

document_count = int_field("document_count")
searchable_document_count = int_field("searchable_document_count")
vector_indexed_document_count = int_field("vector_indexed_document_count")
document_status_counts = dict_field("document_status_counts")
ocr_required_count = count_from(document_status_counts, "ocr_required")

if (
    document_count > 0
    and ocr_required_count > 0
    and not hot_index_fully_covered
    and (
        searchable_document_count < document_count
        or vector_indexed_document_count < document_count
    )
):
    raise SystemExit(0)

raise SystemExit(1)
PY
  status=$?
  set -e
  if [ "$status" -eq 2 ]; then
    fail "current-stage corpus summary privacy/shape validation failed"
  fi
  [ "$status" -eq 0 ]
}

validate_redacted_diagnostics_report() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for redacted diagnostics validation"
  python3 - "$path" "$resume_root" "$data_dir" "$out_dir" <<'PY'
import json
import sys

path, resume_root, data_dir, out_dir = sys.argv[1:5]
with open(path, "r", encoding="utf-8") as handle:
    report = json.load(handle)

if not isinstance(report, dict):
    raise SystemExit("diagnostics report must be an object")


def require_string(mapping, key, expected):
    value = mapping.get(key)
    if value != expected:
        raise SystemExit(f"diagnostics report field failed: {key}")


def require_bool(mapping, key, expected):
    value = mapping.get(key)
    if value is not expected:
        raise SystemExit(f"diagnostics report field failed: {key}")


def require_optional_nested_string(mapping, parent, key, expected):
    if parent not in mapping:
        return
    value = mapping[parent]
    if not isinstance(value, dict):
        raise SystemExit(f"diagnostics report field failed: {parent}")
    require_string(value, key, expected)


require_string(report, "schema_version", "diagnostics.v1")
require_bool(report, "redacted", True)
require_string(report, "raw_paths", "<redacted>")
require_string(report, "raw_queries", "<redacted>")
require_string(report, "raw_resume_text", "<redacted>")
require_string(report, "evidence_level", "local_aggregate_only")
require_optional_nested_string(report, "resource_telemetry", "paths", "<redacted>")
require_optional_nested_string(report, "ocr_runtime", "paths", "<redacted>")
require_optional_nested_string(report, "query_latency", "raw_queries", "<redacted>")

scope = report.get("diagnostic_scope")
if not isinstance(scope, dict):
    raise SystemExit("diagnostics report scope missing")

for key, expected in (
    ("metadata", "aggregate_counts"),
    ("search_index", "state_and_snapshot_health"),
    ("vector_index", "state_backend_and_counts"),
    ("query_latency", "aggregate_observations"),
    ("runtime_dependencies", "presence_only"),
    ("fault_simulations", "available_cases_only"),
):
    require_string(scope, key, expected)


def walk_strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for child in value.values():
            yield from walk_strings(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk_strings(child)


for text in walk_strings(report):
    for marker in (resume_root, data_dir, out_dir, "PRIVATE-current-stage", "/Users/"):
        if marker and marker in text:
            raise SystemExit("diagnostics report leaked local marker")
PY
}

validate_fault_suite_report() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for fault-suite evidence validation"
  python3 scripts/ci/validate-current-stage-fault-suite.py --local-safe-suite "$path"
}

validate_private_benchmark_report() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for private benchmark evidence validation"
  python3 scripts/ci/validate-current-stage-private-benchmark.py \
    --private-benchmark "$path" \
    --validation-profile "$validation_profile"
}

validate_ocr_throughput_report() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for OCR throughput evidence validation"
  python3 scripts/ci/validate-current-stage-ocr-throughput.py --ocr-throughput "$path"
}

validate_dataset_manifest_report() {
  path="$1"
  command -v python3 >/dev/null 2>&1 || fail "python3 is required for dataset manifest validation"
  python3 - "$path" "$resume_root" "$data_dir" "$out_dir" <<'PY'
import json
import sys

path, resume_root, data_dir, out_dir = sys.argv[1:5]
with open(path, "r", encoding="utf-8") as handle:
    report = json.load(handle)

if not isinstance(report, dict):
    raise SystemExit("dataset manifest must be an object")
if report.get("schema_version") != "resume-ir.dataset-manifest.v1":
    raise SystemExit("dataset manifest schema failed")
if report.get("privacy_boundary") != "local_only_redacted_dataset_manifest":
    raise SystemExit("dataset manifest privacy boundary failed")
for key in (
    "contains_paths",
    "contains_file_names",
    "contains_raw_resume_text",
    "contains_file_hashes",
):
    if report.get(key) is not False:
        raise SystemExit(f"dataset manifest privacy sentinel failed: {key}")
fingerprint = report.get("corpus_fingerprint_sha256")
if not isinstance(fingerprint, str) or len(fingerprint) != 64:
    raise SystemExit("dataset manifest corpus fingerprint failed")
if any(ch not in "0123456789abcdefABCDEF" for ch in fingerprint):
    raise SystemExit("dataset manifest corpus fingerprint failed")

def walk_strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for child in value.values():
            yield from walk_strings(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk_strings(child)

for text in walk_strings(report):
    for marker in (resume_root, data_dir, out_dir, "PRIVATE-current-stage", "/Users/"):
        if marker and marker in text:
            raise SystemExit("dataset manifest leaked local marker")
PY
}

validate_no_private_markers_in_file() {
  path="$1"
  context="$2"
  for marker in "$resume_root" "$data_dir" "$out_dir" "PRIVATE-current-stage" "/Users/"; do
    if [ -n "$marker" ] && grep -Fq -- "$marker" "$path"; then
      printf '%s\n' "$context leaked local marker" >&2
      return 1
    fi
  done
  return 0
}

write_query_set_trace_preflight() {
  [ -n "$query_set_trace_root" ] || return 0
  set +e
  RESUME_IR_LOCAL_EVIDENCE_DIR="$out_dir" \
    RESUME_IR_QUERY_ARTIFACT_ROOT="$query_set_trace_root" \
    "$resume_cli" --data-dir "$data_dir" benchmark-query-set preflight-agent-replay \
    --max-queries "$max_queries" \
    > "$out_dir/query-set-trace-preflight.stdout.txt" \
    2> "$out_dir/query-set-trace-preflight.stderr.txt"
  preflight_status=$?
  set -e
  if [ "$preflight_status" -ne 0 ]; then
    rm -f "$out_dir/query-set-trace-preflight.local.json"
    return 0
  fi
  if ! validate_no_private_markers_in_file "$out_dir/query-set-trace-preflight.local.json" "query set trace preflight"; then
    rm -f "$out_dir/query-set-trace-preflight.local.json"
    return 0
  fi
  return 0
}

query_set_trace_preflight_json() {
  path="$out_dir/query-set-trace-preflight.local.json"
  if [ ! -s "$path" ]; then
    printf 'null'
    return
  fi
  python3 - "$path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as handle:
    report = json.load(handle)
if report.get("schema_version") != "resume-ir.query-set-trace-preflight.v1":
    raise SystemExit("query set trace preflight schema invalid")
if report.get("privacy_boundary") != "redacted_local_aggregate":
    raise SystemExit("query set trace preflight privacy boundary invalid")
for flag in (
    "contains_raw_query_text",
    "contains_raw_resume_text",
    "contains_candidate_results",
    "contains_local_paths",
):
    if report.get(flag) is not False:
        raise SystemExit(f"query set trace preflight {flag} must be false")
int_fields = (
    "target_query_count",
    "document_count",
    "searchable_document_count",
    "vector_indexed_document_count",
    "d10k_min_document_count",
    "d10k_min_searchable_document_count",
    "d10k_min_vector_indexed_document_count",
    "trace_logs",
    "trace_lines",
    "source_search_lines",
    "extracted_queries",
    "normalization_rejected",
    "duplicate_queries_dropped",
    "candidate_queries_sampled",
    "zero_hit_queries_dropped",
    "corpus_valid_queries",
)
object_fields = (
    "d10k_corpus_deficits",
    "candidate_bucket_counts",
    "candidate_bucket_deficits",
    "corpus_valid_bucket_counts",
    "required_bucket_counts",
    "corpus_valid_bucket_deficits",
)
out = {
    "schema_version": "resume-ir.query-set-trace-preflight.v1",
    "query_source": report.get("query_source"),
}
query_index_available = report.get("query_index_available")
if not isinstance(query_index_available, bool):
    raise SystemExit("query set trace preflight query_index_available invalid")
out["query_index_available"] = query_index_available
d10k_corpus_ready = report.get("d10k_corpus_ready")
if not isinstance(d10k_corpus_ready, bool):
    raise SystemExit("query set trace preflight d10k_corpus_ready invalid")
out["d10k_corpus_ready"] = d10k_corpus_ready
for field in int_fields:
    value = report.get(field)
    if not isinstance(value, int) or value < 0:
        raise SystemExit(f"query set trace preflight {field} invalid")
    out[field] = value
for field in object_fields:
    value = report.get(field)
    if not isinstance(value, dict) or not value:
        raise SystemExit(f"query set trace preflight {field} invalid")
    checked = {}
    for key, item in value.items():
        if not isinstance(key, str) or not key or not isinstance(item, int) or item < 0:
            raise SystemExit(f"query set trace preflight {field} entry invalid")
        checked[key] = item
    out[field] = checked
print(json.dumps(out, sort_keys=True))
PY
}

status_count_or_empty() {
  path="$1"
  label="$2"
  awk -F': ' -v label="$label" '$1 == label { print $2; exit }' "$path"
}

write_runtime_preflight_blocked_summary() {
  blocked_step="$1"
  blocked_category="$2"
  blocked_reason="$3"
  blocked_exit="$4"

  [ -e "$out_dir/ocr-preflight.json" ] || : > "$out_dir/ocr-preflight.json"

  ocr_preflight_sha256_json=$(sha256_file_json_or_null "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256_json=$(sha256_file_json_or_null "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256_json=$(sha256_file_json_or_null "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256_json=$(sha256_file_json_or_null "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256_json=$(sha256_file_json_or_null "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256_json=$(sha256_file_json_or_null "$out_dir/model-preflight.json")
  ocr_runtime_manifest_sha256_json=$(sha256_file_json_or_null "$ocr_runtime_manifest")
  model_manifest_sha256_json=$(sha256_file_json_or_null "$model_manifest")

  case "$blocked_step" in
    ocr_preflight)
      ocr_probe_status="blocked"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    ocr_manifest_draft)
      ocr_probe_status="passed"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    ocr_manifest_validate)
      ocr_probe_status="passed"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    model_manifest_draft)
      ocr_probe_status="passed"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    model_manifest_validate)
      ocr_probe_status="passed"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    model_preflight)
      ocr_probe_status="passed"
      embedding_protocol_status="blocked"
      steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
    *)
      ocr_probe_status="blocked"
      embedding_protocol_status="not_run"
      steps_json=$(cat <<EOF_STEPS
    {"id": "$blocked_step", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
      ;;
  esac

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": false,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "$blocked_step",
  "blocked_category": "$blocked_category",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": null,
    "query_set_sha256": null,
    "model_manifest_sha256": $model_manifest_sha256_json,
    "ocr_runtime_manifest_sha256": $ocr_runtime_manifest_sha256_json
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "$ocr_probe_status",
    "embedding_protocol": "$embedding_protocol_status"
  },
  "steps": [
$steps_json
  ],
  "redacted_outputs": [
    {"file": "ocr-preflight.json", "sha256": $ocr_preflight_sha256_json},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": $ocr_draft_stdout_sha256_json},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": $ocr_validate_stdout_sha256_json},
    {"file": "ocr-runtime-manifest.local.json", "sha256": $ocr_runtime_manifest_sha256_json},
    {"file": "model-draft-manifest.stdout.txt", "sha256": $model_draft_stdout_sha256_json},
    {"file": "model-validate-manifest.stdout.txt", "sha256": $model_validate_stdout_sha256_json},
    {"file": "model-manifest.local.json", "sha256": $model_manifest_sha256_json},
    {"file": "model-preflight.json", "sha256": $model_preflight_sha256_json}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "private corpus read",
    "runtime preflight",
    "dataset manifest",
    "import/OCR/atomic search publication",
    "current-stage validation evidence",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_import_parser_blocked_summary() {
  blocked_step="$1"
  blocked_reason="$2"
  blocked_exit="$3"

  [ -e "$out_dir/dataset-manifest.stdout.txt" ] || : > "$out_dir/dataset-manifest.stdout.txt"
  [ -e "$out_dir/import.stdout.txt" ] || : > "$out_dir/import.stdout.txt"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  dataset_manifest_sha256_json=$(sha256_file_json_or_null "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")

  private_corpus_read="true"
  if [ "$reuse_imported_corpus" = "true" ]; then
    private_corpus_read="false"
  fi

  if [ "$blocked_step" = "dataset_manifest" ]; then
    steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
  else
    steps_json=$(cat <<EOF_STEPS
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
  fi

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": $private_corpus_read,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "$blocked_step",
  "blocked_category": "import/parser",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": $dataset_manifest_sha256_json,
    "query_set_sha256": null,
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "steps": [
$steps_json
  ],
  "redacted_outputs": [
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "dataset-manifest.local.json", "sha256": $dataset_manifest_sha256_json},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "successful private corpus import",
    "OCR worker bounded run",
    "atomic search publication run",
    "corpus summary",
    "query-set prepare",
    "private query baseline",
    "redacted diagnostics",
    "release-readiness current-stage evidence",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_query_set_blocked_summary() {
  blocked_exit="$1"
  blocked_reason="$2"
  [ -e "$out_dir/query-set-prepare.stderr.txt" ] || : > "$out_dir/query-set-prepare.stderr.txt"
  [ -e "$out_dir/query-set-prepare.stdout.txt" ] || {
    printf '%s\n' "query set: blocked"
    printf '%s\n' "schema: resume-ir.query-set.jsonl.v2"
    printf '%s\n' "privacy boundary: local_only_private_query_set"
    printf '%s\n' "queries: <redacted>"
    printf '%s\n' "paths: <redacted>"
  } > "$out_dir/query-set-prepare.stdout.txt"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_prepare_stdout_sha256=$(sha256_file "$out_dir/query-set-prepare.stdout.txt")
  query_set_prepare_stderr_sha256=$(sha256_file "$out_dir/query-set-prepare.stderr.txt")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")
  query_set_trace_preflight_json_output=$(query_set_trace_preflight_json)
  query_set_trace_preflight_redacted_output=""
  if [ -e "$out_dir/query-set-trace-preflight.local.json" ]; then
    query_set_trace_preflight_sha256=$(sha256_file "$out_dir/query-set-trace-preflight.local.json")
    query_set_trace_preflight_redacted_output=$(cat <<EOF_PREFLIGHT_OUTPUT
    {"file": "query-set-trace-preflight.local.json", "sha256": "$query_set_trace_preflight_sha256"},
EOF_PREFLIGHT_OUTPUT
)
  fi

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "query_set_prepare",
  "blocked_category": "query-set",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": null,
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "query_set_trace_preflight": $query_set_trace_preflight_json_output,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "blocked", "exit_code": $blocked_exit}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
${query_set_trace_preflight_redacted_output}
    {"file": "query-set-prepare.stdout.txt", "sha256": "$query_set_prepare_stdout_sha256"},
    {"file": "query-set-prepare.stderr.txt", "sha256": "$query_set_prepare_stderr_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "local private query-set generation",
    "private query baseline",
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "redacted diagnostics for this blocked run",
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_private_query_blocked_summary() {
  blocked_exit="$1"
  blocked_reason="$2"
  [ -e "$out_dir/private-benchmark-local.json" ] || : > "$out_dir/private-benchmark-local.json"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_prepare_stdout_sha256=$(sha256_file "$out_dir/query-set-prepare.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "private_query_baseline",
  "blocked_category": "benchmark",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "success"},
    {"id": "private_query_baseline", "status": "blocked", "exit_code": $blocked_exit}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "private-query-set.summary.json", "sha256": "$query_set_summary_sha256_output"},
    {"file": "query-set-prepare.stdout.txt", "sha256": "$query_set_prepare_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "private query baseline",
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "redacted diagnostics for this blocked run",
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_ocr_throughput_blocked_summary() {
  blocked_step="$1"
  blocked_reason="$2"
  blocked_exit="$3"
  [ -e "$out_dir/private-ocr-throughput.json" ] || : > "$out_dir/private-ocr-throughput.json"
  [ -e "$out_dir/ocr-throughput-gate.stdout.txt" ] || : > "$out_dir/ocr-throughput-gate.stdout.txt"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_prepare_stdout_sha256=$(sha256_file "$out_dir/query-set-prepare.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  private_ocr_throughput_sha256=$(sha256_file "$out_dir/private-ocr-throughput.json")
  ocr_throughput_gate_sha256=$(sha256_file "$out_dir/ocr-throughput-gate.stdout.txt")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  if [ "$blocked_step" = "private_ocr_throughput_baseline" ]; then
    ocr_throughput_steps=$(cat <<EOF_STEPS
    {"id": "private_ocr_throughput_baseline", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
  else
    ocr_throughput_steps=$(cat <<EOF_STEPS
    {"id": "private_ocr_throughput_baseline", "status": "success"},
    {"id": "ocr_throughput_baseline_gate", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
  fi

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "$blocked_step",
  "blocked_category": "ocr",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries,
    "ocr_throughput_min_pages": $ocr_throughput_min_pages
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
$ocr_throughput_steps
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "private-query-set.summary.json", "sha256": "$query_set_summary_sha256_output"},
    {"file": "query-set-prepare.stdout.txt", "sha256": "$query_set_prepare_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "private-ocr-throughput.json", "sha256": "$private_ocr_throughput_sha256"},
    {"file": "ocr-throughput-gate.stdout.txt", "sha256": "$ocr_throughput_gate_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "private real-corpus OCR throughput baseline",
    "redacted diagnostics for this blocked run",
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_ocr_backlog_blocked_summary() {
  redacted_diagnostics_exit="${1:-0}"
  blocked_exit=1
  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")
  redacted_diagnostics_sha256_json=$(sha256_file_json_or_null "$out_dir/redacted-diagnostics.json")
  doctor_sha256_json=$(sha256_file_json_or_null "$out_dir/doctor.out")

  if [ "$redacted_diagnostics_exit" -eq 0 ]; then
    redacted_diagnostics_step_json='    {"id": "redacted_diagnostics", "status": "success"}'
    redacted_diagnostics_not_completed=''
    doctor_step_json='    {"id": "doctor", "status": "success"}'
    doctor_not_completed=''
  else
    redacted_diagnostics_step_json=$(cat <<EOF_DIAGNOSTICS_STEP
    {"id": "redacted_diagnostics", "status": "blocked", "exit_code": $redacted_diagnostics_exit}
EOF_DIAGNOSTICS_STEP
)
    redacted_diagnostics_not_completed='    "redacted diagnostics for this blocked run",'
    doctor_step_json='    {"id": "doctor", "status": "not_run"}'
    doctor_not_completed='    "doctor diagnostics for this blocked run",'
  fi

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "ocr_search_publication_bounded_loop",
  "blocked_category": "ocr",
  "blocked_reason": "ocr_backlog_exceeds_current_stage_budget",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": null,
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "blocked", "exit_code": $blocked_exit},
    {"id": "corpus_summary", "status": "success"},
$redacted_diagnostics_step_json,
$doctor_step_json
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "redacted-diagnostics.json", "sha256": $redacted_diagnostics_sha256_json},
    {"file": "doctor.out", "sha256": $doctor_sha256_json}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "full OCR backlog drain",
    "private query-set prepare",
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "private real-corpus OCR throughput baseline",
$redacted_diagnostics_not_completed
$doctor_not_completed
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_redacted_diagnostics_blocked_summary() {
  blocked_exit="$1"
  blocked_reason="$2"
  [ -e "$out_dir/redacted-diagnostics.json" ] || : > "$out_dir/redacted-diagnostics.json"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_prepare_stdout_sha256=$(sha256_file "$out_dir/query-set-prepare.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  private_ocr_throughput_sha256=$(sha256_file "$out_dir/private-ocr-throughput.json")
  ocr_throughput_gate_sha256=$(sha256_file "$out_dir/ocr-throughput-gate.stdout.txt")
  redacted_diagnostics_sha256=$(sha256_file "$out_dir/redacted-diagnostics.json")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "redacted_diagnostics",
  "blocked_category": "diagnostics",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
    {"id": "private_ocr_throughput_baseline", "status": "success"},
    {"id": "ocr_throughput_baseline_gate", "status": "success"},
    {"id": "redacted_diagnostics", "status": "blocked", "exit_code": $blocked_exit}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "private-query-set.summary.json", "sha256": "$query_set_summary_sha256_output"},
    {"file": "query-set-prepare.stdout.txt", "sha256": "$query_set_prepare_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "private-ocr-throughput.json", "sha256": "$private_ocr_throughput_sha256"},
    {"file": "ocr-throughput-gate.stdout.txt", "sha256": "$ocr_throughput_gate_sha256"},
    {"file": "redacted-diagnostics.json", "sha256": "$redacted_diagnostics_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "redacted diagnostics for this run",
    "release-readiness current-stage evidence",
    "full 10k/8000-document current-stage baseline",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_release_readiness_blocked_summary() {
  blocked_exit="$1"
  blocked_reason="$2"
  [ -e "$out_dir/release-readiness.json" ] || : > "$out_dir/release-readiness.json"
  [ -e "$out_dir/release-readiness.stderr.txt" ] || : > "$out_dir/release-readiness.stderr.txt"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_prepare_stdout_sha256=$(sha256_file "$out_dir/query-set-prepare.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  private_ocr_throughput_sha256=$(sha256_file "$out_dir/private-ocr-throughput.json")
  ocr_throughput_gate_sha256=$(sha256_file "$out_dir/ocr-throughput-gate.stdout.txt")
  redacted_diagnostics_sha256=$(sha256_file "$out_dir/redacted-diagnostics.json")
  doctor_sha256=$(sha256_file "$out_dir/doctor.out")
  fault_simulation_sha256=$(sha256_file "$out_dir/fault-simulation-storage-low.json")
  fault_simulation_suite_sha256=$(sha256_file "$out_dir/fault-simulation-suite-local-safe.json")
  release_readiness_sha256=$(sha256_file "$out_dir/release-readiness.json")
  release_readiness_stderr_sha256=$(sha256_file "$out_dir/release-readiness.stderr.txt")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": true,
  "full_baseline_satisfied": true,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "release_readiness_intake",
  "blocked_category": "release-readiness",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
    {"id": "private_ocr_throughput_baseline", "status": "success"},
    {"id": "ocr_throughput_baseline_gate", "status": "success"},
    {"id": "redacted_diagnostics", "status": "success"},
    {"id": "doctor", "status": "success"},
    {"id": "fault_simulation_smoke", "status": "success"},
    {"id": "fault_simulation_suite", "status": "success"},
    {"id": "release_readiness_intake", "status": "blocked", "exit_code": $blocked_exit}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "private-query-set.summary.json", "sha256": "$query_set_summary_sha256_output"},
    {"file": "query-set-prepare.stdout.txt", "sha256": "$query_set_prepare_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "private-ocr-throughput.json", "sha256": "$private_ocr_throughput_sha256"},
    {"file": "ocr-throughput-gate.stdout.txt", "sha256": "$ocr_throughput_gate_sha256"},
    {"file": "redacted-diagnostics.json", "sha256": "$redacted_diagnostics_sha256"},
    {"file": "doctor.out", "sha256": "$doctor_sha256"},
    {"file": "fault-simulation-storage-low.json", "sha256": "$fault_simulation_sha256"},
    {"file": "fault-simulation-suite-local-safe.json", "sha256": "$fault_simulation_suite_sha256"},
    {"file": "release-readiness.json", "sha256": "$release_readiness_sha256"},
    {"file": "release-readiness.stderr.txt", "sha256": "$release_readiness_stderr_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "accepted release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

write_fault_simulation_blocked_summary() {
  blocked_exit="$1"
  blocked_reason="$2"
  [ -e "$out_dir/fault-simulation-storage-low.json" ] || : > "$out_dir/fault-simulation-storage-low.json"
  [ -e "$out_dir/fault-simulation-suite-local-safe.json" ] || : > "$out_dir/fault-simulation-suite-local-safe.json"

  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_prepare_stdout_sha256=$(sha256_file "$out_dir/query-set-prepare.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  private_ocr_throughput_sha256=$(sha256_file_json_or_null "$out_dir/private-ocr-throughput.json")
  ocr_throughput_gate_sha256=$(sha256_file_json_or_null "$out_dir/ocr-throughput-gate.stdout.txt")
  redacted_diagnostics_sha256=$(sha256_file "$out_dir/redacted-diagnostics.json")
  fault_simulation_sha256=$(sha256_file "$out_dir/fault-simulation-storage-low.json")
  fault_simulation_suite_sha256=$(sha256_file "$out_dir/fault-simulation-suite-local-safe.json")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  if [ "$validation_profile" = "full" ]; then
    fault_full_baseline_satisfied="true"
    fault_prior_throughput_steps=$(cat <<EOF_STEPS
    {"id": "private_ocr_throughput_baseline", "status": "success"},
    {"id": "ocr_throughput_baseline_gate", "status": "success"},
EOF_STEPS
)
    fault_prior_throughput_outputs=$(cat <<EOF_OUTPUTS
    {"file": "private-ocr-throughput.json", "sha256": $private_ocr_throughput_sha256},
    {"file": "ocr-throughput-gate.stdout.txt", "sha256": $ocr_throughput_gate_sha256},
EOF_OUTPUTS
)
  else
    fault_full_baseline_satisfied="false"
    fault_prior_throughput_steps=""
    fault_prior_throughput_outputs=""
  fi

  if [ "$blocked_reason" = "fault_simulation_suite_failed" ] || [ "$blocked_reason" = "fault_simulation_suite_invalid" ]; then
    fault_blocked_step="fault_simulation_suite"
    fault_step_statuses=$(cat <<EOF_STEPS
    {"id": "fault_simulation_smoke", "status": "success"},
    {"id": "fault_simulation_suite", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
    fault_not_completed='"current-stage local-safe fault simulation suite",'
  else
    fault_blocked_step="fault_simulation_smoke"
    fault_step_statuses=$(cat <<EOF_STEPS
    {"id": "fault_simulation_smoke", "status": "blocked", "exit_code": $blocked_exit},
    {"id": "fault_simulation_suite", "status": "blocked", "exit_code": $blocked_exit}
EOF_STEPS
)
    fault_not_completed='"current-stage fault simulation smoke",'
  fi

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": true,
  "full_baseline_satisfied": $fault_full_baseline_satisfied,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "$fault_blocked_step",
  "blocked_category": "fault-injection",
  "blocked_reason": "$blocked_reason",
  "blocked_exit": $blocked_exit,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
$fault_prior_throughput_steps
    {"id": "redacted_diagnostics", "status": "success"},
$fault_step_statuses
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "private-query-set.summary.json", "sha256": "$query_set_summary_sha256_output"},
    {"file": "query-set-prepare.stdout.txt", "sha256": "$query_set_prepare_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
$fault_prior_throughput_outputs
    {"file": "redacted-diagnostics.json", "sha256": "$redacted_diagnostics_sha256"},
    {"file": "fault-simulation-storage-low.json", "sha256": "$fault_simulation_sha256"},
    {"file": "fault-simulation-suite-local-safe.json", "sha256": "$fault_simulation_suite_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    $fault_not_completed
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
}

require_text_in_file() {
  path="$1"
  text="$2"
  message="$3"
  if ! grep -Fq -- "$text" "$path"; then
    fail "$message"
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
query_set_trace_root=""
validation_profile="full"
model_manifest=""
ocr_runtime_manifest=""
model_artifact=""
embedding_command=""
embedding_runtime_bin_dir=""
embedding_runtime_bin_dir_configured="false"
model_pack_id=""
model_id=""
model_format=""
dimension=""
model_license=""
runtime_pack_id=""
runtime_distribution_mode="bundled"
runtime_package_binaries_included="true"
tesseract_command=""
pdftoppm_command=""
language=""
language_pack_args=""
language_pack_count=0
engine_license=""
renderer_license=""
language_license=""
dataset_manifest_sha256=""
query_set_sha256=""
reuse_imported_corpus="false"
reuse_dataset_manifest=""
model_manifest_sha256=""
ocr_runtime_manifest_sha256=""
renderer_manifest_sha256=""
language_pack_manifest_sha256=""
reviewed_model="false"
reviewed_ocr_runtime="false"
max_files="10000"
max_queries="500"
private_query_request_sample_count=""
top_k="10"
private_query_timeout_ms="30000"
worker_interval_ms="1"
ocr_worker_ticks="10000"
ocr_jobs_per_tick="1"
ocr_max_pages_per_document="20"
ocr_page_timeout_ms="30000"
ocr_render_dpi="150"
embedding_timeout_ms="30000"
ocr_throughput_max_documents="900"
ocr_throughput_max_pages="500"
ocr_throughput_pages_per_document="1"
ocr_throughput_max_run_ms="3600000"
ocr_throughput_min_pages="500"

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
    --query-set-trace-root)
      need_value "$@"; query_set_trace_root="$2"; shift 2
      ;;
    --validation-profile)
      need_value "$@"; validation_profile="$2"; shift 2
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
    --embedding-runtime-bin-dir)
      need_value "$@"; embedding_runtime_bin_dir="$2"; shift 2
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
    --runtime-distribution-mode)
      need_value "$@"; runtime_distribution_mode="$2"; shift 2
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
      need_value "$@"
      if [ "$language_pack_count" -eq 0 ]; then
        language_pack_args="$2"
      else
        language_pack_args="$language_pack_args
$2"
      fi
      language_pack_count=$((language_pack_count + 1))
      shift 2
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
    --reuse-imported-corpus)
      reuse_imported_corpus="true"
      shift
      ;;
    --reuse-dataset-manifest)
      need_value "$@"; reuse_dataset_manifest="$2"; shift 2
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
    --private-query-request-sample-count)
      need_value "$@"; private_query_request_sample_count="$2"; shift 2
      ;;
    --private-query-timeout-ms)
      need_value "$@"; private_query_timeout_ms="$2"; shift 2
      ;;
    --worker-interval-ms)
      need_value "$@"; worker_interval_ms="$2"; shift 2
      ;;
    --ocr-worker-ticks)
      need_value "$@"; ocr_worker_ticks="$2"; shift 2
      ;;
    --ocr-jobs-per-tick)
      need_value "$@"; ocr_jobs_per_tick="$2"; shift 2
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
    --embedding-timeout-ms)
      need_value "$@"; embedding_timeout_ms="$2"; shift 2
      ;;
    --ocr-throughput-max-documents)
      need_value "$@"; ocr_throughput_max_documents="$2"; shift 2
      ;;
    --ocr-throughput-max-pages)
      need_value "$@"; ocr_throughput_max_pages="$2"; shift 2
      ;;
    --ocr-throughput-pages-per-document)
      need_value "$@"; ocr_throughput_pages_per_document="$2"; shift 2
      ;;
    --ocr-throughput-max-run-ms)
      need_value "$@"; ocr_throughput_max_run_ms="$2"; shift 2
      ;;
    --ocr-throughput-min-pages)
      need_value "$@"; ocr_throughput_min_pages="$2"; shift 2
      ;;
    -h|--help)
      usage
      ;;
    *)
      usage
      ;;
  esac
done

if [ -z "$resume_root" ] && [ -n "${RESUME_IR_PRIVATE_RESUME_ROOT:-}" ]; then
  resume_root="$RESUME_IR_PRIVATE_RESUME_ROOT"
fi
if [ -z "$data_dir" ] && [ -n "${RESUME_IR_DATA_DIR:-}" ]; then
  data_dir="$RESUME_IR_DATA_DIR"
fi
if [ -z "$out_dir" ] && [ -n "${RESUME_IR_LOCAL_EVIDENCE_DIR:-}" ]; then
  out_dir="$RESUME_IR_LOCAL_EVIDENCE_DIR"
fi
require_arg "--resume-root" "$resume_root"
require_arg "--data-dir" "$data_dir"
require_arg "--out-dir" "$out_dir"
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
tesseract_command=$(detect_command_path "tesseract" "$tesseract_command")
pdftoppm_command=$(detect_command_path "pdftoppm" "$pdftoppm_command")
require_arg "--language" "$language"
[ "$language_pack_count" -gt 0 ] || fail "missing required argument: --language-pack"
require_arg "--engine-license" "$engine_license"
require_arg "--renderer-license" "$renderer_license"
require_arg "--language-license" "$language_license"
require_positive_int "--dimension" "$dimension"
require_positive_int "--max-files" "$max_files"
require_positive_int "--max-queries" "$max_queries"
require_positive_int "--top-k" "$top_k"
if [ -n "$private_query_request_sample_count" ]; then
  require_positive_int "--private-query-request-sample-count" "$private_query_request_sample_count"
fi
require_positive_int "--private-query-timeout-ms" "$private_query_timeout_ms"
require_positive_int "--worker-interval-ms" "$worker_interval_ms"
require_positive_int "--ocr-worker-ticks" "$ocr_worker_ticks"
require_positive_int "--ocr-jobs-per-tick" "$ocr_jobs_per_tick"
require_positive_int "--ocr-max-pages-per-document" "$ocr_max_pages_per_document"
require_positive_int "--ocr-page-timeout-ms" "$ocr_page_timeout_ms"
require_positive_int "--ocr-render-dpi" "$ocr_render_dpi"
require_positive_int "--embedding-timeout-ms" "$embedding_timeout_ms"
require_positive_int "--ocr-throughput-max-documents" "$ocr_throughput_max_documents"
require_positive_int "--ocr-throughput-max-pages" "$ocr_throughput_max_pages"
require_positive_int "--ocr-throughput-pages-per-document" "$ocr_throughput_pages_per_document"
require_positive_int "--ocr-throughput-max-run-ms" "$ocr_throughput_max_run_ms"
require_positive_int "--ocr-throughput-min-pages" "$ocr_throughput_min_pages"
[ -z "$dataset_manifest_sha256" ] || require_sha256 "--dataset-manifest-sha256" "$dataset_manifest_sha256"
if [ -n "$query_set" ] && [ -n "$query_set_trace_root" ]; then
  fail "--query-set-trace-root cannot be combined with --query-set"
fi
if [ "$reuse_imported_corpus" = "true" ]; then
  require_arg "--reuse-dataset-manifest" "$reuse_dataset_manifest"
elif [ -n "$reuse_dataset_manifest" ]; then
  fail "--reuse-dataset-manifest requires --reuse-imported-corpus"
fi
[ -z "$model_manifest_sha256" ] || require_sha256 "--model-manifest-sha256" "$model_manifest_sha256"
[ -z "$ocr_runtime_manifest_sha256" ] || require_sha256 "--ocr-runtime-manifest-sha256" "$ocr_runtime_manifest_sha256"
[ -z "$renderer_manifest_sha256" ] || require_sha256 "--renderer-manifest-sha256" "$renderer_manifest_sha256"
[ -z "$language_pack_manifest_sha256" ] || require_sha256 "--language-pack-manifest-sha256" "$language_pack_manifest_sha256"
if [ -n "$embedding_runtime_bin_dir" ]; then
  embedding_runtime_bin_dir_configured="true"
fi
case "$validation_profile" in
  full)
    current_stage_target="reproducible_local_10k_baseline"
    query_set_min_queries="$max_queries"
    baseline_min_documents="10000"
    baseline_min_queries="500"
    if [ -z "$private_query_request_sample_count" ]; then
      private_query_request_sample_count="5000"
    fi
    benchmark_gate_smoke_arg=""
    benchmark_gate_smoke_plan=""
    private_query_partial_hot_index_arg=""
    private_query_partial_hot_index_plan=""
    full_baseline_satisfied="false"
    release_readiness_evidence="true"
    ocr_throughput_plan_steps=$(cat <<EOF_STEPS
    {
      "id": "private_ocr_throughput_baseline",
      "command": "resume-benchmark private-ocr-throughput --root <local-resume-root> --pdftoppm-command <local-pdftoppm-command> --tesseract-command <local-tesseract-command> --max-documents $ocr_throughput_max_documents --max-pages $ocr_throughput_max_pages --pages-per-document $ocr_throughput_pages_per_document --page-timeout-ms $ocr_page_timeout_ms --max-run-ms $ocr_throughput_max_run_ms --render-dpi $ocr_render_dpi --ocr-lang <ocr-language> --dataset-manifest-sha256 <dataset-manifest-sha256> --ocr-runtime-manifest-sha256 <ocr-runtime-manifest-sha256> --renderer-manifest-sha256 <renderer-manifest-sha256> --language-pack-manifest-sha256 <language-pack-manifest-sha256> --json > <local-evidence-dir>/private-ocr-throughput.json"
    },
    {
      "id": "ocr_throughput_baseline_gate",
      "command": "resume-benchmark ocr-gate --report <local-evidence-dir>/private-ocr-throughput.json --current-stage-baseline --require-private-real-corpus --min-pages $ocr_throughput_min_pages"
    },
EOF_STEPS
)
    terminal_plan_steps='    {
      "id": "release_readiness_intake",
      "command": "resume-cli --data-dir <local-data-dir> release-readiness --json --benchmark-report <local-evidence-dir>/private-benchmark-local.json --ocr-throughput-report <local-evidence-dir>/private-ocr-throughput.json --model-manifest <local-model-manifest> --ocr-runtime-manifest <local-ocr-runtime-manifest> --diagnostics-report <local-evidence-dir>/redacted-diagnostics.json > <local-evidence-dir>/release-readiness.json"
    },
    {
      "id": "redacted_evidence_manifest",
      "command": "write <local-evidence-dir>/current-stage-validation-evidence.json with schema resume-ir.current-stage-validation-evidence.v2, file digests, step statuses, and privacy sentinels"
    },
    {
      "id": "current_stage_handoff",
      "command": "write <local-evidence-dir>/current-stage-handoff.json with schema resume-ir.current-stage-handoff.v1 from redacted current-stage evidence"
    },
    {
      "id": "redacted_issue_comment_body",
      "command": "write <local-evidence-dir>/current-stage-issue-comment.md from current-stage-handoff.json for redacted #53 comment drafting"
    }'
    ;;
  smoke)
    current_stage_target="local_real_corpus_smoke_chain"
    query_set_min_queries="1"
    baseline_min_documents="1"
    baseline_min_queries="1"
    if [ -z "$private_query_request_sample_count" ]; then
      private_query_request_sample_count="$max_queries"
    fi
    benchmark_gate_smoke_arg="--allow-smoke-confidence"
    benchmark_gate_smoke_plan=" --allow-smoke-confidence"
    private_query_partial_hot_index_arg="--allow-partial-hot-index-for-smoke"
    private_query_partial_hot_index_plan=" --allow-partial-hot-index-for-smoke"
    full_baseline_satisfied="false"
    release_readiness_evidence="false"
    ocr_throughput_plan_steps=""
    terminal_plan_steps='    {
      "id": "redacted_smoke_summary",
      "command": "write <local-evidence-dir>/current-stage-smoke-summary.json with schema resume-ir.current-stage-smoke-summary.v2, file digests, step statuses, and explicit non-release-evidence blockers"
    },
    {
      "id": "current_stage_handoff",
      "command": "write <local-evidence-dir>/current-stage-handoff.json with schema resume-ir.current-stage-handoff.v1 from redacted smoke summary"
    }'
    ;;
  *)
    fail "--validation-profile must be full or smoke"
    ;;
esac
if [ -z "$query_set" ] && [ -z "$query_set_trace_root" ] && [ -n "${RESUME_IR_QUERY_ARTIFACT_ROOT:-}" ]; then
  query_set_trace_root="$RESUME_IR_QUERY_ARTIFACT_ROOT"
fi
if [ -z "$query_set" ] && [ -z "$query_set_trace_root" ]; then
  fail "current-stage validation requires --query-set-trace-root or --query-set"
fi
require_positive_int "--private-query-request-sample-count" "$private_query_request_sample_count"

case "$runtime_distribution_mode" in
  bundled)
    runtime_package_binaries_included="true"
    ;;
  external)
    runtime_package_binaries_included="false"
    ;;
  *)
    fail "--runtime-distribution-mode must be bundled or external"
    ;;
esac

if [ "$reuse_imported_corpus" = "true" ]; then
  dataset_manifest_plan_command="copy <local-redacted-dataset-manifest> to <local-evidence-dir>/dataset-manifest.local.json and validate resume-ir.dataset-manifest.v1 without reading <local-resume-root>"
  import_private_corpus_plan_command="resume-cli --data-dir <local-data-dir> status > <local-evidence-dir>/import.stdout.txt # reuse existing imported corpus; do not rescan private root"
else
  dataset_manifest_plan_command="resume-cli --data-dir <local-data-dir> privacy dataset-manifest --root <local-resume-root> --out <local-evidence-dir>/dataset-manifest.local.json --profile explicit --max-files $max_files"
  import_private_corpus_plan_command="resume-cli --data-dir <local-data-dir> import --root <local-resume-root> --profile explicit --max-files $max_files"
fi
if [ -n "$query_set" ]; then
  query_set_plan_command="copy <local-query-set> and its paired summary to <local-evidence-dir>/private-query-set.local.jsonl, then validate resume-ir.query-set.jsonl.v2 and resume-ir.query-set-summary.v2, and write <local-evidence-dir>/query-set-prepare.stdout.txt with query_source, query_count, query_set_sha256, tune_sha256, holdout_sha256, and redacted query/path markers"
elif [ -n "$query_set_trace_root" ]; then
  query_set_plan_command="RESUME_IR_LOCAL_EVIDENCE_DIR=<local-evidence-dir> RESUME_IR_QUERY_ARTIFACT_ROOT=\$RESUME_IR_QUERY_ARTIFACT_ROOT resume-cli --data-dir <local-data-dir> benchmark-query-set freeze-agent-replay --max-queries $max_queries --min-queries $query_set_min_queries"
else
  fail "current-stage validation requires --query-set-trace-root or --query-set"
fi

if [ "$mode" = "dry-run" ]; then
  cat <<EOF
{
  "schema_version": "resume-ir.current-stage-validation-plan.v2",
  "mode": "dry-run",
  "validation_profile": "$validation_profile",
  "privacy_boundary": "local_only_redacted_plan",
  "resume_root": "<local-resume-root>",
  "data_dir": "<local-data-dir>",
  "out_dir": "<local-evidence-dir>",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "full_baseline_satisfied": $full_baseline_satisfied,
  "release_readiness_evidence": $release_readiness_evidence,
  "performance_optimization_deferred": true,
  "actual_execution_requires": "operator_local_execute_mode",
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries,
    "ocr_throughput_max_documents": $ocr_throughput_max_documents,
    "ocr_throughput_max_pages": $ocr_throughput_max_pages,
    "ocr_throughput_pages_per_document": $ocr_throughput_pages_per_document,
    "ocr_throughput_max_run_ms": $ocr_throughput_max_run_ms,
    "ocr_throughput_min_pages": $ocr_throughput_min_pages
  },
  "ordered_steps": [
    {
      "id": "ocr_preflight",
      "command": "resume-cli --data-dir <local-data-dir> ocr preflight --json --ocr-lang <ocr-language> --tesseract-command <local-tesseract-command> --pdftoppm-command <local-pdftoppm-command>"
    },
    {
      "id": "ocr_manifest_draft",
      "command": "resume-cli --data-dir <local-data-dir> ocr draft-manifest --out <local-ocr-runtime-manifest> --runtime-pack-id <reviewed-runtime-pack-id> --tesseract-command <local-tesseract-command> --pdftoppm-command <local-pdftoppm-command> --language <ocr-language> --language-pack <local-language-pack-or-lang=path> [--language-pack <lang=path> ...] --engine-license <engine-license> --renderer-license <renderer-license> --language-license <language-license> [--reviewed]"
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
      "id": "dataset_manifest",
      "command": "$dataset_manifest_plan_command"
    },
    {
      "id": "import_private_corpus",
      "command": "$import_private_corpus_plan_command"
    },
    {
      "id": "ocr_search_publication_once_primitive",
      "command": "resume-daemon --data-dir <local-data-dir> run --foreground --once --work-ocr-once --ocr-tesseract-command <local-tesseract-command> --ocr-pdftoppm-command <local-pdftoppm-command> --embedding-command <local-embedding-command> --embedding-model-id <reviewed-local-model-id> --embedding-dimension <dimension>"
    },
    {
      "id": "ocr_search_publication_bounded_loop",
      "command": "resume-daemon --data-dir <local-data-dir> run --foreground --work-ocr --ocr-tesseract-command <local-tesseract-command> --ocr-pdftoppm-command <local-pdftoppm-command> --embedding-command <local-embedding-command> --embedding-model-id <reviewed-local-model-id> --embedding-dimension <dimension> --worker-interval-ms <bounded-interval-ms> --max-worker-ticks <bounded-worker-ticks> --ocr-jobs-per-tick <bounded-ocr-jobs-per-tick>"
    },
    {
      "id": "corpus_summary",
      "command": "resume-cli --data-dir <local-data-dir> benchmark-corpus-summary --json > <local-evidence-dir>/benchmark-corpus-summary.local.json"
    },
    {
      "id": "query_set_prepare",
      "command": "$query_set_plan_command"
    },
    {
      "id": "private_query_baseline",
      "command": "resume-benchmark private-query --query-set <local-query-set> --resident-command resume-cli --resident-command-arg --data-dir --resident-command-arg <local-data-dir> --resident-command-arg benchmark-query-protocol --resident-command-arg --batch-jsonl --resident-command-arg --embedding-command --resident-command-arg <local-embedding-command> --resident-command-arg --model-id --resident-command-arg <reviewed-local-model-id> --resident-command-arg --dimension --resident-command-arg <dimension> --corpus-summary <local-evidence-dir>/benchmark-corpus-summary.local.json$private_query_partial_hot_index_plan --max-queries $max_queries --request-sample-count $private_query_request_sample_count --top-k $top_k --timeout-ms $private_query_timeout_ms --dataset-manifest-sha256 <dataset-manifest-sha256> --model-manifest-sha256 <model-manifest-sha256> --json > <local-evidence-dir>/private-benchmark-local.json"
    },
    {
      "id": "baseline_shape_gate",
      "command": "resume-benchmark gate --report <local-evidence-dir>/private-benchmark-local.json --require-private-real-corpus$benchmark_gate_smoke_plan --min-documents $baseline_min_documents --min-queries $baseline_min_queries --max-p95-ms 86400000 --max-zero-result-queries 0"
    },
$ocr_throughput_plan_steps
    {
      "id": "redacted_diagnostics",
      "command": "resume-cli --data-dir <local-data-dir> export-diagnostics --redact > <local-evidence-dir>/redacted-diagnostics.json"
    },
    {
      "id": "fault_simulation_smoke",
      "command": "resume-cli --data-dir <local-data-dir> fault-simulate --case disk-space-low --scratch-dir <local-evidence-dir>/fault-simulation-scratch --required-bytes 4096 --available-bytes 1024 --json > <local-evidence-dir>/fault-simulation-storage-low.json"
    },
    {
      "id": "fault_simulation_suite",
      "command": "resume-cli --data-dir <local-data-dir> fault-simulate --suite local-safe --scratch-dir <local-evidence-dir>/fault-simulation-suite-scratch --daemon-binary <local-resume-daemon> --ocr-command <local-ocr-crash-fixture> --json > <local-evidence-dir>/fault-simulation-suite-local-safe.json"
    },
$terminal_plan_steps
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
    "Execute mode validates OCR and embedding runtime manifests/preflight before reading the private resume root.",
    "Optional --embedding-runtime-bin-dir prepends a local runtime bin directory to child-command PATH in execute mode; dry-run and redacted evidence record only whether it was configured, never the local path.",
    "Optional --reuse-imported-corpus with --reuse-dataset-manifest continues from an already imported local data-dir and a prior redacted dataset manifest; it skips dataset scanning and private import but still validates the manifest digest and writes status-backed aggregate import evidence.",
    "Optional --query-set-trace-root pins generated local query sets to trace_source_search_v1 extraction through the RESUME_IR_QUERY_ARTIFACT_ROOT env default without exposing the local path in dry-run or redacted evidence.",
    "After runtime preflight succeeds, execute mode writes resume-ir.dataset-manifest.v1 under <local-evidence-dir> with privacy boundary local_only_redacted_dataset_manifest, then uses its sha256 as the dataset digest unless --dataset-manifest-sha256 is provided for consistency checking.",
    "If --query-set is omitted, execute mode freezes resume-ir.query-set.jsonl.v2 from --query-set-trace-root under <local-evidence-dir> with privacy boundary local_only_private_query_set, then uses the sibling redacted summary query_set_sha256 as the public-safe query-set digest.",
    "Execute mode writes resume-ir.current-stage-handoff.v1 under <local-evidence-dir> after writing a smoke summary, blocked summary, or full current-stage evidence manifest.",
    "Execute mode runs safe synthetic fault-simulation.v1 disk-space-low smoke plus fault-simulation-suite.v1 local-safe evidence after redacted diagnostics; this proves local diagnostic wiring only and does not clear hardware fault-drill release blockers.",
    "Execute mode keeps all evidence local under <local-evidence-dir>.",
    "The smoke validation profile proves local command wiring and never produces release-readiness evidence.",
    "The baseline shape gate deliberately uses --max-p95-ms 86400000; P95/P99 reduction is deferred.",
    "Release-readiness is expected to remain blocked while signing, notarization, platform installer, and other private quality evidence are missing."
  ]
}
EOF
  exit 0
fi

[ "$mode" = "execute" ] || usage
[ -d "$resume_root" ] || fail "resume root must exist and be a directory"
[ -z "$query_set_trace_root" ] || [ -d "$query_set_trace_root" ] || fail "query set trace root must exist and be a directory"
mkdir -p "$data_dir" "$out_dir"
if [ -n "$embedding_runtime_bin_dir" ]; then
  if [ ! -d "$embedding_runtime_bin_dir" ]; then
    write_runtime_preflight_blocked_summary \
      "model_preflight" "embedding" "embedding_runtime_bin_dir_unavailable" 1
    fail "current-stage validation blocked: embedding runtime bin dir unavailable before reading private corpus"
  fi
  PATH="$embedding_runtime_bin_dir:$PATH"
  export PATH
fi
dataset_manifest="$out_dir/dataset-manifest.local.json"
query_set_generated="false"
provided_query_set=""
if [ -z "$query_set" ]; then
  query_set="$out_dir/private-query-set.local.jsonl"
  query_set_generated="true"
else
  provided_query_set="$query_set"
  query_set="$out_dir/private-query-set.local.jsonl"
fi

ocr_reviewed_arg=""
if [ "$reviewed_ocr_runtime" = "true" ]; then
  ocr_reviewed_arg="--reviewed"
fi
model_reviewed_arg=""
if [ "$reviewed_model" = "true" ]; then
  model_reviewed_arg="--reviewed"
fi

printf '%s\n' "current-stage validation: ocr preflight"
set +e
"$resume_cli" --data-dir "$data_dir" ocr preflight --json \
  --ocr-lang "$language" \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  > "$out_dir/ocr-preflight.json"
ocr_preflight_status=$?
set -e
if [ "$ocr_preflight_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "ocr_preflight" "ocr" "ocr_runtime_preflight_failed" "$ocr_preflight_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi
if ! grep -Fq '"runtime_probe": "passed"' "$out_dir/ocr-preflight.json"; then
  write_runtime_preflight_blocked_summary \
    "ocr_preflight" "ocr" "ocr_runtime_probe_not_passed" 1
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

printf '%s\n' "current-stage validation: ocr manifest draft"
set -- "$resume_cli" --data-dir "$data_dir" ocr draft-manifest \
  --out "$ocr_runtime_manifest" \
  --runtime-pack-id "$runtime_pack_id" \
  --tesseract-command "$tesseract_command" \
  --pdftoppm-command "$pdftoppm_command" \
  --language "$language"
old_ifs=$IFS
IFS='
'
for language_pack_arg in $language_pack_args; do
  set -- "$@" --language-pack "$language_pack_arg"
done
IFS=$old_ifs
set -- "$@" \
  --engine-license "$engine_license" \
  --renderer-license "$renderer_license" \
  --language-license "$language_license"
if [ -n "$ocr_reviewed_arg" ]; then
  set -- "$@" "$ocr_reviewed_arg"
fi
set +e
"$@" > "$out_dir/ocr-draft-manifest.stdout.txt"
ocr_manifest_draft_status=$?
set -e
if [ "$ocr_manifest_draft_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "ocr_manifest_draft" "ocr" "ocr_runtime_manifest_draft_failed" "$ocr_manifest_draft_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

printf '%s\n' "current-stage validation: ocr manifest validate"
set +e
"$resume_cli" --data-dir "$data_dir" ocr validate-manifest \
  --manifest "$ocr_runtime_manifest" \
  > "$out_dir/ocr-validate-manifest.stdout.txt"
ocr_manifest_validate_status=$?
set -e
if [ "$ocr_manifest_validate_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "ocr_manifest_validate" "ocr" "ocr_runtime_manifest_validate_failed" "$ocr_manifest_validate_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

ocr_runtime_manifest_sha256_output=$(sha256_file "$ocr_runtime_manifest")
if [ -n "$ocr_runtime_manifest_sha256" ] && [ "$ocr_runtime_manifest_sha256" != "$ocr_runtime_manifest_sha256_output" ]; then
  write_runtime_preflight_blocked_summary \
    "ocr_manifest_validate" "ocr" "ocr_runtime_manifest_digest_mismatch" 1
  fail "OCR runtime manifest digest mismatch"
fi
ocr_runtime_manifest_sha256="$ocr_runtime_manifest_sha256_output"

printf '%s\n' "current-stage validation: model manifest draft"
set +e
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
model_manifest_draft_status=$?
set -e
if [ "$model_manifest_draft_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "model_manifest_draft" "embedding" "embedding_model_manifest_draft_failed" "$model_manifest_draft_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

printf '%s\n' "current-stage validation: model manifest validate"
set +e
"$resume_cli" --data-dir "$data_dir" model validate-manifest \
  --manifest "$model_manifest" \
  > "$out_dir/model-validate-manifest.stdout.txt"
model_manifest_validate_status=$?
set -e
if [ "$model_manifest_validate_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "model_manifest_validate" "embedding" "embedding_model_manifest_validate_failed" "$model_manifest_validate_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

printf '%s\n' "current-stage validation: model preflight"
set +e
"$resume_cli" --data-dir "$data_dir" model preflight --json \
  --manifest "$model_manifest" \
  --embedding-command "$embedding_command" \
  --model-id "$model_id" \
  --dimension "$dimension" \
  > "$out_dir/model-preflight.json"
model_preflight_status=$?
set -e
if [ "$model_preflight_status" -ne 0 ]; then
  write_runtime_preflight_blocked_summary \
    "model_preflight" "embedding" "embedding_runtime_preflight_failed" "$model_preflight_status"
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi
if ! grep -Fq '"embedding_protocol": "passed"' "$out_dir/model-preflight.json"; then
  write_runtime_preflight_blocked_summary \
    "model_preflight" "embedding" "embedding_protocol_not_passed" 1
  fail "current-stage validation blocked: runtime preflight failed before reading private corpus"
fi

model_manifest_sha256_output=$(sha256_file "$model_manifest")
if [ -n "$model_manifest_sha256" ] && [ "$model_manifest_sha256" != "$model_manifest_sha256_output" ]; then
  write_runtime_preflight_blocked_summary \
    "model_manifest_validate" "embedding" "embedding_model_manifest_digest_mismatch" 1
  fail "model manifest digest mismatch"
fi
model_manifest_sha256="$model_manifest_sha256_output"

if [ "$reuse_imported_corpus" = "true" ]; then
  printf '%s\n' "current-stage validation: dataset manifest (reused)"
  if [ ! -f "$reuse_dataset_manifest" ]; then
    write_import_parser_blocked_summary \
      "dataset_manifest" "reuse_dataset_manifest_unavailable" 1
    fail "current-stage validation blocked: reusable dataset manifest is unavailable"
  fi
  if [ "$reuse_dataset_manifest" != "$dataset_manifest" ]; then
    cp "$reuse_dataset_manifest" "$dataset_manifest" || {
      write_import_parser_blocked_summary \
        "dataset_manifest" "reuse_dataset_manifest_copy_failed" 1
      fail "current-stage validation blocked: reusable dataset manifest copy failed"
    }
  fi
  if ! validate_dataset_manifest_report "$dataset_manifest"; then
    write_import_parser_blocked_summary \
      "dataset_manifest" "reuse_dataset_manifest_invalid" 1
    fail "current-stage validation blocked: reusable dataset manifest is invalid"
  fi
  generated_dataset_manifest_sha256=$(sha256_file "$dataset_manifest")
  if [ -n "$dataset_manifest_sha256" ] && [ "$dataset_manifest_sha256" != "$generated_dataset_manifest_sha256" ]; then
    write_import_parser_blocked_summary \
      "dataset_manifest" "dataset_manifest_digest_mismatch" 1
    fail "dataset manifest digest mismatch"
  fi
  dataset_manifest_sha256="$generated_dataset_manifest_sha256"
  {
    printf '%s\n' "dataset manifest: reused"
    printf '%s\n' "schema: resume-ir.dataset-manifest.v1"
    printf '%s\n' "privacy boundary: local_only_redacted_dataset_manifest"
    printf '%s\n' "manifest sha256: $dataset_manifest_sha256"
    printf '%s\n' "paths: <redacted>"
  } > "$out_dir/dataset-manifest.stdout.txt"
  if ! validate_no_private_markers_in_file "$out_dir/dataset-manifest.stdout.txt" \
    "dataset manifest reuse stdout"; then
    write_import_parser_blocked_summary \
      "dataset_manifest" "reuse_dataset_manifest_stdout_leaked_private_marker" 1
    fail "current-stage validation blocked: reusable dataset manifest stdout leaked private marker"
  fi

  printf '%s\n' "current-stage validation: import private corpus (reused)"
  status_tmp="$out_dir/import-status.tmp"
  set +e
  "$resume_cli" --data-dir "$data_dir" status > "$status_tmp"
  import_status=$?
  set -e
  if [ "$import_status" -ne 0 ]; then
    : > "$out_dir/import.stdout.txt"
    write_import_parser_blocked_summary \
      "import_private_corpus" "reuse_imported_corpus_status_failed" "$import_status"
    rm -f "$status_tmp"
    fail "current-stage validation blocked: reusable data-dir status failed"
  fi
  if ! validate_no_private_markers_in_file "$status_tmp" "reusable data-dir status"; then
    : > "$out_dir/import.stdout.txt"
    write_import_parser_blocked_summary \
      "import_private_corpus" "reuse_imported_corpus_status_leaked_private_marker" 1
    rm -f "$status_tmp"
    fail "current-stage validation blocked: reusable data-dir status leaked private marker"
  fi
  {
    printf '%s\n' "import: reused existing data-dir"
    printf '%s\n' "scan: skipped"
    printf '%s\n' "private root read: false"
    cat "$status_tmp"
  } > "$out_dir/import.stdout.txt"
  recoverable_import_tasks=$(status_count_or_empty "$status_tmp" "import tasks recoverable")
  case "$recoverable_import_tasks" in
    ''|*[!0-9]*)
      write_import_parser_blocked_summary \
        "import_private_corpus" "reuse_imported_corpus_status_missing_recoverable_count" 1
      rm -f "$status_tmp"
      fail "current-stage validation blocked: reusable data-dir status is missing import task terminality"
      ;;
  esac
  if [ "$recoverable_import_tasks" -gt 0 ]; then
    write_import_parser_blocked_summary \
      "import_private_corpus" "reuse_imported_corpus_recoverable_task_present" 1
    rm -f "$status_tmp"
    fail "current-stage validation blocked: reusable data-dir still has recoverable import work"
  fi
  rm -f "$status_tmp"
else
  printf '%s\n' "current-stage validation: dataset manifest"
  set +e
  "$resume_cli" --data-dir "$data_dir" privacy dataset-manifest \
    --root "$resume_root" \
    --out "$dataset_manifest" \
    --profile explicit \
    --max-files "$max_files" \
    > "$out_dir/dataset-manifest.stdout.txt"
  dataset_manifest_status=$?
  set -e
  if [ "$dataset_manifest_status" -ne 0 ]; then
    write_import_parser_blocked_summary \
      "dataset_manifest" "dataset_manifest_failed" "$dataset_manifest_status"
    fail "current-stage validation blocked: import/parser failed"
  fi
  generated_dataset_manifest_sha256=$(sha256_file "$dataset_manifest")
  if [ -n "$dataset_manifest_sha256" ] && [ "$dataset_manifest_sha256" != "$generated_dataset_manifest_sha256" ]; then
    write_import_parser_blocked_summary \
      "dataset_manifest" "dataset_manifest_digest_mismatch" 1
    fail "dataset manifest digest mismatch"
  fi
  dataset_manifest_sha256="$generated_dataset_manifest_sha256"

  printf '%s\n' "current-stage validation: import private corpus"
  set +e
  "$resume_cli" --data-dir "$data_dir" import \
    --root "$resume_root" \
    --profile explicit \
    --max-files "$max_files" \
    > "$out_dir/import.stdout.txt"
  import_status=$?
  set -e
  if [ "$import_status" -ne 0 ]; then
    write_import_parser_blocked_summary \
      "import_private_corpus" "import_private_corpus_failed" "$import_status"
    fail "current-stage validation blocked: import/parser failed"
  fi
fi

printf '%s\n' "current-stage validation: bounded OCR and atomic search publication"
"$resume_daemon" --data-dir "$data_dir" run --foreground \
  --work-ocr \
  --ocr-tesseract-command "$tesseract_command" \
  --ocr-pdftoppm-command "$pdftoppm_command" \
  --ocr-lang "$language" \
  --ocr-render-dpi "$ocr_render_dpi" \
  --ocr-page-timeout-ms "$ocr_page_timeout_ms" \
  --ocr-max-pages-per-document "$ocr_max_pages_per_document" \
  --embedding-command "$embedding_command" \
  --embedding-model-id "$model_id" \
  --embedding-dimension "$dimension" \
  --embedding-timeout-ms "$embedding_timeout_ms" \
  --worker-interval-ms "$worker_interval_ms" \
  --max-worker-ticks "$ocr_worker_ticks" \
  --ocr-jobs-per-tick "$ocr_jobs_per_tick" \
  > "$out_dir/ocr-search-publication.stdout.txt"

printf '%s\n' "current-stage validation: corpus summary"
"$resume_cli" --data-dir "$data_dir" benchmark-corpus-summary --json \
  > "$out_dir/benchmark-corpus-summary.local.json"

if [ "$validation_profile" = "full" ] && corpus_summary_has_bounded_ocr_backlog "$out_dir/benchmark-corpus-summary.local.json"; then
  printf '%s\n' "current-stage validation: redacted diagnostics"
  set +e
  "$resume_cli" --data-dir "$data_dir" export-diagnostics --redact \
    > "$out_dir/redacted-diagnostics.json"
  redacted_diagnostics_status=$?
  set -e
  redacted_diagnostics_invalid=false
  if [ "$redacted_diagnostics_status" -eq 0 ] && ! validate_redacted_diagnostics_report "$out_dir/redacted-diagnostics.json"; then
    redacted_diagnostics_status=1
    redacted_diagnostics_invalid=true
  fi
  write_ocr_backlog_blocked_summary "$redacted_diagnostics_status"
  if [ "$redacted_diagnostics_status" -ne 0 ]; then
    if [ "$redacted_diagnostics_invalid" = "true" ]; then
      printf '%s\n' "current-stage validation blocked: bounded OCR backlog remains; redacted diagnostics evidence failed validation" >&2
    else
      printf '%s\n' "current-stage validation blocked: bounded OCR backlog remains; redacted diagnostics failed" >&2
    fi
    exit "$redacted_diagnostics_status"
  fi
  printf '%s\n' "current-stage validation: doctor"
  set +e
  "$resume_cli" --data-dir "$data_dir" doctor \
    > "$out_dir/doctor.out"
  doctor_status=$?
  set -e
  if [ "$doctor_status" -ne 0 ]; then
    write_ocr_backlog_blocked_summary 1
    printf '%s\n' "current-stage validation blocked: bounded OCR backlog remains; doctor failed" >&2
    exit "$doctor_status"
  fi
  printf '%s\n' "current-stage validation blocked: bounded OCR backlog remains" >&2
  exit 1
fi

printf '%s\n' "current-stage validation: query set"
if [ "$query_set_generated" = "true" ]; then
  set +e
	  RESUME_IR_LOCAL_EVIDENCE_DIR="$out_dir" \
	    RESUME_IR_QUERY_ARTIFACT_ROOT="$query_set_trace_root" \
	    "$resume_cli" --data-dir "$data_dir" benchmark-query-set freeze-agent-replay \
	    --max-queries "$max_queries" \
	    --min-queries "$query_set_min_queries" \
	    > "$out_dir/query-set-prepare.stdout.txt" \
	    2> "$out_dir/query-set-prepare.stderr.txt"
	  query_set_prepare_status=$?
	  set -e
	  if [ "$query_set_prepare_status" -ne 0 ]; then
	    write_query_set_trace_preflight
	    query_set_blocked_reason=$(query_set_prepare_blocked_reason)
	    write_query_set_blocked_summary "$query_set_prepare_status" "$query_set_blocked_reason"
	    query_set_prepare_blocked_message "$query_set_blocked_reason" >&2
	    exit "$query_set_prepare_status"
	  fi
else
  [ -f "$provided_query_set" ] || fail "query set must exist and stay local"
  provided_query_set_summary=$(query_set_summary_path "$provided_query_set")
  query_set_summary=$(query_set_summary_path "$query_set")
  [ -f "$provided_query_set_summary" ] || fail "query set summary must exist and stay local"
  if [ "$provided_query_set" != "$query_set" ]; then
    cp "$provided_query_set" "$query_set" || fail "query set must stay local and readable"
    cp "$provided_query_set_summary" "$query_set_summary" || fail "query set summary must stay local and readable"
  fi
  {
    printf '%s\n' "query set: provided"
    printf '%s\n' "schema: resume-ir.query-set.jsonl.v2"
    printf '%s\n' "privacy boundary: local_only_private_query_set"
    printf '%s\n' "queries: <redacted>"
    printf '%s\n' "paths: <redacted>"
  } > "$out_dir/query-set-prepare.stdout.txt"
fi
query_set_summary=$(query_set_summary_path "$query_set")
[ -f "$query_set_summary" ] || fail "query set summary must exist and stay local"
query_set_output_sha256=$(sha256_file "$query_set")
query_set_summary_sha256_output=$(sha256_file "$query_set_summary")
if [ "$validation_profile" = "full" ]; then
  set +e
  query_set_sha256=$(query_set_summary_sha256 "$query_set_summary" "trace_source_search_v1")
  query_set_summary_status=$?
  set -e
  if [ "$query_set_summary_status" -ne 0 ]; then
    write_query_set_blocked_summary "$query_set_summary_status" "query_set_source_invalid"
    printf '%s\n' "current-stage validation blocked: query-set source invalid" >&2
    exit "$query_set_summary_status"
  fi
else
  query_set_sha256=$(query_set_summary_sha256 "$query_set_summary")
fi
if [ "$query_set_generated" != "true" ]; then
  append_query_set_prepare_summary_stdout "$query_set_summary" >> "$out_dir/query-set-prepare.stdout.txt"
fi

printf '%s\n' "current-stage validation: private query baseline"
set +e
"$resume_benchmark" private-query \
  --query-set "$query_set" \
  --resident-command "$resume_cli" \
  --resident-command-arg --data-dir --resident-command-arg "$data_dir" \
  --resident-command-arg benchmark-query-protocol \
  --resident-command-arg --batch-jsonl \
  --resident-command-arg --embedding-command --resident-command-arg "$embedding_command" \
  --resident-command-arg --model-id --resident-command-arg "$model_id" \
  --resident-command-arg --dimension --resident-command-arg "$dimension" \
  --corpus-summary "$out_dir/benchmark-corpus-summary.local.json" \
  $private_query_partial_hot_index_arg \
  --max-queries "$max_queries" \
  --request-sample-count "$private_query_request_sample_count" \
  --top-k "$top_k" \
  --timeout-ms "$private_query_timeout_ms" \
  --dataset-manifest-sha256 "$dataset_manifest_sha256" \
  --model-manifest-sha256 "$model_manifest_sha256" \
  --json \
  > "$out_dir/private-benchmark-local.json"
private_query_status=$?
set -e
if [ "$private_query_status" -ne 0 ]; then
  write_private_query_blocked_summary "$private_query_status" "private_query_baseline_failed"
  printf '%s\n' "current-stage validation blocked: private query baseline failed" >&2
  exit "$private_query_status"
fi
if ! validate_private_benchmark_report "$out_dir/private-benchmark-local.json"; then
  write_private_query_blocked_summary 1 "private_query_baseline_invalid"
  printf '%s\n' "current-stage validation blocked: private query baseline evidence failed validation" >&2
  exit 1
fi
private_benchmark_query_set_sha256_value=$(private_benchmark_query_set_sha256 "$out_dir/private-benchmark-local.json")
if [ "$private_benchmark_query_set_sha256_value" != "$query_set_sha256" ]; then
  write_private_query_blocked_summary 1 "private_query_baseline_query_set_mismatch"
  printf '%s\n' "current-stage validation blocked: private query report query_set_sha256 mismatch" >&2
  exit 1
fi

printf '%s\n' "current-stage validation: baseline shape gate"
set +e
"$resume_benchmark" gate \
  --report "$out_dir/private-benchmark-local.json" \
  --require-private-real-corpus \
  $benchmark_gate_smoke_arg \
  --min-documents "$baseline_min_documents" \
  --min-queries "$baseline_min_queries" \
  --max-p95-ms 86400000 \
  --max-zero-result-queries 0 \
  > "$out_dir/private-benchmark-gate.stdout.txt"
baseline_gate_status=$?
set -e
if [ "$baseline_gate_status" -ne 0 ] && [ "$validation_profile" = "full" ]; then
  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_prepare_stdout_sha256=$(sha256_file "$out_dir/query-set-prepare.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-blocked-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-blocked-summary.v2",
  "privacy_boundary": "local_only_redacted_blocked_summary",
  "validation_profile": "$validation_profile",
  "current_stage_target": "$current_stage_target",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "private_corpus_read": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "blocked_step": "baseline_shape_gate",
  "blocked_category": "benchmark",
  "blocked_reason": "baseline_shape_gate_failed",
  "blocked_exit": $baseline_gate_status,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "blocked", "exit_code": $baseline_gate_status}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "private-query-set.summary.json", "sha256": "$query_set_summary_sha256_output"},
    {"file": "query-set-prepare.stdout.txt", "sha256": "$query_set_prepare_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "redacted diagnostics for this blocked run",
    "release-readiness current-stage evidence",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-blocked-summary.json"
  printf '%s\n' "current-stage validation blocked: baseline shape gate failed" >&2
  exit "$baseline_gate_status"
fi
if [ "$baseline_gate_status" -ne 0 ]; then
  exit "$baseline_gate_status"
fi

if [ "$validation_profile" = "full" ]; then
  if [ -z "$renderer_manifest_sha256" ]; then
    renderer_manifest_sha256="$ocr_runtime_manifest_sha256"
  fi
  if [ -z "$language_pack_manifest_sha256" ]; then
    language_pack_manifest_sha256="$ocr_runtime_manifest_sha256"
  fi

  printf '%s\n' "current-stage validation: private ocr throughput baseline"
  set +e
  "$resume_benchmark" private-ocr-throughput \
    --root "$resume_root" \
    --pdftoppm-command "$pdftoppm_command" \
    --tesseract-command "$tesseract_command" \
    --max-documents "$ocr_throughput_max_documents" \
    --max-pages "$ocr_throughput_max_pages" \
    --pages-per-document "$ocr_throughput_pages_per_document" \
    --page-timeout-ms "$ocr_page_timeout_ms" \
    --max-run-ms "$ocr_throughput_max_run_ms" \
    --render-dpi "$ocr_render_dpi" \
    --ocr-lang "$language" \
    --dataset-manifest-sha256 "$dataset_manifest_sha256" \
    --ocr-runtime-manifest-sha256 "$ocr_runtime_manifest_sha256" \
    --renderer-manifest-sha256 "$renderer_manifest_sha256" \
    --language-pack-manifest-sha256 "$language_pack_manifest_sha256" \
    --json \
    > "$out_dir/private-ocr-throughput.json"
  private_ocr_throughput_status=$?
  set -e
  if [ "$private_ocr_throughput_status" -ne 0 ]; then
    write_ocr_throughput_blocked_summary \
      "private_ocr_throughput_baseline" \
      "private_ocr_throughput_failed" \
      "$private_ocr_throughput_status"
    printf '%s\n' "current-stage validation blocked: private OCR throughput baseline failed" >&2
    exit "$private_ocr_throughput_status"
  fi
  if ! validate_ocr_throughput_report "$out_dir/private-ocr-throughput.json"; then
    write_ocr_throughput_blocked_summary \
      "private_ocr_throughput_baseline" \
      "private_ocr_throughput_invalid" \
      1
    printf '%s\n' "current-stage validation blocked: private OCR throughput evidence failed validation" >&2
    exit 1
  fi

  printf '%s\n' "current-stage validation: ocr throughput baseline gate"
  set +e
  "$resume_benchmark" ocr-gate \
    --report "$out_dir/private-ocr-throughput.json" \
    --current-stage-baseline \
    --require-private-real-corpus \
    --min-pages "$ocr_throughput_min_pages" \
    > "$out_dir/ocr-throughput-gate.stdout.txt"
  ocr_throughput_gate_status=$?
  set -e
  if [ "$ocr_throughput_gate_status" -ne 0 ]; then
    write_ocr_throughput_blocked_summary \
      "ocr_throughput_baseline_gate" \
      "ocr_throughput_baseline_gate_failed" \
      "$ocr_throughput_gate_status"
    printf '%s\n' "current-stage validation blocked: OCR throughput baseline gate failed" >&2
    exit "$ocr_throughput_gate_status"
  fi
fi

printf '%s\n' "current-stage validation: redacted diagnostics"
set +e
"$resume_cli" --data-dir "$data_dir" export-diagnostics --redact \
  > "$out_dir/redacted-diagnostics.json"
redacted_diagnostics_status=$?
set -e
if [ "$redacted_diagnostics_status" -ne 0 ]; then
  write_redacted_diagnostics_blocked_summary "$redacted_diagnostics_status" "redacted_diagnostics_failed"
  printf '%s\n' "current-stage validation blocked: redacted diagnostics failed" >&2
  exit "$redacted_diagnostics_status"
fi
if ! validate_redacted_diagnostics_report "$out_dir/redacted-diagnostics.json"; then
  write_redacted_diagnostics_blocked_summary 1 "redacted_diagnostics_invalid"
  printf '%s\n' "current-stage validation blocked: redacted diagnostics evidence failed validation" >&2
  exit 1
fi

printf '%s\n' "current-stage validation: doctor"
set +e
"$resume_cli" --data-dir "$data_dir" doctor \
  > "$out_dir/doctor.out"
doctor_status=$?
set -e
if [ "$doctor_status" -ne 0 ]; then
  write_redacted_diagnostics_blocked_summary "$doctor_status" "doctor_failed"
  printf '%s\n' "current-stage validation blocked: doctor failed" >&2
  exit "$doctor_status"
fi

printf '%s\n' "current-stage validation: fault simulation smoke"
set +e
"$resume_cli" --data-dir "$data_dir" fault-simulate \
  --case disk-space-low \
  --scratch-dir "$out_dir/fault-simulation-scratch" \
  --required-bytes 4096 \
  --available-bytes 1024 \
  --json \
  > "$out_dir/fault-simulation-storage-low.json"
fault_simulation_status=$?
set -e
rm -rf "$out_dir/fault-simulation-scratch"
if [ "$fault_simulation_status" -ne 0 ]; then
  write_fault_simulation_blocked_summary "$fault_simulation_status" "fault_simulation_smoke_failed"
  printf '%s\n' "current-stage validation blocked: fault simulation smoke failed" >&2
  exit "$fault_simulation_status"
fi

printf '%s\n' "current-stage validation: fault simulation suite"
mkdir -p "$out_dir/fault-simulation-suite-scratch"
ocr_crash_fixture="$out_dir/fault-simulation-suite-scratch/ocr-crash-fixture.sh"
cat > "$ocr_crash_fixture" <<'EOF'
#!/bin/sh
printf 'PRIVATE_CURRENT_STAGE_OCR_CRASH_STDOUT\n'
printf 'PRIVATE_CURRENT_STAGE_OCR_CRASH_STDERR\n' >&2
exit 17
EOF
chmod 700 "$ocr_crash_fixture"
set +e
"$resume_cli" --data-dir "$data_dir" fault-simulate \
  --suite local-safe \
  --scratch-dir "$out_dir/fault-simulation-suite-scratch" \
  --daemon-binary "$resume_daemon" \
  --ocr-command "$ocr_crash_fixture" \
  --json \
  > "$out_dir/fault-simulation-suite-local-safe.json"
fault_simulation_suite_status=$?
set -e
rm -rf "$out_dir/fault-simulation-suite-scratch"
if [ "$fault_simulation_suite_status" -ne 0 ]; then
  write_fault_simulation_blocked_summary "$fault_simulation_suite_status" "fault_simulation_suite_failed"
  printf '%s\n' "current-stage validation blocked: fault simulation suite failed" >&2
  exit "$fault_simulation_suite_status"
fi
if ! validate_fault_suite_report "$out_dir/fault-simulation-suite-local-safe.json"; then
  write_fault_simulation_blocked_summary 1 "fault_simulation_suite_invalid"
  printf '%s\n' "current-stage validation blocked: fault simulation suite evidence failed validation" >&2
  exit 1
fi

if [ "$validation_profile" = "smoke" ]; then
  ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
  ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
  ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
  model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
  model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
  model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
  import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
  ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
  corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
  corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
  query_set_prepare_stdout_sha256=$(sha256_file "$out_dir/query-set-prepare.stdout.txt")
  private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
  private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
  redacted_diagnostics_sha256=$(sha256_file "$out_dir/redacted-diagnostics.json")
  doctor_sha256=$(sha256_file "$out_dir/doctor.out")
  fault_simulation_sha256=$(sha256_file "$out_dir/fault-simulation-storage-low.json")
  fault_simulation_suite_sha256=$(sha256_file "$out_dir/fault-simulation-suite-local-safe.json")
  dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
  dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")

  cat > "$out_dir/current-stage-smoke-summary.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-smoke-summary.v2",
  "privacy_boundary": "local_only_redacted_aggregate_summary",
  "validation_profile": "smoke",
  "current_stage_target": "local_real_corpus_smoke_chain",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "smoke_satisfied": true,
  "full_baseline_satisfied": false,
  "release_readiness_evidence": false,
  "performance_optimization_deferred": true,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick,
    "query_set_min_queries": $query_set_min_queries,
    "baseline_min_documents": $baseline_min_documents,
    "baseline_min_queries": $baseline_min_queries
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "smoke_success"},
    {"id": "redacted_diagnostics", "status": "success"},
    {"id": "doctor", "status": "success"},
    {"id": "fault_simulation_smoke", "status": "success"},
    {"id": "fault_simulation_suite", "status": "success"}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "private-query-set.summary.json", "sha256": "$query_set_summary_sha256_output"},
    {"file": "query-set-prepare.stdout.txt", "sha256": "$query_set_prepare_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "redacted-diagnostics.json", "sha256": "$redacted_diagnostics_sha256"},
    {"file": "doctor.out", "sha256": "$doctor_sha256"},
    {"file": "fault-simulation-storage-low.json", "sha256": "$fault_simulation_sha256"},
    {"file": "fault-simulation-suite-local-safe.json", "sha256": "$fault_simulation_suite_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "not_completed": [
    "full 10k/8000-document current-stage baseline",
    "500-query private baseline gate",
    "P95/P99 latency reduction",
    "external 100k/1M validation",
    "stable release readiness"
  ],
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
  write_current_stage_handoff "$out_dir/current-stage-smoke-summary.json"
  printf '%s\n' "current-stage validation: smoke summary written under <local-evidence-dir>"
  printf '%s\n' "current-stage validation: handoff summary written under <local-evidence-dir>"
  printf '%s\n' "current-stage validation: local smoke evidence written under <local-evidence-dir>"
  exit 0
fi

printf '%s\n' "current-stage validation: release-readiness intake"
set +e
"$resume_cli" --data-dir "$data_dir" release-readiness --json \
  --benchmark-report "$out_dir/private-benchmark-local.json" \
  --ocr-throughput-report "$out_dir/private-ocr-throughput.json" \
  --model-manifest "$model_manifest" \
  --ocr-runtime-manifest "$ocr_runtime_manifest" \
  --diagnostics-report "$out_dir/redacted-diagnostics.json" \
  > "$out_dir/release-readiness.json" \
  2> "$out_dir/release-readiness.stderr.txt"
release_status=$?
set -e
if [ "$release_status" -ne 0 ]; then
  if grep -Fq "release readiness evidence failed validation" "$out_dir/release-readiness.stderr.txt"; then
    write_release_readiness_blocked_summary \
      "$release_status" "release_readiness_evidence_failed_validation"
    printf '%s\n' \
      "current-stage validation blocked: release-readiness evidence failed validation" >&2
    exit "$release_status"
  fi
  if ! grep -Fq "release readiness blocked: stable release criteria are not met" "$out_dir/release-readiness.stderr.txt"; then
    write_release_readiness_blocked_summary \
      "$release_status" "release_readiness_unexpected_error"
    printf '%s\n' \
      "current-stage validation blocked: release-readiness returned an unexpected error" >&2
    exit "$release_status"
  fi
fi
if [ "$release_status" -eq 0 ]; then
  stable_release_expected_blocked="false"
  release_readiness_step_status="success"
else
  stable_release_expected_blocked="true"
  release_readiness_step_status="expected_blocked"
fi

ocr_preflight_sha256=$(sha256_file "$out_dir/ocr-preflight.json")
ocr_draft_stdout_sha256=$(sha256_file "$out_dir/ocr-draft-manifest.stdout.txt")
ocr_validate_stdout_sha256=$(sha256_file "$out_dir/ocr-validate-manifest.stdout.txt")
model_draft_stdout_sha256=$(sha256_file "$out_dir/model-draft-manifest.stdout.txt")
model_validate_stdout_sha256=$(sha256_file "$out_dir/model-validate-manifest.stdout.txt")
model_preflight_sha256=$(sha256_file "$out_dir/model-preflight.json")
import_stdout_sha256=$(sha256_file "$out_dir/import.stdout.txt")
ocr_search_publication_stdout_sha256=$(sha256_file "$out_dir/ocr-search-publication.stdout.txt")
corpus_summary_sha256=$(sha256_file "$out_dir/benchmark-corpus-summary.local.json")
query_set_prepare_stdout_sha256=$(sha256_file "$out_dir/query-set-prepare.stdout.txt")
private_benchmark_sha256=$(sha256_file "$out_dir/private-benchmark-local.json")
private_benchmark_gate_sha256=$(sha256_file "$out_dir/private-benchmark-gate.stdout.txt")
private_ocr_throughput_sha256=$(sha256_file "$out_dir/private-ocr-throughput.json")
ocr_throughput_gate_sha256=$(sha256_file "$out_dir/ocr-throughput-gate.stdout.txt")
redacted_diagnostics_sha256=$(sha256_file "$out_dir/redacted-diagnostics.json")
doctor_sha256=$(sha256_file "$out_dir/doctor.out")
fault_simulation_sha256=$(sha256_file "$out_dir/fault-simulation-storage-low.json")
fault_simulation_suite_sha256=$(sha256_file "$out_dir/fault-simulation-suite-local-safe.json")
release_readiness_sha256=$(sha256_file "$out_dir/release-readiness.json")
release_readiness_stderr_sha256=$(sha256_file "$out_dir/release-readiness.stderr.txt")
dataset_manifest_sha256_output=$(sha256_file "$dataset_manifest")
dataset_manifest_stdout_sha256=$(sha256_file "$out_dir/dataset-manifest.stdout.txt")
corpus_summary_observability=$(corpus_summary_observability_json "$out_dir/benchmark-corpus-summary.local.json")
private_query_observability=$(private_query_observability_json "$out_dir/private-benchmark-local.json")

cat > "$out_dir/current-stage-validation-evidence.json" <<EOF
{
  "schema_version": "resume-ir.current-stage-validation-evidence.v2",
  "privacy_boundary": "local_only_redacted_evidence_manifest",
  "current_stage_target": "reproducible_local_10k_baseline",
  "runtime_distribution_mode": "$runtime_distribution_mode",
  "runtime_package_binaries_included": $runtime_package_binaries_included,
  "full_baseline_satisfied": true,
  "release_readiness_evidence": true,
  "performance_optimization_deferred": true,
  "release_readiness_exit": $release_status,
  "stable_release_expected_blocked": $stable_release_expected_blocked,
  "input_digests": {
    "dataset_manifest_sha256": "$dataset_manifest_sha256",
    "query_set_sha256": "$query_set_sha256",
    "model_manifest_sha256": "$model_manifest_sha256",
    "ocr_runtime_manifest_sha256": "$ocr_runtime_manifest_sha256"
  },
  "parameters": {
    "max_files": $max_files,
    "max_queries": $max_queries,
    "top_k": $top_k,
    "private_query_timeout_ms": $private_query_timeout_ms,
    "embedding_dimension": $dimension,
    "embedding_runtime_bin_dir_configured": $embedding_runtime_bin_dir_configured,
    "reuse_imported_corpus": $reuse_imported_corpus,
    "ocr_worker_ticks": $ocr_worker_ticks,
    "ocr_jobs_per_tick": $ocr_jobs_per_tick
  },
  "preflight_probes": {
    "ocr_runtime_probe": "passed",
    "embedding_protocol": "passed"
  },
  "corpus_summary_observability": $corpus_summary_observability,
  "private_query_observability": $private_query_observability,
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_search_publication_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_prepare", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
    {"id": "private_ocr_throughput_baseline", "status": "success"},
    {"id": "ocr_throughput_baseline_gate", "status": "success"},
    {"id": "redacted_diagnostics", "status": "success"},
    {"id": "doctor", "status": "success"},
    {"id": "fault_simulation_smoke", "status": "success"},
    {"id": "fault_simulation_suite", "status": "success"},
    {"id": "release_readiness_intake", "status": "$release_readiness_step_status", "exit_code": $release_status}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "$dataset_manifest_sha256_output"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "$dataset_manifest_stdout_sha256"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "$ocr_runtime_manifest_sha256_output"},
    {"file": "ocr-preflight.json", "sha256": "$ocr_preflight_sha256"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "$ocr_draft_stdout_sha256"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "$ocr_validate_stdout_sha256"},
    {"file": "model-manifest.local.json", "sha256": "$model_manifest_sha256_output"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "$model_draft_stdout_sha256"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "$model_validate_stdout_sha256"},
    {"file": "model-preflight.json", "sha256": "$model_preflight_sha256"},
    {"file": "import.stdout.txt", "sha256": "$import_stdout_sha256"},
    {"file": "ocr-search-publication.stdout.txt", "sha256": "$ocr_search_publication_stdout_sha256"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "$corpus_summary_sha256"},
    {"file": "private-query-set.local.jsonl", "sha256": "$query_set_output_sha256"},
    {"file": "private-query-set.summary.json", "sha256": "$query_set_summary_sha256_output"},
    {"file": "query-set-prepare.stdout.txt", "sha256": "$query_set_prepare_stdout_sha256"},
    {"file": "private-benchmark-local.json", "sha256": "$private_benchmark_sha256"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "$private_benchmark_gate_sha256"},
    {"file": "private-ocr-throughput.json", "sha256": "$private_ocr_throughput_sha256"},
    {"file": "ocr-throughput-gate.stdout.txt", "sha256": "$ocr_throughput_gate_sha256"},
    {"file": "redacted-diagnostics.json", "sha256": "$redacted_diagnostics_sha256"},
    {"file": "doctor.out", "sha256": "$doctor_sha256"},
    {"file": "fault-simulation-storage-low.json", "sha256": "$fault_simulation_sha256"},
    {"file": "fault-simulation-suite-local-safe.json", "sha256": "$fault_simulation_suite_sha256"},
    {"file": "release-readiness.json", "sha256": "$release_readiness_sha256"},
    {"file": "release-readiness.stderr.txt", "sha256": "$release_readiness_stderr_sha256"}
  ],
  "privacy_sentinels": {
    "local_paths_included": false,
    "raw_resume_text_included": false,
    "raw_query_text_included": false,
    "model_bytes_included": false,
    "runtime_binaries_included": false,
    "report_bodies_included": false
  },
  "must_not_upload": [
    "raw resumes",
    "query set",
    "local manifests",
    "benchmark reports",
    "diagnostics",
    "indexes",
    "SQLite databases",
    "model caches",
    "runtime binaries"
  ]
}
EOF
write_current_stage_handoff "$out_dir/current-stage-validation-evidence.json"
printf 'current-stage validation: release-readiness exit %s\n' "$release_status"
printf '%s\n' "current-stage validation: redacted evidence manifest written under <local-evidence-dir>"
printf '%s\n' "current-stage validation: handoff summary written under <local-evidence-dir>"
printf '%s\n' "current-stage validation: local evidence written under <local-evidence-dir>"

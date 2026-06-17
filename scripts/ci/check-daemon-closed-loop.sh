#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "daemon closed-loop missing expected output"
  fi
}

require_text() {
  file="$1"
  text="$2"
  require_file "$file"
  if ! grep -Fq -- "$text" "$file"; then
    fail "daemon closed-loop output is missing required evidence: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  label="$3"
  require_file "$file"
  if [ -n "$text" ] && grep -Fq -- "$text" "$file"; then
    fail "daemon closed-loop output leaked $label"
  fi
}

reject_regex() {
  file="$1"
  pattern="$2"
  label="$3"
  require_file "$file"
  if grep -Eq -- "$pattern" "$file"; then
    fail "daemon closed-loop output leaked $label"
  fi
}

run_cli() {
  label="$1"
  stdout_file="$2"
  stderr_file="$stdout_file.stderr"
  shift 2
  if ! "$CARGO_BIN" run --quiet -p resume-cli --locked -- --data-dir "$data_dir" "$@" \
    >"$stdout_file" 2>"$stderr_file"; then
    if [ "${RESUME_IR_CLOSED_LOOP_DEBUG:-0}" = "1" ]; then
      sed -n '1,80p' "$stdout_file" >&2 || true
      sed -n '1,80p' "$stderr_file" >&2 || true
    fi
    fail "daemon closed-loop failed at step: $label"
  fi
  if [ -s "$stderr_file" ]; then
    if [ "${RESUME_IR_CLOSED_LOOP_DEBUG:-0}" = "1" ]; then
      sed -n '1,80p' "$stdout_file" >&2 || true
      sed -n '1,80p' "$stderr_file" >&2 || true
    fi
    fail "daemon closed-loop command wrote stderr at step: $label"
  fi
}

reject_paths() {
  file="$1"
  reject_text "$file" "$tmpdir" "temporary root path"
  reject_text "$file" "$data_dir" "private data-dir path"
  reject_text "$file" "$fixture_root" "fixture root path"
  reject_text "$file" "$canonical_fixture_root" "canonical fixture root path"
  reject_text "$file" "$ocr_command" "OCR command path"
  reject_text "$file" "$embedding_command" "embedding command path"
  reject_regex "$file" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"
}

stop_daemon() {
  if [ -n "${daemon_pid:-}" ]; then
    if kill -0 "$daemon_pid" >/dev/null 2>&1; then
      kill "$daemon_pid" >/dev/null 2>&1 || true
    fi
    wait "$daemon_pid" >/dev/null 2>&1 || true
    daemon_pid=""
  fi
}

cleanup() {
  stop_daemon
  rm -rf "$tmpdir"
}

wait_for_daemon_manifest() {
  count=0
  while [ "$count" -lt 120 ]; do
    if [ -f "$data_dir/ipc.endpoints.json" ]; then
      return 0
    fi
    if [ -n "${daemon_pid:-}" ] && ! kill -0 "$daemon_pid" >/dev/null 2>&1; then
      fail "daemon closed-loop daemon exited before IPC became ready"
    fi
    count=$((count + 1))
    sleep 0.25
  done
  fail "daemon closed-loop IPC manifest did not become ready"
}

wait_for_indexed_daemon_status() {
  count=0
  while [ "$count" -lt 120 ]; do
    status_poll_out="$tmpdir/status-poll-$count.out"
    run_cli "status ipc poll" "$status_poll_out" status --ipc auto
    if grep -Fq "searchable documents: 3" "$status_poll_out" \
      && grep -Fq "ocr queue: 0" "$status_poll_out" \
      && grep -Fq "search index: daemon ipc (full-text state reported by daemon)" "$status_poll_out"; then
      cp "$status_poll_out" "$ready_status_out"
      return 0
    fi
    reject_paths "$status_poll_out"
    count=$((count + 1))
    sleep 0.25
  done
  fail "daemon closed-loop daemon workers did not reach indexed status"
}

wait_for_semantic_daemon_search() {
  count=0
  while [ "$count" -lt 120 ]; do
    semantic_poll_out="$tmpdir/semantic-search-poll-$count.out"
    run_cli "semantic search ipc poll" "$semantic_poll_out" search SemanticOnlyToken --ipc auto --mode semantic --top-k 20
    if grep -Fq "results: 3" "$semantic_poll_out"; then
      cp "$semantic_poll_out" "$semantic_search_out"
      return 0
    fi
    reject_text "$semantic_poll_out" "SemanticOnlyToken" "semantic raw query"
    reject_paths "$semantic_poll_out"
    count=$((count + 1))
    sleep 0.25
  done
  fail "daemon closed-loop semantic search did not become ready"
}

CARGO_BIN="${CARGO:-}"
if [ -z "$CARGO_BIN" ] && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi
if [ -z "$CARGO_BIN" ]; then
  CARGO_BIN=cargo
fi

fixture_root="tests/fixtures/resumes"
if [ ! -d "$fixture_root" ]; then
  fail "daemon closed-loop fixture root is missing"
fi
canonical_fixture_root="$(cd "$fixture_root" && pwd -P)"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-daemon-closed-loop.XXXXXX")"
trap cleanup EXIT HUP INT TERM

data_dir="$tmpdir/PRIVATE_DAEMON_CLOSED_LOOP_DATA"
ocr_command="$tmpdir/daemon-ocr-fixture.sh"
embedding_command="$tmpdir/daemon-embedding-fixture.sh"
daemon_stdout="$tmpdir/daemon.out"
daemon_stderr="$tmpdir/daemon.err"
ready_status_out="$tmpdir/status-ready.out"
semantic_search_out="$tmpdir/semantic-search-ipc.out"
daemon_pid=""

cat >"$ocr_command" <<'SH'
#!/usr/bin/env sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.97\n'
printf 'text:\n'
printf 'DaemonClosedLoopOCRToken page %s\n' "$RESUME_IR_OCR_PAGE_NO"
SH
chmod 700 "$ocr_command"

cat >"$embedding_command" <<'SH'
#!/usr/bin/env sh
if [ ! -s "$RESUME_IR_EMBEDDING_INPUT_PATH" ]; then
  exit 7
fi
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ {
  id=$1;
  sub(/^input=/, "", id);
  printf "vector=%s\t1,0,0,0\n", id
}' "$RESUME_IR_EMBEDDING_INPUT_PATH"
SH
chmod 700 "$embedding_command"

"$CARGO_BIN" run --quiet -p resume-daemon --locked -- \
  --data-dir "$data_dir" \
  run \
  --foreground \
  --work-imports \
  --work-ocr \
  --work-embeddings \
  --work-index \
  --ocr-command "$ocr_command" \
  --ocr-engine-profile fixture-daemon-closed-loop \
  --embedding-command "$embedding_command" \
  --embedding-model-id fixture-local-model \
  --embedding-dimension 4 \
  --embedding-max-docs 8 \
  --embedding-max-text-bytes 100000 \
  --worker-interval-ms 25 \
  --ipc-listen 127.0.0.1:0 \
  --max-requests 300 \
  >"$daemon_stdout" 2>"$daemon_stderr" &
daemon_pid=$!

wait_for_daemon_manifest

initial_status_out="$tmpdir/status-initial.out"
run_cli "initial status ipc" "$initial_status_out" status --ipc auto
require_text "$initial_status_out" "resume-ir status"
require_text "$initial_status_out" "index health:"
reject_paths "$initial_status_out"

import_out="$tmpdir/import-ipc.out"
run_cli "import ipc" "$import_out" import --ipc auto --root "$fixture_root"
require_text "$import_out" "import task submitted"
require_text "$import_out" "status: queued"
require_text "$import_out" "roots queued: 1"
reject_paths "$import_out"

wait_for_indexed_daemon_status
require_text "$ready_status_out" "searchable documents: 3"
require_text "$ready_status_out" "ocr queue: 0"
require_text "$ready_status_out" "search index: daemon ipc (full-text state reported by daemon)"
reject_paths "$ready_status_out"

search_out="$tmpdir/search-ipc.out"
run_cli "search ipc" "$search_out" search Java --ipc auto --top-k 20
require_text "$search_out" "results: 2"
reject_text "$search_out" "query:" "raw query label"
reject_paths "$search_out"

ocr_search_out="$tmpdir/ocr-search-ipc.out"
run_cli "ocr search ipc" "$ocr_search_out" search DaemonClosedLoopOCRToken --ipc auto --top-k 20
require_text "$ocr_search_out" "results: 1"
reject_paths "$ocr_search_out"

wait_for_semantic_daemon_search
require_text "$semantic_search_out" "results: 3"
reject_text "$semantic_search_out" "SemanticOnlyToken" "semantic raw query"
reject_paths "$semantic_search_out"

hybrid_search_out="$tmpdir/hybrid-search-ipc.out"
run_cli "hybrid search ipc" "$hybrid_search_out" search SemanticOnlyToken --ipc auto --mode hybrid --top-k 20
require_text "$hybrid_search_out" "results: 3"
reject_text "$hybrid_search_out" "SemanticOnlyToken" "hybrid raw query"
reject_paths "$hybrid_search_out"

doc_id="$(awk '/^doc_id: / { print $2; exit }' "$search_out")"
if [ -z "$doc_id" ]; then
  fail "daemon closed-loop search did not print a document id"
fi

detail_out="$tmpdir/detail-ipc.out"
run_cli "detail ipc" "$detail_out" detail --doc-id "$doc_id" --ipc auto
require_text "$detail_out" "resume detail"
require_text "$detail_out" "doc_id: $doc_id"
require_text "$detail_out" "document status: searchable"
require_text "$detail_out" "fields:"
require_text "$detail_out" "snippet:"
reject_text "$detail_out" "DaemonClosedLoopOCRToken" "OCR text"
reject_paths "$detail_out"

delete_out="$tmpdir/delete-ipc.out"
run_cli "delete ipc" "$delete_out" delete --doc-id "$doc_id" --ipc auto
require_text "$delete_out" "delete completed"
require_text "$delete_out" "doc_id: $doc_id"
require_text "$delete_out" "status: deleted"
require_text "$delete_out" "index rebuilt: true"
require_text "$delete_out" "indexed documents: 2"
reject_paths "$delete_out"

post_delete_search_out="$tmpdir/search-ipc-after-delete.out"
run_cli "search ipc after delete" "$post_delete_search_out" search Java --ipc auto --top-k 20
require_text "$post_delete_search_out" "results: 1"
reject_text "$post_delete_search_out" "$doc_id" "deleted doc id"
reject_paths "$post_delete_search_out"

post_delete_detail_out="$tmpdir/detail-ipc-after-delete.out"
if "$CARGO_BIN" run --quiet -p resume-cli --locked -- --data-dir "$data_dir" detail --doc-id "$doc_id" --ipc auto >"$post_delete_detail_out" 2>&1; then
  fail "daemon closed-loop detail returned deleted document"
fi
require_text "$post_delete_detail_out" "daemon detail ipc returned an error"
reject_text "$post_delete_detail_out" "$doc_id" "deleted doc id"
reject_paths "$post_delete_detail_out"

stop_daemon

if [ -s "$daemon_stderr" ]; then
  fail "daemon closed-loop daemon wrote stderr"
fi

local_status_out="$tmpdir/status-local-after-workers.out"
run_cli "local status after daemon workers" "$local_status_out" status
require_text "$local_status_out" "searchable documents: 2"
require_text "$local_status_out" "ocr queue: 0"
require_text "$local_status_out" "search index: available (full-text snapshot)"
require_text "$local_status_out" "vector index: available (hnsw ann vector snapshot)"
reject_paths "$local_status_out"

require_text "$daemon_stdout" "resume-daemon foreground ready"
require_text "$daemon_stdout" "ipc status endpoint: http://127.0.0.1:"
require_text "$daemon_stdout" "import worker processed:"
require_text "$daemon_stdout" "ocr worker processed:"
require_text "$daemon_stdout" "embedding worker processed:"
reject_text "$daemon_stdout" "DaemonClosedLoopOCRToken" "OCR text"
reject_paths "$daemon_stdout"

printf '%s\n' "daemon closed-loop check passed"

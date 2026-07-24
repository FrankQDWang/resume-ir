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
    if grep -Fq "searchable documents: 2" "$status_poll_out" \
      && grep -Fq "import tasks queued: 0" "$status_poll_out" \
      && grep -Fq "search index: daemon ipc (full-text state reported by daemon)" "$status_poll_out"; then
      cp "$status_poll_out" "$ready_status_out"
      return 0
    fi
    reject_paths "$status_poll_out"
    count=$((count + 1))
    sleep 0.25
  done
  fail "daemon closed-loop daemon did not expose its seeded core store"
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
daemon_stdout="$tmpdir/daemon.out"
daemon_stderr="$tmpdir/daemon.err"
ready_status_out="$tmpdir/status-ready.out"
daemon_pid=""

seed_import_out="$tmpdir/seed-import.out"
run_cli "seed local import" "$seed_import_out" import --root "$fixture_root"
require_text "$seed_import_out" "import task submitted"
require_text "$seed_import_out" "status: completed"
require_text "$seed_import_out" "searchable documents: 2"
reject_paths "$seed_import_out"

"$CARGO_BIN" run --quiet -p resume-daemon --locked -- \
  --data-dir "$data_dir" \
  run \
  --foreground \
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

wait_for_indexed_daemon_status
require_text "$ready_status_out" "searchable documents: 2"
require_text "$ready_status_out" "import tasks queued: 0"
require_text "$ready_status_out" "search index: daemon ipc (full-text state reported by daemon)"
reject_paths "$ready_status_out"

import_rejected_out="$tmpdir/import-ipc-rejected.out"
if "$CARGO_BIN" run --quiet -p resume-cli --locked -- --data-dir "$data_dir" import --ipc auto --root "$fixture_root" >"$import_rejected_out" 2>&1; then
  fail "daemon closed-loop import unexpectedly accepted without an attested runtime"
fi
require_text "$import_rejected_out" "daemon import ipc capability unavailable"
reject_paths "$import_rejected_out"

search_out="$tmpdir/search-ipc.out"
run_cli "search ipc" "$search_out" search Java --ipc auto --top-k 20
require_text "$search_out" "results: 2"
reject_text "$search_out" "query:" "raw query label"
reject_paths "$search_out"

doc_id="$(awk '/^doc_id: / { print $2; exit }' "$search_out")"
version_id="$(awk '/^version_id: / { print $2; exit }' "$search_out")"
visible_epoch="$(awk '/^visible_epoch: / { print $2; exit }' "$search_out")"
if [ -z "$doc_id" ] || [ -z "$version_id" ] || [ -z "$visible_epoch" ]; then
  fail "daemon closed-loop search did not print a complete selection"
fi

detail_out="$tmpdir/detail-ipc.out"
run_cli "detail ipc" "$detail_out" detail --doc-id "$doc_id" --version-id "$version_id" --visible-epoch "$visible_epoch" --ipc auto
require_text "$detail_out" "resume detail"
require_text "$detail_out" "doc_id: $doc_id"
require_text "$detail_out" "version_id: $version_id"
require_text "$detail_out" "visible_epoch: $visible_epoch"
require_text "$detail_out" "fields:"
require_text "$detail_out" "snippet:"
reject_paths "$detail_out"

stop_daemon

if [ -s "$daemon_stderr" ]; then
  fail "daemon closed-loop daemon wrote stderr"
fi

local_status_out="$tmpdir/status-local-after-daemon.out"
run_cli "local status after daemon" "$local_status_out" status
require_text "$local_status_out" "searchable documents: 2"
require_text "$local_status_out" "search index: available (database Ready full-text snapshot)"
reject_paths "$local_status_out"

require_text "$daemon_stdout" "resume-daemon foreground ready"
require_text "$daemon_stdout" "ipc status endpoint: http://127.0.0.1:"
reject_text "$daemon_stdout" "import worker processed:" "background import worker summary"
reject_text "$daemon_stdout" "ocr worker processed:" "background OCR worker summary"
reject_paths "$daemon_stdout"

printf '%s\n' "daemon closed-loop check passed"

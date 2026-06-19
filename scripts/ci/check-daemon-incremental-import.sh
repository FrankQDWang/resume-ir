#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "daemon incremental import missing expected output"
  fi
}

require_text() {
  file="$1"
  text="$2"
  require_file "$file"
  if ! grep -Fq -- "$text" "$file"; then
    fail "daemon incremental import output is missing required evidence: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  label="$3"
  require_file "$file"
  if [ -n "$text" ] && grep -Fq -- "$text" "$file"; then
    fail "daemon incremental import output leaked $label"
  fi
}

reject_regex() {
  file="$1"
  pattern="$2"
  label="$3"
  require_file "$file"
  if grep -Eq -- "$pattern" "$file"; then
    fail "daemon incremental import output leaked $label"
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
    fail "daemon incremental import failed at step: $label"
  fi
  if [ -s "$stderr_file" ]; then
    if [ "${RESUME_IR_CLOSED_LOOP_DEBUG:-0}" = "1" ]; then
      sed -n '1,80p' "$stdout_file" >&2 || true
      sed -n '1,80p' "$stderr_file" >&2 || true
    fi
    fail "daemon incremental import command wrote stderr at step: $label"
  fi
}

reject_paths() {
  file="$1"
  reject_text "$file" "$tmpdir" "temporary root path"
  reject_text "$file" "$data_dir" "private data-dir path"
  reject_text "$file" "$fixture_root" "fixture root path"
  reject_text "$file" "$canonical_fixture_root" "canonical fixture root path"
  reject_text "$file" "$initial_resume" "initial resume path"
  reject_text "$file" "$new_resume" "new resume path"
  reject_regex "$file" '/Users/|/home/|[A-Za-z]:\\' "absolute local path"
}

create_docx() {
  output_path="$1"
  text="$2"
  python3 - "$output_path" "$text" <<'PY'
import html
import sys
import zipfile

output_path = sys.argv[1]
text = html.escape(sys.argv[2])
content_types = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>
"""
rels = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>
"""
document = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>{text}</w:t></w:r></w:p>
  </w:body>
</w:document>
"""
with zipfile.ZipFile(output_path, "w", zipfile.ZIP_DEFLATED) as archive:
    archive.writestr("[Content_Types].xml", content_types)
    archive.writestr("_rels/.rels", rels)
    archive.writestr("word/document.xml", document)
PY
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

wait_for_search_results() {
  label="$1"
  token="$2"
  expected="$3"
  output_file="$4"
  count=0
  while [ "$count" -lt 80 ]; do
    poll_out="$tmpdir/search-$label-$count.out"
    run_cli "search $label" "$poll_out" search "$token" --top-k 20
    reject_paths "$poll_out"
    if grep -Fq "results: $expected" "$poll_out"; then
      cp "$poll_out" "$output_file"
      return 0
    fi
    count=$((count + 1))
    sleep 0.25
  done
  fail "daemon incremental import search did not reach expected result count for $label"
}

CARGO_BIN="${CARGO:-cargo}"
if ! command -v "$CARGO_BIN" >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi
if ! command -v python3 >/dev/null 2>&1; then
  fail "daemon incremental import check requires python3"
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-daemon-incremental.XXXXXX")"
trap cleanup EXIT HUP INT TERM

data_dir="$tmpdir/PRIVATE_DAEMON_INCREMENTAL_DATA"
fixture_root="$tmpdir/PRIVATE_DAEMON_INCREMENTAL_ROOT"
initial_resume="$fixture_root/incremental-initial.docx"
new_resume="$fixture_root/incremental-new.docx"
daemon_stdout="$tmpdir/daemon.out"
daemon_stderr="$tmpdir/daemon.err"
daemon_pid=""

mkdir -p "$fixture_root"
create_docx "$initial_resume" "InitialIncrementalToken candidate with Rust backend experience."
canonical_fixture_root="$(cd "$fixture_root" && pwd -P)"

import_out="$tmpdir/import.out"
run_cli "initial import" "$import_out" import --root "$fixture_root"
require_text "$import_out" "import task submitted"
require_text "$import_out" "status: completed"
require_text "$import_out" "files discovered: 1"
require_text "$import_out" "searchable documents: 1"
reject_paths "$import_out"

initial_search_out="$tmpdir/initial-search.out"
run_cli "initial search" "$initial_search_out" search InitialIncrementalToken --top-k 20
require_text "$initial_search_out" "results: 1"
reject_paths "$initial_search_out"

"$CARGO_BIN" run --quiet -p resume-daemon --locked -- \
  --data-dir "$data_dir" \
  run \
  --foreground \
  --work-imports \
  --watch-import-roots \
  --worker-interval-ms 25 \
  --max-worker-ticks 600 \
  >"$daemon_stdout" 2>"$daemon_stderr" &
daemon_pid=$!

sleep 1
create_docx "$initial_resume" "WatcherUpdatedToken refreshed candidate with Rust backend experience."
create_docx "$new_resume" "IncrementalNewCandidateToken candidate with Java platform experience."
sleep 1
create_docx "$initial_resume" "WatcherUpdatedToken refreshed candidate with Rust backend and distributed systems experience."

updated_search_out="$tmpdir/updated-search.out"
wait_for_search_results "updated" "WatcherUpdatedToken" 1 "$updated_search_out"

new_search_out="$tmpdir/new-search.out"
wait_for_search_results "new" "IncrementalNewCandidateToken" 1 "$new_search_out"

stale_search_out="$tmpdir/stale-search.out"
wait_for_search_results "stale" "InitialIncrementalToken" 0 "$stale_search_out"

status_out="$tmpdir/status.out"
run_cli "post-incremental status" "$status_out" status --watch-import
require_text "$status_out" "searchable documents: 2"
require_text "$status_out" "import tasks queued: 0"
require_text "$status_out" "search index: available (full-text snapshot)"
reject_paths "$status_out"

stop_daemon

if [ -s "$daemon_stderr" ]; then
  fail "daemon incremental import daemon wrote stderr"
fi
require_text "$daemon_stdout" "resume-daemon foreground ready"
require_text "$daemon_stdout" "import watcher active roots: 1"
require_text "$daemon_stdout" "import watcher requeued imports: 1"
require_text "$daemon_stdout" "import worker processed:"
reject_paths "$daemon_stdout"

printf '%s\n' "daemon incremental import check passed"

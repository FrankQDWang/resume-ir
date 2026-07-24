#!/usr/bin/env sh
set -eu

CARGO_BIN="${CARGO:-cargo}"
if ! command -v "$CARGO_BIN" >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi

"$CARGO_BIN" test -p resume-daemon --locked \
  --test s4_daemon \
  --features native-runtime-tests \
  foreground_import_watcher_requeues_completed_root_after_word_and_pdf_change_without_path_leak \
  -- --exact --test-threads=1

printf '%s\n' "daemon incremental import check passed"

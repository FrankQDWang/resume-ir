#!/usr/bin/env sh
set -eu

CARGO_BIN="${CARGO:-cargo}"
if ! command -v "$CARGO_BIN" >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi

"$CARGO_BIN" metadata --no-deps --locked
"$CARGO_BIN" fmt --check
"$CARGO_BIN" clippy --workspace --all-targets --all-features --locked -- -D warnings
"$CARGO_BIN" test --workspace --locked
./scripts/ci/check-licenses.sh
./scripts/ci/guard-public-repo.sh

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
./scripts/ci/check-cli-closed-loop.sh
./scripts/ci/check-daemon-closed-loop.sh
./scripts/ci/check-benchmark-smoke.sh
./scripts/ci/check-licenses.sh
./scripts/ci/check-runbooks.sh
./scripts/ci/check-current-stage-validation.sh
./scripts/ci/check-workflows.sh
./scripts/ci/check-release-readiness.sh
./scripts/ci/check-release-artifacts.sh
./scripts/ci/check-signing-evidence.sh
./scripts/ci/check-notarization-evidence.sh
./scripts/ci/check-release-sbom.sh
./scripts/ci/check-macos-package.sh
./scripts/ci/check-macos-installer-evidence.sh
./scripts/ci/check-windows-package.sh
./scripts/ci/check-windows-installer-evidence.sh
./scripts/ci/check-windows-service-evidence.sh
./scripts/ci/guard-public-repo.sh

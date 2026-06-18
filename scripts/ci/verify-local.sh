#!/usr/bin/env sh
set -eu

CARGO_BIN="${CARGO:-}"
if [ -z "$CARGO_BIN" ] && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi
if [ -z "$CARGO_BIN" ]; then
  CARGO_BIN=cargo
fi
if ! "$CARGO_BIN" --version >/dev/null 2>&1; then
  printf '%s\n' "verify-local requires cargo" >&2
  exit 1
fi

"$CARGO_BIN" metadata --no-deps --locked
"$CARGO_BIN" fmt --check
"$CARGO_BIN" clippy --workspace --all-targets --all-features --locked -- -D warnings
"$CARGO_BIN" test --workspace --exclude benchmark-runner --exclude embedder --exclude resume-cli --locked
"$CARGO_BIN" test -p resume-cli --locked -- --test-threads=1
"$CARGO_BIN" test -p embedder --locked -- --test-threads=1
"$CARGO_BIN" test -p benchmark-runner --locked -- --test-threads=1
./scripts/ci/check-cli-closed-loop.sh
./scripts/ci/check-daemon-closed-loop.sh
./scripts/ci/check-benchmark-smoke.sh
./scripts/ci/check-licenses.sh
./scripts/ci/check-runtime-bundle-policy.sh
./scripts/ci/check-runbooks.sh
./scripts/ci/check-current-stage-observability.sh
./scripts/ci/check-local-embedding-runtime.sh
./scripts/ci/check-local-ocr-runtime.sh
./scripts/ci/check-local-diagnostics-release-evidence.sh
./scripts/ci/check-local-quality-release-evidence.sh
./scripts/ci/check-current-stage-validation.sh
./scripts/ci/check-current-stage-handoff.sh
./scripts/ci/check-workflows.sh
./scripts/ci/check-release-readiness.sh
./scripts/ci/check-release-artifacts.sh
./scripts/ci/check-runtime-bundle-manifest.sh
./scripts/ci/check-release-publication-evidence.sh
./scripts/ci/check-signing-evidence.sh
./scripts/ci/check-notarization-evidence.sh
./scripts/ci/check-release-sbom.sh
./scripts/ci/check-macos-package.sh
./scripts/ci/check-macos-installer-evidence.sh
./scripts/ci/check-windows-package.sh
./scripts/ci/check-windows-installer-evidence.sh
./scripts/ci/check-windows-service-evidence.sh
./scripts/ci/guard-public-repo.sh

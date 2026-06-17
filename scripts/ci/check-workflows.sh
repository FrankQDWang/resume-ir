#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required workflow policy file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "workflow policy $file is missing required text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  if grep -Fq -- "$text" "$file"; then
    fail "workflow policy $file contains deprecated text: $text"
  fi
}

pr_workflow=".github/workflows/pr.yml"
nightly_workflow=".github/workflows/bench-nightly.yml"
platform_workflow=".github/workflows/ci-platform.yml"
release_workflow=".github/workflows/release.yml"
verify_script="scripts/ci/verify-local.sh"
cli_closed_loop_script="scripts/ci/check-cli-closed-loop.sh"
daemon_closed_loop_script="scripts/ci/check-daemon-closed-loop.sh"
benchmark_smoke_script="scripts/ci/check-benchmark-smoke.sh"
current_stage_handoff_script="scripts/ci/check-current-stage-handoff.sh"
local_ocr_runtime_script="scripts/ci/check-local-ocr-runtime.sh"

for file in "$pr_workflow" "$nightly_workflow" "$platform_workflow" "$release_workflow" "$verify_script" "$cli_closed_loop_script" "$daemon_closed_loop_script" "$benchmark_smoke_script" "$current_stage_handoff_script" "$local_ocr_runtime_script"; do
  require_file "$file"
done

require_text "$pr_workflow" "CLI closed-loop check"
require_text "$pr_workflow" "./scripts/ci/check-cli-closed-loop.sh"
require_text "$pr_workflow" "Daemon closed-loop check"
require_text "$pr_workflow" "./scripts/ci/check-daemon-closed-loop.sh"
require_text "$pr_workflow" "Benchmark smoke"
require_text "$pr_workflow" "./scripts/ci/check-benchmark-smoke.sh"
require_text "$pr_workflow" "Current-stage handoff check"
require_text "$pr_workflow" "./scripts/ci/check-current-stage-handoff.sh"
require_text "$pr_workflow" "check-workflows.sh"
require_text "$pr_workflow" "actions/checkout@v6"

require_text "$nightly_workflow" "resume-benchmark --locked -- synthetic-query"
require_text "$nightly_workflow" "resume-benchmark --locked -- gate"
require_text "$nightly_workflow" "resume-benchmark --locked -- ocr-throughput"
require_text "$nightly_workflow" "resume-benchmark --locked -- ocr-gate"
require_text "$nightly_workflow" "ocr-benchmark-smoke.json"
require_text "$nightly_workflow" "resume-benchmark --locked -- vector-quality"
require_text "$nightly_workflow" "resume-benchmark --locked -- vector-gate"
require_text "$nightly_workflow" "vector-benchmark-smoke.json"
require_text "$nightly_workflow" "--allow-synthetic"
require_text "$nightly_workflow" "actions/checkout@v6"
require_text "$nightly_workflow" "actions/upload-artifact@v7"
require_text "$nightly_workflow" "Check benchmark artifact boundary"
require_text "$nightly_workflow" "nightly benchmark smoke report leaked a local path or runtime-data marker"
require_text "$nightly_workflow" "nightly OCR benchmark smoke report leaked a local path or runtime-data marker"
require_text "$nightly_workflow" "nightly vector benchmark smoke report leaked a local path or runtime-data marker"

require_text "$platform_workflow" "pull_request"
require_text "$platform_workflow" "macos-latest"
require_text "$platform_workflow" "windows-latest"
require_text "$platform_workflow" "cargo build --workspace --locked"
require_text "$platform_workflow" "cargo test --workspace --locked"
require_text "$platform_workflow" "actions/checkout@v6"

require_text "$verify_script" "./scripts/ci/check-workflows.sh"
require_text "$verify_script" "./scripts/ci/check-cli-closed-loop.sh"
require_text "$verify_script" "./scripts/ci/check-daemon-closed-loop.sh"
require_text "$verify_script" "./scripts/ci/check-benchmark-smoke.sh"
require_text "$verify_script" "./scripts/ci/check-current-stage-handoff.sh"
require_text "$verify_script" "./scripts/ci/check-local-ocr-runtime.sh"
require_text "$verify_script" "./scripts/ci/check-release-readiness.sh"
require_text "$verify_script" "./scripts/ci/check-release-artifacts.sh"
require_text "$verify_script" "./scripts/ci/check-release-publication-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-signing-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-notarization-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-release-sbom.sh"
require_text "$verify_script" "./scripts/ci/check-macos-package.sh"
require_text "$verify_script" "./scripts/ci/check-macos-installer-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-windows-package.sh"
require_text "$verify_script" "./scripts/ci/check-windows-installer-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-windows-service-evidence.sh"

require_text "$cli_closed_loop_script" "resume-cli"
require_text "$cli_closed_loop_script" "import --root"
require_text "$cli_closed_loop_script" "search Java --top-k 20"
require_text "$cli_closed_loop_script" "search Java --degree bachelor --skills-any java --top-k 20"
require_text "$cli_closed_loop_script" "ocr-worker --once --command"
require_text "$cli_closed_loop_script" "embed-worker --once --command"
require_text "$cli_closed_loop_script" "--mode semantic"
require_text "$cli_closed_loop_script" "--mode hybrid"
require_text "$cli_closed_loop_script" "export-diagnostics --redact"
require_text "$cli_closed_loop_script" "CLIClosedLoopOCRToken"
require_text "$cli_closed_loop_script" "cli closed-loop check passed"
reject_text "$cli_closed_loop_script" 'require_text "$fulltext_out" "synthetic-java-platform.pdf"'
reject_text "$cli_closed_loop_script" 'require_text "$fulltext_out" "synthetic-java-engineer.docx"'
reject_text "$cli_closed_loop_script" 'require_text "$field_out" "synthetic-java-platform.pdf"'
reject_text "$cli_closed_loop_script" 'require_text "$field_out" "synthetic-java-engineer.docx"'
reject_text "$cli_closed_loop_script" 'require_text "$ocr_search_out" "synthetic-scanned-resume.pdf"'
reject_text "$cli_closed_loop_script" 'require_text "$semantic_out" "synthetic-java-platform.pdf"'
reject_text "$cli_closed_loop_script" 'require_text "$semantic_out" "synthetic-java-engineer.docx"'
reject_text "$cli_closed_loop_script" 'require_text "$semantic_out" "synthetic-scanned-resume.pdf"'
reject_text "$cli_closed_loop_script" 'require_text "$hybrid_out" "synthetic-java-platform.pdf"'
reject_text "$cli_closed_loop_script" 'require_text "$hybrid_out" "synthetic-java-engineer.docx"'
reject_text "$cli_closed_loop_script" 'require_text "$hybrid_out" "synthetic-scanned-resume.pdf"'

require_text "$daemon_closed_loop_script" "resume-daemon"
require_text "$daemon_closed_loop_script" "--work-imports"
require_text "$daemon_closed_loop_script" "--work-ocr"
require_text "$daemon_closed_loop_script" "--work-embeddings"
require_text "$daemon_closed_loop_script" "--work-index"
require_text "$daemon_closed_loop_script" "import --ipc auto --root"
require_text "$daemon_closed_loop_script" "status --ipc auto"
require_text "$daemon_closed_loop_script" "search Java --ipc auto --top-k 20"
require_text "$daemon_closed_loop_script" "search DaemonClosedLoopOCRToken --ipc auto --top-k 20"
require_text "$daemon_closed_loop_script" "--mode semantic"
require_text "$daemon_closed_loop_script" "--mode hybrid"
require_text "$daemon_closed_loop_script" "detail --doc-id"
require_text "$daemon_closed_loop_script" "delete --doc-id"
require_text "$daemon_closed_loop_script" "daemon closed-loop check passed"
reject_text "$daemon_closed_loop_script" 'require_text "$search_out" "synthetic-java-platform.pdf"'
reject_text "$daemon_closed_loop_script" 'require_text "$search_out" "synthetic-java-engineer.docx"'
reject_text "$daemon_closed_loop_script" 'require_text "$ocr_search_out" "synthetic-scanned-resume.pdf"'
reject_text "$daemon_closed_loop_script" 'require_text "$semantic_search_out" "synthetic-java-platform.pdf"'
reject_text "$daemon_closed_loop_script" 'require_text "$semantic_search_out" "synthetic-java-engineer.docx"'
reject_text "$daemon_closed_loop_script" 'require_text "$semantic_search_out" "synthetic-scanned-resume.pdf"'
reject_text "$daemon_closed_loop_script" 'require_text "$hybrid_search_out" "synthetic-java-platform.pdf"'
reject_text "$daemon_closed_loop_script" 'require_text "$hybrid_search_out" "synthetic-java-engineer.docx"'
reject_text "$daemon_closed_loop_script" 'require_text "$hybrid_search_out" "synthetic-scanned-resume.pdf"'

require_text "$benchmark_smoke_script" "resume-benchmark --locked -- synthetic-query"
require_text "$benchmark_smoke_script" "resume-benchmark --locked -- gate"
require_text "$benchmark_smoke_script" "resume-benchmark --locked -- ocr-throughput"
require_text "$benchmark_smoke_script" "resume-benchmark --locked -- ocr-gate"
require_text "$benchmark_smoke_script" "resume-benchmark --locked -- vector-quality"
require_text "$benchmark_smoke_script" "resume-benchmark --locked -- vector-gate"
require_text "$benchmark_smoke_script" "benchmark-smoke.json"
require_text "$benchmark_smoke_script" "ocr-benchmark-smoke.json"
require_text "$benchmark_smoke_script" "vector-benchmark-smoke.json"
require_text "$benchmark_smoke_script" "--allow-synthetic"
require_text "$benchmark_smoke_script" "benchmark smoke check passed"

require_text "$local_ocr_runtime_script" "scripts/local/prepare-local-ocr-runtime-manifest.sh"
require_text "$local_ocr_runtime_script" "--tesseract-command"
require_text "$local_ocr_runtime_script" "--pdftoppm-command"
require_text "$local_ocr_runtime_script" "--language-pack"
require_text "$local_ocr_runtime_script" "--reviewed"
require_text "$local_ocr_runtime_script" "legal review is incomplete"
require_text "$local_ocr_runtime_script" "real resume-cli OCR manifest check passed"
require_text "$local_ocr_runtime_script" "local OCR runtime check passed"

require_text "$current_stage_handoff_script" "scripts/local/summarize-current-stage-validation.py"
require_text "$current_stage_handoff_script" "resume-ir.current-stage-smoke-summary.v1"
require_text "$current_stage_handoff_script" "resume-ir.current-stage-blocked-summary.v1"
require_text "$current_stage_handoff_script" "resume-ir.current-stage-handoff.v1"
require_text "$current_stage_handoff_script" "current-stage handoff check passed"
require_text "$current_stage_handoff_script" "PRIVATE-current-stage"

require_text "scripts/ci/check-current-stage-validation.sh" '"doctor", "status": "success"'
require_text "scripts/ci/check-current-stage-validation.sh" '"doctor.out"'
require_text "scripts/local/run-current-stage-validation.sh" 'current-stage validation: doctor'

require_text "$release_workflow" "scripts/release/create-artifact-manifest.sh"
require_text "$release_workflow" "scripts/release/create-signing-evidence.sh"
require_text "$release_workflow" "scripts/release/create-sbom.sh"
require_text "$release_workflow" "scripts/release/create-release-publication-evidence.sh"
require_text "$release_workflow" "scripts/release/publish-github-release.sh"
require_text "$release_workflow" "./scripts/ci/check-release-readiness.sh"
require_text "$release_workflow" "scripts/release/create-macos-package.sh"
require_text "$release_workflow" "scripts/release/create-macos-installer-evidence.sh"
require_text "$release_workflow" "scripts/release/run-macos-installer-lifecycle.sh"
require_text "$release_workflow" "scripts/release/create-notarization-evidence.sh"
require_text "$release_workflow" "scripts/release/verify-macos-dmg.sh"
require_text "$release_workflow" "scripts/release/create-windows-package.ps1"
require_text "$release_workflow" "scripts/release/create-windows-installer-evidence.sh"
require_text "$release_workflow" "scripts/release/run-windows-installer-lifecycle.ps1"
require_text "$release_workflow" "scripts/release/create-windows-service-evidence.sh"
require_text "$release_workflow" "scripts/release/run-windows-service-lifecycle.ps1"
require_text "$release_workflow" "release-artifacts.json"
require_text "$release_workflow" "signing-evidence.json"
require_text "$release_workflow" "release-sbom.json"
require_text "$release_workflow" "release-publication-evidence.json"
require_text "$release_workflow" "github-release-publication-gate.json"
require_text "$release_workflow" "release artifact manifest leaked a local path or runtime-data marker"
require_text "$release_workflow" "signing evidence manifest leaked a local path or runtime-data marker"
require_text "$release_workflow" "release SBOM leaked a local path or runtime-data marker"
require_text "$release_workflow" "release publication evidence manifest leaked a local path or runtime-data marker"
require_text "$release_workflow" "GitHub Release publication gate leaked a local path or runtime-data marker"
require_text "$release_workflow" "macos-package.json"
require_text "$release_workflow" "macos-installer-evidence.json"
require_text "$release_workflow" "macos-installer-lifecycle-dry-run.json"
require_text "$release_workflow" "notarization-evidence.json"
require_text "$release_workflow" "macos-package-dry-run"
require_text "$release_workflow" "macos-latest"
require_text "$release_workflow" "macOS package manifest leaked a local path or runtime-data marker"
require_text "$release_workflow" "macOS installer evidence manifest leaked a local path or runtime-data marker"
require_text "$release_workflow" "macOS installer lifecycle dry-run plan leaked a local path or runtime-data marker"
require_text "$release_workflow" "macOS notarization evidence manifest leaked a local path or runtime-data marker"
require_text "scripts/release/verify-macos-dmg.sh" "hdiutil verify"
require_text "$release_workflow" "windows-package.json"
require_text "$release_workflow" "windows-installer-evidence.json"
require_text "$release_workflow" "windows-installer-lifecycle-dry-run.json"
require_text "$release_workflow" "windows-service-evidence.json"
require_text "$release_workflow" "windows-service-lifecycle-dry-run.json"
require_text "$release_workflow" "windows-package-dry-run"
require_text "$release_workflow" "windows-latest"
require_text "$release_workflow" "dotnet tool install --global wix --version 6.0.2"
require_text "$release_workflow" 'resume-ir-${{ inputs.version }}-windows.msi'
require_text "$release_workflow" "Windows package manifest leaked a local path or runtime-data marker"
require_text "$release_workflow" "Windows installer evidence manifest leaked a local path or runtime-data marker"
require_text "$release_workflow" "Windows installer lifecycle dry-run plan leaked a local path or runtime-data marker"
require_text "$release_workflow" "Windows service evidence manifest leaked a local path or runtime-data marker"
require_text "$release_workflow" "Windows service lifecycle dry-run plan leaked a local path or runtime-data marker"
require_text "$release_workflow" "actions/upload-artifact"
require_text "$release_workflow" "actions/checkout@v6"
require_text "$release_workflow" "actions/upload-artifact@v7"
require_text "$release_workflow" "Packaging, signing, notarization"

for file in "$pr_workflow" "$nightly_workflow" "$platform_workflow" "$release_workflow"; do
  reject_text "$file" "actions/checkout@v4"
  reject_text "$file" "actions/upload-artifact@v4"
done

printf '%s\n' "workflow check passed"

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

reject_file() {
  if [ -e "$1" ]; then
    fail "retired GitHub CI workflow must remain absent: $1"
  fi
}

release_workflow=".github/workflows/release.yml"
pr_lite_workflow=".github/workflows/pr-lite.yml"
github_config_script="scripts/ci/configure-github-repo.sh"
verify_script="scripts/ci/verify-local.sh"
parallel_verify_script="scripts/ci/verify-local-parallel.py"
parallel_verify_support="scripts/ci/verify_local_parallel_support.py"
parallel_verify_manifest="scripts/ci/verify-local-parallel-manifest.json"
parallel_verify_test="scripts/ci/test-verify-local-parallel.py"
search_runtime_boundary_script="scripts/ci/check-search-runtime-boundary.py"
governance_mutation_script="scripts/ci/test-governance-contract-mutations.py"
cli_closed_loop_script="scripts/ci/check-cli-closed-loop.sh"
daemon_closed_loop_script="scripts/ci/check-daemon-closed-loop.sh"
daemon_incremental_script="scripts/ci/check-daemon-incremental-import.sh"
benchmark_smoke_script="scripts/ci/check-benchmark-smoke.sh"
runtime_bundle_policy_script="scripts/ci/check-runtime-bundle-policy.sh"
runtime_bundle_manifest_script="scripts/ci/check-runtime-bundle-manifest.sh"
runtime_bundle_payload_script="scripts/ci/check-runtime-bundle-payload.sh"
runtime_bundle_sbom_script="scripts/ci/check-runtime-bundle-sbom.sh"
runtime_bundle_package_script="scripts/ci/check-runtime-bundle-package.sh"
current_stage_handoff_script="scripts/ci/check-current-stage-handoff.sh"
current_stage_validation_script="scripts/ci/check-current-stage-validation.sh"
current_stage_observability_script="scripts/ci/check-current-stage-observability.sh"
local_ocr_runtime_script="scripts/ci/check-local-ocr-runtime.sh"
local_diagnostics_evidence_script="scripts/ci/check-local-diagnostics-release-evidence.sh"
local_quality_evidence_script="scripts/ci/check-local-quality-release-evidence.sh"

for file in \
  .github/workflows/pr.yml \
  .github/workflows/security.yml \
  .github/workflows/ci-platform.yml \
  .github/workflows/bench-nightly.yml \
  .github/workflows/model-eval.yml
do
  reject_file "$file"
done

unexpected_workflows=$(find .github/workflows -maxdepth 1 -type f ! -name release.yml ! -name pr-lite.yml -print)
if [ -n "$unexpected_workflows" ]; then
  printf '%s\n' "$unexpected_workflows" >&2
  fail "only the release and PR Lite GitHub workflows are allowed"
fi

for file in "$release_workflow" "$pr_lite_workflow" "$github_config_script" "$verify_script" "$parallel_verify_script" "$parallel_verify_support" "$parallel_verify_manifest" "$parallel_verify_test" "$search_runtime_boundary_script" "$governance_mutation_script" "$cli_closed_loop_script" "$daemon_closed_loop_script" "$daemon_incremental_script" "$benchmark_smoke_script" "$runtime_bundle_policy_script" "$runtime_bundle_manifest_script" "$runtime_bundle_payload_script" "$runtime_bundle_sbom_script" "$runtime_bundle_package_script" "$current_stage_handoff_script" "$current_stage_validation_script" "$current_stage_observability_script" "$local_ocr_runtime_script" "$local_diagnostics_evidence_script" "$local_quality_evidence_script"; do
  require_file "$file"
done

require_text "$release_workflow" "workflow_dispatch:"
reject_text "$release_workflow" "pull_request:"
reject_text "$release_workflow" "schedule:"
reject_text "$release_workflow" "push:"

require_text "$pr_lite_workflow" "name: PR Lite"
require_text "$pr_lite_workflow" "pull_request:"
require_text "$pr_lite_workflow" "- main"
require_text "$pr_lite_workflow" "contents: read"
require_text "$pr_lite_workflow" "runs-on: macos-latest"
require_text "$pr_lite_workflow" "name: contract-and-unit"
require_text "$pr_lite_workflow" "cargo metadata --no-deps --locked --format-version 1"
require_text "$pr_lite_workflow" "cargo fmt --all -- --check"
require_text "$pr_lite_workflow" "cargo test -p embedding-protocol -p embedder -p resume-embedding-runtime --locked"
require_text "$pr_lite_workflow" "cargo clippy -p embedding-protocol -p embedder -p resume-embedding-runtime --all-targets --locked -- -D warnings"
require_text "$pr_lite_workflow" "python3 scripts/ci/check-performance-contracts.py"
require_text "$pr_lite_workflow" "python3 scripts/ci/check-autonomous-goal.py"
require_text "$pr_lite_workflow" "python3 scripts/ci/check-loop-state.py"
require_text "$pr_lite_workflow" "python3 scripts/ci/test-governance-contract-mutations.py"
require_text "$pr_lite_workflow" "python3 scripts/ci/check-pr-budget.py"
require_text "$pr_lite_workflow" "git diff --check origin/\${{ github.base_ref }}...HEAD"
require_text "$pr_lite_workflow" "./scripts/ci/guard-public-repo.sh"
reject_text "$pr_lite_workflow" "workflow_dispatch:"
reject_text "$pr_lite_workflow" "schedule:"
reject_text "$pr_lite_workflow" "push:"
reject_text "$pr_lite_workflow" "ubuntu-latest"
reject_text "$pr_lite_workflow" "windows-latest"

require_text "$github_config_script" '"required_status_checks": {'
require_text "$github_config_script" '"strict": true'
require_text "$github_config_script" '"PR Lite / contract-and-unit"'

require_text "$verify_script" "./scripts/ci/check-workflows.sh"
require_text "$verify_script" "python3 scripts/ci/check-search-runtime-boundary.py"
require_text "$verify_script" "python3 scripts/ci/test-governance-contract-mutations.py"
require_text "$verify_script" "python3 scripts/ci/test-verify-local-parallel.py"
require_text "$verify_script" "./scripts/ci/check-cli-closed-loop.sh"
require_text "$verify_script" "./scripts/ci/check-daemon-closed-loop.sh"
require_text "$verify_script" "./scripts/ci/check-daemon-incremental-import.sh"
require_text "$verify_script" "./scripts/ci/check-benchmark-smoke.sh"
require_text "$verify_script" "./scripts/ci/check-runtime-bundle-policy.sh"
require_text "$verify_script" "./scripts/ci/check-current-stage-handoff.sh"
require_text "$verify_script" "./scripts/ci/check-current-stage-observability.sh"
require_text "$verify_script" "./scripts/ci/check-local-ocr-runtime.sh"
require_text "$verify_script" "./scripts/ci/check-local-diagnostics-release-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-local-quality-release-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-release-readiness.sh"
require_text "$verify_script" "./scripts/ci/check-release-artifacts.sh"
require_text "$verify_script" "./scripts/ci/check-runtime-bundle-manifest.sh"
require_text "$verify_script" "./scripts/ci/check-runtime-bundle-payload.sh"
require_text "$verify_script" "./scripts/ci/check-release-publication-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-signing-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-notarization-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-release-sbom.sh"
require_text "$verify_script" "./scripts/ci/check-runtime-bundle-sbom.sh"
require_text "$verify_script" "./scripts/ci/check-runtime-bundle-package.sh"
require_text "$verify_script" "./scripts/ci/check-macos-package.sh"
require_text "$verify_script" "./scripts/ci/check-macos-installer-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-windows-package.sh"
require_text "$verify_script" "./scripts/ci/check-windows-installer-evidence.sh"
require_text "$verify_script" "./scripts/ci/check-windows-service-evidence.sh"

require_text "$search_runtime_boundary_script" 'FACADE_PACKAGE = "search-runtime"'
require_text "$search_runtime_boundary_script" 'PRODUCTION_CONSUMERS = ("resume-daemon", "resume-cli")'
require_text "$search_runtime_boundary_script" 'FORBIDDEN_DIRECT_DEPENDENCIES = {"index-fulltext", "index-vector"}'
require_text "$search_runtime_boundary_script" '"SnapshotReadLease"'
require_text "$search_runtime_boundary_script" '"FullTextIndex::open_*"'
require_text "$search_runtime_boundary_script" '"VectorSnapshotRoot"'
require_text "$search_runtime_boundary_script" '"VectorSnapshotReader"'
require_text "$search_runtime_boundary_script" '"PersistentVectorSearchIndex"'
require_text "$search_runtime_boundary_script" '"PersistentVectorIndex"'
require_text "$search_runtime_boundary_script" '"with_search_metadata_snapshot"'
require_text "$search_runtime_boundary_script" '"validated_active_projections"'
require_text "$search_runtime_boundary_script" '"latest_visible_resume_version_for_document"'
require_text "$search_runtime_boundary_script" "search-runtime boundary self-test passed"

require_text "$cli_closed_loop_script" "resume-cli"
require_text "$cli_closed_loop_script" "import --root"
require_text "$cli_closed_loop_script" "search Java --top-k 20"
require_text "$cli_closed_loop_script" "search Java --degree bachelor --skills-any java --top-k 20"
require_text "$cli_closed_loop_script" "ocr-worker --once --command"
require_text "$cli_closed_loop_script" "export-diagnostics --redact"
require_text "$cli_closed_loop_script" "CLIClosedLoopOCRToken"
require_text "$cli_closed_loop_script" "cli closed-loop check passed"
reject_text "$cli_closed_loop_script" 'require_text "$fulltext_out" "synthetic-java-platform.pdf"'
reject_text "$cli_closed_loop_script" 'require_text "$fulltext_out" "synthetic-java-engineer.docx"'
reject_text "$cli_closed_loop_script" 'require_text "$field_out" "synthetic-java-platform.pdf"'
reject_text "$cli_closed_loop_script" 'require_text "$field_out" "synthetic-java-engineer.docx"'
reject_text "$cli_closed_loop_script" 'require_text "$ocr_search_out" "synthetic-scanned-resume.pdf"'
reject_text "$cli_closed_loop_script" "embed-worker"

require_text "$daemon_closed_loop_script" "resume-daemon"
require_text "$daemon_closed_loop_script" "import --ipc auto --root"
require_text "$daemon_closed_loop_script" "daemon import ipc capability unavailable"
require_text "$daemon_closed_loop_script" "status --ipc auto"
require_text "$daemon_closed_loop_script" "search Java --ipc auto --top-k 20"
require_text "$daemon_closed_loop_script" "detail --doc-id"
require_text "$daemon_closed_loop_script" "--version-id"
require_text "$daemon_closed_loop_script" "--visible-epoch"
require_text "$daemon_closed_loop_script" "daemon closed-loop check passed"
require_text "$daemon_closed_loop_script" 'reject_text "$daemon_stdout" "import worker processed:"'
require_text "$daemon_closed_loop_script" 'reject_text "$daemon_stdout" "ocr worker processed:"'
reject_text "$daemon_closed_loop_script" "--work-embeddings"
reject_text "$daemon_closed_loop_script" "embedding worker processed:"
reject_text "$daemon_closed_loop_script" 'require_text "$daemon_stdout" "import worker processed:"'
reject_text "$daemon_closed_loop_script" 'require_text "$daemon_stdout" "ocr worker processed:"'
reject_text "$daemon_closed_loop_script" 'require_text "$search_out" "synthetic-java-platform.pdf"'
reject_text "$daemon_closed_loop_script" 'require_text "$search_out" "synthetic-java-engineer.docx"'

require_text "$daemon_incremental_script" '"$CARGO_BIN" test -p resume-daemon'
require_text "$daemon_incremental_script" '--test s4_daemon'
require_text "$daemon_incremental_script" '--features native-runtime-tests'
require_text "$daemon_incremental_script" 'foreground_import_watcher_requeues_completed_root_after_word_and_pdf_change_without_path_leak'
require_text "$daemon_incremental_script" '--exact'
require_text "$daemon_incremental_script" '--test-threads=1'
require_text "$daemon_incremental_script" "daemon incremental import check passed"

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

require_text "$runtime_bundle_policy_script" "bundled-first"
require_text "$runtime_bundle_policy_script" "external override"
require_text "$runtime_bundle_policy_script" "runtime_distribution_mode"
require_text "$runtime_bundle_policy_script" "runtime_package_binaries_included"
require_text "$runtime_bundle_policy_script" "GPL-3.0-or-later"
require_text "$runtime_bundle_policy_script" "source-offer"
require_text "$runtime_bundle_policy_script" "runtime bundle policy check passed"

require_text "$runtime_bundle_manifest_script" "scripts/release/create-runtime-bundle-manifest.sh"
require_text "$runtime_bundle_manifest_script" "release.runtime_bundle.v1"
require_text "$runtime_bundle_manifest_script" "--runtime-bundle-manifest"
require_text "$runtime_bundle_manifest_script" "runtime_bundle_manifests"
require_text "$runtime_bundle_manifest_script" "runtime bundle manifest check passed"
require_text "$runtime_bundle_payload_script" "scripts/release/assemble-runtime-bundle.sh"
require_text "$runtime_bundle_payload_script" "--runtime-bundle-dir <assembled-runtime-dir>"
require_text "$runtime_bundle_payload_script" "runtime bundle payload check passed"

require_text "$runtime_bundle_sbom_script" "scripts/release/create-sbom.sh"
require_text "$runtime_bundle_sbom_script" "--runtime-bundle-manifest"
require_text "$runtime_bundle_sbom_script" "runtime_distribution_mode=bundled"
require_text "$runtime_bundle_sbom_script" "runtime bundle SBOM check passed"
require_text "$runtime_bundle_package_script" "--runtime-bundle-manifest"
require_text "$runtime_bundle_package_script" "--runtime-bundle-dir"
require_text "$runtime_bundle_package_script" "release.runtime_package_payload.v1"
require_text "$runtime_bundle_package_script" "runtime bundle package check passed"

require_text "$local_ocr_runtime_script" "scripts/local/prepare-local-ocr-runtime-manifest.sh"
require_text "$local_ocr_runtime_script" "--tesseract-command"
require_text "$local_ocr_runtime_script" "--pdftoppm-command"
require_text "$local_ocr_runtime_script" "--language-pack"
require_text "$local_ocr_runtime_script" "--reviewed"
require_text "$local_ocr_runtime_script" "legal review is incomplete"
require_text "$local_ocr_runtime_script" "real resume-cli OCR manifest check passed"
require_text "$local_ocr_runtime_script" "local OCR runtime check passed"

require_text "$local_diagnostics_evidence_script" "export-diagnostics --redact"
require_text "$local_diagnostics_evidence_script" "--diagnostics-report"
require_text "$local_diagnostics_evidence_script" "redacted diagnostics evidence"
require_text "$local_diagnostics_evidence_script" "local diagnostics release-evidence check passed"

require_text "$local_quality_evidence_script" "scripts/local/prepare-local-quality-release-evidence.sh"
require_text "$local_quality_evidence_script" "field-quality"
require_text "$local_quality_evidence_script" "dedupe-quality"
require_text "$local_quality_evidence_script" "vector-quality"
require_text "$local_quality_evidence_script" "--reviewed"
require_text "$local_quality_evidence_script" "quality review is incomplete"
require_text "$local_quality_evidence_script" "--field-quality-report"
require_text "$local_quality_evidence_script" "--dedupe-quality-report"
require_text "$local_quality_evidence_script" "--vector-quality-report"
require_text "$local_quality_evidence_script" "real benchmark smoke: passed"
require_text "$local_quality_evidence_script" "local quality release-evidence check passed"

require_text "$current_stage_handoff_script" "scripts/local/summarize-current-stage-validation.py"
require_text "$current_stage_handoff_script" "resume-ir.current-stage-smoke-summary.v2"
require_text "$current_stage_handoff_script" "resume-ir.current-stage-blocked-summary.v2"
require_text "$current_stage_handoff_script" "resume-ir.current-stage-handoff.v1"
require_text "$current_stage_handoff_script" "current-stage handoff check passed"
require_text "$current_stage_handoff_script" "PRIVATE-current-stage"

require_text "$current_stage_validation_script" '"doctor", "status": "success"'
require_text "$current_stage_validation_script" '"doctor.out"'
require_text "$current_stage_validation_script" "validate-current-stage-observability.py --full-evidence"
require_text "$current_stage_validation_script" "validate-current-stage-observability.py --summary"
reject_text "$current_stage_validation_script" 'require_text "$ocr_backlog_summary" '"'"'"ocr_required": 8538'"'"
reject_text "$current_stage_validation_script" 'require_text "$ocr_backlog_summary" '"'"'"vector_indexed_document_count": 0'"'"
require_text "$current_stage_observability_script" "validate-current-stage-observability.py --summary"
require_text "$current_stage_observability_script" "document_count below current-stage floor"
require_text "$current_stage_observability_script" "vector_indexed_document_count is inconsistent"
require_text "$current_stage_observability_script" "forbidden observability field"
require_text "$current_stage_observability_script" "current-stage observability check passed"
require_text "scripts/local/run-current-stage-validation.sh" 'current-stage validation: doctor'

require_text "$release_workflow" "Build reviewed macOS PDFium static pack"
require_text "$release_workflow" "npm run build:macos:pdfium"
require_text "$release_workflow" "Build reviewed macOS OCR runtime pack"
require_text "$release_workflow" "npm run build:macos:ocr"
require_text "$release_workflow" "Build reviewed Windows PDFium static pack"
require_text "$release_workflow" "npm run build:windows:pdfium"
require_text "$release_workflow" "Build reviewed Windows OCR runtime pack"
require_text "$release_workflow" "npm run build:windows:ocr"
require_text "$release_workflow" "ci-synthetic-runtime-bundle"
require_text "$release_workflow" "scripts/release/create-runtime-bundle-manifest.sh"
require_text "$release_workflow" "resume-pdf-render-runtime"
require_text "$release_workflow" "LicenseRef-PDFium-Root-LICENSE"
reject_text "$release_workflow" "ubuntu-latest"
reject_text "$release_workflow" "pdftoppm"
require_text "$release_workflow" "scripts/release/create-macos-package.sh"
require_text "$release_workflow" "--runtime-bundle-manifest macos-package-dry-run/runtime-bundle-manifest.json"
require_text "$release_workflow" '--runtime-bundle-dir "$runtime_dir"'
require_text "$release_workflow" "scripts/release/create-macos-installer-evidence.sh"
require_text "$release_workflow" "scripts/release/run-macos-installer-lifecycle.sh"
require_text "$release_workflow" "scripts/release/create-notarization-evidence.sh"
require_text "$release_workflow" "scripts/release/verify-macos-dmg.sh"
require_text "$release_workflow" "scripts/release/create-windows-package.ps1"
require_text "$release_workflow" "-RuntimeBundleManifest windows-package-dry-run/runtime-bundle-manifest.json"
require_text "$release_workflow" '-RuntimeBundleDir $runtimeDir'
require_text "$release_workflow" "scripts/release/create-windows-installer-evidence.sh"
require_text "$release_workflow" "scripts/release/run-windows-installer-lifecycle.ps1"
require_text "$release_workflow" "scripts/release/create-windows-service-evidence.sh"
require_text "$release_workflow" "scripts/release/run-windows-service-lifecycle.ps1"
require_text "$release_workflow" "runtime-bundle-manifest.json"
require_text "$release_workflow" "macos-package.json"
require_text "$release_workflow" '"schema_version": "release.runtime_package_payload.v1"'
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
require_text "$release_workflow" "Signing, notarization, installer lifecycle validation"

for file in "$release_workflow"; do
  reject_text "$file" "actions/checkout@v4"
  reject_text "$file" "actions/upload-artifact@v4"
done

printf '%s\n' "local-only workflow policy check passed"

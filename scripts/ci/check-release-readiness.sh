#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required release-readiness file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  if grep -Fq -- "$text" "$file"; then
    fail "$file leaked local release-readiness marker: $text"
  fi
}

CARGO_BIN="${CARGO:-cargo}"
if ! command -v "$CARGO_BIN" >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi

verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"
release_workflow=".github/workflows/release.yml"
runbook="docs/runbooks/release-blockers.md"

require_file "$verify_script"
require_file "$workflow_guard"
require_file "$release_workflow"
require_file "$runbook"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-release-readiness-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

data_dir="$tmpdir/PRIVATE-release-readiness-data"
stdout_file="$tmpdir/stdout.txt"
stderr_file="$tmpdir/stderr.txt"
mkdir -p "$data_dir"

set +e
"$CARGO_BIN" run --quiet -p resume-cli --locked -- \
  --data-dir "$data_dir" release-readiness --json \
  > "$stdout_file" 2> "$stderr_file"
status=$?
set -e

if [ "$status" -eq 0 ]; then
  fail "release-readiness command unexpectedly passed stable release"
fi

require_text "$stdout_file" '"schema_version": "release-readiness.v1"'
require_text "$stdout_file" '"stable_release": "blocked"'
require_text "$stdout_file" '"local_dry_run_artifacts": "evidence_only"'
require_text "$stdout_file" '"provided_evidence": []'
require_text "$stdout_file" '"blockers": ['
require_text "$stdout_file" '"label": "signing certificates"'
require_text "$stdout_file" "production signing certificates"
require_text "$stdout_file" "certificate chain"
require_text "$stdout_file" "private key custody"
require_text "$stdout_file" "signature verification evidence"
require_text "$stdout_file" '"label": "macOS notarization"'
require_text "$stdout_file" "Apple Developer ID"
require_text "$stdout_file" "notarization credentials"
require_text "$stdout_file" "notarization ticket"
require_text "$stdout_file" "Gatekeeper validation"
require_text "$stdout_file" '"label": "Windows installer lifecycle"'
require_text "$stdout_file" "MSI install"
require_text "$stdout_file" "upgrade"
require_text "$stdout_file" "uninstall"
require_text "$stdout_file" "rollback"
require_text "$stdout_file" "release Windows runner"
require_text "$stdout_file" '"label": "Windows service lifecycle"'
require_text "$stdout_file" "install/start/stop/status/uninstall/recovery"
require_text "$stdout_file" '"label": "macOS installer lifecycle"'
require_text "$stdout_file" "signed pkg/dmg"
require_text "$stdout_file" "install/upgrade/uninstall/rollback"
require_text "$stdout_file" '"label": "private real-corpus performance evidence"'
require_text "$stdout_file" "reproducible local private real-corpus hot-index hybrid benchmark baseline is not available"
require_text "$stdout_file" "available private corpus"
require_text "$stdout_file" "min-documents 8000"
require_text "$stdout_file" "500 query samples"
require_text "$stdout_file" "observed P50/P95/P99 metrics"
require_text "$stdout_file" "follow-up performance-optimization goal"
require_text "$stdout_file" '"label": "field extraction quality"'
require_text "$stdout_file" "private business labeled field-quality evidence is not available"
require_text "$stdout_file" "min-samples 1000"
require_text "$stdout_file" "precision/recall/F1 >= 0.93"
require_text "$stdout_file" '"label": "dedupe quality"'
require_text "$stdout_file" "private business labeled dedupe-quality evidence is not available"
require_text "$stdout_file" "min-pairs 1000"
require_text "$stdout_file" "min-positive-pairs 100"
require_text "$stdout_file" "precision/recall/F1 >= 0.90"
require_text "$stdout_file" '"label": "vector quality"'
require_text "$stdout_file" "private business labeled vector-quality evidence is not available"
require_text "$stdout_file" "recall@k >= 0.90"
require_text "$stdout_file" "MRR >= 0.85"
require_text "$stdout_file" "NDCG@k >= 0.90"
require_text "$stdout_file" '"label": "OCR throughput"'
require_text "$stdout_file" "private real-corpus OCR throughput evidence is not available"
require_text "$stdout_file" "min-pages 500"
require_text "$stdout_file" "OCR p95 <= 1000ms"
require_text "$stdout_file" "pages_per_second >= 1"
require_text "$stdout_file" '"label": "OCR runtime manifest/dependency evidence"'
require_text "$stdout_file" "reviewed OCR runtime manifest"
require_text "$stdout_file" "Tesseract/tessdata"
require_text "$stdout_file" "Apache-2.0"
require_text "$stdout_file" "Poppler/pdftoppm"
require_text "$stdout_file" "not bundled by default"
require_text "$stdout_file" "dependency detection"
require_text "$stdout_file" '"label": "embedding model license/distribution"'
require_text "$stdout_file" "reviewed licensed embedding model"
require_text "$stdout_file" "model manifest"
require_text "$stdout_file" "offline distribution"
require_text "$stdout_file" "license review"
require_text "$stdout_file" '"label": "cross-platform release validation"'
require_text "$stdout_file" "Windows and macOS release platforms"
require_text "$stdout_file" "fresh release artifacts"
require_text "$stdout_file" "install/upgrade/uninstall"
require_text "$stdout_file" "service lifecycle"
require_text "$stdout_file" '"label": "redacted diagnostics evidence"'
require_text "$stdout_file" "export-diagnostics --redact"
require_text "$stdout_file" "diagnostics.v1"
require_text "$stdout_file" "local aggregate diagnostics"
require_text "$stdout_file" '"label": "hardware fault drills"'
require_text "$stdout_file" "actual ENOSPC"
require_text "$stdout_file" "service-level daemon kill"
require_text "$stdout_file" '"status": "blocked"'
require_text "$stdout_file" '"next_gate": "keep release blocked until every item has current local evidence"'
require_text "$stderr_file" "release readiness blocked: stable release criteria are not met"

reject_text "$stdout_file" "$tmpdir"
reject_text "$stderr_file" "$tmpdir"
reject_text "$stdout_file" "PRIVATE-release-readiness-data"
reject_text "$stderr_file" "PRIVATE-release-readiness-data"
reject_text "$stdout_file" "/Users/"
reject_text "$stderr_file" "/Users/"
reject_text "$stdout_file" "local-data"
reject_text "$stderr_file" "local-data"
reject_text "$stdout_file" "diagnostics/"
reject_text "$stderr_file" "diagnostics/"
reject_text "$stdout_file" "diagnostics.zip"
reject_text "$stderr_file" "diagnostics.zip"
reject_text "$stdout_file" "model-cache"
reject_text "$stderr_file" "model-cache"

signing_evidence="$tmpdir/signing-evidence.json"
notarization_evidence="$tmpdir/notarization-evidence.json"
release_artifacts="$tmpdir/release-artifacts.json"
release_sbom="$tmpdir/release-sbom.json"
macos_package="$tmpdir/macos-package.json"
windows_package="$tmpdir/windows-package.json"
macos_installer_evidence="$tmpdir/macos-installer-evidence.json"
windows_installer_evidence="$tmpdir/windows-installer-evidence.json"
windows_service_evidence="$tmpdir/windows-service-evidence.json"
current_stage_evidence="$tmpdir/current-stage-validation-evidence.json"
evidence_stdout_file="$tmpdir/evidence-stdout.txt"
evidence_stderr_file="$tmpdir/evidence-stderr.txt"

cat > "$signing_evidence" <<'JSON'
{"schema_version":"release.signing_evidence.v1","version":"v0.0.0","signing_status":"blocked","evidence_boundary":"dry_run_no_signing_material","artifact_manifest_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","required_evidence":["certificate_chain"],"blocked_release_steps":["production_signing_certificates"]}
JSON
cat > "$notarization_evidence" <<'JSON'
{"schema_version":"release.notarization_evidence.v1","version":"v0.0.0","notarization_status":"blocked","evidence_boundary":"dry_run_no_notarization_credentials","macos_package_manifest_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","required_evidence":["notarization_ticket"],"blocked_release_steps":["notarytool_submission"]}
JSON
cat > "$release_artifacts" <<'JSON'
{"schema_version":"release.artifacts.v1","version":"v0.0.0","packaging_status":"blocked","artifacts":[{"name":"resume-cli","file":"resume-cli","sha256":"1111111111111111111111111111111111111111111111111111111111111111","bytes":101},{"name":"resume-daemon","file":"resume-daemon","sha256":"2222222222222222222222222222222222222222222222222222222222222222","bytes":202},{"name":"resume-benchmark","file":"resume-benchmark","sha256":"3333333333333333333333333333333333333333333333333333333333333333","bytes":303}],"blocked_release_steps":["packaging","signing","notarization","github_release_upload"],"notes":"Dry-run manifest only; no installer, signature, notarization ticket, release upload, local data, or runtime data is included."}
JSON
cat > "$release_sbom" <<'JSON'
{"spdxVersion":"SPDX-2.3","dataLicense":"CC0-1.0","SPDXID":"SPDXRef-DOCUMENT","name":"resume-ir-v0.0.0","documentNamespace":"https://github.com/FrankQDWang/resume-ir/sbom/v0.0.0","creationInfo":{"created":"2026-06-10T00:00:00Z","creators":["Tool: resume-ir-release-sbom"]},"packages":[{"SPDXID":"SPDXRef-Package-resume-cli","name":"resume-cli","versionInfo":"0.1.0","filesAnalyzed":false,"licenseDeclared":"MIT","externalRefs":[{"referenceCategory":"PACKAGE-MANAGER","referenceType":"purl","referenceLocator":"pkg:cargo/resume-cli@0.1.0"}]},{"SPDXID":"SPDXRef-Package-resume-daemon","name":"resume-daemon","versionInfo":"0.1.0","filesAnalyzed":false,"licenseDeclared":"MIT","externalRefs":[{"referenceCategory":"PACKAGE-MANAGER","referenceType":"purl","referenceLocator":"pkg:cargo/resume-daemon@0.1.0"}]},{"SPDXID":"SPDXRef-Package-benchmark-runner","name":"benchmark-runner","versionInfo":"0.1.0","filesAnalyzed":false,"licenseDeclared":"MIT","externalRefs":[{"referenceCategory":"PACKAGE-MANAGER","referenceType":"purl","referenceLocator":"pkg:cargo/benchmark-runner@0.1.0"}]}],"relationships":[{"spdxElementId":"SPDXRef-DOCUMENT","relationshipType":"DESCRIBES","relatedSpdxElement":"SPDXRef-Package-resume-cli"},{"spdxElementId":"SPDXRef-DOCUMENT","relationshipType":"DESCRIBES","relatedSpdxElement":"SPDXRef-Package-resume-daemon"},{"spdxElementId":"SPDXRef-DOCUMENT","relationshipType":"DESCRIBES","relatedSpdxElement":"SPDXRef-Package-benchmark-runner"}]}
JSON
cat > "$macos_package" <<'JSON'
{"schema_version":"release.macos_package.v1","version":"v0.0.0","packaging_status":"unsigned_dry_run","install_location":"/usr/local/bin","signing_status":"unsigned","notarization_status":"not_requested","artifacts":[{"kind":"pkg","file":"resume-ir-v0.0.0-macos.pkg","sha256":"4444444444444444444444444444444444444444444444444444444444444444","bytes":404},{"kind":"dmg","file":"resume-ir-v0.0.0-macos.dmg","sha256":"5555555555555555555555555555555555555555555555555555555555555555","bytes":505}],"blocked_release_steps":["signing","notarization","github_release_upload","installer_lifecycle_validation","windows_msi"],"notes":"Unsigned local macOS package dry run only; no signing, notarization, installer lifecycle validation, GitHub Release upload, local data, or runtime data is included."}
JSON
cat > "$windows_package" <<'JSON'
{"schema_version":"release.windows_package.v1","version":"v0.0.0","packaging_status":"unsigned_dry_run","installer_kind":"msi","install_location":"ProgramFilesFolder/resume-ir","signing_status":"unsigned","artifacts":[{"kind":"msi","file":"resume-ir-v0.0.0-windows.msi","sha256":"6666666666666666666666666666666666666666666666666666666666666666","bytes":606}],"blocked_release_steps":["signing","github_release_upload","installer_lifecycle_validation","service_install_validation","macos_notarization"],"notes":"Unsigned Windows MSI dry run only; no signing, service lifecycle validation, installer lifecycle validation, GitHub Release upload, local data, or runtime data is included."}
JSON
cat > "$macos_installer_evidence" <<'JSON'
{"schema_version":"release.macos_installer_evidence.v1","version":"v0.0.0","installer_lifecycle_status":"blocked","evidence_boundary":"dry_run_no_macos_installer_execution","macos_package_manifest_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","required_evidence":["installer_lifecycle_validation"],"blocked_release_steps":["macos_pkg_install"],"planned_actions":[{"action":"install","action_status":"blocked"}]}
JSON
cat > "$windows_installer_evidence" <<'JSON'
{"schema_version":"release.windows_installer_evidence.v1","version":"v0.0.0","installer_lifecycle_status":"blocked","evidence_boundary":"dry_run_no_windows_installer_execution","windows_package_manifest_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","required_evidence":["installer_lifecycle_validation"],"blocked_release_steps":["windows_msi_install"],"planned_actions":[{"action":"install","action_status":"blocked"}]}
JSON
cat > "$windows_service_evidence" <<'JSON'
{"schema_version":"release.windows_service_evidence.v1","version":"v0.0.0","service_lifecycle_status":"blocked","evidence_boundary":"dry_run_no_windows_service_registration","windows_package_manifest_sha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","required_evidence":["service_install_validation"],"blocked_release_steps":["windows_service_install"],"planned_actions":[{"action":"install","action_status":"blocked"}]}
JSON
cat > "$current_stage_evidence" <<'JSON'
{
  "schema_version": "resume-ir.current-stage-validation-evidence.v1",
  "privacy_boundary": "local_only_redacted_evidence_manifest",
  "current_stage_target": "reproducible_local_10k_baseline",
  "performance_optimization_deferred": true,
  "release_readiness_exit": 1,
  "stable_release_expected_blocked": true,
  "input_digests": {
    "dataset_manifest_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "query_set_sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    "model_manifest_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    "ocr_runtime_manifest_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
  },
  "parameters": {
    "max_files": 10000,
    "max_queries": 500,
    "top_k": 10,
    "embedding_dimension": 384,
    "ocr_worker_ticks": 10000,
    "embedding_worker_ticks": 10000
  },
  "steps": [
    {"id": "ocr_preflight", "status": "success"},
    {"id": "ocr_manifest_draft", "status": "success"},
    {"id": "ocr_manifest_validate", "status": "success"},
    {"id": "model_manifest_draft", "status": "success"},
    {"id": "model_manifest_validate", "status": "success"},
    {"id": "model_preflight", "status": "success"},
    {"id": "dataset_manifest", "status": "success"},
    {"id": "import_private_corpus", "status": "success"},
    {"id": "ocr_worker_bounded_loop", "status": "success"},
    {"id": "embedding_worker_bounded_loop", "status": "success"},
    {"id": "corpus_summary", "status": "success"},
    {"id": "query_set_draft", "status": "success"},
    {"id": "private_query_baseline", "status": "success"},
    {"id": "baseline_shape_gate", "status": "success"},
    {"id": "redacted_diagnostics", "status": "success"},
    {"id": "release_readiness_intake", "status": "expected_blocked", "exit_code": 1}
  ],
  "redacted_outputs": [
    {"file": "dataset-manifest.local.json", "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
    {"file": "dataset-manifest.stdout.txt", "sha256": "1111111111111111111111111111111111111111111111111111111111111111"},
    {"file": "ocr-runtime-manifest.local.json", "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"},
    {"file": "ocr-preflight.json", "sha256": "1212121212121212121212121212121212121212121212121212121212121212"},
    {"file": "ocr-draft-manifest.stdout.txt", "sha256": "1313131313131313131313131313131313131313131313131313131313131313"},
    {"file": "ocr-validate-manifest.stdout.txt", "sha256": "1414141414141414141414141414141414141414141414141414141414141414"},
    {"file": "model-manifest.local.json", "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"},
    {"file": "model-draft-manifest.stdout.txt", "sha256": "1515151515151515151515151515151515151515151515151515151515151515"},
    {"file": "model-validate-manifest.stdout.txt", "sha256": "1616161616161616161616161616161616161616161616161616161616161616"},
    {"file": "model-preflight.json", "sha256": "1717171717171717171717171717171717171717171717171717171717171717"},
    {"file": "import.stdout.txt", "sha256": "1818181818181818181818181818181818181818181818181818181818181818"},
    {"file": "ocr-worker.stdout.txt", "sha256": "1919191919191919191919191919191919191919191919191919191919191919"},
    {"file": "embedding-worker.stdout.txt", "sha256": "2020202020202020202020202020202020202020202020202020202020202020"},
    {"file": "benchmark-corpus-summary.local.json", "sha256": "2121212121212121212121212121212121212121212121212121212121212121"},
    {"file": "private-query-set.local.jsonl", "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
    {"file": "query-set-draft.stdout.txt", "sha256": "2323232323232323232323232323232323232323232323232323232323232323"},
    {"file": "private-benchmark-local.json", "sha256": "2424242424242424242424242424242424242424242424242424242424242424"},
    {"file": "private-benchmark-gate.stdout.txt", "sha256": "2525252525252525252525252525252525252525252525252525252525252525"},
    {"file": "redacted-diagnostics.json", "sha256": "2626262626262626262626262626262626262626262626262626262626262626"},
    {"file": "release-readiness.json", "sha256": "2727272727272727272727272727272727272727272727272727272727272727"},
    {"file": "release-readiness.stderr.txt", "sha256": "2828282828282828282828282828282828282828282828282828282828282828"}
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
JSON

set +e
"$CARGO_BIN" run --quiet -p resume-cli --locked -- \
  --data-dir "$data_dir" release-readiness --json \
  --release-artifact-manifest "$release_artifacts" \
  --release-sbom "$release_sbom" \
  --macos-package-manifest "$macos_package" \
  --windows-package-manifest "$windows_package" \
  --signing-evidence "$signing_evidence" \
  --notarization-evidence "$notarization_evidence" \
  --macos-installer-evidence "$macos_installer_evidence" \
  --windows-installer-evidence "$windows_installer_evidence" \
  --windows-service-evidence "$windows_service_evidence" \
  --current-stage-evidence "$current_stage_evidence" \
  > "$evidence_stdout_file" 2> "$evidence_stderr_file"
evidence_status=$?
set -e

if [ "$evidence_status" -eq 0 ]; then
  fail "release-readiness command unexpectedly accepted blocked automation evidence as a stable release"
fi

require_text "$evidence_stdout_file" '"label": "signing automation evidence"'
require_text "$evidence_stdout_file" '"label": "notarization automation evidence"'
require_text "$evidence_stdout_file" '"label": "release artifact manifest evidence"'
require_text "$evidence_stdout_file" '"label": "release SBOM evidence"'
require_text "$evidence_stdout_file" '"label": "macOS package manifest evidence"'
require_text "$evidence_stdout_file" '"label": "Windows package manifest evidence"'
require_text "$evidence_stdout_file" '"label": "macOS installer automation evidence"'
require_text "$evidence_stdout_file" '"label": "Windows installer automation evidence"'
require_text "$evidence_stdout_file" '"label": "Windows service automation evidence"'
require_text "$evidence_stdout_file" '"label": "current-stage validation evidence manifest"'
require_text "$evidence_stdout_file" '"privacy_boundary": "blocked_release_evidence_manifest"'
require_text "$evidence_stdout_file" '"privacy_boundary": "local_only_redacted_evidence_manifest"'
require_text "$evidence_stdout_file" "blocked dry-run evidence passed schema and boundary checks"
require_text "$evidence_stdout_file" "current-stage validation evidence manifest passed redacted schema and digest checks"
require_text "$evidence_stdout_file" "release.artifacts.v1 dry-run manifest passed schema and artifact boundary checks"
require_text "$evidence_stdout_file" "SPDX-2.3 release dry-run SBOM passed redaction and package boundary checks"
require_text "$evidence_stdout_file" "release.macos_package.v1 unsigned dry-run manifest passed package boundary checks"
require_text "$evidence_stdout_file" "release.windows_package.v1 unsigned dry-run manifest passed package boundary checks"
require_text "$evidence_stdout_file" '"label": "signing certificates"'
require_text "$evidence_stdout_file" '"label": "macOS notarization"'
require_text "$evidence_stdout_file" '"label": "macOS installer lifecycle"'
require_text "$evidence_stdout_file" '"label": "Windows installer lifecycle"'
require_text "$evidence_stdout_file" '"label": "Windows service lifecycle"'
require_text "$evidence_stdout_file" '"label": "cross-platform release validation"'
require_text "$evidence_stderr_file" "release readiness blocked: stable release criteria are not met"
reject_text "$evidence_stdout_file" "$tmpdir"
reject_text "$evidence_stderr_file" "$tmpdir"
reject_text "$evidence_stdout_file" "PRIVATE-release-readiness-data"
reject_text "$evidence_stderr_file" "PRIVATE-release-readiness-data"
reject_text "$evidence_stdout_file" "/Users/"
reject_text "$evidence_stderr_file" "/Users/"
reject_text "$evidence_stdout_file" "resume-ir-v0.0.0"
reject_text "$evidence_stderr_file" "resume-ir-v0.0.0"
reject_text "$evidence_stdout_file" "local-data"
reject_text "$evidence_stderr_file" "local-data"

require_text "$verify_script" "./scripts/ci/check-release-readiness.sh"
require_text "$workflow_guard" "check-release-readiness.sh"
require_text "$release_workflow" "./scripts/ci/check-release-readiness.sh"
require_text "$runbook" "resume-cli --data-dir <local-data-dir> release-readiness --json"
require_text "$runbook" "--benchmark-report private-benchmark-local.json"
require_text "$runbook" "resume-cli benchmark-query-protocol"
require_text "$runbook" "--command-arg --data-dir"
require_text "$runbook" 'resume-cli search --query-file "$RESUME_IR_QUERY_INPUT_PATH" --mode hybrid'
require_text "$runbook" "--min-documents 8000 --min-queries 500"
require_text "$runbook" "--field-quality-report private-field-quality.json"
require_text "$runbook" "--dedupe-quality-report private-dedupe-quality.json"
require_text "$runbook" "--vector-quality-report private-vector-quality.json"
require_text "$runbook" "--ocr-throughput-report private-ocr-throughput.json"
require_text "$runbook" "--diagnostics-report redacted-diagnostics.json"
require_text "$runbook" "--current-stage-evidence current-stage-validation-evidence.json"
require_text "$runbook" "--release-artifact-manifest release-artifacts.json"
require_text "$runbook" "--release-sbom release-sbom.json"
require_text "$runbook" "--macos-package-manifest macos-package.json"
require_text "$runbook" "--windows-package-manifest windows-package.json"
require_text "$runbook" "--signing-evidence signing-evidence.json"
require_text "$runbook" "--notarization-evidence notarization-evidence.json"
require_text "$runbook" "--macos-installer-evidence macos-installer-evidence.json"
require_text "$runbook" "--windows-installer-evidence windows-installer-evidence.json"
require_text "$runbook" "--windows-service-evidence windows-service-evidence.json"
require_text "$runbook" "blocked_release_evidence_manifest"
require_text "$runbook" "release artifact manifest evidence"
require_text "$runbook" "release SBOM evidence"
require_text "$runbook" "macOS package manifest evidence"
require_text "$runbook" "Windows package manifest evidence"
require_text "$runbook" "signing automation evidence"
require_text "$runbook" "Windows service automation evidence"
require_text "$runbook" "current-stage validation evidence manifest"
require_text "$runbook" "local_only_redacted_evidence_manifest"
require_text "$runbook" "redacted diagnostics evidence"
require_text "$runbook" "provided_evidence"
require_text "$runbook" "hardware fault drills"
require_text "$runbook" "actual ENOSPC"
require_text "$runbook" "service-level daemon kill"
require_text "$runbook" "vector-gate --report private-vector-quality.json"
require_text "$runbook" "ocr-gate --report private-ocr-throughput.json"
require_text "$runbook" "--max-run-ms <release-budget-ms>"
require_text "$runbook" "failed_document_count"
require_text "$runbook" "run_budget_exhausted"

printf '%s\n' "release readiness check passed"

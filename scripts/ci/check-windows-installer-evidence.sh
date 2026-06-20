#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required Windows installer evidence file: $1"
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
    fail "$file leaked Windows installer evidence marker: $text"
  fi
}

installer_script="scripts/release/create-windows-installer-evidence.sh"
lifecycle_script="scripts/release/run-windows-installer-lifecycle.ps1"
verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"
release_workflow=".github/workflows/release.yml"
runbook="docs/runbooks/release-blockers.md"

for file in "$installer_script" "$lifecycle_script" "$verify_script" "$workflow_guard" "$release_workflow" "$runbook"; do
  require_file "$file"
done

if [ ! -x "$installer_script" ]; then
  fail "Windows installer evidence script is not executable"
fi
require_text "$lifecycle_script" 'schema_version = "release.windows_installer_lifecycle_plan.v1"'
require_text "$lifecycle_script" 'execution_mode = "dry_run"'
require_text "$lifecycle_script" 'installer_lifecycle_status = "blocked"'
require_text "$lifecycle_script" 'msiexec.exe'
require_text "$lifecycle_script" 'install'
require_text "$lifecycle_script" 'upgrade'
require_text "$lifecycle_script" 'repair'
require_text "$lifecycle_script" 'uninstall'
require_text "$lifecycle_script" 'rollback'
require_text "$lifecycle_script" 'requires_approval = $true'
require_text "$lifecycle_script" 'admin_elevation = "required_not_observed"'

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-windows-installer-evidence-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

out_dir="$tmpdir/out"
package_manifest="$tmpdir/windows-package.json"
mkdir -p "$out_dir"
cat > "$package_manifest" <<'EOF'
{
  "schema_version": "release.windows_package.v1",
  "version": "v0.0.0",
  "packaging_status": "unsigned_dry_run",
  "installer_kind": "msi",
  "install_location": "ProgramFilesFolder/resume-ir",
  "signing_status": "unsigned",
  "runtime_payload": {
    "schema_version": "release.runtime_package_payload.v1",
    "runtime_distribution_mode": "bundled",
    "runtime_package_binaries_included": true,
    "runtime_binaries_included_in_manifest": false,
    "install_location": "ProgramFilesFolder/resume-ir/runtime",
    "runtime_bundle_manifest": {
      "file": "runtime-bundle-manifest.json",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "bytes": 654,
      "schema_version": "release.runtime_bundle.v1",
      "runtime_distribution_mode": "bundled"
    },
    "components": [
      {
        "id": "synthetic-tesseract",
        "kind": "ocr-engine",
        "file": "tesseract.exe",
        "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "bytes": 101,
        "license": "Apache-2.0",
        "source": "synthetic-reviewed-source"
      },
      {
        "id": "synthetic-pdftoppm",
        "kind": "pdf-renderer",
        "file": "pdftoppm.exe",
        "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "bytes": 202,
        "license": "GPL-compatible-reviewed",
        "source": "synthetic-reviewed-source"
      },
      {
        "id": "synthetic-tessdata-eng",
        "kind": "ocr-language-pack",
        "file": "eng.traineddata",
        "sha256": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "bytes": 303,
        "license": "Apache-2.0",
        "source": "synthetic-reviewed-source"
      }
    ]
  },
  "artifacts": [
    {
      "kind": "msi",
      "file": "resume-ir-v0.0.0-windows.msi",
      "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
      "bytes": 987
    }
  ],
  "blocked_release_steps": [
    "signing",
    "github_release_upload",
    "installer_lifecycle_validation",
    "service_install_validation",
    "macos_notarization"
  ],
  "notes": "Synthetic unsigned Windows MSI dry run only."
}
EOF

"$installer_script" \
  --version v0.0.0 \
  --windows-package-manifest "$package_manifest" \
  --out-dir "$out_dir" >/dev/null

manifest="$out_dir/windows-installer-evidence.json"
require_file "$manifest"
require_text "$manifest" '"schema_version": "release.windows_installer_evidence.v1"'
require_text "$manifest" '"version": "v0.0.0"'
require_text "$manifest" '"installer_lifecycle_status": "blocked"'
require_text "$manifest" '"evidence_boundary": "dry_run_no_windows_installer_execution"'
require_text "$manifest" '"windows_package_manifest_sha256": "'
require_text "$manifest" '"installer_engine": "msiexec.exe"'
require_text "$manifest" '"admin_elevation": "required_not_observed"'
require_text "$manifest" '"installation_status": "not_installed"'
require_text "$manifest" '"rollback_validation_status": "blocked"'
require_text "$manifest" '"action": "install"'
require_text "$manifest" '"action": "upgrade"'
require_text "$manifest" '"action": "repair"'
require_text "$manifest" '"action": "uninstall"'
require_text "$manifest" '"action": "rollback"'
require_text "$manifest" '"action_status": "blocked"'
require_text "$manifest" '"kind": "msi"'
require_text "$manifest" '"installer_lifecycle_validation"'
require_text "$manifest" '"rollback_validation"'

reject_text "$manifest" "$tmpdir"
reject_text "$manifest" "/Users/"
reject_text "$manifest" "target/release"
reject_text "$manifest" "local-data"
reject_text "$manifest" "diagnostics"
reject_text "$manifest" "model-cache"
reject_text "$manifest" "installer-token"
reject_text "$manifest" "administrator-password"

if "$installer_script" --version 0.0.0 --windows-package-manifest "$package_manifest" --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "Windows installer evidence script accepted an invalid version"
fi

if "$installer_script" --version v0.0.1 --windows-package-manifest "$package_manifest" --out-dir "$out_dir/mismatch" >/dev/null 2>&1; then
  fail "Windows installer evidence script accepted a mismatched package manifest version"
fi

unknown_manifest="$tmpdir/windows-package-unknown-field.json"
python3 - "$package_manifest" "$unknown_manifest" <<'PY'
import json
import sys

source = sys.argv[1]
target = sys.argv[2]

with open(source, "r", encoding="utf-8") as handle:
    document = json.load(handle)

document["artifacts"][0]["local_probe_path"] = "PRIVATE-windows-installer-cache"

with open(target, "w", encoding="utf-8") as handle:
    json.dump(document, handle)
    handle.write("\n")
PY

if "$installer_script" --version v0.0.0 --windows-package-manifest "$unknown_manifest" --out-dir "$out_dir/unknown" >/dev/null 2>&1; then
  fail "Windows installer evidence script accepted an unknown Windows package manifest field"
fi

require_text "$verify_script" "./scripts/ci/check-windows-installer-evidence.sh"
require_text "$workflow_guard" "check-windows-installer-evidence.sh"
require_text "$release_workflow" "scripts/release/create-windows-installer-evidence.sh"
require_text "$release_workflow" "scripts/release/run-windows-installer-lifecycle.ps1"
require_text "$release_workflow" "windows-installer-evidence.json"
require_text "$release_workflow" "windows-installer-lifecycle-dry-run.json"
require_text "$runbook" "create-windows-installer-evidence.sh"
require_text "$runbook" "run-windows-installer-lifecycle.ps1"
require_text "$runbook" "release.windows_installer_evidence.v1"
require_text "$runbook" "release.windows_installer_lifecycle_plan.v1"

printf '%s\n' "Windows installer evidence check passed"

#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required Windows service evidence file: $1"
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
    fail "$file leaked Windows service evidence marker: $text"
  fi
}

service_script="scripts/release/create-windows-service-evidence.sh"
verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"
release_workflow=".github/workflows/release.yml"
runbook="docs/runbooks/release-blockers.md"

for file in "$service_script" "$verify_script" "$workflow_guard" "$release_workflow" "$runbook"; do
  require_file "$file"
done

if [ ! -x "$service_script" ]; then
  fail "Windows service evidence script is not executable"
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-windows-service-evidence-check.XXXXXX")
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
  "artifacts": [
    {
      "kind": "msi",
      "file": "resume-ir-v0.0.0-windows.msi",
      "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      "bytes": 789
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

"$service_script" \
  --version v0.0.0 \
  --windows-package-manifest "$package_manifest" \
  --out-dir "$out_dir" >/dev/null

manifest="$out_dir/windows-service-evidence.json"
require_file "$manifest"
require_text "$manifest" '"schema_version": "release.windows_service_evidence.v1"'
require_text "$manifest" '"version": "v0.0.0"'
require_text "$manifest" '"service_lifecycle_status": "blocked"'
require_text "$manifest" '"evidence_boundary": "dry_run_no_windows_service_registration"'
require_text "$manifest" '"windows_package_manifest_sha256": "'
require_text "$manifest" '"service_manager": "sc.exe"'
require_text "$manifest" '"admin_elevation": "required_not_observed"'
require_text "$manifest" '"registration_status": "not_registered"'
require_text "$manifest" '"recovery_validation_status": "blocked"'
require_text "$manifest" '"action": "install"'
require_text "$manifest" '"action": "start"'
require_text "$manifest" '"action": "status"'
require_text "$manifest" '"action": "stop"'
require_text "$manifest" '"action": "uninstall"'
require_text "$manifest" '"action": "recovery"'
require_text "$manifest" '"action_status": "blocked"'
require_text "$manifest" '"kind": "msi"'
require_text "$manifest" '"service_install_validation"'
require_text "$manifest" '"service_recovery_validation"'

reject_text "$manifest" "$tmpdir"
reject_text "$manifest" "/Users/"
reject_text "$manifest" "target/release"
reject_text "$manifest" "local-data"
reject_text "$manifest" "diagnostics"
reject_text "$manifest" "model-cache"
reject_text "$manifest" "windows-service-token"
reject_text "$manifest" "administrator-password"

if "$service_script" --version 0.0.0 --windows-package-manifest "$package_manifest" --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "Windows service evidence script accepted an invalid version"
fi

if "$service_script" --version v0.0.1 --windows-package-manifest "$package_manifest" --out-dir "$out_dir/mismatch" >/dev/null 2>&1; then
  fail "Windows service evidence script accepted a mismatched package manifest version"
fi

require_text "$verify_script" "./scripts/ci/check-windows-service-evidence.sh"
require_text "$workflow_guard" "check-windows-service-evidence.sh"
require_text "$release_workflow" "scripts/release/create-windows-service-evidence.sh"
require_text "$release_workflow" "windows-service-evidence.json"
require_text "$runbook" "create-windows-service-evidence.sh"
require_text "$runbook" "release.windows_service_evidence.v1"

printf '%s\n' "Windows service evidence check passed"

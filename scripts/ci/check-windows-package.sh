#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required Windows package file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

script="scripts/release/create-windows-package.ps1"
verify_script="scripts/ci/verify-local.sh"
workflow=".github/workflows/release.yml"
workflow_guard="scripts/ci/check-workflows.sh"
runbook="docs/runbooks/release-blockers.md"

for file in "$script" "$verify_script" "$workflow" "$workflow_guard" "$runbook"; do
  require_file "$file"
done

require_text "$script" "release.windows_package.v1"
require_text "$script" "wix build"
require_text "$script" "resume-ir-\$Version-windows.msi"
require_text "$script" "signing_status = \"unsigned\""
require_text "$script" "installer_lifecycle_validation"
require_text "$verify_script" "./scripts/ci/check-windows-package.sh"
require_text "$workflow" "scripts/release/create-windows-package.ps1"
require_text "$workflow" "windows-package.json"
require_text "$workflow" "windows-package-dry-run"
require_text "$workflow" "resume-ir-\${{ inputs.version }}-windows.msi"
require_text "$workflow_guard" "check-windows-package.sh"
require_text "$runbook" "create-windows-package.ps1"

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    ;;
  *)
    printf '%s\n' "Windows package check skipped on non-Windows"
    exit 0
    ;;
esac

command -v pwsh >/dev/null 2>&1 || fail "pwsh is required"
command -v wix >/dev/null 2>&1 || fail "wix is required"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-windows-package-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

target_dir="$tmpdir/target/release"
out_dir="$tmpdir/out"
mkdir -p "$target_dir" "$out_dir"
for binary in resume-cli.exe resume-daemon.exe resume-benchmark.exe; do
  printf 'synthetic Windows binary %s\n' "$binary" > "$target_dir/$binary"
done

pwsh -NoLogo -NoProfile -File "$script" -Version v0.0.0 -TargetDir "$target_dir" -OutDir "$out_dir"
manifest="$out_dir/windows-package.json"
msi="$out_dir/resume-ir-v0.0.0-windows.msi"

require_file "$manifest"
require_file "$msi"
require_text "$manifest" '"schema_version": "release.windows_package.v1"'
require_text "$manifest" '"version": "v0.0.0"'
require_text "$manifest" '"packaging_status": "unsigned_dry_run"'
require_text "$manifest" '"installer_kind": "msi"'
require_text "$manifest" '"signing_status": "unsigned"'
require_text "$manifest" '"kind": "msi"'
require_text "$manifest" '"sha256": "'
require_text "$manifest" '"bytes": '

if grep -Fq "$tmpdir" "$manifest"; then
  fail "Windows package manifest leaked an absolute temp path"
fi
if grep -Eq 'manifest_path|src_path|license_file|/Users/|target/release|local-data|diagnostics|model-cache' "$manifest"; then
  fail "Windows package manifest leaked a local path or runtime-data marker"
fi

if pwsh -NoLogo -NoProfile -File "$script" -Version 0.0.0 -TargetDir "$target_dir" -OutDir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "Windows package script accepted an invalid version"
fi

rm "$target_dir/resume-daemon.exe"
if pwsh -NoLogo -NoProfile -File "$script" -Version v0.0.1 -TargetDir "$target_dir" -OutDir "$out_dir/missing" >/dev/null 2>&1; then
  fail "Windows package script accepted missing release binaries"
fi

printf '%s\n' "Windows package check passed"

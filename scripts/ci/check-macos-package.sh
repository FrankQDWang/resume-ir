#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required macOS package file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

script="scripts/release/create-macos-package.sh"
verify_dmg_script="scripts/release/verify-macos-dmg.sh"
verify_script="scripts/ci/verify-local.sh"

require_file "$script"
require_file "$verify_dmg_script"
require_file "$verify_script"

if [ ! -x "$script" ]; then
  fail "macOS package script is not executable"
fi
if [ ! -x "$verify_dmg_script" ]; then
  fail "macOS dmg verification script is not executable"
fi

if [ "$(uname -s)" != "Darwin" ]; then
  require_text "$verify_script" "./scripts/ci/check-macos-package.sh"
  printf '%s\n' "macOS package check skipped on non-Darwin"
  exit 0
fi

command -v pkgutil >/dev/null 2>&1 || fail "pkgutil is required"
command -v hdiutil >/dev/null 2>&1 || fail "hdiutil is required"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-macos-package-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

target_dir="$tmpdir/target/release"
out_dir="$tmpdir/out"
mkdir -p "$target_dir" "$out_dir"
for binary in resume-cli resume-daemon resume-benchmark; do
  printf 'synthetic macOS binary %s\n' "$binary" > "$target_dir/$binary"
  chmod 755 "$target_dir/$binary"
done

"$script" --version v0.0.0 --target-dir "$target_dir" --out-dir "$out_dir"
manifest="$out_dir/macos-package.json"
pkg="$out_dir/resume-ir-v0.0.0-macos.pkg"
dmg="$out_dir/resume-ir-v0.0.0-macos.dmg"

require_file "$manifest"
require_file "$pkg"
require_file "$dmg"
require_text "$manifest" '"schema_version": "release.macos_package.v1"'
require_text "$manifest" '"version": "v0.0.0"'
require_text "$manifest" '"packaging_status": "unsigned_dry_run"'
require_text "$manifest" '"signing_status": "unsigned"'
require_text "$manifest" '"notarization_status": "not_requested"'
require_text "$manifest" '"installer_lifecycle_validation"'
require_text "$manifest" '"windows_msi"'
require_text "$manifest" '"kind": "pkg"'
require_text "$manifest" '"kind": "dmg"'
require_text "$manifest" '"sha256": "'
require_text "$manifest" '"bytes": '

if grep -Fq "$tmpdir" "$manifest"; then
  fail "macOS package manifest leaked an absolute temp path"
fi
if grep -Eq 'manifest_path|src_path|license_file|/Users/|target/release|local-data|diagnostics|model-cache' "$manifest"; then
  fail "macOS package manifest leaked a local path or runtime-data marker"
fi

expanded="$tmpdir/expanded-pkg"
pkgutil --expand "$pkg" "$expanded" >/dev/null
"$verify_dmg_script" "$dmg" >/dev/null

if "$script" --version 0.0.0 --target-dir "$target_dir" --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "macOS package script accepted an invalid version"
fi

rm "$target_dir/resume-daemon"
if "$script" --version v0.0.1 --target-dir "$target_dir" --out-dir "$out_dir/missing" >/dev/null 2>&1; then
  fail "macOS package script accepted missing release binaries"
fi

require_text "$verify_script" "./scripts/ci/check-macos-package.sh"

printf '%s\n' "macOS package check passed"

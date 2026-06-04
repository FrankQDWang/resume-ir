#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required release artifact file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

script="scripts/release/create-artifact-manifest.sh"
workflow=".github/workflows/release.yml"

require_file "$script"
require_file "$workflow"

if [ ! -x "$script" ]; then
  fail "release artifact manifest script is not executable"
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-release-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

target_dir="$tmpdir/target/release"
out_dir="$tmpdir/out"
mkdir -p "$target_dir" "$out_dir"
for binary in resume-cli resume-daemon resume-benchmark; do
  printf 'synthetic binary %s\n' "$binary" > "$target_dir/$binary"
  chmod 755 "$target_dir/$binary"
done

"$script" --version v0.0.0 --target-dir "$target_dir" --out-dir "$out_dir"
manifest="$out_dir/release-artifacts.json"
require_file "$manifest"
require_text "$manifest" '"schema_version": "release.artifacts.v1"'
require_text "$manifest" '"version": "v0.0.0"'
require_text "$manifest" '"packaging_status": "blocked"'
require_text "$manifest" '"signing"'
require_text "$manifest" '"notarization"'
require_text "$manifest" '"resume-cli"'
require_text "$manifest" '"resume-daemon"'
require_text "$manifest" '"resume-benchmark"'
require_text "$manifest" '"sha256": "'
require_text "$manifest" '"bytes": '

if grep -Fq "$tmpdir" "$manifest"; then
  fail "release artifact manifest leaked an absolute temp path"
fi

if "$script" --version 0.0.0 --target-dir "$target_dir" --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "release artifact manifest script accepted an invalid version"
fi

rm "$target_dir/resume-daemon"
if "$script" --version v0.0.1 --target-dir "$target_dir" --out-dir "$out_dir/missing" >/dev/null 2>&1; then
  fail "release artifact manifest script accepted missing release binaries"
fi

require_text "$workflow" "scripts/release/create-artifact-manifest.sh"
require_text "$workflow" "release-artifacts.json"
require_text "$workflow" "actions/upload-artifact"
require_text "$workflow" "Packaging, signing, notarization"

printf '%s\n' "release artifact check passed"

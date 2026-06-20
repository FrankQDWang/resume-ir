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
cat > "$out_dir/runtime-bundle-manifest.json" <<'JSON'
{
  "schema_version": "release.runtime_bundle.v1",
  "version": "v0.0.0",
  "runtime_pack_id": "reviewed-runtime-pack",
  "runtime_distribution_mode": "bundled",
  "runtime_package_binaries_included": true,
  "runtime_binaries_included": false,
  "distribution_license": "GPL-3.0-or-later",
  "legal_review": "reviewed",
  "source_offer": {
    "file": "source-offer.txt",
    "bytes": 101,
    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  },
  "components": [
    {
      "id": "tesseract",
      "kind": "ocr-engine",
      "file": "tesseract",
      "bytes": 202,
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "license": {"id": "Apache-2.0", "reviewed": true},
      "source": "https://github.com/tesseract-ocr/tesseract"
    }
  ]
}
JSON

"$script" \
  --version v0.0.0 \
  --target-dir "$target_dir" \
  --out-dir "$out_dir" \
  --runtime-bundle-manifest "$out_dir/runtime-bundle-manifest.json"
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
require_text "$manifest" '"runtime_bundle_manifests"'
require_text "$manifest" '"runtime-bundle-manifest.json"'
require_text "$manifest" '"runtime_distribution_mode": "bundled"'
require_text "$manifest" '"runtime_package_binaries_included": true'
require_text "$manifest" '"runtime_binaries_included": false'
require_text "$manifest" '"sha256": "'
require_text "$manifest" '"bytes": '

if grep -Fq "$tmpdir" "$manifest"; then
  fail "release artifact manifest leaked an absolute temp path"
fi

if "$script" --version 0.0.0 --target-dir "$target_dir" --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "release artifact manifest script accepted an invalid version"
fi

printf '{"schema_version":"release.runtime_bundle.v1","runtime_distribution_mode":"external","runtime_package_binaries_included":false}\n' \
  > "$out_dir/invalid-runtime-bundle-manifest.json"
if "$script" \
  --version v0.0.1 \
  --target-dir "$target_dir" \
  --out-dir "$out_dir/invalid-runtime" \
  --runtime-bundle-manifest "$out_dir/invalid-runtime-bundle-manifest.json" \
  >/dev/null 2>&1; then
  fail "release artifact manifest script accepted an invalid runtime bundle manifest"
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

#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/create-artifact-manifest.sh --version vX.Y.Z --target-dir DIR --out-dir DIR
  [--runtime-bundle-manifest FILE]

Create a redacted release dry-run artifact manifest for already-built binaries.
The manifest contains artifact names, byte counts, sha256 hashes, and optional
runtime bundle manifest digests only.
EOF
}

version=""
target_dir=""
out_dir=""
runtime_bundle_manifest=""

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      [ $# -ge 2 ] || fail "--version requires a value"
      version="$2"
      shift 2
      ;;
    --target-dir)
      [ $# -ge 2 ] || fail "--target-dir requires a value"
      target_dir="$2"
      shift 2
      ;;
    --out-dir)
      [ $# -ge 2 ] || fail "--out-dir requires a value"
      out_dir="$2"
      shift 2
      ;;
    --runtime-bundle-manifest)
      [ $# -ge 2 ] || fail "--runtime-bundle-manifest requires a value"
      runtime_bundle_manifest="$2"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

[ -n "$version" ] || fail "--version is required"
[ -n "$target_dir" ] || fail "--target-dir is required"
[ -n "$out_dir" ] || fail "--out-dir is required"

printf '%s\n' "$version" | grep -Eq '^v[0-9]+[.][0-9]+[.][0-9]+$' \
  || fail "version must look like vX.Y.Z"

[ -d "$target_dir" ] || fail "target directory does not exist"
if [ -n "$runtime_bundle_manifest" ]; then
  [ -f "$runtime_bundle_manifest" ] || fail "runtime bundle manifest does not exist"
fi
mkdir -p "$out_dir"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{ print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{ print $1 }'
  else
    fail "sha256sum or shasum is required"
  fi
}

byte_count() {
  wc -c < "$1" | tr -d '[:space:]'
}

binary_path() {
  name="$1"
  if [ -f "$target_dir/$name" ]; then
    printf '%s\n' "$target_dir/$name"
  elif [ -f "$target_dir/$name.exe" ]; then
    printf '%s\n' "$target_dir/$name.exe"
  else
    fail "missing release binary: $name"
  fi
}

emit_artifact() {
  name="$1"
  path=$(binary_path "$name")
  file=$(basename "$path")

  [ -x "$path" ] || fail "release binary is not executable: $name"

  sha256=$(sha256_file "$path")
  bytes=$(byte_count "$path")

  printf '    {\n'
  printf '      "name": "%s",\n' "$name"
  printf '      "file": "%s",\n' "$file"
  printf '      "sha256": "%s",\n' "$sha256"
  printf '      "bytes": %s\n' "$bytes"
  printf '    }'
}

emit_runtime_bundle_manifest() {
  path="$1"
  file=$(basename "$path")
  case "$file" in
    */*|.*|"")
      fail "runtime bundle manifest basename is invalid"
      ;;
  esac

  sha256=$(sha256_file "$path")
  bytes=$(byte_count "$path")

  printf '    {\n'
  printf '      "file": "%s",\n' "$file"
  printf '      "sha256": "%s",\n' "$sha256"
  printf '      "bytes": %s,\n' "$bytes"
  printf '      "schema_version": "release.runtime_bundle.v1",\n'
  printf '      "runtime_distribution_mode": "bundled",\n'
  printf '      "runtime_package_binaries_included": true,\n'
  printf '      "runtime_binaries_included": false\n'
  printf '    }'
}

manifest="$out_dir/release-artifacts.json"
tmp_manifest="$manifest.tmp"

{
  printf '{\n'
  printf '  "schema_version": "release.artifacts.v1",\n'
  printf '  "version": "%s",\n' "$version"
  printf '  "packaging_status": "blocked",\n'
  printf '  "artifacts": [\n'
  emit_artifact resume-cli
  printf ',\n'
  emit_artifact resume-daemon
  printf ',\n'
  emit_artifact resume-benchmark
  printf '\n'
  printf '  ]'
  if [ -n "$runtime_bundle_manifest" ]; then
    printf ',\n'
    printf '  "runtime_bundle_manifests": [\n'
    emit_runtime_bundle_manifest "$runtime_bundle_manifest"
    printf '\n'
    printf '  ]'
  fi
  printf ',\n'
  printf '  "blocked_release_steps": [\n'
  printf '    "packaging",\n'
  printf '    "signing",\n'
  printf '    "notarization",\n'
  printf '    "github_release_upload"\n'
  printf '  ],\n'
  printf '  "notes": "Dry-run manifest only; installer packaging, signing, notarization, and release upload remain blocked until explicit release approval. Generate release-sbom.json separately during the release dry run."\n'
  printf '}\n'
} > "$tmp_manifest"

mv "$tmp_manifest" "$manifest"
printf '%s\n' "$manifest"

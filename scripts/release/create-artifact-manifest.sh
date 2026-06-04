#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/create-artifact-manifest.sh --version vX.Y.Z --target-dir DIR --out-dir DIR

Create a redacted release dry-run artifact manifest for already-built binaries.
The manifest contains artifact names, byte counts, and sha256 hashes only.
EOF
}

version=""
target_dir=""
out_dir=""

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
  printf '  ],\n'
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

#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/create-macos-package.sh --version vX.Y.Z --target-dir DIR --out-dir DIR

Create unsigned local macOS pkg/dmg dry-run artifacts for already-built binaries.
This does not sign, notarize, publish, or validate installer lifecycle behavior.
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

[ "$(uname -s)" = "Darwin" ] || fail "macOS packaging requires macOS"
command -v pkgbuild >/dev/null 2>&1 || fail "pkgbuild is required"
command -v productbuild >/dev/null 2>&1 || fail "productbuild is required"
command -v hdiutil >/dev/null 2>&1 || fail "hdiutil is required"

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
  path="$target_dir/$name"
  [ -f "$path" ] || fail "missing release binary: $name"
  [ -x "$path" ] || fail "release binary is not executable: $name"
  printf '%s\n' "$path"
}

version_number=${version#v}
pkg_file="$out_dir/resume-ir-${version}-macos.pkg"
dmg_file="$out_dir/resume-ir-${version}-macos.dmg"
manifest="$out_dir/macos-package.json"

workdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-macos-package.XXXXXX")
trap 'rm -rf "$workdir"' EXIT HUP INT TERM

payload="$workdir/payload"
bin_dir="$payload/usr/local/bin"
mkdir -p "$bin_dir"

for binary in resume-cli resume-daemon resume-benchmark; do
  source_path=$(binary_path "$binary")
  cp "$source_path" "$bin_dir/$binary"
  chmod 755 "$bin_dir/$binary"
done

component_pkg="$workdir/resume-ir-component.pkg"
pkgbuild \
  --root "$payload" \
  --identifier "io.github.frankqdwang.resume-ir" \
  --version "$version_number" \
  --install-location "/" \
  "$component_pkg" >/dev/null

productbuild --package "$component_pkg" "$pkg_file" >/dev/null

dmg_root="$workdir/dmg-root"
mkdir -p "$dmg_root"
cp "$pkg_file" "$dmg_root/"
cat > "$dmg_root/README.txt" <<EOF
resume-ir ${version} unsigned macOS package dry run

This artifact is local release-readiness evidence only.
It is not signed, notarized, published, or approved for installation.
EOF

hdiutil create \
  -quiet \
  -volname "resume-ir ${version}" \
  -srcfolder "$dmg_root" \
  -ov \
  -format UDZO \
  "$dmg_file"

pkg_sha256=$(sha256_file "$pkg_file")
pkg_bytes=$(byte_count "$pkg_file")
dmg_sha256=$(sha256_file "$dmg_file")
dmg_bytes=$(byte_count "$dmg_file")

tmp_manifest="$manifest.tmp"
{
  printf '{\n'
  printf '  "schema_version": "release.macos_package.v1",\n'
  printf '  "version": "%s",\n' "$version"
  printf '  "packaging_status": "unsigned_dry_run",\n'
  printf '  "install_location": "/usr/local/bin",\n'
  printf '  "signing_status": "unsigned",\n'
  printf '  "notarization_status": "not_requested",\n'
  printf '  "artifacts": [\n'
  printf '    {\n'
  printf '      "kind": "pkg",\n'
  printf '      "file": "%s",\n' "$(basename "$pkg_file")"
  printf '      "sha256": "%s",\n' "$pkg_sha256"
  printf '      "bytes": %s\n' "$pkg_bytes"
  printf '    },\n'
  printf '    {\n'
  printf '      "kind": "dmg",\n'
  printf '      "file": "%s",\n' "$(basename "$dmg_file")"
  printf '      "sha256": "%s",\n' "$dmg_sha256"
  printf '      "bytes": %s\n' "$dmg_bytes"
  printf '    }\n'
  printf '  ],\n'
  printf '  "blocked_release_steps": [\n'
  printf '    "signing",\n'
  printf '    "notarization",\n'
  printf '    "github_release_upload",\n'
  printf '    "installer_lifecycle_validation",\n'
  printf '    "windows_msi"\n'
  printf '  ],\n'
  printf '  "notes": "Unsigned local macOS package dry run only; signing, notarization, GitHub Release upload, and installer lifecycle validation remain blocked until explicit release approval and credentials are available."\n'
  printf '}\n'
} > "$tmp_manifest"

mv "$tmp_manifest" "$manifest"
printf '%s\n' "$manifest"

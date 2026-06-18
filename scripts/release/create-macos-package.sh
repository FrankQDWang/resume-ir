#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/create-macos-package.sh --version vX.Y.Z --target-dir DIR --out-dir DIR
  [--runtime-bundle-manifest FILE --runtime-bundle-dir DIR]

Create unsigned local macOS pkg/dmg dry-run artifacts for already-built binaries.
This does not sign, notarize, publish, or validate installer lifecycle behavior.
EOF
}

version=""
target_dir=""
out_dir=""
runtime_bundle_manifest=""
runtime_bundle_dir=""

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
    --runtime-bundle-dir)
      [ $# -ge 2 ] || fail "--runtime-bundle-dir requires a value"
      runtime_bundle_dir="$2"
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
case "${runtime_bundle_manifest:+manifest}:${runtime_bundle_dir:+dir}" in
  : | manifest:dir) ;;
  *) fail "--runtime-bundle-manifest and --runtime-bundle-dir must be supplied together" ;;
esac
if [ -n "$runtime_bundle_manifest" ]; then
  [ -f "$runtime_bundle_manifest" ] || fail "runtime bundle manifest does not exist"
  [ -d "$runtime_bundle_dir" ] || fail "runtime bundle directory does not exist"
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

runtime_payload_json=""
if [ -n "$runtime_bundle_manifest" ]; then
  runtime_payload_dir="$payload/usr/local/lib/resume-ir/runtime"
  runtime_payload_json="$workdir/runtime-payload.json"
  python3 - "$runtime_bundle_manifest" "$runtime_bundle_dir" "$runtime_payload_dir" "$runtime_payload_json" <<'PY'
import hashlib
import json
import os
import re
import shutil
import sys
from pathlib import Path


def fail(message):
    raise SystemExit(message)


def require_string(mapping, key):
    value = mapping.get(key)
    if not isinstance(value, str) or not value:
        fail(f"runtime package blocked: missing {key}")
    return value


def require_bool(mapping, key, expected):
    value = mapping.get(key)
    if value is not expected:
        fail(f"runtime package blocked: invalid {key}")


def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def basename(value, label):
    name = require_string({"value": value}, "value")
    if name != os.path.basename(name) or name in {".", ".."}:
        fail(f"runtime package blocked: invalid {label}")
    return name


manifest_path = Path(sys.argv[1])
runtime_dir = Path(sys.argv[2])
payload_dir = Path(sys.argv[3])
payload_json = Path(sys.argv[4])

try:
    document = json.loads(manifest_path.read_text(encoding="utf-8"))
except json.JSONDecodeError as error:
    fail(f"runtime package blocked: invalid runtime bundle manifest: {error.msg}")

if document.get("schema_version") != "release.runtime_bundle.v1":
    fail("runtime package blocked: runtime bundle manifest schema mismatch")
if document.get("runtime_distribution_mode") != "bundled":
    fail("runtime package blocked: runtime distribution mode must be bundled")
require_bool(document, "runtime_package_binaries_included", True)
require_bool(document, "runtime_binaries_included", False)

components = document.get("components")
if not isinstance(components, list) or not components:
    fail("runtime package blocked: runtime bundle components are missing")

payload_dir.mkdir(parents=True, exist_ok=True)
component_records = []
seen_files = set()
for component in components:
    if not isinstance(component, dict):
        fail("runtime package blocked: component must be an object")
    component_id = require_string(component, "id")
    kind = require_string(component, "kind")
    file_name = basename(component.get("file"), "component file")
    if file_name in seen_files:
        fail("runtime package blocked: duplicate component file")
    seen_files.add(file_name)
    source = require_string(component, "source")
    if source.startswith("/") or "PRIVATE-" in source or "/Users/" in source:
        fail("runtime package blocked: component source is private")
    expected_sha256 = require_string(component, "sha256").lower()
    if not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
        fail("runtime package blocked: component sha256 is invalid")
    expected_bytes = component.get("bytes")
    if not isinstance(expected_bytes, int) or expected_bytes <= 0:
        fail("runtime package blocked: component bytes is invalid")
    license_obj = component.get("license")
    if not isinstance(license_obj, dict):
        fail("runtime package blocked: component license is missing")
    license_id = require_string(license_obj, "id")
    require_bool(license_obj, "reviewed", True)

    source_file = runtime_dir / file_name
    if not source_file.is_file():
        fail("runtime package blocked: component file is unavailable")
    actual_sha256 = sha256_file(source_file)
    actual_bytes = source_file.stat().st_size
    if actual_sha256 != expected_sha256 or actual_bytes != expected_bytes:
        fail("runtime package blocked: component file digest mismatch")
    target_file = payload_dir / file_name
    shutil.copy2(source_file, target_file)
    component_records.append(
        {
            "id": component_id,
            "kind": kind,
            "file": file_name,
            "sha256": actual_sha256,
            "bytes": actual_bytes,
            "license": license_id,
            "source": source,
        }
    )

runtime_bundle_record = {
    "file": manifest_path.name,
    "sha256": sha256_file(manifest_path),
    "bytes": manifest_path.stat().st_size,
    "schema_version": "release.runtime_bundle.v1",
    "runtime_distribution_mode": "bundled",
}

payload = {
    "schema_version": "release.runtime_package_payload.v1",
    "runtime_distribution_mode": "bundled",
    "runtime_package_binaries_included": True,
    "runtime_binaries_included_in_manifest": False,
    "install_location": "/usr/local/lib/resume-ir/runtime",
    "runtime_bundle_manifest": runtime_bundle_record,
    "components": component_records,
}

payload_text = json.dumps(payload, indent=2, sort_keys=True)
for marker in [str(runtime_dir), str(payload_dir), "PRIVATE-", "/Users/", "target/release", "model-cache"]:
    if marker and marker in payload_text:
        fail("runtime package blocked: payload manifest contains private marker")
payload_json.write_text(payload_text + "\n", encoding="utf-8")
PY
fi

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
  if [ -n "$runtime_payload_json" ]; then
    sed '1s/^/  "runtime_payload": /; 2,$s/^/  /; $s/$/,/' "$runtime_payload_json"
  fi
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
  printf '  "notes": "Unsigned local macOS package dry run only; optional reviewed runtime payload can be included when supplied, but signing, notarization, GitHub Release upload, and installer lifecycle validation remain blocked until explicit release approval and credentials are available."\n'
  printf '}\n'
} > "$tmp_manifest"

mv "$tmp_manifest" "$manifest"
printf '%s\n' "$manifest"

#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/create-runtime-bundle-manifest.sh
  --version vX.Y.Z --runtime-pack-id ID --distribution-license ID
  --source-offer FILE --component ID|KIND|LICENSE|SOURCE|FILE
  [--component ID|KIND|LICENSE|SOURCE|FILE ...] [--notice FILE ...]
  --out-dir DIR --reviewed

Create a redacted runtime bundle manifest for already reviewed local runtime
components. The manifest records basenames, byte counts, sha256 hashes,
licenses, sources, and source-offer/notice evidence only. It never copies
runtime binaries and never prints local paths.
EOF
}

need_value() {
  [ "$#" -ge 2 ] || fail "$1 requires a value"
  [ -n "$2" ] || fail "$1 requires a value"
}

version=""
runtime_pack_id=""
distribution_license=""
source_offer=""
out_dir=""
reviewed=0
component_args=""
notice_args=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      need_value "$@"; version="$2"; shift 2
      ;;
    --runtime-pack-id)
      need_value "$@"; runtime_pack_id="$2"; shift 2
      ;;
    --distribution-license)
      need_value "$@"; distribution_license="$2"; shift 2
      ;;
    --source-offer)
      need_value "$@"; source_offer="$2"; shift 2
      ;;
    --component)
      need_value "$@"
      component_args="${component_args}${component_args:+
}$2"
      shift 2
      ;;
    --notice)
      need_value "$@"
      notice_args="${notice_args}${notice_args:+
}$2"
      shift 2
      ;;
    --out-dir)
      need_value "$@"; out_dir="$2"; shift 2
      ;;
    --reviewed)
      reviewed=1; shift
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
[ -n "$runtime_pack_id" ] || fail "--runtime-pack-id is required"
[ -n "$distribution_license" ] || fail "--distribution-license is required"
[ -n "$source_offer" ] || fail "--source-offer is required"
[ -n "$component_args" ] || fail "--component is required"
[ -n "$out_dir" ] || fail "--out-dir is required"
[ "$reviewed" -eq 1 ] || fail "runtime bundle manifest blocked: legal review is incomplete"

printf '%s\n' "$version" | grep -Eq '^v[0-9]+[.][0-9]+[.][0-9]+$' \
  || fail "version must look like vX.Y.Z"
case "$runtime_pack_id" in
  ''|*[!A-Za-z0-9._:/+-]*)
    fail "runtime pack id is invalid"
    ;;
esac
case "$distribution_license" in
  ''|*[!A-Za-z0-9._:+-]*)
    fail "distribution license is invalid"
    ;;
esac
[ -f "$source_offer" ] || fail "source-offer file does not exist"

mkdir -p "$out_dir"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-runtime-bundle.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

components_file="$tmpdir/components.txt"
notices_file="$tmpdir/notices.txt"
printf '%s\n' "$component_args" > "$components_file"
printf '%s\n' "$notice_args" > "$notices_file"

manifest="$out_dir/runtime-bundle-manifest.json"
tmp_manifest="$manifest.tmp"

python3 - "$version" "$runtime_pack_id" "$distribution_license" "$source_offer" "$components_file" "$notices_file" "$tmp_manifest" <<'PY'
import hashlib
import json
import os
import re
import sys

version, runtime_pack_id, distribution_license, source_offer, components_path, notices_path, output_path = sys.argv[1:8]


def fail(message):
    print(message, file=sys.stderr)
    raise SystemExit(1)


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def byte_count(path):
    return os.stat(path).st_size


def basename(path):
    value = os.path.basename(path)
    if not value or value in {".", ".."} or "/" in value or "\\" in value:
        fail("runtime bundle manifest blocked: invalid basename")
    return value


def validate_id(kind, value):
    if not value or not re.fullmatch(r"[A-Za-z0-9._:/+-]+", value):
        fail(f"runtime bundle manifest blocked: invalid {kind}")


def validate_component_kind(value):
    allowed = {
        "ocr-engine",
        "ocr-language-pack",
        "pdf-renderer",
        "embedding-model",
        "model-runtime",
        "license-notice",
        "support-tool",
    }
    if value not in allowed:
        fail("runtime bundle manifest blocked: invalid component kind")


validate_id("runtime pack id", runtime_pack_id)
validate_id("distribution license", distribution_license)

components = []
seen_ids = set()
seen_files = set()
with open(components_path, "r", encoding="utf-8") as handle:
    for line in handle:
        line = line.rstrip("\n")
        if not line:
            continue
        parts = line.split("|")
        if len(parts) != 5:
            fail("runtime bundle manifest blocked: invalid component spec")
        component_id, kind, license_id, source, artifact_path = parts
        validate_id("component id", component_id)
        validate_component_kind(kind)
        validate_id("component license", license_id)
        if not source or source.startswith("/") or "PRIVATE-" in source:
            fail("runtime bundle manifest blocked: invalid component source")
        if component_id in seen_ids:
            fail("runtime bundle manifest blocked: duplicate component id")
        if not os.path.isfile(artifact_path):
            fail("runtime bundle manifest blocked: component artifact is unavailable")
        file_name = basename(artifact_path)
        if file_name in seen_files:
            fail("runtime bundle manifest blocked: duplicate component file")
        seen_files.add(file_name)
        seen_ids.add(component_id)
        components.append(
            {
                "id": component_id,
                "kind": kind,
                "file": file_name,
                "bytes": byte_count(artifact_path),
                "sha256": sha256_file(artifact_path),
                "license": {"id": license_id, "reviewed": True},
                "source": source,
            }
        )

if not components:
    fail("runtime bundle manifest blocked: no components")

notices = []
with open(notices_path, "r", encoding="utf-8") as handle:
    for line in handle:
        notice_path = line.rstrip("\n")
        if not notice_path:
            continue
        if not os.path.isfile(notice_path):
            fail("runtime bundle manifest blocked: notice file is unavailable")
        notices.append(
            {
                "file": basename(notice_path),
                "bytes": byte_count(notice_path),
                "sha256": sha256_file(notice_path),
            }
        )

document = {
    "schema_version": "release.runtime_bundle.v1",
    "version": version,
    "runtime_pack_id": runtime_pack_id,
    "runtime_distribution_mode": "bundled",
    "runtime_package_binaries_included": True,
    "runtime_binaries_included": False,
    "distribution_license": distribution_license,
    "legal_review": "reviewed",
    "source_offer": {
        "file": basename(source_offer),
        "bytes": byte_count(source_offer),
        "sha256": sha256_file(source_offer),
    },
    "notices": notices,
    "components": components,
    "blocked_release_steps": [
        "runtime_binary_copy_into_installer",
        "installer_platform_validation",
        "signing",
        "notarization",
        "github_release_upload",
    ],
    "privacy_sentinels": {
        "local_paths_included": False,
        "runtime_binaries_included": False,
        "raw_resume_text_included": False,
        "model_bytes_included": False,
    },
}

payload = json.dumps(document, indent=2, sort_keys=True)
for marker in ("PRIVATE-", "/Users/", "local-data", "diagnostics", "model-cache", "resume text"):
    if marker in payload:
        fail("runtime bundle manifest blocked: private marker is present")

with open(output_path, "w", encoding="utf-8") as handle:
    handle.write(payload)
    handle.write("\n")
PY

mv "$tmp_manifest" "$manifest"
printf '%s\n' "runtime bundle manifest: written"
printf '%s\n' "file: runtime-bundle-manifest.json"
printf '%s\n' "paths: <redacted>"

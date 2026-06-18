#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/create-sbom.sh --version vX.Y.Z --out-dir DIR
  [--metadata-file FILE] [--runtime-bundle-manifest FILE]

Create a redacted SPDX 2.3 release dry-run SBOM from locked Cargo metadata.
The SBOM can include reviewed runtime bundle components from a redacted runtime
bundle manifest. It omits local manifest paths, source paths, license-file
paths, target directories, runtime data, diagnostics, model caches, and resume
data.
EOF
}

CARGO_BIN="${CARGO:-}"
if [ -z "$CARGO_BIN" ]; then
  CARGO_BIN=cargo
fi
if ! "$CARGO_BIN" --version >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi
if ! "$CARGO_BIN" --version >/dev/null 2>&1; then
  fail "cargo is required"
fi

version=""
out_dir=""
metadata_file=""
runtime_bundle_manifest=""

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      [ $# -ge 2 ] || fail "--version requires a value"
      version="$2"
      shift 2
      ;;
    --out-dir)
      [ $# -ge 2 ] || fail "--out-dir requires a value"
      out_dir="$2"
      shift 2
      ;;
    --metadata-file)
      [ $# -ge 2 ] || fail "--metadata-file requires a value"
      metadata_file="$2"
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
[ -n "$out_dir" ] || fail "--out-dir is required"

printf '%s\n' "$version" | grep -Eq '^v[0-9]+[.][0-9]+[.][0-9]+$' \
  || fail "version must look like vX.Y.Z"

cleanup_metadata=""
if [ -z "$metadata_file" ]; then
  metadata_file="$(mktemp "${TMPDIR:-/tmp}/resume-ir-sbom-metadata.XXXXXX")"
  cleanup_metadata="$metadata_file"
  "$CARGO_BIN" metadata --format-version 1 --locked > "$metadata_file"
fi

if [ -n "$cleanup_metadata" ]; then
  trap 'rm -f "$cleanup_metadata"' EXIT HUP INT TERM
fi

[ -f "$metadata_file" ] || fail "metadata file does not exist"
if [ -n "$runtime_bundle_manifest" ]; then
  [ -f "$runtime_bundle_manifest" ] || fail "runtime bundle manifest does not exist"
fi
mkdir -p "$out_dir"

sbom="$out_dir/release-sbom.json"
tmp_sbom="$sbom.tmp"

python3 - "$metadata_file" "$version" "$tmp_sbom" "${runtime_bundle_manifest:-}" <<'PY'
import datetime
import json
import re
import sys
import urllib.parse

metadata_path, version, output_path, runtime_bundle_manifest_path = sys.argv[1:5]

with open(metadata_path, "r", encoding="utf-8") as handle:
    metadata = json.load(handle)


def sanitize_spdx_id(value):
    sanitized = re.sub(r"[^A-Za-z0-9.-]", "-", value)
    sanitized = re.sub(r"-+", "-", sanitized).strip("-")
    return sanitized or "unknown"


def purl(package):
    name = urllib.parse.quote(package["name"], safe="")
    version_value = urllib.parse.quote(package["version"], safe="")
    return f"pkg:cargo/{name}@{version_value}"


def source_kind(source):
    if source is None:
        return "workspace"
    if source.startswith("registry+"):
        return "registry"
    if source.startswith("git+"):
        return "git"
    return "other"


def dependency_entry(dependency):
    entry = {
        "name": dependency["name"],
        "req": dependency.get("req") or "*",
        "kind": dependency.get("kind") or "normal",
        "optional": bool(dependency.get("optional")),
        "uses_default_features": bool(dependency.get("uses_default_features")),
        "features": sorted(dependency.get("features") or []),
    }
    if dependency.get("rename"):
        entry["rename"] = dependency["rename"]
    if dependency.get("target"):
        entry["target"] = dependency["target"]
    return entry


def fail(message):
    raise SystemExit(message)


def require_runtime_string(mapping, key):
    value = mapping.get(key)
    if not isinstance(value, str) or not value:
        fail(f"runtime bundle manifest missing {key}")
    return value


def require_runtime_bool(mapping, key, expected):
    value = mapping.get(key)
    if value is not expected:
        fail(f"runtime bundle manifest invalid {key}")


def require_runtime_int(mapping, key):
    value = mapping.get(key)
    if not isinstance(value, int) or value <= 0:
        fail(f"runtime bundle manifest invalid {key}")
    return value


def require_runtime_sha256(mapping, key):
    value = require_runtime_string(mapping, key)
    if not re.fullmatch(r"[0-9a-fA-F]{64}", value):
        fail(f"runtime bundle manifest invalid {key}")
    return value.lower()


def runtime_package(component, runtime_document, created, index):
    component_id = require_runtime_string(component, "id")
    kind = require_runtime_string(component, "kind")
    file_name = require_runtime_string(component, "file")
    if "/" in file_name or "\\" in file_name or file_name in {"", ".", ".."}:
        fail("runtime component file must be a basename")
    bytes_value = require_runtime_int(component, "bytes")
    sha256 = require_runtime_sha256(component, "sha256")
    license_obj = component.get("license")
    if not isinstance(license_obj, dict):
        fail("runtime component license is missing")
    license_id = require_runtime_string(license_obj, "id")
    require_runtime_bool(license_obj, "reviewed", True)
    source = require_runtime_string(component, "source")
    if source.startswith("/") or "PRIVATE-" in source:
        fail("runtime component source is private")
    source_offer = runtime_document.get("source_offer")
    if not isinstance(source_offer, dict):
        fail("runtime bundle source_offer is missing")
    source_offer_file = require_runtime_string(source_offer, "file")
    if "/" in source_offer_file or "\\" in source_offer_file:
        fail("runtime source_offer file must be a basename")
    source_offer_sha256 = require_runtime_sha256(source_offer, "sha256")
    distribution_license = require_runtime_string(runtime_document, "distribution_license")
    require_runtime_bool(runtime_document, "runtime_package_binaries_included", True)
    require_runtime_bool(runtime_document, "runtime_binaries_included", False)
    runtime_mode = require_runtime_string(runtime_document, "runtime_distribution_mode")
    if runtime_mode != "bundled":
        fail("runtime bundle mode must be bundled")

    spdx_id = (
        "SPDXRef-Runtime-"
        + sanitize_spdx_id(f"{component_id}-{index}")
    )
    return {
        "SPDXID": spdx_id,
        "name": component_id,
        "versionInfo": runtime_document.get("version") or version,
        "supplier": "NOASSERTION",
        "downloadLocation": source,
        "filesAnalyzed": False,
        "licenseConcluded": license_id,
        "licenseDeclared": license_id,
        "copyrightText": "NOASSERTION",
        "checksums": [{"algorithm": "SHA256", "checksumValue": sha256}],
        "annotations": [
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": f"runtime_distribution_mode={runtime_mode}",
            },
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": "runtime_package_binaries_included=true",
            },
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": "runtime_binaries_included=false",
            },
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": f"runtime_component_kind={kind}",
            },
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": f"runtime_component_file={file_name}",
            },
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": f"runtime_component_bytes={bytes_value}",
            },
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": f"runtime_distribution_license={distribution_license}",
            },
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": f"source_offer_file={source_offer_file}",
            },
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": f"source_offer_sha256={source_offer_sha256}",
            },
        ],
        "externalRefs": [
            {
                "referenceCategory": "OTHER",
                "referenceType": "persistent-id",
                "referenceLocator": f"runtime-bundle:{runtime_document['runtime_pack_id']}:{component_id}",
            }
        ],
    }


packages = []
relationships = []
created = (
    datetime.datetime.now(datetime.timezone.utc)
    .replace(microsecond=0)
    .isoformat()
    .replace("+00:00", "Z")
)
for index, package in enumerate(
    sorted(
        metadata.get("packages", []),
        key=lambda item: (
            item.get("name") or "",
            item.get("version") or "",
            item.get("source") or "",
        ),
    ),
    start=1,
):
    spdx_id = (
        "SPDXRef-Package-"
        + sanitize_spdx_id(f"{package['name']}-{package['version']}-{index}")
    )
    license_expr = (package.get("license") or "").strip() or "NOASSERTION"
    package_record = {
        "SPDXID": spdx_id,
        "name": package["name"],
        "versionInfo": package["version"],
        "supplier": "NOASSERTION",
        "downloadLocation": "NOASSERTION",
        "filesAnalyzed": False,
        "licenseConcluded": license_expr,
        "licenseDeclared": license_expr,
        "copyrightText": "NOASSERTION",
        "externalRefs": [
            {
                "referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": purl(package),
            }
        ],
        "annotations": [
            {
                "annotationType": "OTHER",
                "annotator": "Tool: resume-ir-release-sbom",
                "annotationDate": created,
                "comment": f"source_kind={source_kind(package.get('source'))}",
            }
        ],
        "dependencies": [
            dependency_entry(dependency)
            for dependency in sorted(
                package.get("dependencies") or [],
                key=lambda item: (
                    item.get("name") or "",
                    item.get("kind") or "",
                    item.get("req") or "",
                ),
            )
        ],
    }
    packages.append(package_record)
    relationships.append(
        {
            "spdxElementId": "SPDXRef-DOCUMENT",
            "relationshipType": "DESCRIBES",
            "relatedSpdxElement": spdx_id,
        }
    )

if runtime_bundle_manifest_path:
    with open(runtime_bundle_manifest_path, "r", encoding="utf-8") as handle:
        runtime_document = json.load(handle)
    if runtime_document.get("schema_version") != "release.runtime_bundle.v1":
        fail("runtime bundle manifest schema_version is invalid")
    components = runtime_document.get("components")
    if not isinstance(components, list) or not components:
        fail("runtime bundle manifest components are missing")
    seen_runtime_ids = set()
    for index, component in enumerate(components, start=1):
        if not isinstance(component, dict):
            fail("runtime bundle manifest component is invalid")
        component_id = component.get("id")
        if component_id in seen_runtime_ids:
            fail("runtime bundle manifest duplicate component")
        seen_runtime_ids.add(component_id)
        package_record = runtime_package(component, runtime_document, created, index)
        packages.append(package_record)
        relationships.append(
            {
                "spdxElementId": "SPDXRef-DOCUMENT",
                "relationshipType": "DESCRIBES",
                "relatedSpdxElement": package_record["SPDXID"],
            }
        )

document = {
    "spdxVersion": "SPDX-2.3",
    "dataLicense": "CC0-1.0",
    "SPDXID": "SPDXRef-DOCUMENT",
    "name": f"resume-ir-{version}",
    "documentNamespace": f"https://github.com/FrankQDWang/resume-ir/sbom/{version}",
    "creationInfo": {
        "created": created,
        "creators": ["Tool: resume-ir-release-sbom"],
    },
    "packages": packages,
    "relationships": relationships,
}

with open(output_path, "w", encoding="utf-8") as handle:
    json.dump(document, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY

mv "$tmp_sbom" "$sbom"
printf '%s\n' "$sbom"

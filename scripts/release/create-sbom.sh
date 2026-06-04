#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/create-sbom.sh --version vX.Y.Z --out-dir DIR [--metadata-file FILE]

Create a redacted SPDX 2.3 release dry-run SBOM from locked Cargo metadata.
The SBOM omits local manifest paths, source paths, license-file paths, target
directories, runtime data, diagnostics, model caches, and resume data.
EOF
}

CARGO_BIN="${CARGO:-cargo}"
if ! command -v "$CARGO_BIN" >/dev/null 2>&1 && [ -x /Users/frankqdwang/.cargo/bin/cargo ]; then
  CARGO_BIN=/Users/frankqdwang/.cargo/bin/cargo
fi

version=""
out_dir=""
metadata_file=""

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
mkdir -p "$out_dir"

sbom="$out_dir/release-sbom.json"
tmp_sbom="$sbom.tmp"

python3 - "$metadata_file" "$version" "$tmp_sbom" <<'PY'
import datetime
import json
import re
import sys
import urllib.parse

metadata_path, version, output_path = sys.argv[1:4]

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

#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required release SBOM file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

script="scripts/release/create-sbom.sh"
workflow=".github/workflows/release.yml"
verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"

require_file "$script"
require_file "$workflow"
require_file "$verify_script"
require_file "$workflow_guard"

if [ ! -x "$script" ]; then
  fail "release SBOM script is not executable"
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-sbom-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

out_dir="$tmpdir/out"
private_component_dir="$tmpdir/PRIVATE-runtime-components"
mkdir -p "$private_component_dir"
printf 'synthetic tesseract bytes\n' > "$private_component_dir/tesseract"
printf 'source offer text\n' > "$private_component_dir/source-offer.txt"
scripts/release/create-runtime-bundle-manifest.sh \
  --version v0.0.0 \
  --runtime-pack-id reviewed-runtime-pack \
  --distribution-license GPL-3.0-or-later \
  --source-offer "$private_component_dir/source-offer.txt" \
  --component "tesseract|ocr-engine|Apache-2.0|https://github.com/tesseract-ocr/tesseract|$private_component_dir/tesseract" \
  --out-dir "$out_dir/runtime" \
  --reviewed \
  > "$tmpdir/runtime-bundle.stdout"
unknown_runtime_manifest="$tmpdir/runtime-bundle-unknown-field.json"
python3 - "$out_dir/runtime/runtime-bundle-manifest.json" "$unknown_runtime_manifest" <<'PY'
import json
import sys

source = sys.argv[1]
target = sys.argv[2]

with open(source, "r", encoding="utf-8") as handle:
    document = json.load(handle)

document["components"][0]["local_probe_path"] = "redacted-sbom-runtime-cache"

with open(target, "w", encoding="utf-8") as handle:
    json.dump(document, handle)
    handle.write("\n")
PY

if "$script" \
  --version v0.0.0 \
  --out-dir "$out_dir/unknown-runtime" \
  --runtime-bundle-manifest "$unknown_runtime_manifest" \
  >/dev/null 2>&1; then
  fail "release SBOM script accepted unknown runtime bundle manifest fields"
fi

"$script" \
  --version v0.0.0 \
  --out-dir "$out_dir" \
  --runtime-bundle-manifest "$out_dir/runtime/runtime-bundle-manifest.json"
sbom="$out_dir/release-sbom.json"
require_file "$sbom"
require_text "$sbom" '"spdxVersion": "SPDX-2.3"'
require_text "$sbom" '"name": "resume-ir-v0.0.0"'
require_text "$sbom" '"SPDXID": "SPDXRef-DOCUMENT"'
require_text "$sbom" '"filesAnalyzed": false'
require_text "$sbom" '"referenceType": "purl"'
require_text "$sbom" '"referenceLocator": "pkg:cargo/resume-cli@0.1.0"'
require_text "$sbom" '"referenceLocator": "pkg:cargo/resume-daemon@0.1.0"'
require_text "$sbom" '"referenceLocator": "pkg:cargo/benchmark-runner@0.1.0"'
require_text "$sbom" '"name": "tesseract"'
require_text "$sbom" '"runtime_distribution_mode=bundled"'
require_text "$sbom" '"runtime_package_binaries_included=true"'
require_text "$sbom" '"runtime_binaries_included=false"'
require_text "$sbom" '"source_offer_sha256='
require_text "$sbom" '"licenseDeclared": "MIT"'
require_text "$sbom" '"licenseDeclared": "Apache-2.0"'
require_text "$sbom" '"source_kind=workspace"'
require_text "$sbom" '"source_kind=registry"'

if grep -Fq "$tmpdir" "$sbom"; then
  fail "release SBOM leaked an absolute temp path"
fi
if grep -Eq 'PRIVATE-runtime-components|manifest_path|src_path|license_file|/Users/|target/release|local-data|diagnostics|model-cache' "$sbom"; then
  fail "release SBOM leaked a local path or runtime-data marker"
fi

python3 - "$sbom" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    document = json.load(handle)

packages = document.get("packages")
if not isinstance(packages, list) or not packages:
    raise SystemExit("SBOM package list is empty")
if not any(package.get("name") == "resume-cli" for package in packages):
    raise SystemExit("SBOM does not include resume-cli")
if not any(package.get("name") == "tesseract" for package in packages):
    raise SystemExit("SBOM does not include runtime package")
if not all("manifest_path" not in package for package in packages):
    raise SystemExit("SBOM contains manifest_path")
PY

if "$script" --version 0.0.0 --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "release SBOM script accepted an invalid version"
fi

require_text "$workflow" "scripts/release/create-sbom.sh"
require_text "$workflow" "release-sbom.json"
require_text "$workflow" "release-dry-run/*.json"
require_text "$workflow" "Packaging, signing, notarization"
require_text "$verify_script" "./scripts/ci/check-release-sbom.sh"
require_text "$workflow_guard" "check-release-sbom.sh"

printf '%s\n' "release SBOM check passed"

#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required runtime SBOM file: $1"
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
runtime_bundle_script="scripts/release/create-runtime-bundle-manifest.sh"
release_readiness_check="scripts/ci/check-release-readiness.sh"
release_runbook="docs/runbooks/release-blockers.md"
verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"

require_file "$script"
require_file "$runtime_bundle_script"
require_file "$release_readiness_check"
require_file "$release_runbook"
require_file "$verify_script"
require_file "$workflow_guard"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-runtime-sbom-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

private_component_dir="$tmpdir/PRIVATE-runtime-components"
out_dir="$tmpdir/out"
mkdir -p "$private_component_dir" "$out_dir"
printf 'synthetic tesseract bytes\n' > "$private_component_dir/tesseract"
printf 'synthetic tessdata bytes\n' > "$private_component_dir/eng.traineddata"
printf 'synthetic pdf renderer bytes\n' > "$private_component_dir/pdftoppm"
printf 'source offer text\n' > "$private_component_dir/source-offer.txt"
printf 'notice text\n' > "$private_component_dir/NOTICE.txt"

"$runtime_bundle_script" \
  --version v0.0.0 \
  --runtime-pack-id reviewed-runtime-pack \
  --distribution-license GPL-3.0-or-later \
  --source-offer "$private_component_dir/source-offer.txt" \
  --notice "$private_component_dir/NOTICE.txt" \
  --component "tesseract|ocr-engine|Apache-2.0|https://github.com/tesseract-ocr/tesseract|$private_component_dir/tesseract" \
  --component "eng-tessdata|ocr-language-pack|Apache-2.0|https://github.com/tesseract-ocr/tessdata|$private_component_dir/eng.traineddata" \
  --component "poppler-pdftoppm|pdf-renderer|GPL-3.0-or-later|https://poppler.freedesktop.org/|$private_component_dir/pdftoppm" \
  --out-dir "$out_dir" \
  --reviewed \
  > "$tmpdir/runtime-bundle.stdout"

"$script" \
  --version v0.0.0 \
  --out-dir "$out_dir/sbom" \
  --runtime-bundle-manifest "$out_dir/runtime-bundle-manifest.json" \
  > "$tmpdir/sbom.stdout"

sbom="$out_dir/sbom/release-sbom.json"
require_file "$sbom"
require_text "$sbom" '"spdxVersion": "SPDX-2.3"'
require_text "$sbom" '"name": "resume-ir-v0.0.0"'
require_text "$sbom" '"name": "tesseract"'
require_text "$sbom" '"name": "eng-tessdata"'
require_text "$sbom" '"name": "poppler-pdftoppm"'
require_text "$sbom" '"licenseDeclared": "Apache-2.0"'
require_text "$sbom" '"licenseDeclared": "GPL-3.0-or-later"'
require_text "$sbom" '"runtime_distribution_mode=bundled"'
require_text "$sbom" '"runtime_package_binaries_included=true"'
require_text "$sbom" '"runtime_binaries_included=false"'
require_text "$sbom" '"source_offer_sha256='
require_text "$sbom" '"relationshipType": "DESCRIBES"'

if grep -Eq "$tmpdir|PRIVATE-runtime-components|/Users/|local-data|diagnostics|model-cache|resume text|raw_path" "$sbom"; then
  fail "runtime SBOM leaked a local path or runtime-data marker"
fi

python3 - "$sbom" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    document = json.load(handle)

packages = document.get("packages")
if not isinstance(packages, list):
    raise SystemExit("SBOM packages missing")

runtime_packages = {
    package.get("name"): package
    for package in packages
    if any(
        annotation.get("comment", "").startswith("runtime_distribution_mode=")
        for annotation in package.get("annotations", [])
    )
}
required = {"tesseract", "eng-tessdata", "poppler-pdftoppm"}
if set(runtime_packages) != required:
    raise SystemExit("runtime package set is incomplete")
for name, package in runtime_packages.items():
    if package.get("filesAnalyzed") is not False:
        raise SystemExit(f"runtime package {name} analyzed files")
    if package.get("downloadLocation") == "NOASSERTION":
        raise SystemExit(f"runtime package {name} lacks source")
    if "checksums" not in package or not package["checksums"]:
        raise SystemExit(f"runtime package {name} lacks checksum")
PY

if "$script" \
  --version v0.0.0 \
  --out-dir "$out_dir/missing" \
  --runtime-bundle-manifest "$out_dir/missing-runtime-bundle.json" \
  >/dev/null 2>&1; then
  fail "release SBOM script accepted a missing runtime bundle manifest"
fi

require_text "$verify_script" "./scripts/ci/check-runtime-bundle-sbom.sh"
require_text "$workflow_guard" "check-runtime-bundle-sbom.sh"
require_text "$release_readiness_check" '"name":"tesseract"'
require_text "$release_readiness_check" '"runtime_distribution_mode=bundled"'
require_text "$release_runbook" "--runtime-bundle-manifest release-dry-run/runtime-bundle-manifest.json"
require_text "$release_runbook" "runtime packages"

printf '%s\n' "runtime bundle SBOM check passed"

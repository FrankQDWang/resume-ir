#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required runtime bundle file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

script="scripts/release/create-runtime-bundle-manifest.sh"
release_artifact_script="scripts/release/create-artifact-manifest.sh"
release_runbook="docs/runbooks/release-blockers.md"
worker_runbook="docs/runbooks/ocr-embedding-workers.md"
verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"

require_file "$release_artifact_script"
require_file "$release_runbook"
require_file "$worker_runbook"
require_file "$verify_script"
require_file "$workflow_guard"

if [ ! -f "$script" ]; then
  fail "missing required runtime bundle manifest script"
fi
if [ ! -x "$script" ]; then
  fail "runtime bundle manifest script is not executable"
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-runtime-bundle-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

private_component_dir="$tmpdir/PRIVATE-runtime-components"
target_dir="$tmpdir/target/release"
out_dir="$tmpdir/out"
mkdir -p "$private_component_dir" "$target_dir" "$out_dir"

printf 'synthetic tesseract bytes\n' > "$private_component_dir/tesseract"
printf 'synthetic tessdata bytes\n' > "$private_component_dir/eng.traineddata"
printf 'synthetic pdf renderer bytes\n' > "$private_component_dir/pdftoppm"
printf 'synthetic reviewed embedding model bytes\n' > "$private_component_dir/model.onnx"
printf 'source offer text\n' > "$private_component_dir/source-offer.txt"
printf 'notice text\n' > "$private_component_dir/NOTICE.txt"
for binary in resume-cli resume-daemon resume-benchmark; do
  printf 'synthetic binary %s\n' "$binary" > "$target_dir/$binary"
  chmod 755 "$target_dir/$binary"
done

if "$script" \
  --version v0.0.0 \
  --runtime-pack-id reviewed-runtime-pack \
  --distribution-license GPL-3.0-or-later \
  --source-offer "$private_component_dir/source-offer.txt" \
  --notice "$private_component_dir/NOTICE.txt" \
  --component "tesseract|ocr-engine|Apache-2.0|https://github.com/tesseract-ocr/tesseract|$private_component_dir/tesseract" \
  --out-dir "$out_dir/unreviewed" \
  >/dev/null 2>&1; then
  fail "runtime bundle manifest script accepted unreviewed runtime components"
fi

"$script" \
  --version v0.0.0 \
  --runtime-pack-id reviewed-runtime-pack \
  --distribution-license GPL-3.0-or-later \
  --source-offer "$private_component_dir/source-offer.txt" \
  --notice "$private_component_dir/NOTICE.txt" \
  --component "tesseract|ocr-engine|Apache-2.0|https://github.com/tesseract-ocr/tesseract|$private_component_dir/tesseract" \
  --component "eng-tessdata|ocr-language-pack|Apache-2.0|https://github.com/tesseract-ocr/tessdata|$private_component_dir/eng.traineddata" \
  --component "poppler-pdftoppm|pdf-renderer|GPL-3.0-or-later|https://poppler.freedesktop.org/|$private_component_dir/pdftoppm" \
  --component "all-minilm-l6-v2|embedding-model|Apache-2.0|https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2|$private_component_dir/model.onnx" \
  --reviewed \
  --out-dir "$out_dir" \
  > "$tmpdir/runtime-bundle.stdout"
require_text "$tmpdir/runtime-bundle.stdout" "runtime bundle manifest: written"
require_text "$tmpdir/runtime-bundle.stdout" "paths: <redacted>"
if grep -Eq "$tmpdir|PRIVATE-runtime-components|/Users/" "$tmpdir/runtime-bundle.stdout"; then
  fail "runtime bundle manifest script leaked a local path to stdout"
fi

manifest="$out_dir/runtime-bundle-manifest.json"
require_file "$manifest"
require_text "$manifest" '"schema_version": "release.runtime_bundle.v1"'
require_text "$manifest" '"version": "v0.0.0"'
require_text "$manifest" '"runtime_distribution_mode": "bundled"'
require_text "$manifest" '"runtime_package_binaries_included": true'
require_text "$manifest" '"runtime_binaries_included": false'
require_text "$manifest" '"distribution_license": "GPL-3.0-or-later"'
require_text "$manifest" '"source_offer"'
require_text "$manifest" '"notices"'
require_text "$manifest" '"components"'
require_text "$manifest" '"tesseract"'
require_text "$manifest" '"eng.traineddata"'
require_text "$manifest" '"pdftoppm"'
require_text "$manifest" '"model.onnx"'
require_text "$manifest" '"embedding-model"'
require_text "$manifest" '"sha256"'
require_text "$manifest" '"bytes"'

if grep -Eq "$tmpdir|PRIVATE-runtime-components|raw_path|/Users/|local-data|diagnostics|model-cache|resume text" "$manifest"; then
  fail "runtime bundle manifest leaked a local path or runtime-data marker"
fi

"$release_artifact_script" \
  --version v0.0.0 \
  --target-dir "$target_dir" \
  --out-dir "$out_dir/release" \
  --runtime-bundle-manifest "$manifest" \
  > "$tmpdir/release-artifacts.stdout"
release_manifest="$out_dir/release/release-artifacts.json"
require_file "$release_manifest"
require_text "$release_manifest" '"runtime_bundle_manifests"'
require_text "$release_manifest" '"runtime-bundle-manifest.json"'
require_text "$release_manifest" '"runtime_distribution_mode": "bundled"'
require_text "$release_manifest" '"runtime_package_binaries_included": true'
require_text "$release_manifest" '"runtime_binaries_included": false'
if grep -Eq "$tmpdir|PRIVATE-runtime-components|/Users/|local-data|diagnostics|model-cache|resume text" "$release_manifest"; then
  fail "release artifact manifest leaked a runtime bundle local path"
fi

require_text "$verify_script" "./scripts/ci/check-runtime-bundle-manifest.sh"
require_text "$workflow_guard" "check-runtime-bundle-manifest.sh"
require_text "$release_runbook" "scripts/release/create-runtime-bundle-manifest.sh"
require_text "$release_runbook" "release.runtime_bundle.v1"
require_text "$release_runbook" "runtime-bundle-manifest.json"
require_text "$worker_runbook" "scripts/release/create-runtime-bundle-manifest.sh"
require_text "$worker_runbook" "release.runtime_bundle.v1"

printf '%s\n' "runtime bundle manifest check passed"

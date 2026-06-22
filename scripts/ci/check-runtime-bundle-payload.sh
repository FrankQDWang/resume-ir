#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required runtime payload file: $1"
  fi
}

require_dir() {
  if [ ! -d "$1" ]; then
    fail "missing required runtime payload directory: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required runtime payload text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  if grep -Fq -- "$text" "$file"; then
    fail "$file leaked runtime payload marker: $text"
  fi
}

assemble_script="scripts/release/assemble-runtime-bundle.sh"
macos_package_script="scripts/release/create-macos-package.sh"
verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"
release_runbook="docs/runbooks/release-blockers.md"

for file in "$assemble_script" "$macos_package_script" "$verify_script" "$workflow_guard" "$release_runbook"; do
  require_file "$file"
done

if [ ! -x "$assemble_script" ]; then
  fail "runtime bundle assembly script is not executable"
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-runtime-payload-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

component_dir="$tmpdir/PRIVATE-runtime-components"
out_dir="$tmpdir/out"
target_dir="$tmpdir/target/release"
mkdir -p "$component_dir" "$out_dir" "$target_dir"

printf 'synthetic tesseract runtime binary\n' > "$component_dir/tesseract"
printf 'synthetic English tessdata payload\n' > "$component_dir/eng.traineddata"
printf 'synthetic PDF renderer runtime binary\n' > "$component_dir/pdftoppm"
printf 'synthetic reviewed embedding model payload\n' > "$component_dir/model.onnx"
printf 'synthetic source offer archive\n' > "$component_dir/source-offer.tar.gz"
printf 'synthetic notice text\n' > "$component_dir/NOTICE.txt"
for binary in resume-cli resume-daemon resume-benchmark; do
  printf 'synthetic macOS binary %s\n' "$binary" > "$target_dir/$binary"
  chmod 755 "$target_dir/$binary"
done

if "$assemble_script" \
  --version v0.0.0 \
  --runtime-pack-id reviewed-runtime-pack \
  --distribution-license GPL-3.0-or-later \
  --source-offer "$component_dir/source-offer.tar.gz" \
  --component "tesseract|ocr-engine|Apache-2.0|https://github.com/tesseract-ocr/tesseract|$component_dir/tesseract" \
  --out-dir "$out_dir/unreviewed" \
  >/dev/null 2>&1; then
  fail "runtime bundle assembly accepted unreviewed runtime components"
fi

"$assemble_script" \
  --version v0.0.0 \
  --runtime-pack-id reviewed-runtime-pack \
  --distribution-license GPL-3.0-or-later \
  --source-offer "$component_dir/source-offer.tar.gz" \
  --notice "$component_dir/NOTICE.txt" \
  --component "tesseract|ocr-engine|Apache-2.0|https://github.com/tesseract-ocr/tesseract|$component_dir/tesseract" \
  --component "eng-tessdata|ocr-language-pack|Apache-2.0|https://github.com/tesseract-ocr/tessdata|$component_dir/eng.traineddata" \
  --component "poppler-pdftoppm|pdf-renderer|GPL-3.0-or-later|https://poppler.freedesktop.org/|$component_dir/pdftoppm" \
  --component "all-minilm-l6-v2|embedding-model|Apache-2.0|https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2|$component_dir/model.onnx" \
  --reviewed \
  --out-dir "$out_dir" \
  > "$tmpdir/assemble.stdout"

require_text "$tmpdir/assemble.stdout" "runtime bundle payload: written"
require_text "$tmpdir/assemble.stdout" "runtime-bundle-manifest.json"
require_text "$tmpdir/assemble.stdout" "runtime dir: <redacted>"
if grep -Eq "$tmpdir|PRIVATE-runtime-components|/Users/" "$tmpdir/assemble.stdout"; then
  fail "runtime bundle assembly leaked a local path to stdout"
fi

manifest="$out_dir/runtime-bundle-manifest.json"
runtime_dir="$out_dir/runtime"
evidence_dir="$out_dir/evidence"
require_file "$manifest"
require_dir "$runtime_dir"
require_dir "$evidence_dir"
require_file "$runtime_dir/tesseract"
require_file "$runtime_dir/eng.traineddata"
require_file "$runtime_dir/pdftoppm"
require_file "$runtime_dir/model.onnx"
require_file "$evidence_dir/source-offer.tar.gz"
require_file "$evidence_dir/NOTICE.txt"

require_text "$manifest" '"schema_version": "release.runtime_bundle.v1"'
require_text "$manifest" '"runtime_distribution_mode": "bundled"'
require_text "$manifest" '"runtime_package_binaries_included": true'
require_text "$manifest" '"runtime_binaries_included": false'
require_text "$manifest" '"source_offer"'
require_text "$manifest" '"notices"'
require_text "$manifest" '"components"'
require_text "$manifest" '"file": "tesseract"'
require_text "$manifest" '"file": "eng.traineddata"'
require_text "$manifest" '"file": "pdftoppm"'
require_text "$manifest" '"file": "model.onnx"'
require_text "$manifest" '"kind": "embedding-model"'
if grep -Eq "$tmpdir|PRIVATE-runtime-components|raw_path|/Users/|local-data|diagnostics|model-cache|resume text" "$manifest"; then
  fail "runtime bundle assembly manifest leaked a local path or runtime-data marker"
fi

case "$(uname -s)" in
  Darwin)
    command -v pkgutil >/dev/null 2>&1 || fail "pkgutil is required"
    fake_bin="$tmpdir/fake-bin"
    package_out="$tmpdir/package-out"
    mkdir -p "$fake_bin" "$package_out"
    cat > "$fake_bin/hdiutil" <<'SH'
#!/usr/bin/env sh
set -eu
out=""
for arg in "$@"; do
  out="$arg"
done
[ -n "$out" ] || exit 1
printf '%s\n' "synthetic dmg placeholder" > "$out"
SH
    chmod 755 "$fake_bin/hdiutil"
    PATH="$fake_bin:$PATH" "$macos_package_script" \
      --version v0.0.0 \
      --target-dir "$target_dir" \
      --out-dir "$package_out" \
      --runtime-bundle-manifest "$manifest" \
      --runtime-bundle-dir "$runtime_dir" \
      > "$tmpdir/macos-package.stdout"
    package_manifest="$package_out/macos-package.json"
    require_file "$package_manifest"
    require_text "$package_manifest" '"runtime_payload"'
    require_text "$package_manifest" '"runtime_distribution_mode": "bundled"'
    require_text "$package_manifest" '"file": "tesseract"'
    require_text "$package_manifest" '"file": "eng.traineddata"'
    require_text "$package_manifest" '"file": "pdftoppm"'
    require_text "$package_manifest" '"file": "model.onnx"'
    if grep -Fq "$tmpdir" "$package_manifest"; then
      fail "runtime package manifest leaked assembled payload path"
    fi
    ;;
  *)
    printf '%s\n' "runtime bundle payload package smoke skipped on non-Darwin"
    ;;
esac

require_text "$verify_script" "./scripts/ci/check-runtime-bundle-payload.sh"
require_text "$workflow_guard" "check-runtime-bundle-payload.sh"
require_text "$release_runbook" "scripts/release/assemble-runtime-bundle.sh"
require_text "$release_runbook" "--runtime-bundle-dir <assembled-runtime-dir>"

printf '%s\n' "runtime bundle payload check passed"

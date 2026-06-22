#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required runtime package file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required runtime package text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  if grep -Fq -- "$text" "$file"; then
    fail "$file leaked runtime package marker: $text"
  fi
}

runtime_bundle_script="scripts/release/create-runtime-bundle-manifest.sh"
macos_package_script="scripts/release/create-macos-package.sh"
windows_package_script="scripts/release/create-windows-package.ps1"
verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"
release_runbook="docs/runbooks/release-blockers.md"

for file in "$runtime_bundle_script" "$macos_package_script" "$windows_package_script" "$verify_script" "$workflow_guard" "$release_runbook"; do
  require_file "$file"
done

require_text "$macos_package_script" "--runtime-bundle-manifest"
require_text "$macos_package_script" "--runtime-bundle-dir"
require_text "$macos_package_script" "release.runtime_package_payload.v1"
require_text "$windows_package_script" "RuntimeBundleManifest"
require_text "$windows_package_script" "RuntimeBundleDir"
require_text "$windows_package_script" "release.runtime_package_payload.v1"
require_text "$verify_script" "./scripts/ci/check-runtime-bundle-package.sh"
require_text "$workflow_guard" "check-runtime-bundle-package.sh"
require_text "$release_runbook" "--runtime-bundle-dir <assembled-runtime-dir>"

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-runtime-package-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

runtime_dir="$tmpdir/runtime-components"
out_dir="$tmpdir/runtime-manifest"
mkdir -p "$runtime_dir" "$out_dir"
printf '%s\n' 'synthetic tesseract runtime binary' > "$runtime_dir/tesseract"
printf '%s\n' 'synthetic English tessdata payload' > "$runtime_dir/eng.traineddata"
printf '%s\n' 'synthetic PDF renderer runtime binary' > "$runtime_dir/pdftoppm"
printf '%s\n' 'synthetic reviewed embedding model payload' > "$runtime_dir/model.onnx"
printf '%s\n' 'synthetic source offer archive' > "$runtime_dir/source-offer.tar.gz"

"$runtime_bundle_script" \
  --version v0.0.0 \
  --runtime-pack-id reviewed-runtime-pack \
  --distribution-license GPL-3.0-or-later \
  --source-offer "$runtime_dir/source-offer.tar.gz" \
  --component "tesseract|ocr-engine|Apache-2.0|https://github.com/tesseract-ocr/tesseract|$runtime_dir/tesseract" \
  --component "eng-tessdata|ocr-language-pack|Apache-2.0|https://github.com/tesseract-ocr/tessdata|$runtime_dir/eng.traineddata" \
  --component "poppler-pdftoppm|pdf-renderer|GPL-3.0-or-later|https://poppler.freedesktop.org/|$runtime_dir/pdftoppm" \
  --component "all-minilm-l6-v2|embedding-model|Apache-2.0|https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2|$runtime_dir/model.onnx" \
  --out-dir "$out_dir" \
  --reviewed \
  > "$tmpdir/runtime-bundle.stdout"

case "$(uname -s)" in
  Darwin)
    command -v pkgutil >/dev/null 2>&1 || fail "pkgutil is required"
    command -v hdiutil >/dev/null 2>&1 || fail "hdiutil is required"
    fake_bin="$tmpdir/fake-bin"
    mkdir -p "$fake_bin"
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
    target_dir="$tmpdir/target/release"
    package_out="$tmpdir/package-out"
    mkdir -p "$target_dir" "$package_out"
    for binary in resume-cli resume-daemon resume-benchmark; do
      printf 'synthetic macOS binary %s\n' "$binary" > "$target_dir/$binary"
      chmod 755 "$target_dir/$binary"
    done
    unknown_manifest="$tmpdir/runtime-bundle-unknown-field.json"
    python3 - "$out_dir/runtime-bundle-manifest.json" "$unknown_manifest" <<'PY'
import json
import sys

source = sys.argv[1]
target = sys.argv[2]

with open(source, "r", encoding="utf-8") as handle:
    document = json.load(handle)

document["components"][0]["local_probe_path"] = "PRIVATE-runtime-package-cache"

with open(target, "w", encoding="utf-8") as handle:
    json.dump(document, handle)
    handle.write("\n")
PY

    if PATH="$fake_bin:$PATH" "$macos_package_script" \
      --version v0.0.0 \
      --target-dir "$target_dir" \
      --out-dir "$tmpdir/package-unknown" \
      --runtime-bundle-manifest "$unknown_manifest" \
      --runtime-bundle-dir "$runtime_dir" \
      >/dev/null 2>&1; then
      fail "macOS package script accepted unknown runtime bundle manifest fields"
    fi

    PATH="$fake_bin:$PATH" "$macos_package_script" \
      --version v0.0.0 \
      --target-dir "$target_dir" \
      --out-dir "$package_out" \
      --runtime-bundle-manifest "$out_dir/runtime-bundle-manifest.json" \
      --runtime-bundle-dir "$runtime_dir" \
      > "$tmpdir/macos-package.stdout"
    manifest="$package_out/macos-package.json"
    pkg="$package_out/resume-ir-v0.0.0-macos.pkg"
    require_file "$manifest"
    require_file "$pkg"
    require_text "$manifest" '"runtime_payload"'
    require_text "$manifest" '"schema_version": "release.runtime_package_payload.v1"'
    require_text "$manifest" '"runtime_distribution_mode": "bundled"'
    require_text "$manifest" '"runtime_package_binaries_included": true'
    require_text "$manifest" '"runtime_binaries_included_in_manifest": false'
    require_text "$manifest" '"install_location": "/usr/local/lib/resume-ir/runtime"'
    require_text "$manifest" '"file": "tesseract"'
    require_text "$manifest" '"file": "eng.traineddata"'
    require_text "$manifest" '"file": "pdftoppm"'
    require_text "$manifest" '"file": "model.onnx"'
    if pkgutil --payload-files "$pkg" >/dev/null 2>&1; then
      pkgutil --payload-files "$pkg" > "$tmpdir/payload-files.txt"
      require_text "$tmpdir/payload-files.txt" "usr/local/lib/resume-ir/runtime/tesseract"
      require_text "$tmpdir/payload-files.txt" "usr/local/lib/resume-ir/runtime/eng.traineddata"
      require_text "$tmpdir/payload-files.txt" "usr/local/lib/resume-ir/runtime/pdftoppm"
      require_text "$tmpdir/payload-files.txt" "usr/local/lib/resume-ir/runtime/model.onnx"
    fi
    ;;
  *)
    printf '%s\n' "runtime package payload execution skipped on non-Darwin"
    ;;
esac

if [ -f "${manifest:-/nonexistent}" ]; then
  if grep -Fq "$tmpdir" "$manifest"; then
    fail "runtime package manifest leaked an absolute temp path"
  fi
  reject_text "$manifest" "PRIVATE-runtime-components"
  reject_text "$manifest" "target/release"
  reject_text "$manifest" "model-cache"
fi

printf '%s\n' "runtime bundle package check passed"

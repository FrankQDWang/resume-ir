#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "local OCR runtime check missing expected text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  label="$3"
  if [ -n "$text" ] && grep -Fq -- "$text" "$file"; then
    fail "local OCR runtime check leaked $label"
  fi
}

manifest_script="scripts/local/prepare-local-ocr-runtime-manifest.sh"
if [ ! -f "$manifest_script" ]; then
  fail "missing local OCR runtime manifest preparation script"
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-local-ocr-runtime.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

bin_dir="$tmpdir/PRIVATE-ocr-bin"
mkdir -p "$bin_dir"
tesseract="$bin_dir/tesseract"
pdftoppm="$bin_dir/pdftoppm"
language_pack="$tmpdir/PRIVATE-tessdata-eng.traineddata"
cat > "$tesseract" <<'SH'
#!/usr/bin/env sh
if [ "${1:-}" = "--version" ]; then
  printf 'tesseract 5.5.1\n'
  exit 0
fi
exit 0
SH
cat > "$pdftoppm" <<'SH'
#!/usr/bin/env sh
if [ "${1:-}" = "-v" ]; then
  printf 'pdftoppm version 25.12.0\n'
  exit 0
fi
exit 0
SH
chmod 700 "$tesseract" "$pdftoppm"
printf '%s\n' "SYNTHETIC TESSDATA PAYLOAD" > "$language_pack"

fake_resume_cli="$tmpdir/fake-resume-cli"
fake_resume_cli_args="$tmpdir/fake-resume-cli-args.txt"
cat > "$fake_resume_cli" <<'SH'
#!/usr/bin/env sh
set -eu
printf '%s\n' "$*" >> "$FAKE_RESUME_CLI_ARGS"
if [ "${1:-}" != "ocr" ]; then
  printf 'unexpected fake resume-cli command\n' >&2
  exit 64
fi
case "${2:-}" in
  draft-manifest)
    out=""
    runtime_pack_id=""
    language=""
    reviewed="false"
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --out) out="$2"; shift 2 ;;
        --runtime-pack-id) runtime_pack_id="$2"; shift 2 ;;
        --language) language="$2"; shift 2 ;;
        --reviewed) reviewed="true"; shift ;;
        *) shift ;;
      esac
    done
    [ -n "$out" ] || exit 65
    cat > "$out" <<JSON
{"schema_version":"resume-ir.ocr-runtime-manifest.v1","runtime_pack_id":"$runtime_pack_id","components":[{"id":"tesseract","kind":"ocr-engine","engine":"tesseract","version":"5.5.1","artifact":{"path":"<fake-tesseract>","sha256":"synthetic"},"license":{"id":"Apache-2.0","reviewed":$reviewed}},{"id":"poppler-pdftoppm","kind":"pdf-renderer","engine":"poppler-pdftoppm","version":"25.12.0","artifact":{"path":"<fake-pdftoppm>","sha256":"synthetic"},"license":{"id":"GPL-2.0-or-later","reviewed":$reviewed}}],"languages":[{"id":"$language","artifact":{"path":"<fake-tessdata>","sha256":"synthetic"},"license":{"id":"Apache-2.0","reviewed":$reviewed}}]}
JSON
    printf 'ocr runtime manifest draft: written\npaths: <redacted>\n'
    ;;
  validate-manifest)
    printf 'ocr runtime manifest: valid\npaths: <redacted>\n'
    ;;
  *)
    printf 'unexpected fake resume-cli ocr command\n' >&2
    exit 64
    ;;
esac
SH
chmod 700 "$fake_resume_cli"

manifest_out="$tmpdir/PRIVATE-ocr-runtime-manifest.json"
manifest_stdout="$tmpdir/manifest-stdout.txt"
manifest_stderr="$tmpdir/manifest-stderr.txt"
FAKE_RESUME_CLI_ARGS="$fake_resume_cli_args" "$manifest_script" \
  --resume-cli "$fake_resume_cli" \
  --out "$manifest_out" \
  --runtime-pack-id reviewed-local-ocr-pack \
  --tesseract-command "$tesseract" \
  --pdftoppm-command "$pdftoppm" \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  --reviewed \
  > "$manifest_stdout" 2> "$manifest_stderr"

if [ -s "$manifest_stderr" ]; then
  fail "local OCR manifest preparation wrote stderr on success"
fi
if [ ! -s "$manifest_out" ]; then
  fail "local OCR manifest preparation did not write manifest"
fi
require_text "$manifest_stdout" "ocr runtime manifest: written"
require_text "$manifest_stdout" "schema: resume-ir.ocr-runtime-manifest.v1"
require_text "$manifest_stdout" "runtime pack: reviewed-local-ocr-pack"
require_text "$manifest_stdout" "engine: tesseract"
require_text "$manifest_stdout" "renderer: poppler-pdftoppm"
require_text "$manifest_stdout" "language: eng"
require_text "$manifest_stdout" "license reviewed: yes"
require_text "$manifest_stdout" "paths: <redacted>"
require_text "$fake_resume_cli_args" "draft-manifest"
require_text "$fake_resume_cli_args" "--tesseract-command"
require_text "$fake_resume_cli_args" "--pdftoppm-command"
require_text "$fake_resume_cli_args" "--language-pack"
require_text "$fake_resume_cli_args" "--reviewed"
require_text "$fake_resume_cli_args" "validate-manifest"
reject_text "$manifest_stdout" "$tmpdir" "temporary local path"
reject_text "$manifest_stderr" "$tmpdir" "temporary local path"
reject_text "$manifest_stdout" "PRIVATE-ocr-bin" "private OCR bin marker"
reject_text "$manifest_stderr" "PRIVATE-ocr-bin" "private OCR bin marker"
reject_text "$manifest_stdout" "SYNTHETIC TESSDATA" "language pack bytes"

unreviewed_stdout="$tmpdir/unreviewed-stdout.txt"
unreviewed_stderr="$tmpdir/unreviewed-stderr.txt"
set +e
FAKE_RESUME_CLI_ARGS="$fake_resume_cli_args" "$manifest_script" \
  --resume-cli "$fake_resume_cli" \
  --out "$tmpdir/PRIVATE-unreviewed-ocr-runtime-manifest.json" \
  --runtime-pack-id reviewed-local-ocr-pack \
  --tesseract-command "$tesseract" \
  --pdftoppm-command "$pdftoppm" \
  --language eng \
  --language-pack "$language_pack" \
  --engine-license Apache-2.0 \
  --renderer-license GPL-2.0-or-later \
  --language-license Apache-2.0 \
  > "$unreviewed_stdout" 2> "$unreviewed_stderr"
unreviewed_status=$?
set -e
if [ "$unreviewed_status" -eq 0 ]; then
  fail "local OCR manifest preparation accepted unreviewed runtime evidence"
fi
require_text "$unreviewed_stderr" "ocr runtime manifest blocked: legal review is incomplete"
reject_text "$unreviewed_stdout" "$tmpdir" "temporary local path"
reject_text "$unreviewed_stderr" "$tmpdir" "temporary local path"

printf '%s\n' "local OCR runtime check passed"

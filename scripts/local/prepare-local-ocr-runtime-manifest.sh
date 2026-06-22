#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage: scripts/local/prepare-local-ocr-runtime-manifest.sh
  --out FILE --runtime-pack-id ID --language LANG --language-pack FILE_OR_LANG_EQ_FILE
  --engine-license ID --renderer-license ID --language-license ID --reviewed
  [--tesseract-command PATH] [--pdftoppm-command PATH] [--resume-cli PATH]

Creates a reviewed local OCR runtime manifest from already installed external
Tesseract/tessdata and Poppler/pdftoppm dependencies. The command never installs
dependencies and prints only redacted status output.
EOF
  exit 2
}

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

need_value() {
  [ "$#" -ge 2 ] || usage
  [ -n "$2" ] || usage
}

require_arg() {
  name="$1"
  value="$2"
  [ -n "$value" ] || fail "ocr runtime manifest blocked: missing $name"
}

validate_identifier() {
  name="$1"
  value="$2"
  case "$value" in
    ''|*[!A-Za-z0-9._:/+-]*)
      fail "ocr runtime manifest blocked: invalid $name"
      ;;
  esac
}

validate_license_expression() {
  name="$1"
  value="$2"
  [ -n "$value" ] || fail "ocr runtime manifest blocked: invalid $name"
  if printf '%s' "$value" | LC_ALL=C grep -Eq '[^-A-Za-z0-9._:/+() ]'; then
    fail "ocr runtime manifest blocked: invalid $name"
  fi
}

resolve_command() {
  configured="$1"
  command_name="$2"
  if [ -n "$configured" ]; then
    case "$configured" in
      */*)
        [ -x "$configured" ] \
          || fail "ocr runtime manifest blocked: $command_name command is unavailable"
        printf '%s' "$configured"
        ;;
      *)
        resolved="$(command -v "$configured" 2>/dev/null || true)"
        [ -n "$resolved" ] \
          || fail "ocr runtime manifest blocked: $command_name command is unavailable"
        printf '%s' "$resolved"
        ;;
    esac
  else
    resolved="$(command -v "$command_name" 2>/dev/null || true)"
    [ -n "$resolved" ] \
      || fail "ocr runtime manifest blocked: $command_name command is unavailable"
    printf '%s' "$resolved"
  fi
}

resolve_resume_cli() {
  candidate="$1"
  case "$candidate" in
    */*)
      [ -x "$candidate" ] || fail "ocr runtime manifest blocked: resume-cli is unavailable"
      printf '%s' "$candidate"
      ;;
    *)
      command -v "$candidate" >/dev/null 2>&1 \
        || fail "ocr runtime manifest blocked: resume-cli is unavailable"
      printf '%s' "$candidate"
      ;;
  esac
}

out=""
runtime_pack_id=""
tesseract_command=""
pdftoppm_command=""
language=""
language_packs=""
engine_license=""
renderer_license=""
language_license=""
reviewed=0
resume_cli="resume-cli"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --out)
      need_value "$@"; out="$2"; shift 2
      ;;
    --runtime-pack-id)
      need_value "$@"; runtime_pack_id="$2"; shift 2
      ;;
    --tesseract-command)
      need_value "$@"; tesseract_command="$2"; shift 2
      ;;
    --pdftoppm-command)
      need_value "$@"; pdftoppm_command="$2"; shift 2
      ;;
    --language)
      need_value "$@"; language="$2"; shift 2
      ;;
    --language-pack)
      need_value "$@"
      if [ -n "$language_packs" ]; then
        language_packs="${language_packs}
$2"
      else
        language_packs="$2"
      fi
      shift 2
      ;;
    --engine-license)
      need_value "$@"; engine_license="$2"; shift 2
      ;;
    --renderer-license)
      need_value "$@"; renderer_license="$2"; shift 2
      ;;
    --language-license)
      need_value "$@"; language_license="$2"; shift 2
      ;;
    --reviewed)
      reviewed=1; shift
      ;;
    --resume-cli)
      need_value "$@"; resume_cli="$2"; shift 2
      ;;
    -h|--help)
      usage
      ;;
    *)
      usage
      ;;
  esac
done

require_arg "--out" "$out"
require_arg "--runtime-pack-id" "$runtime_pack_id"
require_arg "--language" "$language"
require_arg "--language-pack" "$language_packs"
require_arg "--engine-license" "$engine_license"
require_arg "--renderer-license" "$renderer_license"
require_arg "--language-license" "$language_license"

validate_identifier "--runtime-pack-id" "$runtime_pack_id"
validate_identifier "--language" "$language"
validate_license_expression "--engine-license" "$engine_license"
validate_license_expression "--renderer-license" "$renderer_license"
validate_license_expression "--language-license" "$language_license"

if [ "$reviewed" -ne 1 ]; then
  fail "ocr runtime manifest blocked: legal review is incomplete"
fi

resume_cli_cmd="$(resolve_resume_cli "$resume_cli")"
tesseract_cmd="$(resolve_command "$tesseract_command" "tesseract")"
pdftoppm_cmd="$(resolve_command "$pdftoppm_command" "pdftoppm")"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-ocr-manifest.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

language_pack_args_file="$tmpdir/language-packs.txt"
printf '%s\n' "$language_packs" > "$language_pack_args_file"
while IFS= read -r language_pack_arg; do
  [ -n "$language_pack_arg" ] || continue
  case "$language_pack_arg" in
    *=*)
      language_pack_path="${language_pack_arg#*=}"
      ;;
    *)
      language_pack_path="$language_pack_arg"
      ;;
  esac
  [ -f "$language_pack_path" ] \
    || fail "ocr runtime manifest blocked: language pack is unavailable"
done < "$language_pack_args_file"

out_parent="$(dirname "$out")"
if [ -n "$out_parent" ] && [ "$out_parent" != "." ]; then
  mkdir -p "$out_parent" \
    || fail "ocr runtime manifest blocked: output is unavailable"
fi

set -- "$resume_cli_cmd" ocr draft-manifest \
  --out "$out" \
  --runtime-pack-id "$runtime_pack_id" \
  --tesseract-command "$tesseract_cmd" \
  --pdftoppm-command "$pdftoppm_cmd" \
  --language "$language"
while IFS= read -r language_pack_arg; do
  [ -n "$language_pack_arg" ] || continue
  set -- "$@" --language-pack "$language_pack_arg"
done < "$language_pack_args_file"
set -- "$@" \
  --engine-license "$engine_license" \
  --renderer-license "$renderer_license" \
  --language-license "$language_license" \
  --reviewed

set +e
"$@" > "$tmpdir/draft.stdout" 2> "$tmpdir/draft.stderr"
draft_status=$?
set -e
if [ "$draft_status" -ne 0 ]; then
  fail "ocr runtime manifest blocked: manifest draft failed"
fi

set +e
"$resume_cli_cmd" ocr validate-manifest \
  --manifest "$out" \
  > "$tmpdir/validate.stdout" \
  2> "$tmpdir/validate.stderr"
validate_status=$?
set -e
if [ "$validate_status" -ne 0 ]; then
  fail "ocr runtime manifest blocked: manifest validation failed"
fi

printf '%s\n' "ocr runtime manifest: written"
printf '%s\n' "schema: resume-ir.ocr-runtime-manifest.v1"
printf 'runtime pack: %s\n' "$runtime_pack_id"
printf '%s\n' "engine: tesseract"
printf '%s\n' "renderer: poppler-pdftoppm"
printf 'language: %s\n' "$language"
printf 'engine license: %s\n' "$engine_license"
printf 'renderer license: %s\n' "$renderer_license"
printf 'language license: %s\n' "$language_license"
printf '%s\n' "license reviewed: yes"
printf '%s\n' "paths: <redacted>"

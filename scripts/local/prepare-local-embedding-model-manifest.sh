#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage: scripts/local/prepare-local-embedding-model-manifest.sh
  --out FILE --model-id ID --model-pack-id ID --dimension N --license ID
  [--hf-cache-root DIR] [--resume-cli PATH]

Creates a reviewed local model manifest from an already cached Hugging Face
sentence-transformers snapshot. The command never downloads model weights and
prints only redacted status output.
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
  [ -n "$value" ] || fail "embedding model manifest blocked: missing $name"
}

require_positive_int() {
  name="$1"
  value="$2"
  case "$value" in
    ''|*[!0-9]*) fail "embedding model manifest blocked: invalid $name" ;;
    0) fail "embedding model manifest blocked: invalid $name" ;;
  esac
}

normalize_license() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]'
}

extract_model_card_license() {
  readme="$1"
  awk '
    BEGIN { in_header = 0 }
    NR == 1 && $0 == "---" { in_header = 1; next }
    in_header && $0 == "---" { exit }
    in_header && $0 ~ /^license:[[:space:]]*/ {
      sub(/^license:[[:space:]]*/, "", $0)
      gsub(/["'\'']/, "", $0)
      print $0
      exit
    }
  ' "$readme"
}

resolve_resume_cli() {
  candidate="$1"
  case "$candidate" in
    */*)
      [ -x "$candidate" ] || fail "embedding model manifest blocked: resume-cli is unavailable"
      printf '%s' "$candidate"
      ;;
    *)
      command -v "$candidate" >/dev/null 2>&1 \
        || fail "embedding model manifest blocked: resume-cli is unavailable"
      printf '%s' "$candidate"
      ;;
  esac
}

model_id=""
model_pack_id=""
dimension=""
license_id=""
out=""
resume_cli="resume-cli"
hf_cache_root="${HF_HOME:-$HOME/.cache/huggingface}/hub"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --out)
      need_value "$@"; out="$2"; shift 2
      ;;
    --model-id)
      need_value "$@"; model_id="$2"; shift 2
      ;;
    --model-pack-id)
      need_value "$@"; model_pack_id="$2"; shift 2
      ;;
    --dimension)
      need_value "$@"; dimension="$2"; shift 2
      ;;
    --license)
      need_value "$@"; license_id="$2"; shift 2
      ;;
    --hf-cache-root)
      need_value "$@"; hf_cache_root="$2"; shift 2
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
require_arg "--model-id" "$model_id"
require_arg "--model-pack-id" "$model_pack_id"
require_arg "--dimension" "$dimension"
require_arg "--license" "$license_id"
require_positive_int "--dimension" "$dimension"

case "$model_id" in
  *[!A-Za-z0-9._~:/-]*|'')
    fail "embedding model manifest blocked: invalid model id"
    ;;
esac

cache_name="models--$(printf '%s' "$model_id" | sed 's#/#--#g')"
model_cache="$hf_cache_root/$cache_name"
ref_file="$model_cache/refs/main"
[ -f "$ref_file" ] || fail "embedding model manifest blocked: local model cache ref is unavailable"
snapshot="$(cat "$ref_file" 2>/dev/null || true)"
case "$snapshot" in
  ''|*[!A-Za-z0-9._-]*)
    fail "embedding model manifest blocked: local model cache ref is invalid"
    ;;
esac

snapshot_dir="$model_cache/snapshots/$snapshot"
readme="$snapshot_dir/README.md"
artifact="$snapshot_dir/model.safetensors"
[ -f "$readme" ] || fail "embedding model manifest blocked: local model card is unavailable"
[ -f "$artifact" ] || fail "embedding model manifest blocked: local model artifact is unavailable"

card_license="$(extract_model_card_license "$readme" || true)"
[ -n "$card_license" ] \
  || fail "embedding model manifest blocked: local model card license is unavailable"
if [ "$(normalize_license "$card_license")" != "$(normalize_license "$license_id")" ]; then
  fail "embedding model manifest blocked: local model license mismatch"
fi

resume_cli_cmd="$(resolve_resume_cli "$resume_cli")"
tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-embedding-manifest.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

out_parent="$(dirname "$out")"
if [ -n "$out_parent" ] && [ "$out_parent" != "." ]; then
  mkdir -p "$out_parent" \
    || fail "embedding model manifest blocked: output is unavailable"
fi

set +e
"$resume_cli_cmd" model draft-manifest \
  --out "$out" \
  --model-pack-id "$model_pack_id" \
  --model-id "$model_id" \
  --model-type embedding \
  --dimension "$dimension" \
  --format safetensors \
  --artifact "$artifact" \
  --license "$license_id" \
  --reviewed \
  > "$tmpdir/draft.stdout" \
  2> "$tmpdir/draft.stderr"
draft_status=$?
set -e
if [ "$draft_status" -ne 0 ]; then
  fail "embedding model manifest blocked: model manifest draft failed"
fi

set +e
"$resume_cli_cmd" model validate-manifest \
  --manifest "$out" \
  > "$tmpdir/validate.stdout" \
  2> "$tmpdir/validate.stderr"
validate_status=$?
set -e
if [ "$validate_status" -ne 0 ]; then
  fail "embedding model manifest blocked: model manifest validation failed"
fi

printf '%s\n' "embedding model manifest: written"
printf '%s\n' "schema: resume-ir.model-manifest.v1"
printf 'model id: %s\n' "$model_id"
printf 'dimension: %s\n' "$dimension"
printf 'format: %s\n' "safetensors"
printf 'license: %s\n' "$license_id"
printf '%s\n' "license source: local model card"
printf '%s\n' "license reviewed: yes"
printf '%s\n' "artifact: model.safetensors"
printf '%s\n' "paths: <redacted>"

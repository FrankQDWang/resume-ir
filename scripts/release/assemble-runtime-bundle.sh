#!/usr/bin/env sh
set -eu

usage() {
  cat <<'EOF'
usage: scripts/release/assemble-runtime-bundle.sh
  --version vX.Y.Z --runtime-pack-id ID --distribution-license ID
  --source-offer FILE --component ID|KIND|LICENSE|SOURCE|FILE
  [--component ID|KIND|LICENSE|SOURCE|FILE ...] [--notice FILE ...]
  --out-dir DIR --reviewed

Copy already reviewed local runtime components into an assembled release
payload directory and generate a redacted runtime bundle manifest from those
assembled files. This script does not download, license-review, sign, notarize,
publish, or upload runtime binaries.
EOF
}

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

need_value() {
  [ "$#" -ge 2 ] || fail "$1 requires a value"
  [ -n "$2" ] || fail "$1 requires a value"
}

require_tool() {
  command -v "$1" >/dev/null 2>&1 || fail "$1 is required"
}

version=""
runtime_pack_id=""
distribution_license=""
source_offer=""
out_dir=""
reviewed=0
component_args=""
notice_args=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      need_value "$@"; version="$2"; shift 2
      ;;
    --runtime-pack-id)
      need_value "$@"; runtime_pack_id="$2"; shift 2
      ;;
    --distribution-license)
      need_value "$@"; distribution_license="$2"; shift 2
      ;;
    --source-offer)
      need_value "$@"; source_offer="$2"; shift 2
      ;;
    --component)
      need_value "$@"
      component_args="${component_args}${component_args:+
}$2"
      shift 2
      ;;
    --notice)
      need_value "$@"
      notice_args="${notice_args}${notice_args:+
}$2"
      shift 2
      ;;
    --out-dir)
      need_value "$@"; out_dir="$2"; shift 2
      ;;
    --reviewed)
      reviewed=1; shift
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
[ -n "$runtime_pack_id" ] || fail "--runtime-pack-id is required"
[ -n "$distribution_license" ] || fail "--distribution-license is required"
[ -n "$source_offer" ] || fail "--source-offer is required"
[ -n "$component_args" ] || fail "--component is required"
[ -n "$out_dir" ] || fail "--out-dir is required"
[ "$reviewed" -eq 1 ] || fail "runtime bundle assembly blocked: legal review is incomplete"

require_tool python3

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
manifest_script="$script_dir/create-runtime-bundle-manifest.sh"
[ -x "$manifest_script" ] || fail "runtime bundle assembly blocked: manifest script is unavailable"

runtime_dir="$out_dir/runtime"
evidence_dir="$out_dir/evidence"

if [ -d "$runtime_dir" ] && [ -n "$(find "$runtime_dir" -mindepth 1 -maxdepth 1 2>/dev/null | head -n 1)" ]; then
  fail "runtime bundle assembly blocked: runtime directory is not empty"
fi
if [ -d "$evidence_dir" ] && [ -n "$(find "$evidence_dir" -mindepth 1 -maxdepth 1 2>/dev/null | head -n 1)" ]; then
  fail "runtime bundle assembly blocked: evidence directory is not empty"
fi

mkdir -p "$runtime_dir" "$evidence_dir"

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-runtime-assembly.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT HUP INT TERM

components_in="$tmpdir/components.in"
notices_in="$tmpdir/notices.in"
components_staged="$tmpdir/components.staged"
notices_staged="$tmpdir/notices.staged"
source_offer_staged="$tmpdir/source-offer.staged"
printf '%s\n' "$component_args" > "$components_in"
printf '%s\n' "$notice_args" > "$notices_in"

python3 - "$source_offer" "$components_in" "$notices_in" "$runtime_dir" "$evidence_dir" "$components_staged" "$notices_staged" "$source_offer_staged" <<'PY'
import os
import re
import shutil
import sys

(
    source_offer,
    components_path,
    notices_path,
    runtime_dir,
    evidence_dir,
    components_staged_path,
    notices_staged_path,
    source_offer_staged_path,
) = sys.argv[1:9]


def fail(message):
    print(message, file=sys.stderr)
    raise SystemExit(1)


def basename(path):
    value = os.path.basename(path)
    if not value or value in {".", ".."} or "/" in value or "\\" in value:
        fail("runtime bundle assembly blocked: invalid basename")
    return value


def validate_id(kind, value):
    if not value or not re.fullmatch(r"[A-Za-z0-9._:/+-]+", value):
        fail(f"runtime bundle assembly blocked: invalid {kind}")


def validate_component_kind(value):
    allowed = {
        "ocr-engine",
        "ocr-language-pack",
        "pdf-renderer",
        "embedding-model",
        "model-runtime",
        "license-notice",
        "support-tool",
    }
    if value not in allowed:
        fail("runtime bundle assembly blocked: invalid component kind")


def validate_source(value):
    if not value or value.startswith("/") or "PRIVATE-" in value:
        fail("runtime bundle assembly blocked: invalid component source")


def copy_reviewed_file(src, dst_dir, seen_basenames, context):
    if not os.path.isfile(src):
        fail(f"runtime bundle assembly blocked: {context} is unavailable")
    name = basename(src)
    if name in seen_basenames:
        fail("runtime bundle assembly blocked: duplicate payload basename")
    seen_basenames.add(name)
    dst = os.path.join(dst_dir, name)
    shutil.copy2(src, dst)
    return dst


component_basenames = set()
component_ids = set()
staged_components = []
with open(components_path, "r", encoding="utf-8") as handle:
    for line in handle:
        line = line.rstrip("\n")
        if not line:
            continue
        parts = line.split("|")
        if len(parts) != 5:
            fail("runtime bundle assembly blocked: invalid component spec")
        component_id, kind, license_id, source, artifact_path = parts
        validate_id("component id", component_id)
        validate_component_kind(kind)
        validate_id("component license", license_id)
        validate_source(source)
        if component_id in component_ids:
            fail("runtime bundle assembly blocked: duplicate component id")
        component_ids.add(component_id)
        staged_artifact = copy_reviewed_file(
            artifact_path,
            runtime_dir,
            component_basenames,
            "component artifact",
        )
        staged_components.append(
            "|".join([component_id, kind, license_id, source, staged_artifact])
        )

if not staged_components:
    fail("runtime bundle assembly blocked: no components")

evidence_basenames = set()
staged_source_offer = copy_reviewed_file(
    source_offer,
    evidence_dir,
    evidence_basenames,
    "source-offer file",
)

staged_notices = []
with open(notices_path, "r", encoding="utf-8") as handle:
    for line in handle:
        notice_path = line.rstrip("\n")
        if not notice_path:
            continue
        staged_notices.append(
            copy_reviewed_file(
                notice_path,
                evidence_dir,
                evidence_basenames,
                "notice file",
            )
        )

for candidate in staged_components + [staged_source_offer] + staged_notices:
    for marker in ("PRIVATE-", "/Users/", "local-data", "diagnostics", "model-cache"):
        if marker in os.path.basename(candidate):
            fail("runtime bundle assembly blocked: private marker in basename")

with open(components_staged_path, "w", encoding="utf-8") as handle:
    for component in staged_components:
        handle.write(component)
        handle.write("\n")
with open(notices_staged_path, "w", encoding="utf-8") as handle:
    for notice in staged_notices:
        handle.write(notice)
        handle.write("\n")
with open(source_offer_staged_path, "w", encoding="utf-8") as handle:
    handle.write(staged_source_offer)
    handle.write("\n")
PY

staged_source_offer=$(sed -n '1p' "$source_offer_staged")

set -- "$manifest_script" \
  --version "$version" \
  --runtime-pack-id "$runtime_pack_id" \
  --distribution-license "$distribution_license" \
  --source-offer "$staged_source_offer"

while IFS= read -r notice; do
  [ -n "$notice" ] || continue
  set -- "$@" --notice "$notice"
done < "$notices_staged"

while IFS= read -r component; do
  [ -n "$component" ] || continue
  set -- "$@" --component "$component"
done < "$components_staged"

"$@" --out-dir "$out_dir" --reviewed >/dev/null

printf '%s\n' "runtime bundle payload: written"
printf '%s\n' "manifest: runtime-bundle-manifest.json"
printf '%s\n' "runtime dir: <redacted>"
printf '%s\n' "evidence dir: <redacted>"

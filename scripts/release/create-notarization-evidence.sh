#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/create-notarization-evidence.sh --version vX.Y.Z --macos-package-manifest FILE --out-dir DIR

Create a redacted blocked notarization-evidence dry-run manifest from a macOS
package manifest. This does not submit to Apple notary service, staple tickets,
or validate Gatekeeper behavior.
EOF
}

version=""
package_manifest=""
out_dir=""

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      [ $# -ge 2 ] || fail "--version requires a value"
      version="$2"
      shift 2
      ;;
    --macos-package-manifest)
      [ $# -ge 2 ] || fail "--macos-package-manifest requires a value"
      package_manifest="$2"
      shift 2
      ;;
    --out-dir)
      [ $# -ge 2 ] || fail "--out-dir requires a value"
      out_dir="$2"
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
[ -n "$package_manifest" ] || fail "--macos-package-manifest is required"
[ -n "$out_dir" ] || fail "--out-dir is required"

printf '%s\n' "$version" | grep -Eq '^v[0-9]+[.][0-9]+[.][0-9]+$' \
  || fail "version must look like vX.Y.Z"

[ -f "$package_manifest" ] || fail "macOS package manifest does not exist"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"

mkdir -p "$out_dir"
manifest="$out_dir/notarization-evidence.json"
tmp_manifest="$manifest.tmp"

python3 - "$version" "$package_manifest" "$tmp_manifest" <<'PY'
import hashlib
import json
import re
import sys
from pathlib import Path

version = sys.argv[1]
package_manifest = Path(sys.argv[2])
output = Path(sys.argv[3])


def fail(message):
    raise SystemExit(message)


if not re.fullmatch(r"v[0-9]+[.][0-9]+[.][0-9]+", version):
    fail("version must look like vX.Y.Z")

raw = package_manifest.read_bytes()
try:
    report = json.loads(raw)
except json.JSONDecodeError as error:
    fail(f"macOS package manifest is not valid JSON: {error.msg}")

if report.get("schema_version") != "release.macos_package.v1":
    fail("macOS package manifest schema_version must be release.macos_package.v1")
if report.get("version") != version:
    fail("macOS package manifest version does not match requested version")
if report.get("notarization_status") != "not_requested":
    fail("macOS package manifest must be a non-notarized dry run")

artifacts = report.get("artifacts")
if not isinstance(artifacts, list) or not artifacts:
    fail("macOS package manifest must contain artifacts")

required_kinds = {"pkg", "dmg"}
seen_kinds = set()
artifact_records = []
for artifact in artifacts:
    if not isinstance(artifact, dict):
        fail("artifact entries must be objects")
    kind = artifact.get("kind")
    file_name = artifact.get("file")
    sha256 = artifact.get("sha256")
    bytes_count = artifact.get("bytes")
    if kind not in required_kinds:
        fail("macOS package artifact kind must be pkg or dmg")
    if not isinstance(file_name, str) or "/" in file_name or "\\" in file_name:
        fail("macOS package artifact file must be a basename")
    if not isinstance(sha256, str) or not re.fullmatch(r"[0-9a-f]{64}", sha256):
        fail("macOS package artifact sha256 must be lowercase hex")
    if not isinstance(bytes_count, int) or bytes_count <= 0:
        fail("macOS package artifact bytes must be a positive integer")
    seen_kinds.add(kind)
    artifact_records.append(
        {
            "kind": kind,
            "file": file_name,
            "artifact_sha256": sha256,
            "bytes": bytes_count,
            "ticket_status": "missing",
            "staple_status": "blocked",
            "gatekeeper_status": "blocked",
        }
    )

missing = sorted(required_kinds - seen_kinds)
if missing:
    fail("macOS package manifest is missing required pkg/dmg artifacts")

document = {
    "schema_version": "release.notarization_evidence.v1",
    "version": version,
    "notarization_status": "blocked",
    "evidence_boundary": "dry_run_no_notarization_credentials",
    "macos_package_manifest_sha256": hashlib.sha256(raw).hexdigest(),
    "artifacts": artifact_records,
    "required_evidence": [
        "apple_developer_id_certificate",
        "notarytool_submission",
        "notarization_ticket",
        "stapled_ticket",
        "gatekeeper_validation",
    ],
    "blocked_release_steps": [
        "apple_developer_id_certificate",
        "notarytool_submission",
        "notarization_ticket_stapling",
        "spctl_gatekeeper_validation",
    ],
    "prohibited_public_material": [
        "notary_credentials",
        "notary_password",
        "notary_api_secret",
        "local_paths",
        "raw_resume_data",
    ],
    "notes": (
        "Blocked notarization evidence dry run only; Apple Developer ID "
        "certificate availability, notarytool submission, notarization ticket "
        "stapling, and Gatekeeper validation remain blocked until explicit "
        "release approval and notarization credentials are available."
    ),
}

output.write_text(json.dumps(document, indent=2, sort_keys=False) + "\n", encoding="utf-8")
PY

mv "$tmp_manifest" "$manifest"
printf '%s\n' "$manifest"

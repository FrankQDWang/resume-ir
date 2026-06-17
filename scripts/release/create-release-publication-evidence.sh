#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/create-release-publication-evidence.sh --version vX.Y.Z --artifact-manifest FILE --out-dir DIR

Create a redacted blocked GitHub Release publication dry-run manifest from the
release artifact manifest. This does not call GitHub, read tokens, create a
release, or upload artifacts.
EOF
}

version=""
artifact_manifest=""
out_dir=""

while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      [ $# -ge 2 ] || fail "--version requires a value"
      version="$2"
      shift 2
      ;;
    --artifact-manifest)
      [ $# -ge 2 ] || fail "--artifact-manifest requires a value"
      artifact_manifest="$2"
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
[ -n "$artifact_manifest" ] || fail "--artifact-manifest is required"
[ -n "$out_dir" ] || fail "--out-dir is required"

printf '%s\n' "$version" | grep -Eq '^v[0-9]+[.][0-9]+[.][0-9]+$' \
  || fail "version must look like vX.Y.Z"

[ -f "$artifact_manifest" ] || fail "artifact manifest does not exist"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"

mkdir -p "$out_dir"
manifest="$out_dir/release-publication-evidence.json"
tmp_manifest="$manifest.tmp"

python3 - "$version" "$artifact_manifest" "$tmp_manifest" <<'PY'
import hashlib
import json
import re
import sys
from pathlib import Path

version = sys.argv[1]
artifact_manifest = Path(sys.argv[2])
output = Path(sys.argv[3])


def fail(message):
    raise SystemExit(message)


if not re.fullmatch(r"v[0-9]+[.][0-9]+[.][0-9]+", version):
    fail("version must look like vX.Y.Z")

raw = artifact_manifest.read_bytes()
try:
    report = json.loads(raw)
except json.JSONDecodeError as error:
    fail(f"artifact manifest is not valid JSON: {error.msg}")

if report.get("schema_version") != "release.artifacts.v1":
    fail("artifact manifest schema_version must be release.artifacts.v1")
if report.get("version") != version:
    fail("artifact manifest version does not match requested version")

artifacts = report.get("artifacts")
if not isinstance(artifacts, list) or not artifacts:
    fail("artifact manifest must contain artifacts")

required_names = {"resume-cli", "resume-daemon", "resume-benchmark"}
seen_names = set()
publication_artifacts = []
for artifact in artifacts:
    if not isinstance(artifact, dict):
        fail("artifact entries must be objects")
    name = artifact.get("name")
    file_name = artifact.get("file")
    sha256 = artifact.get("sha256")
    bytes_count = artifact.get("bytes")
    if not isinstance(name, str) or name not in required_names:
        fail("artifact name is not a required release binary")
    if not isinstance(file_name, str) or "/" in file_name or "\\" in file_name:
        fail("artifact file must be a basename")
    if not isinstance(sha256, str) or not re.fullmatch(r"[0-9a-f]{64}", sha256):
        fail("artifact sha256 must be lowercase hex")
    if not isinstance(bytes_count, int) or bytes_count <= 0:
        fail("artifact bytes must be a positive integer")
    seen_names.add(name)
    publication_artifacts.append(
        {
            "name": name,
            "file": file_name,
            "artifact_sha256": sha256,
            "bytes": bytes_count,
            "upload_status": "blocked",
        }
    )

missing = sorted(required_names - seen_names)
if missing:
    fail("artifact manifest is missing required release binaries")

document = {
    "schema_version": "release.publication_evidence.v1",
    "version": version,
    "publication_status": "blocked",
    "evidence_boundary": "dry_run_no_release_publication",
    "artifact_manifest_sha256": hashlib.sha256(raw).hexdigest(),
    "artifacts": publication_artifacts,
    "required_evidence": [
        "human_release_approval",
        "github_actions_release_token",
        "github_release_upload_evidence",
    ],
    "blocked_release_steps": [
        "github_release_approval",
        "github_release_create",
        "github_release_upload",
        "release_artifact_download_verification",
    ],
    "prohibited_public_material": [
        "github_token",
        "release_pat",
        "local_paths",
        "raw_resume_data",
        "diagnostic_packages",
        "model_caches",
    ],
    "notes": (
        "Blocked GitHub Release publication dry run only; no GitHub API call, "
        "release creation, token access, or artifact upload was performed."
    ),
}

output.write_text(json.dumps(document, indent=2, sort_keys=False) + "\n", encoding="utf-8")
PY

mv "$tmp_manifest" "$manifest"
printf '%s\n' "$manifest"

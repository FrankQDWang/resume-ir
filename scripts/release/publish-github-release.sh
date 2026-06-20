#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: scripts/release/publish-github-release.sh --dry-run --version vX.Y.Z --repo OWNER/REPO --artifact-manifest FILE --publication-evidence FILE --out-dir DIR
       scripts/release/publish-github-release.sh --execute --approve-release --version vX.Y.Z --repo OWNER/REPO --artifact-manifest FILE --publication-evidence FILE --artifact-dir DIR --out-dir DIR

Fail-closed GitHub Release publication gate. Dry-run mode writes a redacted gate
manifest and does not call GitHub, read tokens, create releases, or upload
artifacts. Execute mode requires explicit approval, a GitHub token in
GITHUB_TOKEN or GH_TOKEN, gh, and a local artifact directory.
EOF
}

mode=""
approved="false"
version=""
repo=""
artifact_manifest=""
publication_evidence=""
artifact_dir=""
out_dir=""

while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run)
      [ -z "$mode" ] || fail "choose only one of --dry-run or --execute"
      mode="dry_run"
      shift
      ;;
    --execute)
      [ -z "$mode" ] || fail "choose only one of --dry-run or --execute"
      mode="execute"
      shift
      ;;
    --approve-release)
      approved="true"
      shift
      ;;
    --version)
      [ $# -ge 2 ] || fail "--version requires a value"
      version="$2"
      shift 2
      ;;
    --repo)
      [ $# -ge 2 ] || fail "--repo requires a value"
      repo="$2"
      shift 2
      ;;
    --artifact-manifest)
      [ $# -ge 2 ] || fail "--artifact-manifest requires a value"
      artifact_manifest="$2"
      shift 2
      ;;
    --publication-evidence)
      [ $# -ge 2 ] || fail "--publication-evidence requires a value"
      publication_evidence="$2"
      shift 2
      ;;
    --artifact-dir)
      [ $# -ge 2 ] || fail "--artifact-dir requires a value"
      artifact_dir="$2"
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

[ -n "$mode" ] || fail "one of --dry-run or --execute is required"
[ -n "$version" ] || fail "--version is required"
[ -n "$repo" ] || fail "--repo is required"
[ -n "$artifact_manifest" ] || fail "--artifact-manifest is required"
[ -n "$publication_evidence" ] || fail "--publication-evidence is required"
[ -n "$out_dir" ] || fail "--out-dir is required"

printf '%s\n' "$version" | grep -Eq '^v[0-9]+[.][0-9]+[.][0-9]+$' \
  || fail "version must look like vX.Y.Z"
printf '%s\n' "$repo" | grep -Eq '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' \
  || fail "repo must look like OWNER/REPO"
[ -f "$artifact_manifest" ] || fail "artifact manifest does not exist"
[ -f "$publication_evidence" ] || fail "publication evidence manifest does not exist"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"

mkdir -p "$out_dir"
gate="$out_dir/github-release-publication-gate.json"
tmp_gate="$gate.tmp"

python3 - "$mode" "$version" "$repo" "$artifact_manifest" "$publication_evidence" "$tmp_gate" <<'PY'
import hashlib
import json
import re
import sys
from pathlib import Path

mode, version, repo = sys.argv[1], sys.argv[2], sys.argv[3]
artifact_manifest = Path(sys.argv[4])
publication_evidence = Path(sys.argv[5])
output = Path(sys.argv[6])


def fail(message):
    raise SystemExit(message)


def require_allowed_keys(mapping, allowed, context):
    unexpected = sorted(set(mapping) - set(allowed))
    if unexpected:
        fail(f"{context} contains unsupported field")


def is_basename(value):
    return (
        isinstance(value, str)
        and value not in {"", ".", ".."}
        and "/" not in value
        and "\\" not in value
        and ":" not in value
    )


def require_sha256(value, message):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        fail(message)


def require_positive_int(value, message):
    if not isinstance(value, int) or value <= 0:
        fail(message)


if mode not in {"dry_run", "execute"}:
    fail("mode is invalid")
if not re.fullmatch(r"v[0-9]+[.][0-9]+[.][0-9]+", version):
    fail("version must look like vX.Y.Z")
if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
    fail("repo must look like OWNER/REPO")

artifact_raw = artifact_manifest.read_bytes()
publication_raw = publication_evidence.read_bytes()
try:
    artifacts_report = json.loads(artifact_raw)
    publication_report = json.loads(publication_raw)
except json.JSONDecodeError as error:
    fail(f"release publication input is not valid JSON: {error.msg}")

if not isinstance(artifacts_report, dict):
    fail("artifact manifest root must be an object")
require_allowed_keys(
    artifacts_report,
    {
        "schema_version",
        "version",
        "packaging_status",
        "artifacts",
        "runtime_bundle_manifests",
        "blocked_release_steps",
        "notes",
    },
    "artifact manifest root",
)
if artifacts_report.get("schema_version") != "release.artifacts.v1":
    fail("artifact manifest schema_version must be release.artifacts.v1")
if artifacts_report.get("version") != version:
    fail("artifact manifest version does not match requested version")
if artifacts_report.get("packaging_status") != "blocked":
    fail("artifact manifest packaging_status must be blocked")

runtime_bundle_manifests = artifacts_report.get("runtime_bundle_manifests")
if runtime_bundle_manifests is not None:
    if not isinstance(runtime_bundle_manifests, list) or not runtime_bundle_manifests:
        fail("artifact manifest runtime bundle manifests are invalid")
    for runtime_bundle in runtime_bundle_manifests:
        if not isinstance(runtime_bundle, dict):
            fail("artifact manifest runtime bundle entries must be objects")
        require_allowed_keys(
            runtime_bundle,
            {
                "file",
                "sha256",
                "bytes",
                "schema_version",
                "runtime_distribution_mode",
                "runtime_package_binaries_included",
                "runtime_binaries_included",
            },
            "artifact manifest runtime bundle",
        )
        if not is_basename(runtime_bundle.get("file")):
            fail("runtime bundle manifest file must be a basename")
        require_sha256(
            runtime_bundle.get("sha256"),
            "runtime bundle manifest sha256 must be lowercase hex",
        )
        require_positive_int(
            runtime_bundle.get("bytes"),
            "runtime bundle manifest bytes must be a positive integer",
        )
        if runtime_bundle.get("schema_version") != "release.runtime_bundle.v1":
            fail("runtime bundle manifest schema_version must be release.runtime_bundle.v1")
        if runtime_bundle.get("runtime_distribution_mode") != "bundled":
            fail("runtime bundle manifest distribution mode must be bundled")
        if runtime_bundle.get("runtime_package_binaries_included") is not True:
            fail("runtime bundle manifest must include package binaries")
        if runtime_bundle.get("runtime_binaries_included") is not False:
            fail("runtime bundle manifest must not include raw runtime binaries")

if not isinstance(publication_report, dict):
    fail("publication evidence root must be an object")
require_allowed_keys(
    publication_report,
    {
        "schema_version",
        "version",
        "publication_status",
        "evidence_boundary",
        "artifact_manifest_sha256",
        "artifacts",
        "required_evidence",
        "blocked_release_steps",
        "prohibited_public_material",
        "notes",
    },
    "publication evidence root",
)
if publication_report.get("schema_version") != "release.publication_evidence.v1":
    fail("publication evidence schema_version must be release.publication_evidence.v1")
if publication_report.get("version") != version:
    fail("publication evidence version does not match requested version")
if publication_report.get("publication_status") != "blocked":
    fail("publication evidence must be blocked")
if publication_report.get("evidence_boundary") != "dry_run_no_release_publication":
    fail("publication evidence boundary is invalid")
if publication_report.get("artifact_manifest_sha256") != hashlib.sha256(artifact_raw).hexdigest():
    fail("publication evidence artifact manifest digest does not match")

required_names = {"resume-cli", "resume-daemon", "resume-benchmark"}
publication_artifacts = publication_report.get("artifacts")
if not isinstance(publication_artifacts, list) or not publication_artifacts:
    fail("publication evidence must contain artifacts")
seen_publication = set()
for artifact in publication_artifacts:
    if not isinstance(artifact, dict):
        fail("publication evidence artifact entries must be objects")
    require_allowed_keys(
        artifact,
        {"name", "file", "artifact_sha256", "bytes", "upload_status"},
        "publication evidence artifact",
    )
    name = artifact.get("name")
    file_name = artifact.get("file")
    sha256 = artifact.get("artifact_sha256")
    bytes_count = artifact.get("bytes")
    if name not in required_names:
        fail("publication evidence artifact name is not a required release binary")
    if not is_basename(file_name):
        fail("publication evidence artifact file must be a basename")
    require_sha256(sha256, "publication evidence artifact sha256 must be lowercase hex")
    require_positive_int(
        bytes_count,
        "publication evidence artifact bytes must be a positive integer",
    )
    if artifact.get("upload_status") != "blocked":
        fail("publication evidence artifact upload_status must be blocked")
    seen_publication.add(name)
if sorted(required_names - seen_publication):
    fail("publication evidence is missing required release binaries")

source_artifacts = artifacts_report.get("artifacts")
if not isinstance(source_artifacts, list) or not source_artifacts:
    fail("artifact manifest must contain artifacts")
artifacts = []
for artifact in source_artifacts:
    if not isinstance(artifact, dict):
        fail("artifact entries must be objects")
    require_allowed_keys(artifact, {"name", "file", "sha256", "bytes"}, "artifact manifest artifact")
    name = artifact.get("name")
    file_name = artifact.get("file")
    sha256 = artifact.get("sha256")
    bytes_count = artifact.get("bytes")
    if name not in required_names:
        fail("artifact name is not a required release binary")
    if not is_basename(file_name):
        fail("artifact file must be a basename")
    require_sha256(sha256, "artifact sha256 must be lowercase hex")
    require_positive_int(bytes_count, "artifact bytes must be a positive integer")
    artifacts.append(
        {
            "name": name,
            "file": file_name,
            "artifact_sha256": sha256,
            "bytes": bytes_count,
            "publish_status": "blocked" if mode == "dry_run" else "pending_execute",
        }
    )

seen = {artifact["name"] for artifact in artifacts}
missing = sorted(required_names - seen)
if missing:
    fail("artifact manifest is missing required release binaries")

document = {
    "schema_version": "release.github_publication_gate.v1",
    "version": version,
    "repo": repo,
    "execution_mode": mode,
    "publication_status": "blocked" if mode == "dry_run" else "ready_for_execute",
    "approval_gate": "human_release_approval_required",
    "secret_interface": "GITHUB_TOKEN_or_GH_TOKEN_required_for_execute",
    "artifact_manifest_sha256": hashlib.sha256(artifact_raw).hexdigest(),
    "publication_evidence_sha256": hashlib.sha256(publication_raw).hexdigest(),
    "planned_steps": [
        "validate_release_artifact_manifest",
        "validate_publication_evidence_manifest",
        "gh_release_create",
        "gh_release_upload",
        "gh_release_download_verify",
    ],
    "artifacts": artifacts,
    "prohibited_public_material": [
        "github_token",
        "release_pat",
        "local_paths",
        "raw_resume_data",
        "diagnostic_packages",
        "model_caches",
    ],
    "notes": (
        "Dry-run mode does not call GitHub, read tokens, create releases, or upload "
        "artifacts. Execute mode is fail-closed behind explicit approval and token checks."
    ),
}

output.write_text(json.dumps(document, indent=2, sort_keys=False) + "\n", encoding="utf-8")
PY

mv "$tmp_gate" "$gate"

if [ "$mode" = "dry_run" ]; then
  printf '%s\n' "$gate"
  exit 0
fi

[ "$approved" = "true" ] || fail "execute mode requires --approve-release"
[ -n "$artifact_dir" ] || fail "execute mode requires --artifact-dir"
[ -d "$artifact_dir" ] || fail "artifact directory does not exist"
if [ -z "${GITHUB_TOKEN:-}" ] && [ -z "${GH_TOKEN:-}" ]; then
  fail "execute mode requires GITHUB_TOKEN or GH_TOKEN"
fi
command -v gh >/dev/null 2>&1 || fail "execute mode requires gh"

python3 - "$artifact_manifest" "$artifact_dir" <<'PY'
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
artifact_dir = Path(sys.argv[2])
for artifact in manifest["artifacts"]:
    file_name = artifact["file"]
    path = artifact_dir / file_name
    if not path.is_file():
        raise SystemExit(f"release artifact is missing: {file_name}")
PY

if ! gh release view "$version" --repo "$repo" >/dev/null 2>&1; then
  gh release create "$version" --repo "$repo" --title "$version" --notes "resume-ir $version"
fi

python3 - "$artifact_manifest" "$artifact_dir" <<'PY' | while IFS= read -r artifact_path; do
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
artifact_dir = Path(sys.argv[2])
for artifact in manifest["artifacts"]:
    print(str(artifact_dir / artifact["file"]))
PY
  gh release upload "$version" "$artifact_path" --repo "$repo" --clobber
done

printf '%s\n' "$gate"

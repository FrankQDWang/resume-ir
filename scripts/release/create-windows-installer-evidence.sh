#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage: scripts/release/create-windows-installer-evidence.sh --version vX.Y.Z --windows-package-manifest FILE --out-dir DIR

Create a redacted blocked Windows installer lifecycle evidence dry-run manifest
from a Windows MSI package dry-run manifest. This does not install, upgrade,
repair, uninstall, roll back, or execute an MSI package.
EOF
}

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

version=""
package_manifest=""
out_dir=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -ge 2 ] || fail "missing value for --version"
      version="$2"
      shift 2
      ;;
    --windows-package-manifest)
      [ "$#" -ge 2 ] || fail "missing value for --windows-package-manifest"
      package_manifest="$2"
      shift 2
      ;;
    --out-dir)
      [ "$#" -ge 2 ] || fail "missing value for --out-dir"
      out_dir="$2"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage
      fail "unknown argument: $1"
      ;;
  esac
done

[ -n "$version" ] || fail "missing --version"
[ -n "$package_manifest" ] || fail "missing --windows-package-manifest"
[ -n "$out_dir" ] || fail "missing --out-dir"

case "$version" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) fail "version must look like vX.Y.Z" ;;
esac

[ -f "$package_manifest" ] || fail "Windows package manifest does not exist"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"

mkdir -p "$out_dir"
manifest="$out_dir/windows-installer-evidence.json"

python3 - "$version" "$package_manifest" "$manifest" <<'PY'
import hashlib
import json
import os
import re
import sys

version, package_manifest_path, output_path = sys.argv[1:4]


def fail(message):
    print(message, file=sys.stderr)
    sys.exit(1)


def basename_only(value, label):
    if not isinstance(value, str) or not value:
        fail(f"{label} must be a non-empty string")
    if os.path.basename(value) != value:
        fail(f"{label} must be a basename")
    return value


with open(package_manifest_path, "rb") as handle:
    package_bytes = handle.read()

try:
    report = json.loads(package_bytes.decode("utf-8"))
except UnicodeDecodeError:
    fail("Windows package manifest must be UTF-8 JSON")
except json.JSONDecodeError as error:
    fail(f"Windows package manifest is not valid JSON: {error.msg}")

if report.get("schema_version") != "release.windows_package.v1":
    fail("Windows package manifest schema_version must be release.windows_package.v1")
if report.get("version") != version:
    fail("Windows package manifest version does not match requested version")
if report.get("packaging_status") != "unsigned_dry_run":
    fail("Windows package manifest must be an unsigned dry run")
if report.get("installer_kind") != "msi":
    fail("Windows package manifest installer_kind must be msi")
if report.get("signing_status") != "unsigned":
    fail("Windows package manifest signing_status must be unsigned")

blocked_steps = report.get("blocked_release_steps")
if not isinstance(blocked_steps, list) or "installer_lifecycle_validation" not in blocked_steps:
    fail("Windows package manifest must keep installer_lifecycle_validation blocked")

artifacts = report.get("artifacts")
if not isinstance(artifacts, list) or not artifacts:
    fail("Windows package manifest must contain artifacts")

msi_records = []
for artifact in artifacts:
    if not isinstance(artifact, dict):
        fail("Windows package artifact must be an object")
    kind = artifact.get("kind")
    file_name = basename_only(artifact.get("file"), "Windows package artifact file")
    sha256 = artifact.get("sha256")
    byte_count = artifact.get("bytes")
    if kind != "msi":
        continue
    if not isinstance(sha256, str) or not re.fullmatch(r"[0-9a-f]{64}", sha256):
        fail("Windows package artifact sha256 must be lowercase hex")
    if not isinstance(byte_count, int) or byte_count <= 0:
        fail("Windows package artifact bytes must be a positive integer")
    msi_records.append(
        {
            "kind": "msi",
            "file": file_name,
            "artifact_sha256": sha256,
            "bytes": byte_count,
            "installer_validation_status": "not_executed",
        }
    )

if not msi_records:
    fail("Windows package manifest is missing required MSI artifact")

planned_actions = [
    ("install", "administrator-elevated msiexec install transcript and post-install binary checks"),
    ("upgrade", "prior-version install plus upgrade transcript and version replacement evidence"),
    ("repair", "msiexec repair transcript and installed-file integrity evidence"),
    ("uninstall", "msiexec uninstall transcript and user-data preservation evidence"),
    ("rollback", "forced-failure rollback transcript and system state restoration evidence"),
]

document = {
    "schema_version": "release.windows_installer_evidence.v1",
    "version": version,
    "installer_lifecycle_status": "blocked",
    "evidence_boundary": "dry_run_no_windows_installer_execution",
    "windows_package_manifest_sha256": hashlib.sha256(package_bytes).hexdigest(),
    "installer_engine": "msiexec.exe",
    "admin_elevation": "required_not_observed",
    "installation_status": "not_installed",
    "rollback_validation_status": "blocked",
    "installer_artifacts": msi_records,
    "planned_actions": [
        {
            "action": action,
            "action_status": "blocked",
            "required_evidence": required_evidence,
        }
        for action, required_evidence in planned_actions
    ],
    "required_evidence": [
        "administrator-elevated install transcript",
        "installer_lifecycle_validation",
        "upgrade_validation",
        "repair_validation",
        "uninstall_validation",
        "rollback_validation",
        "post_install_binary_version_checks",
        "user_data_preservation_checks",
    ],
    "blocked_release_steps": [
        "windows_msi_install",
        "windows_msi_upgrade",
        "windows_msi_repair",
        "windows_msi_uninstall",
        "windows_msi_rollback",
    ],
    "prohibited_public_material": [
        "installer_tokens",
        "administrator_passwords",
        "local_paths",
        "raw_installer_logs",
        "raw_resume_data",
        "diagnostic_packages",
        "model_artifact_caches",
    ],
    "notes": (
        "Blocked Windows installer lifecycle evidence dry run only; MSI "
        "install, upgrade, repair, uninstall, rollback, and administrator-"
        "elevated validation remain blocked until proven on a release Windows "
        "runner with explicit release approval."
    ),
}

with open(output_path, "w", encoding="utf-8") as handle:
    json.dump(document, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
print(output_path)
PY

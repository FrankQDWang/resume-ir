#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage: scripts/release/create-windows-service-evidence.sh --version vX.Y.Z --windows-package-manifest FILE --out-dir DIR

Create a redacted blocked Windows service lifecycle evidence dry-run manifest
from a Windows MSI package dry-run manifest. This does not register, start,
stop, query, recover, uninstall, or roll back a Windows service.
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
manifest="$out_dir/windows-service-evidence.json"

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
if not isinstance(blocked_steps, list) or "service_install_validation" not in blocked_steps:
    fail("Windows package manifest must keep service_install_validation blocked")

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
            "service_validation_status": "not_executed",
        }
    )

if not msi_records:
    fail("Windows package manifest is missing required MSI artifact")

planned_actions = [
    (
        "install",
        "administrator-elevated install log, service name, binary binding, and user-data preservation evidence",
    ),
    ("start", "service start event and daemon health evidence"),
    ("status", "service status query evidence from a release Windows runner"),
    ("stop", "service stop event and daemon shutdown evidence"),
    ("uninstall", "service uninstall log and user-data preservation evidence"),
    ("recovery", "service recovery policy and restart-after-kill evidence"),
]

document = {
    "schema_version": "release.windows_service_evidence.v1",
    "version": version,
    "service_lifecycle_status": "blocked",
    "evidence_boundary": "dry_run_no_windows_service_registration",
    "windows_package_manifest_sha256": hashlib.sha256(package_bytes).hexdigest(),
    "service_manager": "sc.exe",
    "admin_elevation": "required_not_observed",
    "registration_status": "not_registered",
    "recovery_validation_status": "blocked",
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
        "service_install_validation",
        "service_start_validation",
        "service_status_validation",
        "service_stop_validation",
        "service_uninstall_validation",
        "service_recovery_validation",
        "rollback_or_upgrade_validation",
    ],
    "blocked_release_steps": [
        "windows_service_install",
        "windows_service_start",
        "windows_service_status",
        "windows_service_stop",
        "windows_service_uninstall",
        "windows_service_recovery",
        "windows_service_rollback",
    ],
    "prohibited_public_material": [
        "service_tokens",
        "administrator_passwords",
        "local_paths",
        "raw_service_logs",
        "raw_resume_data",
        "diagnostic_packages",
        "model_artifact_caches",
    ],
    "notes": (
        "Blocked Windows service lifecycle evidence dry run only; service "
        "registration, start/stop/status, recovery, uninstall, rollback, and "
        "administrator-elevated validation remain blocked until proven on a "
        "release Windows runner with explicit release approval."
    ),
}

with open(output_path, "w", encoding="utf-8") as handle:
    json.dump(document, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
print(output_path)
PY

#!/usr/bin/env sh
set -eu

usage() {
  cat >&2 <<'EOF'
usage: scripts/release/create-macos-installer-evidence.sh --version vX.Y.Z --macos-package-manifest FILE --out-dir DIR

Create a redacted blocked macOS installer lifecycle evidence dry-run manifest
from an unsigned macOS pkg/dmg package manifest. This does not install,
upgrade, uninstall, roll back, mount a DMG, or start/stop a LaunchAgent.
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
    --macos-package-manifest)
      [ "$#" -ge 2 ] || fail "missing value for --macos-package-manifest"
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
[ -n "$package_manifest" ] || fail "missing --macos-package-manifest"
[ -n "$out_dir" ] || fail "missing --out-dir"

printf '%s\n' "$version" | grep -Eq '^v[0-9]+[.][0-9]+[.][0-9]+$' \
  || fail "version must look like vX.Y.Z"

[ -f "$package_manifest" ] || fail "macOS package manifest does not exist"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"

mkdir -p "$out_dir"
manifest="$out_dir/macos-installer-evidence.json"
tmp_manifest="$manifest.tmp"

python3 - "$version" "$package_manifest" "$tmp_manifest" <<'PY'
import hashlib
import json
import os
import re
import sys

version, package_manifest_path, output_path = sys.argv[1:4]


def fail(message):
    print(message, file=sys.stderr)
    sys.exit(1)


def require_allowed_keys(mapping, allowed, context):
    unexpected = sorted(set(mapping) - set(allowed))
    if unexpected:
        fail(f"macOS package manifest contains unsupported {context} field")


def is_basename(value):
    return (
        isinstance(value, str)
        and value not in {"", ".", ".."}
        and "/" not in value
        and "\\" not in value
        and ":" not in value
    )


def basename_only(value, label):
    if not is_basename(value):
        fail(f"{label} must be a basename")
    return value


def require_string(mapping, key, context):
    value = mapping.get(key)
    if not isinstance(value, str) or not value:
        fail(f"macOS package manifest invalid: {context}.{key}")
    return value


def require_exact_string(mapping, key, expected, context):
    if mapping.get(key) != expected:
        fail(f"macOS package manifest invalid: {context}.{key}")


def require_bool(mapping, key, expected, context):
    if mapping.get(key) is not expected:
        fail(f"macOS package manifest invalid: {context}.{key}")


def require_sha256(value, message):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        fail(message)


def require_positive_int(value, message):
    if not isinstance(value, int) or value <= 0:
        fail(message)


def validate_runtime_payload(report):
    payload = report.get("runtime_payload")
    if payload is None:
        return
    if not isinstance(payload, dict):
        fail("macOS package manifest runtime payload must be an object")
    require_allowed_keys(
        payload,
        {
            "schema_version",
            "runtime_distribution_mode",
            "runtime_package_binaries_included",
            "runtime_binaries_included_in_manifest",
            "install_location",
            "runtime_bundle_manifest",
            "components",
        },
        "runtime payload",
    )
    require_exact_string(
        payload,
        "schema_version",
        "release.runtime_package_payload.v1",
        "runtime_payload",
    )
    require_exact_string(payload, "runtime_distribution_mode", "bundled", "runtime_payload")
    require_bool(payload, "runtime_package_binaries_included", True, "runtime_payload")
    require_bool(payload, "runtime_binaries_included_in_manifest", False, "runtime_payload")
    require_exact_string(
        payload,
        "install_location",
        "/usr/local/lib/resume-ir/runtime",
        "runtime_payload",
    )

    bundle_manifest = payload.get("runtime_bundle_manifest")
    if not isinstance(bundle_manifest, dict):
        fail("macOS package manifest runtime bundle manifest must be an object")
    require_allowed_keys(
        bundle_manifest,
        {"file", "sha256", "bytes", "schema_version", "runtime_distribution_mode"},
        "runtime bundle manifest",
    )
    if not is_basename(bundle_manifest.get("file")):
        fail("runtime bundle manifest file must be a basename")
    require_sha256(
        bundle_manifest.get("sha256"),
        "runtime bundle manifest sha256 must be lowercase hex",
    )
    require_positive_int(
        bundle_manifest.get("bytes"),
        "runtime bundle manifest bytes must be a positive integer",
    )
    require_exact_string(
        bundle_manifest,
        "schema_version",
        "release.runtime_bundle.v1",
        "runtime_bundle_manifest",
    )
    require_exact_string(
        bundle_manifest,
        "runtime_distribution_mode",
        "bundled",
        "runtime_bundle_manifest",
    )

    components = payload.get("components")
    if not isinstance(components, list) or not components:
        fail("macOS package manifest runtime payload must contain components")
    seen_files = set()
    ocr_component_kinds = set()
    for component in components:
        if not isinstance(component, dict):
            fail("macOS package manifest runtime payload components must be objects")
        require_allowed_keys(
            component,
            {"id", "kind", "file", "sha256", "bytes", "license", "source"},
            "runtime component",
        )
        require_string(component, "id", "runtime_component")
        kind = require_string(component, "kind", "runtime_component")
        if kind in {"ocr-engine", "pdf-renderer", "ocr-language-pack"}:
            ocr_component_kinds.add(kind)
        file_name = component.get("file")
        if not is_basename(file_name) or file_name in seen_files:
            fail("runtime component file must be a unique basename")
        seen_files.add(file_name)
        require_sha256(component.get("sha256"), "runtime component sha256 must be lowercase hex")
        require_positive_int(component.get("bytes"), "runtime component bytes must be positive")
        require_string(component, "license", "runtime_component")
        source = require_string(component, "source", "runtime_component")
        if source.startswith("/") or "PRIVATE-" in source or "/Users/" in source:
            fail("runtime component source must not contain local/private markers")
    if ocr_component_kinds and not {
        "ocr-engine",
        "pdf-renderer",
        "ocr-language-pack",
    }.issubset(ocr_component_kinds):
        fail("macOS package manifest runtime payload has partial OCR runtime components")


with open(package_manifest_path, "rb") as handle:
    package_bytes = handle.read()

try:
    report = json.loads(package_bytes.decode("utf-8"))
except UnicodeDecodeError:
    fail("macOS package manifest must be UTF-8 JSON")
except json.JSONDecodeError as error:
    fail(f"macOS package manifest is not valid JSON: {error.msg}")

if not isinstance(report, dict):
    fail("macOS package manifest root must be an object")
require_allowed_keys(
    report,
    {
        "schema_version",
        "version",
        "packaging_status",
        "install_location",
        "signing_status",
        "notarization_status",
        "runtime_payload",
        "artifacts",
        "blocked_release_steps",
        "notes",
    },
    "root",
)
if report.get("schema_version") != "release.macos_package.v1":
    fail("macOS package manifest schema_version must be release.macos_package.v1")
if report.get("version") != version:
    fail("macOS package manifest version does not match requested version")
if report.get("packaging_status") != "unsigned_dry_run":
    fail("macOS package manifest must be an unsigned dry run")
if report.get("install_location") != "/usr/local/bin":
    fail("macOS package manifest install_location must be /usr/local/bin")
if report.get("signing_status") != "unsigned":
    fail("macOS package manifest signing_status must be unsigned")
if report.get("notarization_status") != "not_requested":
    fail("macOS package manifest notarization_status must be not_requested")

blocked_steps = report.get("blocked_release_steps")
if not isinstance(blocked_steps, list):
    fail("macOS package manifest blocked_release_steps must be an array")
for step in [
    "signing",
    "notarization",
    "github_release_upload",
    "installer_lifecycle_validation",
]:
    if step not in blocked_steps:
        fail("macOS package manifest is missing required blocked release step")

validate_runtime_payload(report)

artifacts = report.get("artifacts")
if not isinstance(artifacts, list) or not artifacts:
    fail("macOS package manifest must contain artifacts")

required_kinds = {"pkg", "dmg"}
seen_kinds = set()
artifact_records = []
for artifact in artifacts:
    if not isinstance(artifact, dict):
        fail("macOS package artifact must be an object")
    require_allowed_keys(artifact, {"kind", "file", "sha256", "bytes"}, "artifact")
    kind = artifact.get("kind")
    if kind not in required_kinds:
        fail("macOS package artifact kind must be pkg or dmg")
    file_name = basename_only(artifact.get("file"), "macOS package artifact file")
    sha256 = artifact.get("sha256")
    byte_count = artifact.get("bytes")
    require_sha256(sha256, "macOS package artifact sha256 must be lowercase hex")
    require_positive_int(byte_count, "macOS package artifact bytes must be a positive integer")
    seen_kinds.add(kind)
    artifact_records.append(
        {
            "kind": kind,
            "file": file_name,
            "artifact_sha256": sha256,
            "bytes": byte_count,
            "installer_validation_status": "not_executed",
        }
    )

missing = sorted(required_kinds - seen_kinds)
if missing:
    fail("macOS package manifest is missing required pkg/dmg artifacts")

planned_actions = [
    ("install", "administrator-elevated installer transcript and post-install binary checks"),
    ("upgrade", "prior-version install plus upgrade transcript and version replacement evidence"),
    ("uninstall", "installer uninstall transcript and user-data preservation evidence"),
    ("rollback", "forced-failure rollback transcript and system state restoration evidence"),
    ("launch-agent-start", "launchctl bootstrap/start transcript and daemon IPC health evidence"),
    ("launch-agent-stop", "launchctl stop/bootout transcript and daemon shutdown evidence"),
]

document = {
    "schema_version": "release.macos_installer_evidence.v1",
    "version": version,
    "installer_lifecycle_status": "blocked",
    "evidence_boundary": "dry_run_no_macos_installer_execution",
    "macos_package_manifest_sha256": hashlib.sha256(package_bytes).hexdigest(),
    "installer_tool": "installer",
    "installer_supporting_tools": ["pkgutil", "hdiutil", "launchctl"],
    "admin_elevation": "required_not_observed",
    "installation_status": "not_installed",
    "rollback_validation_status": "blocked",
    "launch_agent_validation_status": "blocked",
    "installer_artifacts": artifact_records,
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
        "uninstall_validation",
        "rollback_validation",
        "launch_agent_start_validation",
        "launch_agent_stop_validation",
        "post_install_binary_version_checks",
        "user_data_preservation_checks",
    ],
    "blocked_release_steps": [
        "macos_pkg_install",
        "macos_pkg_upgrade",
        "macos_pkg_uninstall",
        "macos_pkg_rollback",
        "macos_launch_agent_start",
        "macos_launch_agent_stop",
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
        "Blocked macOS installer lifecycle evidence dry run only; pkg/dmg "
        "install, upgrade, uninstall, rollback, LaunchAgent start/stop, and "
        "administrator-elevated validation remain blocked until proven on a "
        "release macOS runner with explicit release approval."
    ),
}

with open(output_path, "w", encoding="utf-8") as handle:
    json.dump(document, handle, ensure_ascii=False, indent=2)
    handle.write("\n")
print(output_path)
PY

mv "$tmp_manifest" "$manifest"
printf '%s\n' "$manifest"

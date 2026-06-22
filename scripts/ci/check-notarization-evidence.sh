#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required notarization evidence file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

reject_text() {
  file="$1"
  text="$2"
  if grep -Fq -- "$text" "$file"; then
    fail "$file leaked notarization evidence marker: $text"
  fi
}

notarization_script="scripts/release/create-notarization-evidence.sh"
verify_script="scripts/ci/verify-local.sh"
workflow_guard="scripts/ci/check-workflows.sh"
release_workflow=".github/workflows/release.yml"
runbook="docs/runbooks/release-blockers.md"

for file in "$notarization_script" "$verify_script" "$workflow_guard" "$release_workflow" "$runbook"; do
  require_file "$file"
done

if [ ! -x "$notarization_script" ]; then
  fail "notarization evidence script is not executable"
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-notarization-evidence-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

out_dir="$tmpdir/out"
package_manifest="$tmpdir/macos-package.json"
mkdir -p "$out_dir"
cat > "$package_manifest" <<'EOF'
{
  "schema_version": "release.macos_package.v1",
  "version": "v0.0.0",
  "packaging_status": "unsigned_dry_run",
  "install_location": "/usr/local/bin",
  "signing_status": "unsigned",
  "notarization_status": "not_requested",
  "runtime_payload": {
    "schema_version": "release.runtime_package_payload.v1",
    "runtime_distribution_mode": "bundled",
    "runtime_package_binaries_included": true,
    "runtime_binaries_included_in_manifest": false,
    "install_location": "/usr/local/lib/resume-ir/runtime",
    "runtime_bundle_manifest": {
      "file": "runtime-bundle-manifest.json",
      "sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
      "bytes": 789,
      "schema_version": "release.runtime_bundle.v1",
      "runtime_distribution_mode": "bundled"
    },
    "components": [
      {
        "id": "synthetic-tesseract",
        "kind": "ocr-engine",
        "file": "tesseract",
        "sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "bytes": 101,
        "license": "Apache-2.0",
        "source": "synthetic-reviewed-source"
      },
      {
        "id": "synthetic-pdftoppm",
        "kind": "pdf-renderer",
        "file": "pdftoppm",
        "sha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        "bytes": 202,
        "license": "GPL-compatible-reviewed",
        "source": "synthetic-reviewed-source"
      },
      {
        "id": "synthetic-tessdata-eng",
        "kind": "ocr-language-pack",
        "file": "eng.traineddata",
        "sha256": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "bytes": 303,
        "license": "Apache-2.0",
        "source": "synthetic-reviewed-source"
      }
    ]
  },
  "artifacts": [
    {
      "kind": "pkg",
      "file": "resume-ir-v0.0.0-macos.pkg",
      "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "bytes": 123
    },
    {
      "kind": "dmg",
      "file": "resume-ir-v0.0.0-macos.dmg",
      "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "bytes": 456
    }
  ],
  "blocked_release_steps": [
    "signing",
    "notarization",
    "github_release_upload",
    "installer_lifecycle_validation",
    "windows_msi"
  ],
  "notes": "Synthetic unsigned local macOS package dry run only."
}
EOF

"$notarization_script" \
  --version v0.0.0 \
  --macos-package-manifest "$package_manifest" \
  --out-dir "$out_dir" >/dev/null

manifest="$out_dir/notarization-evidence.json"
require_file "$manifest"
require_text "$manifest" '"schema_version": "release.notarization_evidence.v1"'
require_text "$manifest" '"version": "v0.0.0"'
require_text "$manifest" '"notarization_status": "blocked"'
require_text "$manifest" '"evidence_boundary": "dry_run_no_notarization_credentials"'
require_text "$manifest" '"macos_package_manifest_sha256": "'
require_text "$manifest" '"apple_developer_id_certificate"'
require_text "$manifest" '"notarytool_submission"'
require_text "$manifest" '"notarization_ticket"'
require_text "$manifest" '"stapled_ticket"'
require_text "$manifest" '"gatekeeper_validation"'
require_text "$manifest" '"kind": "pkg"'
require_text "$manifest" '"kind": "dmg"'
require_text "$manifest" '"ticket_status": "missing"'
require_text "$manifest" '"staple_status": "blocked"'
require_text "$manifest" '"gatekeeper_status": "blocked"'

reject_text "$manifest" "$tmpdir"
reject_text "$manifest" "/Users/"
reject_text "$manifest" "target/release"
reject_text "$manifest" "local-data"
reject_text "$manifest" "diagnostics"
reject_text "$manifest" "model-cache"
reject_text "$manifest" "notary-password"
reject_text "$manifest" "notary-api-secret"

if "$notarization_script" --version 0.0.0 --macos-package-manifest "$package_manifest" --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "notarization evidence script accepted an invalid version"
fi

if "$notarization_script" --version v0.0.1 --macos-package-manifest "$package_manifest" --out-dir "$out_dir/mismatch" >/dev/null 2>&1; then
  fail "notarization evidence script accepted a mismatched macOS package manifest version"
fi

unknown_manifest="$tmpdir/macos-package-unknown-field.json"
python3 - "$package_manifest" "$unknown_manifest" <<'PY'
import json
import sys

source = sys.argv[1]
target = sys.argv[2]

with open(source, "r", encoding="utf-8") as handle:
    document = json.load(handle)

document["artifacts"][0]["local_probe_path"] = "PRIVATE-notary-cache"

with open(target, "w", encoding="utf-8") as handle:
    json.dump(document, handle)
    handle.write("\n")
PY

if "$notarization_script" --version v0.0.0 --macos-package-manifest "$unknown_manifest" --out-dir "$out_dir/unknown" >/dev/null 2>&1; then
  fail "notarization evidence script accepted an unknown macOS package manifest field"
fi

require_text "$verify_script" "./scripts/ci/check-notarization-evidence.sh"
require_text "$workflow_guard" "check-notarization-evidence.sh"
require_text "$release_workflow" "scripts/release/create-notarization-evidence.sh"
require_text "$release_workflow" "notarization-evidence.json"
require_text "$runbook" "create-notarization-evidence.sh"
require_text "$runbook" "release.notarization_evidence.v1"

printf '%s\n' "notarization evidence check passed"

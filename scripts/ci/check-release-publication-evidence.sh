#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required release publication file: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "$file is missing required text: $text"
  fi
}

script="scripts/release/create-release-publication-evidence.sh"
publish_script="scripts/release/publish-github-release.sh"
workflow=".github/workflows/release.yml"
verify_script="scripts/ci/verify-local.sh"

require_file "$script"
require_file "$publish_script"
require_file "$workflow"
require_file "$verify_script"

if [ ! -x "$script" ]; then
  fail "release publication evidence script is not executable"
fi
if [ ! -x "$publish_script" ]; then
  fail "GitHub Release publication script is not executable"
fi

tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/resume-ir-release-publication-check.XXXXXX")
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

artifact_manifest="$tmpdir/release-artifacts.json"
out_dir="$tmpdir/out"
mkdir -p "$out_dir"
cat > "$artifact_manifest" <<'JSON'
{
  "schema_version": "release.artifacts.v1",
  "version": "v0.0.0",
  "packaging_status": "blocked",
  "artifacts": [
    {"name": "resume-cli", "file": "resume-cli", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "bytes": 101},
    {"name": "resume-daemon", "file": "resume-daemon", "sha256": "2222222222222222222222222222222222222222222222222222222222222222", "bytes": 202},
    {"name": "resume-benchmark", "file": "resume-benchmark", "sha256": "3333333333333333333333333333333333333333333333333333333333333333", "bytes": 303}
  ],
  "blocked_release_steps": ["packaging", "signing", "notarization", "github_release_upload"],
  "notes": "Synthetic dry-run fixture only."
}
JSON

"$script" --version v0.0.0 --artifact-manifest "$artifact_manifest" --out-dir "$out_dir"
manifest="$out_dir/release-publication-evidence.json"
require_file "$manifest"
require_text "$manifest" '"schema_version": "release.publication_evidence.v1"'
require_text "$manifest" '"publication_status": "blocked"'
require_text "$manifest" '"evidence_boundary": "dry_run_no_release_publication"'
require_text "$manifest" '"artifact_manifest_sha256": "'
require_text "$manifest" '"human_release_approval"'
require_text "$manifest" '"github_actions_release_token"'
require_text "$manifest" '"github_release_upload_evidence"'
require_text "$manifest" '"github_release_create"'
require_text "$manifest" '"github_release_upload"'
require_text "$manifest" '"prohibited_public_material": ['
require_text "$manifest" '"github_token"'
require_text "$manifest" '"local_paths"'

if grep -Fq "$tmpdir" "$manifest"; then
  fail "release publication evidence leaked an absolute temp path"
fi

if "$script" --version 0.0.0 --artifact-manifest "$artifact_manifest" --out-dir "$out_dir/invalid" >/dev/null 2>&1; then
  fail "release publication evidence accepted an invalid version"
fi

artifact_manifest_unknown="$tmpdir/release-artifacts-unknown-field.json"
cat > "$artifact_manifest_unknown" <<JSON
{
  "schema_version": "release.artifacts.v1",
  "version": "v0.0.0",
  "packaging_status": "blocked",
  "artifacts": [
    {"name": "resume-cli", "file": "resume-cli", "local_probe_path": "$tmpdir/PRIVATE-release-cache", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "bytes": 101},
    {"name": "resume-daemon", "file": "resume-daemon", "sha256": "2222222222222222222222222222222222222222222222222222222222222222", "bytes": 202},
    {"name": "resume-benchmark", "file": "resume-benchmark", "sha256": "3333333333333333333333333333333333333333333333333333333333333333", "bytes": 303}
  ],
  "blocked_release_steps": ["packaging", "signing", "notarization", "github_release_upload"],
  "notes": "Synthetic dry-run fixture only."
}
JSON
if "$script" --version v0.0.0 --artifact-manifest "$artifact_manifest_unknown" --out-dir "$out_dir/unknown-artifact" >/dev/null 2>&1; then
  fail "release publication evidence accepted an unknown artifact manifest field"
fi

artifact_manifest_missing_blockers="$tmpdir/release-artifacts-missing-blockers.json"
cat > "$artifact_manifest_missing_blockers" <<'JSON'
{
  "schema_version": "release.artifacts.v1",
  "version": "v0.0.0",
  "packaging_status": "blocked",
  "artifacts": [
    {"name": "resume-cli", "file": "resume-cli", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "bytes": 101},
    {"name": "resume-daemon", "file": "resume-daemon", "sha256": "2222222222222222222222222222222222222222222222222222222222222222", "bytes": 202},
    {"name": "resume-benchmark", "file": "resume-benchmark", "sha256": "3333333333333333333333333333333333333333333333333333333333333333", "bytes": 303}
  ],
  "blocked_release_steps": ["github_release_upload"],
  "notes": "Synthetic dry-run fixture only."
}
JSON
if "$script" --version v0.0.0 --artifact-manifest "$artifact_manifest_missing_blockers" --out-dir "$out_dir/missing-artifact-blockers" >/dev/null 2>&1; then
  fail "release publication evidence accepted incomplete artifact blocker evidence"
fi

artifact_manifest_duplicate="$tmpdir/release-artifacts-duplicate.json"
cat > "$artifact_manifest_duplicate" <<'JSON'
{
  "schema_version": "release.artifacts.v1",
  "version": "v0.0.0",
  "packaging_status": "blocked",
  "artifacts": [
    {"name": "resume-cli", "file": "resume-cli", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "bytes": 101},
    {"name": "resume-cli", "file": "resume-cli-copy", "sha256": "4444444444444444444444444444444444444444444444444444444444444444", "bytes": 404},
    {"name": "resume-daemon", "file": "resume-daemon", "sha256": "2222222222222222222222222222222222222222222222222222222222222222", "bytes": 202},
    {"name": "resume-benchmark", "file": "resume-benchmark", "sha256": "3333333333333333333333333333333333333333333333333333333333333333", "bytes": 303}
  ],
  "blocked_release_steps": ["packaging", "signing", "notarization", "github_release_upload"],
  "notes": "Synthetic dry-run fixture only."
}
JSON
if "$script" --version v0.0.0 --artifact-manifest "$artifact_manifest_duplicate" --out-dir "$out_dir/duplicate-artifact" >/dev/null 2>&1; then
  fail "release publication evidence accepted duplicate artifact entries"
fi

"$publish_script" \
  --dry-run \
  --version v0.0.0 \
  --repo FrankQDWang/resume-ir \
  --artifact-manifest "$artifact_manifest" \
  --publication-evidence "$manifest" \
  --out-dir "$out_dir"
gate="$out_dir/github-release-publication-gate.json"
require_file "$gate"
require_text "$gate" '"schema_version": "release.github_publication_gate.v1"'
require_text "$gate" '"execution_mode": "dry_run"'
require_text "$gate" '"publication_status": "blocked"'
require_text "$gate" '"approval_gate": "human_release_approval_required"'
require_text "$gate" '"secret_interface": "GITHUB_TOKEN_or_GH_TOKEN_required_for_execute"'
require_text "$gate" '"gh_release_create"'
require_text "$gate" '"gh_release_upload"'
require_text "$gate" '"gh_release_download_verify"'
if grep -Fq "$tmpdir" "$gate"; then
  fail "GitHub Release publication gate leaked an absolute temp path"
fi
publication_unknown="$tmpdir/release-publication-evidence-unknown-field.json"
python3 - "$manifest" "$publication_unknown" <<'PY'
import json
import sys
from pathlib import Path

source = Path(sys.argv[1])
target = Path(sys.argv[2])
document = json.loads(source.read_text(encoding="utf-8"))
document["artifacts"][0]["local_probe_path"] = "PRIVATE-release-cache"
target.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
PY
if "$publish_script" \
  --dry-run \
  --version v0.0.0 \
  --repo FrankQDWang/resume-ir \
  --artifact-manifest "$artifact_manifest" \
  --publication-evidence "$publication_unknown" \
  --out-dir "$out_dir/unknown-publication" >/dev/null 2>&1; then
  fail "GitHub Release publication gate accepted an unknown publication evidence field"
fi
publication_mismatch="$tmpdir/release-publication-evidence-mismatched-artifact.json"
python3 - "$manifest" "$publication_mismatch" <<'PY'
import json
import sys
from pathlib import Path

source = Path(sys.argv[1])
target = Path(sys.argv[2])
document = json.loads(source.read_text(encoding="utf-8"))
document["artifacts"][0]["artifact_sha256"] = "9999999999999999999999999999999999999999999999999999999999999999"
target.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
PY
if "$publish_script" \
  --dry-run \
  --version v0.0.0 \
  --repo FrankQDWang/resume-ir \
  --artifact-manifest "$artifact_manifest" \
  --publication-evidence "$publication_mismatch" \
  --out-dir "$out_dir/mismatched-publication" >/dev/null 2>&1; then
  fail "GitHub Release publication gate accepted mismatched publication artifact evidence"
fi
publication_missing_required="$tmpdir/release-publication-evidence-missing-required.json"
python3 - "$manifest" "$publication_missing_required" <<'PY'
import json
import sys
from pathlib import Path

source = Path(sys.argv[1])
target = Path(sys.argv[2])
document = json.loads(source.read_text(encoding="utf-8"))
document["required_evidence"] = ["github_release_upload_evidence"]
document["blocked_release_steps"] = ["github_release_upload"]
document["prohibited_public_material"] = ["local_paths"]
target.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
PY
if "$publish_script" \
  --dry-run \
  --version v0.0.0 \
  --repo FrankQDWang/resume-ir \
  --artifact-manifest "$artifact_manifest" \
  --publication-evidence "$publication_missing_required" \
  --out-dir "$out_dir/missing-required-publication" >/dev/null 2>&1; then
  fail "GitHub Release publication gate accepted incomplete publication blocker evidence"
fi
artifact_manifest_gate_missing_blockers="$tmpdir/release-artifacts-gate-missing-blockers.json"
cat > "$artifact_manifest_gate_missing_blockers" <<'JSON'
{
  "schema_version": "release.artifacts.v1",
  "version": "v0.0.0",
  "packaging_status": "blocked",
  "artifacts": [
    {"name": "resume-cli", "file": "resume-cli", "sha256": "1111111111111111111111111111111111111111111111111111111111111111", "bytes": 101},
    {"name": "resume-daemon", "file": "resume-daemon", "sha256": "2222222222222222222222222222222222222222222222222222222222222222", "bytes": 202},
    {"name": "resume-benchmark", "file": "resume-benchmark", "sha256": "3333333333333333333333333333333333333333333333333333333333333333", "bytes": 303}
  ],
  "blocked_release_steps": ["github_release_upload"],
  "notes": "Synthetic dry-run fixture only."
}
JSON
publication_gate_missing_blockers="$tmpdir/release-publication-evidence-gate-missing-artifact-blockers.json"
python3 - "$manifest" "$artifact_manifest_gate_missing_blockers" "$publication_gate_missing_blockers" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

source = Path(sys.argv[1])
artifact_manifest = Path(sys.argv[2])
target = Path(sys.argv[3])
document = json.loads(source.read_text(encoding="utf-8"))
document["artifact_manifest_sha256"] = hashlib.sha256(artifact_manifest.read_bytes()).hexdigest()
target.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
PY
if "$publish_script" \
  --dry-run \
  --version v0.0.0 \
  --repo FrankQDWang/resume-ir \
  --artifact-manifest "$artifact_manifest_gate_missing_blockers" \
  --publication-evidence "$publication_gate_missing_blockers" \
  --out-dir "$out_dir/missing-gate-artifact-blockers" >/dev/null 2>&1; then
  fail "GitHub Release publication gate accepted incomplete artifact blocker evidence"
fi
if "$publish_script" --execute --version v0.0.0 --repo FrankQDWang/resume-ir --artifact-manifest "$artifact_manifest" --publication-evidence "$manifest" --out-dir "$out_dir/execute" >/dev/null 2>&1; then
  fail "GitHub Release publication execute mode passed without explicit approval"
fi
execute_artifact_dir="$tmpdir/execute-artifacts"
mkdir -p "$execute_artifact_dir"
printf '%s\n' "synthetic cli release artifact" > "$execute_artifact_dir/resume-cli"
printf '%s\n' "synthetic daemon release artifact" > "$execute_artifact_dir/resume-daemon"
printf '%s\n' "synthetic benchmark release artifact" > "$execute_artifact_dir/resume-benchmark"
execute_artifact_manifest="$tmpdir/release-artifacts-execute.json"
execute_publication_evidence="$tmpdir/release-publication-evidence-execute.json"
python3 - "$execute_artifact_dir" "$execute_artifact_manifest" "$execute_publication_evidence" <<'PY'
import hashlib
import json
import sys
from pathlib import Path

artifact_dir = Path(sys.argv[1])
artifact_manifest = Path(sys.argv[2])
publication_evidence = Path(sys.argv[3])
release_artifacts = []
publication_artifacts = []
for name in ("resume-cli", "resume-daemon", "resume-benchmark"):
    path = artifact_dir / name
    raw = path.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    release_artifacts.append(
        {"name": name, "file": name, "sha256": digest, "bytes": len(raw)}
    )
    publication_artifacts.append(
        {
            "name": name,
            "file": name,
            "artifact_sha256": digest,
            "bytes": len(raw),
            "upload_status": "blocked",
        }
    )
artifact_document = {
    "schema_version": "release.artifacts.v1",
    "version": "v0.0.0",
    "packaging_status": "blocked",
    "artifacts": release_artifacts,
    "blocked_release_steps": ["packaging", "signing", "notarization", "github_release_upload"],
    "notes": "Synthetic execute-mode fixture only.",
}
artifact_manifest.write_text(json.dumps(artifact_document, indent=2) + "\n", encoding="utf-8")
publication_document = {
    "schema_version": "release.publication_evidence.v1",
    "version": "v0.0.0",
    "publication_status": "blocked",
    "evidence_boundary": "dry_run_no_release_publication",
    "artifact_manifest_sha256": hashlib.sha256(artifact_manifest.read_bytes()).hexdigest(),
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
    "notes": "Synthetic execute-mode fixture only.",
}
publication_evidence.write_text(json.dumps(publication_document, indent=2) + "\n", encoding="utf-8")
PY
fake_bin="$tmpdir/fake-bin"
fake_log="$tmpdir/fake-gh.log"
mkdir -p "$fake_bin"
cat > "$fake_bin/gh" <<'SH'
#!/usr/bin/env sh
set -eu
printf '%s\n' "$*" >> "$FAKE_GH_LOG"
if [ "$#" -ge 2 ] && [ "$1" = "release" ] && [ "$2" = "view" ]; then
  exit 1
fi
if [ "$#" -ge 2 ] && [ "$1" = "release" ] && [ "$2" = "create" ]; then
  exit 0
fi
if [ "$#" -ge 2 ] && [ "$1" = "release" ] && [ "$2" = "upload" ]; then
  exit 0
fi
if [ "$#" -ge 2 ] && [ "$1" = "release" ] && [ "$2" = "download" ]; then
  dir=""
  pattern=""
  shift 2
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --dir)
        [ "$#" -ge 2 ] || exit 2
        dir="$2"
        shift 2
        ;;
      --pattern)
        [ "$#" -ge 2 ] || exit 2
        pattern="$2"
        shift 2
        ;;
      *)
        shift
        ;;
    esac
  done
  [ -n "$dir" ] || exit 2
  [ -n "$pattern" ] || exit 2
  cp "$FAKE_RELEASE_ARTIFACT_DIR/$pattern" "$dir/$pattern"
  exit 0
fi
exit 2
SH
chmod +x "$fake_bin/gh"
: > "$fake_log"
PATH="$fake_bin:$PATH" \
GH_TOKEN="synthetic-token" \
FAKE_GH_LOG="$fake_log" \
FAKE_RELEASE_ARTIFACT_DIR="$execute_artifact_dir" \
  "$publish_script" \
    --execute \
    --approve-release \
    --version v0.0.0 \
    --repo FrankQDWang/resume-ir \
    --artifact-manifest "$execute_artifact_manifest" \
    --publication-evidence "$execute_publication_evidence" \
    --artifact-dir "$execute_artifact_dir" \
    --out-dir "$out_dir/execute-with-download" >/dev/null
require_file "$out_dir/execute-with-download/github-release-publication-gate.json"
require_text "$out_dir/execute-with-download/github-release-publication-gate.json" '"execution_mode": "execute"'
require_text "$out_dir/execute-with-download/github-release-publication-gate.json" '"publication_status": "ready_for_execute"'
require_text "$fake_log" "release upload"
if ! grep -Fq "release download" "$fake_log"; then
  fail "GitHub Release publication execute mode did not download uploaded artifacts for verification"
fi
corrupt_release_artifact_dir="$tmpdir/corrupt-release-artifacts"
mkdir -p "$corrupt_release_artifact_dir"
cp "$execute_artifact_dir/resume-cli" "$corrupt_release_artifact_dir/resume-cli"
cp "$execute_artifact_dir/resume-daemon" "$corrupt_release_artifact_dir/resume-daemon"
cp "$execute_artifact_dir/resume-benchmark" "$corrupt_release_artifact_dir/resume-benchmark"
printf '%s\n' "corrupted downloaded cli release artifact" > "$corrupt_release_artifact_dir/resume-cli"
if PATH="$fake_bin:$PATH" \
GH_TOKEN="synthetic-token" \
FAKE_GH_LOG="$fake_log" \
FAKE_RELEASE_ARTIFACT_DIR="$corrupt_release_artifact_dir" \
  "$publish_script" \
    --execute \
    --approve-release \
    --version v0.0.0 \
    --repo FrankQDWang/resume-ir \
    --artifact-manifest "$execute_artifact_manifest" \
    --publication-evidence "$execute_publication_evidence" \
    --artifact-dir "$execute_artifact_dir" \
    --out-dir "$out_dir/execute-corrupt-download" >/dev/null 2>&1; then
  fail "GitHub Release publication execute mode accepted a corrupted downloaded artifact"
fi

require_text "$workflow" "scripts/release/create-release-publication-evidence.sh"
require_text "$workflow" "scripts/release/publish-github-release.sh"
require_text "$workflow" "release-publication-evidence.json"
require_text "$workflow" "github-release-publication-gate.json"
require_text "$workflow" "release publication evidence manifest leaked a local path or runtime-data marker"
require_text "$workflow" "GitHub Release publication gate leaked a local path or runtime-data marker"
require_text "$verify_script" "./scripts/ci/check-release-publication-evidence.sh"

printf '%s\n' "release publication evidence check passed"

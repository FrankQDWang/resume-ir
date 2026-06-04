# Release Blockers Runbook

## Scope

Local-only release-readiness runbook. Do not upload real resumes, local data
directories, diagnostics, logs, indexes, model caches, tokens, or signing
material. Synthetic fixtures are the only public reproduction input.

This repository is not ready for stable release while any BLOCKED item below is
unresolved.

## Current BLOCKED Items

- signing certificates are not available for production installers
- notarization credentials are not available for macOS release artifacts
- Windows MSI install, upgrade, uninstall, and rollback are not proven
- Windows service install, start, stop, status, uninstall, rollback, and recovery
  are not proven
- macOS signed pkg/dmg install, upgrade, uninstall, and rollback are not proven
- 100k and 1M hot-index hybrid real-corpus benchmarks are not available
- private business labeled field-quality evidence is not available
- a reviewed licensed OCR engine is not selected or distributed
- a reviewed licensed embedding model is not selected or distributed
- Windows and macOS cross-platform validation are not complete

## Pre-Release Local Gate

Run the local gate before any public push:

```bash
./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
```

The explicit stable-release readiness gate must remain blocked until every
release criterion has current local evidence:

```bash
resume-cli --data-dir <local-data-dir> release-readiness --json
```

Run the benchmark smoke only as smoke evidence, not as production performance
proof:

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  synthetic-query --documents 24 --queries 6 --top-k 5 --json
```

Run the synthetic benchmark gate against a local or nightly benchmark artifact:

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  gate --report benchmark-smoke.json --allow-synthetic \
  --min-documents 1000 --min-queries 100 \
  --max-p95-ms 500 --max-zero-result-queries 0
```

The installed binary form is `resume-benchmark gate --report
benchmark-smoke.json`.

The explicit `--allow-synthetic` flag is required for synthetic smoke artifacts.
Do not treat a passing synthetic gate as 100k or 1M real-corpus proof.

Run private real-corpus benchmark gates only against local redacted aggregate
reports. The report must use `dataset_kind: "private-real-corpus"`,
`corpus_origin: "private_local"`, `privacy_boundary:
"redacted_local_aggregate"`, `query_mode: "hybrid"`, `retrieval_layers:
"fulltext+field+vector+rrf"`, `hot_index: true`, false hot-path OCR/parsing/
heavy-model-inference booleans, false raw-data/path/query booleans, and sha256
digests for the local dataset manifest plus query set. Do not upload reports if
they contain raw resume text, local paths, queries, sample IDs, or filenames.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  gate --report private-benchmark-100k.json \
  --require-private-real-corpus \
  --min-documents 100000 --min-queries 500 \
  --max-p95-ms 200 --max-zero-result-queries 0

cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  gate --report private-benchmark-1m.json \
  --require-private-real-corpus --require-million-scale \
  --min-documents 1000000 --min-queries 500 \
  --max-p95-ms 200 --max-zero-result-queries 0
```

These gates are release evidence validators. They do not create, upload, or
sanitize private benchmark reports and cannot clear the benchmark blocker until
representative local 100k and 1M runs exist.

Run private business field-quality gates only against local redacted aggregate
reports. The report must use `dataset_kind: "private-business-labeled"`,
`target_claim: "field_quality_target_met"`, `corpus_origin: "private_local"`,
`privacy_boundary: "redacted_local_aggregate"`, `field_taxonomy:
"resume-ir.fields.v1"`, false raw-data/path/field-value/sample-ID booleans, and
sha256 digests for both the dataset and annotation manifests. It must include
production field metrics for email, phone, school, degree, company, title,
skill, and date ranges. Do not upload reports if they contain raw resume text,
local paths, field values, sample IDs, filenames, or notes.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  field-gate --report private-field-quality.json \
  --require-private-business-labeled \
  --min-samples 1000 \
  --min-precision 0.93 --min-recall 0.93 --min-f1 0.93
```

This gate is a release evidence validator. It does not create, upload, label,
or sanitize private field-quality reports and cannot clear the field extraction
quality blocker until representative local business labels and aggregate field
metrics exist.

Generate a local release dry-run manifest only after release binaries have been
built:

```bash
scripts/release/create-artifact-manifest.sh \
  --version v0.1.0 \
  --target-dir target/release \
  --out-dir release-dry-run
scripts/release/create-sbom.sh \
  --version v0.1.0 \
  --out-dir release-dry-run
```

The generated `release-artifacts.json` records binary names, byte counts, and
sha256 hashes. The generated `release-sbom.json` is a redacted SPDX 2.3 package
inventory derived from locked Cargo metadata. These dry-run files are not an
installer, signature, notarization ticket, or GitHub Release upload, and they
must not contain local paths or runtime data.

On macOS only, generate unsigned pkg/dmg dry-run artifacts after release
binaries have been built:

```bash
scripts/release/create-macos-package.sh \
  --version v0.1.0 \
  --target-dir target/release \
  --out-dir release-dry-run
```

The generated `macos-package.json` records only artifact filenames, byte counts,
hashes, unsigned status, and still-blocked release steps. The pkg/dmg files are
local evidence only. They are not signed, not notarized, not uploaded, and do
not prove install, upgrade, uninstall, rollback, or Gatekeeper behavior.

On Windows only, generate an unsigned MSI dry-run artifact after release
binaries have been built and the WiX .NET tool is installed:

```powershell
dotnet tool install --global wix --version 6.0.2
scripts/release/create-windows-package.ps1 `
  -Version v0.1.0 `
  -TargetDir target/release `
  -OutDir release-dry-run
```

The generated `windows-package.json` records only artifact filenames, byte
counts, hashes, unsigned status, MSI kind, and still-blocked release steps. The
MSI file is local evidence only. It is not signed, not uploaded, and does not
prove install, upgrade, uninstall, rollback, Windows service registration, or
service lifecycle behavior.

Generate local Windows Service dry-run evidence without registering a service:

```bash
resume-cli --data-dir <local-data-dir> service install \
  --platform windows-service \
  --daemon-binary <path-to-resume-daemon.exe> \
  --dry-run
resume-cli --data-dir <local-data-dir> service status \
  --platform windows-service \
  --dry-run
resume-cli --data-dir <local-data-dir> service start \
  --platform windows-service \
  --dry-run
resume-cli --data-dir <local-data-dir> service stop \
  --platform windows-service \
  --dry-run
resume-cli --data-dir <local-data-dir> service uninstall \
  --platform windows-service \
  --dry-run
```

These dry-runs are redacted command-plan evidence only. They do not prove
Windows service registration, service recovery, rollback, upgrade behavior, or
administrator-elevated install/uninstall.

Validate any proposed local model pack before worker configuration:

```bash
resume-cli --data-dir <local-data-dir> model validate-manifest \
  --manifest <local-model-manifest.json>
```

This command is governance evidence only. A valid manifest does not by itself
complete licensed model selection, model quality evaluation, distribution
approval, or production performance proof.

Validate any proposed local OCR runtime pack before worker configuration:

```bash
resume-cli --data-dir <local-data-dir> ocr validate-manifest \
  --manifest <local-ocr-runtime-manifest.json>
```

This command is governance evidence only. A valid OCR runtime manifest does not
by itself complete OCR engine distribution approval, language-pack distribution
approval, non-English OCR quality validation, platform installer validation, or
production OCR throughput proof.

## Stable Release Exit Criteria

Stable release requires current evidence for:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --workspace --locked`
- redacted diagnostics without raw resume text or complete paths
- Windows install, upgrade, uninstall, service start, service stop, and rollback
- macOS install, upgrade, uninstall, LaunchAgent start, LaunchAgent stop, signing,
  and notarization
- 100k and 1M hot-index hybrid benchmark runs on representative hardware
- OCR and embedding model license review
- OCR runtime manifest checksum validation
- model pack manifest checksum validation

If any item is missing, keep the release blocked and update `PROGRESS.md` with
the exact missing evidence and owner.

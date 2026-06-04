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
- Windows MSI/service install, upgrade, uninstall, and rollback are not proven
- macOS signed pkg/dmg install, upgrade, uninstall, and rollback are not proven
- 100k and 1M real-corpus benchmarks are not available
- a reviewed licensed OCR engine is not selected or distributed
- a reviewed licensed embedding model is not selected or distributed
- Windows and macOS cross-platform validation are not complete

## Pre-Release Local Gate

Run the local gate before any public push:

```bash
./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
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

Validate any proposed local model pack before worker configuration:

```bash
resume-cli --data-dir <local-data-dir> model validate-manifest \
  --manifest <local-model-manifest.json>
```

This command is governance evidence only. A valid manifest does not by itself
complete licensed model selection, model quality evaluation, distribution
approval, or production performance proof.

## Stable Release Exit Criteria

Stable release requires current evidence for:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --workspace --locked`
- redacted diagnostics without raw resume text or complete paths
- Windows install, upgrade, uninstall, service start, service stop, and rollback
- macOS install, upgrade, uninstall, LaunchAgent start, LaunchAgent stop, signing,
  and notarization
- 100k and 1M benchmark runs on representative hardware
- OCR and embedding model license review
- model pack manifest checksum validation

If any item is missing, keep the release blocked and update `PROGRESS.md` with
the exact missing evidence and owner.

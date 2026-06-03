# Progress

This file tracks long-running production Goal execution against `GOAL.md`, the
system design docs, the execution docs, and this running evidence log. Obsolete
preliminary checklists are historical execution context only, not the
production-ready scope source.

## Execution Boundaries

- Repository: `/Users/frankqdwang/MLE/resume-ir`
- Data policy: S0-S65 used synthetic fixtures only; user has authorized future local-only real resume scanning/verification as long as resume data is not uploaded or transmitted over the network.
- Remote side effects: no push, PR, release, upload, signing, or notarization. S65 prepared GitHub repository automation locally, but remote repository creation/push/branch protection was not executed because the local `gh` credential for `FrankQDWang` is invalid.
- Slice rule: acceptance command passes before a slice is marked complete.

## Production Gap Audit

S42 included a read-only P0-P6 product gap audit using `GOAL.md`, the system
design docs, the execution docs, and this evidence log as scope sources. Deleted
obsolete preliminary files and checklists are not product scope.

- P0 architecture: Rust workspace, CLI/daemon entrypoints, SQLite metadata,
  task/status tables, loopback status IPC, an authenticated loopback import
  command IPC endpoint, CLI import-over-IPC submission, an authenticated
  loopback import cancellation command IPC endpoint, CLI cancel-over-IPC
  submission, authenticated loopback full-text search command IPC, CLI
  search-over-IPC submission, local and authenticated loopback redacted detail
  retrieval, doctor, diagnostics, a one-shot daemon import worker, a
  long-running daemon import scheduler, a daemon OCR worker loop for queued OCR
  jobs, and a daemon embedding worker loop for local vector snapshot generation
  exist. Import tasks have retry
  backoff, running-task heartbeat, stale-running task recovery, queued/
  retryable/running cancellation markers, cancelled-task status reporting, and
  cooperative cancellation checks during import scanning plus per-file import
  processing, and status-pollable live import progress persisted in scan scope
  counters without path disclosure. The daemon can also stream authenticated
  redacted import progress snapshots over loopback IPC while import, OCR, or
  embedding worker loops run. The daemon now writes a local endpoint discovery
  manifest, and the CLI can use `--ipc auto` for status, import progress,
  import, cancel-import, search, and detail commands. A daemon full-text index
  maintenance worker can now force a local snapshot rebuild or run in a loop to
  repair non-ready snapshot roots. Public-repository governance now includes
  MIT licensing, CODEOWNERS, contribution/security policy, PR templates,
  GitHub Actions workflow definitions, dependency update configuration, local
  license checks, and public push guardrails. Missing or BLOCKED production
  control-plane work includes remote GitHub repository creation/push/branch
  protection until `gh` is re-authenticated, service lifecycle, and macOS plus
  Windows validation.
- P1 import/search: directory scanning, DOCX/text-layer PDF/UTF-8 and
  BOM-marked UTF-16 TXT parsing, cleaning, sectioning, full-text snapshot
  publish/recover, delete rebuild, and redacted snippets exist. Missing
  production work includes watcher/background incremental import, legacy `.doc`
  support, production-grade PDF coverage, large-corpus proof, and incremental
  index updates.
- P2 fields/dedupe/privacy: high-confidence rules for name, contacts/date/
  education/company/title/skills/certs/years, persisted entity mentions,
  contact HMAC assignment, and candidate folding exist. Missing production work
  includes broader dictionaries, stronger normalization, soft-dedupe scoring,
  labeled F1 metrics, encrypted local storage, and physical purge.
- P3 semantic/hybrid: local embedding command protocol, persisted vector
  snapshot, linear KNN, RRF helpers, embedding worker, model/dimension-scoped
  durable per-version embedding jobs, model-scoped vector query isolation,
  section-level vector inputs, and CLI semantic/hybrid query execution now
  exist. The daemon can now execute a configured local embedding command in
  one-shot or long-running worker mode, persist a vector snapshot while serving
  status IPC, skip already completed version jobs across daemon restarts,
  re-embed completed versions when the configured model id or dimension
  changes, and write document plus section vectors inside one version job.
  Missing or BLOCKED work includes licensed model selection/distribution,
  ONNX/HNSW/FAISS or equivalent ANN, semantic quality metrics, and real
  performance proof.
- P4 OCR: OCR_REQUIRED routing, durable OCR jobs, pause/resume control, page
  cache schema, local OCR command client, timeout/cancel/temp cleanup, and OCR
  text indexing exist. The daemon can now claim queued OCR jobs, execute a
  configured local OCR command, persist cache entries, index OCR text, honor
  persistent pause state, and keep serving status IPC while OCR runs. Missing or
  BLOCKED work includes real PDF page rendering, multi-page OCR, bbox
  persistence, concrete OCR engine install/license, backpressure, and real
  scanned-resume witness runs.
- P5 packaging/platform: not production-ready. Installer, signing,
  notarization, LaunchAgent/user-mode service, Windows service/MSI, upgrade/
  uninstall, and release workflow remain absent or externally blocked by
  platform credentials/runners.
- P6 performance/stability: synthetic benchmark runner, status/doctor/export
  diagnostics, snapshot fallback, and targeted fault tests exist. Missing or
  BLOCKED work includes 100k/1M real-corpus benchmarks, nightly gates,
  destructive kill/disk-space fault injection, resource telemetry, runbooks, and
  cross-platform performance evidence.

## Slice Status

| Slice | Status | Evidence | Blockers |
|---|---|---|---|
| S0 | Complete | Git initialized; initial design baseline committed as `43e3d1c`; acceptance showed only S0 files pending before commit. | None |
| S1 | Complete | `cargo metadata --no-deps`, `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None |
| S2 | Complete | `cargo fmt --check`, `cargo test -p core-domain`, `cargo test -p config`, and `cargo clippy -p core-domain -p config --all-targets -- -D warnings` passed after review-fix changes. | None |
| S3 | Complete | `cargo fmt --check`, `cargo test -p meta-store`, and `cargo clippy -p meta-store --all-targets -- -D warnings` passed. | None |
| S4 | Complete | `cargo fmt --check`, `cargo test -p meta-store`, `cargo test -p resume-cli`, `cargo test -p resume-daemon`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S4 CLI/daemon smoke commands passed. | None for the S4 slice; product search, indexing, OCR, embeddings, IPC, diagnostics, and cross-platform verification remain not complete. |
| S5 | Slice complete | `cargo fmt --check`, `cargo test -p fs-crawler`, and `cargo clippy -p fs-crawler --all-targets -- -D warnings` passed. | None for the S5 slice; product import execution, document parsing, indexing, OCR, and query closure remain not complete. |
| S6 | Slice complete | `cargo fmt --check`, `cargo test -p parser-common`, `cargo test -p parser-docx`, `cargo test -p parser-pdf`, and `cargo clippy -p parser-common -p parser-docx -p parser-pdf --all-targets -- -D warnings` passed. | None for the S6 slice; OCR execution, text cleaning, indexing, search, and S7+ remain not complete. |
| S7 | Slice complete | `cargo fmt --check`, `cargo test -p text-normalizer`, `cargo test -p sectionizer`, `cargo test -p extractor-rules`, and `cargo clippy -p text-normalizer -p sectionizer -p extractor-rules --all-targets -- -D warnings` passed. | None for the S7 slice; import execution, indexing, search, OCR execution, embeddings, and S8+ remain not complete. |
| S8 | Slice complete | `cargo fmt --check`, `cargo test -p index-fulltext`, `cargo test -p search-planner`, `cargo run -p resume-cli -- search "Java 支付"`, and `cargo clippy -p index-fulltext -p search-planner -p resume-cli --all-targets -- -D warnings` passed. | None for the S8 slice; import execution, OCR execution, embeddings, vector search, and S9+ remain not complete. |
| S9 | Slice complete | `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S9 import/status/search smoke commands passed. | None for the S9 slice; OCR execution, embeddings, field filtering, packaging, and production-scale performance remain not complete. |
| S10 | Slice complete | `cargo fmt --check`, `cargo test -p extractor-rules`, `cargo test -p rank-fusion`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S10 filtered search smoke command passed. | None for the S10 slice; filters are recall-then-filter over the top full-text candidates, and OCR/embeddings/production-scale performance remain not complete. |
| S11 | Slice complete | `cargo test -p embedder`, `cargo test -p index-vector`, `cargo test -p rank-fusion`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None for the S11 skeleton; deterministic embedder and in-memory vector index are test-only scaffolding, not product semantic search or performance claims. |
| S12 | Slice complete | `cargo test -p ocr-client`, `cargo test -p ingest-scheduler`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None for the S12 skeleton; OCR remains disabled by default and no real OCR worker, DB page queue, or query-path OCR was added. |
| S13 | Slice complete | `cargo test --workspace`, `cargo run -p resume-cli -- doctor`, and `cargo run -p resume-cli -- export-diagnostics --redact` passed. | None for the S13 skeleton; query smoke is a small current-run measurement only, and fault handling is simulated/diagnostic rather than a destructive daemon kill or disk-fill exercise. |
| S14 | Product slice complete | `cargo fmt --check`, `cargo test -p meta-store`, `cargo test -p import-pipeline`, `cargo test -p resume-cli --test s8_search_cli`, `cargo test -p resume-cli --test s14_delete_search`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S14 import/search/delete/search CLI smoke passed. | None for this soft-delete/default-search slice; physical deletion, vector-index deletion, queue cancellation, atomic snapshot rollback, and complete audit retention remain not complete. |
| S15 | Product slice complete | `cargo fmt --check`, `cargo test -p meta-store`, `cargo test -p import-pipeline`, `cargo test -p resume-cli --test s15_ocr_handoff`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S15 import/status/doctor/search/export-diagnostics CLI smoke passed. | None for this durable OCR handoff slice; real OCR execution, page rendering/cache, pause/resume worker recovery, searchable OCR text indexing, bbox/confidence persistence, and deleted-document queue cancellation remain not complete. |
| S16 | Product slice complete | `cargo fmt --check`, `cargo test -p extractor-rules`, `cargo test -p meta-store`, `cargo test -p import-pipeline`, `cargo test -p resume-cli --test s16_persisted_fields`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S16 import/status/filtered-search/export-diagnostics CLI smoke passed. | None for this persisted-field-mention slice; Tantivy field fast fields, DB/index pre-filtering before recall, candidate soft dedupe/folding, contact hash indexes, field F1 benchmark, and production-scale field performance remain not complete. |
| S17 | Product slice complete | `cargo fmt --check`, `cargo test -p benchmark-runner`, `cargo clippy -p benchmark-runner --all-targets -- -D warnings`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, and the S17 `resume-benchmark synthetic-query` CLI smoke passed. | None for this synthetic benchmark-runner slice; real 10万/100万 corpus runs, real business query sets, OCR/vector benchmarks, RSS/CPU/disk telemetry, cross-platform benchmark evidence, and P95 target pass/fail gates remain not complete. |
| S18 | Product slice complete | `cargo fmt --check`, `cargo test -p resume-cli --test s18_candidate_folding`, `cargo test -p resume-cli --test s8_search_cli`, `cargo test -p resume-cli --test s10_search_filters`, `cargo test -p resume-cli --test s14_delete_search`, `cargo test -p resume-cli --test s16_persisted_fields`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None for this assigned-candidate search folding slice; automatic candidate assignment, contact-hash dedupe, merge confidence, candidate table/indexes, low-confidence suspected-same-person hints, and version expansion UI remain not complete. |
| S19 | Product slice complete | `cargo fmt --check`, `cargo test -p core-domain contact_hash_only_hydrates_external_keyed_digests`, `cargo test -p meta-store`, `cargo test -p import-pipeline`, `cargo test -p resume-cli --test s16_persisted_fields`, `cargo test -p resume-cli --test s18_candidate_folding`, `cargo clippy -p core-domain -p meta-store --all-targets -- -D warnings`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace` passed. | None for this candidate persistence and hashed-contact assignment slice; import-time keyed hashing, key management/rotation, automatic candidate assignment from extracted fields, candidate merge review, foreign-key migration enforcement, low-confidence duplicate hints, and version expansion UI remain not complete. |
| S20 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this loopback status IPC slice; final production IPC remains not complete: no gRPC/UDS/Named Pipe transport, authenticated command API, import/search IPC endpoints, service lifecycle integration, Windows IPC validation, or remote access support. |
| S21 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p privacy`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext snippets_redact_contact_values_near_query_matches`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p import-pipeline -p index-fulltext -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this import-time keyed-contact assignment slice; key rotation, encrypted metadata, candidate merge review UI, low-confidence duplicate hints, multi-contact conflict workflow, key backup/recovery, and full dedupe quality metrics remain not complete. |
| S22 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this `entity_mention` contact redaction slice; SQLite encryption, `resume_version.raw_text`/`clean_text`, full-text index contact storage, physical free-page/WAL purge, SQLCipher, key rotation/backup, diagnostic key health, and full PII audit remain not complete. |
| S23 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this future full-text index contact-redaction slice; existing Tantivy segments, SQLite `resume_version.raw_text`/`clean_text`, SQLCipher, physical deletion/free-page/WAL purge, hash-based contact search, and full PII audit remain not complete. |
| S24 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p privacy`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this contact-hash key diagnostics slice; key rotation, backup/recovery, SQLCipher, full diagnostic package audit, and complete PII audit remain not complete. |
| S25 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the S25 synthetic import/status/search/doctor/export-diagnostics CLI smoke passed. | None for this active full-text snapshot publish and diagnostics slice; last-good fallback after active pointer corruption, old snapshot GC, physical segment purge, vector snapshotting, SQLCipher, full disk-full/kill-daemon fault injection, and cross-platform atomic rename validation remain not complete. |
| S26 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this read-path full-text snapshot last-good fallback slice; snapshot GC/retention, active-pointer repair, staging cleanup, physical purge, vector fallback, real disk-full/kill-daemon fault injection, and cross-platform filesystem validation remain not complete. |
| S27 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p fs-crawler -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local discovery-profile slice; default whole-machine root presets, multi-root CLI/UI, progress/cancel/budget limits, persisted scan-profile schema, symlink cycle protection if follow-symlink is later enabled, real local resume witness runs, and cross-platform root/exclusion validation remain not complete. |
| S28 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this multi-root CLI import slice; automatic default root presets, persisted scan scope metadata, import progress/cancel, per-root partial-failure UX, true atomic multi-root transaction semantics, real local resume witness runs, and cross-platform root path validation remain not complete. |
| S29 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client`, `/Users/frankqdwang/.cargo/bin/cargo test -p ingest-scheduler`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p ingest-scheduler --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local OCR command execution client slice; concrete OCR engine selection/license/install, PDF page rendering, OCR cache persistence, worker queue integration, searchable OCR text indexing, bbox persistence, full pause/resume worker recovery, real scanned-resume witness run, and Windows command execution validation remain not complete or BLOCKED. |
| S30 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this SQLite OCR page cache slice; PDF page rendering, OCR worker queue integration, cache lookup/write from actual OCR execution, bbox storage, full-text indexing of OCR output, cache GC/retention, real scanned-resume witness run, and SQLCipher/physical purge remain not complete. |
| S31 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local OCR worker command/cache-write slice; PDF page rendering, per-page multi-page OCR, daemon-loop OCR execution, searchable OCR text indexing, bbox persistence, full pause/resume loop, concrete OCR engine install/license, real scanned-resume witness run, and Windows process-tree validation remain not complete. |
| S32 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local-discovery root preset slice; real whole-machine witness runs, explicit user confirmation UX, persisted scan-scope records, progress/cancel/budget limits, per-root partial-failure UX, cross-platform root enumeration validation, and proof that all local resumes are discoverable remain not complete. |
| S33 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this persisted OCR pause/resume control slice; daemon OCR loop integration, interrupting an already-running OCR child, process-tree pause semantics, PDF page rendering, concrete engine install/license, searchable OCR indexing, bbox persistence, real scanned-resume witness, and Windows process-control validation remain not complete. |
| S34 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p embedder`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p embedder -p index-vector --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local embedding command client slice; concrete embedding model selection/license/install, model distribution, embedding daemon/queue integration, persistent vector index, CLI semantic/hybrid search using the vector channel, quality/performance benchmarks, real data validation, and cross-platform process-tree validation remain not complete or BLOCKED. |
| S35 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p embedder`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this persisted import scan-scope metadata slice; live progress streaming, cancel/resume controls for import scans, budget limits, per-file scan error UI, real whole-machine witness runs, encrypted path metadata, and cross-platform root validation remain not complete. |
| S36 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p fs-crawler -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this scan file-budget enforcement slice; live progress streaming, user-triggered import cancellation, time/byte/CPU budgets, persisted per-file errors, real whole-machine witness runs, encrypted path metadata, and Windows/macOS full-disk validation remain not complete. |
| S37 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this redacted persisted scan-error slice; live progress streaming, user-triggered import cancellation, time/byte/CPU budgets, file-level UI/UX, real whole-machine witness runs, encrypted path metadata, keyed path-error correlation, and Windows/macOS full-disk validation remain not complete. |
| S38 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-vector -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this persisted vector snapshot slice; real licensed embedding model selection/distribution, import-time embedding queue integration, CLI semantic/hybrid query execution, vector snapshot GC/repair, quality benchmarks, real data validation, and cross-platform validation remain not complete or BLOCKED. |
| S39 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli embed_worker_debug_output_redacts_candidate_text_and_command_path`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p embedder`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p embedder -p index-vector --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this CLI local embedding worker slice; real licensed embedding model selection/distribution, OS-enforced no-network sandboxing for user-provided commands, daemon-loop embedding execution, import-time embedding job state, CLI semantic/hybrid query execution, vector snapshot GC/repair, quality benchmarks, real data validation, and cross-platform command validation remain not complete or BLOCKED. |
| S40 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this local-discovery default budget and multi-root budget-summary slice; live progress streaming, user cancellation, time/byte/CPU budgets, user-facing partial-result UX, real whole-machine witness runs, and Windows/macOS full-disk validation remain not complete. |
| S41 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this OCR worker searchable-index slice; multi-page PDF rendering, daemon-loop OCR execution, concrete OCR engine install/license, bbox persistence, real scanned-resume witness runs, encrypted OCR text storage/physical purge, and Windows process-tree validation remain not complete. |
| S42 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p rank-fusion -p resume-cli --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this CLI semantic/hybrid query slice; licensed embedding model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, daemon-loop embedding queue, section vectors, real semantic quality/performance benchmarks, OS-enforced no-network command sandboxing, and cross-platform validation remain not complete or BLOCKED. |
| S43 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this one-shot daemon import worker slice; long-running scheduling loop, authenticated import command IPC endpoint, import cancellation/progress streaming, background OCR/vector workers, multi-process stress testing, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S44 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this long-running daemon import scheduler slice; authenticated import command IPC endpoint, import cancellation/progress streaming, configurable retry policy, singleton service lifecycle enforcement, background OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S45 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_status_while_import_worker_processes_late_queued_task -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_does_not_start_import_worker_when_ipc_bind_fails -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_worker_tick_limit_in_combined_ipc_worker_mode -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this combined status IPC plus import worker event-loop slice; authenticated command IPC endpoint, import cancellation/progress streaming, configurable retry policy, singleton service lifecycle enforcement, background OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S46 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_authenticates_and_queues_import_command_over_ipc -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_requires_bearer_token_for_import_command_ipc -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_task_and_scan_scope_insert_atomically_for_daemon_command_ipc -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `/Users/frankqdwang/.cargo/bin/cargo test --workspace` passed. | None for this authenticated loopback import command IPC slice; search/detail IPC endpoints, CLI import-over-IPC UX, command token rotation/revocation, import cancellation/progress streaming, singleton service lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S47 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_import_command_preserves_local_discovery_preset_scope -- --exact`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this CLI import-over-IPC UX slice; search/detail IPC endpoints, daemon endpoint discovery UX, token rotation/revocation, import cancellation/progress streaming, singleton service lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S48 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this authenticated full-text search-over-IPC slice; daemon endpoint discovery UX, semantic/hybrid daemon search IPC, token rotation/revocation, import/search progress streaming, singleton service lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S49 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s49_detail_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this redacted detail retrieval slice; daemon endpoint discovery UX, semantic/hybrid daemon search IPC, token rotation/revocation, import/search/detail progress streaming, singleton service lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness runs, and Windows/macOS service validation remain not complete. |
| S50 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this daemon OCR worker slice; real PDF page rendering, multi-page OCR, bbox persistence, concrete OCR engine install/license, OCR backpressure, encrypted OCR text purge, real scanned-resume witness runs, daemon embedding worker, and Windows/macOS service/process validation remain not complete or BLOCKED. |
| S51 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this daemon local embedding worker slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, durable per-version embedding job state, section vectors, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S52 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this durable embedding job-state slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, model/version invalidation, section vectors, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S53 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this model/dimension-scoped durable embedding job slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, model-scoped vector query isolation, section vectors, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S54 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this model-scoped vector query isolation slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, section vectors, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S55 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this section-level vector input slice; licensed model selection/distribution, ONNX/HNSW/FAISS or equivalent ANN, semantic quality metrics, real performance proof, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S56 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this queued/retryable import cancellation slice; live progress streaming, cooperative cancellation of already-running import scans, daemon endpoint discovery UX, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, and Windows/macOS validation remain not complete. |
| S57 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p parser-text`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this TXT parser/import/search slice; legacy `.doc`, broader TXT encoding heuristics beyond UTF-8/BOM-marked UTF-16, watcher/background incremental import, production-grade PDF coverage, large-corpus proof, and incremental index updates remain not complete. |
| S58 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this high-confidence name mention slice; broad name dictionaries, multilingual name normalization, name-based soft-dedupe scoring, labeled field F1 metrics, encrypted local storage, and physical purge remain not complete. |
| S59 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this local IPC endpoint auto-discovery slice; live progress streaming, cooperative cancellation of already-running import scans, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, and Windows/macOS validation remain not complete. |
| S60 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this running-import cooperative cancellation slice; live progress streaming, cancel-over-IPC UX, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, and Windows/macOS validation remain not complete. |
| S61 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this status-pollable import progress slice; dedicated push progress streaming, cancel-over-IPC UX, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, and Windows/macOS validation remain not complete. |
| S62 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this authenticated import cancel-over-IPC slice; dedicated progress stream, token rotation/revocation, singleton service lifecycle enforcement, real whole-machine witness runs, Windows/macOS validation, and packaging/signing remain not complete or BLOCKED. |
| S63 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this authenticated import progress stream slice; daemon index-maintenance workers, service lifecycle, CI, CODEOWNERS, real whole-machine witness runs, Windows/macOS validation, token rotation/revocation, and packaging/signing remain not complete or BLOCKED. |
| S64 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace`, and the obsolete-reference marker scan passed with no matches. | None for this daemon full-text index maintenance worker slice; queued incremental index jobs, snapshot GC/retention, vector or ANN index maintenance, service lifecycle, CI, CODEOWNERS, real whole-machine witness runs, Windows/macOS validation, token rotation/revocation, and packaging/signing remain not complete or BLOCKED. |
| S65 | Local slice complete; remote BLOCKED | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo metadata --no-deps --locked --format-version 1`, `/Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- synthetic-query --documents 24 --queries 6 --top-k 5 --json`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace --locked`, `./scripts/ci/check-licenses.sh`, `./scripts/ci/guard-public-repo.sh`, `sh -n scripts/ci/guard-public-repo.sh scripts/ci/check-licenses.sh scripts/ci/verify-local.sh scripts/ci/configure-github-repo.sh`, and the obsolete-reference marker scan passed with no matches. | Remote GitHub repository creation, initial push, PR creation, and branch protection configuration are BLOCKED until the local `gh` credential for `FrankQDWang` is re-authenticated. Service lifecycle, real whole-machine witness runs, Windows/macOS validation, token rotation/revocation, and packaging/signing remain not complete or BLOCKED. |

## Command Log

### S65

Design target:

- S65 prepares the repository for public GitHub hosting without uploading real
  resumes, local data directories, daemon tokens, diagnostic bundles, logs,
  indexes, or model caches.
- The repository now has MIT licensing, CODEOWNERS, contribution and security
  policies, PR and issue templates, GitHub Actions workflow definitions,
  Dependabot configuration, AI coding harness instructions, local license
  checking, local public-repository guardrails, and a GitHub configuration
  script for repo creation, first push, and branch protection.
- Workspace crate metadata now uses `MIT` while keeping `publish = false`.

Implementation checks:

```bash
sh -n scripts/ci/guard-public-repo.sh scripts/ci/check-licenses.sh scripts/ci/verify-local.sh scripts/ci/configure-github-repo.sh
git diff --check
/Users/frankqdwang/.cargo/bin/cargo metadata --no-deps --locked --format-version 1
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- synthetic-query --documents 24 --queries 6 --top-k 5 --json
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace --locked
./scripts/ci/check-licenses.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
gh auth status
```

Output summary:

- Script syntax check: exit 0.
- `git diff --check`: exit 0.
- `cargo metadata --no-deps --locked --format-version 1`: exit 0 and shows
  workspace crates licensed as MIT.
- `cargo fmt --check`: exit 0.
- Synthetic benchmark smoke: exit 0 and emitted redacted synthetic JSON with
  no paths or raw resume text.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`:
  exit 0.
- `cargo test --workspace --locked`: exit 0.
- `./scripts/ci/check-licenses.sh`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `gh auth status`: exit 1; the active `FrankQDWang` token is invalid. Remote
  GitHub repository creation, initial push, PR creation, and branch protection
  configuration are therefore BLOCKED until re-authentication.

Scope note:

- S65 does not prove GitHub Actions execution on hosted runners, does not create
  the public remote repository, does not push a branch, and does not configure
  branch protection because the local GitHub CLI credential is invalid. It also
  does not implement service lifecycle, release packaging, signing,
  notarization, token rotation/revocation, real whole-machine witness runs, or
  Windows/macOS validation.

### S64

Design target:

- S64 closes the P0 gap where full-text index rebuild/repair existed only as a
  local CLI operation. The daemon now accepts `--work-index-once` to force a
  full-text snapshot rebuild from persisted local metadata and `--work-index`
  to repair non-ready snapshot roots inside the long-running worker loop.
- The full-text index worker is separate from embedding `UpdateIndex` jobs. It
  does not claim or repurpose embedding job queues.
- Because the product is not yet shipped, `--work-index` treats legacy
  root-layout indexes as unhealthy and rebuilds the published snapshot layout
  rather than preserving backward-compatible read behavior.
- Worker output reports only rebuild state and indexed document count. It does
  not print data directories, import roots, file paths, token material, raw
  resume text, or local query contents.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon foreground_once_index_worker_rebuilds_missing_full_text_snapshot_without_path_leak --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon foreground_index_worker_loop_repairs_missing_snapshot_once_per_health_change --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon foreground_index_worker_loop_rebuilds_legacy_root_snapshot_layout --test s4_daemon
```

Output summary:

- The one-shot worker test failed before implementation because
  `--work-index-once` was not parsed and daemon usage was returned.
- The loop worker test failed before implementation because `--work-index` was
  not parsed and daemon usage was returned.
- The legacy-root regression test failed before the predicate fix because the
  loop treated a `Ready` legacy root as healthy and skipped rebuild.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-daemon --test s4_daemon`: exit 0; 10 tests passed.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0.
- `cargo test --workspace`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.

Scope note:

- S64 does not implement queued incremental index jobs, snapshot GC/retention,
  vector or ANN index maintenance, singleton service lifecycle, CI, CODEOWNERS,
  token rotation/revocation, real whole-machine witness runs, Windows/macOS
  validation, or packaging/signing. Those remain incomplete or externally
  blocked.

### S63

Design target:

- S63 closes the P0 control-plane gap where import progress was visible only by
  polling status. The daemon now advertises an `import_progress` endpoint in
  its local endpoint manifest and serves authenticated newline-delimited JSON
  progress events over loopback IPC.
- The progress stream events reuse the same redacted import scan snapshot fields
  as status and never include requested roots, canonical roots, token material,
  raw resume text, or local data directory paths.
- `resume-cli status --watch-import --ipc auto` now discovers the daemon,
  validates the status endpoint, reads the local daemon token file, subscribes
  to the progress stream, and renders each progress event. Because the product
  is not yet shipped, explicit watch mode requires the real `/imports/progress`
  endpoint rather than accepting `/status` as a compatibility alias.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_streams_redacted_import_progress_over_loopback_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_watch_import_ipc_auto_streams_redacted_progress_without_local_store -- --exact
```

Output summary:

- The daemon test failed before implementation because `/imports/progress` did
  not return `200 OK`.
- The CLI test failed before implementation because `status --watch-import`
  was not parsed and did not connect to the fake daemon.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-cli --test s20_status_ipc`: exit 0; 6 tests passed.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 18 tests passed.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S63 does not implement daemon index-maintenance workers, singleton service
  lifecycle, CI, CODEOWNERS, token rotation/revocation, real whole-machine
  witness runs, Windows/macOS validation, or packaging/signing. Those remain
  incomplete or externally blocked.

### S62

Design target:

- S62 closes the P0 user-control gap where import cancellation markers and
  running-task cooperative checks existed, but a user could not request import
  cancellation through the daemon command IPC control plane.
- The daemon endpoint manifest now advertises a redacted `import_cancel`
  endpoint. Authenticated `POST /imports/cancel` accepts a task id, validates
  the task state, records the cancellation marker, and returns a response that
  does not include root paths, token material, or raw store diagnostics.
- `resume-cli cancel import` keeps the existing local store path and now also
  supports explicit cancel IPC plus `--ipc auto` endpoint discovery with the
  shared local daemon token file.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_import_cancel_command_records_cancellation_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc cancel_import_ipc_submits_authenticated_request_without_touching_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc cancel_import_ipc_auto_discovers_endpoint_and_token_file -- --exact
```

Output summary:

- The daemon test failed before implementation because `/imports/cancel` did
  not return `202 Accepted`.
- The CLI tests failed before implementation because `cancel import` did not
  parse IPC options and did not connect to the fake daemon.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-cli --test s47_import_ipc`: exit 0; 10 tests passed.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 17 tests passed.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S62 does not implement a dedicated progress stream, token rotation/
  revocation, singleton service lifecycle, real whole-machine witness runs,
  Windows/macOS validation, or release packaging/signing. Those remain
  incomplete or externally blocked.

### S61

Design target:

- S61 closes the local status visibility gap where import scan scopes were
  initialized and finalized but did not receive pipeline-owned progress updates
  while import work advanced.
- `import-pipeline` now updates an existing `ImportScanScope` after scan error
  persistence, periodically during per-file processing, after deletion
  propagation, during searchable document finalization, and after final index
  state update. The update path is a no-op when no scope exists, preserving older
  direct import callers.
- `resume-cli status` now prints latest import progress counters without root
  paths. Daemon `/status` now includes a `latest_import_scan` object with the
  same redacted counters, and CLI IPC status renders it.
- This is status-pollable live progress, not a dedicated push/SSE progress
  stream.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline tests::import_root_updates_existing_scan_scope_progress_without_daemon_postprocessing -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli status_reports_latest_import_scan_progress_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_can_read_redacted_daemon_status_over_loopback_ipc -- --exact
```

Output summary:

- `import-pipeline` failed before implementation because the existing scan
  scope still had `files_discovered = 0` after import completed without daemon
  post-processing.
- Local CLI status failed before implementation because it did not print latest
  import progress counters.
- IPC status rendering failed before implementation because CLI ignored
  daemon-provided latest import progress fields.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p import-pipeline`: exit 0; 5 unit tests passed.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S61 does not implement a dedicated progress stream, cancel-over-IPC UX, token
  rotation/revocation, singleton service lifecycle, real whole-machine witness
  runs, or Windows/macOS validation. Those remain incomplete or externally
  blocked.

### S60

Design target:

- S60 closes the P0 control-plane gap where cancellation markers existed for
  queued/retryable import tasks but a task already marked `Running` could not be
  cancelled cooperatively.
- `MetaStore::cancel_import_task` now records cancellation markers for running
  import tasks. Cancelled running tasks are excluded from root de-duplication,
  worker recovery, queued/recoverable status counts, and worker claims through
  the existing marker checks.
- `fs-crawler` now exposes explicit scan control with cancellation checks during
  directory traversal and fingerprinting. Cancellation returns a redacted
  cancellation error instead of a path-bearing scan error.
- `import-pipeline` now checks the cancellation marker before scan, during scan,
  before per-file work, around expensive parse/index steps, before deletion
  propagation, before snapshot publish, and before final index-state updates.
  A cancelled import transitions out of `Running` to retryable failure while the
  marker keeps it out of retry/recovery queues.
- The daemon import worker now counts a cooperatively cancelled import as
  cancelled in its summary rather than generic failed work.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store running_import_task_cancellation_is_recorded_and_removed_from_recovery -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler scan_control_cancels_directory_walk_without_path_leakage -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline import_root_stops_running_task_when_cancellation_marker_exists -- --exact
```

Output summary:

- `meta-store` failed before implementation because running-task cancellation
  returned `InvalidTransition`.
- `fs-crawler` failed before implementation because `ScanControl`,
  `crawl_with_fs_options_and_control`, and cancellation error variants did not
  exist.
- `import-pipeline` failed before implementation because there was no
  `Cancelled` import error path; after the first implementation pass it also
  exposed a timestamp boundary where finish time could be earlier than the
  cancellation marker. The final test uses the full module path and passed with
  one executed test.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p fs-crawler`: exit 0; 10 tests passed.
- `cargo test -p meta-store`: exit 0; 41 integration tests plus identity passed.
- `cargo test -p import-pipeline`: exit 0; 4 unit tests passed.
- `cargo test -p resume-daemon --test s4_daemon`: exit 0; 7 tests passed.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 16 tests passed.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 17 tests passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S60 does not implement live import progress streaming, cancel-over-IPC UX,
  token rotation/revocation, singleton service lifecycle, real whole-machine
  witness runs, or Windows/macOS validation. Those remain incomplete or
  externally blocked.

### S59

Design target:

- S59 closes the P0 control-plane gap where users had to copy a printed
  daemon IPC URL and separately locate the local command token file.
- The daemon now writes a local `ipc.endpoints.json` manifest after a loopback
  IPC bind succeeds. The manifest includes only status/import/search/detail
  loopback URLs and schema version; it does not include the token, token path,
  data directory, query text, roots, or resume text.
- CLI `status`, `import`, `search`, and `detail` now accept `--ipc auto`.
  Status reads only the manifest. Command endpoints read the manifest and then
  use `data-dir/ipc.auth` locally for bearer-token authentication.
- Auto command endpoints perform an unauthenticated `/status` liveness probe
  before sending any bearer token or private request body, and the daemon removes
  the manifest on normal IPC shutdown. Manifest writes reject symlink/non-file
  destinations and publish through an owner-only temporary file.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_ipc_auto_discovers_endpoint_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc import_ipc_auto_discovers_endpoint_and_token_file -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_auto_discovers_endpoint_and_token_file -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_auto_rejects_stale_manifest_without_sending_token_or_query -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc detail_ipc_auto_discovers_endpoint_and_token_file -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_redacted_status_over_loopback_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_symlinked_ipc_endpoint_manifest_without_clobbering_target -- --exact
```

Output summary:

- Before implementation, the CLI focused tests failed because `--ipc auto` did
  not connect to the fake daemon.
- Before implementation, the daemon focused test failed because no endpoint
  discovery manifest existed after IPC bind.
- Reviewer-driven RED checks then failed because auto command endpoints did not
  probe status before sending command payloads, normal IPC shutdown did not
  remove the manifest, and symlinked manifests were not rejected.
- After implementation, the focused tests passed and proved auto-discovered
  status/import/search/detail IPC, stale-manifest rejection before token/query
  send, manifest cleanup, symlink rejection, and manifest redaction.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-cli --test s20_status_ipc`: exit 0; 5 tests passed.
- `cargo test -p resume-cli --test s47_import_ipc`: exit 0; 8 tests passed.
- `cargo test -p resume-cli --test s48_search_ipc`: exit 0; 7 tests passed.
- `cargo test -p resume-cli --test s49_detail_ipc`: exit 0; 4 tests passed.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 16 tests passed.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S59 does not implement live progress streaming, cooperative cancellation of
  already-running import scans, token rotation/revocation, singleton service
  lifecycle enforcement, real whole-machine witness runs, or Windows/macOS
  validation. Those remain incomplete or externally blocked.

### S58

Design target:

- S58 closes the P2 gap where the domain and metadata model already supported
  `EntityType::Name` but rules never produced a name mention.
- Added high-confidence name extraction for explicit `Name:`/localized labels
  and conservative resume-heading candidates. The rule rejects section headers,
  contact lines, school/company lines, and known title lines to reduce false
  positives.
- Import now maps `FieldType::Name` to `EntityType::Name`, so extracted names
  are persisted with evidence, confidence, and extractor metadata through the
  existing entity mention path. The S57 synthetic TXT import test now asserts
  the persisted name mention.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules extracts_candidate_name_from_labeled_line_and_heading_with_evidence -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_txt_resume_builds_searchable_index_without_path_leakage -- --exact
```

Output summary:

- Before implementation, the focused extractor test failed to compile because
  `FieldType::Name` did not exist.
- Before implementation, the focused CLI import test failed because no
  persisted `EntityType::Name` mention existed for the synthetic TXT resume.
- After implementation, the focused tests prove labeled and heading name
  extraction, debug redaction of name text, and import-time persistence of the
  synthetic name mention.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p extractor-rules
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p extractor-rules`: exit 0; 9 tests passed, covering name
  extraction, false-positive avoidance, existing contact/date/education/company/
  title/skill/certificate extraction, and debug redaction.
- `cargo test -p import-pipeline`: exit 0; 2 tests passed.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 17 tests passed,
  including TXT import/search plus persisted name mention.
- `cargo test -p resume-cli`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S58 does not implement broad name dictionaries, multilingual name
  normalization, name-based soft-dedupe scoring, labeled field F1 metrics,
  encrypted local storage, or physical purge. Those remain incomplete.

### S57

Design target:

- S57 closes the P1 gap where `.txt` files were discovered by the crawler but
  failed permanently in import because no text parser was connected.
- Added a production `parser-text` crate for UTF-8, UTF-8 BOM, and BOM-marked
  UTF-16 text with parser-level budget support. Parser debug/error formatting
  does not expose raw text bytes.
- Import now routes `FileExtension::Txt` through the parser, then uses the
  existing normalizer, extractor, candidate assignment, and full-text snapshot
  path. CLI import/search tests cover a synthetic TXT resume without leaking
  temporary root paths or contact values in search output.
- TXT import has a pre-read byte cap and treats blank text as a failed document
  instead of enqueueing OCR, because OCR is not a valid recovery path for a
  plaintext file.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_txt_resume_builds_searchable_index_without_path_leakage -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_blank_txt_resume_fails_without_queueing_ocr -- --exact
```

Output summary:

- Before implementation, the focused CLI test failed because import stdout did
  not contain `searchable documents: 1`; the file was discovered but not
  parsed into a searchable document.
- Before the blank-TXT fix, the focused CLI test failed because import stdout
  did not contain `ocr required documents: 0`; blank TXT was incorrectly routed
  to the OCR queue.
- After implementation, the focused test proves a synthetic TXT resume imports
  as searchable, search can find it, blank TXT does not enqueue OCR, and output
  redacts the temp root and contact value.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p parser-text
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p parser-text`: exit 0; 8 tests passed, covering UTF-8,
  UTF-16LE/BE with BOM, unsupported-extension rejection, invalid UTF-8/UTF-16
  redaction, and parser byte-budget enforcement.
- `cargo test -p import-pipeline`: exit 0; 2 tests passed, preserving existing
  discovery deletion behavior with the TXT parser dependency wired in.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 17 tests passed.
  Coverage includes the TXT import/search loop, blank TXT non-OCR behavior,
  and existing import/search regressions.
- `cargo test -p resume-cli`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S57 does not implement legacy `.doc` parsing, broad non-BOM encoding
  detection, file streaming beyond the current pre-read cap, watcher/background
  incremental import, production-grade PDF coverage, large-corpus proof, or
  incremental index updates. Those remain incomplete.

### S56

Design target:

- S56 adds a production control-plane cancellation path for import tasks that
  have not started running yet. The metadata store now has a V14
  `import_task_cancellation` table, a task-id cancellation API, status summary
  counts, and claim/pending queries that exclude cancelled tasks.
- `resume-cli cancel import --task-id <id>` records cancellation without
  printing roots or paths. Local status and daemon status IPC include
  `import tasks cancelled`.
- Daemon import workers do not need a separate skip branch because cancelled
  tasks are no longer claimable. Daemon import command IPC can enqueue a new
  task for a root whose previous queued task was cancelled.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store cancelled_import_tasks_are_not_claimed_or_reported_as_queued -- --exact
```

Output summary:

- Before implementation, the focused metadata test failed to compile because
  `cancel_import_task`, `is_import_task_cancelled`, and
  `import_tasks_cancelled` did not exist.
- After implementation, metadata tests prove queued and retryable cancelled
  import tasks are not returned by pending lookup, are not claimed by workers,
  and are not counted as queued/recoverable.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
git diff --check
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p meta-store`: exit 0; 40 tests passed, including V14
  migration, queued/retryable cancellation, claim exclusion, and status counts.
- `cargo test -p resume-cli --test s20_status_ipc`: exit 0; 4 tests passed,
  including cancelled-count rendering from daemon status IPC.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 15 tests passed,
  including task-id cancellation without running import or leaking paths.
- `cargo test -p resume-cli`: exit 0.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 15 tests passed,
  including cancelled-count daemon status IPC and requeue after cancellation.
- `cargo test -p resume-daemon --test s4_daemon`: exit 0; 7 tests passed,
  including worker skip of a cancelled queued task.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S56 does not implement cooperative cancellation for an import task already
  inside a running scanner/import pipeline. That still needs cancellation-token
  plumbing through crawler, per-file processing, parser/index phases, and
  partial-write semantics. Live import progress streaming also remains
  incomplete.

### S55

Design target:

- S55 closes the product gap where embedding workers only generated one vector
  per resume version. CLI and daemon workers now keep the document-level vector
  for compatibility and additionally expand sectionizer output into
  `version:section:n` local embedding inputs.
- Section vector identity is stored only in the vector id. Vector `doc_id`
  remains the document id, so existing semantic hit hydration, candidate
  folding, and hybrid RRF behavior stay unchanged.
- Daemon durable jobs remain per version/model/dimension. A single claimed
  version job writes the document vector plus any section vectors, then
  completes once.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker semantic_and_hybrid_search_can_rank_section_vectors_over_document_vectors -- --exact
```

Output summary:

- Before implementation, semantic top-1 returned the synthetic document whose
  document-level vector was closer, because no section vector existed for the
  actual section match.
- After implementation, semantic and hybrid top-1 return the synthetic
  section-match document while redacting the query and local temp paths.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
git diff --check
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 6 tests
  passed. Coverage includes section vectors outranking a document vector in
  semantic and hybrid modes, model-scoped search isolation, and local command
  snapshot persistence without path leakage.
- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed, preserving one-shot and looped daemon embedding worker behavior.
- `cargo test -p resume-daemon --test s52_embedding_jobs`: exit 0; 4 tests
  passed. Coverage includes one version job writing multiple vectors and
  completing once, restart skip, and model-change re-embedding.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S55 does not choose or distribute a licensed model, add ANN/HNSW/FAISS, run
  semantic quality metrics, prove production-scale vector performance, enforce
  OS-level no-network sandboxing for configured commands, or validate
  Windows/macOS command behavior. Those remain incomplete or BLOCKED.

### S54

Design target:

- S54 fixes semantic/hybrid query isolation after embedding model changes. The
  persistent vector snapshot now writes v2 vector records with optional model id
  metadata while still reading existing v1 snapshots.
- `VectorIndex::knn_for_model` filters by explicit stored model id and falls
  back to the legacy vector-id prefix for old snapshots. Unscoped `knn` remains
  available for existing callers.
- CLI and daemon embedding workers now write model metadata with each vector,
  and CLI semantic search uses the requested model id when searching the vector
  snapshot. Hybrid search inherits the same protection through its semantic
  channel.
- During workspace verification, two daemon IPC long-poll tests exposed a
  request-budget race. Their test request budget now has headroom, with no
  production daemon behavior change.
- The workspace run also reproduced an existing CLI import-IPC closed-port
  race. That test now uses a deterministic local dropped-response fixture
  instead of relying on an unused port remaining unused.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_filters_knn_by_model_scope_after_reopen -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker semantic_search_uses_only_vectors_for_requested_model -- --exact
```

Output summary:

- Before implementation, both tests failed to compile because
  `VectorDocument::new_for_model` and `knn_for_model` did not exist.
- After implementation, the vector-index test proves a reopened snapshot can
  exclude a higher-scoring old-model vector. The CLI test proves both semantic
  and hybrid mode return only the requested model's vector result even when an
  old-model vector would otherwise win top-1.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
git diff --check
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p index-vector`: exit 0; 6 tests passed. Coverage includes v2
  model-scoped snapshot persistence after reopen and legacy v1 model-prefix
  fallback.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 5 tests
  passed. Coverage includes CLI semantic and hybrid model-scoped vector search.
- `cargo test -p resume-cli --test s47_import_ipc`: exit 0; 7 tests passed.
  Coverage includes deterministic import-IPC transport failure without local
  store fallback.
- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed, preserving daemon vector snapshot writes with the new format.
- `cargo test -p resume-daemon --test s52_embedding_jobs`: exit 0; 3 tests
  passed, preserving durable model/dimension-scoped embedding job behavior.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 14 tests passed after
  increasing long-poll test request budget headroom.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S54 does not choose or distribute a licensed model, add ANN/HNSW/FAISS, create
  section vectors, run semantic quality metrics, prove production-scale vector
  performance, enforce OS-level no-network sandboxing for configured commands,
  or validate Windows/macOS command behavior. Those remain incomplete or
  BLOCKED.

### S53

Design target:

- S53 fixes daemon embedding job invalidation when the configured model id or
  dimension changes. The metadata store now has a v13 `embedding_job_spec`
  table and scopes durable embedding jobs by `resume_version_id`, model id, and
  dimension.
- The daemon now enqueues and claims embedding jobs only for the active
  model/dimension pair, so completed jobs for one model no longer suppress
  embedding work for a different model or dimension.
- This slice still requires a user-provided local command, model id, and
  dimension. It does not choose, bundle, download, license, or claim a
  production embedding model.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store embedding_update_jobs_are_scoped_by_model_and_dimension -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs daemon_embedding_worker_once_reembeds_completed_jobs_for_new_model -- --exact
```

Output summary:

- Before implementation, the meta-store test failed to compile because
  `enqueue_embedding_job_for_resume_version` and `claim_next_embedding_job`
  did not accept model id or dimension.
- Before implementation, the daemon model-change test failed because the second
  run with a different model did not process the completed versions again.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p meta-store`: exit 0; 39 tests passed. Coverage includes v13
  migration, model/dimension-scoped embedding job idempotence, and
  model/dimension-filtered embedding-job claim.
- `cargo test -p resume-daemon --test s52_embedding_jobs`: exit 0; 3 tests
  passed. Coverage includes restart skip for the same model and re-embedding
  when the model id changes.
- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed, preserving daemon embedding worker behavior.
- `cargo test -p resume-daemon`: exit 0.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S53 does not choose or distribute a licensed model, add ANN/HNSW/FAISS, add
  model-scoped vector query isolation, create section vectors, run semantic
  quality metrics, prove production-scale vector performance, enforce OS-level
  no-network sandboxing for configured commands, or validate Windows/macOS
  command behavior. Those remain incomplete or BLOCKED.

### S52

Design target:

- S52 makes daemon embedding work durable at the resume-version level. The
  metadata store now persists idempotent `UpdateIndex` jobs with
  `resume_version_id`, exposes a dedicated embedding-job claim path that skips
  unrelated index jobs, and reports `embedding_queue_depth` from queued durable
  jobs instead of document lifecycle state.
- The daemon embedding worker now enqueues missing version jobs, claims durable
  embedding jobs, marks successful jobs completed, marks failed command/vector
  writes retryable, and skips completed version jobs after daemon restart.
- This slice still requires a user-provided local command, model id, and
  dimension. It does not select, bundle, download, license, or claim a
  production embedding model.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store embedding_update_jobs_are_durable_idempotent_and_claimable_by_resume_version -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs daemon_embedding_worker_once_skips_completed_jobs_after_restart -- --exact
```

Output summary:

- Before implementation, the meta-store test failed to compile because
  `MetaStore` had no version-level embedding job enqueue API.
- Before implementation, the daemon restart test failed because the second run
  still invoked the local embedding command and did not print
  `embedding worker processed: 0`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s52_embedding_jobs
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p meta-store`: exit 0; 38 tests passed. Coverage includes the
  new v12 migration, durable/idempotent version embedding jobs, dedicated
  embedding-job claim filtering, and status summary queue-depth aggregation from
  durable jobs.
- `cargo test -p resume-daemon --test s52_embedding_jobs`: exit 0; 2 tests
  passed. Coverage includes persisted completed embedding jobs and no repeated
  local embedding command invocation after daemon restart.
- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed, preserving daemon local embedding command execution and status IPC
  behavior.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 4 tests
  passed, preserving CLI local embedding worker and semantic/hybrid search.
- `cargo test -p resume-daemon`: exit 0; daemon identity, IPC, import, OCR,
  S51, and S52 tests passed.
- `cargo clippy --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0.

Scope note:

- S52 does not choose or distribute a licensed model, add ANN/HNSW/FAISS,
  invalidate completed jobs when the embedding model/dimension changes, create
  section vectors, run semantic quality metrics, prove production-scale vector
  performance, enforce OS-level no-network sandboxing for configured commands,
  or validate Windows/macOS command behavior. Those remain incomplete or
  BLOCKED.

### S51

Design target:

- S51 moves local embedding execution into the daemon control plane. A daemon
  can now run `--work-embeddings-once` or a long-running `--work-embeddings`
  loop, execute an explicitly configured local embedding command, persist the
  vector snapshot, and keep serving status IPC while embedding work runs.
- This slice still requires a user-provided local command, model id, and
  dimension. It does not select, bundle, download, license, or claim a
  production embedding model.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker daemon_embedding_worker_once_runs_local_command_and_persists_vector_snapshot -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker daemon_embedding_worker_loop_serves_status_ipc_while_persisting_vectors -- --exact
```

Output summary:

- Before implementation, the one-shot daemon embedding test failed because
  `resume-daemon run` rejected `--work-embeddings-once` as usage.
- Before implementation, the loop daemon embedding test failed because the
  daemon rejected `--work-embeddings` and exited before printing an IPC
  endpoint.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-daemon --test s51_embedding_worker`: exit 0; 2 tests
  passed. Coverage includes one-shot daemon embedding command execution,
  vector snapshot persistence, no stdout leakage of paths or embedded text, and
  a worker loop serving status IPC while persisting vectors.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 4 tests
  passed, preserving CLI local embedding worker and semantic/hybrid search
  behavior.
- `cargo test -p resume-daemon`: exit 0; daemon identity, status/search/detail
  IPC, import worker, OCR worker, and S51 embedding worker tests passed.
- Focused daemon clippy passed.

Scope note:

- S51 does not choose or distribute a licensed model, add ANN/HNSW/FAISS,
  persist durable per-version embedding job state, create section vectors, run
  semantic quality metrics, prove production-scale vector performance, enforce
  OS-level no-network sandboxing for configured commands, or validate
  Windows/macOS command behavior. Those remain incomplete or BLOCKED.

### S50

Design target:

- S50 moves OCR execution into the daemon control plane. A daemon can now run
  `--work-ocr-once` or a long-running `--work-ocr` loop, claim durable
  `OcrDocument` jobs, honor the persistent OCR pause flag, execute a configured
  local OCR command, persist the page cache, index successful OCR text, and
  keep serving status IPC while the OCR worker loop runs.
- The daemon summary output reports counts only. It does not print source
  document paths, data-dir paths, OCR command paths, or OCR text.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_executes_local_command_and_indexes_scanned_pdf -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_loop_serves_status_ipc_while_indexing_scanned_pdf -- --exact
```

Output summary:

- Before implementation, the one-shot daemon OCR test failed because
  `resume-daemon run` rejected `--work-ocr-once` as usage.
- Before implementation, the loop daemon OCR test failed because the daemon
  rejected `--work-ocr` and exited before printing an IPC endpoint.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `cargo test -p resume-daemon --test s50_ocr_worker`: exit 0; 3 tests passed.
  Coverage includes one-shot daemon OCR command execution, cache persistence,
  searchable OCR text indexing, persistent pause preventing job claim/command
  invocation, no stdout leakage of paths or OCR text, and an OCR worker loop
  serving status IPC while processing a queued OCR job.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 7 OCR handoff
  tests passed, preserving CLI OCR worker command/cache/pause/resume behavior.
- `cargo test -p resume-daemon`: exit 0; daemon identity, status/search/detail
  IPC, import worker, combined IPC-worker, and S50 OCR worker tests passed.
- `cargo clippy -p resume-daemon --all-targets -- -D warnings`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt
  --check`, `git diff --check`, and `cargo test --workspace` all passed.
- The obsolete-reference marker scan returned no matches.

Scope note:

- S50 does not render real PDF pages for OCR, split multi-page scanned PDFs,
  persist OCR bounding boxes, choose/install/license an OCR engine, implement
  OCR backpressure, encrypt/purge OCR text, run a real scanned-resume witness,
  add daemon embedding execution, or validate Windows/macOS service and process
  behavior. Those remain incomplete or BLOCKED.

### S49

Design target:

- S49 adds local `resume-cli detail --doc-id <doc_id>` and authenticated
  loopback `resume-cli detail --ipc ... --ipc-token-file ...` against daemon
  `POST /details`.
- Detail output returns redacted structured fields and a short redacted snippet
  for the latest non-hidden resume version. It does not return source URI,
  normalized path, raw full text, tokens, or local private paths.
- CLI IPC mode validates the success protocol, checks the returned doc id
  against the request, validates enum-like strings before printing them, and
  does not fall back to opening the local store when IPC fails.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_cli detail_local_prints_redacted_fields_and_short_snippet_without_private_paths -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc detail_ipc_submits_authenticated_request_and_renders_redacted_detail_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s49_detail_ipc daemon_detail_ipc_authenticates_and_returns_redacted_structured_detail -- --exact
```

Output summary:

- The local CLI red test failed before implementation because `resume-cli`
  did not recognize the `detail` command.
- The CLI IPC red test failed before implementation because it never connected
  to the fake daemon listener.
- The daemon red test failed before implementation because `/details` returned
  a non-200 response.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s49_detail_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s49_detail_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p meta-store -p resume-cli -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo test -p resume-cli --test s49_detail_cli`: exit 0; 2 tests passed.
  Coverage includes local redacted details, latest non-hidden version
  selection, deleted-document hiding, contact/path redaction, and no full
  raw-text leakage.
- `cargo test -p resume-cli --test s49_detail_ipc`: exit 0; 3 tests passed.
  Coverage includes successful authenticated request rendering, no local-store
  fallback, HTTP error behavior, invalid token/non-loopback rejection, malformed
  success protocol rejection, response doc-id matching, enum validation, and no
  token/path/contact leakage.
- `cargo test -p resume-daemon --test s49_detail_ipc`: exit 0; 3 tests passed.
  Coverage includes bearer authentication, redacted structured details, latest
  version selection, deleted-document hiding, invalid JSON/doc-id rejection, and
  not-found responses without sensitive values.
- `cargo test -p index-fulltext`: exit 0; 12 tests passed, including common
  local path redaction.
- `cargo test -p meta-store`: exit 0; 37 tests passed, including latest
  visible resume-version selection.
- `cargo test -p resume-cli`, `cargo test -p resume-daemon`, `cargo fmt
  --check`, `git diff --check`, focused clippy, workspace clippy, and
  `cargo test --workspace` passed.
- The obsolete-reference marker scan was re-run and returned no matches.

Sub-agent review:

- Two read-only Codex sub-agents reviewed the S49 diff. Medium findings around
  path-like detail redaction, unstable version selection, and loose CLI IPC
  protocol validation were fixed before commit and covered by tests. A low
  duplication note remains accepted for this slice because the CLI and daemon
  are separate binaries and no shared protocol crate exists yet.

Scope note:

- S49 does not add daemon endpoint discovery UX, semantic/hybrid daemon search
  IPC, token rotation/revocation, progress streaming, singleton service
  lifecycle enforcement, daemon OCR/vector workers, real whole-machine witness
  scans, or macOS/Windows service validation. Those remain incomplete.

### S48

Design target:

- S48 adds an authenticated loopback daemon search command IPC endpoint and CLI
  `resume-cli search --ipc ... --ipc-token-file ...` mode. This lets a local
  caller query the daemon's persistent full-text index without opening the
  metadata database or index from the CLI process.
- The new endpoint is bearer-token protected with the existing daemon IPC token,
  rejects non-loopback CLI targets, returns static redacted errors, validates the
  response protocol on the CLI, and redacts contact values in file names and
  snippets before rendering or returning results.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc search_ipc_submits_authenticated_request_and_renders_redacted_results_without_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_authenticates_filters_and_redacts_results -- --exact
```

Output summary:

- The CLI red test failed before implementation because `resume-cli search` did
  not recognize the IPC flags and never connected to the fake daemon listener.
- The daemon red test failed before implementation because the daemon did not
  have search IPC support and lacked the full-text search dependency.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc -- --test-threads=1
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo test -p resume-cli --test s48_search_ipc`: exit 0; 5 tests passed.
  Coverage includes successful authenticated request rendering, HTTP error
  no-fallback behavior, invalid success protocol rejection, invalid JSON,
  malformed response, non-loopback rejection, invalid token rejection, wrong
  path rejection, connect failure, local-store no-fallback, and query/token/path
  redaction.
- `cargo test -p resume-daemon --test s48_search_ipc`: exit 0; 4 tests passed.
  Coverage includes authenticated full-text search with degree/skill/years
  filters, result contact redaction, missing/wrong bearer token, invalid JSON,
  empty query, unsupported mode, malformed filters, not-ready index response,
  and no query/token/path leakage.
- `cargo test -p resume-daemon --test s20_ipc -- --test-threads=1`: exit 0; 14
  tests passed after confirming import/status IPC compatibility.
- `cargo test -p resume-cli`, `cargo test -p resume-daemon`,
  `cargo test -p rank-fusion`, `cargo fmt --check`, `git diff --check`, and the
  focused clippy command passed.
- Workspace clippy and `cargo test --workspace` passed after the final S48
  changes; all workspace tests and doc-tests completed with 0 failures.
- The obsolete-reference marker scan was re-run and returned no matches.

Sub-agent review:

- A read-only Codex sub-agent review found no high or medium security/privacy
  regressions. Its low-risk findings were addressed before commit by tightening
  CLI response schema validation and adding daemon/CLI negative tests for
  malformed protocol, invalid JSON, unsupported modes, malformed filters, wrong
  path, and not-ready index behavior.

Scope note:

- S48 does not add detail IPC endpoints, daemon endpoint discovery UX,
  semantic/hybrid daemon search IPC, token rotation/revocation, import/search
  progress streaming, singleton service lifecycle enforcement, daemon OCR/vector
  workers, real whole-machine witness scans, or macOS/Windows service
  validation. Those remain incomplete.

### S47

Design target:

- S47 wires the S46 authenticated daemon import command IPC into the CLI. A
  local caller can now run `resume-cli import --ipc ... --ipc-token-file ...`
  to submit explicit roots to the daemon without opening or writing the
  metadata store directly.
- IPC mode remains loopback-only, reads the bearer token from a caller-supplied
  local token file, sends only the import command payload to `/imports`, and
  keeps stdout/stderr free of token values and local paths.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc import_ipc_submits_authenticated_request_without_touching_local_store -- --exact
```

Output summary:

- Failed before implementation because `resume-cli import` did not recognize
  the IPC flags and never connected to the fake daemon listener.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc import_ipc_submits_authenticated_request_without_touching_local_store -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s47_import_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_import_command_preserves_local_discovery_preset_scope -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- The successful IPC import test passed: CLI sends `POST /imports`, includes
  the bearer token in the `Authorization` header, serializes roots/profile/file
  budget as JSON, renders the daemon `202 Accepted` response as queued import
  output, omits root path/token path/token content from stdout, and does not
  create the local `--data-dir`.
- `cargo test -p resume-cli --test s47_import_ipc`: exit 0; 7 tests passed.
  The suite covers success, local-discovery preset preservation, HTTP error,
  invalid JSON, connect failure, malformed response, non-loopback rejection,
  missing token file, invalid token content, no local-store fallback, and
  token/path redaction.
- The daemon focused preset test passed and verified that import command IPC
  persists local-discovery scan scope metadata with `Preset` root kind.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 14 tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed, including existing
  local import/search/status/OCR/embedding behavior.
- `cargo fmt --check`, `git diff --check`, workspace clippy, and
  `cargo test --workspace` passed.
- The obsolete-reference marker scan was re-run and returned no matches.

Sub-agent orchestration:

- A separate Codex sub-agent review was spawned after local verification to
  inspect the S47 diff for token/path leakage, endpoint parsing, no-fallback
  guarantees, and compatibility with existing import/status commands. Review
  findings were fixed before commit: IPC mode now preserves root preset
  semantics, validates the daemon token shape before composing headers, reads
  fake-daemon request bodies by `Content-Length`, and tests connect-failure,
  malformed-response, and invalid-JSON no-fallback paths.

Scope note:

- S47 does not add search/detail IPC endpoints, daemon endpoint discovery UX,
  token rotation/revocation, import cancellation/progress streaming, singleton
  service lifecycle enforcement, daemon OCR/vector workers, real whole-machine
  witness scans, or macOS/Windows service validation. Those remain incomplete.

### S0

```bash
git init
git add GOAL.md MANIFEST.md 01_system_design_系统设计 02_execution_plan_执行方案 docs
git commit -m "docs: commit initial design baseline"
```

Output summary:

- Initialized empty Git repository.
- Created root commit `43e3d1c` with 25 design and execution planning files.

```bash
git status --short
git log --oneline -3
```

Output summary:

- `git status --short`: `.gitignore`, `PROGRESS.md`, and `README.md` were the only untracked files before the S0 commit.
- `git log --oneline -3`: `43e3d1c docs: commit initial design baseline`.

### S1

Baseline red check:

```bash
cargo metadata --no-deps
```

Output summary:

- Failed before implementation with `could not find Cargo.toml`.

TDD checks:

```bash
cargo test
cargo test -p resume-daemon --test identity
```

Output summary:

- First test run failed because `core-domain`, `config`, and `meta-store` did not expose `crate_name()`.
- After adding library identities, `resume-cli --identity` failed because the binary produced no stdout.
- After adding the CLI identity output, `resume-daemon --identity` failed because the binary produced no stdout.

Acceptance:

```bash
cargo metadata --no-deps
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo metadata --no-deps`: exit 0; workspace contains `core-domain`, `config`, `meta-store`, `resume-daemon`, and `resume-cli` with edition 2021. Cargo emitted the expected compatibility warning about omitting `--format-version`.
- `cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; 5 identity tests passed, plus crate unit/doc test harnesses with 0 failures.

### S2

TDD red checks:

```bash
cargo test -p core-domain
cargo test -p config
```

Output summary:

- `core-domain` failed before implementation because the S2 domain ID, model, and error types were unresolved imports in the new behavior tests.
- `config` failed before implementation because `Profile` and `ProfileDefaults` were unresolved imports in the new behavior tests.

Review-fix red check:

```bash
cargo test -p core-domain
```

Output summary:

- Failed before the review-fix implementation because tests required design-aligned model fields, full document lifecycle states, the exact layered `ErrorKind` list, validated ID hydration, the golden opaque ID string, and the `ContactHash` privacy boundary.

Acceptance:

```bash
cargo fmt --check
cargo test -p core-domain
cargo test -p config
cargo clippy -p core-domain -p config --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `core-domain`: exit 0; identity test plus 7 S2 tests passed, covering design-aligned fields, full lifecycle states, exact error kinds, diagnostic redaction, redacted domain debug output, validated ID hydration, golden opaque ID generation, and `ContactHash` hydration.
- `config`: exit 0; identity test plus 2 S2 tests passed, covering default Balanced profile and deterministic Economy/Balanced/Turbo resource tiers.
- `cargo clippy -p core-domain -p config --all-targets -- -D warnings`: exit 0.

### S3

Baseline check:

```bash
cargo test -p meta-store
```

Output summary:

- Passed before S3 work with only the existing meta-store identity test.

TDD red check:

```bash
cargo test -p meta-store
```

Output summary:

- Failed before implementation because the S3 tests imported missing SQLite store APIs, migration reporting, task queue types, and index state persistence types.

Implementation check:

```bash
cargo test -p meta-store
```

Output summary:

- First implementation run passed migration idempotency, visible-document filtering, and index-state persistence tests, then failed the recovery query because the internal job query SQL was malformed.
- After fixing the query template and adding the file-backed open path, exit 0; identity plus 5 S3 tests passed.

Acceptance:

```bash
cargo fmt --check
cargo test -p meta-store
cargo clippy -p meta-store --all-targets -- -D warnings
```

Output summary:

- Initial `cargo fmt --check` reported formatting diffs; after `cargo fmt`, `cargo fmt --check` exited 0.
- `cargo test -p meta-store`: exit 0; identity test plus 5 S3 tests passed, covering migration idempotency/schema version/table existence, hidden deleted documents, recovery query filtering, job status update, resume version persistence, index state upsert/query, and file-backed SQLite reopen behavior.
- `cargo clippy -p meta-store --all-targets -- -D warnings`: exit 0.

Review-fix:

```bash
cargo test -p core-domain
cargo test -p meta-store
cargo fmt --check
cargo clippy -p core-domain -p meta-store --all-targets -- -D warnings
```

Output summary:

- Red checks failed before the review fix because `ContactHash` display exposed the full digest and `meta-store` lacked claim-next-job, job lookup, and file-backed PRAGMA APIs.
- After the review fix, `core-domain` tests passed with `ContactHash` display redacted while `.as_str()` still exposes explicit persistence material.
- After the review fix, `meta-store` tests passed with 12 S3 integration tests covering queue/recovery separation, atomic claim semantics, timestamp transitions, invalid transition errors, schema CHECK constraints, file-backed PRAGMA setup, FK rejection/cascade, file-backed reopen recovery, and SQLite metadata/task persistence.
- This remains plaintext SQLite metadata/task persistence only; no SQLCipher or production data encryption claim is made.

### S4

Baseline red checks:

```bash
cargo run -p resume-cli -- status
cargo run -p resume-cli -- import --root tests/fixtures/empty
cargo run -p resume-cli -- search "Java"
```

Output summary:

- Before S4 implementation, all three commands exited 2 with `resume-cli: no commands are implemented in S1`.

Implementation checks:

```bash
cargo fmt --check
cargo test -p meta-store
cargo test -p resume-cli
cargo test -p resume-daemon
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p meta-store`: exit 0; identity plus 16 SQLite tests passed, including import-task persistence without document FK, import-task lifecycle constraints, status aggregation, schema v2 idempotency, v1-to-v2 upgrade, CHECK constraints, recovery queries, and file-backed reopen behavior.
- `cargo test -p resume-cli`: exit 0; identity plus 3 S4 CLI tests passed, covering status, import-root task submission, no path leak, unavailable search without metadata writes, and no query echo for unavailable search.
- `cargo test -p resume-daemon`: exit 0; identity plus foreground-once lifecycle test passed.

Acceptance:

```bash
cargo run -p resume-cli -- status
cargo run -p resume-cli -- import --root tests/fixtures/empty
cargo run -p resume-cli -- search "Java"
cargo run -p resume-daemon -- run --foreground --once
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `resume-cli status`: exit 0; opened the local metadata store, ran migrations, and printed real aggregate counts plus `search index: unavailable (S4 skeleton: no full-text or vector backend)`.
- `resume-cli import --root tests/fixtures/empty`: exit 0; submitted a persistent `imp_...` import task without creating document or resume rows.
- `resume-cli search "Java"`: exit 0; returned `search index not available yet` and `results: 0`, with no fake result rows.
- `resume-daemon run --foreground --once`: exit 0; opened the metadata store, ran migrations, reported foreground readiness, and exited.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S4 is only a control-plane slice. It does not complete product search, full-text indexing, OCR, embeddings, local IPC, diagnostics, packaging, or cross-platform verification.

### S5

TDD red check:

```bash
cargo test -p fs-crawler
```

Output summary:

- Failed before implementation because `fs-crawler` lacked the S5 scanning, path normalization, filtering, fingerprinting, fake filesystem, and error classification APIs required by the new behavior tests.

Implementation checks:

```bash
cargo test -p fs-crawler
cargo fmt --check
cargo clippy -p fs-crawler --all-targets -- -D warnings
```

Output summary:

- Initial implementation test run surfaced test-side type and borrow errors; after fixing the tests, `cargo test -p fs-crawler` passed with 1 identity test and 6 S5 tests.
- Initial `cargo fmt --check` reported formatting diffs; after `cargo fmt`, `cargo fmt --check` exited 0.
- Initial `cargo clippy -p fs-crawler --all-targets -- -D warnings` reported two sort helpers that should use `sort_by_key`; after updating them, clippy exited 0.

Coverage summary:

- Tests cover Chinese paths, deterministic mixed separator, drive-relative, and UNC normalization, non-UTF-8 path rejection without lossy replacement, same-name files under different normalized paths, temporary/hidden directory/hidden file/unsupported filtering, bounded head/tail quick fingerprint sampling with redacted display/debug, and deterministic fake-filesystem simulation for permission denied, source unavailable, and locked/unreadable states.

Scope note:

- S5 is only a file discovery slice. It does not perform product import execution, document parsing, full-text/vector indexing, OCR, or search-query closure.

### S6

TDD red checks:

```bash
cargo test -p parser-common
cargo test -p parser-docx
cargo test -p parser-pdf
```

Output summary:

- `parser-common` failed before implementation because the parser trait, probe/input/output, budget, support level, and parser error mapping APIs were missing.
- `parser-docx` failed before implementation because `DocxParser` and the shared parser APIs were missing.
- `parser-pdf` failed before implementation because `PdfParser`, shared parser APIs, and the dev test dependency on `core-domain` were missing.

Implementation checks:

```bash
cargo test -p parser-common
cargo test -p parser-docx
cargo test -p parser-pdf
```

Output summary:

- `cargo test -p parser-common`: exit 0; 7 S6 tests passed, covering file probes, support ordering, zero and nonzero timeout mapping, corrupted/OCR_REQUIRED parser error mapping, and redacted parse output debug.
- `cargo test -p parser-docx`: exit 0; 6 S6 tests passed, covering synthetic zip+xml `.docx` paragraph extraction, XML entity unescape, corrupted archive handling, missing `word/document.xml` handling, input byte budget enforcement, and excessive zip entry rejection.
- `cargo test -p parser-pdf`: exit 0; 7 S6 tests passed, covering synthetic text-layer PDF extraction/status, scanned/image PDF `ParseStatus::OcrRequired`, corrupted PDF handling, input byte budget enforcement, runtime timeout enforcement for text-layer and no-text-layer scans, deadline-aware PDF scans, and redacted parse output debug.

Acceptance:

```bash
cargo fmt --check
cargo test -p parser-common
cargo test -p parser-docx
cargo test -p parser-pdf
cargo clippy -p parser-common -p parser-docx -p parser-pdf --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0 after formatting.
- `cargo test -p parser-common`: exit 0; 7 tests passed.
- `cargo test -p parser-docx`: exit 0; 6 tests passed.
- `cargo test -p parser-pdf`: exit 0; 7 tests passed.
- `cargo clippy -p parser-common -p parser-docx -p parser-pdf --all-targets -- -D warnings`: exit 0.

Scope note:

- S6 is only the parser skeleton/docx/PDF text-layer slice. It does not implement OCR execution, indexing, full-text search, text cleaning, extraction, or S7+ behavior.

Additional workspace regression:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Output summary:

- `cargo test --workspace`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.

### S10

TDD red checks:

```bash
cargo test -p extractor-rules --test s10_fields
cargo test -p rank-fusion
cargo test -p resume-cli --test s10_search_filters
```

Output summary:

- `extractor-rules --test s10_fields` failed before implementation because the S10 field variants and extraction behavior were not present.
- `rank-fusion` failed before implementation because the filter, fusion, and candidate-fold APIs were not present.
- `resume-cli --test s10_search_filters` failed before implementation because `resume-cli search` only accepted a bare query and rejected field-filter arguments.

Implementation checks:

```bash
cargo fmt --check
cargo test -p extractor-rules
cargo test -p rank-fusion
cargo test -p resume-cli --test s10_search_filters
cargo test -p resume-cli --test s9_import_search
```

Output summary:

- `cargo fmt --check`: exit 0 after formatting.
- `cargo test -p extractor-rules`: exit 0; S7 coverage plus 2 S10 tests passed, covering school, degree, skill, date-range-derived years, field confidence, original evidence offsets, and Debug redaction.
- `cargo test -p rank-fusion`: exit 0; 4 S10 tests passed, covering degree/skill/year filters, case-insensitive skill matching, candidate fold skeleton, and reciprocal-rank fusion.
- `cargo test -p resume-cli --test s10_search_filters`: exit 0; filtered synthetic search passed for degree, top-k, lower-case skill, and years-experience filters without query-label echo.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; S9 import/search behavior still passed after fixture enrichment.

Acceptance:

```bash
cargo test -p extractor-rules
cargo test -p rank-fusion
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p resume-cli -- import --root tests/fixtures/resumes
cargo run -p resume-cli -- search "Java" --degree bachelor --top-k 20
```

Output summary:

- `cargo test -p extractor-rules`: exit 0.
- `cargo test -p rank-fusion`: exit 0.
- `cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.
- `resume-cli import --root tests/fixtures/resumes`: exit 0; completed an import task for 3 synthetic files, with 2 searchable documents, 1 OCR-required document, 0 failed documents, and 0 scan errors.
- `resume-cli search "Java" --degree bachelor --top-k 20`: exit 0; returned 2 synthetic results, `synthetic-java-platform.pdf` and `synthetic-java-engineer.docx`.

Scope note:

- S10 implements MVP field filtering by overfetching full-text results and filtering in memory. It is not a persistent field index and can miss matches outside the overfetch window.
- Candidate soft dedupe is a pure `rank-fusion` skeleton and is not yet wired into CLI search output.
- S10 does not run OCR, generate embeddings, claim production-scale filtering, or package/release the app.

### S11

TDD red checks:

```bash
cargo test -p embedder
cargo test -p index-vector
cargo test -p rank-fusion
```

Output summary:

- `embedder` failed before implementation because `Embedder`, `EmbeddingInput`, `EmbeddingBudget`, and `DeterministicTestEmbedder` were unresolved.
- `index-vector` failed before implementation because `VectorIndex`, `InMemoryVectorIndex`, `VectorDocument`, and `QueryVector` were unresolved.
- `rank-fusion` failed before implementation because the typed hybrid RRF APIs were unresolved.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p embedder
cargo test -p index-vector
cargo test -p rank-fusion
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p embedder`: exit 0; 2 S11 tests passed, covering the `Embedder` trait, deterministic local test embedder stability, budget rejection, vector dimensions, and text/value Debug redaction.
- `cargo test -p index-vector`: exit 0; 2 S11 tests passed, covering the `VectorIndex` trait, in-memory cosine KNN, deletion marks, snapshots, dimension checks, and vector Debug redaction.
- `cargo test -p rank-fusion`: exit 0; S10 tests plus 2 S11 hybrid RRF tests passed, covering full-text/vector channel fusion, scale-independent RRF, and candidate-key preservation for later folding.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Review notes:

- Sub-agent review confirmed the slice should remain a skeleton only: no model download, no CLI/import pipeline wiring, and no semantic-quality claim.
- The deterministic embedder is explicitly documented as a lexical hash test vectorizer, not a licensed semantic model.

Scope note:

- S11 adds local interfaces and test scaffolding only. It does not download or bundle embedding models, persist vector indexes, wire semantic search into the CLI, or claim production vector-search latency/recall.

### S12

TDD red checks:

```bash
cargo test -p ocr-client
cargo test -p ingest-scheduler
```

Output summary:

- `ocr-client` failed before implementation because the OCR worker client, cache key, rendered page, page request, budget, cancellation, page result, and disabled client APIs were unresolved.
- `ingest-scheduler` failed before implementation because the OCR scheduler, scheduling input, scheduling policy, and scheduling decision APIs were unresolved.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p ocr-client
cargo test -p ingest-scheduler
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p ocr-client`: exit 0; 4 S12 tests passed, covering cache-key validation, page-level OCR result shape, page timeout budget, cancellation priority, disabled-worker behavior, and Debug redaction for image bytes, OCR text, and content hashes.
- `cargo test -p ingest-scheduler`: exit 0; 4 S12 tests passed, covering default OCR-disabled planning, `OCR_REQUIRED` queue membership, enabled page-limit planning, page timeout propagation, cache-key construction, and searchable-document exclusion.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including parser-PDF scanned-file `OcrRequired` regression and S9 import/search regressions.

Review notes:

- Sub-agent review confirmed S12 should not add a database page-task table or wire OCR into import/query yet. `DocumentStatus::OcrRequired` remains the persisted queue membership for this skeleton.
- Known follow-up risk: current document content hash is still the quick fingerprint, not a full-file OCR cache hash. Real OCR worker support must address that before cache correctness claims.
- Known follow-up risk: a real multi-worker OCR queue needs atomic claim semantics and persisted page metadata; this slice only plans in-memory page items.

Scope note:

- S12 adds local OCR client and scheduling interfaces only. It does not call Tesseract/OCRmyPDF, render pages, write OCR cache files, persist page-level OCR tasks, or run OCR from search/import paths.

### S13

TDD red check:

```bash
cargo test -p resume-cli --test s13_diagnostics
```

Output summary:

- The S13 CLI diagnostics test failed before implementation because `resume-cli doctor` and `resume-cli export-diagnostics --redact` were not implemented.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p resume-cli --test s13_diagnostics
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p resume-cli -- doctor
cargo run -p resume-cli -- export-diagnostics --redact
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p resume-cli --test s13_diagnostics`: exit 0; 3 tests passed, covering no-index doctor output, corrupt-index doctor output, redacted diagnostics export, no private path leakage, and no fake P95 benchmark output.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.
- `resume-cli doctor`: exit 0; reported metadata ok, index/searchable/OCR/recovery counts, full-text search-index state, a current-run query smoke result, simulated fault hooks, and diagnostics redaction availability.
- `resume-cli export-diagnostics --redact`: exit 0; emitted a redacted JSON skeleton with aggregate counts, search index state, query smoke status, and simulated fault hook names.

Review notes:

- `doctor` treats a corrupt Tantivy snapshot as a diagnostic state and keeps output path-free.
- Fault hooks are intentionally simulated names only: `daemon_restart`, `index_snapshot_corrupt`, and `disk_space_low`. No process kill or disk-fill command is run in this slice.

Scope note:

- S13 is a small diagnostics and smoke slice. It does not claim production benchmark results, P95 latency, destructive fault injection, complete diagnostic bundles, or release readiness.

### S14

Sub-agent read-only audit:

- Deletion/recovery explorer identified that deleted documents were modeled but not propagated through import rescans, and that default no-filter search trusted the full-text index without metadata visibility hydration.
- Parser/OCR explorer identified OCR handoff as a future high-value slice, but it remains separate because this slice targets the stable-blocking deletion behavior without requiring external OCR/model dependencies.

TDD red checks:

```bash
cargo test -p meta-store mark_document_deleted_sets_tombstone_hides_versions_and_status_counts
cargo test -p resume-cli --test s14_delete_search
```

Output summary:

- `meta-store` failed before implementation because `MetaStore::mark_document_deleted` did not exist.
- `resume-cli --test s14_delete_search` failed before implementation because `resume-cli delete` was not a recognized command.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p meta-store
cargo test -p import-pipeline
cargo test -p resume-cli --test s8_search_cli
cargo test -p resume-cli --test s14_delete_search
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 18 S3 tests passed, including the new soft-delete tombstone and hidden-version test.
- `cargo test -p import-pipeline`: exit 0.
- `cargo test -p resume-cli --test s8_search_cli`: exit 0 after upgrading the test to seed matching synthetic metadata for default visibility hydration.
- `cargo test -p resume-cli --test s14_delete_search`: exit 0; 3 tests passed, covering explicit CLI soft delete, import-rescan deletion propagation, and stale-index metadata filtering.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

CLI smoke:

```bash
cargo run -p resume-cli -- --data-dir "$tmp/data" import --root tests/fixtures/resumes
cargo run -p resume-cli -- --data-dir "$tmp/data" search Java
cargo run -p resume-cli -- --data-dir "$tmp/data" delete --doc-id "$doc_id"
cargo run -p resume-cli -- --data-dir "$tmp/data" search Java
```

Output summary:

- Import completed for 3 synthetic files with 2 searchable documents, 1 OCR-required document, 0 failed documents, and 0 deleted documents.
- Search before delete returned 2 synthetic Java results.
- `delete --doc-id` returned `status: deleted`, `index rebuilt: true`, and `indexed documents: 1`.
- Search after delete returned 1 synthetic Java result and did not return the deleted DOCX fixture.

Scope note:

- S14 implements soft tombstones and default-search metadata visibility filtering for full-text search. Import-rescan deletion propagation only runs after a clean crawl with no scan errors. It does not physically delete user files, cancel OCR/vector work, delete vector-index records, implement staging snapshot pointer swaps, or claim complete audit/retention policy.

### S15

Sub-agent read-only audit:

- The OCR handoff audit recommended reusing the existing `ingest_job` table for a document-level `ocr_document` job instead of adding page-level OCR tasks before a renderer/cache/worker exists.
- The boundary remains explicit: this slice makes scanned/OCR-required PDFs durable and restart-claimable, but it does not execute OCR, generate OCR text, or mark OCR as complete.

TDD red checks:

```bash
cargo test -p meta-store ocr_document_jobs_are_durable_idempotent_and_claimable_by_kind
cargo test -p resume-cli --test s15_ocr_handoff
```

Output summary:

- `meta-store` failed before implementation because `IngestJobKind::OcrDocument`, `MetaStore::enqueue_ocr_job_for_document`, `MetaStore::claim_next_job_by_kind`, and OCR job queue status counts did not exist.
- `resume-cli --test s15_ocr_handoff` failed before implementation because imports could persist `DocumentStatus::OcrRequired` without a durable OCR handoff job.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p meta-store
cargo test -p import-pipeline
cargo test -p resume-cli --test s15_ocr_handoff
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 19 S3 tests passed, including durable, idempotent, kind-filtered claim behavior for `ocr_document` jobs and schema V3 migration coverage.
- `cargo test -p import-pipeline`: exit 0.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 2 tests passed, covering scanned synthetic PDF import into a durable OCR document job, restart claim by kind, no searchable OCR text, and no duplicate OCR job on repeated import.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

CLI smoke:

```bash
cargo run -p resume-cli -- --data-dir "$tmp/data" import --root tests/fixtures/resumes
cargo run -p resume-cli -- --data-dir "$tmp/data" status
cargo run -p resume-cli -- --data-dir "$tmp/data" doctor
cargo run -p resume-cli -- --data-dir "$tmp/data" search scanned --top-k 20
cargo run -p resume-cli -- --data-dir "$tmp/data" export-diagnostics --redact
```

Output summary:

- Import completed for 3 synthetic files with 2 searchable documents, 1 OCR-required document, 1 queued OCR handoff job, 0 failed documents, and 0 scan errors.
- `status` and `doctor` reported `ocr queue: 1` and `ocr jobs queued: 1`.
- Searching `scanned` returned `results: 0`, confirming the scanned fixture was not made searchable without OCR.
- Redacted diagnostics included aggregate `ocr_jobs_queued` and did not include raw paths, queries, or resume text.

Scope note:

- S15 implements a durable document-level OCR handoff queue only. It does not call Tesseract/OCRmyPDF, render PDF pages, persist page-level OCR tasks, write OCR cache files, index OCR output, persist bbox/confidence evidence, or claim worker crash recovery beyond the existing retryable job claim primitive.

### S16

Sub-agent read-only audit:

- The field-search audit confirmed the next highest-value local slice was to move rule field extraction out of the CLI query path and persist extracted evidence as `EntityMention` rows during import.
- The audit also flagged that candidate folding, Tantivy field fast fields, contact hash indexes, and field F1/performance claims must remain out of scope unless separately implemented and verified.

TDD red checks:

```bash
cargo test -p extractor-rules extracts_company_title_and_certificate_with_evidence
cargo test -p meta-store entity_mentions_replace_query_and_redact_values
cargo test -p resume-cli --test s16_persisted_fields
```

Output summary:

- `extractor-rules` failed before implementation because `FieldType::Company`, `FieldType::Title`, and `FieldType::Certificate` did not exist.
- `meta-store` failed before implementation because `EntityMention`, `EntityMentionId`, `EntityType`, `replace_entity_mentions`, `entity_mentions_for_version`, and `StoreStatusSummary::entity_mentions` were not exposed.
- `resume-cli --test s16_persisted_fields` failed before implementation because field filtering depended on re-extracting from `ResumeVersion.clean_text` during search.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p extractor-rules
cargo test -p meta-store
cargo test -p import-pipeline
cargo test -p resume-cli --test s10_search_filters
cargo test -p resume-cli --test s16_persisted_fields
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p extractor-rules`: exit 0; 7 tests passed, including company/title/certificate extraction with span evidence and Debug redaction.
- `cargo test -p meta-store`: exit 0; 20 S3 tests passed, including schema V4 migration, version-scoped mention replacement/query, mention count, and raw value Debug redaction.
- `cargo test -p import-pipeline`: exit 0.
- `cargo test -p resume-cli --test s10_search_filters`: exit 0; existing degree/skill/years filters still pass after moving to persisted mentions.
- `cargo test -p resume-cli --test s16_persisted_fields`: exit 0; filtered search still worked after test code cleared persisted `raw_text` and `clean_text`, proving the filter path reads persisted mentions instead of doing search-time extraction.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

CLI smoke:

```bash
cargo run -p resume-cli -- --data-dir "$tmp/data" import --root tests/fixtures/resumes
cargo run -p resume-cli -- --data-dir "$tmp/data" status
cargo run -p resume-cli -- --data-dir "$tmp/data" search Java --degree bachelor --skills-any java --years-experience-min 4 --top-k 20
cargo run -p resume-cli -- --data-dir "$tmp/data" export-diagnostics --redact
```

Output summary:

- Import completed for 3 synthetic files with 2 searchable documents, 1 OCR-required document, and 1 queued OCR handoff job.
- `status` reported `entity mentions: 16` for the two searchable synthetic resumes.
- Filtered search using degree, skill, and years constraints returned the 2 synthetic Java resumes.
- Redacted diagnostics included aggregate `entity_mentions` only; it did not include field raw values, paths, queries, or resume text.

Scope note:

- S16 persists rule field mentions and removes search-time field extraction from CLI filtering. It does not implement Tantivy fast-field filtering, pre-recall DB/index field filtering, candidate soft dedupe/folding, hashed contact indexes, model-based extraction, field F1 evaluation, or production-scale field latency claims.

### S17

Sub-agent read-only audit:

- The benchmark audit confirmed the next local slice should be a synthetic query benchmark runner, not a 10万/100万 production benchmark or P95 pass claim.
- The audit also flagged that small synthetic runs must keep `target_claim` as `not_evaluated` and `million_scale_verified` as false unless a real large-scale run is actually executed.

TDD red checks:

```bash
cargo test -p benchmark-runner --test s17_benchmark_runner
cargo test -p benchmark-runner --test s17_benchmark_cli
```

Output summary:

- The first benchmark-runner test failed before implementation because `SyntheticBenchmarkConfig` and `run_synthetic_query_benchmark` did not exist.
- The CLI test failed before implementation because no `resume-benchmark` binary existed for Cargo to expose as `CARGO_BIN_EXE_resume-benchmark`.
- After adding initial implementation, the report-field red check failed because `BenchmarkReport` lacked `qps`, `index_size_bytes`, and `percentile_confidence`.

Implementation and acceptance:

```bash
cargo fmt --check
cargo test -p benchmark-runner
cargo clippy -p benchmark-runner --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p benchmark-runner`: exit 0; 3 S17 tests passed, covering synthetic benchmark config validation, real Tantivy-backed query measurements, redacted JSON, and the `resume-benchmark` CLI.
- `cargo clippy -p benchmark-runner --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

CLI smoke:

```bash
cargo run -p benchmark-runner -- synthetic-query --index-dir "$tmp/index" --documents 128 --queries 20 --top-k 10 --json
```

Output summary:

- The command generated a synthetic Tantivy full-text index and emitted redacted JSON with `run_id`, `platform`, `dataset_kind`, document/query counts, build time, query total time, QPS, index size, latency min/mean/P50/P95/P99/max, zero-result count, and total hits.
- The output explicitly included `million_scale_verified:false`, `percentile_confidence:"smoke"`, and `target_claim:"not_evaluated"`.
- The output did not include raw synthetic resume text, raw query text, or the local index path.

Scope note:

- S17 adds a synthetic query benchmark runner only. It does not execute 10万/100万 mixed-corpus benchmarks, does not benchmark OCR or vector recall, does not collect RSS/CPU/disk telemetry, does not verify Windows/macOS benchmark parity, and does not claim any P95 target is met.

### S18

Sub-agent read-only audit:

- The candidate-folding audit confirmed the smallest safe slice is CLI search folding over already assigned `candidate_id` values after metadata hydration.
- The audit also flagged that filtering must happen before folding in filtered search, so a non-matching version cannot hide a matching version for the same candidate.

TDD red check:

```bash
cargo test -p resume-cli --test s18_candidate_folding
```

Output summary:

- The new CLI integration test failed before implementation because default search returned both synthetic versions sharing the same assigned candidate instead of folding to the best version.

Implementation and acceptance:

```bash
cargo test -p resume-cli --test s18_candidate_folding
cargo test -p resume-cli --test s8_search_cli
cargo test -p resume-cli --test s10_search_filters
cargo test -p resume-cli --test s14_delete_search
cargo test -p resume-cli --test s16_persisted_fields
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo test -p resume-cli --test s18_candidate_folding`: exit 0; default and filtered CLI search folded two synthetic versions with the same assigned `candidate_id` to the best search hit while preserving two synthetic documents without `candidate_id` as independent results.
- `cargo test -p resume-cli --test s8_search_cli`: exit 0; existing no-candidate full-text CLI search behavior still passed.
- `cargo test -p resume-cli --test s10_search_filters`: exit 0; persisted field filtering and top-k behavior still passed.
- `cargo test -p resume-cli --test s14_delete_search`: exit 0; soft-deleted and stale-index hits remained hidden.
- `cargo test -p resume-cli --test s16_persisted_fields`: exit 0; filtered search still used persisted entity mentions.
- `cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S18 adds search result folding only for versions that already have an assigned `candidate_id`. It does not implement automatic candidate assignment, contact hashing, candidate soft-dedupe rules, merge confidence, suspected-duplicate hints, or UI/API support for expanding all versions of a candidate.

### S19

Sub-agent read-only audit:

- The candidate-store audit recommended a meta-store-first slice: persist `Candidate`, index already-keyed `ContactHash` values, and expose explicit assignment APIs without deriving hashes from extracted email/phone text.
- The audit recommended not wiring import-pipeline yet because no keyed hashing/key-management boundary exists in the repo.

TDD red checks:

```bash
cargo test -p meta-store candidates_persist_and_are_found_only_by_hashed_contact_material
cargo test -p meta-store explicit_candidate_assignment_requires_existing_candidate
```

Output summary:

- The first candidate persistence test failed before implementation because `meta-store` did not re-export `Candidate`/`ContactHash` and did not expose candidate persistence or contact-hash lookup APIs.
- The explicit assignment test failed before implementation because `MetaStore::assign_candidate_to_version` did not exist.

Implementation and acceptance:

```bash
cargo test -p core-domain contact_hash_only_hydrates_external_keyed_digests
cargo test -p meta-store
cargo test -p import-pipeline
cargo test -p resume-cli --test s16_persisted_fields
cargo test -p resume-cli --test s18_candidate_folding
cargo fmt --check
cargo clippy -p core-domain -p meta-store --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Output summary:

- `cargo test -p core-domain contact_hash_only_hydrates_external_keyed_digests`: exit 0; `ContactHash` still requires external keyed digest material, redacts display/debug, rejects invalid digests, and now canonicalizes uppercase hex to lowercase.
- `cargo test -p meta-store`: exit 0; 24 meta-store tests passed, including schema v5 migration, candidate persistence, contact-hash lookup, unique contact-hash indexes, explicit assignment requiring an existing candidate, hashed-contact assignment reuse, version-count updates, and v1-to-v5 upgrade preservation.
- `cargo test -p import-pipeline`: exit 0; import-pipeline still compiles without automatic candidate assignment.
- `cargo test -p resume-cli --test s16_persisted_fields`: exit 0; persisted field mentions remain the filtering source and no search-time extraction was reintroduced.
- `cargo test -p resume-cli --test s18_candidate_folding`: exit 0; assigned-candidate folding still works with schema v5.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p core-domain -p meta-store --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S19 persists candidate records and supports assignment only from existing `CandidateId` values or already-keyed `ContactHash` values. It deliberately does not derive hashes from `EntityMention` email/phone raw values, does not add import-time automatic candidate assignment, does not implement key storage/rotation, does not enforce a `resume_version.candidate_id` foreign key yet, and does not provide merge review or suspected-duplicate UI.

### S20

Sub-agent read-only audit:

- The IPC audit recommended a loopback-only status endpoint first, exposed as `resume-daemon run --foreground --ipc-listen 127.0.0.1:0`, with stdout printing a machine-readable `ipc status endpoint: http://127.0.0.1:<port>/status` line.
- Review flagged that raw snapshot tokens must not leave the store boundary through IPC; the implementation now exposes only the aggregate boolean `snapshot_present`.
- Review also flagged missing negative-path coverage; tests now cover no SQLite fallback on IPC failures, non-loopback rejection, and wrong-path rejection.
- Follow-up sub-agent review reported no remaining S20 must-fix findings.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
```

Output summary:

- Before the CLI IPC implementation, the new CLI IPC test failed because `resume-cli status --ipc` did not connect to the fake daemon. The test server was then tightened to read complete HTTP headers instead of one partial TCP read.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 3 tests passed, covering loopback `/status` JSON, non-loopback bind rejection, and 404 for non-status IPC paths. The JSON includes aggregate counts plus `snapshot_present`, and test-seeded private snapshot/manifest tokens are not emitted.
- `cargo test -p resume-cli --test s20_status_ipc`: exit 0; 4 tests passed, covering text rendering from daemon IPC, connect failure without SQLite fallback, HTTP error without SQLite fallback, and non-loopback/wrong-path URL rejection.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p resume-cli -p resume-daemon --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including the new S20 daemon and CLI IPC tests.

Scope note:

- S20 completes only a local loopback HTTP/JSON status IPC slice. It does not complete the final IPC target: no gRPC/UDS/Named Pipe transport, authenticated command API, import/search IPC endpoints, daemon service lifecycle integration, Windows IPC validation, or cross-platform IPC packaging is implemented.

### S21

Sub-agent read-only audit:

- The candidate import audit recommended a separate privacy boundary for keyed contact hashing, rather than adding PII hashing to `core-domain`.
- The audit recommended deriving hashes only from normalized email/phone `EntityMention` values, then using the existing `MetaStore::assign_candidate_from_hashed_contacts` API after each resume-version upsert to preserve idempotency across reimports.
- The audit also flagged search snippets as a possible PII path; this slice now redacts email and phone patterns in full-text snippets.
- Follow-up review found two must-fix gaps: compact phone numbers such as `+14155550132` still leaked through snippets, and reimport could clear an existing candidate assignment before reassignment. Both were fixed with targeted regression coverage.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext snippets_redact_contact_values_near_query_matches
```

Output summary:

- Before implementation, `resume-cli --test s21_import_candidate_assignment` failed because no local contact-hash key was created and import did not assign candidates from extracted contacts.
- After import assignment was added, the same test exposed a search snippet leakage path for `Shared.Candidate@Example.Test`; the index-fulltext redaction test failed until snippets redacted email/phone patterns before returning hits.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p privacy
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext snippets_redact_contact_values_near_query_matches
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p import-pipeline -p index-fulltext -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo test -p privacy`: exit 0; 2 tests passed, covering deterministic HMAC contact hashes, lowercase 64-hex digest output, Debug redaction, local key creation, key reload stability, and Unix 0600 key-file permissions.
- `cargo test -p index-fulltext snippets_redact_contact_values_near_query_matches`: exit 0; snippets preserve the query context while replacing email, separated phone, and compact phone values with redaction markers.
- `cargo test -p resume-cli --test s21_import_candidate_assignment`: exit 0; 2 tests passed, covering two synthetic PDFs sharing normalized email/phone importing to the same assigned candidate, durable local key creation under `data_dir/secrets/contact-hash-key-v1`, key/assignment stability across reimport, `version_count` remaining stable, search folding without contact leakage, and preservation of an existing manual candidate assignment on same-version reimport without contacts.
- `cargo test -p resume-cli --test s18_candidate_folding`: exit 0; pre-existing assigned-candidate folding still passes.
- `cargo test -p import-pipeline`: exit 0.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p privacy -p import-pipeline -p index-fulltext -p resume-cli --all-targets -- -D warnings`: exit 0 after replacing a range-loop key decoder with iterator-based decoding.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including the new privacy, snippet redaction, and S21 import assignment tests.

Scope note:

- S21 implements import-time automatic candidate assignment only when normalized email or phone fields are available. It does not encrypt the existing SQLite `entity_mention` raw/normalized fields, rotate or back up contact-hash keys, implement merge-review UX, resolve conflicting multi-contact candidates, add low-confidence duplicate hints, or prove dedupe precision/recall on a real corpus.

### S22

Sub-agent review:

- A read-only explorer recommended S22 as the highest-value local production slice after S21: stop duplicating email/phone plaintext in `entity_mention` while preserving keyed contact assignment.
- Spec review found one blocking gap in the first implementation: future writes were redacted, but existing v5 databases would keep plaintext `entity_mention` contact rows. S22 added schema v6 to rewrite those rows.
- Code-quality review reported no blocking or non-blocking findings after the v6 migration and tests were added.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store contact_entity_mentions_do_not_persist_contact_values
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store schema_v6_redacts_existing_contact_entity_mentions
```

Output summary:

- `contact_entity_mentions_do_not_persist_contact_values` failed before implementation because the hydrated email mention still returned `Sensitive.Candidate@Example.Test` instead of `<redacted:email>`.
- `schema_v6_redacts_existing_contact_entity_mentions` failed before the migration because `run_migrations()` applied no version 6 migration and legacy contact rows kept plaintext values.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 26 tests passed, including direct SQLite assertions that future and legacy email/phone `entity_mention` rows store `<redacted:email>`/`<redacted:phone>` with `normalized_value = NULL` while retaining spans, confidence, extractor, and non-contact fields.
- `cargo test -p resume-cli --test s21_import_candidate_assignment`: exit 0; 2 tests passed, including keyed-contact candidate assignment stability and imported contact mentions hydrating without email/phone plaintext.
- `cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including S22 meta-store migration/redaction coverage and S21 import assignment regression coverage.

Scope note:

- S22 only removes email/phone plaintext duplication from `entity_mention.raw_value` and `entity_mention.normalized_value` for future writes and existing rows reached by schema v6 migration. It does not encrypt SQLite, scrub `resume_version.raw_text`/`clean_text`, remove contact text from the full-text index, prove physical deletion from SQLite free pages or WAL files, implement SQLCipher, rotate or back up contact-hash keys, or complete a full PII audit.

### S23

Sub-agent review:

- A read-only explorer recommended full-text index contact redaction over doctor/key-health work because S22 had already removed one duplicate contact storage path while Tantivy stored fields still accepted raw contact values.
- Spec/quality review found one blocking gap in the first S23 diff: phone redaction missed no-space parenthesized forms like `(415)555-0132` and `+1(415)555-0132`. The regex and stored-field test were expanded to cover those forms.
- The same review noted that stored-field inspection alone is weaker than checking indexed query behavior. The S23 test now also asserts raw email/phone queries do not match after redaction.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext stored_index_fields_redact_contact_values_before_commit
```

Output summary:

- Before implementation, the stored-field test failed because the Tantivy stored text did not contain `<redacted-email>`, proving raw contact values were still written to index fields.
- After the initial implementation, the expanded no-space phone coverage failed because `(415)555-0132` and `+1(415)555-0132` still left `415` in stored fields. The phone regex was tightened and the test then passed.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p index-fulltext`: exit 0; 9 tests passed, including direct Tantivy stored-field assertions for `file_name`, `clean_text`, `all_sections`, and `section_text` contact redaction plus non-contact search preservation and raw contact query non-match behavior.
- `cargo test -p resume-cli --test s21_import_candidate_assignment`: exit 0; 2 tests passed, including import-time keyed-contact assignment, redacted contact mention hydration, folded search results without contact leakage, and raw contact search returning zero results.
- `cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including S23 full-text storage redaction and S21 import assignment regressions.

Scope note:

- S23 redacts email/phone-like contact values only on future full-text index writes for `file_name`, `clean_text`, `all_sections`, and `section_text`, and raw contact queries are not claimed as a supported full-text feature. It does not rewrite already-existing Tantivy segments, encrypt or scrub SQLite `resume_version.raw_text`/`clean_text`, implement hash-based contact search, prove physical deletion from SQLite/Tantivy storage, rotate keys, or complete a full PII audit.

### S24

Sub-agent review:

- A read-only review found one blocking issue in the first S24 implementation: `Path::exists()` could collapse metadata/access errors into `missing`, so unreadable key paths might not be reported as `unreadable`.
- S24 changed the inspector to use `try_exists()` and added Unix-only unreadable coverage at both the privacy and CLI layers.
- The review found no evidence that doctor/export creates keys, repairs permissions, outputs key paths/material, or claims key rotation/backup/SQLCipher/full privacy audit completion.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p privacy contact_hash_key_inspection_is_read_only_and_redacted
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics
```

Output summary:

- Before implementation, the privacy test failed to compile because `inspect_contact_hash_key` and `ContactHashKeyState` did not exist.
- Before CLI integration, `resume-cli --test s13_diagnostics` failed because doctor/export output did not include contact-hash key health and could not report invalid key material.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p privacy
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics
/Users/frankqdwang/.cargo/bin/cargo clippy -p privacy -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p privacy`: exit 0; 4 tests passed, covering read-only missing/invalid/weak-permissions/ready/unreadable key inspection, key material/path redaction in debug output, and no key creation during inspection.
- `cargo test -p resume-cli --test s13_diagnostics`: exit 0; 5 tests passed, covering missing, invalid, and unreadable contact-hash key diagnostics in doctor/export output without path or key-material leakage.
- `cargo clippy -p privacy -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0 on rerun; all workspace tests passed. The first workspace run exposed a transient S20 IPC connection test failure, and a focused rerun of that single S20 test passed before the final full workspace rerun passed.

Scope note:

- S24 adds only read-only contact-hash key health reporting for `missing`, `ready`, `invalid`, `weak_permissions`, and `unreadable` states in doctor/export diagnostics. It does not rotate keys, back up or restore keys, encrypt SQLite, verify all diagnostic package contents, implement SQLCipher, or complete a full PII/security audit.

### S25

Sub-agent review:

- One read-only explorer recommended making the next production slice a real atomic full-text snapshot publish path so failed writes do not destroy the last committed query surface.
- A second read-only explorer recommended adding meta-store `index_health` to doctor/export diagnostics to avoid filesystem-only index-health misreports.
- S25 combines these local, non-external parts: active full-text snapshot publishing, active/legacy read resolution, staging orphan reporting, and redacted metadata index-health diagnostics.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext published_snapshot_becomes_active_without_reading_staging_orphans
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_metadata_index_health_with_active_snapshot
```

Output summary:

- Before implementation, the index-fulltext test failed to compile because `publish_snapshot`, `inspect_snapshot_root`, `SnapshotReadTarget`, `SnapshotRootState`, and `FullTextIndex::open_active` did not exist.
- Before CLI integration, the diagnostics test failed to compile because `publish_snapshot` did not exist and doctor/export did not report meta-store index-health alongside filesystem/Tantivy state.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p index-fulltext`: exit 0; 10 tests passed, including published snapshot activation, active snapshot read resolution, and staging orphan detection while preserving existing full-text behavior.
- `cargo test -p resume-cli --test s13_diagnostics`: exit 0; 6 tests passed, including active snapshot diagnostics, meta-store `index_health`, last-snapshot redaction, read-target reporting, staging orphan count, and no data-dir leakage.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 3 tests passed, covering import-built active snapshots, status/search reopening, recoverable import task reuse, and no live-running task takeover.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p index-fulltext -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Synthetic CLI smoke:

```bash
mktemp -d /tmp/resume-ir-s25-smoke.XXXXXX
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly import --root tests/fixtures/resumes
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly status
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly search Java
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly doctor
/Users/frankqdwang/.cargo/bin/cargo run -p resume-cli -- --data-dir /tmp/resume-ir-s25-smoke.5l8aly export-diagnostics --redact
```

Output summary:

- Import completed against synthetic fixtures: 3 files discovered, 2 searchable documents, 1 OCR-required document, 1 OCR job queued, 0 failed documents, and 0 scan errors.
- Status reported `index health: ready`, a full-text snapshot token, and `search index: available (full-text snapshot)`.
- Search for `Java` returned the 2 synthetic searchable fixtures through the active snapshot.
- Doctor reported `last snapshot: present`, `search index read target: published_snapshot`, `query smoke: ok`, `staging orphans: 0`, and no data-dir path.
- `export-diagnostics --redact` reported `search_index_state: available`, `search_index_read_target: published_snapshot`, `index_health: ready`, `last_snapshot: present`, and `staging_orphans: 0` without raw paths, raw queries, or raw resume text.

Scope note:

- S25 publishes future full-text writes into staging directories, validates them, then switches an active snapshot pointer; search/status/doctor/export now resolve the active snapshot and remain compatible with legacy root indexes. This does not yet implement fallback if the active pointer itself is later corrupted, old snapshot garbage collection, physical purge of old Tantivy segments, vector snapshots, SQLCipher, full disk-full or kill-daemon fault injection, or Windows/macOS atomic rename validation.

### S26

Sub-agent review:

- A read-only explorer confirmed the S25 read path failed hard when `active-snapshot` was invalid, missing, pointed to a missing snapshot directory, or pointed to a corrupt snapshot despite other usable snapshots being present.
- The recommended S26 scope was read-only last-good selection only: enumerate published snapshots, ignore staging, pick the newest usable snapshot, report recovered state in redacted diagnostics, and avoid GC, repair, or retention policy changes.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext active_snapshot_corruption_falls_back_to_last_good_snapshot
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_search_use_last_good_snapshot_after_active_snapshot_corruption
```

Output summary:

- Before implementation, the index-fulltext test failed to compile because `SnapshotRootState::Recovered` and `SnapshotRootInspection::fallback_snapshot()` did not exist.
- Before CLI integration, the diagnostics test failed because search failed instead of falling back when the active snapshot was corrupted.
- The first green attempt exposed that Tantivy could still open a snapshot after a weak metadata corruption; S26 now also checks that `meta.json` has JSON-shaped metadata before considering a snapshot usable.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p index-fulltext`: exit 0; 11 tests passed, including active snapshot corruption falling back to the previous usable published snapshot without reading staging or corrupt active content.
- `cargo test -p resume-cli --test s13_diagnostics`: exit 0; 7 tests passed, including search using last-good fallback, doctor reporting `recovered (full-text snapshot)`, export-diagnostics reporting `search_index_state: recovered`, and no snapshot token/path leakage.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 3 import/search snapshot regressions passed.
- `cargo clippy -p index-fulltext -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S26 adds read-path fallback only. It does not mutate or repair `active-snapshot`, delete corrupt snapshots, clean staging orphans, implement retention/GC, physically purge deleted content from old segments, add vector snapshot fallback, run real disk-full/kill-daemon fault injection, or validate atomic rename semantics on Windows.

### S27

Sub-agent review:

- A read-only explorer confirmed the existing product shape should stay unified around authorized `roots`: specified-directory scanning is the base capability, and whole-disk or large-root discovery is a safer profile over the same root scanning path rather than a separate pipeline.
- Review agents found and drove fixes for discovery-specific risks: an overly narrow system-directory skip list, profile-split task identity, deletion propagation across skipped or unreadable subtrees, duplicate CLI flags, and misplaced import deletion semantics in `fs-crawler`.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler discovery_profile_skips_system_cache_and_dependency_directories
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search discovery_profile_reuses_root_scan_without_deleting_skipped_directories
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search discovery_import_does_not_take_over_live_running_task_for_same_root
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli import_rejects_duplicate_root_and_profile_flags_without_path_leak
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline discovery_deletion_requires_direct_parent_directory_to_be_scanned
```

Output summary:

- Before implementation, the crawler red test failed to compile because `ScanProfile` and `crawl_with_fs_profile` did not exist.
- Before CLI integration, `import --root <path> --profile discovery` failed with the old usage string.
- After the first green path, review red tests exposed that discovery still split task identity by profile, accepted duplicate flags with last-wins behavior, skipped too little at disk roots, and globally disabled deletion instead of applying deletion only to safely traversed directories.
- Final import-pipeline red test showed that using the scanned root as a deletion parent could still delete historical documents under unreadable child directories; S27 now requires a direct scanned parent directory for discovery deletion propagation.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p fs-crawler -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p fs-crawler`: exit 0; 7 crawler tests passed, including discovery skipping root-level system directories and dependency/cache directories while preserving nested business directories such as `Target`.
- `cargo test -p import-pipeline`: exit 0; 2 import-pipeline tests passed, including discovery deletion requiring a directly scanned parent directory and excluding skipped subtrees.
- `cargo test -p resume-cli`: exit 0; 30 CLI tests passed, including discovery import, duplicate flag rejection, same-root running-task protection across profiles, and discovery reimport preserving skipped subtree documents while deleting missing documents from traversed directories.
- `cargo clippy -p fs-crawler -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S27 keeps `resume-cli import --root <path>` as the unified import path and adds `--profile discovery` for local large-root discovery. It does not add automatic whole-machine root selection, multi-root CLI/UI, scan progress/cancel, time/file-count/IO budgets, real local resume witness runs, follow-symlink traversal, persisted scan-profile metadata, or cross-platform validation of root exclusions.

### S28

Sub-agent review:

- A read-only explorer recommended keeping `ImportTask.root_path` as a single canonical root and implementing multi-root import as CLI batching over existing per-root tasks, rather than storing a composite root key.
- A final read-only reviewer confirmed the implemented S28 path uses one existing `ImportTask` per canonical root, preserves running/retryable task behavior, rejects duplicate/overlapping roots without path leakage, and keeps deletion propagation isolated to each single-root import.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_multiple_roots_builds_searchable_index_without_path_leak
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli import_rejects_overlapping_roots_without_path_leak
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search multi_root_reimport_marks_missing_files_deleted_per_root
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search multi_root_import_does_not_take_over_live_running_task_for_any_root
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search multi_root_import_reuses_recoverable_task_for_each_root
```

Output summary:

- Before implementation, multi-root import tests failed because `resume-cli import` still rejected a second `--root` with the old usage path.
- After the first implementation, review red tests showed the composite multi-root task key bypassed per-root running and retryable task semantics.
- S28 now validates canonical roots as distinct and non-overlapping, then executes each root through its own existing `ImportTask` and merges the user-facing summary without printing requested or canonical paths.

Implementation and acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p import-pipeline`: exit 0; 2 import-pipeline tests passed.
- `cargo test -p resume-cli --test s4_cli`: exit 0; 5 CLI base tests passed, including duplicate and overlapping root rejection without path leakage.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 7 import/search tests passed, including multi-root import, multi-root running-task refusal, and per-root retryable task reuse.
- `cargo test -p resume-cli --test s14_delete_search`: exit 0; 5 delete/search tests passed, including multi-root reimport tombstoning a missing file in one root without hiding the other root.
- `cargo test -p resume-cli`: exit 0; 34 CLI tests passed.
- `cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S28 adds multi-root CLI import over existing per-root task semantics. It does not add automatic root presets, persisted scan-scope records, progress/cancel, per-root partial-failure reporting beyond the merged summary, a true all-or-nothing multi-root transaction, real local resume witness runs, or cross-platform validation of Windows/macOS root overlap behavior.

### S29

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client
```

Output summary:

- Before implementation, `ocr-client` failed because `LocalOcrCommandClient`, `LocalOcrCommandSpec`, dynamic `CancellationToken::cancel`, and `OcrErrorKind::EngineFailed` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client
/Users/frankqdwang/.cargo/bin/cargo test -p ingest-scheduler
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p ingest-scheduler --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p ocr-client`: exit 0; 14 OCR client tests passed, covering disabled mode, redacted debug output, local command execution, structured stdout with confidence, missing binary as `WorkerUnavailable`, timeout, running cancellation, descendant process cleanup, owner-only input files, CRLF schema output, non-schema output rejection, out-of-range confidence rejection, and malformed engine output as `EngineFailed`.
- `cargo test -p ingest-scheduler`: exit 0; 4 ingest scheduler tests passed after the cancellation token became dynamically cancellable.
- `cargo clippy -p ocr-client -p ingest-scheduler --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `git diff --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed, including the 14 OCR client tests.

Sub-agent review:

- A read-only S29 reviewer found three pre-commit blockers: timeout/cancel did not terminate OCR descendant processes that kept inherited pipes open, the temporary rendered-page input file used default permissions, and non-structured stdout was accepted as successful OCR output. S29 now starts Unix OCR commands in a new process group and terminates that group on timeout/cancel/direct-child exit, creates a private temp directory plus `0600` input file on Unix, and requires the `resume-ir-ocr-v1` structured stdout schema with valid confidence.

Scope note:

- S29 adds a production local command OCR client that launches a configured local executable, passes rendered page bytes through a private temporary local input file, supplies page/options via environment variables, parses only `resume-ir-ocr-v1` stdout with valid confidence and text, enforces page timeout, kills on cancellation, terminates Unix descendant processes in the OCR process group, and redacts debug/error surfaces. It does not bundle or license a concrete OCR engine, render PDF pages into images, persist OCR page cache/results, connect the durable OCR queue to this client, index OCR text, persist bbox evidence, run a real scanned-resume witness, implement Windows job-object process-tree termination, or validate Windows command execution.

### S30

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store ocr_page_cache
```

Output summary:

- Before implementation, `meta-store` failed because `OcrPageCacheKey`, `OcrPageCacheEntry`, `OcrPageCacheStatus`, `MetaStore::upsert_ocr_page_cache_entry`, and `MetaStore::ocr_page_cache_entry` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 28 meta-store tests passed, including V7 migration creation, OCR page cache success/failure upsert, redacted Debug output, key lookup, and invalid key/confidence rejection.
- `cargo clippy -p meta-store --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `git diff --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S30 adds a V7 SQLite OCR page cache table plus redacted key/result APIs for success and retryable/permanent failures. It does not connect the cache to real OCR execution, render PDF pages, store bbox evidence, index OCR output, run a scanned-resume witness, implement cache GC/retention, or encrypt/purge the cached OCR text beyond existing local SQLite behavior.

### S31

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
```

Output summary:

- Before implementation, `resume-cli` failed because `ocr-worker` was not a recognized command and no CLI path claimed `OcrDocument` jobs for local command OCR execution.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 4 OCR handoff tests passed, including the blocked no-command worker path and the local command cache-write path.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S31 adds `resume-cli ocr-worker --once --command <path>` to claim one durable `OcrDocument` job, invoke a configured local OCR command, persist a page-1 OCR cache entry, and complete the OCR job/document without printing raw OCR text or paths. The no-command path reports a blocked worker and leaves the queued job untouched. This slice passes local source-document bytes to the command-wrapper input; it still does not render PDF pages, split multi-page documents, index OCR text into search, persist bounding boxes, run the daemon OCR loop, install or license a concrete OCR engine, run a real scanned-resume witness, or validate Windows process-tree cleanup.

### S32

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak
```

Output summary:

- Before implementation, `resume-cli import --root-preset local-discovery` failed with the import usage message because the CLI only accepted explicit `--root` values.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p resume-cli --test s4_cli`: exit 0; 5 CLI usage/status/import tests passed, including rejection of mixed `--root` and `--root-preset`.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 8 import/search tests passed, including `--root-preset local-discovery` with a synthetic env-overridden root, discovery-profile skipping of dependency directories, path redaction, and searchability of the discovered synthetic PDF.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Sub-agent review:

- A read-only explorer recommended modeling whole-machine/local discovery as a root-selection preset rather than a new crawler. S32 follows that by adding `--root-preset local-discovery`, keeping `--root` and preset selection mutually exclusive, defaulting the preset to `ScanProfile::Discovery`, and still using existing canonical root validation plus `import_root_with_options`.

Scope note:

- S32 adds a root preset layer over the existing explicit-root import path. On non-Windows hosts the default local-discovery root set starts at `/`; on Windows it enumerates available drive roots, and tests use the local `RESUME_IR_LOCAL_DISCOVERY_ROOTS` override to avoid reading real user files. This does not prove that the product can find every resume on a real machine, does not add progress/cancel/budget controls, does not persist scan-scope metadata, does not implement explicit real-data confirmation UX, does not run a real local witness scan, and does not validate Windows drive enumeration in a Windows environment.

### S33

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store worker_task_control_defaults_to_running_and_persists_pause_state
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff pause_and_resume_ocr_task_persistently_controls_worker_claims
```

Output summary:

- Before implementation, `meta-store` failed because `WorkerTaskControl`, `WorkerTaskKind`, `MetaStore::worker_task_control`, and `MetaStore::set_worker_task_paused` did not exist.
- Before implementation, the CLI test failed because `resume-cli pause --task ocr` was not implemented.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 29 meta-store tests passed, including V8 migration creation, file-backed pause-state persistence, default running state, resume-state update, and legacy V1 upgrade through V8.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 5 OCR handoff tests passed, including pause/resume control preventing `ocr-worker` from claiming queued OCR jobs while paused and allowing claim after resume.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p meta-store -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S33 adds SQLite schema V8 `worker_task_control`, `resume-cli pause --task ocr`, `resume-cli resume --task ocr`, status reporting for `ocr task`, and an `ocr-worker` pre-claim pause gate that returns without consuming queued jobs. It does not interrupt an OCR process that is already running, does not add daemon-loop orchestration, does not render PDF pages, does not bundle or license a concrete OCR engine, does not index OCR output into search, does not persist OCR bounding boxes, does not run a real scanned-resume witness, and does not validate Windows process-control behavior.

### S34

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p embedder
```

Output summary:

- Before implementation, `embedder` failed because `LocalEmbeddingCommandSpec`, `LocalEmbeddingCommandEmbedder`, and the command-execution error variants were unresolved.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p embedder
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector
/Users/frankqdwang/.cargo/bin/cargo clippy -p embedder -p index-vector --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p embedder`: exit 0; 5 embedder tests passed, including configured local command execution, structured vector parsing, missing-worker classification, malformed-output rejection without payload leakage, timeout handling, and private input-file permissions.
- `cargo test -p index-vector`: exit 0; 2 vector-index tests passed.
- `cargo clippy -p embedder -p index-vector --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S34 adds a structured local embedding command client that writes a private local input file, invokes a configured local executable, parses the `resume-ir-embedding-v1` stdout protocol, validates model/dimension/output shape, times out stalled workers, and redacts payloads from errors/debug output. It does not select, bundle, license, download, or install a concrete embedding model; the deterministic embedder remains test-only scaffolding, `index-vector` remains in-memory, and product semantic/hybrid search is still not complete.

### S35

Sub-agent note:

- A read-only explorer confirmed the next scan/import slice should persist scan-scope metadata before implementing progress or cancel/budget controls, because progress and cancellation need a durable scan-scope object to attach to.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_scan_scope_persists_root_profile_and_redacted_progress_counts
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak
```

Output summary:

- Before implementation, `meta-store` failed because `ImportScanScope`, `ImportRootKind`, `ImportRootPreset`, `ImportScanProfile`, `MetaStore::upsert_import_scan_scope`, `MetaStore::import_scan_scope_by_task_id`, `MetaStore::latest_import_scan_scope`, and `StoreStatusSummary::import_scan_scopes` did not exist.
- Before implementation, the CLI test failed because `latest_import_scan_scope` and the scan-scope enum types did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_ipc_connect_failure_does_not_fallback_to_sqlite -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p embedder
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0 after formatting.
- `git diff --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 30 meta-store tests passed, including V9 `import_scan_scope` migration, V1-to-V9 upgrade, scope persistence/reopen, redacted Debug output, and status-summary counts.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 8 import/search tests passed, including local-discovery preset scope persistence without stdout/path leakage.
- `cargo test -p resume-cli --test s14_delete_search`: exit 0; 5 delete/search regression tests passed.
- `cargo test -p resume-cli --test s20_status_ipc status_ipc_connect_failure_does_not_fallback_to_sqlite -- --exact`: exit 0 after an earlier full-file run hit a transient port collision in the negative IPC test.
- `cargo test -p embedder`: exit 0; 5 embedder tests passed after hardening the timeout test to record private input-file permissions before sleeping.
- `cargo test -p resume-cli`: exit 0; all CLI integration tests passed.
- `cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S35 adds SQLite schema V9 `import_scan_scope`, typed scan-scope APIs, redacted scan-scope Debug output, CLI import writes for explicit roots and `local-discovery` preset roots, persisted summary counts, and status/doctor/diagnostics/daemon status counters. It does not implement live progress streaming, import cancellation, scan budget enforcement, per-file error UX, encrypted path metadata, a real whole-machine witness scan, or Windows root validation.

### S36

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler scan_options_stop_after_file_budget_without_path_leakage
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_scan_scope_persists_root_profile_and_redacted_progress_counts
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_max_files_limits_scan_and_persists_budget_state_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search budgeted_reimport_does_not_mark_unscanned_missing_files_deleted -- --exact
```

Output summary:

- Before implementation, `fs-crawler` failed because `ScanOptions`, `ScanBudgetKind`, and `crawl_with_fs_options` did not exist.
- Before implementation, `meta-store` and CLI scan-scope tests failed because `ImportScanBudgetKind` and scan budget fields were missing.
- Before implementation, the budgeted reimport CLI test failed because `resume-cli import --max-files` was rejected by usage parsing.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p fs-crawler -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p fs-crawler`: exit 0; 8 crawler tests passed, including deterministic file-budget stop and redacted budget Debug output.
- `cargo test -p meta-store`: exit 0; 30 meta-store tests passed, including V10 scan budget fields on `import_scan_scope` and V1-to-V10 upgrade.
- `cargo test -p import-pipeline`: exit 0; import-pipeline unit tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI integration tests passed, including `--max-files`, persisted budget state, and no deletion propagation on budgeted partial reimport.
- `cargo clippy -p fs-crawler -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S36 adds `ScanOptions::max_files`, `resume-cli import --max-files <count>`, scan budget reporting, SQLite schema V10 scan-budget columns on `import_scan_scope`, and disables missing-file deletion propagation when a scan is budget-exhausted. It does not implement live progress streaming, user-triggered cancellation, time/byte/CPU budgets, persisted per-file scan errors, real whole-machine witness scans, encrypted path metadata, or cross-platform full-disk validation.

### S39

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
```

Output summary:

- Before implementation, the new CLI tests failed because `resume-cli` did not recognize `embed-worker`; the expected blocked/no-command behavior and local vector snapshot persistence were absent.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli embed_worker_debug_output_redacts_candidate_text_and_command_path
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo test -p embedder
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p embedder -p index-vector --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p resume-cli embed_worker_debug_output_redacts_candidate_text_and_command_path`: exit 0; the new CLI unit test passed and confirms `EmbedWorkerCandidate` redacts resume text and `EmbedWorkerArgs` redacts the configured command path from Debug output.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 2 tests passed, covering blocked operation without a local embedding command and local command execution that writes 2 synthetic searchable resume vectors to the persistent vector snapshot without leaking paths or hiding full-text search results.
- `cargo clippy -p resume-cli --all-targets -- -D warnings`: exit 0.
- `cargo test -p resume-cli`: exit 0; all CLI integration tests passed.
- `cargo test -p embedder`: exit 0; 5 embedder tests passed.
- `cargo test -p index-vector`: exit 0; 4 vector-index tests passed.
- `cargo clippy -p resume-cli -p embedder -p index-vector --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S39 adds `resume-cli embed-worker --once`, an explicit local embedding command requirement, model/dimension/budget/timeout parsing, visible searchable resume-version candidate selection, local command execution through the S34 embedder protocol, persistent local vector snapshot writes, and redacted Debug output for embedding worker candidates/args. It does not choose, bundle, license, download, or install a concrete embedding model; the configured command is trusted to be local/offline and OS-enforced no-network sandboxing is not yet implemented. It does not add daemon-loop embedding, semantic/hybrid query execution, vector snapshot GC/repair, real-data validation, or cross-platform command validation.

### S40

Design note:

- Whole-machine scanning remains a root-selection case over the existing import scanner. This slice does not add a second scanning pipeline; it makes the existing `local-discovery` preset safer by adding a default file-count budget that explicit roots do not inherit and that users can override with `--max-files`.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak -- --exact
```

Output summary:

- Before implementation, the focused CLI test failed because `--root-preset local-discovery` printed `scan file limit: none` and did not persist budget metadata when the default discovery scan was not exhausted.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_scan_scope_persists_root_profile_and_redacted_progress_counts -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_allows_explicit_file_budget_override_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search explicit_root_import_without_max_files_has_no_default_scan_budget -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search multi_root_import_reports_budget_exhausted_when_later_root_hits_file_limit -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p meta-store import_scan_scope_persists_root_profile_and_redacted_progress_counts -- --exact`: exit 0; the scan-scope test now covers configured but not exhausted file budgets.
- `cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_uses_discovery_profile_without_path_leak -- --exact`: exit 0; the preset now reports `scan file limit: 10000`, persists the non-exhausted file budget, and does not leak local roots.
- `cargo test -p resume-cli --test s9_import_search local_discovery_root_preset_allows_explicit_file_budget_override_without_path_leak -- --exact`: exit 0; explicit `--max-files 1` still overrides the preset default and records an exhausted budget without path leakage.
- `cargo test -p resume-cli --test s9_import_search explicit_root_import_without_max_files_has_no_default_scan_budget -- --exact`: exit 0; explicit roots without `--max-files` still report `scan file limit: none` and persist no scan budget.
- `cargo test -p resume-cli --test s9_import_search multi_root_import_reports_budget_exhausted_when_later_root_hits_file_limit -- --exact`: exit 0 after a sub-agent review found and the implementation fixed aggregate multi-root budget reporting when a later root exhausts the file limit.
- `cargo test -p meta-store`: exit 0; 31 meta-store tests passed.
- `cargo test -p import-pipeline`: exit 0; 2 import-pipeline tests passed.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 13 import/search tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p meta-store -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Scope note:

- S40 sets `local-discovery` to a default 10,000 file budget, keeps explicit `--root` imports unbudgeted unless `--max-files` is supplied, allows user override of the preset budget, persists configured file-budget metadata even when the scan is not exhausted, and reports aggregate multi-root budget exhaustion if any root exhausts the file limit. It does not add progress streaming, user cancellation, time/byte/CPU budgets, a UI for partial results, real whole-machine witness scans, encrypted path metadata, or Windows/macOS full-disk validation.

### S41

Design note:

- Successful OCR output is now part of the same local import/index pipeline as text-layer documents: normalize text, persist a searchable OCR resume version, refresh rule-extracted fields/candidate assignment, mark the document `Searchable`, and rebuild the active full-text snapshot. Whole-machine scanning remains a root-selection case over the existing scanner; explicit directory scanning is retained, and selecting `/`, `/Users`, `C:\`, or `D:\` should use the same scanner with stronger defaults and user-facing guardrails rather than a separate pipeline.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_executes_local_command_persists_cache_and_indexes_searchable_text -- --exact
```

Output summary:

- Before implementation, the focused OCR worker test failed because a successful local OCR command left the scanned document in `OcrDone` and searching the OCR-only token returned `results: 0`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_executes_local_command_persists_cache_and_indexes_searchable_text -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff pause_and_resume_ocr_task_persistently_controls_worker_claims -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p import-pipeline`: exit 0; import-pipeline unit tests passed.
- `cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_executes_local_command_persists_cache_and_indexes_searchable_text -- --exact`: exit 0; the local OCR command writes the page cache, marks the scanned document searchable, and searching the OCR-only token returns one redacted result without leaking local data or fixture paths.
- `cargo test -p resume-cli --test s15_ocr_handoff pause_and_resume_ocr_task_persistently_controls_worker_claims -- --exact`: exit 0; pause/resume still controls worker claims and the eventual successful OCR output becomes searchable.
- `cargo test -p resume-cli --test s15_ocr_handoff`: exit 0; 7 OCR handoff tests passed, including direct cache-hit indexing and empty OCR text staying non-searchable.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p import-pipeline -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Sub-agent review fix:

- Newton found two P2 issues before commit: `index_ocr_text` promoted the document to `Searchable` before full-text snapshot publish, and tests did not directly cover OCR cache-hit indexing or empty OCR text. The implementation now writes the rebuilt full-text snapshot with pending OCR documents first, then promotes document status and index state after publish succeeds. The S15 handoff suite now covers command success, cache-hit success without invoking the command, and empty successful OCR text remaining `OcrDone` with no searchable version.

Scope note:

- S41 adds `import_pipeline::index_ocr_text`, connects OCR worker cache-hit and command-success paths to OCR text indexing, keeps empty OCR text non-searchable as `OcrDone`, persists OCR text in local SQLite resume versions, reuses existing rule extraction/contact-hash assignment, and rebuilds the full-text index after OCR completion. It does not render multi-page PDF pages, run OCR from the daemon loop, choose/install/license a concrete OCR engine, persist bounding boxes, prove behavior on real scanned resumes, encrypt OCR text at rest, physically purge SQLite/WAL data, or validate Windows process-tree behavior.

### S42

Design note:

- S42 completes the local P3 query loop that remained after the embedding worker slice: `resume-cli search --mode semantic` embeds the query through an explicit local command, opens the persisted vector snapshot, performs KNN, hydrates visible documents from SQLite, applies persisted field filters, folds candidates, and prints redacted output. `--mode hybrid` combines full-text and vector channels with existing RRF. Full-text search remains the default and does not create metadata when the full-text index is missing.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker semantic_and_hybrid_search_use_persistent_vector_snapshot_with_local_query_embedding -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker semantic_search_reports_missing_vector_snapshot_even_when_dimension_is_supplied -- --exact
```

Output summary:

- Before implementation, `semantic_and_hybrid_search_use_persistent_vector_snapshot_with_local_query_embedding` failed because `resume-cli search` did not accept `--mode` or any query embedding/vector options.
- Before the missing-snapshot fix, `semantic_search_reports_missing_vector_snapshot_even_when_dimension_is_supplied` failed because semantic search succeeded against an implicitly created empty vector index when `--dimension` was supplied.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext -p rank-fusion -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p resume-cli --test s39_embedding_worker`: exit 0; 4 embedding/semantic tests passed, covering local command query embedding, semantic search over the persisted vector snapshot, hybrid RRF search, missing command behavior, and missing vector snapshot behavior without query/path leakage.
- `cargo test -p index-fulltext`: exit 0; 11 full-text tests passed.
- `cargo test -p rank-fusion`: exit 0; 6 rank-fusion tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p index-fulltext -p rank-fusion -p resume-cli --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Sub-agent review:

- Kant completed a read-only P0-P6 audit and identified P3 semantic/hybrid closure as the highest-value local production feature after restoring build health. It did not edit files. Its stale source snapshot saw missing search symbols before this implementation; the final local verification above covers the corrected state.

Scope note:

- S42 does not choose, install, license, or distribute a production embedding model; it does not add ONNX/HNSW/FAISS or another ANN engine; it does not run a daemon embedding queue, section-level vectors, OS-enforced no-network sandboxing for user embedding commands, real semantic quality benchmarks, real resume witness scans, or cross-platform validation. Those remain incomplete or BLOCKED.

### S43

Design note:

- S43 moves import execution closer to the daemon-owned production control plane. `resume-cli import --enqueue` now persists queued import tasks and scan scope metadata without doing foreground indexing. `resume-daemon run --foreground --once --work-imports-once` claims queued/retryable import tasks from SQLite, reconstructs scan options from persisted scope, runs the existing real import/index pipeline, records updated scan counts, and continues past retryable failures in the same worker pass.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_worker_claim_atomically_marks_next_task_running_and_skips_attempted_tasks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search import_enqueue_persists_task_without_running_foreground_import -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_once_worker_processes_queued_import_task_from_persistent_scope -- --exact
```

Output summary:

- Before implementation, the meta-store test failed because there was no atomic import-task claim API for daemon workers.
- Before implementation, the CLI enqueue test failed because `resume-cli import` did not accept `--enqueue`.
- Before implementation, the daemon worker test failed because `resume-daemon run` did not accept `--work-imports-once`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-cli -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo test -p meta-store`: exit 0; 32 meta-store tests passed, including atomic import worker claim and attempted-task exclusion.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 14 import/search tests passed, including enqueue without foreground import and preserved scan budget metadata.
- `cargo test -p resume-daemon`: exit 0; daemon identity, IPC, foreground once, queued import worker, and failure-continuation tests passed.
- `cargo test -p resume-cli`: exit 0; all CLI tests passed.
- `cargo clippy -p meta-store -p import-pipeline -p resume-cli -p resume-daemon --all-targets -- -D warnings`: exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests passed.

Sub-agent review fix:

- Feynman found three issues before commit: queued imports dropped scan-budget metadata, worker task selection was not atomic, and a single import failure aborted the worker pass. The implementation now persists initial budget metadata, uses an atomic SQLite `UPDATE ... RETURNING` claim API, excludes attempted task IDs during a one-shot worker pass to avoid immediate retry loops, and counts failures while continuing to later queued tasks.

Scope note:

- S43 does not add a long-running scheduler loop, authenticated import command IPC, progress streaming, cancellation, background OCR/vector workers, multi-process stress proof, real whole-machine witness scans, or Windows/macOS service validation. Those remain incomplete.

### S44

Design note:

- S44 adds `resume-daemon run --foreground --work-imports` as a long-running
  local import scheduler. It polls queued import tasks after startup, keeps
  new queued tasks immediately claimable, applies a fixed retry backoff to
  retryable failures so bad roots are not hot-looped, records terminal task
  status at import finish time, heartbeats active `Running` import tasks, and
  recovers stale `Running` import tasks to retryable after a daemon crash/stall
  window.

TDD red checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_processes_task_enqueued_after_startup -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_worker_claim_respects_retryable_due_time_without_delaying_queued_tasks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store stale_running_import_tasks_can_be_recovered_for_worker_retry -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store running_import_task_heartbeat_prevents_stale_recovery -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_backs_off_retryable_failures -- --exact
```

Output summary:

- Before implementation, the scheduler test failed because `resume-daemon run`
  did not accept `--work-imports`.
- Before implementation, the retry-due and stale-running meta-store tests
  failed because the worker claim API had no retryable due cutoff and there was
  no stale running import recovery API.
- Before implementation, the running-task heartbeat test failed because there
  was no worker heartbeat API to keep active long imports out of stale recovery.
- Before the backoff fix, the bad-root scheduler test failed with 30 retryable
  failures across 30 worker ticks instead of one failure followed by backoff.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_worker_claim_respects_retryable_due_time_without_delaying_queued_tasks -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store stale_running_import_tasks_can_be_recovered_for_worker_retry -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store running_import_task_heartbeat_prevents_stale_recovery -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_backs_off_retryable_failures -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_recovers_stale_running_import_task -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_processes_task_enqueued_after_startup -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
```

Output summary:

- The retry-due meta-store test passed and confirms retryable task due time no
  longer delays fresh queued work.
- The stale-running meta-store test passed and confirms stale `Running` import
  tasks can be moved to `FailedRetryable` with finished/updated timestamps.
- The running-task heartbeat meta-store test passed and confirms active
  `Running` imports can refresh `updated_at` to avoid stale recovery.
- The scheduler backoff test passed and confirms a missing root produces one
  retryable failure across 30 short ticks, without leaking local paths.
- The scheduler stale recovery test passed and confirms daemon loop recovery
  emits only redacted counts and leaves the task retryable instead of stuck
  running.
- `cargo test -p import-pipeline`: exit 0; import-pipeline tests passed after
  terminal import-task timestamps were moved to finish time.
- `cargo test -p meta-store`: exit 0; 35 meta-store tests passed.
- `cargo test -p resume-daemon`: exit 0; daemon identity, IPC, one-shot worker,
  long-running scheduler, retry backoff, and stale recovery tests passed.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p import-pipeline -p resume-daemon --all-targets -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo clippy -p meta-store -p import-pipeline -p resume-daemon --all-targets -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests and doc-tests passed.

Sub-agent review fix:

- Bernoulli/Cicero found a P1 hot retry loop: `--work-imports` reset the
  attempted set every tick, so a retryable bad root could be retried forever at
  the worker interval. The implementation now separates queued eligibility
  from retryable due time and applies a fixed 60-second daemon retry backoff.
- Cicero also found a P1 stale-running lifecycle gap after daemon crash. The
  implementation now heartbeats active running import tasks and recovers stale
  running import tasks after a 15-minute no-heartbeat window in the
  long-running worker loop. Cicero's P3 child-cleanup test issue was fixed by
  killing/waiting the daemon child when readiness never appears.
- Halley found that retry backoff was still measured from import start time for
  long failed imports, and that stale recovery could steal an active long import
  without a worker lease. Terminal task status is now stamped at import finish
  time, and active daemon imports now refresh a running-task heartbeat before
  they can be considered stale.

Scope note:

- S44 does not combine the IPC status server with the worker loop, add an
  authenticated import command IPC endpoint, stream import progress, implement
  user cancellation, make retry policy configurable, enforce a packaged
  singleton service lifecycle, run OCR/vector workers, execute real
  whole-machine witness scans, or validate macOS/Windows service lifecycle
  behavior. Those remain incomplete.

### S45

Design note:

- S45 removes the staged daemon restriction that forced status IPC and the
  import worker loop to run separately. `resume-daemon run --foreground
  --work-imports --ipc-listen 127.0.0.1:0` now starts the import worker on a
  separate local metadata connection while the main thread serves loopback
  `/status`. Test hooks still use `--max-requests` and `--max-worker-ticks` for
  deterministic shutdown; production mode keeps both loops running.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_status_while_import_worker_processes_late_queued_task -- --exact
```

Output summary:

- Before implementation, the daemon did not print an IPC endpoint because
  `--work-imports --ipc-listen` was rejected by argument validation.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_serves_status_while_import_worker_processes_late_queued_task -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_does_not_start_import_worker_when_ipc_bind_fails -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_worker_tick_limit_in_combined_ipc_worker_mode -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- The focused S45 test passed and confirms a task queued after daemon startup
  is processed by the worker while status IPC remains available and reports
  `searchable_documents: 2` without leaking local paths.
- The bind-failure test passed and confirms the import worker is not started
  before IPC bind succeeds, so a failed combined daemon startup leaves queued
  tasks untouched.
- The combined-mode tick-limit test passed and confirms `--max-worker-ticks` is
  rejected with IPC to avoid a worker exiting while `/status` still reports
  healthy service.
- `cargo test -p resume-daemon`: exit 0; identity, status IPC, standalone
  worker, combined IPC plus import worker, bind failure, and combined-mode
  validation tests passed.
- `cargo clippy -p resume-daemon --all-targets -- -D warnings`: exit 0.

Sub-agent review fix:

- Gauss found a P1 where the worker could exit while IPC continued serving
  healthy status. The combined IPC loop now monitors the worker result channel
  and returns an error if the worker exits while IPC is still running; test-only
  `--max-worker-ticks` is rejected in combined mode.
- Gauss found a P2 where the worker started before IPC bind succeeded. The
  daemon now binds and prints the IPC endpoint before spawning the worker, and
  the bind-failure test confirms queued imports remain untouched.
- Gauss found P3 test cleanup and post-import leakage proof gaps. Endpoint
  readiness failure now kills/waits the child, and the S45 test checks both the
  initial and post-import status responses for local path leakage.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests and doc-tests passed,
  including S45 combined daemon IPC plus import worker coverage.

Scope note:

- S45 does not add authenticated command IPC endpoints, import cancellation,
  progress streaming, configurable retry policy, packaged singleton service
  enforcement, daemon OCR/vector workers, real whole-machine witness scans, or
  macOS/Windows service validation. Those remain incomplete.

### S46

Design target:

- S46 adds the first authenticated local command IPC surface for import
  enqueue. This closes part of the P0 control-plane gap by allowing local
  agents/UI callers to submit explicit import roots through the daemon instead
  of writing SQLite internals directly.
- The endpoint remains loopback-only, uses a locally generated bearer token
  stored under the data directory, keeps responses path/token-redacted, and
  only queues import tasks plus initial scan-scope metadata. It does not run
  OCR/vector workers or claim product completion.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_authenticates_and_queues_import_command_over_ipc -- --exact
```

Output summary:

- Failed before implementation because the daemon did not create
  `ipc.auth` and did not expose an authenticated import command IPC endpoint:
  the test panicked while reading the missing token file.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_authenticates_and_queues_import_command_over_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_requires_bearer_token_for_import_command_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store import_task_and_scan_scope_insert_atomically_for_daemon_command_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_malformed_ipc_request_without_stopping -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_import_command_for_running_root_without_rewriting_scope -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_import_command_ipc_feeds_running_import_worker_loop -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_rejects_wrong_bearer_token_for_import_command_ipc -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc daemon_repairs_existing_weak_ipc_token_permissions -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-daemon --all-targets -- -D warnings
```

Output summary:

- The authorized `POST /imports` test passed: a request with the local bearer
  token returns `202 Accepted`, creates one queued import task, persists an
  explicit scan scope with file budget metadata, and omits the data directory,
  root path, canonical root path, token, and raw resume text from the response.
- The unauthorized `POST /imports` test passed: missing bearer token returns
  `401 Unauthorized`, does not enqueue an import task, and does not leak local
  paths or resume text.
- The meta-store atomic insert test passed, covering daemon command enqueue
  inserting `ImportTask` and `ImportScanScope` in one SQLite transaction so the
  import worker cannot claim a scope-less task.
- Malformed IPC request testing passed: invalid `Content-Length` returns a
  per-request `400 Bad Request`, and a subsequent `/status` request still
  succeeds, proving the daemon stays alive.
- Running-root duplicate testing passed: authenticated `POST /imports` returns
  `409 Conflict` instead of silently accepting/reusing a live running task.
- Combined daemon testing passed: `POST /imports` into a daemon running both
  IPC and the import worker was processed to completion, with two searchable
  synthetic documents and no path/token leakage in responses.
- Wrong-token and existing weak-token-permission tests passed: bad bearer
  tokens do not enqueue tasks, and pre-existing Unix `0644` `ipc.auth` files
  are repaired to `0600` before use.
- `cargo test -p resume-daemon --test s20_ipc`: exit 0; 13 IPC tests passed,
  covering redacted status, non-loopback rejection, 404 path handling,
  authenticated import command IPC, malformed-request liveness, wrong token,
  token permissions, running duplicate conflict, combined IPC plus import
  worker, bind failure, and worker tick-limit rejection.
- `cargo test -p meta-store`: exit 0; 36 tests passed, including atomic
  import task plus scan scope insertion.
- `cargo test -p resume-daemon`: exit 0; daemon identity, IPC, and worker
  scheduler tests passed.
- `cargo clippy -p meta-store -p resume-daemon --all-targets -- -D warnings`:
  exit 0.

Workspace acceptance:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace
```

Output summary:

- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  exit 0.
- `cargo test --workspace`: exit 0; all workspace tests and doc-tests passed,
  including the S46 daemon import command IPC coverage.
- The obsolete-reference marker scan exited 1 with no matches, confirming those
  obsolete preliminary references remain absent from the repository.

Sub-agent orchestration:

- Subagent-driven guidance was used as implementation discipline. After local
  implementation and verification, a separate Codex sub-agent review was
  spawned for the S46 diff to check command IPC correctness/security risks.
- The sub-agent found four actionable issues before commit: task/scope enqueue
  was not atomic in combined mode, malformed requests could terminate the
  daemon, existing weak-permission token files were trusted, and duplicate
  running imports returned misleading `202 Accepted`. All four were fixed and
  covered by the implementation checks above.

Scope note:

- S46 does not add search/detail IPC endpoints, CLI import-over-IPC UX, token
  rotation/revocation, import cancellation/progress streaming, singleton
  service lifecycle enforcement, daemon OCR/vector workers, real whole-machine
  witness scans, or macOS/Windows service validation. Those remain incomplete.

### S9

TDD red checks:

```bash
cargo test -p meta-store import_task_status_updates_support_completion_and_retry
cargo test -p resume-cli --test s9_import_search
```

Output summary:

- `meta-store` failed before implementation because `MetaStore::update_import_task_status` did not exist.
- `resume-cli --test s9_import_search` failed before implementation because `import` still left tasks queued and did not build a search index.

Implementation and review checks:

```bash
cargo test -p meta-store
cargo test -p resume-cli --test s4_cli
cargo test -p resume-cli --test s8_search_cli
cargo test -p resume-cli --test s9_import_search
```

Output summary:

- `cargo test -p meta-store`: exit 0; 17 tests passed, including import task completion/retry lifecycle, root-based pending task lookup, timestamp lifecycle rejection, resume version lookup by document, and existing SQLite recovery tests.
- `cargo test -p resume-cli --test s4_cli`: exit 0; S4 no-index behavior and no-path-leak import behavior still passed after synchronous import execution.
- `cargo test -p resume-cli --test s8_search_cli`: exit 0; CLI search still read an existing full-text index.
- `cargo test -p resume-cli --test s9_import_search`: exit 0; 3 S9 tests passed, covering synthetic docx/PDF import, OCR_REQUIRED scanned PDF, reopened full-text snapshot search, failed-retryable task retry, live running task non-takeover, and empty-root import preserving prior searchable documents.

Acceptance:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run -p resume-cli -- import --root tests/fixtures/resumes
cargo run -p resume-cli -- status
cargo run -p resume-cli -- search "Java"
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test --workspace`: exit 0; all workspace tests, including the 3 S9 CLI smoke tests, passed.
- `resume-cli import --root tests/fixtures/resumes`: exit 0; completed an import task for 3 synthetic files, with 2 searchable documents, 1 OCR-required document, 0 failed documents, and 0 scan errors.
- `resume-cli status`: exit 0; reported `indexed documents: 2`, `searchable documents: 2`, `ocr queue: 1`, `import tasks queued: 0`, `import tasks recoverable: 0`, `index health: ready`, and full-text index available.
- `resume-cli search "Java"`: exit 0; returned 2 results: `synthetic-java-engineer.docx` and `synthetic-java-platform.pdf`, with snippets from synthetic fixture text.

Review notes:

- Sub-agent spec review found no P0 issues and identified missing PROGRESS evidence plus missing retry smoke; both were fixed.
- Sub-agent code-quality review found one P1 around empty-root import clearing the Tantivy index while SQLite still counted searchable documents; the pipeline now rebuilds the full-text index from persisted searchable/partial documents plus newly imported documents, and the S9 CLI test covers this.
- Remaining non-blocking P2: scan errors are counted but not yet persisted as recoverable import diagnostics. This is left for later diagnostics/fault-injection slices.

Scope note:

- S9 completes a synthetic import-to-search smoke loop only. It does not run OCR, generate embeddings, implement field-filter search, claim production-scale performance, or package/release the app.

### S8

TDD red checks:

```bash
cargo test -p index-fulltext
cargo test -p search-planner
cargo test -p resume-cli --test s8_search_cli
```

Output summary:

- `index-fulltext` failed before implementation because `FullTextIndex`, `IndexDocument`, `IndexSection`, and `SearchQuery` were unresolved imports.
- `search-planner` failed before implementation because `plan_search` and `SearchPlan` were unresolved imports.
- `resume-cli --test s8_search_cli` failed before implementation because CLI tests could not seed or read a full-text index.

Implementation checks:

```bash
cargo test -p index-fulltext
cargo test -p search-planner
cargo test -p resume-cli --test s8_search_cli
cargo test -p resume-cli --test s4_cli
```

Output summary:

- `cargo test -p index-fulltext`: exit 0; 7 S8 tests passed, covering committed documents searchable after reload, deleted documents hidden by default, duplicate sections not hiding distinct topN documents, malformed query syntax returning safe results, topN snippets only for returned hits, mixed Chinese-English query matching, and redacted debug output.
- `cargo test -p search-planner`: exit 0; 4 S8 tests passed, covering mixed query planning, debug redaction, empty/too-broad query rejection, and topN limit clamping.
- `cargo test -p resume-cli --test s8_search_cli`: exit 0; CLI search read an existing synthetic full-text index and printed rank, doc_id, version_id, file_name, and snippet without a query label.
- `cargo test -p resume-cli --test s4_cli`: exit 0; no-index search still returned unavailable/results 0 without echoing the query or creating a data directory.

Acceptance:

```bash
cargo fmt --check
cargo test -p index-fulltext
cargo test -p search-planner
cargo run -p resume-cli -- search "Java 支付"
cargo clippy -p index-fulltext -p search-planner -p resume-cli --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p index-fulltext`: exit 0; 7 tests passed.
- `cargo test -p search-planner`: exit 0; 4 tests passed.
- `cargo run -p resume-cli -- search "Java 支付"`: exit 0; no local full-text index existed, so CLI returned `search index not available yet` and `results: 0` without fake rows.
- `cargo clippy -p index-fulltext -p search-planner -p resume-cli --all-targets -- -D warnings`: exit 0.

Scope note:

- S8 is only the Tantivy full-text index/search CLI slice. It does not implement import execution, OCR execution, embeddings, vector search, packaging, or S9+ behavior.

Additional workspace regression:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Output summary:

- `cargo test --workspace`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.

### S7

TDD red checks:

```bash
cargo test -p text-normalizer
cargo test -p sectionizer
cargo test -p extractor-rules
```

Output summary:

- `text-normalizer` failed before implementation because `TextNormalizer` and normalized offset mapping APIs were unresolved imports.
- `sectionizer` failed before implementation because `Sectionizer` and section chunk APIs were unresolved imports.
- `extractor-rules` failed before implementation because `extract_strong_fields` and `FieldType` were unresolved imports.

Implementation checks:

```bash
cargo test -p text-normalizer
cargo test -p sectionizer
cargo test -p extractor-rules
```

Output summary:

- `cargo test -p text-normalizer`: exit 0; 5 S7 tests passed, covering mixed Chinese-English whitespace cleanup, table-linearized text, offset mapping across inserted newlines, repeated page header/footer removal, simple OCR spacing repair, bullet preservation, and redacted debug output.
- `cargo test -p sectionizer`: exit 0; 5 S7 tests passed, covering Chinese/English resume heading recognition, fallback paragraph/length chunks including single overlong paragraphs, table-linearized text staying inside the nearest section, character offsets, and redacted debug output.
- `cargo test -p extractor-rules`: exit 0; 4 S7 tests passed, covering strong email, phone, and date-range extraction, normalized values, byte offsets over table-linearized text, and low-confidence candidates not entering strong filtering.

Acceptance:

```bash
cargo fmt --check
cargo test -p text-normalizer
cargo test -p sectionizer
cargo test -p extractor-rules
cargo clippy -p text-normalizer -p sectionizer -p extractor-rules --all-targets -- -D warnings
```

Output summary:

- `cargo fmt --check`: exit 0.
- `cargo test -p text-normalizer`: exit 0; 5 tests passed.
- `cargo test -p sectionizer`: exit 0; 5 tests passed.
- `cargo test -p extractor-rules`: exit 0; 4 tests passed.
- `cargo clippy -p text-normalizer -p sectionizer -p extractor-rules --all-targets -- -D warnings`: exit 0.

Scope note:

- S7 is only the text cleanup, section fallback, and strong-rule extraction slice. It does not implement import execution, DB writes, indexing, search, OCR execution, embeddings, or S8+ behavior.

Additional workspace regression:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Output summary:

- `cargo test --workspace`: exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit 0.

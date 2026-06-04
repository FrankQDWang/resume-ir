# Progress

This file tracks long-running production Goal execution against `GOAL.md`, the
system design docs, the execution docs, and this running evidence log. Obsolete
preliminary checklists are historical execution context only, not the
production-ready scope source.

## Execution Boundaries

- Repository: `/Users/frankqdwang/MLE/resume-ir`
- Data policy: S0-S96, S98, S101, S102, S103, S104, S107, S108, S111, S112, S114, S115, S116, S117, S118, S119, S120, and S121 used synthetic fixtures only.
  S97, S99, S100, S105, S106, S109, S110, S113, S122, and S123 also used private local-only witnesses against anonymized temporary copies from a
  user-authorized local resume sample directory; no real resume data, filenames,
  paths, counts, raw text, or diagnostics were committed or uploaded.
- Remote side effects: the public GitHub repository `FrankQDWang/resume-ir` was created during S67 after public-repo guard passed, and local `main` was pushed at `cc009da12c7c5753bbf3e66642fccee7db2ebeae`, then updated to `135f927` after S67 and `d0798fa` after S68. Main branch protection has been configured, draft PR #8 exists for the branch-protection progress record, and draft PR #9 exists for the current feature branch. No release, upload of runtime data, signing, or notarization has been performed.
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
  processing, status-pollable live import progress persisted in scan scope
  counters without path disclosure, and a polling import-rescan mode that
  requeues completed roots for background incremental import without printing
  root paths. The daemon can also stream authenticated redacted import progress
  snapshots over loopback IPC while import, OCR, or embedding worker loops run.
  The daemon now writes a local endpoint discovery
  manifest, and the CLI can use `--ipc auto` for status, import progress,
  import, cancel-import, search, and detail commands. A daemon full-text index
  maintenance worker can now force a local snapshot rebuild or run in a loop to
  repair non-ready snapshot roots. Public-repository governance now includes
  MIT licensing, CODEOWNERS, contribution/security policy, PR templates,
  GitHub Actions workflow definitions, dependency update configuration, local
  license checks, public push guardrails, and a PR-triggered hosted macOS plus
  Windows workspace build/test workflow. Missing or BLOCKED production
  control-plane work includes full service lifecycle proof, platform installer
  proof, and platform service validation.
- P1 import/search: directory scanning, DOCX/legacy `.doc` via local converter,
  text-layer PDF/UTF-8 and BOM-marked UTF-16 TXT parsing, cleaning, sectioning,
  polling background rescan for completed import roots, OS filesystem watcher
  integration that requeues completed roots through the existing durable import
  task path on local file changes, full-text snapshot publish/recover with
  Windows transient read-open retry, delete rebuild, redacted snippets, and an
  isolated local PDF/Word witness command
  that can use either an explicit root or the local-discovery root preset,
  anonymizes selected inputs, runs the real import path in a temporary data
  directory, can optionally run bounded OCR jobs through the existing local OCR
  worker path, reports redacted success/failure counters without stopping a
  budgeted witness on the first per-document OCR failure, prints only aggregate
  redacted output, can run a redacted internal full-text search probe without
  printing the private query or matched files, can run a redacted field-extraction
  aggregate probe without printing field values, filenames, or paths, and
  removes private witness data. Missing production work includes
  production-grade PDF coverage, full
  legacy Word converter distribution and cross-platform proof, large-corpus
  proof, cross-platform watcher behavior proof, and incremental index updates.
- P2 fields/dedupe/privacy: high-confidence rules for name, contacts/date/
  education/company/title/skills/certs/years, persisted entity mentions,
  metadata-indexed field prefiltering before the full-text TopDocs cutoff,
  contact HMAC assignment, candidate folding, and explicit best-effort local
  purge of tombstoned documents across metadata, obsolete full-text snapshots,
  full-text staging directories, vector records, current ingest jobs, and
  current OCR page-cache records exist. A labeled field-quality evaluator and
  gate now score precision/recall/F1 from JSONL samples without emitting raw
  text, sample IDs, paths, or field values. Soft-dedupe scoring now compares
  same-name profiles with bounded non-contact evidence overlap and surfaces
  redacted suspected-duplicate hints in local CLI and daemon search results
  without low-confidence candidate folding. The local PDF/Word witness can now
  run a redacted field-extraction probe that
  verifies persisted field mentions by aggregate field type only, without
  selecting, printing, or committing raw/normalized field values, filenames,
  paths, private queries, raw text, or diagnostics. Missing production work
  includes broader dictionaries, stronger normalization, real business labeled
  F1 datasets/results, dedupe quality metrics, candidate merge review
  workflows, encrypted local storage, future bbox/PII surface purge coverage,
  and forensic erase proof.
- P3 semantic/hybrid: local embedding command protocol, persisted vector
  snapshot, in-memory linear KNN, persistent HNSW ANN query backend, RRF
  helpers, embedding worker, model/dimension-scoped durable per-version
  embedding jobs, model-scoped vector query isolation, section-level vector
  inputs, CLI semantic/hybrid query execution, and local model-pack manifest
  validation with checksum plus license-reviewed gates now exist. The daemon
  can now execute a configured local embedding command in one-shot or
  long-running worker mode, persist a vector snapshot while serving status IPC,
  skip already completed version jobs across daemon restarts, re-embed
  completed versions when the configured model id or dimension changes, and
  write document plus section vectors inside one version job. Persistent vector
  search now rebuilds a process-local HNSW ANN graph from the durable vector
  snapshot, preserves model-scoped graph isolation, and reports the ANN backend
  through redacted CLI status, doctor, and diagnostics output without emitting
  vectors or local paths. Persistent vector mutations now use a stable
  sidecar file lock, reload the latest snapshot while holding that lock, merge
  the current mutation, and refresh local HNSW state before returning, preventing
  stale CLI/daemon writers from overwriting each other's vector updates or
  tombstones.
  A labeled vector-quality evaluator and gate now score recall@k, MRR, NDCG@k,
  and zero-recall queries from JSONL samples using the local embedding command
  protocol without emitting raw queries, candidate text, sample IDs, candidate
  IDs, vectors, command paths, or resume paths.
  Missing or BLOCKED work includes licensed model selection/download/
  distribution, real business semantic quality datasets/results, real ANN
  recall/latency proof at large corpus scale, and real performance proof.
- P4 OCR: OCR_REQUIRED routing, durable OCR jobs, pause/resume control, page
  cache schema, local OCR command client, local PDF page-render command
  protocol, local Poppler `pdftoppm` PDF renderer adapter, local Tesseract OCR
  adapter with TSV confidence and word-box parsing, timeout/cancel/temp cleanup,
  page-count detection for scanned PDFs, multi-page OCR fan-out, per-page cache
  entries with persisted OCR word boxes, aggregate OCR text indexing, a
  per-document OCR page-count backpressure guard, redacted page-budget
  remediation diagnostics, and redacted local OCR runtime availability
  diagnostics exist. The CLI and daemon can now claim queued OCR jobs, reject
  scanned PDFs above the configured local page budget before renderer/OCR
  invocation, persist a safe `ocr_page_budget_exceeded` job failure kind,
  surface aggregate page-budget blocks through local status, doctor, redacted
  diagnostics, and daemon status IPC, report local `pdftoppm`, Tesseract, and
  requested Tesseract OCR language-pack availability without binary paths or
  dumping the full local language list, render valid PDF pages through local `pdftoppm` or a configured renderer,
  execute local OCR commands or local Tesseract on the rendered image, persist
  cache entries for each page, index combined OCR text with page count, honor
  persistent pause state, keep serving status IPC while OCR runs, and exercise
  OCR from the local PDF/Word witness command with redacted completed, blocked,
  and per-document failure aggregate output. The benchmark runner can now
  exercise synthetic OCR page throughput through the existing local command or
  Tesseract OCR clients and gate redacted page-latency/pages-per-second reports
  with explicit synthetic opt-in.
  Deleted-document purge now removes
  current OCR jobs and current OCR page-cache entries that are no longer shared
  by visible documents. Missing or BLOCKED work includes final OCR/renderer
  distribution policy, full non-English OCR quality and language-pack distribution policy, full-library scanned
  resume OCR proof beyond bounded local witness budgets, real large-corpus OCR
  throughput proof, and Windows/macOS
  validation.
- P5 packaging/platform: not production-ready. A local CLI service lifecycle
  now writes, reports, removes, starts, stops, and reports runtime state for a
  macOS user LaunchAgent without CLI path disclosure. A local-only macOS
  LaunchAgent witness has installed a temporary daemon, observed `not_loaded`
  before start, observed `running` after start, read daemon status through
  authenticated IPC auto-discovery, stopped the daemon, observed `not_loaded`
  after stop, uninstalled the LaunchAgent, and removed temporary local data.
  Hosted macOS and Windows workspace build/test checks now run for pull
  requests through Platform CI. A release dry-run workflow can now generate and
  upload a redacted `release-artifacts.json` checksum manifest for locally built
  release binaries without recording local paths or runtime data.
  Installer packaging, signing, notarization, Windows service/MSI, real upgrade/
  uninstall runs, hosted release workflow execution, and platform installer/
  service validation remain absent, not complete, or externally blocked by
  platform credentials/runners.
- P6 performance/stability: synthetic benchmark runner, status/doctor/export
  diagnostics, redacted resource telemetry for the data-disk volume, current
  process memory, CPU cores, OCR page-budget remediation, and OCR runtime
  availability, snapshot fallback, explicit obsolete
  full-text snapshot and staging cleanup for deleted-document purge, safe fault
  simulation for disk-space budget, permission-denied probes, file-lock
  contention probes, metadata migration failure probes against synthetic broken
  scratch databases, daemon-kill/restart probes against configured daemon
  binaries, OCR command crash probes, model-checksum probes against controlled
  local model artifacts, local model-pack manifest validation, targeted fault
  tests, persistent vector snapshot writer-lock protection against stale
  concurrent writers, hosted-Windows full-text snapshot read-open retry,
  local-only macOS LaunchAgent start/stop witness evidence, local-only
  production runbooks, a runbook CI policy guard, a workflow policy guard, and a
  release artifact manifest policy guard, and a synthetic OCR throughput
  benchmark/gate exist.
  The benchmark runner now has explicit synthetic query, synthetic OCR
  throughput, and labeled vector-quality benchmark gates; query, OCR, and
  vector smoke gates are wired into PR and nightly workflows. Synthetic runs
  must opt in with `--allow-synthetic` and cannot prove 100k/1M production
  performance.
  Missing or BLOCKED work includes 100k/1M real-corpus benchmarks,
  real-corpus nightly/release performance gates, licensed model selection/
  distribution, real semantic/vector quality datasets/results, destructive
  service-level kill/actual ENOSPC fault injection, battery/external-drive
  fault drills, Windows/macOS validation, and cross-platform performance
  evidence.

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
| S65 | Local slice complete; remote unblocked later | `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo metadata --no-deps --locked --format-version 1`, `/Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- synthetic-query --documents 24 --queries 6 --top-k 5 --json`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace --locked`, `./scripts/ci/check-licenses.sh`, `./scripts/ci/guard-public-repo.sh`, `sh -n scripts/ci/guard-public-repo.sh scripts/ci/check-licenses.sh scripts/ci/verify-local.sh scripts/ci/configure-github-repo.sh`, and the obsolete-reference marker scan passed with no matches. | Remote GitHub repository work was blocked in S65 by invalid CLI auth but was unblocked and started during S67. Real whole-machine witness runs, Windows/macOS validation, token rotation/revocation, and packaging/signing remain not complete or BLOCKED. |
| S66 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked`, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `git diff --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `/Users/frankqdwang/.cargo/bin/cargo test --workspace --locked`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan passed with no matches. | None for this macOS LaunchAgent CLI lifecycle slice; live `launchctl` start/stop was implemented but not exercised against the user's real login session, and Windows service/MSI, signed pkg/dmg, notarization, real upgrade/uninstall, hosted runner validation, and complete release packaging remain not complete or BLOCKED. |
| S67 | Product governance slice complete | `gh repo view FrankQDWang/resume-ir` showed the repository was initially absent, `gh repo create FrankQDWang/resume-ir --public --source=. --remote=origin --description "Local-first resume search engine" --disable-wiki` created it, `git remote -v` showed HTTPS origin, `./scripts/ci/guard-public-repo.sh` passed, `git push -u origin main` pushed `cc009da12c7c5753bbf3e66642fccee7db2ebeae`, and `sh -n scripts/ci/configure-github-repo.sh` plus `git diff --check` passed after the HTTPS fallback script fix. | Branch protection is intentionally deferred until this S67 progress/script-fix commit is pushed. PR creation, hosted Actions results, releases, signing, notarization, Windows/macOS package validation, and real whole-machine witness runs remain not complete or BLOCKED. |
| S68 | Product governance slice complete | `./scripts/ci/configure-github-repo.sh FrankQDWang resume-ir` failed at `gh repo edit` with `HTTP 422` because `--allow-forking` is only applicable to org-owned private repositories, `sh -n scripts/ci/configure-github-repo.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan passed after removing that invalid option. | Branch protection still has to be rerun after S68 is pushed. Hosted Actions results, releases, signing, notarization, Windows/macOS package validation, and real whole-machine witness runs remain not complete or BLOCKED. |
| S71 | Product slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, and `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings` passed after the RED test first failed because `fault-simulate` did not exist. | None for this safe fault-simulation CLI slice; actual disk-fill/ENOSPC, real file-lock semantics, kill-daemon fault injection, OCR worker crash injection, migration-failure injection, model checksum fault, battery mode, external-drive disconnect, and cross-platform validation remain not complete or BLOCKED. |
| S72 | Stability slice complete | `./scripts/ci/verify-local.sh` first exposed a concurrent local-command embedder temp-directory collision as `EngineFailed`; after the fix, `/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked` passed with 6 tests and `./scripts/ci/verify-local.sh` passed end to end. | None for this CI stability slice; licensed model packaging, ANN, real semantic quality metrics, OS-enforced no-network sandboxing for configured commands, and Windows/macOS validation remain not complete or BLOCKED. |
| S73 | CI portability slice complete | GitHub Actions PR #9 `rust workspace` failed on Linux because the embedder permission test used macOS `stat -f` before GNU `stat -c`; the fixture command now uses GNU `stat -c` first and falls back to macOS `stat -f`. | None for this Linux CI test portability slice; broader Linux package validation, Windows validation, signed installers, notarization, and full cross-platform release evidence remain not complete or BLOCKED. |
| S74 | CI fix attempt; superseded by S75 | GitHub Actions PR #9 `rust workspace` then failed in `ocr-client` because timeout cleanup could return after the direct child exited while descendants still held output pipes. The first local fix sent `KILL` to the process group even after the direct child exited, and `./scripts/ci/verify-local.sh` passed locally, but GitHub Actions still failed on the same descendant-pipe timing test. | S74 alone did not clear Linux CI; S75 follows with the actual timeout-path reader fix. |
| S75 | CI fix attempt; superseded by S76 | GitHub Actions PR #9 `rust workspace` still failed in `local_command_worker_terminates_descendants_that_keep_output_pipes_open` after S74. The timeout/cancel/error path returned the terminal OCR error without joining stdout/stderr reader threads, preventing inherited pipes from delaying timeout return, and `./scripts/ci/verify-local.sh` passed locally. | GitHub Actions later failed with exit 143 while running `tests/s50_ocr_worker.rs`, so S75 was not sufficient; S76 follows with child-process cleanup plus output-reader joining. |
| S76 | CI fix attempt; superseded by S77 | GitHub Actions PR #9 `rust workspace` failed after S75 with exit 143 while running daemon OCR worker tests. The S76 fix restored timeout/cancel error-path output-reader joining, while terminating direct child processes before the parent exited and then terminating the process group so inherited pipes would not hang cleanup. Focused local checks passed for `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked`; `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan also passed. | GitHub Actions later failed in the original inherited-pipe descendant timeout test, so S76 was not sufficient on Linux; S77 follows with portable process-group signal syntax. |
| S77 | CI fix attempt; superseded by S78 | GitHub Actions PR #9 `rust workspace` failed after S76 in `local_command_worker_terminates_descendants_that_keep_output_pipes_open`; the timeout returned only after the descendant closed inherited pipes. The S77 fix used `/bin/kill <signal> -- -PGID` for OCR Unix process-group signaling and removed the unreliable direct-child `pkill -P` helper. Focused local checks passed for `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked`; `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan also passed. | GitHub Actions later passed OCR but failed with exit 143 while running daemon embedding worker tests, exposing the same process-group signaling gap in the embedder; S78 follows. |
| S78 | CI portability slice complete | GitHub Actions PR #9 `rust workspace` failed after S77 with exit 143 while running `tests/s51_embedding_worker.rs`, after OCR tests had passed. The S78 fix applies the same `/bin/kill <signal> -- -PGID` Unix process-group syntax to the local command embedder and adds an embedder inherited-pipe descendant timeout regression test. Focused local checks passed for `/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker --locked`, and `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked`; `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and PR #9 hosted checks also passed after formatting. | None for this process-cleanup portability slice. Real embedding model packaging, ANN, Linux/macOS/Windows service validation, signed installers, notarization, and full release evidence remain not complete or BLOCKED. |
| S79 | Product diagnostics slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_redacted_resource_telemetry --locked` first failed because doctor/export did not report resource telemetry; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings`, `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and PR #9 hosted checks passed. | None for this redacted resource telemetry slice; 100k/1M real-corpus benchmarks, nightly gates, destructive kill/actual ENOSPC fault injection, file-lock semantics, runbooks, and cross-platform performance evidence remain not complete or BLOCKED. |
| S80 | Product fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_file_lock_reproduces_contention_without_path_leak --locked` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked` first failed because `file-lock` was not supported and diagnostics did not advertise `file_lock`; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings`, `./scripts/ci/verify-local.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and the obsolete-reference marker scan passed. | None for this safe file-lock contention slice; 100k/1M real-corpus benchmarks, nightly gates, destructive kill/actual ENOSPC fault injection, kill-daemon/OCR-crash fault injection, model checksum fault, battery mode, external-drive disconnect, runbooks, and cross-platform performance evidence remain not complete or BLOCKED. |
| S81 | Product fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_daemon_kill_restarts_configured_daemon_without_path_leak -- --exact` first failed because `daemon-kill` was not supported, and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths -- --exact` first failed because diagnostics did not advertise `daemon_kill`; after implementation, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s81_daemon_kill --locked`, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings`, `git diff --check`, and `./scripts/ci/verify-local.sh` passed. | None for this safe daemon-kill/restart probe slice; destructive service-manager kill, actual ENOSPC, OCR-crash fault injection, model checksum fault, battery mode, external-drive disconnect, runbooks, Windows/macOS service validation, and cross-platform performance evidence remain not complete or BLOCKED. |
| S82 | Product fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_ocr_crash_reproduces_engine_failure_without_payload_or_path_leak -- --exact` first failed because `ocr-crash` was not supported, and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths -- --exact` first failed because diagnostics did not advertise `ocr_crash`; after implementation, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked`, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings`, `git diff --check`, and `./scripts/ci/verify-local.sh` passed. | None for this safe OCR command-crash probe and retryable worker-failure slice; destructive service-manager kill, actual ENOSPC, model checksum fault, battery mode, external-drive disconnect, runbooks, Windows/macOS service validation, and cross-platform performance evidence remain not complete or BLOCKED. |
| S83 | Product runbook/CI guard slice complete | `sh scripts/ci/check-runbooks.sh` first failed with `missing required runbook: docs/runbooks/diagnostics-redaction.md`; after adding local-only runbooks and wiring the guard into local/hosted CI, `./scripts/ci/check-runbooks.sh`, `sh -n scripts/ci/check-runbooks.sh scripts/ci/verify-local.sh scripts/ci/guard-public-repo.sh scripts/ci/check-licenses.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, and `./scripts/ci/verify-local.sh` passed. | None for this production runbook and policy-guard slice; 100k/1M real-corpus benchmarks, nightly performance gates, destructive service-level kill/actual ENOSPC fault injection, model checksum fault, battery mode, external-drive disconnect, Windows/macOS service validation, and cross-platform performance evidence remain not complete or BLOCKED. |
| S84 | Product benchmark-gate slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked` first failed because `evaluate_benchmark_gate_json` and `BenchmarkGateConfig` did not exist; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings`, the `resume-benchmark synthetic-query` plus `resume-benchmark gate` smoke, `./scripts/ci/check-runbooks.sh`, `git diff --check`, and `./scripts/ci/verify-local.sh` passed. | None for this synthetic benchmark gate and workflow wiring slice; 100k/1M real-corpus benchmark datasets, real-corpus nightly/release performance gates, semantic/vector quality gates, OCR throughput gates, Windows/macOS benchmark runners, and cross-platform performance evidence remain not complete or BLOCKED. |
| S85 | Product fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_model_checksum --locked` first failed because `model-checksum` was unsupported; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings`, `./scripts/ci/check-runbooks.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this controlled local model artifact checksum probe slice; real licensed model selection/download/distribution, model package manifest governance, semantic/vector quality gates, battery mode, external-drive disconnect, destructive actual ENOSPC/service-manager drills, Windows/macOS validation, and cross-platform performance evidence remain not complete or BLOCKED. |
| S86 | Product model-governance slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker model_manifest_validate --locked` first failed because `model validate-manifest` was unsupported, then failed after schema tightening because the implementation only accepted a single-model manifest instead of `model_pack_id` plus `models[]`; `./scripts/ci/verify-local.sh` also exposed a daemon scheduler test race where a post-startup queued task could be claimed before its scan scope was written, fixed by using the existing atomic `insert_import_task_with_scan_scope` API in the test helper. After implementation and the stability fix, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings`, `./scripts/ci/check-runbooks.sh`, `git diff --check`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this local model-pack manifest validation slice and daemon scheduler test stability repair; real licensed OCR/embedding model selection/download/distribution, model quality evaluation, ANN production indexing, semantic/vector quality gates, production model performance proof, and cross-platform release evidence remain not complete or BLOCKED. |
| S87 | Product search slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_fields_before_fulltext_top_k_cutoff --locked -- --exact` first failed because field filters were applied only after the full-text TopDocs cutoff, causing a synthetic Rust candidate outside the top five unfiltered keyword hits to be missed with `--top-k 1`; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p index-fulltext -p meta-store --all-targets --locked -- -D warnings`, `git diff --check`, `./scripts/ci/check-runbooks.sh`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this metadata-indexed field prefilter slice; broader dictionaries, stronger normalization, labeled field F1, ANN/vector quality gates, SQLCipher/encrypted metadata, physical purge, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S88 | Product privacy/delete slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact` first failed because `resume-cli purge` was unsupported; after implementation, `/Users/frankqdwang/.cargo/bin/cargo fmt --check`, `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector --locked`, `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked`, `/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p index-fulltext -p index-vector -p meta-store --all-targets --locked -- -D warnings`, `git diff --check`, `./scripts/ci/check-runbooks.sh`, `./scripts/ci/guard-public-repo.sh`, the obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this explicit best-effort local deleted-document purge slice; SQLCipher/encrypted metadata, forensic erase, full OCR/cache/job-retention purge coverage, real-resume witness runs, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S89 | Product OCR slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_processes_all_scanned_pdf_pages_before_indexing --locked -- --exact` first failed because OCR worker behavior was single-page/cache-write `1`; after implementation, focused CLI, daemon, OCR client, import-pipeline, parser-pdf, fmt, clippy, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this local PDF render command protocol and multi-page OCR fan-out slice; concrete PDF renderer/OCR engine install and license evidence, real Poppler/PDFium/Tesseract witness runs, bbox persistence, backpressure, full OCR cache/job purge coverage, real scanned-resume witness runs, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S90 | Product privacy/delete slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact` first failed after the test was tightened because `purge --deleted` did not report or remove OCR cache/job retention surfaces; after implementation, the focused RED/GREEN test, full `s14_delete_search`, `meta-store`, focused clippy, fmt, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this current OCR cache/job purge slice; SQLCipher/encrypted metadata, forensic erase, future OCR bbox purge surfaces, real-resume witness runs, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S91 | Product OCR slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client pdftoppm_renderer_renders_valid_pdf_page_to_ppm_without_payload_debug_leaks --locked -- --exact` first failed because `PdftoppmPdfRenderer` and `PdftoppmRenderSpec` did not exist; after implementation, OCR client, CLI handoff, daemon worker, fmt, focused clippy, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this local Poppler `pdftoppm` renderer adapter and CLI/daemon worker wiring slice; Tesseract or equivalent real OCR recognition engine, renderer/OCR distribution policy, bbox persistence, backpressure, real scanned-resume witness runs, large-corpus proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S92 | Product OCR slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client tesseract_worker_recognizes_synthetic_image_without_payload_debug_leaks -- --exact` first failed because `TesseractOcrClient` and `TesseractOcrSpec` did not exist; after implementation, local Tesseract 5.5.2 was installed, the focused Tesseract OCR client, CLI worker, daemon worker, fmt, focused clippy, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this local Tesseract adapter and CLI/daemon wiring slice; final OCR/renderer distribution policy, non-English language packs, OCR bbox persistence, backpressure, real scanned-resume witness runs, large-corpus OCR throughput proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S93 | Product OCR slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client tesseract_worker_recognizes_synthetic_image_without_payload_debug_leaks --locked -- --exact` and `/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite ocr_page_cache_persists_word_boxes_without_debug_payload_leak --locked -- --exact` first failed because OCR word-box APIs and cache persistence did not exist; after implementation, OCR client, meta-store, CLI handoff, daemon worker, fmt, focused clippy, `git diff --check`, schema expectation guard, and `./scripts/ci/verify-local.sh` passed. | None for this OCR word-box persistence slice; final OCR/renderer distribution policy, non-English language packs, backpressure, real scanned-resume witness runs, large-corpus OCR throughput proof, Windows/macOS validation, and future OCR bbox purge surface audits remain not complete or BLOCKED. |
| S94 | Product OCR backpressure slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact` first failed because OCR max-page budget parameters and guards did not exist; after implementation, CLI OCR handoff, daemon OCR worker, service lifecycle, fmt, focused clippy, `git diff --check`, runbook guard, public-repo guard, obsolete-reference marker guard, and `./scripts/ci/verify-local.sh` passed. | None for this OCR page-count backpressure slice; final OCR/renderer distribution policy, non-English language packs, real scanned-resume witness runs, large-corpus OCR throughput proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S95 | Product OCR remediation slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact` first failed because `status` did not report `ocr page budget blocked`; after implementation, meta-store, CLI OCR handoff, daemon OCR worker, CLI status IPC, fmt, focused clippy, and related full suites passed. | None for this redacted OCR page-budget remediation slice; final OCR/renderer distribution policy, non-English language packs, real scanned-resume witness runs, large-corpus OCR throughput proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S96 | Product OCR diagnostics slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_ocr_runtime_without_paths_or_language_dump --locked -- --exact` first failed because doctor did not report `ocr renderer pdftoppm`; after implementation, OCR runtime diagnostics, non-executable tool handling, full diagnostics, fmt, focused clippy, guards, and local verification passed. | None for this redacted local OCR runtime diagnostics slice; final OCR/renderer distribution policy, non-English language pack install/selection policy, real scanned-resume witness runs, large-corpus OCR throughput proof, and Windows/macOS validation remain not complete or BLOCKED. |
| S97 | Product import slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p parser-doc --test s6_doc extracts_legacy_doc_text_with_local_converter_without_output_leakage --locked -- --exact` first failed because `DocParser::with_converter` did not exist; after implementation, parser-doc, parser-common, import-pipeline, fmt, focused clippy, and a private local-only PDF/Word witness passed with no path leaks. | None for this legacy Word local-converter slice; converter distribution policy, Windows/Linux converter proof, remaining malformed/encrypted DOC behavior, full OCR completion for scanned PDFs, large-corpus proof, and full real-resume library validation remain not complete or BLOCKED. |
| S98 | Product import scheduler slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_rescans_completed_root_without_path_leak --locked -- --exact` first failed because daemon did not accept `--rescan-completed-imports`; after implementation, daemon import scheduler, meta-store, fmt, focused clippy, focused tests, `git diff --check`, runbook guard, public-repo guard, private-witness marker scan, obsolete-reference marker scan, and `./scripts/ci/verify-local.sh` passed. | None for this polling background rescan slice; true OS filesystem watcher integration, large-corpus long-running rescan proof, cross-platform watcher behavior, and incremental index-update-only writes remain not complete or BLOCKED. |
| S99 | Product local witness slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_imports_only_pdf_and_word_samples_without_persisting_private_data --locked -- --exact` first failed because `resume-cli witness` was unsupported; after implementation, focused witness, full `s9_import_search`, fmt, focused clippy, guard checks, `./scripts/ci/verify-local.sh`, and a private local-only PDF/Word witness with redacted output passed. | None for this isolated local PDF/Word witness command slice; it is not a production benchmark, does not package converters/OCR/model runtimes, does not prove Windows/Linux behavior, and does not complete full real-library quality/performance validation. |
| S100 | Product local OCR witness slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_executes_local_command_without_output_or_path_leak --locked -- --exact` and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_without_command_reports_blocked_without_persisting_private_data --locked -- --exact` first failed because `resume-cli witness` did not accept OCR options; after implementation, focused witness OCR tests, full import-search and OCR suites, fmt, focused clippy, guard checks, `./scripts/ci/verify-local.sh`, and bounded private local-only OCR witnesses passed. | None for this isolated local OCR witness option slice; it is not a full-library OCR proof, does not package OCR runtimes, does not prove non-English OCR quality, does not prove Windows/Linux behavior, and does not complete large-corpus OCR throughput validation. |
| S101 | Product import watcher slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_watcher_requeues_completed_root_after_file_change_without_path_leak --locked -- --exact` first failed because daemon did not accept `--watch-import-roots`; after implementation, focused watcher exact, full daemon import scheduler suite, daemon clippy, license guard, fmt, guard checks, and `./scripts/ci/verify-local.sh` passed. | None for this local OS watcher requeue slice; it does not prove Windows watcher behavior, long-running watcher soak stability, large-corpus event storms, or incremental index-update-only writes. |
| S102 | Product field-quality gate slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality --locked` first failed because the field-quality APIs did not exist, and `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_quality_outputs_redacted_report_and_gate --locked -- --exact` first failed because `resume-benchmark` did not accept `field-quality`; after implementation, focused field-quality tests, full benchmark-runner tests, focused clippy, license guard, fmt, guard checks, and `./scripts/ci/verify-local.sh` passed. | None for this labeled field-quality evaluator/gate slice; it does not supply real business labeled datasets, prove production field F1, improve dictionaries, or complete soft-dedupe scoring. |
| S103 | Product soft-dedupe hint slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion soft_dedupe --locked` first failed because soft-dedupe APIs did not exist; `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding search_marks_soft_duplicate_hints_without_low_confidence_folding --locked -- --exact` first failed because local search did not print soft-dedupe hints; and `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_includes_redacted_soft_dedupe_hints --locked -- --exact` first failed because daemon search JSON omitted `soft_dedupe`. After implementation, focused rank-fusion, CLI, daemon IPC tests, related suites, focused clippy, fmt, diff, runbook, public guard, and `./scripts/ci/verify-local.sh` passed. | None for this bounded redacted soft-dedupe hint slice; it does not prove real dedupe precision/recall, does not implement manual merge review, does not add large-name-bucket indexing beyond existing mention indexes and bounded candidate scans, and does not prove million-corpus latency impact. |
| S104 | Product metadata migration fault-injection slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_metadata_migration_failure_reproduces_without_path_or_schema_leak --locked -- --exact` first failed because `migration-failure` was unsupported; `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked -- --exact` first failed because diagnostics did not list `metadata_migration`. After implementation, focused fault/diagnostics tests, related suites, focused clippy, fmt, diff, runbook, public guard, marker scans, and `./scripts/ci/verify-local.sh` passed. | None for this safe synthetic migration-failure probe; it does not perform destructive migration rollback drills against real user metadata, backup/restore workflow proof, cross-platform filesystem fault proof, or upgrade rehearsal. |
| S105 | Product local OCR witness-budget slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_can_budget_documents_after_full_private_scan_without_path_leak --locked -- --exact` first failed because `witness` rejected `--ocr-max-documents`; after implementation, focused witness exact, full import-search witness suite, OCR handoff suite, focused clippy, fmt, diff, runbook, public guard, marker scans, `./scripts/ci/verify-local.sh`, and a private local-only full-directory witness with a bounded OCR document budget passed. | None for this redacted local OCR witness-budget control; it does not prove full-library OCR completion, OCR throughput, OCR quality, non-English OCR behavior, packaged OCR runtime distribution, Windows/Linux behavior, or large-corpus performance. |
| S106 | Product local-discovery witness slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_local_discovery_preset_uses_discovery_profile_without_path_leak --locked -- --exact` first failed because `witness` rejected `--root-preset local-discovery`; after implementation, focused local-discovery witness exact, full import-search witness suite, fs-crawler suite, focused clippy, fmt, diff, runbook, public guard, marker scans, `./scripts/ci/verify-local.sh`, and a private local-only local-discovery witness using the user-authorized sample directory override passed. | None for this redacted local-discovery witness path; it does not prove default whole-machine scans from `/`, Windows drive scanning, full-library OCR completion, large-corpus performance, or cross-platform watcher behavior. |
| S107 | Product synthetic OCR throughput gate slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner synthetic_ocr_throughput_reports_page_latency_without_payload_or_path_leakage --locked -- --exact` first failed because the OCR throughput API did not exist, and `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_ocr_throughput_outputs_redacted_report_and_gate --locked -- --exact` first failed because `resume-benchmark` rejected `ocr-throughput`; after implementation, focused OCR throughput tests, full benchmark-runner tests, focused clippy, fmt, diff, runbook, public guard, marker scans, and `./scripts/ci/verify-local.sh` passed. | None for this synthetic OCR throughput benchmark/gate; it does not prove real scanned-resume OCR quality, full-library OCR completion, non-English OCR behavior, packaged OCR runtime distribution, 100k/1M corpus performance, or Windows/Linux behavior. |
| S108 | Product workflow-gate slice complete | `sh scripts/ci/check-workflows.sh` first failed because PR/nightly workflows did not include `ocr-throughput`; after implementation, workflow guard, synthetic local OCR benchmark smoke plus redaction scan, shell syntax checks, fmt, diff, and `./scripts/ci/verify-local.sh` passed. | None for this OCR benchmark workflow wiring slice; it does not prove real scanned-resume OCR quality, full-library OCR completion, non-English OCR behavior, packaged OCR runtime distribution, 100k/1M corpus performance, or Windows/Linux behavior. |
| S109 | Product local OCR witness resilience slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_budget_reports_failed_documents_without_stopping_or_leaking_paths --locked -- --exact` first failed because a budgeted witness stopped as `blocked` on the first per-document OCR failure; after implementation, the focused exact, full `s9_import_search`, focused CLI clippy, fmt, diff, guard checks, marker scans, `./scripts/ci/verify-local.sh`, and private local-only PDF/Word witness runs passed with redacted aggregate output and temporary private data removal. | None for this bounded local witness resilience slice; it does not prove OCR quality, full-library OCR completion, non-English OCR behavior, packaged runtime distribution, 100k/1M corpus performance, or Windows/Linux behavior. |
| S110 | Product vector-quality gate slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner vector_quality_report_scores_labeled_samples_without_text_id_path_or_vector_leakage --locked -- --exact` first failed because vector-quality APIs did not exist, and `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_vector_quality_outputs_redacted_report_and_gate --locked -- --exact` first failed because `resume-benchmark` rejected `vector-quality`; after implementation, focused vector-quality tests, full benchmark-runner tests, focused benchmark-runner clippy, fmt, diff, guard checks, `./scripts/ci/verify-local.sh`, and private local-only bounded PDF/Word witness runs passed with redacted aggregate output and temporary private data removal. | None for this labeled vector-quality evaluator/gate slice; it does not supply real business labeled semantic datasets, choose/license/package a production embedding model, add ANN production indexing, prove large-corpus semantic latency, or validate Windows/Linux behavior. |
| S111 | Product vector workflow-gate slice complete | `./scripts/ci/check-workflows.sh` first failed because PR/nightly workflows did not include `vector-quality`; after implementation, workflow guard, strict local vector smoke/gate reproduction with redaction scan, shell syntax, workflow YAML parse, diff, public guard, marker scans, and `./scripts/ci/verify-local.sh` passed. | None for this vector-quality workflow wiring slice; it uses a synthetic labeled smoke dataset and temporary fixture embedding command, so it does not prove real semantic quality, licensed production model selection, ANN latency, 100k/1M corpus performance, or Windows/Linux behavior. |
| S112 | Product platform PR validation slice complete | `./scripts/ci/check-workflows.sh` first failed because `.github/workflows/ci-platform.yml` did not include a PR trigger; after implementation, workflow guard, workflow YAML parse, diff, public guard, and `./scripts/ci/verify-local.sh` passed. Hosted Platform CI then exposed two test-portability gaps, a hosted macOS test wait budget issue, a real Windows path-normalization bug in missing-file deletion propagation, Windows full-text snapshot publish instability during CLI imports, and Windows witness temp cleanup semantics. Local fixes now keep OCR/embedding command tests enabled on Windows with `.cmd` fixtures, extend daemon test waiting without changing product tick limits, compare deletion candidates using normalized paths, publish full-text snapshots before validation and retry transient publish locks, release witness metadata handles before cleanup, and retry witness cleanup. The final hosted PR checks passed: macOS Platform CI, Windows Platform CI, Rust workspace, dependency tree, license policy, runbook policy, and public repository guard. | None for this PR-triggered hosted build/test validation slice; it still does not prove installer packaging, signing, notarization, Windows service/MSI install/upgrade/uninstall/rollback, macOS pkg/dmg install/upgrade/uninstall/rollback, platform-specific service lifecycle behavior, real whole-machine scans, or complete release readiness. |
| S113 | Product local PDF/Word witness validation slice complete | Two authorized local-only witness runs over the private sample root passed without uploading or committing real resume data. The import-only run reported redacted aggregate import status and removed private witness data. The bounded OCR run used local `tesseract` and `pdftoppm`, reported redacted aggregate OCR status, and removed private witness data. No real resume data, filenames, paths, counts, raw text, or diagnostics were committed or uploaded. | None for this local-only private sample witness; it does not prove full-library OCR completion, OCR quality, non-English OCR quality, large-corpus latency/throughput, packaging/signing/installers, Windows/Linux real sample behavior, or production model/ANN readiness. |
| S114 | Product persistent vector ANN slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_uses_hnsw_ann_backend_after_reopen_and_keeps_model_scope --locked -- --exact` first failed because `VectorSearchBackend` and `VectorSnapshot::search_backend()` did not exist. The CLI diagnostics exact tests first failed because vector status still reported `available (vector snapshot)`. After implementation, focused index-vector and CLI diagnostics tests, fmt, diff, focused clippy, license policy, and `./scripts/ci/verify-local.sh` passed. | None for this HNSW ANN backend slice; it does not choose/license/package a production embedding model, prove real semantic quality, prove ANN recall/latency on 100k/1M corpora, add durable serialized HNSW graph artifacts separate from the existing vector snapshot, or validate hosted Windows/macOS for the new dependency. |
| S115 | Product persistent vector writer-lock slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_merges_writes_from_stale_concurrent_openers --locked -- --exact` first failed because a second stale `PersistentVectorIndex` opener rewrote the snapshot from old in-memory state and dropped the first opener's vector. After implementation, `cargo test -p index-vector --locked`, focused clippy, fmt, diff, license policy, and `./scripts/ci/verify-local.sh` passed. | None for this vector writer-lock slice; it uses cooperative local file locking and does not prove network filesystem locking semantics, durable serialized ANN graph artifacts, real large-corpus vector performance, production embedding model selection, or hosted Windows/macOS validation for this specific change. |
| S116 | Product Windows full-text read-open retry slice complete | Hosted Windows Platform CI for `f15ce1e` first failed in `published_snapshot_becomes_active_without_reading_staging_orphans` because immediate read-open of a just-inspected Tantivy snapshot returned `Access is denied. (os error 5)`. After implementation, the retry unit test, the hosted-failing full-text test, `cargo test -p index-fulltext --locked`, focused clippy, fmt, diff, public guard, `./scripts/ci/verify-local.sh`, and final hosted PR checks passed. | None for this hosted-Windows transient read-open retry; it does not prove installer/service behavior, real full-library scans, network filesystem semantics, or large-corpus full-text latency. |
| S117 | Product macOS service runtime witness slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli tests::launchctl_status_success_with_running_state_reports_running --locked -- --exact` first failed because service runtime state parsing did not exist. After implementation, the launchctl parser tests, service lifecycle integration tests, focused clippy, fmt, diff, public guard, `./scripts/ci/verify-local.sh`, and a local-only temporary macOS LaunchAgent install/start/status IPC/stop/uninstall witness passed. | None for this local macOS LaunchAgent runtime witness; it does not prove signed pkg/dmg packaging, notarization, upgrade/rollback behavior, Windows service/MSI behavior, or hosted release workflow execution. |
| S118 | Product service status cross-platform portability slice complete | Hosted Windows Platform CI for `288a4c9` first failed in `service_status_and_uninstall_are_redacted_and_preserve_user_data` because `service status` tried to derive a macOS launchctl domain through `/usr/bin/id` on Windows. After implementation, service lifecycle integration tests, launchctl parser tests, focused clippy, fmt, diff, public guard, and `./scripts/ci/verify-local.sh` passed. Hosted Rust Workspace for `c56e966` then exposed a non-macOS clippy dead-code gap that is handled in S119. | None for this portability fix; Windows service/MSI install/start/stop behavior remains not implemented or proven, and non-macOS service runtime status intentionally reports `unknown` for the macOS LaunchAgent command surface. |
| S119 | Product service runtime cfg portability slice complete | Hosted Rust Workspace for `c56e966` first failed on Ubuntu clippy because non-macOS binary builds treated macOS-only launchctl parser code and `running`/`loaded` runtime states as dead code, and newer clippy flagged a needless return in the non-macOS branch. After implementation, service lifecycle integration tests, launchctl parser tests, focused CLI clippy, fmt, diff, public guard, `./scripts/ci/verify-local.sh`, and final hosted PR checks passed. | None for this cfg portability fix; it proves the macOS LaunchAgent command surface remains portable across hosted clippy/builds, but it does not implement Windows services/MSI or prove Windows service lifecycle behavior. |
| S120 | Product OCR requested-language diagnostics slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_check_requested_ocr_language_without_language_dump --locked -- --exact` first failed because `doctor` did not accept OCR diagnostic arguments and diagnostics always reported only `eng`. After implementation, the focused exact test, full diagnostics suite, focused CLI clippy, fmt, diff, public guard, `./scripts/ci/verify-local.sh`, and final hosted PR checks passed. | None for this OCR runtime diagnostics slice; it does not distribute OCR engines or language packs, prove non-English OCR quality, complete full-library OCR, or validate Windows/macOS installed OCR runtime behavior beyond local/hosted command checks. |
| S121 | Product release dry-run manifest slice complete | `sh scripts/ci/check-release-artifacts.sh` first failed because `scripts/release/create-artifact-manifest.sh` did not exist. After implementation, the release artifact guard, workflow guard, runbook guard, diff check, and `./scripts/ci/verify-local.sh` passed. | None for this dry-run manifest/checksum slice; it does not build MSI/pkg/dmg installers, sign, notarize, generate an SBOM, create a GitHub Release, upload release binaries, or prove install/upgrade/uninstall/rollback behavior. |
| S122 | Product local witness search-probe slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_search_runs_private_query_without_leaking_query_or_paths --locked -- --exact` first failed because `witness` rejected `--probe-search`. After implementation, the focused exact test, full `s9_import_search` suite, focused CLI clippy, fmt, diff, marker scan, public guard, `./scripts/ci/verify-local.sh`, private local-only import/search witness, private local-only bounded OCR/search witness, and final hosted PR checks passed. | None for this redacted witness search-probe slice; it does not prove full-library OCR completion, real search quality, real large-corpus latency/throughput, production embedding model readiness, Windows/Linux real sample behavior, or installer/release readiness. |
| S123 | Product local witness field-probe slice complete | `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_fields_reports_aggregate_counts_without_values_or_paths --locked -- --exact` first failed because `witness` rejected `--probe-fields`. After implementation, the focused exact test, full `s9_import_search` suite, focused CLI clippy, fmt, diff, marker scan, public guard, `./scripts/ci/verify-local.sh`, private local-only field witness, and private local-only bounded OCR/field witness passed with metadata-only field-type aggregation, redacted aggregate output, and temporary private data removal. | None for this redacted witness field-probe slice; it does not prove field extraction quality, real labeled field F1, full-library OCR completion, real search/ranking quality, large-corpus latency/throughput, Windows/Linux real sample behavior, or installer/release readiness. |

## Command Log

### S123

Design target:

- Let `resume-cli witness` prove the persisted field-extraction path on private
  PDF/Word samples without printing field values.
- Keep the probe aggregate-only: status, document count, mention count, and
  per-field-type counts.
- Read field probe evidence from metadata-only `entity_type` count aggregation
  without selecting raw or normalized field values.
- Never print or commit private field values, filenames, paths, raw text,
  private queries, diagnostics, or temporary witness data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_fields_reports_aggregate_counts_without_values_or_paths --locked -- --exact
```

Output summary:

- The focused witness test failed because `resume-cli witness` did not accept
  `--probe-fields` and returned the witness usage string.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_fields_reports_aggregate_counts_without_values_or_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Focused witness field-probe test: exit 0; it confirmed the probe completes
  with non-zero field mentions and does not print the private root, canonical
  private root, data dir, private filenames, fixture filenames, or extracted
  field values.
- Full import/search witness suite: exit 0; 25 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Private local-only field witness: exit 0; redacted aggregate status showed
  completed import, completed field probe, and private data removal; the probe
  used metadata-only field-type aggregation, temporary stdout/stderr logs were
  removed, and no private output was committed.
- Private local-only bounded OCR/field witness: exit 0; redacted aggregate
  status showed completed import, completed OCR, completed field probe, and
  private data removal; temporary stdout/stderr logs were removed and no
  private output was committed.
- Marker scan: no private sample root, path marker, token marker, or temporary
  witness-log marker was present in tracked progress/code changes.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, and public repository guard passed.

Scope note:

- S123 proves only a redacted local witness field-extraction probe and bounded
  local OCR/field witness behavior. It does not prove field extraction quality,
  real labeled field F1, full-library OCR completion, real ranking quality,
  large-corpus performance, Windows/Linux private sample behavior, or release
  readiness.

### S122

Design target:

- Let `resume-cli witness` prove a local import-to-search loop on private
  PDF/Word samples without requiring the user to supply a query.
- Generate the search probe query only inside the temporary private witness
  data directory, never print the query, matched filenames, snippets, paths, or
  raw resume text, and remove temporary private witness data after the run.
- Keep the probe aggregate-only: status plus hit count.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_search_runs_private_query_without_leaking_query_or_paths --locked -- --exact
```

Output summary:

- The focused witness test failed because `resume-cli witness` did not accept
  `--probe-search` and returned the witness usage string.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_probe_search_runs_private_query_without_leaking_query_or_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Focused witness search-probe test: exit 0; it confirmed the probe completes
  with non-zero hits and does not print the private root, canonical private
  root, data dir, private filenames, fixture filenames, or internal query.
- Full import/search witness suite: exit 0; 24 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, and public repository guard passed.
- Private local-only import/search witness: exit 0; redacted aggregate status
  showed completed import, completed search probe, and private data removal;
  temporary stdout/stderr logs were removed and no private output was committed.
- Private local-only bounded OCR/search witness: exit 0; redacted aggregate
  status showed completed import, completed OCR, completed search probe, and
  private data removal; temporary stdout/stderr logs were removed and no private
  output was committed.
- Hosted PR checks: final run passed macOS Platform CI, Windows Platform CI,
  Rust workspace, dependency tree, license policy, runbook policy, and public
  repository guard.

Scope note:

- S122 proves only a redacted local witness search probe and bounded local
  OCR/search witness behavior. It does not prove full-library OCR completion,
  real ranking quality, large-corpus performance, production embedding model
  readiness, Windows/Linux private sample behavior, or release readiness.

### S121

Design target:

- Generate a release dry-run manifest for already-built binaries with artifact
  names, byte counts, and sha256 hashes only.
- Keep packaging status explicitly blocked until installer packaging, signing,
  notarization, SBOM, and release upload are separately approved and proven.
- Wire the manifest check into local verification and the release workflow
  without recording local build paths or runtime data.

Observed RED:

```bash
sh scripts/ci/check-release-artifacts.sh
```

Output summary:

- The focused release artifact guard failed because
  `scripts/release/create-artifact-manifest.sh` did not exist.

Implementation checks:

```bash
sh scripts/ci/check-release-artifacts.sh
sh scripts/ci/check-workflows.sh
sh scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- Release artifact guard: exit 0; generated a synthetic
  `release-artifacts.json`, rejected an invalid version, rejected a missing
  release binary, verified workflow artifact upload wiring, and verified the
  manifest did not contain the synthetic temp path.
- Workflow guard: exit 0.
- Runbook guard: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, release
  artifact check, and public repository guard passed.

Scope note:

- S121 is release dry-run evidence only. It does not create install packages,
  sign or notarize artifacts, create an SBOM, create a GitHub Release, upload
  release binaries, or validate installer/service lifecycle behavior.

### S120

Design target:

- Let users check the configured Tesseract OCR language from local diagnostics
  without dumping the full local `--list-langs` output.
- Keep `doctor` and `export-diagnostics --redact` output path-redacted and
  free of unrelated language-pack names.
- Preserve the default English check when no OCR language is requested.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_check_requested_ocr_language_without_language_dump --locked -- --exact
```

Output summary:

- The focused diagnostics test failed because `doctor --ocr-lang chi_sim`
  returned non-zero; diagnostics did not parse a requested OCR language and
  only checked `eng`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_check_requested_ocr_language_without_language_dump --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Focused requested-language diagnostics test: exit 0; 1 test passed.
- Full diagnostics suite: exit 0; 12 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.
- Hosted PR checks: final run passed macOS Platform CI, Windows Platform CI,
  Rust workspace, dependency tree, license policy, runbook policy, and public
  repository guard.

Scope note:

- S120 improves local OCR runtime diagnostics only. It does not package OCR
  engines or language packs and does not prove non-English OCR quality.

### S119

Design target:

- Keep `service status` portable under non-macOS binary clippy while preserving
  macOS launchctl parser coverage in tests.
- Compile `running` and `loaded` service runtime states only for macOS or test
  builds, where they are meaningful and exercised.
- Keep non-macOS service status behavior at `runtime: unknown`.

Observed RED:

```bash
gh run view 26935300792 --job 79463500481 --log
```

Output summary:

- Hosted Rust Workspace for `c56e966` failed during Ubuntu clippy.
- Clippy flagged a needless `return` in the non-macOS `service status` branch.
- Clippy also flagged `Running`, `Loaded`, and the launchctl parser as dead code
  in non-macOS binary builds.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli launchctl_status --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Focused CLI clippy: exit 0.
- Launchctl parser tests: exit 0; 4 tests passed.
- Service lifecycle integration tests: exit 0; 4 tests passed.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.
- Hosted PR checks: final run passed macOS Platform CI, Windows Platform CI,
  Rust workspace, dependency tree, license policy, runbook policy, and public
  repository guard.

Scope note:

- A local Linux-target clippy reproduction was attempted, but the macOS host did
  not have `x86_64-linux-gnu-gcc` for `zstd-sys`; hosted Ubuntu CI is the
  cross-platform witness for this slice.

### S118

Design target:

- Keep the macOS LaunchAgent service command surface portable in tests and
  hosted Windows builds.
- On non-macOS platforms, `service status` must remain redacted and successful
  for installed plist fixtures, reporting `runtime: unknown` instead of trying
  macOS-only `/usr/bin/id` or `/bin/launchctl`.
- Preserve the S117 macOS runtime query behavior on macOS.

Observed RED:

```bash
gh run view 26934977875 --job 79462477830 --log
```

Output summary:

- Hosted Windows Platform CI for `288a4c9` failed in
  `service_status_and_uninstall_are_redacted_and_preserve_user_data`.
- The status command returned non-zero because the runtime query attempted the
  macOS launchctl-domain path before handling the non-macOS platform.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli launchctl_status --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Service lifecycle integration tests: exit 0; 4 tests passed and status now
  asserts that a redacted `runtime:` line is present.
- Launchctl parser tests: exit 0; 4 tests passed.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.
- Hosted PR checks for `c56e966`: Windows Platform CI was addressed, but Rust
  Workspace failed on Ubuntu clippy and is handled in S119.

Scope note:

- S118 is a portability fix for the macOS LaunchAgent command surface. It does
  not implement Windows services/MSI or prove Windows service lifecycle.

### S117

Design target:

- Make `resume-cli service status` report runtime state, not only plist
  presence, while preserving redacted CLI output.
- Query `launchctl print` for installed macOS LaunchAgents and map results to
  `running`, `loaded`, `not_loaded`, or `unknown` without printing launchctl
  diagnostics, local paths, logs, or data directories.
- Prove a local-only temporary LaunchAgent can install, start, serve status
  through authenticated IPC auto-discovery, stop, and uninstall without reading
  or persisting real resume data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli tests::launchctl_status_success_with_running_state_reports_running --locked -- --exact
```

Output summary:

- The test failed before implementation because
  `service_runtime_state_from_launchctl_result` and `ServiceRuntimeState` did
  not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli launchctl_status --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo build -p resume-cli -p resume-daemon --locked
target/debug/resume-cli --data-dir "$data_dir" service install --launch-agent-dir "$launch_dir" --label "$label" --daemon-binary "$PWD/target/debug/resume-daemon"
target/debug/resume-cli --data-dir "$data_dir" service status --launch-agent-dir "$launch_dir" --label "$label"
target/debug/resume-cli --data-dir "$data_dir" service start --launch-agent-dir "$launch_dir" --label "$label"
target/debug/resume-cli --data-dir "$data_dir" status --ipc auto
target/debug/resume-cli --data-dir "$data_dir" service stop --launch-agent-dir "$launch_dir" --label "$label"
target/debug/resume-cli --data-dir "$data_dir" service uninstall --launch-agent-dir "$launch_dir" --label "$label"
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Launchctl status parser tests: exit 0; 4 tests passed for running, loaded,
  not-loaded, and unknown states.
- Service lifecycle integration tests: exit 0; install/status/uninstall and
  dry-run start/stop output remained redacted.
- Local macOS LaunchAgent witness: exit 0; install reported configured,
  pre-start status reported `runtime: not_loaded`, start reported started,
  post-start status reported `runtime: running`, `status --ipc auto` returned a
  redacted empty-store daemon status, stop reported stopped, post-stop status
  reported `runtime: not_loaded`, uninstall reported user data preserved, and
  temporary local data was removed.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.

Scope note:

- S117 proves a local temporary macOS LaunchAgent start/stop/status path on the
  current host only. It does not prove signed macOS pkg/dmg packaging,
  notarization, upgrade/rollback, Windows service/MSI install/uninstall, or
  release workflow execution.

### S116

Design target:

- Make full-text snapshot read-open robust to transient Windows directory/file
  handle release delays observed immediately after snapshot inspection.
- Keep retry bounded and specific to `FullTextIndex::open`, without changing
  full-text write, publish, or fallback semantics.
- Treat persistent access denial as a real error after retry exhaustion.

Observed RED:

```bash
gh run view 26934219893 --job 79460099591 --log
```

Output summary:

- Hosted Windows Platform CI for `f15ce1e` failed in
  `published_snapshot_becomes_active_without_reading_staging_orphans`.
- The failing call was `FullTextIndex::open_active(&index_root).unwrap()`,
  with Tantivy reporting `Access is denied. (os error 5)` after
  `inspect_snapshot_root` had just validated the active snapshot.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext tests::index_open_retries_transient_windows_access_denied --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext published_snapshot_becomes_active_without_reading_staging_orphans --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-fulltext --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
gh pr checks 9 --watch
```

Output summary:

- Retry unit test: exit 0; the synthetic Tantivy access-denied diagnostic was
  retried and succeeded on the third attempt.
- Hosted-failing full-text snapshot test: exit 0 locally.
- `cargo test -p index-fulltext --locked`: exit 0; 3 unit tests, 12 integration
  tests, and doc-tests passed.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0; public repo guard passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.
- Hosted PR checks: final run passed macOS Platform CI, Windows Platform CI,
  Rust workspace, dependency tree, license policy, runbook policy, and public
  repository guard.

Scope note:

- S116 covers transient local Windows read-open access denial around Tantivy
  snapshot directories. It does not prove network filesystem behavior,
  installer/service behavior, or production large-corpus full-text latency.

### S115

Design target:

- Prevent stale CLI/daemon vector-index writers from losing each other's
  updates when multiple `PersistentVectorIndex` instances open the same local
  `vector-index` root.
- Use a stable sidecar lock file rather than locking the replaceable snapshot
  file, so Windows snapshot rename semantics stay isolated from locking.
- While holding the writer lock, reload the latest durable vector snapshot,
  apply the current mutation, atomically rewrite the snapshot, and refresh the
  current instance's HNSW ANN state before returning.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_merges_writes_from_stale_concurrent_openers --locked -- --exact
```

Output summary:

- The test failed before implementation with final `vector_count` equal to 1
  instead of 2, proving that a stale second opener overwrote the first opener's
  vector update.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-vector --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-licenses.sh
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo test -p index-vector --locked`: exit 0; 10 vector index tests passed,
  including stale concurrent opener merge, stale opener tombstone preservation,
  local ANN refresh after merge, model-scoped ANN search, and stale-node
  prevention after upsert/tombstone.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/check-licenses.sh`: exit 0; license check passed.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.

Sub-agent orchestration:

- Subagent guidance was used under the Codex host-approved sub-agent tool as a
  read-only sidecar audit. The sub-agent confirmed the lost-update scenario,
  recommended a stable sidecar lock plus lock-held reload/merge/write, and did
  not edit files.

Scope note:

- S115 covers cooperative local file locking for vector snapshot mutations. It
  does not prove behavior on network filesystems, serialized HNSW graph
  persistence, production model quality, or real large-corpus vector latency.

### S114

Design target:

- Move the persistent vector query path beyond linear scan by adding a
  permissive-license HNSW ANN backend inside `index-vector`.
- Preserve the existing durable vector snapshot format and model-scoped query
  isolation; rebuild the in-process ANN graph from persisted vectors on open,
  upsert, deletion, and purge.
- Report the ANN backend through local status, doctor, and redacted diagnostics
  without emitting vector values, local paths, model command paths, or resume
  text.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector persistent_vector_index_uses_hnsw_ann_backend_after_reopen_and_keeps_model_scope --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_persistent_vector_snapshot_without_path_or_values --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker embed_worker_runs_local_command_and_persists_vector_snapshot_without_hiding_search_results --locked -- --exact
```

Output summary:

- The index-vector test failed before implementation because
  `VectorSearchBackend` and `VectorSnapshot::search_backend()` were unresolved.
- The CLI diagnostics and embed-worker exact tests failed before diagnostics
  implementation because vector status still reported
  `available (vector snapshot)` instead of the ANN backend.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_persistent_vector_snapshot_without_path_or_values --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker embed_worker_runs_local_command_and_persists_vector_snapshot_without_hiding_search_results --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-licenses.sh
/Users/frankqdwang/.cargo/bin/cargo clippy -p index-vector -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo test -p index-vector --locked`: exit 0; 8 vector index tests passed,
  including persistent HNSW ANN backend reporting after reopen, model-scoped
  ANN search, and HNSW rebuild after upsert/tombstone so stale nodes are not
  returned.
- Focused CLI diagnostics and embed-worker tests: exit 0; status, doctor, and
  redacted diagnostics now report `hnsw_ann`/`available (hnsw ann vector
  snapshot)` without local paths or vector values.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/check-licenses.sh`: exit 0; license check passed for the new
  `hnsw_rs` dependency set.
- Focused clippy: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0; workspace metadata, fmt, clippy,
  tests, doc-tests, license check, runbook check, workflow check, and public
  repository guard passed.

Scope note:

- S114 adds an HNSW ANN query backend for the persisted vector index but does
  not prove a production embedding model, real semantic quality, large-corpus
  ANN recall/latency, cross-platform hosted execution for the new dependency,
  or signed/release packaging.

### S113

Local-only private sample validation:

```bash
target/debug/resume-cli --data-dir <temporary-unused-data-dir> witness --root <authorized-private-sample-root> --max-files 10000
target/debug/resume-cli --data-dir <temporary-unused-data-dir> witness --root <authorized-private-sample-root> --max-files 10000 --run-ocr --ocr-max-documents 5 --ocr-tesseract-command <local-tesseract> --ocr-pdftoppm-command <local-pdftoppm>
```

Output summary:

- Import-only witness: exit 0; redacted aggregate status showed completed
  import and private data removal.
- Bounded OCR witness: exit 0; redacted aggregate status showed completed
  import, completed bounded OCR, expected OCR budget behavior, and private data
  removal.
- No real resume paths, filenames, counts, extracted text, OCR output, tokens,
  diagnostics packages, or model caches were committed or uploaded.

Scope note:

- S113 validates the current local-only import/OCR witness behavior on the
  authorized private sample root. It does not prove full-library OCR completion,
  quality, performance, installer/service behavior, or cross-platform real-data
  behavior.

### S112

Design target:

- Make hosted macOS and Windows workspace build/test validation run on pull
  requests rather than only on manual or scheduled workflows.
- Extend the workflow policy guard so the platform matrix and core build/test
  commands cannot be silently removed.
- Keep the scope to build/test validation only; packaging, signing,
  notarization, MSI/pkg/dmg install flows, and service lifecycle proof remain
  separate release blockers.

Observed RED:

```bash
./scripts/ci/check-workflows.sh
```

Output summary:

- The workflow guard failed because `.github/workflows/ci-platform.yml` was
  missing required text: `pull_request`.

Implementation checks:

```bash
./scripts/ci/check-workflows.sh
ruby -e 'require "yaml"; ARGV.each { |file| YAML.load_file(file); puts "yaml ok: #{file}" }' .github/workflows/ci-platform.yml .github/workflows/pr.yml .github/workflows/bench-nightly.yml
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- Workflow guard: exit 0 after Platform CI required `pull_request`,
  `macos-latest`, `windows-latest`, `cargo build --workspace --locked`, and
  `cargo test --workspace --locked`.
- Workflow YAML parse: exit 0 for Platform CI, PR, and nightly workflow files.
- `git diff --check`: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, workflow check, and
  public repo guard.

Hosted CI follow-up:

- The first pushed Platform CI run passed `macos-latest` and failed
  `windows-latest`.
- Windows failed compiling `crates/cli/tests/s9_import_search.rs` because two
  witness OCR tests called `write_fixture_executable` while that helper was
  gated behind `#[cfg(unix)]`.
- The fix keeps witness OCR command execution covered on Windows by writing
  `.cmd` fixture commands under `#[cfg(windows)]` instead of skipping the
  tests.
- Local focused verification after the fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked`
  passed with 23 tests.
- Local Windows target checking was attempted with
  `/Users/frankqdwang/.cargo/bin/cargo check -p resume-cli --test s9_import_search --target x86_64-pc-windows-gnu --locked`,
  but this macOS host lacks `x86_64-w64-mingw32-gcc`; hosted Windows CI remains
  the authoritative validation for the Windows path.
- The next hosted Platform CI run compiled through the witness fix, then failed
  macOS in `resume-daemon --test s4_daemon` because two daemon integration tests
  exceeded the test harness's 8 second child-process wait on the hosted runner.
  The daemon was still making progress; the fix raises the test harness wait
  budget to 45 seconds while leaving the product `--max-worker-ticks` settings
  unchanged.
- That hosted run failed Windows in `benchmark-runner --test s17_benchmark_cli`
  because OCR and embedding benchmark fixtures still generated Unix shell
  scripts. The fix adds Windows `.cmd` fixtures for the same local command
  protocols in both CLI-level and runner-level benchmark tests.
- Local focused verification after the second fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli --locked`,
  `/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner --locked`,
  and
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked`
  passed.
- The next hosted Windows Platform CI run reached `resume-cli --test
  s14_delete_search` and failed three reimport deletion assertions. Root cause:
  deletion propagation compared stored slash-normalized document paths like
  `d:/...` against native Windows `PathBuf` roots with `Path::starts_with`,
  so missing files were not consistently recognized under the import root on
  Windows.
- The fix now normalizes import roots with `fs_crawler::normalize_path` and
  compares normalized path boundaries for root/skipped-subtree checks.
- Local focused verification after the Windows path fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked` and
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked`
  passed.
- Red/green verification: with the old native `Path::starts_with` comparison
  temporarily restored, `/Users/frankqdwang/.cargo/bin/cargo test -p
  import-pipeline deletion_candidate_matches_windows_normalized_paths --locked`
  failed on the Windows-style normalized path assertion; after restoring the
  fix, the same focused regression, `s14_delete_search`, `cargo fmt --check`,
  public guard/marker scans, and `./scripts/ci/verify-local.sh` passed.
- The next hosted Windows Platform CI run passed the deletion assertions but
  failed three `s14_delete_search` cases during initial CLI import with the
  redacted error `resume-cli: search index update failed`; macOS, Rust
  workspace, and all policy checks passed in the same run.
- Root cause: full-text snapshot publishing validated by opening a reader on the
  staging directory, then immediately renamed that same staging directory. That
  is fragile on Windows where recently opened index files can remain locked
  briefly after handles are dropped.
- The fix now publishes the staging snapshot to the immutable snapshots
  directory before validation, validates the published snapshot before moving
  the active pointer, removes a failed published snapshot best-effort, and
  retries transient publish locks.
- Local focused verification after the full-text publish fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked` and
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked`
  passed.
- The next hosted Windows Platform CI run passed `s14_delete_search` and then
  failed witness tests in `s9_import_search` after successful import/OCR
  summaries because private witness temp data cleanup reported `cleanup_failed`.
- Root cause: the witness command attempted to delete the temporary private data
  root while metadata/index handles could still be open; Unix tolerated that,
  but Windows does not delete open files/directories.
- The fix now drops the witness metadata store before cleanup and retries
  temporary witness root deletion to absorb transient Windows handle release.
- Local focused verification after the witness cleanup fix:
  `/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked`,
  `cargo fmt --check`, guard/marker scans, and `./scripts/ci/verify-local.sh`
  passed.
- Final hosted PR validation for run `26932238947` / `26932238946` /
  `26932238956`: `macos-latest` passed in 1m30s, `windows-latest` passed in
  4m14s, `rust workspace` passed in 1m18s, and dependency tree, license policy,
  runbook policy, and public repository guard all passed.

Scope note:

- S112 improves hosted cross-platform build/test coverage only. It does not
  prove platform installer behavior, service manager behavior, signing,
  notarization, upgrade, uninstall, rollback, real whole-machine scans, or
  complete release readiness.
- Full product is still not complete.

### S111

Design target:

- Wire the S110 vector-quality evaluator/gate into PR and nightly benchmark
  smoke workflows.
- Keep the workflow smoke local-only, synthetic-labeled, redacted, and explicit
  that it is not proof of production semantic quality.
- Extend the workflow policy guard so future edits cannot silently drop the
  vector smoke gate.

Observed RED:

```bash
./scripts/ci/check-workflows.sh
```

Output summary:

- The workflow guard failed because `.github/workflows/pr.yml` was missing the
  required `resume-benchmark --locked -- vector-quality` command.

Implementation checks:

```bash
./scripts/ci/check-workflows.sh
sh -n scripts/ci/check-workflows.sh
ruby -e 'require "yaml"; ARGV.each { |file| YAML.load_file(file); puts "yaml ok: #{file}" }' .github/workflows/pr.yml .github/workflows/bench-nightly.yml
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Workflow guard: exit 0 after PR and nightly workflows were wired to run
  `vector-quality` and `vector-gate`.
- Strict local vector smoke reproduction: exit 0; `vector quality gate passed`,
  and the generated report did not contain the temporary command path, raw
  queries, candidate text, candidate IDs, or vector values.
- Shell syntax check: exit 0.
- Workflow YAML parse: exit 0 for PR and nightly workflow files.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, workflow check, and
  public repo guard.

Scope note:

- S111 adds CI coverage for the benchmark gate path only. It does not provide
  real business relevance labels, choose or license a production embedding
  model, prove ANN behavior, prove 100k/1M semantic latency, or complete product
  readiness.
- Full product is still not complete.

### S110

Design target:

- Add a redacted labeled vector-quality evaluator and gate that use the existing
  local embedding command protocol.
- Score recall@k, MRR, NDCG@k, and zero-recall query count from JSONL samples.
- Keep reports free of raw queries, candidate text, sample IDs, candidate IDs,
  vectors, command paths, resume paths, and real filenames.
- Keep private PDF/Word witness validation local-only and bounded.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner vector_quality_report_scores_labeled_samples_without_text_id_path_or_vector_leakage --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_vector_quality_outputs_redacted_report_and_gate --locked -- --exact
```

Output summary:

- The runner exact failed because `VectorQualityConfig`,
  `VectorQualityGateConfig`, `run_vector_quality_jsonl`, and
  `evaluate_vector_quality_gate_json` did not exist.
- The CLI exact failed because `resume-benchmark` rejected the
  `vector-quality` command.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner vector_quality_report_scores_labeled_samples_without_text_id_path_or_vector_leakage --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_vector_quality_outputs_redacted_report_and_gate --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
```

Output summary:

- Focused vector-quality runner exact: exit 0.
- Focused vector-quality CLI exact: exit 0.
- `benchmark-runner`: exit 0; full crate tests passed, including vector gate
  acceptance/rejection and redaction coverage.
- Focused benchmark-runner clippy: exit 0.
- A private local-only bounded PDF/Word witness against the user-authorized
  sample directory completed with redacted aggregate output and temporary
  private data removal.
- A private local-only bounded OCR witness completed with redacted processed and
  failed document counters, explicit OCR budget exhaustion reporting, and
  temporary private data removal.
- A private local-only Word-only witness completed with redacted aggregate
  output and temporary private data removal.
- No real resume path, filename, raw text, OCR text, command path, count, or
  diagnostic payload was committed or uploaded.

Scope note:

- S110 adds a quality gate surface and redaction boundary for labeled vector
  retrieval evaluation. It does not choose a licensed embedding model, ship a
  model pack, provide real business relevance labels, add ANN indexing, prove
  production semantic latency, or complete product readiness.
- Full product is still not complete.

### S109

Design target:

- Make `resume-cli witness --run-ocr --ocr-max-documents <n>` useful against a
  real private resume directory when one OCR document fails before the budget is
  exhausted.
- Count OCR document attempts as successful plus failed documents, continue
  through the configured document budget after per-document OCR failures, and
  report redacted aggregate `ocr documents failed` output.
- Preserve the existing `blocked` status when no OCR command is configured, and
  do not print real paths, filenames, OCR text, command paths, or diagnostics.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_budget_reports_failed_documents_without_stopping_or_leaking_paths --locked -- --exact
```

Output summary:

- The test failed because `resume-cli witness` still reported
  `witness ocr status: blocked` after the first OCR failure instead of
  completing the bounded witness and reporting a failed-document counter.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_budget_reports_failed_documents_without_stopping_or_leaking_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused OCR witness resilience exact: exit 0.
- `s9_import_search`: exit 0; 23 tests passed.
- `cargo fmt --check`: exit 0.
- Focused CLI clippy: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, workflow check, and
  public repo guard.
- A private local-only full-directory PDF/Word witness using the
  user-authorized sample directory passed with redacted aggregate output and
  temporary private data removal.
- A private local-only bounded OCR witness using local renderer/OCR runtimes
  completed with redacted processed and failed document counters, explicit OCR
  budget exhaustion reporting, and temporary private data removal.
- No real resume path, filename, raw text, OCR text, count, command path, or
  diagnostic payload was committed or uploaded.

Scope note:

- S109 makes bounded real-library OCR witnessing more resilient. It does not
  prove OCR quality, full-library OCR completion, non-English OCR behavior,
  packaged runtime distribution, 100k/1M performance, Windows/Linux behavior,
  or complete product readiness.
- Full product is still not complete.

### S108

Design target:

- Wire the synthetic OCR throughput benchmark/gate from S107 into PR and nightly
  benchmark smoke workflows.
- Add a workflow policy guard so required query and OCR benchmark smoke gates
  cannot silently disappear from workflows or local verification.
- Keep workflow artifacts redacted; no real resume paths, raw resume text,
  diagnostics, or local data are uploaded.

Observed RED:

```bash
sh scripts/ci/check-workflows.sh
```

Output summary:

- The new workflow policy guard failed because `.github/workflows/pr.yml` did
  not include `resume-benchmark --locked -- ocr-throughput`.

Implementation checks:

```bash
sh scripts/ci/check-workflows.sh
tmpdir=$(mktemp -d); trap 'rm -rf "$tmpdir"' EXIT; printf '%s\n' '#!/usr/bin/env sh' 'printf "resume-ir-ocr-v1\nconfidence=0.97\ntext:\nSynthetic OCR smoke page %s\n" "$RESUME_IR_OCR_PAGE_NO"' > "$tmpdir/ocr-fixture.sh"; chmod 700 "$tmpdir/ocr-fixture.sh"; /Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- ocr-throughput --command "$tmpdir/ocr-fixture.sh" --pages 3 --page-timeout-ms 5000 --json > "$tmpdir/ocr-benchmark-smoke.json"; /Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- ocr-gate --report "$tmpdir/ocr-benchmark-smoke.json" --allow-synthetic --min-pages 3 --max-p95-ms 5000 --min-pages-per-second 0.001; if rg -n 'Synthetic OCR smoke|resume-ir-ocr-v1|RESUME_IR_OCR|/tmp/' "$tmpdir/ocr-benchmark-smoke.json"; then exit 1; fi
sh -n scripts/ci/check-workflows.sh scripts/ci/verify-local.sh scripts/ci/check-runbooks.sh scripts/ci/guard-public-repo.sh
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- Workflow policy guard: exit 0.
- Synthetic local OCR benchmark smoke and gate: exit 0; redacted report did not
  include synthetic OCR text, OCR protocol text, OCR environment names, or temp
  paths.
- Shell syntax checks: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, workflow check, and
  public repo guard.

Scope note:

- S108 adds workflow enforcement for synthetic OCR smoke only. It does not
  prove real scanned-resume OCR quality, full-library OCR completion,
  non-English language behavior, packaged OCR runtime distribution, 100k/1M
  corpus performance, or Windows/Linux validation.
- Full product is still not complete.

### S107

Design target:

- Add `resume-benchmark ocr-throughput` so the benchmark runner can measure
  synthetic OCR page throughput through the existing local OCR command protocol
  or Tesseract adapter without touching real resumes.
- Add `resume-benchmark ocr-gate` so synthetic OCR reports require explicit
  `--allow-synthetic` before they can pass a gate.
- Keep reports redacted: no raw OCR text, page bytes, command paths, resume
  paths, sample IDs, or private data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner synthetic_ocr_throughput_reports_page_latency_without_payload_or_path_leakage --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_ocr_throughput_outputs_redacted_report_and_gate --locked -- --exact
```

Output summary:

- The library test failed because OCR throughput API symbols did not exist.
- The CLI test failed because `resume-benchmark` rejected `ocr-throughput` as
  unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner synthetic_ocr_throughput_reports_page_latency_without_payload_or_path_leakage -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_ocr_throughput_outputs_redacted_report_and_gate -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused OCR throughput library exact: exit 0.
- Focused OCR throughput CLI exact: exit 0.
- `benchmark-runner`: exit 0; 19 integration tests plus doc-tests passed.
- Focused benchmark-runner clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S107 proves only a synthetic OCR throughput report/gate path that exercises
  existing local OCR clients without leaking payloads or paths.
- It does not prove real scanned-resume OCR quality, full-library OCR
  completion, non-English language behavior, packaged OCR runtime distribution,
  100k/1M corpus performance, or Windows/Linux validation.
- Full product is still not complete.

### S106

Design target:

- Add `resume-cli witness --root-preset local-discovery` so the local witness
  command can exercise the same root-preset discovery path users need when they
  do not know where resumes are stored.
- Use the existing discovery profile skip rules for system/cache/dependency
  directories and keep output redacted.
- Continue anonymizing selected PDF/Word inputs into a temporary witness data
  directory and remove private witness data before returning.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_local_discovery_preset_uses_discovery_profile_without_path_leak --locked -- --exact
```

Output summary:

- The test failed because `resume-cli witness` rejected
  `--root-preset local-discovery` as unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_local_discovery_preset_uses_discovery_profile_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p fs-crawler --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused local-discovery witness exact: exit 0.
- `s9_import_search`: exit 0; 22 tests passed.
- `fs-crawler`: exit 0; 11 tests passed plus doc-tests.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- A private local-only local-discovery witness using the user-authorized sample
  directory override passed with redacted aggregate output and temporary
  private data removal. No real resume path, filename, raw text, or diagnostic
  payload was committed or uploaded.

Scope note:

- S106 makes local-discovery witnessing possible without pretending to prove a
  full default whole-machine scan, Windows drive behavior, full-library OCR,
  OCR quality, or large-corpus performance.
- Full product is still not complete.

### S105

Design target:

- Add `resume-cli witness --run-ocr --ocr-max-documents <n>` so a private
  local root can be scanned/imported at its full witness file budget while OCR
  execution is independently bounded.
- Preserve the existing real OCR worker path for each processed document and
  output only aggregate redacted counters.
- Report whether the OCR document budget was exhausted; do not imply full OCR
  completion when queued OCR work remains.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_can_budget_documents_after_full_private_scan_without_path_leak --locked -- --exact
```

Output summary:

- The test failed because `resume-cli witness` rejected
  `--ocr-max-documents` as unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_can_budget_documents_after_full_private_scan_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused OCR witness-budget exact: exit 0.
- `s9_import_search`: exit 0; 21 tests passed.
- `s15_ocr_handoff`: exit 0; 12 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- A private local-only full-directory witness using the user-authorized sample
  directory and local OCR runtimes passed with a bounded OCR document budget,
  redacted aggregate output, explicit OCR budget exhaustion reporting, and
  temporary private data removal. No real resume path, filename, raw text, or
  diagnostic payload was committed or uploaded.

Scope note:

- S105 makes full-root local OCR witnessing practical without pretending to
  complete full-library OCR. It does not prove OCR quality, throughput,
  non-English behavior, packaged runtime distribution, Windows/Linux behavior,
  or large-corpus performance.
- Full product is still not complete.

### S104

Design target:

- Add a safe `resume-cli fault-simulate --case migration-failure` probe that
  creates a synthetic broken migration-state SQLite database under scratch,
  invokes the real `MetaStore::run_migrations()` path, and removes probe data.
- Report only redacted aggregate output: fault name, reproduced status,
  migration check state, recovery guidance, and `paths: <redacted>`.
- Do not touch the caller's data directory and do not print paths, schema SQL,
  table names, raw SQLite errors, or resume data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_metadata_migration_failure_reproduces_without_path_or_schema_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked -- --exact
```

Output summary:

- The fault simulation exact failed because `migration-failure` was rejected as
  unsupported usage.
- The diagnostics exact failed because the redacted diagnostics skeleton did not
  include `metadata_migration`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_metadata_migration_failure_reproduces_without_path_or_schema_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused migration-failure fault simulation exact: exit 0.
- Focused diagnostics exact: exit 0.
- `s71_fault_injection`: exit 0; 10 tests passed.
- `s13_diagnostics`: exit 0; 11 tests passed.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S104 adds a safe synthetic migration failure fault simulation and updates
  doctor/diagnostics hook listings. It does not run destructive migration
  rollback drills on real metadata, prove backup/restore operations, prove
  cross-platform filesystem fault behavior, or complete upgrade rehearsal.
- Full product is still not complete.

### S103

Design target:

- Add a non-contact soft-dedupe scorer for same-name profiles using school,
  company, and skill overlap as bounded evidence.
- Surface low-confidence suspected-duplicate hints in local CLI and daemon
  full-text search results without assigning `candidate_id` or folding those
  versions.
- Output only aggregate hint data: suspected version count, maximum confidence,
  and `folded=false`; do not output raw names, schools, companies, contacts,
  paths, or dedupe keys.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion soft_dedupe --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding search_marks_soft_duplicate_hints_without_low_confidence_folding --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_includes_redacted_soft_dedupe_hints --locked -- --exact
```

Output summary:

- The rank-fusion test failed because `DedupeProfile` and
  `soft_dedupe_score` did not exist.
- The local CLI test failed because search output had no
  `soft_dedupe: suspected_versions=...` hint line.
- The daemon IPC test failed because search result JSON had no `soft_dedupe`
  object.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --test s10_rank_fusion soft_dedupe --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding search_marks_soft_duplicate_hints_without_low_confidence_folding --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc daemon_search_ipc_includes_redacted_soft_dedupe_hints --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p rank-fusion --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s18_candidate_folding --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s48_search_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p rank-fusion -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- Focused rank-fusion RED/GREEN: exit 0 after implementation.
- Focused local CLI soft-dedupe RED/GREEN: exit 0 after implementation.
- Focused daemon IPC soft-dedupe RED/GREEN: exit 0 after implementation.
- `rank-fusion`: exit 0; 7 tests passed plus doc-tests.
- CLI candidate-folding suite: exit 0; 2 tests passed.
- Daemon search IPC suite: exit 0; 5 tests passed.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S103 adds bounded soft-dedupe scoring and redacted search hints. It does not
  strong-fold low-confidence matches, does not persist manual merge decisions,
  does not prove real dedupe precision/recall, and does not prove million-corpus
  latency.
- Full product is still not complete.

### S102

Design target:

- Add `resume-benchmark field-quality --dataset <jsonl> --json` for labeled
  field extraction quality evaluation.
- Add `resume-benchmark field-gate --report <path>` with configurable minimum
  sample count, precision, recall, and F1 thresholds.
- Output only aggregate metrics; do not output raw resume text, sample IDs,
  paths, expected values, predicted values, email addresses, or phone numbers.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality --locked
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_quality_outputs_redacted_report_and_gate --locked -- --exact
```

Output summary:

- The library test failed because `evaluate_field_quality_gate_json`,
  `run_field_quality_jsonl`, and `FieldQualityGateConfig` did not exist.
- The CLI exact failed because `resume-benchmark` rejected `field-quality` as
  unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_runner field_quality --locked
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --test s17_benchmark_cli resume_benchmark_field_quality_outputs_redacted_report_and_gate --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
./scripts/ci/check-licenses.sh
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_ipc_connect_failure_does_not_fallback_to_sqlite --locked -- --exact --nocapture
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s21_import_candidate_assignment --locked
./scripts/ci/verify-local.sh
```

Output summary:

- Focused field-quality library tests: exit 0; 3 tests passed.
- Focused field-quality CLI exact: exit 0.
- `benchmark-runner`: exit 0; 13 tests passed plus doc-tests.
- Focused benchmark-runner clippy: exit 0.
- License guard: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- An initial full verify attempt hit an existing IPC connect-failure test
  failure; its exact rerun passed.
- A later full verify attempt exposed an existing flaky contact-hash key test
  assertion that rejected any random key containing the short digit fragment
  `415`; the assertion was hardened to check full synthetic contact strings,
  and `s21_import_candidate_assignment` then passed.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S102 adds the evaluator/gate needed to measure field precision/recall/F1. It
  does not provide real business labeled datasets, does not prove production
  field F1 targets, does not broaden dictionaries, and does not complete
  soft-dedupe scoring.
- Full product is still not complete.

### S101

Design target:

- Add `resume-daemon run --foreground --work-imports --watch-import-roots`.
- Watch latest completed import roots through a real local filesystem watcher,
  aggregate relevant create/modify/remove events, and requeue the affected root
  through the existing durable import task plus scan-scope path.
- Print only aggregate watcher counts; do not print source roots, event paths,
  filenames, or notify error details.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_watcher_requeues_completed_root_after_file_change_without_path_leak --locked -- --exact
```

Output summary:

- The test failed because `resume-daemon` rejected `--watch-import-roots` as
  unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_watcher_requeues_completed_root_after_file_change_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon --all-targets --locked -- -D warnings
./scripts/ci/check-licenses.sh
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused import watcher exact: exit 0.
- `s4_daemon`: exit 0; 12 tests passed.
- Focused daemon clippy: exit 0.
- License guard: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S101 adds local OS watcher event-to-import-task integration for completed
  roots. It does not prove Windows watcher behavior, long-running watcher soak
  stability, large-corpus event storms, or incremental index-update-only writes.
- Full product is still not complete.

### S100

Design target:

- Add optional `resume-cli witness --run-ocr` support that reuses the existing
  OCR worker path inside the isolated witness data directory.
- Accept local OCR command/Tesseract and renderer/pdftoppm options without
  printing command paths, rendered bytes, OCR text, source paths, filenames, or
  diagnostics.
- Report `completed` aggregate OCR work when local OCR executes, or explicit
  `blocked` aggregate output when OCR is requested but no local OCR command is
  configured. Always remove private witness input/data directories.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_executes_local_command_without_output_or_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_without_command_reports_blocked_without_persisting_private_data --locked -- --exact
```

Output summary:

- Both tests failed because `resume-cli witness` rejected `--run-ocr` and OCR
  options as unsupported usage.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_executes_local_command_without_output_or_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_run_ocr_without_command_reports_blocked_without_persisting_private_data --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused witness OCR completed exact: exit 0.
- Focused witness OCR blocked exact: exit 0.
- `s9_import_search`: exit 0; 20 tests passed.
- `s15_ocr_handoff`: exit 0; 12 tests passed.
- `cargo fmt --check`: exit 0.
- Focused CLI clippy: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- Private bounded local-only OCR witness using the user-authorized sample
  directory without an OCR command: exit 0 with explicit redacted `blocked`
  output and no metadata persisted in the external data directory.
- Private bounded local-only OCR witness using local OCR runtime commands:
  exit 0 with redacted `completed` output and no metadata persisted in the
  external data directory. This run used a file budget and did not prove
  full-library OCR coverage or throughput.

Scope note:

- S100 adds witness-level OCR execution/blocked reporting. It does not package
  OCR runtimes, prove non-English OCR, prove full-library OCR, prove
  large-corpus OCR throughput, or validate Windows/Linux.
- Full product is still not complete.

### S99

Design target:

- Add `resume-cli witness --root <path> [--max-files <count>]` for
  user-authorized local-only PDF/Word validation.
- Select only PDF/DOCX/DOC inputs, copy them under anonymized temporary
  filenames, run the existing import/index path in an isolated temporary data
  directory, and remove the temporary private input and data directories before
  returning.
- Print only aggregate redacted output; do not print source paths, filenames,
  resume text, diagnostics, or user sample counts in committed artifacts.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_imports_only_pdf_and_word_samples_without_persisting_private_data --locked -- --exact
```

Output summary:

- The test failed because the CLI rejected `witness` as an unknown top-level
  command.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search witness_imports_only_pdf_and_word_samples_without_persisting_private_data --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s9_import_search --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension|[p]rivate-sample-path-marker' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused witness exact: exit 0.
- `s9_import_search`: exit 0; 18 tests passed.
- `cargo fmt --check`: exit 0.
- Focused CLI clippy: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- Private local-only PDF/Word witness using the user-authorized sample directory:
  exit 0 with redacted output, no scan-budget exhaustion at the default witness
  budget, and no metadata persisted in the external data directory. No real
  resume path, filename, count, raw text, or diagnostic payload was committed or
  uploaded.

Scope note:

- S99 adds a privacy-preserving local witness command for PDF/Word validation.
  It does not prove production-scale performance, complete converter/OCR/model
  packaging, validate Windows/Linux, or replace the remaining full-library
  quality gates.
- Full product is still not complete.

### S98

Design target:

- Add a cross-platform polling background import-rescan mode for completed
  import roots.
- Preserve import task history by creating a new queued task from the latest
  completed root scan scope, only when the same root has no queued/running/
  retryable task.
- Keep worker output redacted; do not print root paths, data directories, or
  filenames.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_rescans_completed_root_without_path_leak --locked -- --exact
```

Output summary:

- The test failed because `resume-daemon` rejected
  `--rescan-completed-imports` as an unknown usage path.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon foreground_import_scheduler_rescans_completed_root_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-daemon -p meta-store --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused daemon import-rescan exact: exit 0.
- `s4_daemon`: exit 0; 11 tests passed.
- `s3_sqlite`: exit 0; 43 tests passed.
- Focused daemon/meta-store clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S98 implements polling background rescan for completed roots. It does not
  implement a native OS filesystem watcher, prove long-running full-library
  rescans, or replace full snapshot rebuilds with incremental index writes.
- Full product is still not complete.

### S97

Design target:

- Treat legacy Word `.doc` as Word input rather than permanently failing it
  before parsing.
- Use a local converter with private temp input/output files, fixed timeout,
  bounded output size, hidden stdout/stderr, and redacted debug surfaces.
- Keep synthetic tests as the committed proof; use real samples only as
  uncommitted local witness data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p parser-doc --test s6_doc extracts_legacy_doc_text_with_local_converter_without_output_leakage --locked -- --exact
```

Output summary:

- The test failed because `DocParser::with_converter` was not implemented.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p parser-doc --test s6_doc extracts_legacy_doc_text_with_local_converter_without_output_leakage --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline tests::import_root_parses_legacy_doc_with_local_converter_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p parser-doc --locked
/Users/frankqdwang/.cargo/bin/cargo test -p parser-common --locked
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p parser-doc -p import-pipeline --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n --hidden --glob '!target/**' --glob '!.git/**' '[r]esume-ir-real-witness|[s]elected_pdf|[s]elected_docx|[s]elected_doc|[d]ocument_status_by_extension' .; then exit 1; else echo "no private witness markers"; fi
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused legacy DOC parser exact: exit 0.
- Focused import-pipeline legacy DOC exact: exit 0.
- `parser-doc`: exit 0; 2 tests passed.
- `parser-common`: exit 0; 7 tests passed.
- `import-pipeline`: exit 0; 6 tests passed.
- Focused parser/import clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Private witness marker scan: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.
- Private local-only witness using anonymized temporary PDF/DOCX/DOC copies:
  DOCX imported as searchable, text-layer/scanned PDF routed to OCR as expected,
  most legacy DOC samples became searchable through the local converter, and one
  DOC sample remained a safe permanent failure. No real resume path, filename,
  count, raw text, or diagnostic payload was committed or uploaded.

Scope note:

- S97 adds legacy `.doc` support through a local converter path. It does not
  finish converter packaging/distribution, Windows/Linux converter proof, full
  OCR completion for scanned PDFs, large-corpus proof, or full-library
  validation.
- Full product is still not complete.

### S96

Design target:

- Report local OCR runtime availability in `resume-cli doctor` and
  `resume-cli export-diagnostics --redact` without leaking binary paths,
  command output, language dumps, or resume data.
- Check `pdftoppm`, Tesseract, and the `eng` Tesseract language pack through
  local-only process inspection. Tests use temporary synthetic executables on
  `PATH`, not real resumes or network calls.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_ocr_runtime_without_paths_or_language_dump --locked -- --exact
```

Output summary:

- The test failed because doctor output did not contain
  `ocr renderer pdftoppm: available`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_ocr_runtime_without_paths_or_language_dump --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_reports_non_executable_ocr_tools_as_missing_without_paths --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused OCR runtime diagnostics exact: exit 0.
- Focused non-executable OCR runtime exact: exit 0.
- `s13_diagnostics`: exit 0; 11 tests passed, including redacted OCR runtime
  availability and non-executable tool handling without path or language-list
  leakage.
- Focused CLI clippy: exit 0.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- Runbook guard: exit 0.
- Public repository guard: exit 0.
- Obsolete reference marker guard: exit 0.
- `./scripts/ci/verify-local.sh`: exit 0, including metadata, fmt, workspace
  clippy/tests/doc-tests, license check, runbook check, and public repo guard.

Scope note:

- S96 reports local OCR runtime availability only. It does not implement final
  OCR/renderer distribution policy, non-English language-pack install/selection
  policy, real scanned-resume witness runs, large-corpus OCR throughput proof,
  or Windows/macOS validation.
- Full product is still not complete.

### S95

Design target:

- Persist an enum-only OCR job failure reason for scanned PDFs blocked by the
  local page budget. Do not persist raw worker stderr, local paths, commands,
  resume text, or OCR payloads as failure diagnostics.
- Surface aggregate remediation through `resume-cli status`, daemon status IPC,
  `resume-cli doctor`, and `resume-cli export-diagnostics --redact`.
- Keep over-budget documents non-searchable, avoid renderer/OCR invocation, and
  preserve the S94 no-partial-cache/no-partial-index behavior.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
```

Output summary:

- The test failed after adding status/doctor/diagnostics expectations because
  `resume-cli status` did not report `ocr page budget blocked: 1`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite ocr_job_failure_kind_persists_reports_and_clears_on_retry_claim --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc status_can_read_redacted_daemon_status_over_loopback_ipc --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli --locked
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- Focused meta-store, CLI, daemon, and IPC checks passed after implementation.
- `s3_sqlite`: exit 0; 43 tests passed, including schema v16, persisted
  `OcrPageBudgetExceeded`, aggregate blocked count, and clearing stale failure
  kind when the job is reclaimed.
- `s15_ocr_handoff`: exit 0; 12 tests passed, including local
  status/doctor/redacted diagnostics reporting the page-budget block without
  path, command, marker, or OCR payload leakage.
- `s50_ocr_worker`: exit 0; 8 tests passed, including daemon page-budget
  failure-kind persistence.
- `s20_status_ipc`: exit 0; 6 tests passed, including daemon status IPC
  rendering of the aggregate blocked count and remediation text.
- `cargo fmt --check`: exit 0.
- Focused clippy: exit 0.
- `s13_diagnostics`: exit 0; 9 tests passed after the diagnostics output
  changes.
- `s4_cli`: exit 0; 6 tests passed after the status output changes.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker guard: exit 0 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S95 adds redacted visibility/remediation for over-budget OCR documents. It
  does not implement real scanned-resume witness runs, large-corpus OCR
  throughput proof, final OCR/renderer distribution policy, non-English
  language-pack policy, or Windows/macOS validation.
- Full product is still not complete.

### S94

Design target:

- Add OCR page-count backpressure for scanned PDFs so a single oversized document
  cannot trigger unbounded local rendering/OCR work.
- Apply the guard to both `resume-cli ocr-worker` and `resume-daemon run
  --work-ocr*`; expose `--max-pages-per-document` on the CLI worker and
  `--ocr-max-pages-per-document` on the daemon. The macOS service install path
  can pass the daemon budget into LaunchAgent ProgramArguments.
- When a document exceeds the limit, do not invoke renderer/OCR, do not write
  partial OCR cache entries, do not index partial text, and keep paths/payloads
  out of output.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr --locked -- --exact
```

Output summary:

- CLI test failed because the worker did not recognize
  `--max-pages-per-document`, returning usage instead of the backpressure error.
- Daemon test failed because `resume-daemon run` did not recognize
  `--ocr-max-pages-per-document`, returning usage instead of reporting one OCR
  worker failure.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- `s15_ocr_handoff`: exit 0; 12 tests passed, including CLI OCR
  backpressure before renderer/OCR invocation.
- `s50_ocr_worker`: exit 0; 8 tests passed, including daemon OCR
  backpressure before renderer/OCR invocation.
- `s66_service_lifecycle`: exit 0; 4 tests passed, including LaunchAgent
  ProgramArguments carrying `--ocr-max-pages-per-document` without stdout path
  leakage.
- `cargo fmt --check`: exit 0.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker guard: exit 0 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S94 prevents over-budget OCR execution and partial indexing. S95 later adds
  redacted user-facing remediation diagnostics. Large-corpus OCR throughput
  proof, real scanned-resume witness runs, OCR/renderer distribution policy,
  non-English language-pack policy, and Windows/macOS validation remain not
  complete or BLOCKED.
- Full product is still not complete.

### S93

Design target:

- Persist OCR word bounding boxes from local Tesseract TSV output into the local
  OCR page cache without putting OCR payloads or file paths into debug/user
  output.
- Keep the existing custom OCR command protocol compatible with empty word boxes;
  only concrete OCR engines that return boxes populate the metadata.
- Prove the path with synthetic fixtures only: OCR client parses word boxes,
  meta-store round-trips redacted word-box cache metadata, and CLI/daemon
  Tesseract worker paths write boxes into cache before search indexing.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client tesseract_worker_recognizes_synthetic_image_without_payload_debug_leaks --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite ocr_page_cache_persists_word_boxes_without_debug_payload_leak --locked -- --exact
```

Output summary:

- `ocr-client` failed with exit 101 because `OcrPage::word_boxes()` did not
  exist.
- `meta-store` failed with exit 101 because `OcrWordBox`,
  `OcrPageCacheEntry::succeeded_with_word_boxes`, and
  `OcrPageCacheEntry::word_boxes()` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --test s3_sqlite --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p meta-store -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
if rg -n "schema_version\\(\\)\\.unwrap\\(\\), 14|\\[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14\\]" crates/meta-store/tests/s3_sqlite.rs; then exit 1; else echo "no stale schema version expectations"; fi
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
if rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .; then exit 1; else echo "no obsolete reference markers"; fi
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client` test suite: exit 0; 17 tests passed, including real Tesseract
  recognition of a synthetic image and word-box parsing for the `S92` token.
- `meta-store` SQLite suite: exit 0; 42 tests passed, including schema V15 and
  OCR word-box cache round-trip with redacted Debug output.
- `s15_ocr_handoff`: exit 0; 11 tests passed, including CLI Tesseract worker
  cache word-box persistence and search indexing.
- `s50_ocr_worker`: exit 0; 7 tests passed, including daemon Tesseract worker
  cache word-box persistence and search indexing.
- `cargo fmt --check`: exit 0 after formatting.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- Schema expectation guard: exit 0 with no stale schema-version expectations.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker guard: exit 0 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S93 stores OCR word boxes locally in OCR cache rows and keeps them out of
  Debug. It does not add bbox-aware retrieval/ranking UI, final OCR distribution,
  non-English language-pack policy, real scanned-resume witness proof,
  large-corpus OCR throughput, or Windows/macOS validation.
- Full product is still not complete.

### S92

Design target:

- Install and validate a concrete local OCR recognition engine for English
  synthetic OCR witness runs. Homebrew installed `tesseract 5.5.2`; local
  `brew info --json=v2 tesseract` reports license `Apache-2.0`, and the
  installation includes a local LICENSE file.
- Add a Tesseract OCR client that writes private temp image input, runs
  `tesseract <image> stdout --psm 6 -l <lang> tsv`, parses TSV word text plus
  average confidence, and redacts payloads/paths from debug and user-visible
  output.
- Wire `resume-cli ocr-worker --tesseract-command` and
  `resume-daemon run --ocr-tesseract-command`, mutually exclusive with the
  existing custom OCR command protocol.
- Prove worker-level cache/search integration with synthetic images rendered
  by a local fixture and recognized by real Tesseract. Use synthetic fixtures
  only; do not scan real resumes.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client tesseract_worker_recognizes_synthetic_image_without_payload_debug_leaks -- --exact
```

Output summary:

- Exit 101 before implementation because `TesseractOcrClient` and
  `TesseractOcrSpec` were unresolved imports.

Additional wiring RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_uses_tesseract_for_rendered_image_before_indexing --locked -- --exact
```

Output summary:

- Exit 101 after adding the daemon integration test because the daemon startup
  guard still required `ocr_command` and did not accept `ocr_tesseract_command`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client` test suite: exit 0; 17 tests passed, including real Tesseract
  recognition of a synthetic image rendered in memory by the test.
- `s15_ocr_handoff`: exit 0; 11 tests passed, including CLI worker handoff
  through a rendered synthetic image into real Tesseract, OCR page cache, and
  full-text search without token/path leakage.
- `s50_ocr_worker`: exit 0; 7 tests passed, including daemon one-shot worker
  handoff through a rendered synthetic image into real Tesseract, OCR page
  cache, and full-text search without token/path leakage.
- `cargo fmt --check`: exit 0 after formatting.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S92 proves a real local Tesseract recognition engine path for English
  synthetic OCR. It does not claim final distribution packaging, non-English
  language packs, OCR bounding-box persistence, real scanned-resume witness
  proof, large-corpus OCR throughput, or Windows/macOS validation.
- Full product is still not complete.

### S91

Design target:

- Add a concrete local Poppler `pdftoppm` PDF page renderer adapter that writes
  private temp PDF input and private temp PPM output, bounds captured output,
  observes timeout/cancellation, and keeps payloads/paths out of debug and
  user-visible output.
- Wire the renderer through `resume-cli ocr-worker --pdftoppm-command` and
  `resume-daemon run --ocr-pdftoppm-command`, with mutual exclusion against the
  existing generic render-command path.
- Prove the path with valid synthetic PDF bytes rendered to PPM before the OCR
  command receives the page input. Install `poppler-utils` in PR CI so hosted
  tests exercise the real renderer instead of skipping for a missing binary.
- Use synthetic fixtures only; do not claim Tesseract or real OCR recognition
  engine completion from this slice.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client pdftoppm_renderer_renders_valid_pdf_page_to_ppm_without_payload_debug_leaks --locked -- --exact
```

Output summary:

- Exit 101 before implementation because `PdftoppmPdfRenderer` and
  `PdftoppmRenderSpec` were unresolved imports.

Additional wiring RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker daemon_ocr_worker_once_uses_pdftoppm_renderer_for_valid_pdf_before_ocr --locked -- --exact
```

Output summary:

- Exit 101 after adding the daemon integration test because `RunOptions` did
  not yet have `ocr_pdftoppm_command`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client` test suite: exit 0; 16 tests passed, including the Poppler
  `pdftoppm` renderer witness that produced a `P6` PPM page from a valid
  synthetic PDF.
- `s15_ocr_handoff`: exit 0; 10 tests passed, including CLI worker handoff
  from `pdftoppm` PPM bytes to OCR command/cache/search without token/path
  leakage.
- `s50_ocr_worker`: exit 0; 6 tests passed, including daemon one-shot worker
  handoff from `pdftoppm` PPM bytes to OCR command/cache/search without
  token/path leakage.
- `cargo fmt --check`: exit 0 after formatting.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S91 proves the local Poppler renderer adapter and CLI/daemon worker wiring
  on valid synthetic PDFs when `pdftoppm` is installed. It does not install,
  select, or license-review a real OCR recognition engine; local OCR text
  recognition remains through the existing command protocol and synthetic test
  commands.
- It does not persist OCR bounding boxes, prove behavior on real resumes, prove
  large-corpus OCR throughput, define final renderer/OCR distribution policy,
  or validate Windows/macOS behavior.
- Full product is still not complete.

### S90

Design target:

- Extend `resume-cli purge --deleted` so tombstoned-document cleanup also
  removes current ingest jobs and OCR page-cache entries associated with the
  purged documents.
- Keep cache deletion content-hash scoped, but preserve shared OCR cache entries
  when the same content hash is still referenced by a visible document.
- Print only aggregate counts for the new purge surfaces; do not print OCR text,
  local paths, data directories, fixture roots, or command payloads.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
```

Output summary:

- Exit 101 after tightening the purge test because stdout did not contain
  `ingest jobs purged: 1`, exposing that current purge output and cleanup did
  not cover OCR job/cache retention surfaces.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p meta-store -p resume-cli --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- Target RED/GREEN test: exit 0 after implementation.
- `s14_delete_search`: exit 0; 7 tests passed, including tombstoned metadata,
  full-text snapshots/staging, vector records, OCR job, and OCR page-cache
  cleanup without private text/path leakage.
- `meta-store`: exit 0; 41 tests passed plus doc-tests.
- Focused clippy: exit 0.
- `cargo fmt --check`: exit 0 after formatting.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S90 covers current OCR page-cache rows and ingest jobs for purged documents.
  It does not claim encrypted storage, forensic erase, future OCR bbox/table
  purge coverage, real-resume witness proof, large-corpus proof, or cross-
  platform validation.
- Full product is still not complete.

### S89

Design target:

- Add a local PDF page-render command protocol for scanned PDFs while keeping
  command paths, input paths, and OCR payloads out of user-visible output.
- Detect scanned PDF page count, render and OCR each page, persist per-page OCR
  cache entries, aggregate page text in order, and index one searchable OCR
  version with the correct page count.
- Wire the path through both `resume-cli ocr-worker --render-command` and
  `resume-daemon run --ocr-render-command`.
- Use synthetic PDF fixtures only; do not claim a concrete Poppler/PDFium/
  Tesseract integration or real resume witness from this slice.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_processes_all_scanned_pdf_pages_before_indexing --locked -- --exact
```

Output summary:

- Exit 101 before implementation because the OCR worker processed the scanned
  PDF as a single page, so the test did not observe two per-page OCR cache
  writes or two rendered page handoffs before indexing.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff ocr_worker_processes_all_scanned_pdf_pages_before_indexing --locked -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p import-pipeline --locked
/Users/frankqdwang/.cargo/bin/cargo test -p parser-pdf --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p ocr-client -p import-pipeline -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- Target RED/GREEN test: exit 0 after implementation.
- `ocr-client` test suite: exit 0; 15 tests passed, including render command
  page-byte handoff without debug payload leakage.
- `s15_ocr_handoff`: exit 0; 9 tests passed, including CLI multi-page OCR
  fan-out, per-page cache writes, page-count persistence, and searchability.
- `s50_ocr_worker`: exit 0; 5 tests passed, including daemon multi-page render
  and OCR fan-out.
- `cargo fmt --check`: exit 0.
- `import-pipeline`: exit 0; 5 tests passed plus doc-tests.
- `parser-pdf`: exit 0; 7 tests passed plus doc-tests.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S89 adds a local command protocol and tested multi-page fan-out path. It does
  not install or license-review a concrete renderer/OCR engine, persist OCR
  bounding boxes, prove behavior on real resumes, prove large-corpus OCR
  throughput, complete OCR cache/job retention purge, or validate Windows/
  macOS behavior.
- Full product is still not complete.

### S88

Design target:

- Add an explicit `resume-cli purge --deleted` command for local tombstoned
  document cleanup.
- Remove matching vectors from the persistent vector snapshot, rebuild the
  active full-text snapshot from visible metadata, delete obsolete full-text
  snapshots and staging directories, purge deleted rows from SQLite metadata,
  refresh candidate counts, and run WAL checkpoint plus `VACUUM`.
- Keep command output path-free and clear that the scope is local best-effort,
  not forensic erase or encrypted-storage proof.
- Use synthetic fixtures only.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak --locked -- --exact
```

Output summary:

- Exit 101 before implementation because `resume-cli purge` was not recognized
  as a command.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s14_delete_search --locked
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked
/Users/frankqdwang/.cargo/bin/cargo test -p index-vector --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p index-fulltext -p index-vector -p meta-store --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- Target RED/GREEN test: exit 0 after implementation.
- `cargo fmt --check`: exit 0.
- `s14_delete_search`: exit 0; 7 tests passed, including explicit purge of
  tombstoned metadata, old full-text snapshots, and vector records without data
  directory or fixture path leakage.
- `index-fulltext`: exit 0; 12 tests passed.
- `index-vector`: exit 0; 6 tests passed.
- `meta-store`: exit 0; 42 tests passed across unit/integration/doc-test
  targets.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S88 adds best-effort local purge for documents already tombstoned by the
  product: vector records are physically removed from the vector snapshot, a
  clean full-text snapshot is rebuilt from visible metadata, old full-text
  snapshots/staging directories are removed, and deleted metadata rows are
  purged with SQLite checkpoint/VACUUM.
- It does not delete original user files, claim forensic erasure, encrypt local
  storage, purge every possible future PII surface such as OCR cache or
  queued-job retention, prove behavior on real resumes, or validate Windows/
  macOS filesystem semantics.
- Full product is still not complete.

### S87

Design target:

- Make full-text search with structured filters constrain recall by persisted
  field metadata before the full-text TopDocs cutoff.
- Keep the final profile filter as a correctness guard after hydration.
- Avoid any real resume data; use synthetic `.txt` files in a temporary local
  directory only.
- Do not claim field F1, dictionary completeness, or million-scale performance
  from this slice.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters filtered_search_prefilters_fields_before_fulltext_top_k_cutoff --locked -- --exact
```

Output summary:

- Exit 101 before implementation because `resume-cli search needle
  --skills-any rust --top-k 1` returned `results: 0` when five high-scoring
  decoy documents occupied the unfiltered full-text TopDocs window and the
  lower-scoring Rust candidate was filtered out before it could be considered.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s10_search_filters --locked
/Users/frankqdwang/.cargo/bin/cargo test -p index-fulltext --locked
/Users/frankqdwang/.cargo/bin/cargo test -p meta-store --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p index-fulltext -p meta-store --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/check-runbooks.sh
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- The RED test now passes and returns the field-matching synthetic Rust
  candidate even when `--top-k 1`.
- `s10_search_filters`: exit 0; 2 tests passed.
- `index-fulltext`: exit 0; 12 tests passed.
- `meta-store`: exit 0; 41 tests passed.
- Focused clippy: exit 0.
- `git diff --check`: exit 0.
- `check-runbooks.sh`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S87 adds indexed metadata prefiltering for current degree, skill, and years
  filters before full-text TopDocs retrieval. It does not prove field F1,
  complete dictionaries, million-scale latency, ANN quality, encrypted
  metadata, physical purge, or cross-platform release validation.
- Full product is still not complete.

### S86

Design target:

- Add a local-only model package manifest validation command:
  `resume-cli model validate-manifest --manifest <path>`.
- Validate schema `resume-ir.model-manifest.v1`, `model_pack_id`, non-empty
  `models[]`, per-model id/type/format, embedding `dim`, local artifact
  checksum, and `license.reviewed: true`.
- Keep outputs redacted: no manifest path, model artifact path, model bytes, or
  complete digest should be printed.
- Record that this is governance evidence only; it does not select, download,
  distribute, or quality-evaluate a real OCR/embedding model.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker model_manifest_validate --locked
```

Output summary:

- Exit 101 before implementation because `model` was not a supported top-level
  CLI command.
- After aligning the test with the production model-pack schema, the same
  command failed again because the initial implementation accepted only a
  single-model manifest and rejected `model_pack_id` plus `models[]` as an
  invalid manifest.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s39_embedding_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --check`: exit 0.
- `s39_embedding_worker`: exit 0; 9 tests passed, including valid reviewed
  model-pack manifest, unreviewed-license rejection, checksum-mismatch
  rejection, existing local embedding worker, semantic, and hybrid search paths.
- `verify-local.sh` initially failed twice in
  `foreground_import_scheduler_processes_task_enqueued_after_startup` because
  the test helper inserted a queued import task before writing its scan scope,
  allowing the running daemon to claim the task and mark it failed under
  parallel test timing; the helper now uses the existing atomic
  `insert_import_task_with_scan_scope` API.
- `s4_daemon`: exit 0 after the stability repair; 10 tests passed.
- `resume-cli` clippy: exit 0.
- `check-runbooks.sh`: exit 0; worker and release runbooks now require
  `resume-cli model validate-manifest`.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S86 adds local governance for model-pack checksum and license-review evidence.
  It does not choose/download/distribute a real model, prove semantic/vector
  quality, implement ANN, prove production model performance, or complete model
  release approval.
- Full product is still not complete.

### S85

Design target:

- Add a local-only model checksum fault simulation for controlled model
  artifacts:
  `resume-cli fault-simulate --case model-checksum --model-file <path> --expected-sha256 <hex>`.
- Compute the actual SHA-256 locally, report match/mismatch as a safe
  reproduced/not-reproduced probe, and expose the hook in doctor plus redacted
  diagnostics.
- Keep outputs redacted: no model path, model bytes, full digest, or local data
  directory should be printed.
- Do not select, license, download, package, distribute, or validate a real
  production embedding/OCR model in this slice.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_model_checksum --locked
```

Output summary:

- Exit 101 before implementation because `fault-simulate` usage did not include
  `model-checksum`, and the CLI rejected the new checksum probe arguments.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '[s]uperpowers|docs/[s]uperpowers|2026-05-30-long-running-goal-[e]xecution' .
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --check`: exit 0.
- `s71_fault_injection`: exit 0; 9 tests passed, including checksum mismatch
  and checksum match probes against synthetic local bytes.
- `s13_diagnostics`: exit 0; 9 tests passed, including redacted diagnostics
  advertising `model_checksum`.
- `resume-cli` clippy: exit 0.
- `check-runbooks.sh`: exit 0; the fault-injection runbook documents
  `resume-cli fault-simulate --case model-checksum`.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S85 adds a local checksum fault probe for a caller-supplied, controlled model
  artifact. It does not select/license/download/distribute a real model, prove
  semantic/vector quality, prove OCR/embedding model performance, or complete
  model package governance.
- Full product is still not complete.

### S84

Design target:

- Add a real benchmark policy gate for existing benchmark JSON artifacts, so a
  benchmark smoke can fail on insufficient sample size, P95 latency regression,
  zero-result regressions, or unproven million-scale claims.
- Wire the gate into PR benchmark smoke and nightly benchmark smoke workflows.
- Keep synthetic smoke explicitly scoped: `--allow-synthetic` is required, and a
  passing synthetic gate must not be treated as 100k/1M real-corpus evidence.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
```

Output summary:

- Exit 101 before implementation because the new RED tests imported missing
  symbols `evaluate_benchmark_gate_json` and `BenchmarkGateConfig`.
- The CLI RED tests also required `resume-benchmark gate --report <path>` to
  exist and reject synthetic artifacts unless `--allow-synthetic` is supplied.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p benchmark-runner --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p benchmark-runner --all-targets --locked -- -D warnings
tmpdir=$(mktemp -d); /Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- synthetic-query --index-dir "$tmpdir/index" --documents 24 --queries 6 --top-k 5 --json > "$tmpdir/benchmark-smoke.json" && /Users/frankqdwang/.cargo/bin/cargo run -p benchmark-runner --bin resume-benchmark --locked -- gate --report "$tmpdir/benchmark-smoke.json" --allow-synthetic --min-documents 24 --min-queries 6 --max-p95-ms 1000 --max-zero-result-queries 0; rc=$?; rm -rf "$tmpdir"; exit $rc
./scripts/ci/check-runbooks.sh
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- `cargo fmt --check`: exit 0.
- `benchmark-runner` tests: exit 0; 9 integration tests passed, including gate
  rejection for synthetic-without-allowance, latency regression, and unproven
  million-scale claims.
- `benchmark-runner` clippy: exit 0.
- CLI smoke: exit 0; `resume-benchmark gate` printed `benchmark gate passed`
  against a generated redacted synthetic report.
- `check-runbooks.sh`: exit 0; the release blocker runbook now documents
  `resume-benchmark gate`.
- `git diff --check`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S84 adds synthetic benchmark policy gates and workflow wiring. It does not run
  or claim 100k/1M real-corpus benchmarks, semantic/vector recall gates, OCR
  throughput gates, representative hardware runs, Windows/macOS benchmark
  evidence, or production P95 target compliance.
- Full product is still not complete.

### S83

Design target:

- Close the P6 runbook gap with production runbooks for diagnostics redaction,
  fault injection, OCR/embedding workers, and release blockers.
- Enforce local-only privacy language and required operational commands with a
  CI guard so runbooks cannot silently disappear from local or hosted checks.
- Keep this slice synthetic-fixture only; do not read, scan, upload, or transmit
  real resumes.

Observed RED:

```bash
sh scripts/ci/check-runbooks.sh
```

Output summary:

- Exit 1 before runbooks existed with `missing required runbook:
  docs/runbooks/diagnostics-redaction.md`.
- After the files were created, the same guard exposed missing canonical command
  strings for `resume-cli export-diagnostics --redact` and
  `resume-cli fault-simulate --case disk-space-low`; those checks were kept in
  the guard and the runbooks were corrected.

Implementation checks:

```bash
./scripts/ci/check-runbooks.sh
sh -n scripts/ci/check-runbooks.sh scripts/ci/verify-local.sh scripts/ci/guard-public-repo.sh scripts/ci/check-licenses.sh
git diff --check
./scripts/ci/guard-public-repo.sh
./scripts/ci/verify-local.sh
```

Output summary:

- `check-runbooks.sh`: exit 0; required runbook files, Local-only/Do not upload/
  Synthetic fixtures privacy language, diagnostics, fault-simulation, worker,
  and release-blocker command strings were present.
- `sh -n`: exit 0 for the runbook, verify-local, public guard, and license
  scripts.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0; public repo guard passed.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, runbook check, and public repository guard passed.

Scope note:

- S83 adds documentation and CI policy coverage only. It does not perform real
  resume scanning, package signing/notarization, Windows/macOS release
  validation, real 100k/1M corpus benchmarks, destructive service-manager
  failure drills, or actual disk-exhaustion drills.
- Full product is still not complete.

### S82

Design target:

- Add a local-only `resume-cli fault-simulate --case ocr-crash
  --ocr-command <path>` probe that runs a configured local OCR command against
  synthetic page bytes, treats an engine crash as reproduced, and redacts command
  output, paths, and payload bytes.
- Add CLI and daemon OCR worker crash-recovery evidence: a crashing OCR command
  must leave the scanned document `OcrRequired`, keep the ingest job
  `FailedRetryable`, write a retryable OCR cache failure, and avoid leaking OCR
  stdout/stderr, command paths, data paths, or fixture roots.
- Expose `ocr_crash` in doctor/export diagnostics without weakening the privacy
  boundary. Do not read real resumes or run a real OCR engine.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_ocr_crash_reproduces_engine_failure_without_payload_or_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths -- --exact
```

Output summary:

- `s71_fault_injection`: exit 101 before implementation because
  `fault-simulate --case ocr-crash` returned the usage error.
- `s13_diagnostics`: exit 101 before implementation because diagnostics did
  not include `"ocr_crash"`.
- The CLI and daemon worker retryable-failure tests passed against existing
  worker failure semantics after being added, proving the current worker paths
  already preserved retryability and redaction for crashing OCR commands.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s15_ocr_handoff --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- `resume-cli --test s71_fault_injection`: exit 0; 7 tests passed, including
  `fault_simulate_ocr_crash_reproduces_engine_failure_without_payload_or_path_leak`.
- `resume-cli --test s13_diagnostics`: exit 0; 9 tests passed.
- `resume-cli --test s15_ocr_handoff`: exit 0; 8 tests passed, including
  retryable CLI OCR worker command-crash handling.
- `resume-daemon --test s50_ocr_worker`: exit 0; 4 tests passed, including
  retryable daemon OCR worker command-crash handling.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D
  warnings`: exit 0.
- `git diff --check`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, and public repository guard passed.

Scope note:

- S82 safely exercises OCR command crash behavior using a controlled local
  command and synthetic bytes. It does not install or license a real OCR engine,
  render real PDF pages, crash a production service manager, simulate actual disk
  exhaustion, or prove Windows/macOS behavior.
- Full product is still not complete.

### S81

Design target:

- Add a local-only `resume-cli fault-simulate --case daemon-kill
  --daemon-binary <path>` probe that starts a configured daemon binary against a
  synthetic data directory, waits for readiness, terminates the controlled
  process, runs a same-directory `--once` restart check, and redacts paths.
- Add actual `resume-daemon` kill/restart integration evidence using the real
  daemon binary and a synthetic data directory.
- Expose `daemon_kill` in doctor/export diagnostics without weakening the
  privacy boundary. Do not kill user services or read real resumes.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_daemon_kill_restarts_configured_daemon_without_path_leak -- --exact
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths -- --exact
```

Output summary:

- `s71_fault_injection`: exit 101 before implementation because
  `fault-simulate --case daemon-kill` returned the usage error.
- `s13_diagnostics`: exit 101 before implementation because diagnostics did
  not include `"daemon_kill"`.
- The real daemon kill/restart integration test was added as production
  evidence and passed against existing daemon behavior, so no daemon production
  code change was required for restart health.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s81_daemon_kill --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
git diff --check
./scripts/ci/verify-local.sh
```

Output summary:

- `resume-cli --test s71_fault_injection`: exit 0; 6 tests passed, including
  `fault_simulate_daemon_kill_restarts_configured_daemon_without_path_leak`.
- `resume-cli --test s13_diagnostics`: exit 0; 9 tests passed.
- `resume-daemon --test s81_daemon_kill`: exit 0; the real foreground daemon
  was killed and restarted with the same synthetic data directory without path
  leakage.
- `cargo fmt --check`: exit 0.
- `cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D
  warnings`: exit 0.
- `git diff --check`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, and public repository guard passed.

Scope note:

- S81 safely exercises process kill/restart for a controlled daemon binary and
  synthetic data directory. It does not kill a user-installed service, simulate
  actual disk exhaustion, crash OCR workers, validate service managers, or prove
  Windows/macOS behavior.
- Full product is still not complete.

### S80

Design target:

- Add a real local file-lock contention probe to `resume-cli fault-simulate`
  without leaking paths or leaving probe files behind.
- Expose the new `file_lock` hook in doctor/export diagnostics.
- Keep the probe synthetic and local-only; do not scan or upload user data.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection fault_simulate_file_lock_reproduces_contention_without_path_leak --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics export_diagnostics_redact_outputs_skeleton_without_paths --locked
```

Output summary:

- `s71_fault_injection`: exit 101 before implementation because
  `fault-simulate --case file-lock` returned the usage error.
- `s13_diagnostics`: exit 101 before implementation because diagnostics did
  not include `"file_lock"`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `resume-cli --test s71_fault_injection`: exit 0; 5 tests passed, including
  `fault_simulate_file_lock_reproduces_contention_without_path_leak`.
- `resume-cli --test s13_diagnostics`: exit 0; 9 tests passed.
- `cargo clippy -p resume-cli --all-targets --locked -- -D warnings`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.

Scope note:

- S80 exercises advisory file-lock contention against a local synthetic probe
  file. It does not implement destructive ENOSPC, daemon-kill, OCR-crash,
  model-checksum, battery-mode, or external-drive-disconnect fault injection.
- Full product is still not complete.

### S79

Design target:

- Add local resource telemetry to doctor and redacted diagnostics without
  reading resume files or printing local paths.
- Report data-volume disk total/available bytes, current-process memory bytes,
  and CPU core count.
- Keep `export-diagnostics --redact` valid JSON with resource paths explicitly
  redacted.

Observed RED:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics doctor_and_diagnostics_report_redacted_resource_telemetry --locked
```

Output summary:

- Exit 101 before implementation; the new test failed because stdout did not
  contain `resource telemetry: available`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo fmt --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `resume-cli --test s13_diagnostics`: exit 0; 9 tests passed, including the
  new resource telemetry test that parses redacted JSON and checks numeric
  telemetry fields.
- `cargo clippy -p resume-cli --all-targets --locked -- -D warnings`: exit 0.
- `verify-local.sh`: exit 0; metadata, fmt, workspace clippy, workspace tests,
  license check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- PR #9 hosted checks after push: dependency tree pass 26s, license policy
  pass 20s, public repository guard pass 5s, rust workspace pass 1m3s.

Scope note:

- S79 reports local resource numbers only; it does not run real-resume witness
  scans, does not prove 100k/1M corpus performance, and does not implement
  destructive ENOSPC or kill-daemon fault injection.
- Full product is still not complete.

### S78

Design target:

- Apply the same portable Unix process-group signaling syntax to the local
  command embedder that S77 applied to OCR.
- Add an embedder regression test for descendant processes that keep stdout
  pipes open after a timeout.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26864432512 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m57s.
- OCR tests passed in the hosted run, including
  `local_command_worker_terminates_descendants_that_keep_output_pipes_open`.
- The hosted run then failed with exit 143 while running
  `tests/s51_embedding_worker.rs`.
- The embedder had the same `/bin/kill <signal> -PGID` process-group signaling
  form as the pre-S77 OCR client, so S78 updates it to
  `/bin/kill <signal> -- -PGID`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s51_embedding_worker --locked
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `embedder --test s11_embedder`: exit 0; 7 tests passed, including the new
  inherited-pipe descendant timeout regression test.
- `resume-daemon --test s51_embedding_worker`: exit 0; 2 tests passed.
- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed.
- First `verify-local.sh` attempt failed at `cargo fmt --check` after the new
  test; `/Users/frankqdwang/.cargo/bin/cargo fmt --all` was run.
- Second `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests,
  license check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0; public repo guard passed.
- Obsolete-reference marker scan: exit 1 with no matches.

Hosted checks:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir
```

Output summary:

- `dependency tree`: pass in 19s.
- `license policy`: pass in 17s.
- `public repository guard`: pass in 3s.
- `rust workspace`: pass in 2m8s.

Scope note:

- S78 fixes local embedding command process cleanup only. It does not package
  or validate a real embedding model.

### S77

Design target:

- Make OCR command process-group termination portable across macOS and Linux.
- Keep timeout/cancel error paths joining stdout/stderr readers after the worker
  process group has actually been terminated, so timeout returns before
  descendants close inherited pipes naturally.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26864213730 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m38s.
- The failing test was
  `local_command_worker_terminates_descendants_that_keep_output_pipes_open`.
- The assertion message was `timeout returned only after descendant closed
  inherited pipes`.
- The failure indicates the S76 Unix cleanup still did not signal the Linux
  process group; S77 changes `/bin/kill` calls to pass `--` before the negative
  process-group id and removes the unreliable direct-child helper.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed, including the
  inherited-pipe descendant timeout case.
- `resume-daemon --test s50_ocr_worker`: exit 0; 3 tests passed.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0; public repo guard passed.
- Obsolete-reference marker scan: exit 1 with no matches.

Pending remote check:

- PR #9 hosted GitHub Actions checks after push

Scope note:

- S77 fixes process-group signal syntax for local OCR command cleanup only. It
  does not package or validate a real OCR engine.

### S76

Design target:

- Restore OCR timeout/cancel error-path output-reader cleanup so the client
  does not leave detached reader threads or background process side effects.
- Before the OCR shell exits, terminate its direct child processes and then the
  process group, so descendants that inherited stdout/stderr pipes do not hang
  reader joins.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26863872803 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after the S75 push.
- The failed run exited with code 143 while running
  `tests/s50_ocr_worker.rs`, after
  `daemon_ocr_worker_once_respects_pause_without_claiming_or_invoking_command`
  had completed.
- S75 passed local verification but was not stable enough for Linux hosted CI.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s50_ocr_worker --locked
./scripts/ci/verify-local.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed, including the
  inherited-pipe descendant timeout case.
- `resume-daemon --test s50_ocr_worker`: exit 0; 3 tests passed.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.
- `git diff --check`: exit 0.
- `guard-public-repo.sh`: exit 0; public repo guard passed.
- Obsolete-reference marker scan: exit 1 with no matches.

Pending remote check:

- PR #9 hosted GitHub Actions checks after push

Scope note:

- S76 fixes local OCR command timeout cleanup stability only. It does not
  package or validate a real OCR engine.

### S75

Design target:

- OCR timeout/cancel/error paths should not wait for stdout/stderr reader
  threads after the worker process has already been terminated.
- Descendant processes that inherited output pipes must not delay the caller's
  timeout result.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26863687741 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m36s.
- The failing test remained `local_command_worker_terminates_descendants_that_keep_output_pipes_open`.
- The S74 process-group kill change passed local verification but still did not
  prevent the error path from waiting for inherited output pipes on Linux CI.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.

Scope note:

- S75 fixed OCR command timeout return behavior locally, but later hosted CI
  failed with exit 143 while running daemon OCR worker tests. S76 supersedes it.

### S74

Design target:

- PR #9 `rust workspace` should not hang until OCR command descendants close
  inherited stdout/stderr pipes after a timeout.
- OCR fixture permission checks should use Linux GNU `stat` first and fall
  back to macOS `stat`.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26863536781 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m40s.
- The failing test was `local_command_worker_terminates_descendants_that_keep_output_pipes_open`.
- The failure message showed timeout cleanup returned only after a descendant
  closed inherited pipes.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p ocr-client --test s12_ocr_client --locked
./scripts/ci/verify-local.sh
```

Output summary:

- `ocr-client --test s12_ocr_client`: exit 0; 14 tests passed.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.

Scope note:

- S74 fixes local OCR command timeout cleanup and a Linux/macOS fixture
  portability issue. It does not package or validate a real OCR engine.

### S73

Design target:

- PR #9 required GitHub Actions should pass on Linux, not only local macOS.
- The embedder permission test should inspect owner-only temp input file
  permissions using portable `stat` invocation order.

Observed RED:

```bash
gh pr checks 9 --repo FrankQDWang/resume-ir --watch --interval 10
gh run view 26863418606 --repo FrankQDWang/resume-ir --log-failed
```

Output summary:

- `rust workspace` failed in GitHub Actions after 1m40s.
- The failing test was `local_command_embedder_times_out_and_keeps_input_file_private`.
- On Linux, `stat -f '%Lp'` returned filesystem information plus `600`
  instead of failing, so the assertion compared a multi-line string to `600`.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked
```

Scope note:

- S73 changes only the synthetic test fixture command. It does not alter the
  product embedder protocol or claim Linux installer/release readiness.

### S72

Design target:

- `verify-local.sh` must be stable enough to gate public PR work.
- Local embedding command temp input directories must not collide when multiple
  embedding tests or worker requests run concurrently in the same process.

Observed RED:

```bash
./scripts/ci/verify-local.sh
```

Output summary:

- Exit 101 before the fix; `local_command_embedder_runs_configured_binary_and_parses_structured_vectors`
  failed with `EmbeddingError::EngineFailed` during the workspace test phase.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p embedder --test s11_embedder --locked
./scripts/ci/verify-local.sh
```

Output summary:

- `embedder --test s11_embedder`: exit 0; 6 tests passed, including the new
  parallel local-command request regression.
- `verify-local.sh`: exit 0; metadata, fmt, clippy, workspace tests, license
  check, and public repository guard passed.

Scope note:

- S72 only fixes local temp-directory uniqueness for the command embedder. It
  does not add a licensed model, ANN index, semantic quality proof, or
  OS-enforced network isolation for external embedding commands.

### S71

Design target:

- S71 closes the P6 gap where doctor/export listed fault simulation hooks but
  the CLI had no executable local fault-simulation entrypoint.
- `resume-cli fault-simulate --case disk-space-low` now safely reproduces a
  low-space budget condition without filling the real disk, or writes and
  removes a bounded probe when the configured available budget is sufficient.
- `resume-cli fault-simulate --case permission-denied` now attempts a redacted
  local write probe and reports permission denial without printing paths.
- Doctor/export diagnostics now include the permission-denied probe hook.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
```

Output summary:

- Exit 101 before implementation; all four S71 tests failed because
  `resume-cli fault-simulate` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s71_fault_injection --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --all
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s13_diagnostics --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli --all-targets --locked -- -D warnings
```

Output summary:

- `resume-cli --test s71_fault_injection`: exit 0; 4 tests passed, covering
  disk-space-low reproduction without probe writes, bounded probe write cleanup
  when the budget is sufficient, permission-denied reproduction, and usage
  errors without path leaks.
- `cargo fmt --all`: exit 0.
- `resume-cli --test s13_diagnostics`: exit 0; 8 tests passed.
- `cargo clippy -p resume-cli --all-targets --locked -- -D warnings`: exit 0.

Scope note:

- S71 is a safe local simulation/probe slice. It does not fill the actual disk,
  does not claim real ENOSPC coverage, does not implement advisory/mandatory
  file-lock behavior, and does not cover kill-daemon or OCR crash injection.
- Full product is still not complete.

### S68

Design target:

- S68 fixes the GitHub configuration script after a real public-repository
  configuration attempt exposed an invalid `gh repo edit` option for personal
  public repositories.
- The failed run happened after guard and push and before branch protection; no
  branch protection settings were applied by the failed command.

Checks and remote operations:

```bash
./scripts/ci/configure-github-repo.sh FrankQDWang resume-ir
sh -n scripts/ci/configure-github-repo.sh
git diff --check
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
```

Output summary:

- First `configure-github-repo.sh`: exit 1 after `public repo guard passed`,
  `Everything up-to-date`, and `branch 'main' set up to track 'origin/main'`;
  `gh repo edit` returned `HTTP 422` because the forking option is only valid
  for org-owned private repositories.
- Removed the invalid repo edit option.
- `sh -n scripts/ci/configure-github-repo.sh`: exit 0.
- `git diff --check`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.

Scope note:

- Branch protection is rerun after this fix is committed and pushed.
- Full product is still not complete.

### S67

Design target:

- S67 unblocks the public GitHub setup now that `FrankQDWang` keyring auth is
  available outside the sandbox.
- The public repository was created only after the local public-repository guard
  passed; no local data directory, token file, diagnostic bundle, index, model
  cache, or real resume was committed or uploaded.
- The GitHub configuration script fallback remote now uses HTTPS, matching the
  selected Git protocol.

Checks and remote operations:

```bash
gh repo view FrankQDWang/resume-ir
gh repo create FrankQDWang/resume-ir --public --source=. --remote=origin --description "Local-first resume search engine" --disable-wiki
git remote -v
./scripts/ci/guard-public-repo.sh
git rev-parse main
git push -u origin main
sh -n scripts/ci/configure-github-repo.sh
git diff --check
```

Output summary:

- `gh repo view FrankQDWang/resume-ir`: exit 1 before creation; repository did
  not exist.
- `gh repo create ...`: exit 0 and returned
  `https://github.com/FrankQDWang/resume-ir`.
- `git remote -v`: origin is `https://github.com/FrankQDWang/resume-ir.git`.
- `./scripts/ci/guard-public-repo.sh`: exit 0.
- `git rev-parse main`: `cc009da12c7c5753bbf3e66642fccee7db2ebeae`.
- `git push -u origin main`: exit 0; new remote branch `main` was pushed and
  set as upstream.
- `sh -n scripts/ci/configure-github-repo.sh`: exit 0 after the HTTPS fallback
  fix.
- `git diff --check`: exit 0.

Scope note:

- S67 does not prove hosted GitHub Actions results, does not create a release,
  and does not package/sign/notarize the app. Branch protection is executed
  after this commit is pushed so that the progress/script-fix commit is not
  blocked by protection.
- Full product is still not complete.

### S66

Design target:

- S66 closes the local P5 gap where the daemon could run in foreground but the
  CLI had no service lifecycle entrypoint.
- The CLI now supports `resume-cli service install|uninstall|status|start|stop`.
  Install writes a macOS user LaunchAgent plist with `ProgramArguments` for
  `resume-daemon --data-dir <local> run --foreground --work-imports
  --work-index --ipc-listen 127.0.0.1:0`, preserves user data on uninstall, and
  keeps CLI stdout/stderr path-redacted.
- Optional OCR and embedding worker command flags can be included in the
  generated plist, but no concrete engine/model is bundled by this slice.

TDD red check:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
```

Output summary:

- Exit 101 before implementation; all four S66 tests failed because
  `resume-cli service` did not exist.

Implementation checks:

```bash
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s66_service_lifecycle --locked
/Users/frankqdwang/.cargo/bin/cargo fmt --check
git diff --check
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s4_cli --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-cli --test s20_status_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s4_daemon --locked
/Users/frankqdwang/.cargo/bin/cargo test -p resume-daemon --test s20_ipc --locked
/Users/frankqdwang/.cargo/bin/cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
/Users/frankqdwang/.cargo/bin/cargo test --workspace --locked
./scripts/ci/guard-public-repo.sh
rg -n -i --hidden --glob '!target/**' --glob '!.git/**' '<obsolete wrapper/doc markers>' .
gh auth status
```

Output summary:

- `resume-cli --test s66_service_lifecycle`: exit 0; 4 tests passed, covering
  LaunchAgent plist install, XML escaping, redacted install/status/uninstall
  output, user-data preservation, start/stop dry-run output, and invalid label
  rejection without path leaks.
- `cargo fmt --check`: exit 0.
- `git diff --check`: exit 0.
- `resume-cli --test s4_cli`: exit 0; 6 tests passed.
- `resume-cli --test s20_status_ipc`: exit 0; 6 tests passed.
- `resume-daemon --test s4_daemon`: exit 0; 10 tests passed.
- `resume-daemon --test s20_ipc`: exit 0; 18 tests passed.
- `cargo clippy -p resume-cli -p resume-daemon --all-targets --locked -- -D warnings`: exit 0.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: exit 0.
- `cargo test --workspace --locked`: exit 0.
- `./scripts/ci/guard-public-repo.sh`: exit 0.
- Obsolete-reference marker scan: exit 1 with no matches.
- Sandboxed `gh auth status`: exit 1 with stale invalid credential. Escalated
  `gh auth status`: exit 0; `FrankQDWang` is logged in from keyring with
  `repo` and `workflow` scopes.

Scope note:

- S66 does not create a signed macOS pkg/dmg, does not notarize, does not build
  Windows MSI/service registration, does not run real `launchctl` start/stop
  against the user's login session, does not execute hosted GitHub Actions, and
  does not prove cross-platform install/upgrade/uninstall.
- Full product is still not complete.

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

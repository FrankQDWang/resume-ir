# Production Readiness Audit

Date: 2026-05-31

This audit records the current gap between the repository and the product goal in
`GOAL.md`. It is intentionally stricter than the S0-S16 long-running execution
checklist: a slice can pass while the full product is still incomplete.

## Current State

- The repository was documentation-only after `f3e1a54 revert: remove goal generated implementation`.
- S1-S16 foundation code has been rebuilt in the current worktree: `Cargo.toml`,
  `Cargo.lock`, and the first `crates/` workspace members now exist with local
  acceptance evidence, including SQLite metadata schema, queue recovery, and
  CLI/daemon lifecycle skeletons, filesystem crawling, and parser skeletons for
  DOCX/PDF classification, text normalization, section fallback, strong
  email/phone/date-range/school/degree/skill rules, a real Tantivy full-text
  index/search layer, a synchronous synthetic import-to-search smoke path, and
  an S10 query-time field-filter/dedupe skeleton plus S11 fake-interface semantic
  retrieval skeleton crates and RRF fusion tests plus S12 OCR client/queue
  skeleton crates that do not run OCR plus S13 redacted doctor/diagnostics
  skeleton commands and a one-query full-text smoke plus S14 CLI-level
  deletion propagation by `doc_id`, including malformed-id rejection,
  tombstone-preserving rediscovery, SQLite-first delete state, and
  metadata-filtered search protection against stale full-text hits, plus S15
  local synthetic benchmark tooling that seeds synthetic metadata and Tantivy
  documents with batched SQLite writes, searches through the metadata gate,
  deletes one hit through the existing delete path, verifies post-delete hiding,
  cleans scratch data on success and handled failure paths, and reports
  aggregate-only metrics without claiming unrun 100k/1M performance, plus S16
  local redacted diagnostics package generation that writes aggregate-only
  `manifest.json`, `status.txt`, and `checks.txt` files without printing or
  storing private paths, document identifiers, file names, queries, snippets,
  email/phone payloads, or raw resume text.
- `.github/` is absent. `tests/fixtures/` contains the empty CLI smoke fixture
  plus synthetic DOCX/PDF fixtures for S9 text-layer import and OCR-required routing.
  Runtime data remains local-only and ignored.
- `PROGRESS.md` records S0 through S16 as complete while P0/P1/P2/P3/P6 remain
  incomplete and P4 remains blocked for real OCR execution.
- No repo-local `AGENTS.md` exists. The in-thread workflow instructions remain active.
- Rust is available at `/Users/frankqdwang/.cargo/bin`, but not on the default shell `PATH`.
- `sqlite3` is available. `tesseract` and `ocrmypdf` are not currently available on `PATH`.

## P0-P6 Gap Audit

| Phase | Complete | Incomplete | Must Rebuild | External Blockers |
|---|---|---|---|---|
| P0 architecture skeleton | Design baseline, Git repo, README, PROGRESS, `.gitignore`; S1-S16 workspace/domain/config/SQLite metadata/CLI/daemon/fs-crawler/parser/text-processing/fulltext/import-smoke/field-filter/fake-semantic-interface/OCR-queue/doctor-diagnostics/delete-propagation/synthetic-benchmark/local-redacted-diagnostics-package foundation has local acceptance evidence. | IPC, production logs/observability, CI, production async import/index/delete orchestration, and deeper kill-recovery smoke. | Previous skeleton code was deleted and must not count as product progress; future code must carry fresh verification evidence. | Rust must be invoked with the user cargo path unless shell PATH is fixed; local CI/branch protection cannot be fully verified without remote setup. |
| P1 text import and full-text search | Product design for docx/PDF text, normalization, sectioning, Tantivy, snippets; S5 filesystem crawler exists with synthetic tests; S6 adds parser contracts, basic DOCX extraction, and honest PDF text-layer/image-only/unknown classification; S7 adds basic text cleanup, offset mapping, and heading/fallback sectioning; S8 adds real Tantivy indexing/search and CLI search over an existing index; S9 adds a synchronous synthetic DOCX/PDF fixture import loop that persists metadata/index state and searches after reader reopen; S14 adds `resume-cli delete --doc-id` tombstoning, committed Tantivy doc-id deletion when an index exists, no-index deletion without creating an index, tombstone-preserving re-import skip behavior, SQLite `DELETE_PENDING`/`DELETED`/`DELETE_ERROR` index state, and metadata-filtered search hiding for stale full-text hits after delete errors; S15 adds a local synthetic benchmark runner that exercises direct synthetic metadata/Tantivy indexing with batched metadata writes, metadata-gated search, delete, and post-delete search verification. | Production import worker, broader PDF text extraction, durable worker claiming/lease semantics, async deletion propagation, and real 100k text import/delete benchmark runs. | The S9 importer, S14 deleter, and S15 benchmark runner are CLI/local synthetic smoke or tooling paths, not the final asynchronous import/delete worker orchestration or a production benchmark result. | Large synthetic or desensitized corpora are not present. |
| P2 fields and dedupe | Field model and confidence rules are specified; S10 adds deterministic synthetic email/phone/date-range/school/degree/skill extraction, original evidence/confidence on `StrongEntity`, `rank-fusion` field summaries and `degree_min`/`skills_any`/`years_experience_min` tests, hashed soft-dedupe skeleton, and CLI `--degree`/`--top-k` smoke wiring; S14 makes `clean_text_by_doc_id` return no clean text for deleted documents. | Production dictionaries, field-index fast fields, candidate/version merge workflow, quality harness, field-labeled evaluation set, and broad multilingual extraction. | S10 is query-time MVP filtering over returned Tantivy hits, not production field indexing or complete P2 dedupe. S14 prevents stale field-filter clean-text resurrection but does not add a production field index deletion pipeline. | Field-labeled desensitized evaluation set and dictionary/license decisions are not present. |
| P3 semantic retrieval | ONNX/vector/RRF architecture is specified; S11 adds typed `Embedder` request/response/vector APIs, deterministic `FakeEmbedder`, `VectorIndex` with deterministic in-memory cosine/dot search tests, and `rank-fusion` RRF hybrid fusion tests. | Real model manifest, batch inference, production vector index, hybrid retrieval integration, recall benchmark. | S11 is only a fake-interface skeleton; fake embedders and in-memory vector tests cannot satisfy production semantic search. | Model choice, license, checksums, distribution approval, and production vector engine selection require human confirmation. |
| P4 OCR | OCR routing, cache, worker isolation, and degradation design are specified; S6 can classify simple unencoded image-only PDFs as OCR-required; S12 adds typed OCR request/response/cache-key/options/timeout/cancellation APIs and a deterministic in-memory OCR_REQUIRED queue with priority/resource-policy claiming, defer/retry, cancellation, and query-path no-claim behavior. | Real OCR client/worker execution, robust scan detection, durable page cache, language pack/profile integration, worker isolation/recovery tests, and scanned synthetic corpus. | Disabled/noop OCR cannot satisfy production OCR. The S12 path intentionally returns deferred/cancelled/timed-out non-execution results and must remain off the query hot path. | `tesseract`/`ocrmypdf` and language packs are absent on PATH; scanned test corpus is absent. |
| P5 packaging | Packaging, signing, install, upgrade, uninstall design exists. | Windows MSI, macOS pkg/dmg, signing/notarization, user-mode daemon install, rollback tests. | Packaging cannot start until binaries exist. | Windows signing certs, Apple Developer credentials, secrets, and platform runners require external setup. |
| P6 performance and stability | Performance targets and fault-injection plan exist; S13 adds a redacted `resume-cli doctor`, required-redaction `export-diagnostics` skeleton, missing/corrupt full-text status, one-query small-data smoke, and helper-level daemon-kill/disk-full simulation tests. S15 adds `resume-cli benchmark --synthetic-count <n> --query <query>` for local synthetic metadata/Tantivy indexing with batched SQLite writes, metadata-gated search, existing delete-path verification, aggregate-only metrics, and scratch cleanup on success and handled failure paths. S16 adds local `resume-cli export-diagnostics --redact --output <dir>` package generation with aggregate-only manifest/status/check files plus redacted stdout and errors. | Real 100k/1M benchmark runs, real fault injection, restart/recovery soak, production observability/log collection, and performance gates. | S13 is skeleton/smoke only. S15 is honest local synthetic benchmark tooling/smoke only and must not be counted as production P6 performance or stability evidence unless large runs are actually executed and recorded; CLI large-corpus status is deliberately `synthetic-only` rather than a production pass. S16 is local redacted diagnostics package generation only and must not be counted as real production observability or P6 completion. Previous smoke commands were deleted and cannot count as stability coverage. | 100k/1M corpus, real query set, platform performance machines, and long-running runners are not present. |

## Production Build Plan

1. Rebuild P0 foundation with tested Rust workspace, core domain/config types, durable metadata schema, daemon/CLI entrypoints, and progress evidence.
2. Complete a real P1 synthetic-data import/search/delete loop: crawl, parse docx/PDF text layer, normalize, section, index in Tantivy, query, snippet, delete propagation, restart, and recovery.
3. Add P2 extraction and filtering with golden synthetic fixtures, confidence/evidence tracking, and candidate/version folding.
4. Continue P3 behind model-license gates: S11 fake interfaces exist, but real locally bundled or user-provided embedding integration must wait for explicit model/license/checksum/distribution approval and a production vector-engine decision.
5. Continue P4 OCR from the S12 skeleton toward an isolated asynchronous worker. Until OCR dependencies and scanned corpus are available, mark real OCR execution as blocked rather than completed.
6. Continue P6 beyond the S13/S15/S16 local diagnostics and benchmark tooling with real production observability, recorded 100k/1M benchmark runs, fault injection, and soak only with real binaries, platform runners, and explicit signing/release approval.

## Completion Rule

Do not mark the overall goal complete until P0-P6 production gates in
`01_system_design_系统设计/08_安全性能与验收指标.md` and
`02_execution_plan_执行方案/07_里程碑风险与取舍.md` have either passed with evidence
or are explicitly accepted as out of scope by the user.

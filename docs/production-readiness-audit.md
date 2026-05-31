# Production Readiness Audit

Date: 2026-05-31

This audit records the current gap between the repository and the product goal in
`GOAL.md`. It is intentionally stricter than the S0-S13 long-running execution
checklist: a slice can pass while the full product is still incomplete.

## Current State

- The repository was documentation-only after `f3e1a54 revert: remove goal generated implementation`.
- S1-S13 foundation code has been rebuilt in the current worktree: `Cargo.toml`,
  `Cargo.lock`, and the first `crates/` workspace members now exist with local
  acceptance evidence, including SQLite metadata schema, queue recovery, and
  CLI/daemon lifecycle skeletons, filesystem crawling, and parser skeletons for
  DOCX/PDF classification, text normalization, section fallback, strong
  email/phone/date-range/school/degree/skill rules, a real Tantivy full-text
  index/search layer, a synchronous synthetic import-to-search smoke path, and
  an S10 query-time field-filter/dedupe skeleton plus S11 fake-interface semantic
  retrieval skeleton crates and RRF fusion tests plus S12 OCR client/queue
  skeleton crates that do not run OCR plus S13 redacted doctor/diagnostics
  skeleton commands and a one-query full-text smoke.
- `.github/` is absent. `tests/fixtures/` contains the empty CLI smoke fixture
  plus synthetic DOCX/PDF fixtures for S9 text-layer import and OCR-required routing.
  Runtime data remains local-only and ignored.
- `PROGRESS.md` records S0 through S13 as complete while P0/P1/P2/P3/P6 remain
  incomplete and P4 remains blocked for real OCR execution.
- No repo-local `AGENTS.md` exists. The in-thread workflow instructions remain active.
- Rust is available at `/Users/frankqdwang/.cargo/bin`, but not on the default shell `PATH`.
- `sqlite3` is available. `tesseract` and `ocrmypdf` are not currently available on `PATH`.

## P0-P6 Gap Audit

| Phase | Complete | Incomplete | Must Rebuild | External Blockers |
|---|---|---|---|---|
| P0 architecture skeleton | Design baseline, Git repo, README, PROGRESS, `.gitignore`; S1-S13 workspace/domain/config/SQLite metadata/CLI/daemon/fs-crawler/parser/text-processing/fulltext/import-smoke/field-filter/fake-semantic-interface/OCR-queue/doctor-diagnostics foundation has local acceptance evidence. | IPC, production logs/diagnostics packaging, CI, production async import/index orchestration, and deeper kill-recovery smoke. | Previous skeleton code was deleted and must not count as product progress; future code must carry fresh verification evidence. | Rust must be invoked with the user cargo path unless shell PATH is fixed; local CI/branch protection cannot be fully verified without remote setup. |
| P1 text import and full-text search | Product design for docx/PDF text, normalization, sectioning, Tantivy, snippets; S5 filesystem crawler exists with synthetic tests; S6 adds parser contracts, basic DOCX extraction, and honest PDF text-layer/image-only/unknown classification; S7 adds basic text cleanup, offset mapping, and heading/fallback sectioning; S8 adds real Tantivy indexing/search and CLI search over an existing index; S9 adds a synchronous synthetic DOCX/PDF fixture import loop that persists metadata/index state and searches after reader reopen. | Production import worker, broader PDF text extraction, durable worker claiming/lease semantics, 100k text import benchmark. | The S9 importer is a narrow smoke path, not the final asynchronous import worker. | Large synthetic or desensitized corpora are not present. |
| P2 fields and dedupe | Field model and confidence rules are specified; S10 adds deterministic synthetic email/phone/date-range/school/degree/skill extraction, original evidence/confidence on `StrongEntity`, `rank-fusion` field summaries and `degree_min`/`skills_any`/`years_experience_min` tests, hashed soft-dedupe skeleton, and CLI `--degree`/`--top-k` smoke wiring. | Production dictionaries, field-index fast fields, candidate/version merge workflow, quality harness, field-labeled evaluation set, and broad multilingual extraction. | S10 is query-time MVP filtering over returned Tantivy hits, not production field indexing or complete P2 dedupe. | Field-labeled desensitized evaluation set and dictionary/license decisions are not present. |
| P3 semantic retrieval | ONNX/vector/RRF architecture is specified; S11 adds typed `Embedder` request/response/vector APIs, deterministic `FakeEmbedder`, `VectorIndex` with deterministic in-memory cosine/dot search tests, and `rank-fusion` RRF hybrid fusion tests. | Real model manifest, batch inference, production vector index, hybrid retrieval integration, recall benchmark. | S11 is only a fake-interface skeleton; fake embedders and in-memory vector tests cannot satisfy production semantic search. | Model choice, license, checksums, distribution approval, and production vector engine selection require human confirmation. |
| P4 OCR | OCR routing, cache, worker isolation, and degradation design are specified; S6 can classify simple unencoded image-only PDFs as OCR-required; S12 adds typed OCR request/response/cache-key/options/timeout/cancellation APIs and a deterministic in-memory OCR_REQUIRED queue with priority/resource-policy claiming, defer/retry, cancellation, and query-path no-claim behavior. | Real OCR client/worker execution, robust scan detection, durable page cache, language pack/profile integration, worker isolation/recovery tests, and scanned synthetic corpus. | Disabled/noop OCR cannot satisfy production OCR. The S12 path intentionally returns deferred/cancelled/timed-out non-execution results and must remain off the query hot path. | `tesseract`/`ocrmypdf` and language packs are absent on PATH; scanned test corpus is absent. |
| P5 packaging | Packaging, signing, install, upgrade, uninstall design exists. | Windows MSI, macOS pkg/dmg, signing/notarization, user-mode daemon install, rollback tests. | Packaging cannot start until binaries exist. | Windows signing certs, Apple Developer credentials, secrets, and platform runners require external setup. |
| P6 performance and stability | Performance targets and fault-injection plan exist; S13 adds a redacted `resume-cli doctor`, required-redaction `export-diagnostics` skeleton, missing/corrupt full-text status, one-query small-data smoke, and helper-level daemon-kill/disk-full simulation tests. | 100k/1M benchmark runner, real fault injection, restart/recovery soak, diagnostics package generation, and performance gates. | S13 is skeleton/smoke only and must not be counted as production P6 performance or stability evidence. Previous smoke commands were deleted and cannot count as stability coverage. | 100k/1M corpus, real query set, platform performance machines, and long-running runners are not present. |

## Production Build Plan

1. Rebuild P0 foundation with tested Rust workspace, core domain/config types, durable metadata schema, daemon/CLI entrypoints, and progress evidence.
2. Complete a real P1 synthetic-data import-to-search loop: crawl, parse docx/PDF text layer, normalize, section, index in Tantivy, query, snippet, restart.
3. Add P2 extraction and filtering with golden synthetic fixtures, confidence/evidence tracking, and candidate/version folding.
4. Continue P3 behind model-license gates: S11 fake interfaces exist, but real locally bundled or user-provided embedding integration must wait for explicit model/license/checksum/distribution approval and a production vector-engine decision.
5. Continue P4 OCR from the S12 skeleton toward an isolated asynchronous worker. Until OCR dependencies and scanned corpus are available, mark real OCR execution as blocked rather than completed.
6. Continue P6 beyond the S13 skeleton with real diagnostics packaging, benchmarks, fault injection, and soak only with real binaries, platform runners, and explicit signing/release approval.

## Completion Rule

Do not mark the overall goal complete until P0-P6 production gates in
`01_system_design_系统设计/08_安全性能与验收指标.md` and
`02_execution_plan_执行方案/07_里程碑风险与取舍.md` have either passed with evidence
or are explicitly accepted as out of scope by the user.

# Production Readiness Audit

Date: 2026-05-31

This audit records the current gap between the repository and the product goal in
`GOAL.md`. It is intentionally stricter than the S0-S13 long-running execution
checklist: a slice can pass while the full product is still incomplete.

## Current State

- The repository was documentation-only after `f3e1a54 revert: remove goal generated implementation`.
- S1-S9 foundation code has been rebuilt in the current worktree: `Cargo.toml`,
  `Cargo.lock`, and the first `crates/` workspace members now exist with local
  acceptance evidence, including SQLite metadata schema, queue recovery, and
  CLI/daemon lifecycle skeletons, filesystem crawling, and parser skeletons for
  DOCX/PDF classification, text normalization, section fallback, and strong
  email/phone/date-range rules, plus a real Tantivy full-text index/search
  layer and a synchronous synthetic import-to-search smoke path.
- `.github/` is absent. `tests/fixtures/` contains the empty CLI smoke fixture
  plus synthetic DOCX/PDF fixtures for S9 text-layer import and OCR-required routing.
  Runtime data remains local-only and ignored.
- `PROGRESS.md` records S0 through S9 as complete while P0/P1 remain incomplete.
- No repo-local `AGENTS.md` exists. The in-thread workflow instructions remain active.
- Rust is available at `/Users/frankqdwang/.cargo/bin`, but not on the default shell `PATH`.
- `sqlite3` is available. `tesseract` and `ocrmypdf` are not currently available on `PATH`.

## P0-P6 Gap Audit

| Phase | Complete | Incomplete | Must Rebuild | External Blockers |
|---|---|---|---|---|
| P0 architecture skeleton | Design baseline, Git repo, README, PROGRESS, `.gitignore`; S1-S9 workspace/domain/config/SQLite metadata/CLI/daemon/fs-crawler/parser/text-processing/fulltext/import-smoke foundation has local acceptance evidence. | IPC, logs/diagnostics, CI, production async import/index orchestration, and deeper kill-recovery smoke. | Previous skeleton code was deleted and must not count as product progress; future code must carry fresh verification evidence. | Rust must be invoked with the user cargo path unless shell PATH is fixed; local CI/branch protection cannot be fully verified without remote setup. |
| P1 text import and full-text search | Product design for docx/PDF text, normalization, sectioning, Tantivy, snippets; S5 filesystem crawler exists with synthetic tests; S6 adds parser contracts, basic DOCX extraction, and honest PDF text-layer/image-only/unknown classification; S7 adds basic text cleanup, offset mapping, and heading/fallback sectioning; S8 adds real Tantivy indexing/search and CLI search over an existing index; S9 adds a synchronous synthetic DOCX/PDF fixture import loop that persists metadata/index state and searches after reader reopen. | Production import worker, broader PDF text extraction, durable worker claiming/lease semantics, 100k text import benchmark. | The S9 importer is a narrow smoke path, not the final asynchronous import worker. | Large synthetic or desensitized corpora are not present. |
| P2 fields and dedupe | Field model and confidence rules are specified; S7 adds strong email/phone/date-range rules with low-confidence exclusion tests. | Broader extractors, dictionaries, confidence evidence, field filters, candidate/version folding, quality harness. | Any prior skeleton field logic is gone and cannot be reused as completion evidence; S7 strong rules are not full P2 field extraction or dedupe. | Field-labeled desensitized evaluation set and dictionary/license decisions are not present. |
| P3 semantic retrieval | ONNX/vector/RRF architecture is specified. | Embedder, model manifest, batch inference, vector index, hybrid retrieval, recall benchmark. | Fake embedders may only support interface tests; they cannot satisfy production semantic search. | Model choice, license, checksums, and distribution approval require human confirmation. |
| P4 OCR | OCR routing, cache, worker isolation, and degradation design are specified; S6 can classify simple unencoded image-only PDFs as OCR-required. | OCR client/worker, robust scan detection, page cache, timeout/cancel, language profiles, recovery tests. | Noop OCR cannot satisfy production OCR. The OCR path must remain off the query hot path. | `tesseract`/`ocrmypdf` and language packs are absent on PATH; scanned test corpus is absent. |
| P5 packaging | Packaging, signing, install, upgrade, uninstall design exists. | Windows MSI, macOS pkg/dmg, signing/notarization, user-mode daemon install, rollback tests. | Packaging cannot start until binaries exist. | Windows signing certs, Apple Developer credentials, secrets, and platform runners require external setup. |
| P6 performance and stability | Performance targets and fault-injection plan exist. | 100k/1M benchmark runner, fault injection, restart/recovery soak, redacted diagnostics, performance gates. | Previous smoke commands were deleted and cannot count as stability coverage. | 100k/1M corpus, real query set, platform performance machines, and long-running runners are not present. |

## Production Build Plan

1. Rebuild P0 foundation with tested Rust workspace, core domain/config types, durable metadata schema, daemon/CLI entrypoints, and progress evidence.
2. Complete a real P1 synthetic-data import-to-search loop: crawl, parse docx/PDF text layer, normalize, section, index in Tantivy, query, snippet, restart.
3. Add P2 extraction and filtering with golden synthetic fixtures, confidence/evidence tracking, and candidate/version folding.
4. Add P3 interfaces behind model-license gates, then integrate a real locally bundled or user-provided ONNX embedding model only after license approval.
5. Add P4 OCR as an isolated asynchronous worker. Until OCR dependencies and scanned corpus are available, mark real OCR execution as blocked rather than completed.
6. Add P5/P6 packaging, CI, benchmarks, diagnostics, and fault injection only with real binaries, platform runners, and explicit signing/release approval.

## Completion Rule

Do not mark the overall goal complete until P0-P6 production gates in
`01_system_design_系统设计/08_安全性能与验收指标.md` and
`02_execution_plan_执行方案/07_里程碑风险与取舍.md` have either passed with evidence
or are explicitly accepted as out of scope by the user.

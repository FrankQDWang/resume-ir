# Progress

This file tracks long-running Goal execution against
`02_execution_plan_执行方案/10_长时间Goal执行清单.md`.

## Execution Boundaries

- Repository: `/Users/frankqdwang/MLE/resume-ir`
- Data policy: synthetic fixtures only; no real resumes or PII.
- Remote side effects: no push, PR, release, upload, signing, or notarization.
- Slice rule: acceptance command passes before a slice is marked complete.

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

## Command Log

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

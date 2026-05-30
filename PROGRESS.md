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
| S1 | Complete | Rust workspace scaffolded with five crates; red/green tests run; metadata, fmt, test, and clippy passed. | None |
| S2 | Complete | Domain models, typed IDs, error model, and runtime profiles added; slice tests and workspace tests passed. | None |
| S3 | Complete | SQLite schema v1, idempotent migrations, document visibility, resume versions, and retryable ingest jobs added. | None |
| S4 | Complete | `resume-cli status/import/search` skeleton and daemon foreground lifecycle added; smoke commands passed. | None |
| S5 | Complete | `fs-crawler` crate added with recursive scanning, path normalization, extension/temp filtering, fingerprints, and unreachable error status. | None |
| S6 | Complete | Parser trait/common types, DOCX zip+xml text extraction, PDF text-layer/OCR_REQUIRED skeleton, and parser error mapping added. | None |
| S7 | Complete | Text normalization with offsets, section heading/fallback chunking, and strong email/phone/date rules added. | None |
| S8 | Complete | Tantivy full-text index, search planner, commit/reload search tests, deletion filtering, snippets, and ranked CLI search output added. | None |
| S9 | Complete | CLI import-to-search snapshot loop added for synthetic DOCX/PDF fixtures; status/search read committed snapshot across processes. | None |
| S10 | Complete | MVP field extraction and field filters added; `rank-fusion` crate covers degree/skill/experience filters and soft dedupe skeleton; CLI degree filter works on imported synthetic snapshot. | None |
| S11 | Complete | Semantic retrieval skeleton added with model-free fake embedder, in-memory vector index, and RRF hybrid fusion tests. | None |
| S12 | Not started |  |  |
| S13 | Not started |  |  |

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

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test
```

Red output summary:

- Initial scaffold tests failed with `E0425` because `crate_name` and `binary_name` were not implemented yet.
- This confirmed the skeleton tests were checking missing crate exports before implementation.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo metadata --no-deps
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
```

Output summary:

- `cargo metadata --no-deps`: succeeded and listed workspace members `core-domain`, `config`, `meta-store`, `daemon`, and `resume-cli`; Cargo 1.96 emitted a compatibility warning requesting explicit `--format-version`.
- `cargo fmt --check`: passed with no output.
- `cargo test`: passed all scaffold tests across the five workspace crates.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

### S2

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p core-domain -p config
```

Red output summary:

- Initial S2 tests failed with unresolved imports for domain models, typed IDs, error model types, `Profile`, and `RuntimeProfile`.
- This confirmed tests covered the required missing behavior before implementation.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p core-domain
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p config
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
```

Output summary:

- `cargo test -p core-domain`: passed ID generation, domain model, error redaction, and skeleton tests.
- `cargo test -p config`: passed profile default tests and skeleton tests.
- `cargo fmt --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace`: passed all workspace tests.

### S9

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test commands
```

Red output summary:

- Initial S9 test failed because `resume_cli::run_with_state_dir` did not exist.
- This confirmed tests covered state-dir-injected import/status/search before the persistent snapshot implementation.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo run -p resume-cli -- import --root tests/fixtures/resumes
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo run -p resume-cli -- status
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo run -p resume-cli -- search Java
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
```

Output summary:

- `cargo test --workspace`: passed all workspace tests.
- `import --root tests/fixtures/resumes`: imported 2 synthetic fixture documents, both `SEARCHABLE`.
- `status`: reported `indexed_documents: 2`, `searchable_documents: 2`, `ocr_required_documents: 0`, active profile `balanced`.
- `search Java`: returned 2 ranked hits, `java_backend.docx` and `java_payment_text.pdf`, each with a snippet.
- Snapshot recovery smoke: `status` and `search` were run as separate CLI processes after import and read the committed `local-data/cli-index.tsv` snapshot.
- Incomplete/retryable job recovery remains covered by the S3 `retryable_job_query_recovers_interrupted_work` test.
- `cargo fmt --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.

### S10

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli --test commands search_filters_persisted_snapshot_by_degree
```

Red output summary:

- `extractor-rules` failed because `extract_resume_fields`, field `evidence`, and `EntityType::Degree` were missing.
- `rank-fusion` failed because the new filter/dedupe APIs were missing.
- The focused CLI degree-filter test initially failed with nonzero search status before search option parsing and field filtering existed.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo run -p resume-cli -- import --root tests/fixtures/resumes
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo run -p resume-cli -- search "Java" --degree bachelor --top-k 20
```

Output summary:

- `cargo test -p extractor-rules`: passed 3 rule tests covering contact/date evidence plus school, degree, skill, and date-range extraction.
- `cargo test -p rank-fusion`: passed 3 tests covering degree/skill/experience filters and non-contact soft dedupe skeleton.
- `cargo fmt --check`: passed after rustfmt.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace`: passed all workspace tests.
- Snapshot refresh import: imported 2 synthetic fixture documents, both `SEARCHABLE`.
- `search "Java" --degree bachelor --top-k 20`: returned 1 ranked hit, `java_payment_text.pdf`.

### S11

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p embedder
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-vector
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion --test rrf
```

Red output summary:

- `embedder` failed because `Embedder` and `FakeEmbedder` were missing.
- `index-vector` failed because `VectorIndex`, `VectorDocument`, and `InMemoryVectorIndex` were missing.
- `rank-fusion --test rrf` failed because `RankedHit` and `reciprocal_rank_fusion` were missing.
- A follow-up hybrid-interface red test failed because `HybridRankInput` and `fuse_hybrid_results` were missing.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p embedder
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-vector
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p rank-fusion
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
```

Output summary:

- `cargo test -p embedder`: passed 2 tests for deterministic, model-free fake embeddings.
- `cargo test -p index-vector`: passed 2 tests for in-memory nearest-neighbor search and vector replacement.
- `cargo test -p rank-fusion`: passed field-filter tests plus 3 RRF/hybrid tests for explicit hybrid fusion, rank fusion, and top-k truncation.
- `cargo fmt --check`: passed after rustfmt.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace`: passed all workspace tests.

### S8

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext -p search-planner -p resume-cli
```

Red output summary:

- Initial S8 tests failed with unresolved planner and full-text index APIs.
- CLI search test was updated from the S4 "no index" behavior to require ranked hits with `doc_id`, `file_name`, and `snippet`.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p index-fulltext
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p search-planner
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo run -p resume-cli -- search "Java 支付"
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
```

Output summary:

- `cargo test -p index-fulltext`: passed commit-after-reader-reload, deleted-document filtering, topN snippet generation, and doc tests.
- `cargo test -p search-planner`: passed top-k clamping/default planning tests.
- `cargo run -p resume-cli -- search "Java 支付"`: returned one ranked synthetic fixture hit with `doc_id`, `file_name`, and `snippet`.
- `cargo fmt --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace`: passed all workspace tests.

### S7

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p text-normalizer -p sectionizer -p extractor-rules
```

Red output summary:

- Initial S7 tests failed with unresolved imports for text normalization, sectionization, and extractor-rules APIs.
- This confirmed tests covered the missing cleaning, offset mapping, fallback chunking, and strong field extraction behavior before implementation.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p text-normalizer
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p sectionizer
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p extractor-rules
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
```

Output summary:

- `cargo test -p text-normalizer`: passed whitespace/header/footer cleanup, Chinese/English mixed text, offset mapping, and doc tests.
- `cargo test -p sectionizer`: passed heading recognition and paragraph/length fallback chunking.
- `cargo test -p extractor-rules`: passed strong email, phone, date-range extraction and low-confidence exclusion.
- `cargo fmt --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace`: passed all workspace tests.

### S5

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p fs-crawler
```

Red output summary:

- Initial S5 tests failed with unresolved imports for scan APIs, path normalization, extension filtering, and error kinds.
- This confirmed tests covered missing crawler behavior before implementation.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p fs-crawler
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
```

Output summary:

- `cargo test -p fs-crawler`: passed Chinese path scanning, duplicate file names in different directories, temp/unsupported filtering, missing root error mapping, Windows separator normalization, and doc tests.
- `cargo fmt --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed after tightening a test assertion.
- `cargo test --workspace`: passed all workspace tests.

### S6

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p parser-common -p parser-docx -p parser-pdf
```

Red output summary:

- Initial S6 tests failed with unresolved parser-common contract types and missing `DocxParser`/`PdfParser`.
- This confirmed tests covered parser traits, parser error mapping, DOCX extraction, corrupt DOCX handling, text-layer PDF parsing, and OCR_REQUIRED routing before implementation.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p parser-common
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p parser-docx
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p parser-pdf
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
```

Output summary:

- `cargo test -p parser-common`: passed parser trait contract and timeout-to-core-error mapping tests.
- `cargo test -p parser-docx`: passed basic `.docx` text extraction, corrupt zip error handling, support detection, and doc tests.
- `cargo test -p parser-pdf`: passed text-layer PDF parsing, scanned PDF `OCR_REQUIRED`, support detection, and doc tests.
- `cargo fmt --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace`: passed all workspace tests.

### S4

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p resume-cli -p daemon
```

Red output summary:

- Initial S4 tests failed because `resume_cli::run` and `daemon::run_foreground_once` did not exist.
- A follow-up import test exposed a fixture path issue in Cargo's crate-local test cwd; the test was corrected to use the repository fixture path.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo run -p resume-cli -- status
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo run -p resume-cli -- import --root tests/fixtures/empty
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo run -p resume-cli -- search Java
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
```

Output summary:

- `status`: printed `health: ok`, zero indexed/searchable documents, and active profile `balanced`.
- `import --root tests/fixtures/empty`: printed `import_job: queued` with a local skeleton job id.
- `search Java`: printed `results: 0` with the message that the full-text index is not available yet.
- `cargo fmt --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace`: passed all workspace tests.

### S3

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p meta-store
```

Red output summary:

- Initial S3 tests failed with unresolved imports for `MetaStore`, `DocumentRecord`, `ResumeVersionRecord`, `IngestJobStatus`, and `RetryableJob`.
- This confirmed tests covered the missing SQLite storage API before implementation.

```bash
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test -p meta-store
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo fmt --check
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo clippy --all-targets --all-features -- -D warnings
PATH=/Users/frankqdwang/.cargo/bin:$PATH cargo test --workspace
```

Output summary:

- `cargo test -p meta-store`: passed migration idempotency, schema table creation, deleted document filtering, retryable job recovery, resume version recording, and skeleton tests.
- `cargo fmt --check`: passed after formatting.
- `cargo clippy --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace`: passed all workspace tests.

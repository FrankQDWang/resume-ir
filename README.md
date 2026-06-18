# resume-ir

Local-first resume search engine for large collections of Word and PDF resumes.

## Goal

This repository is building a Rust-based local search kernel with:

- a foreground daemon and CLI;
- SQLite metadata and ingest job state;
- Tantivy full-text indexing;
- asynchronous import, parsing, OCR routing, and indexing;
- privacy-preserving local storage, logs, fixtures, and diagnostics.

The hot query path must stay read-only and avoid OCR, full parsing, model inference, or full index merges.

## Current Stage

The project is in production build execution. Work proceeds from `GOAL.md`, the
system design documents, the execution-plan documents, and the current state in
`PROGRESS.md`.

Each completed slice must:

- update `PROGRESS.md`;
- run its listed acceptance commands;
- be committed separately after acceptance passes.

## Common Commands

Before Rust workspace creation:

```bash
git status --short
git log --oneline -3
```

After Rust workspace creation:

```bash
./scripts/ci/verify-local.sh
```

Equivalent core checks:

```bash
cargo metadata --no-deps --locked
cargo fmt --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
./scripts/ci/check-licenses.sh
./scripts/ci/guard-public-repo.sh
```

Do not import real resumes, upload data, push, release, sign binaries, or run heavy OCR/model workflows without explicit human confirmation.

## License

Current source code is licensed under the MIT License. See `LICENSE`. This is
not a product packaging constraint; the source or release distribution license
may change before stable release when bundled OCR/PDF/model runtime components
require a different reviewed license. See `LICENSES/README.md`.

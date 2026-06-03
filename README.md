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
cargo metadata --no-deps
cargo fmt --check
cargo test --workspace
```

Do not import real resumes, upload data, push, release, sign binaries, or run heavy OCR/model workflows without explicit human confirmation.

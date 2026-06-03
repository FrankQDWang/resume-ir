# Contributing

This repository is a local-first resume search engine. Public contributions
must preserve the privacy boundary: never commit real resumes, local data
directories, daemon tokens, diagnostic packages, model caches, or raw personal
data.

## Development Rules

- One PR should do one thing.
- Use conventional prefixes such as `feat:`, `fix:`, `perf:`, `refactor:`,
  `docs:`, `build:`, or `sec:`.
- Add or update tests for behavior changes.
- Keep user data local. Synthetic fixtures under `tests/fixtures/` are allowed.
- Do not add heavy OCR/model workflows to PR CI without an explicit product
  gate.

## Local Verification

Run this before opening or updating a PR:

```bash
./scripts/ci/verify-local.sh
```

The verification script runs Rust metadata, formatting, linting, tests, and the
public repository guard. For small focused changes, also run the narrower test
that proves the changed behavior.

## Public Repository Guardrails

The guard script fails if tracked files include sensitive local runtime paths,
SQLite databases, logs, diagnostic packages, common token formats, private key
material, or model weight/cache artifacts.

```bash
./scripts/ci/guard-public-repo.sh
```

If the guard fails, remove the sensitive file from the index and keep it local.

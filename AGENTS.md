# Agent Instructions

## Truth and Workflow

- Repo files are source of truth. Read the current `GOAL.md`,
  `ACTIVE_GOAL.toml`, `PROGRESS.md`, relevant design docs, and fresh command
  output before relying on memory or older summaries.
- Use Superpowers as the default engineering workflow. Use only the local
  gstack-lite gates for product, scope, plan, and readiness decisions:
  - Direction: use `office-hours-lite` first; use `ceo-review-lite` only when
    scope, ambition, or premise challenge is needed. Stop before planning unless
    the user asks to continue.
  - Planning: use Superpowers planning directly. Use `plan-review-lite` before
    build for non-trivial plans or plans touching contracts, storage, UI,
    migrations, or multiple crates/modules.
  - Build, debug, review, and finish: use Superpowers direct skills plus these
    repository-local instructions. Do not load raw upstream gstack or old
    curated `fw-*` wrappers unless the user explicitly requests them.
  - Readiness: use `ship-readiness-lite` for readiness reports only.
    Push, PR, merge, deploy, release, canary, and branch cleanup require
    separate explicit approval.
- Conditional autonomous delivery exception: when the active goal's
  `ACTIVE_GOAL.toml` explicitly enables autonomous delivery for a future
  implementation run, the run has passed runtime capability attestation, and
  the requested operation is allowed by `[autonomous_delivery.permissions]`,
  routine commit, push, PR, issue, private benchmark, and allowed auto-merge
  operations are pre-authorized for that goal and must not ask for mid-run
  human confirmation. This exception does not allow admin bypass, direct pushes
  to `main`, branch-protection bypass, gate weakening, raw private-data
  publication, or sandbox/credential escalation beyond observed runtime
  capability. If a required capability is unavailable, record the configured
  machine terminal/blocking state instead of asking for routine approval.
- One task has one execution owner.
- For current-state, release-readiness, blocker, or gap questions, start with a
  read-only pass and report fresh evidence before proposing remediation.

## Privacy and Evidence Boundary

- Never commit or upload real resumes, raw resume text, raw queries, candidate
  results, local paths, local runtime data, diagnostic packages, model caches,
  tokens, credentials, private benchmark reports, installer secrets, or other
  personal data.
- Synthetic fixtures under `tests/fixtures/` and `perf/fixtures/` are allowed
  only when they contain no real personal data, secrets, local paths, raw
  queries, or private corpus material.
- Private local witnesses may be used only when the user authorizes them. Keep
  them temporary and local, and commit only redacted aggregate summaries or
  schemas that pass the public boundary checks.
- Keep smoke, W0/W1, private local benchmark, soak/fault, GUI/manual, and
  release-readiness evidence distinct. Do not treat smoke output or dry-run
  plans as stable-release evidence.
- Before any public push, run:

```bash
./scripts/ci/guard-public-repo.sh
```

## Rust Workspace Discipline

- This is a Rust 2021 workspace. Keep `unsafe_code = "forbid"` intact unless a
  current repo document explicitly authorizes a tiny reviewed platform carve-out.
- Prefer focused crates and modules over growing central files. Avoid adding new
  production logic to already-large orchestration files such as
  `crates/cli/src/main.rs`, `crates/meta-store/src/lib.rs`,
  `crates/daemon/src/main.rs`, and large crate-level `src/lib.rs` files. Add a
  module or crate when it gives the concept a clear home.
- Keep public crate APIs small. Prefer private modules with explicit exports.
- Avoid bool or ambiguous `Option` parameters that make call sites read like
  `foo(false)` or `bar(None)`. Prefer enums, named methods, newtypes, or other
  self-documenting API shapes.
- Prefer inline `format!` arguments, collapsed `if` statements, method
  references over redundant closures, and exhaustive `match` statements when the
  domain is closed.
- Newly added traits must include doc comments explaining their role and how
  implementations should use them. Prefer native trait methods returning
  `impl Future + Send` over `#[async_trait]` or `#[allow(async_fn_in_trait)]`.
- Do not create small helper methods that are referenced only once unless they
  clarify a meaningful boundary.
- For tracing async work, instrument the function or method definition with
  `#[tracing::instrument(...)]` when useful. Check whether the callee is already
  instrumented before adding more spans.
- If Rust dependencies change, update `Cargo.toml` and `Cargo.lock` in the same
  slice and verify lockfile consistency with the relevant Cargo checks.

## Contract Surfaces

- Treat these as integration contracts: CLI arguments and exit codes, daemon IPC,
  diagnostics JSON, config files, metadata schema/migrations, release-readiness
  JSON, performance schemas, runtime bundle manifests, installer/package
  contracts, GitHub workflow gates, and `docs/superpowers` specs/plans.
- Before changing a contract surface, search current call sites and fixtures,
  update schemas/docs/tests/check scripts together, and preserve or deliberately
  version compatibility. Use the repo's existing `snake_case` JSON style unless
  a specific contract says otherwise.
- Query hot paths must stay read-only: no OCR, full parsing, heavy model
  inference, or full index merge in the search path. Do not relax documented
  query semantics for latency unless the current goal contract explicitly
  authorizes it.
- Public reports must be bounded redacted aggregates. Any new evidence payload
  needs explicit size bounds and privacy fields that make raw/private leakage
  machine-checkable.

## Testing

- Prefer focused tests before production code. Use crate-level tests for the
  changed behavior first, then broaden only as risk increases.
- For CLI and daemon behavior, prefer integration tests that exercise the public
  command, IPC, or JSON contract rather than unit tests of incidental internals.
- Prefer comparing complete objects or payloads over asserting field by field
  when that keeps failures readable. Do not add tests for statically defined
  constants or negative tests for logic that was removed.
- Avoid mutating process environment in tests. Prefer passing environment-derived
  flags, paths, commands, or dependencies from above.
- When adding a new unit-test module, prefer a sibling `*_tests.rs` file or the
  crate's existing test organization. Do not move existing tests solely for style
  churn.
- UI, visible text, or report-format changes must include appropriate snapshot,
  screenshot, fixture, or contract evidence so reviewers can see the user-facing
  impact.

## Coding and Delivery Discipline

- For non-trivial behavior changes, state the working assumption and success
  criteria before editing.
- Touch only files needed for the current slice.
- Keep interfaces clean; this product has not shipped, so do not add
  compatibility shims unless a current repo document requires them.
- Update `PROGRESS.md` for completed production slices.
- Keep completed work commit-sized. When committing is part of the approved
  delivery scope, commit each completed slice separately after verification
  passes.

## Verification

- Use focused checks first, then the relevant broad checks.
- The default local pre-PR command is:

```bash
./scripts/ci/verify-local.sh
```

- For release/current-stage truth, use the repo's machine gate instead of prose
  inference:

```bash
cargo run --quiet -p resume-cli --locked -- release-readiness --json
```

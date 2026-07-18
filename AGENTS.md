# Agent Instructions

## Truth and Workflow

- Repo files are source of truth. Read the current `GOAL.md`,
  `ACTIVE_GOAL.toml`, `PROGRESS.md`, relevant design docs, and fresh command
  output before relying on memory or older summaries.
- When searching code or docs, prefer `rg` and `rg --files` before slower
  alternatives.
- For long-running active-goal execution, read the current `GOAL.md`,
  `ACTIVE_GOAL.toml`, `PROGRESS.md`, `perf/acceptance-matrix.toml`,
  `perf/current-loop-state.json`, and the goal docs relevant to the current
  slice before implementation work. For the performance-loop goal, treat
  documents 13, 14, 17, and 18 in `03_next_goal_高性能本地检索GUI闭环/` as the
  required startup reads for loop, autonomous-delivery, and evidence contracts.
  For the mixed-directory import product-correctness train inside the same
  goal, also read documents 01, 03, 04, 09, and 10 before changing benchmark,
  classification, import pipeline, storage, privacy, or issue-train contracts.
  Read the full goal-doc directory only when changing goal contracts,
  acceptance schemas, cross-module architecture, or when the current slice
  explicitly depends on multiple goal documents.
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
    Except for operations covered by the conditional autonomous delivery
    exception below, push, PR, merge, deploy, release, canary, and branch cleanup
    require separate explicit approval.
- Conditional autonomous delivery exception: when the active goal's
  `ACTIVE_GOAL.toml` explicitly enables autonomous delivery for a future
  implementation run, the run has passed runtime capability attestation, and
  the requested operation is allowed by `[autonomous_delivery.permissions]`,
  routine commit, push, PR, issue, private benchmark, allowed auto-merge, and
  post-merge branch cleanup operations are pre-authorized for that goal and
  must not ask for mid-run human confirmation. This exception does not allow
  admin bypass, direct pushes to `main`, branch-protection bypass, gate
  weakening, raw private-data publication, or sandbox/credential escalation
  beyond observed runtime capability. If a required capability is unavailable,
  record the configured machine terminal/blocking state instead of asking for
  routine approval.
- One task has one execution owner.
- For current-state, release-readiness, blocker, or gap questions, start with a
  read-only pass and report fresh evidence before proposing remediation.

## Workspace and Git Safety

- The worktree may contain user or generated changes. Never revert, overwrite,
  or reformat changes you did not make unless the user explicitly asks.
- If `main` has tracked dirty files, stop before pull, merge, rebase, sync, or
  branch cleanup and report the dirty files. Do not hide the state with stash
  or checkout unless explicitly authorized by the user or by the active
  autonomous-delivery contract.
- Do not use destructive Git commands such as `git reset --hard`,
  `git checkout --`, force push, or branch deletion unless the user explicitly
  asks or the active autonomous-delivery contract allows that exact operation.

## Privacy and Evidence Boundary

- Never commit or upload real resumes, raw resume text, raw queries, candidate
  results, local paths, local runtime data, diagnostic packages, model caches,
  tokens, credentials, private benchmark reports, installer secrets, or other
  personal data.
- Synthetic fixtures under `tests/fixtures/` and `perf/fixtures/` are allowed
  only when they contain no real personal data, secrets, local paths, raw
  queries, or private corpus material.
- Private local witnesses may be used only when the user authorizes them. For
  autonomous active-goal execution, the user-provided `/goal` prompt plus
  `ACTIVE_GOAL.toml` and successful runtime capability attestation count as
  authorization for the configured private roots only. Keep witnesses temporary
  and local, and commit only redacted aggregate summaries or schemas that pass
  the public boundary checks.
- For mixed-directory benchmark construction, `$RESUME_IR_MIXED_SOURCE_ROOT`
  may point under `$HOME` only when the user explicitly authorizes it. Exclude
  secrets, credentials, system, browser profile, model/cache, build, VCS, and
  runtime-state directories by default. Commit only synthetic fixtures,
  schemas, and redacted aggregate manifests; never commit sampled files, raw
  labels with paths, filenames, text, direct raw file hashes, or private
  manifests.
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
- Target Rust modules under roughly 500 LoC, excluding tests. If a file is
  already above roughly 800 LoC, put new functionality in a new module unless a
  current design doc gives a stronger reason. When extracting code, move the
  related tests and type/module docs near the new owner.
- Keep public crate APIs small. Prefer private modules with explicit exports.
- Avoid bool or ambiguous `Option` parameters that make call sites read like
  `foo(false)` or `bar(None)`. Prefer enums, named methods, newtypes, or other
  self-documenting API shapes.
- If an opaque positional literal is unavoidable, use an exact Rust argument
  comment such as `/*limit*/ 100` or `/*include_archived*/ false`. Do not use
  these comments for strings or chars unless they add real clarity, and keep
  the comment name aligned with the callee signature.
- Prefer inline `format!` arguments, collapsed `if` statements, method
  references over redundant closures, and exhaustive `match` statements when the
  domain is closed.
- Newly added traits must include doc comments explaining their role and how
  implementations should use them. Prefer native trait methods that spell the
  returned future contract explicitly, for example
  `fn run(&self) -> impl std::future::Future<Output = T> + Send;`. Do not use
  `#[async_trait]` or `#[allow(async_fn_in_trait)]` as shortcuts.
- Do not create small helper methods that are referenced only once unless they
  clarify a meaningful boundary.
- For tracing async work, instrument the function or method definition with
  `#[tracing::instrument(...)]` when useful. Check whether the callee is already
  instrumented before adding more spans.
- If Rust dependencies change, update `Cargo.toml` and `Cargo.lock` in the same
  slice and verify lockfile consistency with the relevant Cargo checks.
- If code adds `include_str!`, `include_bytes!`, migrations, embedded assets,
  or other build-time/runtime resource reads, update the relevant fixtures,
  packaging manifests, docs, and CI checks in the same slice.
- Long Rust builds and tests can block on Cargo locks or shared target state.
  Be patient with expected long-running commands; do not kill build/test
  processes by PID unless the user asked to cancel or the command is clearly
  hung.

## Tauri v2 GUI Discipline

- Use only the currently locked Tauri v2 APIs and official v2 documentation. Do
  not copy v1 allowlists, `@tauri-apps/api/tauri`, or APIs moved to plugins, and
  do not force core, CLI, JS API, and plugin patch versions to use one number.
- Treat the WebView as an untrusted caller. Every `#[tauri::command]` must
  validate lengths, enums, paths/URLs, identifiers, and authorization in Rust,
  and return only bounded, redacted data needed by the UI.
- Custom commands are callable by every window by default. When adding or
  changing one, review the single `invoke_handler`, `build.rs` `AppManifest`,
  window capabilities, plugin permissions, and allow/deny scopes together. Do
  not use wildcard windows, broad paths, or `core:default` without justification.
- Use explicit serde request, response, and error types for IPC and keep JS/Rust
  names aligned. Command arguments are camelCase unless
  `rename_all = "snake_case"` is declared. Never register a second
  `invoke_handler`.
- Keep synchronous commands to short in-memory work. Put I/O, blocking
  libraries, and CPU-heavy work behind async commands plus `spawn_blocking` or
  a dedicated worker, with timeouts, cancellation, concurrency limits, and
  bounded results. Never block the main thread or async executor.
- Store only necessary shared data in `State`. Keep lock scopes short, copy
  needed data before `.await`, never hold a lock across IPC, disk, network, or
  callback work, and do not wrap Tauri-managed `State` in an unnecessary `Arc`.
- Use events only for small, low-frequency broadcasts, channels for ordered
  streams, and commands for request/response. Bound payload size, rate, and
  queues; target the intended window and release listeners during cleanup.
- Keep CSP strict and production assets bundled locally. Do not add remote
  scripts, `unsafe-eval`, or remote capabilities. Any exception must document
  its threat model and least-privilege scope.
- Never depend on the current working directory. Resolve bundled resources
  through `bundle.resources` and `BaseDirectory::Resource`; configure sidecars
  with `externalBin`, target triples, bundle coverage, and exact argument
  scopes. Never accept untrusted input through unrestricted `args: true`.
- When changing config, plugins, or capabilities, update Rust/JS dependencies,
  both lockfiles, `build.rs`, the v2 `$schema`, and generated schemas together.
  Verify the merged release configuration with a target-platform `tauri build`.
- Test at three layers: frontend `mockIPC` contract tests with `clearMocks`, Rust
  command/state tests, and target-platform release binary/bundle native smoke or
  E2E. Mock or dev-server success is not native evidence.
- Before release, verify resources/sidecars, signing, notarization, and updater
  end to end. Updater artifacts must be signed and served over HTTPS; private
  signing keys belong only in protected CI secrets, never the repository,
  frontend, logs, or diagnostics.

## Contract Surfaces

- Treat these as integration contracts: CLI arguments and exit codes, daemon IPC,
  diagnostics JSON, config files, metadata schema/migrations, release-readiness
  JSON, performance schemas, runtime bundle manifests, installer/package
  contracts, GitHub workflow gates, and `docs/superpowers` specs/plans.
- Before changing a contract surface, search current call sites and fixtures,
  update schemas/docs/tests/check scripts together, and preserve or deliberately
  version compatibility. Use the repo's existing `snake_case` JSON style unless
  a specific contract says otherwise.
- When a wire/API schema changes, keep Rust structs, JSON schemas, generated
  fixtures, docs/examples, and tests aligned in the same slice. Prefer
  structured parsers and typed payloads over ad hoc JSON/string inspection.
- Query hot paths must stay read-only: no OCR, full parsing, heavy model
  inference, or full index merge in the search path. Do not relax documented
  query semantics for latency unless the current goal contract explicitly
  authorizes it.
- Public reports must be bounded redacted aggregates. Any new evidence payload
  needs explicit size bounds and privacy fields that make raw/private leakage
  machine-checkable.
- Anything injected into model-visible context, GUI state, diagnostics,
  benchmark evidence, or public reports must have explicit size caps. New
  payloads that can grow with corpus size need schema or fixture coverage for
  the cap and redaction behavior.

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
  crate's existing test organization, using an explicit
  `#[path = "..._tests.rs"] mod tests;` when that matches local style. Do not
  move existing tests solely for style churn.
- When tests spawn workspace binaries, use existing repo helpers or
  `CARGO_BIN_EXE_*` paths instead of hard-coded `target/` paths or local
  absolute paths.
- For benchmark work, run the smallest smoke benchmark that proves the harness
  works before running private full-corpus or long soak benchmarks. Keep smoke
  output separate from W0/W1 acceptance evidence.
- UI, visible text, or report-format changes must include appropriate snapshot,
  screenshot, fixture, or contract evidence so reviewers can see the user-facing
  impact.

## Coding and Delivery Discipline

- For non-trivial behavior changes, state the working assumption and success
  criteria before editing.
- Touch only files needed for the current slice.
- Keep interfaces clean; this product has not shipped, so do not add
  compatibility shims unless a current repo document requires them.
- Unless a current PR budget or Scope Exception says otherwise, keep
  non-mechanical changes under 800 net changed lines and complex logic changes
  under 500 net changed lines. Split larger work into reviewable stages based on
  actual dependencies and affected call sites.
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

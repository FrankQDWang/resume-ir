# v0.1.3–v0.1.8 Feature-Train Verification Ledger

This is the auditable test authority for the feature train. A passing row may
be reused only while its input fingerprint and behavior boundary remain valid.
Unknown is not passed. A failure has no reuse value. Repairs rerun the failed
row and rows whose declared inputs changed; they do not reopen unrelated rows.

## Checkpoint

| Round | Scope | Commit | Result | Reuse |
| --- | --- | --- | --- | --- |
| P0-C01 | S810 daemon bootstrap/capability hard cut | `b2e1258dd694dcd5b54ae967ad89b3eb137acadf` | checkpoint committed; prior R07–R12 evidence remains in the S810 ledger | immutable base; do not rerun merely to start this train |

## Row schema

Each execution row must record:

- stable row and runner cell id;
- exact command and behavior boundary;
- git tree plus declared input fingerprint;
- start/end time, exit code and bounded receipt;
- `passed`, `failed`, `invalidated` or `not_run`;
- invalidating commit/files when applicable;
- DMG SHA-256, install receipt id and screenshot SHA-256 for native rows;
- privacy declaration and residual risk.

## P0 contract/version round

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| P0-01 | Product version has one manifest authority and Tauri uses its path form | `f36d7009b194981157cbe7c0f6a9de7fcda9330545dfd89e62772a864d84e69b` | passed | product version helper/config/build-plan changes |
| P0-02 | Install/reinstall/source-binding evidence derives the canonical version | `4404c062a4d1ecfbd25b072f8f83028af2309f7264bf124c6e5c9b9fcb84190e` | passed | lifecycle, source binding or deployment changes |
| P0-03 | Feature-train machine contract and mutation guards are exact | `bcb97b8b4d950ca6b1d054661e980d12e12cd30d6df3646d658b6b14029cd832` | passed | active goal, matrix, loop state, fixture pin or checker changes |
| P0-04 | Public boundary and changed-file whitespace are clean | `d2ca4f1c8ccc9ea236421aeeaf9818c0d0d1375c23e2c4e01846c1dfa504b29b` | passed | any later public-input change |
| P0-05 | OCR runtime pack exposes macOS-only identities only on the supported macOS target | `be176872b22588183ff239c3f1b00e5eb35c3b0c7897f1fe2d74d4ce78bfbbb7` | passed: local focused test and hosted Linux Clippy | OCR runtime-pack target ownership changes |
| P0-06 | Portable workspace tests and reviewed native-runtime tests are separate explicit lanes | `9ee78c55dbfc6fd060112a98abc9a817a82f377b7b33d0871fd84098992eba4f` | passed: local focused plus hosted Linux/macOS portable lanes | daemon test target, native runtime feature, reviewed-pack harness, or lane workflow changes |
| P0-07 | Detail IPC test client completes one bounded HTTP response by `Content-Length`, without requiring transport EOF | `490bd01875132783a30c017814c55266c55ef0eb012f38651845dfcadf9a025b` | passed: local focused plus hosted Linux workspace replay | s49 response reader, response framing, or detail request-limit lifecycle changes |
| P0-08 | Initializing-generation shutdown observes complete discovery and auth withdrawal | `8e7d55aac19e47688bbb7b44022b7cf59b43d073fca46a9c7d6116a66d3f4f74` | passed: local exact plus hosted Linux workspace replay | initializing control-file withdrawal or its test synchronization changes |
| P0-09 | Byte-stability snapshots model the two held process-owner locks without reading their locked bytes | `8d974924d88179b70c62bde4ccf6f279c94c099a106879c2b988be89aa24d8b1:e44e11ffdca60c366e0ac86ba540e4d43800eafe3f4c81f199898927027df1c6:a27afd24f9d912c018ec811c75c013927156bf5b38037f34832edad8be426796` | local exact passes; hosted Windows replay pending | owner-lock names, data-directory locking, or migration byte-stability snapshot helpers change |

P0-01 commands passed on 2026-07-24: the exact product-version Node test,
affected DMG-plan/worktree-release/config Node tests, locked desktop Cargo
metadata and official Tauri `info` config resolution.

P0-02 commands passed on 2026-07-24: the exact source-binding,
release-deployment, install lifecycle, lifecycle journal and reinstall Node
tests. No Rust workspace, frontend suite or DMG build was replayed.

P0-03 commands passed on 2026-07-24: governance mutation tests, performance
contract checker after updating the two invalidated synthetic pins,
autonomous-goal checker, loop-state checker and parallel-runner self-test.

P0-04 commands passed on 2026-07-24: public repository guard and
`git diff --check`. The two user-owned research documents and generated
`node_modules/` remain outside the train.

P0-05 repair round started after hosted Linux Clippy rejected macOS pack
constants and `mac_identity` as dead code under `-D warnings`. Their former
`cfg(test)` ownership made Linux all-target builds compile production macOS
identity data that no Linux test used. The repair gives those production
symbols the exact `macos/aarch64` target boundary instead of suppressing the
warning.

P0-05 focused verification on 2026-07-24:

- `cargo test -p resume-daemon --bin resume-daemon runtime_pack::tests --locked`
  passed: 8 passed, 86 filtered out.
- `cargo fmt --all -- --check` and the changed-file `git diff --check` passed.
- The local Linux cross-target Clippy attempt produced no repository verdict
  because this Mac has no `x86_64-linux-gnu-gcc`.
- A native daemon all-target Clippy attempt was interrupted after the Clippy
  process stopped making progress; it is not recorded as passed.
- Hosted Linux Clippy passed on repair commit `4424204`; the original failing
  boundary is closed.

That hosted job then reached two arm64 Mach-O tests that had been incorrectly
owned by every host target. Linux failed before test behavior with
`current_target() == None`; macOS passed both tests. They are now named and
compiled as macOS arm64 executable-attestation tests, including their fixture
and test-only imports.

The platform workspace run also exposed a separate evidence-lane defect:
daemon integration tests that intentionally require the uncommitted, reviewed
embedding/classifier/OCR runtime packs were part of the default public Cargo
suite. A public GitHub runner cannot possess those local build inputs. The
repair adds the explicit `native-runtime-tests` feature, makes the wholly native
`s4_daemon`, `s50_ocr_worker`, and `s82_classifier_model` targets require it,
and marks only the reviewed-runtime cases in mixed `s20_ipc`,
`s48_search_ipc`, and `s81_daemon_kill` targets ignored without it. Portable
tests in those mixed targets remain in the default suite.

P0-06 focused verification on 2026-07-24:

- Default exact `s20_ipc` reviewed-runtime case: 1 explicitly ignored with the
  bounded reason `requires reviewed native runtime packs`; 32 unrelated tests
  filtered out.
- The same exact case with `--features native-runtime-tests`: 1 passed,
  32 unrelated tests filtered out, using the existing local reviewed packs.
- macOS arm64 runtime-pack unit filter: 8 passed, 86 unrelated tests filtered
  out. The Linux follow-up compile failure on its two fixture byte writers was
  closed by giving those helpers the same macOS arm64 ownership as the Mach-O
  fixture.
- Locked Cargo metadata exposes the feature and binds exactly the three wholly
  native integration targets to it.
- Hosted Linux Clippy, workspace tests, CLI closed-loop and daemon closed-loop
  all passed after the lane split. The only subsequent failure was the public
  workflow still invoking the native-only incremental-import script.
- The incremental-import script now explicitly enables
  `native-runtime-tests`, remains in local/full delivery verification, and is
  forbidden in the public PR workflow. Its exact watcher regression passed
  locally: 1 passed, 21 unrelated tests filtered out.
- `check-workflows.sh` passed with the public/native lane separation. The next
  hosted Linux/macOS/Windows reruns remain the decisive final receipts.

The next hosted portable run reached the existing
`detail_distinguishes_stale_from_unpublished_or_invalid_selections` case and
reported `ConnectionReset` from its test-only `read_to_string` call after the
fourth and final request. The test client had treated transport EOF as the HTTP
message boundary even though daemon responses already carry an exact
`Content-Length`. P0-07 replaces that unbounded EOF dependency only in the
affected s49 harness with a 2 MiB bounded frame reader. It accepts a transport
reset only after the declared frame is complete and preserves the reset error
for a partial body.

P0-07 focused verification on 2026-07-24:

- The new exact synthetic regression was observed red against the old
  EOF-based reader with `ConnectionReset`.
- `cargo test -p resume-daemon --locked --test s49_detail_ipc
  http_response_reader_ -- --nocapture` passed: 2 passed, 4 unrelated tests
  filtered out. The pair proves complete-frame acceptance and partial-frame
  rejection.
- `cargo test -p resume-daemon --locked --test s49_detail_ipc
  detail_distinguishes_stale_from_unpublished_or_invalid_selections -- --exact
  --nocapture` passed: 1 passed, 5 unrelated tests filtered out.
- Focused s49 Clippy with `-D warnings`, `rustfmt --check`,
  `guard-public-repo.sh` and `git diff --check` passed. No daemon crate or
  workspace suite was replayed.

The following hosted Linux run stopped earlier in the daemon unit-test binary:
`parent_shutdown_revokes_initializing_discovery_before_bootstrap_finishes`
waited only for `ipc.endpoints.json` to disappear, then asserted that
`ipc.auth` was also absent. Generation withdrawal deliberately removes those
two owned files in that order, so the assertion could run between the two
unlinks. P0-08 makes the existing one-second bounded observation wait for the
complete two-file invariant; it does not increase the deadline or change
production cleanup.

P0-08 focused verification on 2026-07-24:

- `cargo test -p resume-daemon --locked --bin resume-daemon
  ipc::server::tests::parent_shutdown_revokes_initializing_discovery_before_bootstrap_finishes
  -- --exact --nocapture` passed: 1 passed, 93 unrelated tests filtered out.
- Focused daemon-bin Clippy with `-D warnings`, `rustfmt --check`,
  `guard-public-repo.sh` and `git diff --check` passed. No other daemon or
  workspace test was replayed.

The next hosted platform run passed the complete macOS lane and reached one
shared Windows-only test-model defect in 15 meta-store cases. Each byte-stability
snapshot recursively read `data-directory-owner.lock` and
`daemon.owner.lock` while that same test process held the corresponding kernel
lock. Unix permits the read, but Windows correctly returned OS error 33. The
database and migration assertions were not reached by those cases.

P0-09 gives the two exact process-owner lock names a typed `OwnerLock`
snapshot entry. Their presence and file type remain part of the before/after
comparison, but their locked bytes are not read. Every other regular file is
still read byte-for-byte and still fails the test on any read error; this is not
a generic Windows exception or a relaxed ciphertext invariant.

P0-09 focused verification on 2026-07-24:

- `cargo test -p meta-store --lib --locked
  migration_v29::tests::fresh_owner_directory_initializes_and_reopens_exact_current_v29
  -- --exact` passed: 1 passed, 127 unrelated tests filtered out.
- `cargo test -p meta-store --lib --locked --features migration-test-support
  migration_test_support::v28_artifact::tests::public_v28_legacy_fixture_covers_each_byte_stable_hard_cut_head_shape
  -- --exact` passed: 1 passed, 130 unrelated tests filtered out.
- Focused meta-store library Clippy with `migration-test-support` and
  `-D warnings`, `rustfmt --check`, `guard-public-repo.sh` and
  `git diff --check` passed.
- The failed hosted receipt is Platform CI run `30084951841`, Windows job
  `89454828414`. The hosted Windows replay on the repair commit remains the
  decisive receipt. No meta-store crate or workspace test suite was replayed
  locally.

## Version rounds

Rows for v0.1.3 through v0.1.8 are appended when each linked issue opens.
Every round begins with focused failing regressions, retains unaffected earlier
passes, and ends with an exact-commit installed native row before issue closure.

## Final round

The complete resumable parallel matrix, merged-main install and soak remain
`not_run` until v0.1.8 is merged. Their absence does not block an individual
feature issue from closing after its own installed acceptance, but it does
block #217 and release-ready claims.

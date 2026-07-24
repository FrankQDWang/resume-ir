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
| P0-09 | Byte-stability snapshots model the two held process-owner locks without reading their locked bytes | `8d974924d88179b70c62bde4ccf6f279c94c099a106879c2b988be89aa24d8b1:e44e11ffdca60c366e0ac86ba540e4d43800eafe3f4c81f199898927027df1c6:a27afd24f9d912c018ec811c75c013927156bf5b38037f34832edad8be426796` | passed: local exact plus hosted Windows | owner-lock names, data-directory locking, or migration byte-stability snapshot helpers change |
| P0-10 | Oversized resident-command output is tested independently from long-running-command timeout behavior | `3061010c9986b56dd4afd0b10dccde6ee27c51e4dc6c3883ae78ccfaf964a0f6` | passed: local exact plus hosted Windows | resident command pipe cap, timeout precedence, or oversized-output fixture changes |
| P0-11 | One-shot responses half-close after the declared frame, and an orderly request-limit exit waits for its final peer close | `52bc4c9590e42f3bab34c38d109de6e4c5284041455276200c6de196f2b7e517:b238323d0018b2a3bc76e262a02fd1a7ad9d857cf2967f8337b62cf059b8612a:1c169b8e7c563b027b9970bc0b414d7fb32c859956c72ab1f65f92e14a356736` | invalidated: hosted parallel s49 proved the nested one-second wait was premature | one-shot response framing, final-peer acknowledgement, streaming ownership, or request-limit lifecycle changes |
| P0-12 | Metadata-key restore rejects a cross-platform unsafe authority object without replacing it | `44c9cd156a91eda2fae1f78627e2572e25ffbe7676f64a20df3ea5feb6735680:3e25a1fb07e376f040dd3e3428bae9184746f36efd0db659a7d008432cdbaeac:e44e11ffdca60c366e0ac86ba540e4d43800eafe3f4c81f199898927027df1c6` | passed: local exact plus hosted Windows | metadata-key restore, owner-directory validation, or unsafe-authority fixtures change |
| P0-13 | The final request-limit connection clears request-phase socket timeouts and remains owned until peer close under the single five-second connection deadline | `f1bb4bb8822f093f5b1ee2679c3563d9c3dce39319f1afad15249ca55dde8d08:b238323d0018b2a3bc76e262a02fd1a7ad9d857cf2967f8337b62cf059b8612a:52bc4c9590e42f3bab34c38d109de6e4c5284041455276200c6de196f2b7e517:490bd01875132783a30c017814c55266c55ef0eb012f38651845dfcadf9a025b:f31e55a67aa82e035f4f475c80407814565b6c6fd3771825f7367e53ba992f45` | local exact lifecycle, s48 and affected s49 pass; hosted Linux/Windows replay pending | socket timeout phases, final-peer ownership, connection hard deadline, deferred response ownership, or request-limit lifecycle changes |

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

Platform CI run `30086174951` moved past the former 15 Windows owner-lock
failures and stopped in the benchmark runner's oversized-output contract. The
test combined two independent requirements in one child script: produce more
than the 8 MiB pipe cap, then stay alive for 30 seconds. Windows PowerShell can
retain part of its text output while the process remains alive, so the
five-second benchmark deadline won before the reader could observe the cap.

P0-10 makes the integration fixture emit its oversized payload and terminate.
That test now deterministically owns output classification. The existing
`private_query_command_pipe` unit regression remains the owner for observing a
pipe cap before its reader is joined, while the separate timeout tests retain
long-running-command ownership. No timeout was increased and no retry was
added.

P0-10 focused verification on 2026-07-24:

- `private_query_benchmark_rejects_oversized_resident_batch_stdout` passed
  against the rebuilt exact integration-test binary: 1 passed, 115 unrelated
  tests filtered out, 0.30 seconds.
- The first Cargo invocation never entered the Rust test body and was
  terminated after remaining in the macOS dynamic loader; it is not recorded
  as test evidence or as a failed behavior.
- Exact benchmark-runner test-target Clippy with `-D warnings`, rustfmt,
  public guard and diff checks passed. Hosted Windows remains the platform
  receipt.

The same hosted round independently showed that the previous s49 repair had
correctly rejected a reset before a complete HTTP frame: the fifth and final
detail-contract response was actually truncated at request-limit process exit.
The server's one-shot response functions wrote the declared frame but relied
on ordinary socket drop to establish the response boundary. P0-11 first made
those response functions shut down the TCP write half only after the entire
frame was accepted. The streaming import and batch writers continue using the
separate multi-write/flush path and are unchanged.

PR run `30087200184` proved that half-close alone was insufficient: a different
s49 case reached the same final-request reset while all earlier requests
passed. This isolated the remaining ownership gap to the explicit
`--max-requests` terminal path. The server now marks only the final bounded
request as `AwaitPeerClose`; after the response sends FIN, that connection stays
owned until the client closes or a one-second bounded peer-close read ends.
The existing five-second connection watchdog remains active. Normal resident
daemon requests use `Immediate` and incur no new wait.

P0-11 focused verification on 2026-07-24:

- `cargo test -p resume-daemon --test s49_detail_ipc --locked -- --nocapture`
  passed all 6 directly affected response/detail cases, including both bounded
  reset-reader regressions and both final-request paths.
- The exact keyword-search success case passed with 12 unrelated s48 cases
  filtered out, and the exact redacted status case passed with 32 unrelated s20
  cases filtered out. These are the minimal direct consumers of the shared
  search-response and ordinary HTTP-response finish paths.
- Exact s49 test-target Clippy with `-D warnings`, rustfmt, public guard and
  diff checks passed. No daemon crate or workspace suite was replayed.
- After adding final-peer ownership, the same 6-case s49 target passed again
  and the exact
  `ipc::server::tests::request_limit_stops_status_updater_before_draining_data_plane`
  lifecycle case passed with 93 unrelated tests filtered out. Combined
  daemon-bin/s49 Clippy with `-D warnings`, rustfmt, public guard and diff
  checks passed.
- The failed hosted receipt is PR run `30086174923`; its exact failing test was
  `detail_contract_rejects_legacy_shape_unbounded_ids_and_oversized_pages`;
  the half-close-only follow-up `30087200184` failed
  `detail_and_hydrate_read_one_exact_selection_across_unrelated_publications`.
  Hosted Linux replay on the final-peer repair commit remains decisive.

Platform CI run `30087200255` then passed the repaired owner-lock and
oversized-output boundaries and reached a Windows-only fixture defect in
`privacy_cli_backs_up_and_restores_metadata_sqlcipher_key_without_output_leaks`.
The fixture created an ordinary `metadata-secrets` directory and made it
permission-unsafe only on Unix, so Windows correctly accepted it. P0-12 uses a
regular file at that authority path on every platform and verifies that the
failed restore preserves its sentinel bytes. The existing Unix meta-store
regression separately retains ownership of rejecting a permissive 0755 key
directory without chmod repair; no production validator was relaxed.

P0-12 focused verification on 2026-07-24:

- `cargo test -p resume-cli --test s146_metadata_key_cli --locked
  privacy_cli_backs_up_and_restores_metadata_sqlcipher_key_without_output_leaks
  -- --exact` passed: 1 passed, 0 filtered out.
- Exact s146 test-target Clippy with `-D warnings`, workspace rustfmt,
  `git diff --check` and the public-boundary guard passed. No CLI crate or
  workspace suite was replayed.
- The failed hosted receipt was Platform CI run `30087200255`, Windows job
  `89462046881`. Follow-up Platform run `30088754382`, Windows job
  `89466992046`, passed the exact s146 case before reaching a later daemon
  response-lifecycle failure, so this row is closed.

PR run `30088754395` passed Clippy and the daemon unit binary, including the
request-limit cleanup case, then failed three concurrent s49 integration cases.
This proved that P0-11's one-second peer-close read was a second, shorter
deadline: detail/hydrate responses are owned by a deferred search worker, so
the server could release its final connection and begin process cleanup before
that worker completed under hosted load.

P0-13 first removed the nested one-second deadline rather than increasing it.
PR run `30090661541` then passed s48 and five of six s49 cases but still reset
one final detail response. The remaining timeout was inherited from request
parsing: `TcpStream::set_read_timeout(2s)` changes the shared socket and
therefore also affected the peer-close clone. The lifecycle now clears that
request-phase timeout before waiting and treats any residual timeout as
non-terminal. The existing five-second connection watchdog is the single
bounded owner for both response work and peer-close observation. Only the
explicit final request-limit connection takes this path; normal resident
requests remain immediate.

P0-13 focused verification on 2026-07-24:

- The exact lifecycle regression injects a 25 ms request read timeout, keeps
  the peer open beyond it and proves that final-connection ownership has not
  ended; it passed: 1 passed, 94 unrelated tests filtered out.
- `cargo test -p resume-daemon --test s49_detail_ipc --locked --
  --nocapture` passed all 6 directly affected detail/hydrate and response-frame
  cases.
- The same old hosted commit failed two final deferred search responses on
  Windows in Platform run `30088754382`: `client_disconnect_only_ends_that_connection`
  and `content_update_publishes_a_new_immutable_version_pair`. Both exact s48
  cases passed against P0-13 locally with 12 unrelated cases filtered out.
- After clearing the request-phase timeout, all 6 s49 cases and the same two
  exact s48 cases passed again.
- Combined daemon-bin/s48/s49 Clippy with `-D warnings`, rustfmt, public guard
  and changed-file checks passed. No daemon crate or workspace suite was
  replayed.
- Hosted Linux replay remains decisive for the deferred-response load boundary.

## Version rounds

Rows for v0.1.3 through v0.1.8 are appended when each linked issue opens.
Every round begins with focused failing regressions, retains unaffected earlier
passes, and ends with an exact-commit installed native row before issue closure.

## Final round

The complete resumable parallel matrix, merged-main install and soak remain
`not_run` until v0.1.8 is merged. Their absence does not block an individual
feature issue from closing after its own installed acceptance, but it does
block #217 and release-ready claims.

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
| P0-13 | The final request-limit exit waits first for exactly-once response completion, stops the request watchdog, then grants a bounded TCP delivery window | `e38dc69c9a2fc132b7914cc0299143948b639a0b6f32cd593710fbec855156ba:913985e11e4e026bb8360ff7e783a62f02e23aa99a7fccb046f40a2ad3227369:db79e491b28871d2335eebe2de694fa580eae6796f25e9ac8db8494773c16b7c:25d0d14868986e3b87f845f6e356aa92fbdc607a91bcacd510f39eef18d2428c:f31e55a67aa82e035f4f475c80407814565b6c6fd3771825f7367e53ba992f45:6aa3024047c5efbd23d890edf2db3145f7a711e7ea88b0fd1c82e213dd323f7c` | hosted Linux, macOS, Windows and local exact lifecycle/s48/s49 passed | completion capability, bounded delivery receipt, deferred response ownership, connection hard deadline, or request-limit lifecycle changes |
| P0-14 | Detail IPC integration owns daemon shutdown through the real parent-lifecycle capability after every response is fully read | `6aa3024047c5efbd23d890edf2db3145f7a711e7ea88b0fd1c82e213dd323f7c` | hosted Linux, macOS and Windows plus local all 6 s49 cases passed | s49 daemon harness, process containment, parent lifecycle, response framing, or detail/hydrate request sequence changes |
| P0-15 | Rejected hypothesis: closing request input after parse prevents the hosted s49 response reset | `2e7f4fb504e027d787ddcc7da15a99dbebff15a1970106e86912c4ece24adb75:52bc4c9590e42f3bab34c38d109de6e4c5284041455276200c6de196f2b7e517:db79e491b28871d2335eebe2de694fa580eae6796f25e9ac8db8494773c16b7c:6aa3024047c5efbd23d890edf2db3145f7a711e7ea88b0fd1c82e213dd323f7c:23fd9ede7e7d330e06afd3181b9095671f8f5d28a7df5157bc2157e9087e329e:f31e55a67aa82e035f4f475c80407814565b6c6fd3771825f7367e53ba992f45` | failed: Linux PR run `30104547488` still reset one s49 response; production change reverted | never reused; retained only as negative diagnostic evidence |
| P0-16 | Historical diagnosis of the non-release Linux s49 reset | `b7910b0140b3fc70044b3286deafcab6152fa79354e36b5700758e348f37c642:b014282b3981a5cd68d72ebb2662dbbf8083c3388f02ea320973b93c3392dc8a:6b02342a05c30852465bb8176f07b7a7d78edfee12cf8f268716129ebbc204b6:23fd9ede7e7d330e06afd3181b9095671f8f5d28a7df5157bc2157e9087e329e` | stopped by product-scope correction; all temporary diagnostics removed | never reused; Linux is not a native release gate for this feature train |
| P0-17 | Daemon IPC integration startup and shutdown are truly bounded, and s49 serializes only expensive fixture construction rather than product execution | `c5541a0d7581c45ca5de78f929f172e891b747f2f22aa6ed96a540ea796c6e4f:e823a4f27c06ee35c4db66f986a228ce42f63a7a02b8a9a42f536d11d60ffae0:89e6282d4af0ff1cfaeeab7c2761a5ca9f733061963557132c56df9d3dc88129:8bbd5f6c560509ee57866c46646361ce761429c4083dd2740d024801613b191b` | local focused execution/Clippy and hosted Windows s48/s49/s81 passed | shared daemon test-process support, s48/s49 harness lifecycle, s49 fixture construction, or detail capability startup changes |
| P0-18 | Test IPC clients consume one bounded `Content-Length` frame and never use transport EOF as the success boundary | `6868904bf1e9e69486e7312e56bb1f9172f962155524143ce1cfc3253c520dad:3c6d6b9a7791d75a9c03d5d40f3cb215ea381f3fe98555522e8cb425b1f9a5a1:5a9e20659fe9618a1375bdbba4169d3ce40c73fe369fed084aa00ac515bf4fdc:88d976d94a78148e91f855a4fb660655290f9dbccee4c21777e5ed8a30e6e146:2070a3cc5da91dfe45accfdc2a87570a6eaec06e5b030f2eed12e03bdd73e764:a7dc48cca6431478f638b92a3a006a6fe2f480706ecf05339c0dffa1ecd9d1d5` | passed: local fail-late batch proved the parser; hosted Linux/Windows then correctly exposed partial production frames | shared HTTP frame reader or the four listed IPC harnesses change |
| P0-19 | Response writers own frame bytes only; the final request-limit lifecycle owner establishes the write boundary after exactly-once completion and watchdog join | `a30ee9944b4ea16f705f2b7896a513730d51979ff259e67c72935315f1f55fdd:7cda9234405a270f67e962d416fa03e6bf97c861934441067f1cfbd942aea3fd:23fd9ede7e7d330e06afd3181b9095671f8f5d28a7df5157bc2157e9087e329e:5a9e20659fe9618a1375bdbba4169d3ce40c73fe369fed084aa00ac515bf4fdc:88d976d94a78148e91f855a4fb660655290f9dbccee4c21777e5ed8a30e6e146:2070a3cc5da91dfe45accfdc2a87570a6eaec06e5b030f2eed12e03bdd73e764:a7dc48cca6431478f638b92a3a006a6fe2f480706ecf05339c0dffa1ecd9d1d5:6868904bf1e9e69486e7312e56bb1f9172f962155524143ce1cfc3253c520dad` | passed: local RED/GREEN, lifecycle 4/4, IPC 19/19 and s20 passed; hosted Linux/macOS plus Windows response-ownership cases passed | response framing, business connection owner topology, completion capability, watchdog join, delivery receipt, request-limit shutdown, or listed IPC consumers change |
| P0-20 | A completed control connection establishes its write boundary in `ActiveControlConnection::join`, including while another socket owner remains alive | `ff4eead677750065655052d556d916b6009a3105d135dca02393a645de0e95a0:25d0d14868986e3b87f845f6e356aa92fbdc607a91bcacd510f39eef18d2428c:a30ee9944b4ea16f705f2b7896a513730d51979ff259e67c72935315f1f55fdd` | local RED/GREEN, hosted-hanging exact case 1/1 and affected control-loop cases 3/3 passed; hosted replay pending | control connection join/cancellation, one-shot response boundary, control-loop handoff, or control-only routing changes |

The complete hosted batch for P0-18 finished before the next repair began.
Security and macOS passed. Linux stopped at an s48 partial response frame;
Windows passed s20, s48, s49 and s81, then stopped at an s83 partial response
frame. The shared reader therefore behaved correctly: it rejected incomplete
declared frames instead of turning a transport close into success. The
failures crossed business and control routes, so they were analyzed as one
production transport-ownership defect rather than patched per test.

P0-19 removes socket-global `shutdown(Write)` from the one-shot response
writers. Those writers can share the socket with deadline and lifecycle owners
and therefore own only the frame bytes. The final request-limit delivery owner
now establishes the write boundary after it receives exactly-once response
completion and joins the watchdog. Ordinary resident requests close when their
last owner drops.

The ownership regression was observed red on the prior implementation: after
`write_http_response`, a lifecycle clone failed its next write with
`BrokenPipe`. The repaired exact unit batch passed 4/4 under Nextest run
`e11f8b01-4642-4836-8302-1a63bc333442`. The fail-late s48/s49/s83/s84 batch
then passed 19/19 under run `97db1e60-a67c-48bd-abf7-4ad17d512f81`,
including both hosted failure cases and the previously unreached s84 target.
The exact s20 request-limit status consumer also passed. No crate-wide or
workspace suite was replayed.

The complete P0-19 hosted batch passed every required check, Linux workspace
and macOS platform job. Windows completed the response-ownership regression,
the affected daemon lifecycle tests and all preceding workspace targets, but
then kept one control-only unit case alive until the 60-minute job limit:
`control_only_owner_never_returns_404_if_ready_is_published_out_of_order`.
That test was the remaining one-shot control client still waiting for TCP EOF.
The response writer could no longer close a socket shared with the control
loop, while the control lifecycle owner had not yet assumed responsibility for
the write boundary.

P0-20 makes `ActiveControlConnection::join` establish `Shutdown::Write` only
after its handler has completed. A cancelled or panicked handler closes both
directions instead. A new cross-platform regression retains an independent
socket owner after join: it failed red with a bounded `WouldBlock` on the prior
implementation and passed green after the lifecycle repair. Nextest run
`b6e653fa-8952-4ffc-a019-27a3533f3e49` passed that regression and the exact
hosted-hanging case 2/2; run `0a3c9514-d110-4eac-a585-1ae6f9387a1c`
passed the three affected control-loop shutdown, accept and blocked-routing
cases. Previously passed business and integration rows remain reusable.

## v0.1.3 schema-v30 implementation round

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| V13-01 | Fresh authority initializes exact v30; exact v29 migrates through encrypted COW; preparing/ready/published receipts recover; future authority fails closed | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9` | passed: 5 exact `migration_v30::tests` cases, 128 filtered out | manifest, registry, COW copy, receipt, source witness, publication or current-store validation changes |
| V13-02 | Missing v29 key cannot create, repair or mutate migration authority | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9` | passed: 1 exact case, 134 filtered out | key read, predecessor validation or migration entry changes |
| V13-03 | Tampered forward-migration checksum fails closed without repair | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9` | passed: 1 exact case, 134 filtered out | registry checksum, history schema or validation changes |
| V13-04 | Native desktop accepts discovery v4/status v4/diagnostics v5 and the bounded migrating state | `ffd3085c7274fb33a0867d99a9ef4a46ee9088bb375dd9a166eec8e4a89eb378` | passed: Nextest `27dd6fdb-8f15-49a3-a052-2abc3587aae2`, 4 passed, 69 skipped | discovery/auth binding, status/diagnostics projection or migrating health contract changes |
| V13-05 | WebView validator and runtime projection accept migrating without stale store authority | `eda866b1987b237da2ead756b8d201aa9b9842865428045afb870699512a4f18` | passed: exact Vitest case, 1 passed, 15 skipped | TS contract validator, daemon health projection or runtime-state mapping changes |
| V13-06 | v30 storage plus daemon/CLI consumers compile as one affected production boundary | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9:ffd3085c7274fb33a0867d99a9ef4a46ee9088bb375dd9a166eec8e4a89eb378` | passed: locked `cargo check` for meta-store, daemon-contract, resume-daemon and resume-cli | any listed crate production source or dependency changes |
| V13-07 | Machine contracts pin bootstrap v2 and the v0.1.3 feature-train versions exactly | `9d2eca4d1c5060c5eeea3c74fbe9d01a795d4b6d080e78b05bb105b4bf137ed2` | passed: exact bootstrap mutation test and performance contract checker | active goal, acceptance matrix, loop pins, checker or fixture pin changes |
| V13-08 | Root-workspace pure status v4 health tests | `ffd3085c7274fb33a0867d99a9ef4a46ee9088bb375dd9a166eec8e4a89eb378` | not_run: two exact test binaries compiled, then remained at zero CPU before emitting test results and were terminated | run only after the local Rust test-process stall is understood or in a clean exact-commit worktree |
| V13-09 | Frontend type contract, Rust formatting and changed-file whitespace | `eda866b1987b237da2ead756b8d201aa9b9842865428045afb870699512a4f18` | passed: TypeScript no-emit, rustfmt check and `git diff --check` | frontend types, Rust sources or changed text changes |
| V13-10 | Affected Rust production targets are warning-free | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9:ffd3085c7274fb33a0867d99a9ef4a46ee9088bb375dd9a166eec8e4a89eb378` | passed: focused root and desktop Clippy with `-D warnings`; test targets were not built | affected production Rust source or dependency changes |
| V13-11 | An internal-test installer binds a worktree artifact manifest, DMG bytes, mounted composition, installed composition and signature without weakening exact-main release provenance | `1ad8acab59a7b9c0042750982b73d4d44ee907b58e948309f58e5c35266ee1c0:794454e8597e98bfe9ddffc132019f7fba2e834c5b7729e3b97c5efc15ed2f85:ea7a20d87ca7a6f18b00eb49752b759c1690df7108d04d864f2fec4f95be44ab` | passed: 3 exact Node cases; valid snapshot installed, DMG drift rejected and copied-App composition drift rejected | worktree artifact schema, source binding, DMG verification, install lifecycle or package runner changes |
| V13-12 | Product manifest is the v0.1.3 version authority and the worktree installer does not duplicate it | `d181f7f1f894655adccfb5d18563ca27373b32ff65149b6c7ef27f4bb01bf450:ea7a20d87ca7a6f18b00eb49752b759c1690df7108d04d864f2fec4f95be44ab` | passed: 2 exact product-version Node cases | product manifest, version resolver, Tauri version path or listed lifecycle scripts change |
| V13-13 | Exact feature commit produces one verified arm64 internal-test DMG and installs it without removing user data | `2175fa7958a435b96828ee51b12fdc793d2e23ae:eedb24209c40c27855db0f3bc101c3a11b60cc8510b35afbc5e94836d5fad708` | passed: DMG `8cdfd7771777b6079c3064a6c42c45dbcac0fcb338f9b0adf2981f5948d9dd6c`, composition `28cbe3ba083ba2e4645a1d4a7a7dccc1c2d735c572bed739a855cb54ec78d87d`, install receipt `4a19ece125c33fd2859e8cc978743ffd58a868814add336abaf00572d9fbdcdc`; `user_data_removed=false` | any bundled source, resource, packaging, worktree installer or product version change |
| V13-14 | Installed v29 authority visibly migrates to v30, retains the source authority and restores the existing searchable/source aggregates | `8699e1cec536ba9fee74bf09908ccbd44ee03c74` | passed: migration screenshot `b867b41e961ee1a630f161afe12955505cba30dc3ce153901321a2821f019cda`; ready-source screenshot `1647c82f473508b63293a93a256c0986f8e0597fbba299f373c9b55b6ef79e23`; 1 root, 8,720 discovered and 7,607 searchable restored | migration/store/daemon bootstrap, migrating projection, source aggregate or data-preservation behavior changes |
| V13-15 | Latest installed exact-v30 authority initializes without claiming another migration and reaches ready with the preserved searchable aggregate | `a62d791a611f3d97d4d7b1d81e6a06f347b1d07ffc083e6504e126a8ebc2f017:db7aa5d311efe5e0b2f28c5bda27c8a3d87257d8edce9fba0eab42c66d28dee0` | passed: initializing screenshot `4d680c2a9d5e3af6c0b03e77f2f6ab38b8c40b7c6af7006e03d0dd9a23c08dce`; ready screenshot `46fd5c77ce4ebeac001c8590324bdbeed2beff4994f39cf31487a2a222d37638`; fail-closed transient status loss recovered to 7,607 searchable | exact-v30 open, lifecycle/status polling, health copy or ready projection changes |
| V13-X01 | Broad Nextest inventory discovery attempt | working tree before V13-08 | not_run: cancelled during integration-binary enumeration before selected root tests executed | never reuse; exact `--lib`/`--bin` targeting is required |
| V13-X02 | Unrelated privacy-maintenance receipt test selected by an overly broad `receipt_` filter | working tree before V13-01 | incidental pass; excluded from v0.1.3 evidence and reuse decisions | never use as feature evidence |
| V13-X03 | Root-launched Vitest command accidentally discovered an immutable cached worktree | pre-V13-15 working tree | invalid invocation: cached copy could not resolve React; current App test did not execute. Replaced by a desktop-root exact run: 2 passed, 3 skipped | never reuse; frontend Vitest commands must run from `apps/desktop` |

The first v29→v30 regression failed before schema application because the COW
copy path reused a create-new-only writer to reopen the already-created staging
database. The repair split `create_encrypted_writer` from
`open_existing_encrypted_writer`; only the failed migration case was rerun at
that point and passed. A later registry review corrected future-chain counting,
which invalidated all five v30 migration cases; those five and only those five
were then rerun together and passed.

The first installed-worktree attempt correctly refused a branch artifact at
the exact-main provenance gate. Supplying the worktree source identity then
proved a second missing boundary: the installer compared snapshot App bytes to
an unrelated current build directory. V13-11 adds a separate internal-test
entrypoint that binds the emitted artifact manifest, recomputed DMG digest,
mounted and installed bundle-composition digests, source identity and signature.
The exact-main installer is unchanged. The existing unreceipted v0.1.2 App was
restored after both failed pre-install checks; neither attempt removed user
data.

V13-14 remains valid after the installer and health-copy repairs because no
store, migration, bootstrap, daemon-contract or migrating-state code changed.
The latest exact-commit DMG was therefore verified with the non-repeating
clean-v30 start in V13-15 instead of mutating the retained real predecessor to
force another migration. Screenshots are local-only aggregate UI evidence and
contain no source path, resume text, query, token or candidate result.

## v0.1.3 schema-v30 implementation round

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| V13-01 | Fresh authority initializes exact v30; exact v29 migrates through encrypted COW; preparing/ready/published receipts recover; future authority fails closed | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9` | passed: 5 exact `migration_v30::tests` cases, 128 filtered out | manifest, registry, COW copy, receipt, source witness, publication or current-store validation changes |
| V13-02 | Missing v29 key cannot create, repair or mutate migration authority | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9` | passed: 1 exact case, 134 filtered out | key read, predecessor validation or migration entry changes |
| V13-03 | Tampered forward-migration checksum fails closed without repair | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9` | passed: 1 exact case, 134 filtered out | registry checksum, history schema or validation changes |
| V13-04 | Native desktop accepts discovery v4/status v4/diagnostics v5 and the bounded migrating state | `ffd3085c7274fb33a0867d99a9ef4a46ee9088bb375dd9a166eec8e4a89eb378` | passed: Nextest `27dd6fdb-8f15-49a3-a052-2abc3587aae2`, 4 passed, 69 skipped | discovery/auth binding, status/diagnostics projection or migrating health contract changes |
| V13-05 | WebView validator and runtime projection accept migrating without stale store authority | `eda866b1987b237da2ead756b8d201aa9b9842865428045afb870699512a4f18` | passed: exact Vitest case, 1 passed, 15 skipped | TS contract validator, daemon health projection or runtime-state mapping changes |
| V13-06 | v30 storage plus daemon/CLI consumers compile as one affected production boundary | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9:ffd3085c7274fb33a0867d99a9ef4a46ee9088bb375dd9a166eec8e4a89eb378` | passed: locked `cargo check` for meta-store, daemon-contract, resume-daemon and resume-cli | any listed crate production source or dependency changes |
| V13-07 | Machine contracts pin bootstrap v2 and the v0.1.3 feature-train versions exactly | `9d2eca4d1c5060c5eeea3c74fbe9d01a795d4b6d080e78b05bb105b4bf137ed2` | passed: exact bootstrap mutation test and performance contract checker | active goal, acceptance matrix, loop pins, checker or fixture pin changes |
| V13-08 | Root-workspace pure status v4 health tests | `ffd3085c7274fb33a0867d99a9ef4a46ee9088bb375dd9a166eec8e4a89eb378` | not_run: two exact test binaries compiled, then remained at zero CPU before emitting test results and were terminated | run only after the local Rust test-process stall is understood or in a clean exact-commit worktree |
| V13-09 | Frontend type contract, Rust formatting and changed-file whitespace | `eda866b1987b237da2ead756b8d201aa9b9842865428045afb870699512a4f18` | passed: TypeScript no-emit, rustfmt check and `git diff --check` | frontend types, Rust sources or changed text changes |
| V13-10 | Affected Rust production targets are warning-free | `7acfcfa717d9a49007615ce1eca19a3b8902e66daec6b343d6d0076f4e52a0b9:ffd3085c7274fb33a0867d99a9ef4a46ee9088bb375dd9a166eec8e4a89eb378` | passed: focused root and desktop Clippy with `-D warnings`; test targets were not built | affected production Rust source or dependency changes |
| V13-11 | An internal-test installer binds a worktree artifact manifest, DMG bytes, mounted composition, installed composition and signature without weakening exact-main release provenance | `1ad8acab59a7b9c0042750982b73d4d44ee907b58e948309f58e5c35266ee1c0:794454e8597e98bfe9ddffc132019f7fba2e834c5b7729e3b97c5efc15ed2f85:ea7a20d87ca7a6f18b00eb49752b759c1690df7108d04d864f2fec4f95be44ab` | passed: 3 exact Node cases; valid snapshot installed, DMG drift rejected and copied-App composition drift rejected | worktree artifact schema, source binding, DMG verification, install lifecycle or package runner changes |
| V13-12 | Product manifest is the v0.1.3 version authority and the worktree installer does not duplicate it | `d181f7f1f894655adccfb5d18563ca27373b32ff65149b6c7ef27f4bb01bf450:ea7a20d87ca7a6f18b00eb49752b759c1690df7108d04d864f2fec4f95be44ab` | passed: 2 exact product-version Node cases | product manifest, version resolver, Tauri version path or listed lifecycle scripts change |
| V13-13 | Exact feature commit produces one verified arm64 internal-test DMG and installs it without removing user data | `2175fa7958a435b96828ee51b12fdc793d2e23ae:eedb24209c40c27855db0f3bc101c3a11b60cc8510b35afbc5e94836d5fad708` | passed: DMG `8cdfd7771777b6079c3064a6c42c45dbcac0fcb338f9b0adf2981f5948d9dd6c`, composition `28cbe3ba083ba2e4645a1d4a7a7dccc1c2d735c572bed739a855cb54ec78d87d`, install receipt `4a19ece125c33fd2859e8cc978743ffd58a868814add336abaf00572d9fbdcdc`; `user_data_removed=false` | any bundled source, resource, packaging, worktree installer or product version change |
| V13-14 | Installed v29 authority visibly migrates to v30, retains the source authority and restores the existing searchable/source aggregates | `8699e1cec536ba9fee74bf09908ccbd44ee03c74` | passed: migration screenshot `b867b41e961ee1a630f161afe12955505cba30dc3ce153901321a2821f019cda`; ready-source screenshot `1647c82f473508b63293a93a256c0986f8e0597fbba299f373c9b55b6ef79e23`; 1 root, 8,720 discovered and 7,607 searchable restored | migration/store/daemon bootstrap, migrating projection, source aggregate or data-preservation behavior changes |
| V13-15 | Latest installed exact-v30 authority initializes without claiming another migration and reaches ready with the preserved searchable aggregate | `a62d791a611f3d97d4d7b1d81e6a06f347b1d07ffc083e6504e126a8ebc2f017:db7aa5d311efe5e0b2f28c5bda27c8a3d87257d8edce9fba0eab42c66d28dee0` | passed: initializing screenshot `4d680c2a9d5e3af6c0b03e77f2f6ab38b8c40b7c6af7006e03d0dd9a23c08dce`; ready screenshot `46fd5c77ce4ebeac001c8590324bdbeed2beff4994f39cf31487a2a222d37638`; fail-closed transient status loss recovered to 7,607 searchable | exact-v30 open, lifecycle/status polling, health copy or ready projection changes |
| V13-X01 | Broad Nextest inventory discovery attempt | working tree before V13-08 | not_run: cancelled during integration-binary enumeration before selected root tests executed | never reuse; exact `--lib`/`--bin` targeting is required |
| V13-X02 | Unrelated privacy-maintenance receipt test selected by an overly broad `receipt_` filter | working tree before V13-01 | incidental pass; excluded from v0.1.3 evidence and reuse decisions | never use as feature evidence |
| V13-X03 | Root-launched Vitest command accidentally discovered an immutable cached worktree | pre-V13-15 working tree | invalid invocation: cached copy could not resolve React; current App test did not execute. Replaced by a desktop-root exact run: 2 passed, 3 skipped | never reuse; frontend Vitest commands must run from `apps/desktop` |

The first v29→v30 regression failed before schema application because the COW
copy path reused a create-new-only writer to reopen the already-created staging
database. The repair split `create_encrypted_writer` from
`open_existing_encrypted_writer`; only the failed migration case was rerun at
that point and passed. A later registry review corrected future-chain counting,
which invalidated all five v30 migration cases; those five and only those five
were then rerun together and passed.

The first installed-worktree attempt correctly refused a branch artifact at
the exact-main provenance gate. Supplying the worktree source identity then
proved a second missing boundary: the installer compared snapshot App bytes to
an unrelated current build directory. V13-11 adds a separate internal-test
entrypoint that binds the emitted artifact manifest, recomputed DMG digest,
mounted and installed bundle-composition digests, source identity and signature.
The exact-main installer is unchanged. The existing unreceipted v0.1.2 App was
restored after both failed pre-install checks; neither attempt removed user
data.

V13-14 remains valid after the installer and health-copy repairs because no
store, migration, bootstrap, daemon-contract or migrating-state code changed.
The latest exact-commit DMG was therefore verified with the non-repeating
clean-v30 start in V13-15 instead of mutating the retained real predecessor to
force another migration. Screenshots are local-only aggregate UI evidence and
contain no source path, resume text, query, token or candidate result.

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

P0-13 first removed the nested one-second deadline. PR run `30090661541`
passed s48 and five of six s49 cases but still reset one final detail response
because request parsing's two-second socket timeout was shared with the
peer-close clone. Clearing that timeout made Linux pass, but Platform runs
`30090661590` and `30093654236` then left Windows in workspace tests for about
47 and 49 minutes. Both peer-close implementations still used transport state
as a proxy for whether the deferred response owner had finished; both
invalidated Windows jobs were cancelled as hung after macOS passed.

Completion-only PR run `30096972706` passed s48 and five of six s49 cases but
still reset one final detail response after its writer completed. This
separates two required phases. `ConnectionCompletion` is the exactly-once
business receipt shared with deferred search/detail workers; it proves the
complete frame reached the kernel. A subsequent transport receipt keeps the
final socket owned until the `Connection: close` client closes, bounded to one
second from the completed response. The transport window never starts before
completion, so it cannot hide slow or lost workers. Lost deferred owners remain
bounded by the existing five-second watchdog. Normal resident requests remain
immediate.

Two-phase PR run `30097965483` passed s48 but reset two s49 final responses.
The request watchdog was still alive during the transport receipt, so a
response that completed within its five-second request budget received only
the remainder of that budget for delivery. When the shared deadline expired,
`Shutdown::Both` reset the completed socket. A deterministic 300 ms regression
first failed with this exact ordering. The corrected lifecycle stops and joins
the request watchdog as soon as completion is observed, then starts the
independent one-second delivery window. The request execution budget remains
five seconds and is not enlarged. The invalidated Platform run `30097965482`
was cancelled after macOS passed while Windows was still testing.

Follow-up PR run `30099276417` passed both lifecycle unit regressions and s48,
but another s49 case reset. This proved the remaining defect was the s49
harness contract, not another transport duration. s49 alone used
`--max-requests` to make the Nth business request double as a process-exit
signal. TCP supplies no portable receipt that the peer application has
consumed a response, so any peer-close duration would remain a guess. s49 now
uses the same contained child and authenticated parent-lifecycle stdin owner as
s48. Every response is read and validated first; `wait_success` asserts the
daemon is still alive, closes the parent capability, and only then expects a
clean exit. All six s49 cases passed locally. The invalidated Platform run
`30099276430` was cancelled after macOS passed while Windows was still testing.

P0-13 focused verification on 2026-07-24:

- The new exact deadline-separation regression passed after failing before the
  repair: 1 passed, 95 unrelated tests filtered out.
- The exact lifecycle regression proves the final connection does not release
  before deferred response completion, then keeps the TCP peer open and proves
  it still does not release before the delivery receipt; closing the peer
  releases it immediately: 1 passed, 95 unrelated tests filtered out.
- The two s49 cases that failed in PR run `30097965483` passed concurrently
  against isolated temporary directories and loopback ports: 1 passed each,
  5 unrelated cases filtered out from each process. The other four s49 passes
  from that hosted run remain valid and were not replayed.
- The same old hosted commit failed two final deferred search responses on
  Windows in Platform run `30088754382`: `client_disconnect_only_ends_that_connection`
  and `content_update_publishes_a_new_immutable_version_pair`. Both exact s48
  cases passed against the completion-capability tree with 12 unrelated cases
  filtered out.
- Focused daemon-bin/s49 Clippy with `-D warnings`, rustfmt and changed-file
  checks passed. No daemon crate or workspace suite was replayed.
- Hosted Linux replay remains decisive for the deferred-response load boundary.

P0-14 focused verification on 2026-07-24:

- `cargo test -p resume-daemon --test s49_detail_ipc --locked -- --nocapture`
  passed all 6 cases with the contained parent-lifecycle harness.
- A first harness attempt using a raw child failed closed before endpoint
  publication because supervised stdin lifecycle requires an isolated process
  group. The final harness reuses the repository's cross-platform
  `ContainedChild`; it does not bypass that safety contract.
- No s48, daemon crate or workspace suite was replayed for the harness change.
- PR run `30100318566`, Security run `30100318710`, and the macOS job in
  Platform run `30100318606` passed. The Windows job remained in workspace
  tests for 35 minutes, beyond the recent 19–26 minute observed range, and was
  cancelled before the prior 47–49 minute failure mode. The harness now bounds
  each post-parent-close child wait to ten seconds and reports only whether
  discovery/auth still exist before terminating the contained process tree.
  This is diagnostic fail-fast behavior, not a retry or a relaxed product
  timeout.

PR run `30103086599` then failed three concurrent s49 responses on Linux while
using the parent-owned daemon harness. Closing the request read half after
every terminal parser outcome was tested as a falsifiable socket-state
hypothesis. It added no retry, sleep, enlarged deadline or peer wait.

P0-15 focused verification on 2026-07-24:

- Exact connection invariant
  `ipc::connection::tests::terminal_request_input_does_not_abort_the_complete_response`
  passed: 1 passed, 96 unrelated unit tests filtered out. It leaves unread
  client input, closes the server read half, writes a 32 KiB bounded response
  and requires a complete non-reset close.
- Nextest run `ede53eeb-ab53-4b8b-a1ff-806c58b88267` ran the six s49 cases plus
  exact s20 malformed-request and s48 client-disconnect boundaries with three
  workers: 8 passed, 44 skipped, 16.893 seconds.
- Focused daemon-bin/s49/s20/s48 Clippy with `-D warnings`, rustfmt and
  changed-file checks passed. No daemon crate or workspace suite was replayed.
- Platform run `30103086413` was cancelled because its commit was invalidated
  by this connection-state experiment.

PR run `30104547488` passed the new invariant, all daemon unit tests, s20 and
s48, then reset
`detail_and_hydrate_read_one_exact_selection_across_unrelated_publications`.
The read-half transition therefore did not fix the hosted failure and has been
reverted rather than retained as speculative production code. P0-15 is closed
as negative evidence.

P0-16 adds bounded test-only diagnosis at the actual failing client boundary.
On reset it records only request ordinal, route, whether the daemon is still
running, received response byte count and declared frame length. It never
records payload, local path, token, request id or candidate data. Nextest run
`444e7973-df5a-48f6-9c7e-f16a24d28d79` passed the two response-reader
regressions and the hosted-failing detail/hydrate case locally: 3 passed,
3 skipped. The diagnostic remains temporary and must be removed after the
cause is proven.

PR run `30105663529` produced two identical first-request traces:
`request_ordinal=1`, `route=/details`, `daemon_state=running`,
`received_bytes=0`, and no declared response frame. This rules out large-body
write truncation, hydrate pagination and response-reader framing. The next
probe is failure-only and distinguishes cancellation-socket clone failure,
watchdog spawn/cancellation and response write/shutdown failure. The test
harness drains daemon stderr continuously into a 64 KiB local cap and emits
only tagged closed diagnostics; untagged stderr, payload, paths, tokens and
request identifiers never enter the CI report.

The exact hosted-failing s49 case passed locally after this diagnostic-only
change. Nextest run `65d9bbe9-7044-4c99-a2df-e99c73f0d323` passed the exact
abortive-response and independent-delivery-window boundaries: 2 passed,
94 skipped. Focused daemon-bin/s49 Clippy, rustfmt, changed-file checks and the
public guard passed. Platform run `30105663695` was cancelled after the Linux
trace invalidated that commit.

The first server-side probe in PR run `30106509107` was intentionally
failure-only but was enabled for every daemon. The existing s20 fault smoke
correctly rejected its tagged stderr after deliberately disconnecting a
client, so that run did not reach s49 and contains no new product verdict.
The probe is now enabled only in the s49 child process; all other tests and
normal product runs retain empty stderr. Nextest run
`02461f85-6549-4c62-a9e1-b3fd0b193b0f` ran the exact s20 fault smoke and
hosted-failing s49 case in parallel: 2 passed, 37 skipped, 2.477 seconds.
Focused daemon-bin/s20/s49 Clippy, rustfmt, changed-file checks and the public
guard passed.

Product-scope correction: Linux is not a native target or release gate for
this macOS-first feature train. Runs `30107075347` and `30107075263` were
cancelled, all temporary reset diagnostics were removed, and no further
feature work is gated on that investigation. This historical row remains only
to explain the consumed evidence; it cannot delay v0.1.3 implementation.

Platform CI run `30134629516` then completed the macOS workspace lane and the
Windows workspace through s20 and all default s48 cases. Windows entered s49,
passed both pure HTTP frame-reader cases, then ran all four store-backed cases
for more than 36 minutes until the job's 60-minute hard limit cancelled the
batch. No additional failure was printed; s81, s83 and s84 were not reached.

P0-17 removes both sources of unbounded test behavior without changing daemon
production timing. The shared test-process module drains stdout on a dedicated
thread, publishes discovery through a channel and keeps the caller's endpoint
deadline real. It also provides a bounded contained-child exit receipt. s49 now
waits on authenticated `daemon.status.v3` detail capability state rather than
the old post-initialization stdout line, fails immediately on a closed blocked
state, and bounds every loopback request. Only the expensive encrypted
store/index fixture construction is serialized; the actual daemon and IPC
cases remain parallel.

P0-17 focused verification on 2026-07-25:

- `cargo test -p resume-daemon --locked --test s48_search_ipc --test
  s49_detail_ipc -- --nocapture` passed: s48 6 passed and 7 reviewed-runtime
  cases remained explicitly ignored; s49 passed all 6 cases in 21.36 seconds.
- Focused s48/s49 Clippy with `-D warnings`, `cargo fmt --all -- --check` and
  `git diff --check` passed in the same fail-late batch.
- The decisive Windows continuation must run the invalidated s48/s49 targets
  and the previously unreached s81/s83/s84 targets. The already-passed macOS
  workspace receipt remains valid and is not replayed manually.

Hosted run `30137653620` closed P0-17's original hang: Windows completed s48,
s49 and s81. s49 passed all six cases in 21.05 seconds. The run then reached
s83 and reported one `ConnectionReset` while the client used
`read_to_string` to wait for transport EOF after a complete blocked-status
response. Four sibling s83 cases passed; s84 was not reached because Cargo
stopped after the failed test binary. The PR and Security workflows and the
macOS platform job all passed independently.

P0-18 moves the already-tested s49 frame parser into shared test support.
s48, s49, s83 and s84 now accept exactly one bounded response by declared
`Content-Length`; a reset after the complete frame is irrelevant, while a
partial frame, missing/invalid length, overflow, extra bytes and oversized
response still fail. s83 and s84 also use bounded loopback connect/read/write
deadlines.

P0-18 focused verification on 2026-07-25 ran every invalidated or previously
unreached target without short-circuiting:

- s48: 6 passed, 7 reviewed-runtime cases explicitly ignored.
- s49: 6 passed.
- s83: 5 passed, including the hosted Windows failure.
- s84: 2 passed; this target had not run in the failed Windows batch.
- Combined target Clippy with `-D warnings`, rustfmt, changed-file checks and
  the public guard passed.

## Version rounds

The original v0.1.3 rows remain historical evidence for their exact inputs.
The approved execution contract subsequently made v0.1.3–v0.1.8 one atomic
business implementation train: later production code, contracts, packaging
declarations and UI were completed without intermediate test, build, Linux,
DMG, install or Computer Use runs. No historical pass is inherited when its
declared behavior boundary or input fingerprint changed.

| Round | Business boundary | Status | Validation disposition |
| --- | --- | --- | --- |
| V14 | schema v31 source-root authority, path truth, watcher/periodic/manual scan coordinator and per-root progress UI | implementation_complete | not_run |
| V15 | schema v32 durable root deletion and privacy purge | implementation_complete | not_run |
| V16 | schema v33 PDFium text/render runtime and resumable OCR/reprocess | implementation_complete | not_run |
| V17 | selection-bound original-PDF reader and resizable detail drawer | implementation_complete | not_run |
| V18 | selection-bound native source reveal and final v0.1.8 packaging contract | implementation_complete | not_run |

## Final round

The complete resumable parallel matrix, exact-tree macOS DMG/install,
Computer Use acceptance and 120-minute soak remain `not_run` until the full
v0.1.8 business tree is frozen. The matrix runs fail-late and records every
failure before any repair. Repair invalidates only the rows whose declared
inputs or behavior boundary changed; all other passes remain reusable. Until
these rows pass, #217 and every release-ready claim remain open.

### Final matrix round F01 — 2026-07-25

The frozen v0.1.8 tree entered the resumable fail-late matrix. No Linux product
lane was run. Runner receipts remain local and contain only bounded command
metadata and redacted logs.

| Row | Behavior boundary | Receipt | Status |
| --- | --- | --- | --- |
| F01-01 | Workspace Rust targets satisfy deny-warnings Clippy policy | `20260725T172451Z-2dad7880` | passed in 742.37 seconds; later PDFium source-identity constant change invalidated only the daemon boundary |
| F01-02 | Embedder tests | `20260725T173724Z-56c66437` | passed in 406.26 seconds; reusable |
| F01-03 | Benchmark-runner tests | `20260725T173724Z-56c66437` | passed in 586.91 seconds; reusable |
| F01-04 | License gate | `20260725T173724Z-56c66437` | passed; reusable |
| F01-05 | Local quality release evidence | `20260725T173724Z-56c66437` | passed; reusable |
| F01-06 | Release SBOM | `20260725T173724Z-56c66437` | passed; reusable |
| F01-07 | Desktop Tauri Rust tests | `20260725T180648Z-bdeef06f` | passed after removing the obsolete path-based import IPC and correcting v5 fixtures; 63 passed |
| F01-08 | Desktop Tauri deny-warnings Clippy | `20260725T180754Z-72692d5a` | passed after the same dead-contract removal; reusable |
| F01-09 | Release/operator runbooks | `20260725T180856Z-1a99114b` plus focused current-tree rerun | passed after exact vocabulary and Xcode prerequisite documentation |
| F01-10 | macOS Xcode/PDFium build prerequisite | focused Node tests and native preflight | 5 tests passed; explicit `DEVELOPER_DIR` is isolated from global selection; current host blocked before source synchronization because only Apple Command Line Tools are installed |
| F01-11 | Changed-file whitespace | focused current-tree rerun | passed |
| F01-12 | Daemon/PDFium source-identity deny-warnings Clippy boundary | `CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo clippy -p resume-daemon --bin resume-daemon --locked -- -D warnings` | passed in 6m02s; replaces only the daemon boundary invalidated after F01-01 |
| F01-13 | Public repository privacy boundary | `./scripts/ci/guard-public-repo.sh` | passed on the current public-input tree |

The fail-late receipt `20260725T173724Z-56c66437` also recorded one shared
native-link blocker across workspace-core, resume-cli, CLI/daemon closed loops,
incremental import, benchmark smoke, local OCR, diagnostics evidence and
release readiness: the reviewed macOS `libpdfium.a` pack did not yet exist.
The worktree bundle failed for the same missing pack. This was not treated as
ten unrelated product defects.

The source builder was then repaired to use real pinned depot_tools revision
`f394ab2c993283e94680ca13db98b99927868e98`, deterministic minimal PDFium
checkout configuration and official depot_tools bootstrap. The pinned PDFium
source synced successfully. Generation then failed at the official Chromium
macOS SDK probe because the machine's selected developer directory is
`/Library/Developer/CommandLineTools` and no complete Xcode installation is
present. PDFium's official macOS prerequisites require Xcode; the build does
not spoof `xcodebuild`, patch the upstream checkout or claim a Command Line
Tools build as release evidence.

The production builder now performs this capability check before any network
or source-sync work, prefers a command-scoped explicit `DEVELOPER_DIR`, falls
back to the selected Xcode directory, binds that directory to the child build,
rejects symlinked/unsafe selections and leaves the global `xcode-select`
setting unchanged. Once complete Xcode is available, the remaining
invalid/failed set is exactly:

- `cargo-test-workspace-core`
- `cargo-test-resume-cli`
- `cli-closed-loop`
- `daemon-closed-loop`
- `daemon-incremental-import`
- `benchmark-smoke`
- `local-ocr-runtime`
- `local-diagnostics-release-evidence`
- `release-readiness`
- `desktop-macos-worktree-bundle`

Those cells, followed by installed macOS acceptance, Computer Use and the
120-minute soak, remain `not_run`/blocked. Already-valid rows above must not be
replayed merely because the host prerequisite is later supplied.

### Final repair round F02 — 2026-07-26

The host prerequisite is now satisfied by `/Applications/Xcode.app`:
`xcodebuild -version` reports Xcode 16.4 (build 16F6). The reviewed local
PDFium build pack exists under the private build cache, and the bounded runtime
identity pack is staged for daemon tests. This unblocks only the ten F01 cells
listed above; reusable F01 passes remain closed.

The current repair-boundary fingerprint is
`e3843830c1be594c478817c2370f980664dfa5bf0d66198617953ac6eb6f4c40`
over the source-root retry, import publication, frozen-PDF manifest,
PDFium-attestation and watcher regression inputs. The repository base is
`0f1b45c3fdd10eb857a0feb56d768745e8331865`; this fingerprint intentionally
describes the dirty feature-train tree rather than claiming an immutable
commit.

| Row | Behavior boundary | Exact command/result | Status |
| --- | --- | --- | --- |
| F02-01 | A retryable source-root failure receives a new scan attempt while a queued retry keeps its identity | `cargo test -p meta-store --lib --locked source_roots_tests::source_root_retry_ -- --test-threads=1`; 2 passed | passed; reusable unless source-root retry/head logic changes |
| F02-02 | Frozen PDFs remain part of source scan truth without making the sealed import disposition manifest impossible to satisfy | `CARGO_INCREMENTAL=0 cargo test -p import-pipeline --lib --locked import_run::orchestrator::tests:: -- --test-threads=1`; 2 passed, 126 filtered out | passed; reusable unless import summary, PDF policy or sealed-manifest construction changes |
| F02-03 | A completed watched root is requeued after text and PDF-family changes using the current reviewed runtime contracts | `CARGO_INCREMENTAL=0 cargo test -p resume-daemon --locked --test s4_daemon --features native-runtime-tests foreground_import_watcher_requeues_completed_root_after_word_and_pdf_change_without_path_leak -- --exact --test-threads=1`; 1 passed, 21 filtered out, 53.62 seconds | passed; reusable unless watcher/coordinator, source occurrence publication, runtime attestation or daemon test harness changes |
| F02-04 | Rust formatting after the repair set | `cargo fmt --all -- --check` | passed; invalidated by later Rust edits |
| F02-X01 | Initial exact watcher retry with incremental compilation enabled | compilation remained at zero CPU before test execution and was cancelled orderly | not a test result; never reuse |

Earlier failing watcher attempts have no reuse value. Their combined root
causes were repaired at their owning boundaries: failed scan attempts now
restart instead of retaining terminal identity; searchable source occurrences
are published only after immutable document/version facts exist; frozen PDF
deferrals are excluded from the sealed processed-disposition count while
remaining visible in source scan progress; and the daemon's reviewed PDFium
identity constants match the generated pack. No Linux lane was run.

### Final fail-late round F02-B — 2026-07-26

Runner receipt `20260726T091240Z-183931d2` resumed exactly the ten F01 cells
that Xcode/PDFium had blocked. It ran to completion without fail-fast:

| Cell | Result | Reuse disposition |
| --- | --- | --- |
| `cargo-test-workspace-core` | failed after 866.62 seconds: 125 import-pipeline unit tests passed and three PDF fixture/metrics cases failed | failure has no reuse value; only the parser/import boundary is reopened |
| `cargo-test-resume-cli` | failed after 334.16 seconds: all reached tests passed except s146 expected schema 29 while the current authority returned 33 | failure has no reuse value; s147 had not yet executed and shares the same stale literal |
| `cli-closed-loop` | passed in 39.89 seconds | invalidated by the subsequent PDFium metrics CLI-output hard cut; rerun this cell only |
| `daemon-closed-loop` | passed in 25.03 seconds | reusable |
| `daemon-incremental-import` | passed in 109.35 seconds | reusable |
| `benchmark-smoke` | passed in 22.36 seconds | reusable |
| `local-ocr-runtime` | passed in 2.82 seconds | reusable |
| `local-diagnostics-release-evidence` | passed in 4.08 seconds | reusable |
| `release-readiness` | passed in 3.37 seconds | reusable |
| `desktop-macos-worktree-bundle` | passed in 241.30 seconds | invalidated by the subsequent source repair; do not install this artifact |

The invalidated pre-repair artifact was
`resume-ir_0.1.8_aarch64_e299dc95a986.dmg`, source-tree SHA-256
`e299dc95a9868f4a2dd06d0a619f8ae75329def51512bb7ba665cfb8cef436f9`,
DMG SHA-256
`dc6b69dc4903ed428be5948c220ad30a6397a35c279af202e88085a6430fdc6e`
and composition digest
`9827aff82096f70b5d463336dddc3369e7f011b4b4d5cd89ef3b46d7e08261ec`.
It is retained only as bounded build evidence and is not an install candidate.

F02-03 and `daemon-incremental-import` were the same exact test under
different evidence identifiers. The latter therefore repeated one already
passed test. Future executions must use the manifest cell id
`daemon-incremental-import` as the ledger authority rather than recording an
ad-hoc alias row.

### Final repair round F03 — 2026-07-26

The three workspace-core failures were caused by legacy lopdf fixture
assumptions rather than a production PDFium defect. The UTF-16BE literal used a
Type1 Helvetica font without `ToUnicode`, so its raw bytes are not valid
visible Chinese text; that case now asserts the intended OCR-required
disposition. The valid `ToUnicode` fixture had used `T*` without defining text
leading, placing every line at the same location; it now has explicit leading
and remains directly searchable. The production visual-text quality gate is
unchanged.

The failure exposed a second contract defect before the next assertion could
run: `PdfTextExtractionTimings` and CLI output still named removed lopdf phases
that PDFium could never populate. F03 hard-cuts that dead surface to
`PdfTextExtractionMetrics`, containing only PDFium document load, page-text
load, character iteration, quality evaluation, page/character and byte
counts. No compatibility fields or zero-valued aliases remain.

The CLI key tests now use the exported `CURRENT_SCHEMA_VERSION` authority
instead of literals 29 or 33, covering both backup/restore and the previously
unreached rotation case.

Focused F03 verification:

- `tests::import_root_routes_utf16be_literal_without_tounicode_to_ocr` passed:
  1 passed, 127 filtered out, 13.65 seconds.
- `tests::import_root_keeps_tounicode_cmap_pdf_text_layer_searchable_without_ocr`
  passed: 1 passed, 127 filtered out, 14.62 seconds.
- `tests::parallel_parse_workers_record_pdfium_and_post_parser_metrics` passed:
  1 passed, 127 filtered out, 14.64 seconds.
- `cargo test -p resume-cli --locked --test s146_metadata_key_cli --test
  s147_metadata_key_rotation_cli -- --test-threads=1 --nocapture` passed both
  exact one-case targets.
- `cli-closed-loop` passed in runner receipt
  `20260726T103040Z-3f02d3f5`: one executed cell, no failure.
- Rust formatting and focused changed-file whitespace checks passed.

Before the successful run, macOS repeatedly held new Rust processes before
their entry point while Storage Management and `syspolicyd` scanned local
build artifacts. Attempts cancelled before a selected test entered Rust
`main` remain `not_run`, not failures. Closing the Storage settings UI and
terminating its exact helper processes did not modify project or user data.
The user accepted the macOS execution prompt, after which compiler and test
processes ran normally.

One attempted `launchctl submit` workaround was incorrectly inferred by
launchd as a keep-alive job. It restarted the same UTF-16BE exact case six
times before removal; its overwritten logs are not evidence. The subsequent
direct, single execution above is the authoritative result. No further
launchd-backed test execution is allowed in this train.

### Final native manual round F04 — 2026-07-26

The repaired worktree produced one new internal-test artifact in runner receipt
`20260726T103209Z-3ba42dfb`. The `desktop-macos-worktree-bundle` cell passed in
306.43 seconds with input fingerprint
`ecc1133f009c47747e942b9df34f28af49c5b445de503786f2e80d1ab70723dd`.
The artifact is `resume-ir_0.1.8_aarch64_f161c5b820ad.dmg`, source-tree digest
`f161c5b820adb5e1006c9f2ebe99283d242408497c75a2a9f7821742b0426bb1`,
DMG digest
`caabc8890fe699f0f0a074b5f791c1617afed38175f6bfb09fa910f235315897`
and App composition digest
`cbe255133e4af2d43588361300ce7ceaea18ac9f1a8f58a2bff121b2f8fb392b`.
The mounted composition contained one arm64 App, daemon, embedding and PDF
renderer sidecars, all four reviewed runtime packs, an ad-hoc valid hardened
runtime signature and no build-machine path marker.

The receipt-bound installer installed version 0.1.8 in `/Applications` and
returned `user_data_removed=false`. The previously installed 0.1.3 App and its
incompatible pre-release install receipt were archived as recoverable local
artifacts after both current install and uninstall transactions correctly
failed closed. No metadata database, encryption key, authorized root, search
index or source file was moved during that installer-contract hard cut.

Computer Use then launched the installed App against an isolated synthetic
HOME. Acceptance proved:

- an empty current store initialized directly at schema v33 and reached daemon
  ready without reading the operator's normal application data;
- one selected directory displayed per-root progress, nullable ETA, watcher
  state, start/rescan, pause and delete controls;
- first scan reached 100% with one discovered/searchable TXT; a zero-change
  rescan kept the count at one;
- adding a public synthetic PDF was detected by the live watcher without a
  manual scan, increasing the searchable count to two;
- keyword search returned the exact synthetic TXT and PDF;
- the detail drawer exposed structured fields, extracted text, a draggable
  left edge and reset control; a visual drag increased its width;
- Finder reveal selected the exact synthetic TXT without returning a path to
  WebView state;
- the original-PDF view rendered the visible one-page synthetic PDF inside the
  drawer;
- root deletion revoked the old generation, removed the root card and all
  search results, and a new explicit search returned zero of zero;
- both source files remained present, and the PDF retained digest
  `38a88c1ebeb3d02b499b3dfb04e952dcfd346c87df3217707b669eb74ee1c011`;
- normal App exit left no installed App/daemon/runtime process, IPC file or
  lifecycle workspace behind.

The six local-only screenshots are bounded synthetic UI evidence. Their
digests, in state order, are:

- initial:
  `187af378166642b50a890c20a6458894ab3786a5601a7ddccbb65db2bd292acd`
- scan:
  `cf1e8b9a45bc5bbc46f4424ce9a7f1243bb70620f88cd71c672a69ce242f668e`
- resized detail:
  `17f2fdd16f49e6dcdc2411f36866dc19c0b2ce2b5902ae3c4f7687a33285eb1c`
- Finder reveal:
  `0f54c6273a618134cc55c47d111ad524bd8ddb53063205a981f4125fb78c3866`
- PDF:
  `3a1e996853486dc60d01563634d28571e3e2ee9b652ec76c24c40c308260835f`
- deleted:
  `83a39b0532ce77dbe93b8d42813e7a549584aae26909542d15151e95ecf6be8e`

The post-repair focused deny-warnings boundary passed for `parser-pdf`,
`import-pipeline` and `resume-cli`. Its `--all-targets` selection linted every
CLI test target and took 8 minutes 09 seconds; that was wider than necessary.
It is retained as valid evidence, but future repair rounds must select only the
affected lib/bin and named test targets.

Every failed or unrun F02-B cell is now closed by its authoritative focused
repair result, while the unaffected F01/F02-B passes remain reused. The
remaining release-train sequence is commit/PR reconciliation, an exact
merged-main build and installed-main acceptance, followed by the uninterrupted
120-minute soak bound to that same commit. No soak result is claimed yet.

### Platform boundary correction F05 — 2026-07-26

The user reaffirmed that the current delivery is macOS-only. Windows and Linux
execution, investigation, repair, packaging and evidence are out of scope and
cannot block this train unless a later explicit user instruction changes the
boundary. The repository-level agent contract and active-goal machine contract
now preserve this rule across context compaction and agent handoff.

One already-running non-required Windows CI retry reached the workspace test
step and was cancelled by its 60-minute job limit. It is recorded only as an
out-of-scope observation: it is not a failure to repair, it will not be rerun,
and it does not invalidate any macOS evidence. The PR and required security
workflows now execute on macOS; the non-required platform workflow no longer
runs on pull requests and has no Windows matrix. Required check names and
branch protection remain unchanged.

### macOS PR PDFium prerequisite repair F06 — 2026-07-26

PR run `30202037705`, job `89793521886`, completed metadata, the search
boundary, formatting and workspace Clippy, then failed before executing any
test because the linker could not find `libpdfium.a`. This is a CI prerequisite
failure, not a failed business assertion. The exact macOS DMG path had already
proved the reviewed static archive; the PR workflow alone had omitted that
same preparation boundary.

The repair adds an exact-key Actions cache for the reviewed macOS PDFium static
pack. Cache misses build from the pinned source contract; every restored or
new pack is revalidated before Cargo runs. No dynamic fallback, feature
disablement or test skip was added. Independent post-test checks now use
`!cancelled()` so one failed test cell does not hide the remaining failures.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F06-01 | PR workflow prepares and validates the same reviewed macOS PDFium static archive required by production linking | `0c64de0fa53675e9714eb49013e014baeb5b3a6abb5f5a692aae4a45d983d7b7:a9676ff49e0a7f852fe4753672540782837ace154292200ab76af166a3e0184c:2f52425a0fc7e0ac5d38415fb8655e3692f4f132ec695e1e6ff53297579e2665` | focused workflow checker and pack verifier passed | PR workflow, PDFium contract/build/verification or workspace static-link configuration changes |
| F06-02 | The PDF renderer test targets link against the reviewed archive | `0c64de0fa53675e9714eb49013e014baeb5b3a6abb5f5a692aae4a45d983d7b7` | `cargo test -p resume-pdf-render-runtime --locked --no-run` passed in 0.43 seconds | PDF renderer crate, PDFium archive, Cargo link configuration or target toolchain changes |

One attempted local execution was left launch-suspended by the command runner
after linking. It executed no test body, was terminated by exact PID, and is
`not_run`; F06-02 is the authoritative compile/link result. No previously valid
workspace or feature row was replayed.

### clean-checkout PDFium dependency repair F07 — 2026-07-26

PR run `30202411939`, job `89794541844`, proved the first cache-miss path was
not hermetic. The builder requested PDFium's `minimal` checkout, while the
pinned PDFium DEPS makes `third_party/simdutf` conditional on
`checkout_v8 = checkout_configuration != "minimal"`. GN still loads
`//third_party/simdutf` while generating the reviewed complete static library,
so a clean runner failed. The local source workspace retained that dependency
from an older broader checkout and had masked the defect.

The builder now uses PDFium's documented `small` checkout: it omits corpora and
instrumented libraries but includes dependencies required to generate PDFium.
The production GN arguments remain unchanged, including `pdf_enable_v8=false`.
The fail-late workflow was also narrowed so Cargo-dependent CLI, daemon and
benchmark checks continue after test failures only when the PDFium pack
verified successfully; prerequisite failure no longer creates three misleading
secondary failures. License, runbook, handoff and workflow checks remain
independently fail-late.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F07-01 | Clean PDFium source checkout includes complete-static-library generation dependencies | `dbcc01dc52d3045ace02df95b9db560c18e6fb8514d14c3bb0cb5da1c55f59dd:bf949f46603081e81aa76a57e2b808eaec12af34a30d59ccdbd7ce72796f6835` | exact Node regression failed before the export/fix, then 2 cases passed in 40 ms | macOS PDFium builder, checkout mode, pinned source DEPS or regression test changes |
| F07-02 | Fail-late checks distinguish a failed prerequisite from business-test failures | `3c8d712427a6171d51de77fad4409e698ecd9e81f23ebe960692d4823174f26a:a43391b545d7b81185dc3f2d304cd5bb60574125519904e3cec0879598367e22` | workflow checker passed | PR step dependencies or workflow policy changes |

The CLI import, daemon seed and benchmark smoke failures in run `30202411939`
all occurred after `libpdfium.a` failed to build. They are classified as
`invalidated_by_prerequisite`, not product failures and not reusable evidence.
License, runbook, handoff and workflow checks passed and remain reusable.

### public synthetic PDF fixture repair F08 — 2026-07-26

PR run `30202721429`, job `89795371823`, successfully built and verified the
reviewed PDFium archive, completed Clippy and reached the workspace tests. The
only failed assertion was
`frozen_public_synthetic_fixture_matches_production_admission`: observed
classification counts `(2, 2, 3, 1, 1)` differed from expected
`(3, 3, 1, 1, 1)`. Every earlier workspace test in the job passed, and the
fail-late CLI, daemon, license, runbook, handoff, workflow and benchmark checks
all passed.

Both misclassified samples were the fixture's synthetic text-layer PDFs. Its
builder emitted `T*` for multiple lines without defining text leading, placing
all lines at the same visual location under PDFium. The fixture now defines
14-point leading before those operations, matching the visible multiline text
the frozen sample claims. Production PDF quality and classification logic are
unchanged.

The workflow also replaces the monolithic cache action with explicit restore
and save actions. A verified cache miss is saved immediately before tests, so a
later business assertion cannot discard a successful 14-minute PDFium build.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F08-01 | Frozen public synthetic PDF samples expose distinct visible lines to production PDFium admission | `2af924eabcb91900cb06df1e1bd6f72bdd2c98faee312d0ef6e861461d54f2ca` | passed in macOS PR run `30203505154`; the repaired public synthetic admission case no longer appears among workspace failures | public synthetic admission fixture, PDF text extraction, quality or classifier changes |
| F08-02 | A verified PDFium pack is cached before later test failures | `8a2924957941fd4de7a0bd9c3f2f456539a20f1a63e48fd8abc8d0b8b84600e7:297571b242256bd081325303a241f13761adce53e2f046a15f15afbccdddad22` | workflow checker passed | PR cache ordering, key, verification or workflow policy changes |

A focused local F08-01 attempt did not reach the test: the compiler spent more
than three minutes blocked enumerating the oversized local
`target/debug/deps` directory, and a direct filesystem enumeration blocked at
the same syscall. Both exact processes were terminated. This is `not_run`; it
does not replace the deterministic hosted red result or the required hosted
green result.

### current-schema and migration recovery repair F09 — 2026-07-26

macOS PR run `30203505154`, job `89797466189`, restored and verified the cached
PDFium pack, passed Clippy, passed the repaired F08 public fixture and completed
every fail-late CLI, daemon, license, runbook, handoff, workflow and benchmark
check. Workspace tests reached 141 meta-store library cases and isolated eight
failures after 133 passes.

The failures had three shared causes:

- current-store recovery validated a ready v33 target through the predecessor
  validator, which accepts only v29–v32 source authorities;
- historical v29 publication fixtures requested a production sibling
  connection, which correctly migrated their authority to v33 before the
  historical assertion ran;
- two current-store tests still asserted v29, and one configured-rescan test
  still expected a terminal retryable head to be reused instead of creating a
  new queued attempt.

The repair dispatches receipt validation by manifest role, gives synthetic
historical fixtures a test-only consuming publication-session seam that cannot
enter production, and updates current-schema and manual-rescan assertions
without relaxing migration, integrity or publication validation. The current
delivery contract also removes the obsolete Windows private-corpus transfer
section; the checker now rejects its reintroduction. macOS remains the only
active delivery platform.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F09-01 | A ready v33 COW receipt validates and atomically publishes its already-verified target | `d771454b6be5dad98b7340b93f8f8b4cdb789bc317f54d3e8f3e43c2ad27d69a` | passed in macOS PR run `30204531887` | current-store receipt reconciliation, manifest dispatch or current-store validation changes |
| F09-02 | Historical v29 publication fixtures remain v29 without entering the production migration boundary | `df187302b5501308b89db3fbdf60ae75b8f628e355c22a99e66b632320b3a44a:b217941a8a313f0bb68fc763b875ae58ee70b6f4ae7a4ca14a7cc71f92d25c0b` | four affected assertions passed in macOS PR run `30204531887`; the complete meta-store lib result was 141 passed | historical fixture seam, v29 publication validation or migration-test support changes |
| F09-03 | Fresh owner/read paths assert the schema selected by the current product, not the retired v29 hard cut | `dd188f730183e9c8cdc7fef1719306fb89f61e78ca6338828e21f387ddf775a1` | two affected assertions passed in macOS PR run `30204531887` | current schema, owner open or read-only current-store contract changes |
| F09-04 | An explicit rescan after a failed retryable task creates a new queued attempt and cancels the terminal head | `0778537f42156afad582d44768a6809d60eea2ffe0cac039b0429d8d35919ced` | passed in macOS PR run `30204531887` | configured rescan, task-head retention or retry semantics change |
| F09-05 | Active goal cannot re-enable Windows/Linux execution through the retired private-corpus transfer policy | `e5d57102184b33586cc83487779292955e28693137fbddf41ce86c86706d48ea:ea873c97d3e92ec546081cc6c3b3323b1690fc3697ca129c7a6842c31bdfc879:050ceb75b004160133b3648533c6387c8442d077a2346cdf65248149f956656d:c477bb451c00357a8adf73df638289202fd7e05467ce4f2923128b67f7eb47bc` | autonomous-goal, loop-state and performance checkers plus seven governance mutation tests passed | active platform contract, pinned synthetic fixtures or autonomous-goal checker changes |

The local exact F09-01 binary compiled, then remained suspended in macOS
`_dyld_start` before entering the Rust test harness. A read-only process sample
confirmed zero test-body execution; the exact cargo parent and orphaned child
were terminated orderly. This is `not_run`, not a failed assertion. The
existing macOS PR job is the authoritative red-capable feedback loop and will
reuse the saved PDFium pack. No unrelated local Rust test was run.

### final stale schema assertion repair F10 — 2026-07-26

macOS PR run `30204531887` proved every F09 repair: the meta-store library
reported 141 passed and no failures. It also completed Clippy plus every
fail-late CLI, daemon, license, runbook, handoff, workflow and benchmark check.
The only remaining workspace failure was the separate
`excluded_document_status` integration target: its first test called
`EphemeralMetaStore::run_migrations()`, which now deliberately initializes
current schema v33, while the old test name and final assertion still expected
v29. Its excluded-status round-trip and non-deletion assertions had already
passed, and the target's publication test passed.

The repair renames that one case to current-schema semantics and compares with
`CURRENT_SCHEMA_VERSION`. No production code or behavior changed.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F10-01 | Excluded status round-trips without deletion in the schema initialized by the current ephemeral migration API | `51d549db58f437176fc7cd9fe23d27e23ffc59c6cf0ebbdc6d280a29dd41ccf0` | passed with the target's second case in macOS PR run `30204933297`; 2 passed | excluded document status, ephemeral migration target or current schema changes |

The local exact integration binary again remained suspended in `_dyld_start`
after compilation and before the Rust test harness. A process sample confirmed
the same host loader failure and the exact processes were terminated. It is
`not_run`; the next macOS PR run is the assertion authority. Every unrelated
pass from run `30204531887` remains reusable.

### source-root authority and remaining current-schema repair F11 — 2026-07-26

macOS PR run `30204933297` passed F10 and again passed the complete meta-store
library. The next integration target then isolated one failure in
`s26_import_root_control`: its periodic-requeue assertion expected root B, but
the old fixture had created only legacy import task/scope rows. Current
production requeue correctly requires a matching active `source_root`, so it
returned no roots. The repair registers root A and root B through the current
authority before exercising pause, requeue, claim and resume semantics; the
production authority query remains strict.

Because Cargo stopped after s26, later integration targets were unrun. A bounded
static audit of those remaining meta-store tests found five more current API
assertions in `s3_sqlite` and `s807_v27` that still hard-coded schema 29. The
historical v29 migration fixtures remain unchanged. Current ephemeral/owner/read
tests now use `CURRENT_SCHEMA_VERSION`, the migration-history assertion covers
the complete contiguous range, and the current schema table inventory includes
the v30–v33 authority, deletion and PDF reprocessing tables.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F11-01 | A paused current source root is excluded while another active root remains eligible for periodic requeue and worker claim | `02dd09a868c3ad6c1ffaa2df431c7c259941706c0b4c0ee8c76815d25d6e0aee` | passed in macOS PR run `30205337013`; 2 target cases passed | source-root authority, pause/resume, periodic requeue or worker-claim filters change |
| F11-02 | Current ephemeral, owner and read APIs expose the contiguous current schema and complete v30–v33 table set | `95d7618e796c378621cbd7e942130bee289fbc91c97955b8ada7740401394c64` | the affected current-schema assertions passed in macOS PR run `30205337013`; the s3 target reached 56 passes before two independent OCR fixture failures | current schema, migration registry or current table inventory changes |
| F11-03 | Current owned-store identity and derived rows remain insert-once without claiming historical v29 | `e40d84f2786155bf47d26b5e23c8206a0504e77f5ec97912ee3182c340aaf677` | passed in macOS PR run `30206076466`; all 38 s807_v27 cases passed | current schema or immutable identity/derived-row semantics change |

All independent and fail-late checks in runs `30204933297` and `30205337013`
passed. F09 and F10 remain reusable and are not reopened by these test-only
fixture changes.

### OCR cache source-authority fixture repair F12 — 2026-07-26

macOS PR run `30205337013` proved F11-01 and the current-schema portion of
F11-02. The s3 target ran 58 cases and isolated two failures after 56 passes:
both OCR cache persistence fixtures used arbitrary content hashes with no
current source authority.

Production intentionally accepts an OCR page-cache write only while the
content hash is referenced by a non-deleted document, current source revision
and present occurrence under a non-deleting source root. This prevents deleted
or unauthorized source content from retaining application-derived OCR data.
The repair preserves that guard and seeds the minimum real authority graph in
the two persistence tests. It does not weaken cache deletion, root deletion or
privacy behavior.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F12-01 | OCR success, retryable failure and word-box cache rows persist only for an actively referenced source revision while debug output remains redacted | `044645f9b4e14ff7f903e5a195636756dda90903bc86d48ba793383486454438` | passed in macOS PR run `30206076466`; all 58 s3 cases passed | OCR cache authority guard, scan/occurrence lifetime, cache payload persistence or debug redaction changes |

The first fixture repair created the root, document, revision and occurrence
but omitted the occurrence's referenced scan snapshot. Both cases failed at
the same public `observe_source_occurrence` call before any cache assertion.
The completed fixture now creates that snapshot through `begin_scan`; it does
not bypass or weaken the foreign key.

No local Rust test body was started because the already-confirmed macOS loader
condition would make another launch non-authoritative. Only the affected s3
target was compiled and linted. The next macOS PR run continues from the
previous fail point; valid F09–F11 results and all independent checks remain
reused.

### deleted-data OCR authority and workspace fail-late repair F13 — 2026-07-26

macOS PR run `30206076466` proved F12, F11-03 and every intervening target,
then isolated one failure in the CLI deleted-data purge target. The test
constructed an OCR cache entry for an imported document but did not prove that
the document had a current root/scan/occurrence authority. The guarded cache
upsert therefore remained a no-op and the later purge correctly reported zero
cache rows instead of the fixture's expected one.

The fixture now establishes the source authority through public APIs before
the cache write and immediately reads the cache row back. The purge assertion
therefore measures real retained derived data instead of assuming an
unverified insert. Production cache and purge behavior remain unchanged.

The run also exposed that `cargo test --workspace` stops after the first failed
test binary. That contradicted the train's explicit fail-late requirement even
though independent workflow steps continued. The macOS PR test command now
uses Cargo's native `--no-fail-fast`, and the workflow checker pins this
behavior so future failures cannot silently hide later workspace targets.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F13-01 | Deleted-data purge removes a proven OCR cache row and word-box payload without exposing source paths or OCR text | `4922574b6d3a566645ad37dceb4c617f5f0697c77c38d4da563d35fd3fa7e134` | focused local target compilation entered the known zero-CPU target-directory stall and was terminated as `not_run`; exact assertion pending the next macOS PR run | deleted-data purge, OCR authority/cache retention, residual scan or redaction changes |
| F13-02 | A macOS workspace test failure does not prevent later workspace test binaries from running | `591ca7951cbd882cb2c466ef38bbc374c9f2be5e50114146b1faf4653ad4e7f7:24cbbb903150585f3d70b2733724cffa6a5648d43bd760eb4e2f62a2da1efff9` | focused workflow checker passed; execution behavior pending the next macOS PR run | PR workflow test command or workflow checker changes |

All independent checks in run `30206076466` passed. Earlier meta-store,
parser, index, search, OCR client and CLI pass results remain valid and are not
reopened by this CLI fixture plus workflow-policy repair.

### complete fail-late workspace repair F14 — 2026-07-26

macOS PR run `30206651549` proved F13 and the workflow's new fail-late
behavior: after the first failed target, every remaining workspace test target
and every independent post-test check still ran. The run collected the complete
remaining set of 21 failures across seven test targets instead of stopping at
the first one.

The failures reduced to four shared contract boundaries:

- OCR worker fixtures had documents and revisions but no present occurrence
  under the current source-root/scan authority, so the production cache guard
  correctly rejected writes;
- PDF and release-readiness fixtures still described the retired `lopdf` /
  Poppler path or emitted multiline PDF text without leading, while explicit
  rescans still expected terminal retryable task reuse;
- daemon worker gates did not freeze PDF import for every runtime combination
  already declared unavailable by the capability matrix, and two tests
  conflated missing, invalid and not-configured runtime evidence;
- the v28 rejection and v29 preservation tests constructed historical
  authorities through current APIs, so their own observation/setup could
  invoke v33 behavior before the daemon under test.

The repair keeps the production source-authority, runtime and migration
validation strict. It adds only one production change: worker gating now uses
the same embedding/classifier/PDFium conjunction as the published PDF-import
capability. Historical setup uses an explicitly feature-gated synthetic test
seam and the v29 acceptance now proves the real `migrating` transition followed
by exact business-head, epoch and artifact-digest preservation at v33.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F14-01 | OCR work and cache reuse require a present occurrence under current root/scan authority | `5d9d932300e3b2efb5a7c7683409e3bfa9ab9856d1cf3bff68e8ea580d7e2450` | seven previously failed exact cases passed locally; one direct warm-up plus Nextest run `b150028a-1c14-4e9a-a603-47b810b3853c` (6/6) | OCR cache authority, source occurrence, worker claim, renderer or OCR publication changes |
| F14-02 | PDFium metrics/text layout, release evidence and explicit-rescan task replacement match current contracts | `f3286deb070a04eb6d9f19138ae72702961b2104ad8b8659a45392da15152b6c` | seven previously failed exact cases passed locally: s21 2/2, Nextest `508dcfc1-6b68-445b-b6f4-abde514c7b48` 2/2 and `aa01fa02-7098-4e1f-ba04-aec16f88ae55` 3/3 | PDF extraction/metrics, release-readiness evidence, task-head coordination or candidate assignment changes |
| F14-03 | Runtime capability publication and worker claims use one embedding/classifier/OCR/PDFium gate; completed scans have current root authority | `8f823f903ea3a9bf669421624e553502fed6bf0c3650b6045a04eab315a386b8` | four previously failed exact daemon unit cases passed locally | runtime attestation/reason vocabulary, capability derivation, worker gate or periodic rescan changes |
| F14-04 | v28 remains byte-preserving unsupported while exact v29 migrates through the explicit COW boundary and preserves business state | `78b992b26a9c5b8e6a3476de9be8ba053ffe0f2b168a3b2c625fe18844460602` | two exact v28 rejection cases and one exact v29→v33 preservation case passed locally | migration-test support, v28 rejection, COW registry, core migration state or preservation summary changes |

The first multi-binary Nextest attempts were cancelled before any test body
after macOS held several new binaries in `_dyld_start` while `syspolicyd`
validated them. Serial first launch followed by same-binary parallel execution
completed successfully. These cancelled enumerations are `not_run`, not failed
tests. Focused production/test-seam Clippy, loop-state, performance-contract,
autonomous-goal and public-repository checks all passed. No previously valid
workspace target was replayed.

### direct CLI managed-root scan coordination repair F15 — 2026-07-26

macOS PR run `30210820223` proved every F14 case except one previously valid
s15 idempotency case. The complete fail-late run then passed all later workspace
targets plus CLI/daemon closed loops, licenses, runbooks, handoff, workflow and
benchmark smoke. The sole failure was a second direct CLI import after the test
had established current source-root authority: the CLI still created only the
legacy task/scope head, so occurrence persistence had no matching task-keyed
`ScanSnapshot` and failed closed.

The repair gives direct CLI imports of managed roots the same atomic
task/snapshot coordination contract used by daemon scans. Scan success and
failure completion moved from daemon-private code into a small shared
import-pipeline module, so both execution owners update progress, missing-file
truth, PDF reprocessing and terminal scan state identically. Legacy-only direct
imports remain batched exactly as before; one invocation cannot mix managed
and unmanaged roots.

| Row | Behavior boundary | Input fingerprint | Status | Re-run only when |
| --- | --- | --- | --- | --- |
| F15-01 | A second direct import of a managed source root creates a matching task-keyed scan, preserves one OCR job and completes the scan | `6676ac93976590aa1a8fa44723bf4966299f967237c96397377af8bc80d9f67d` | the sole failed exact case passed locally; focused Clippy for import-pipeline, resume-cli and resume-daemon passed | direct import coordination, source scan completion, occurrence persistence or OCR job identity changes |

No previously valid workspace target was replayed. The next PR run needs only
to validate this affected boundary and the repository's required macOS gates.

### exact-main native build timeout repair F16 — 2026-07-27

Required macOS and security run `30211712917` passed completely, and PR #238
merged as exact main commit
`b18d5ceaee684b43c59841a2bf0c94a8653ebc42`. The receipt-bound worktree build
then produced DMG digest
`b00a98bd2d13ac01d32be46db5481bb15549381643a8d28d92a5eb75e5328f43`
from source-tree digest
`b98391bb375e697a9d19db1cdea2ca17db2ce2072fd2c006548a3d84384dcde1`,
and installed version 0.1.8 without removing user data.

Installed-main acceptance reached the isolated exact-main release build but
reported `release_build_failed`. A focused reproduction retained the child
result: the child emitted a complete valid
`resume-ir.macos-dmg-composition.v4` receipt, then the parent terminated it at
the shared 20-minute timeout before clean process exit. The App build,
composition, source identity, signature and DMG verification had all
succeeded. The defect was the acceptance harness assigning one timeout budget
to both source/dependency preparation and a full cold Tauri release build.

The release build now owns a separate bounded 40-minute budget while clone and
dependency preparation retain 20 minutes. This preserves process bounds
without turning normal cold-build variance into a false product failure.

| Row | Behavior boundary | Status | Re-run only when |
| --- | --- | --- | --- |
| F16-01 | A full exact-main release build has an independent bounded budget greater than clone/dependency preparation | focused `release-deployment.node-test.mjs`: 12/12 passed | installed-main release deployment, bounded-process timeout or immutable build staging changes |

No previously valid product, workspace or PR test was replayed. Native
installed-main acceptance remains incomplete and must resume from this failed
release-build boundary after the repair reaches main.

### bounded-process timeout ceiling repair F17 — 2026-07-27

The first native retry after F16 failed before spawning the exact-main build
with `tool_invocation_invalid`. The release deployment correctly requested
the new 40-minute budget, but `runBoundedTool` still rejected every timeout
above the former 20-minute clone limit. This was the second half of the same
split-timeout contract and no native scenario or user-data mutation ran.

The bounded-process ceiling now admits the exact release-build maximum while
rejecting larger caller values. Individual call sites remain responsible for
their smaller operation-specific timeout, so the repair does not make ordinary
tools unbounded.

| Row | Behavior boundary | Status | Re-run only when |
| --- | --- | --- | --- |
| F17-01 | The process executor accepts the exact release-build maximum and rejects any larger timeout | focused process suite: 14/14 passed | bounded-process validation or release-build timeout changes |
| F17-02 | Release deployment still uses the independent bounded budget | focused release-deployment suite: 12/12 passed | release deployment or timeout routing changes |

No product or workspace suite was replayed. Native installed-main acceptance
remains incomplete and resumes only after F17 reaches main.

### release-promotion timeout ownership repair F18 — 2026-07-27

The next native retry was instrumented only through release preparation.
Source authority, the exclusive acceptance lease, interrupted-run recovery and
the exact-main release build all completed. The default reinstall child then
threw `ReferenceError: CLONE_TIMEOUT_MS is not defined`; no installed product
scenario or authorized-data clone ran.

F16 separated the build budget from clone preparation but removed the old
timeout import while promotion still referenced it. Release promotion now owns
an explicit 20-minute lifecycle timeout, independent of the 40-minute release
build and the clone/dependency budget.

| Row | Behavior boundary | Status | Re-run only when |
| --- | --- | --- | --- |
| F18-01 | Default exact-release reinstall invokes the lifecycle child with its explicit bounded promotion budget | focused release-deployment suite: 13/13 passed | release deployment, lifecycle promotion or timeout routing changes |

No previously valid product, workspace or delivery suite was replayed. Native
installed-main acceptance remains incomplete and resumes only after F18
reaches main.

### release-promotion source authority repair F19 — 2026-07-27

The post-F18 native retry built the exact-main DMG and reached the default
reinstall child, which returned `macOS build source provenance is invalid`.
A focused replay retained the immutable source only for read-only diagnosis:
HEAD, GitHub origin, detached state, clean status, remote main and source-tree
identity all passed immediately after the child failure. The build had already
verified the same identity before and after artifact construction.

The defect was duplicate authority. Promotion discarded the exact source
capability already verified under the acceptance lease, then attempted to
re-derive it through a fresh Git/remote probe after the high-load build.
Install and reinstall now consume a bounded, closed serialized identity from
the owner. DMG receipt, bundle composition and install receipt verification
remain fail-closed against that identity; no retry or compatibility path was
added.

| Row | Behavior boundary | Status | Re-run only when |
| --- | --- | --- | --- |
| F19-01 | Serialized source identity accepts only the exact bounded closed contract | focused source-identity suite passed | source-identity serialization or validation changes |
| F19-02 | Default install and reinstall promotion receive the verified source capability and keep independent lifecycle bounds | focused release-deployment suite passed | release deployment or promotion argument changes |
| F19-03 | Native install/reinstall and worktree install bind verified DMG consumption to the supplied identity | focused install/reinstall/worktree suites passed | install transaction, DMG verification or source binding changes |

The five focused suites passed 39/39. No product, workspace or previously valid
delivery suite was replayed. Native installed-main acceptance remains
incomplete and resumes only after F19 reaches main.

### COW release-receipt authority repair F20 — 2026-07-27

After F19, exact-main promotion and installation completed and native acceptance
reached its first authorized v29 APFS/COW clone. File cloning succeeded, but
the workspace required the historical install receipt captured with the source
data to match the newly installed exact-main App. That cross-authority
comparison was impossible after any release change and surfaced as
`acceptance_internal_failure`.

The source data authority and installed release authority are now separate.
The cloned historical receipt is used only as the compare-and-swap predecessor.
The temporary workspace receives a current receipt derived from the verified
installed composition and DMG digest, while the authorized source receipt and
all source bytes remain unchanged.

| Row | Behavior boundary | Status | Re-run only when |
| --- | --- | --- | --- |
| F20-01 | COW clone preserves the source receipt, atomically binds the temporary HOME to the current installed release, and returns typed clone failure on invalid evidence | focused exact COW test passed | COW workspace construction, install receipt persistence or installed binding changes |

The first focused file run had 10 unaffected passes and one assertion-path
failure after the implementation itself completed. Only that exact test was
rerun after correcting the assertion path, and it passed. No product,
workspace or previously valid delivery suite was replayed. Native
installed-main acceptance resumes only after F20 reaches main.

### encrypted metadata-authority inspection repair F21 — 2026-07-27

The first post-F20 clone failed before daemon launch because the acceptance
harness sent encrypted SQLCipher metadata to the system SQLite CLI. Meta-store
now owns a read-only exact-v29/current logical-authority inspector, and the
packaged daemon exposes only its bounded path-free receipt. Installed
acceptance uses that verified daemon boundary for both sides of migration.

| Row | Behavior boundary | Status | Re-run only when |
| --- | --- | --- | --- |
| F21-01 | Exact encrypted v29 and current metadata authority can be inspected without owner acquisition or writes | exact meta-store v29/current tests passed; direct authorized-source receipt contained 24 closed fields and source bytes were unchanged | metadata manifest/key/integrity, authority descriptor or inspector changes |
| F21-02 | Daemon emits one bounded path-free metadata-authority receipt | exact daemon receipt test passed | daemon internal command or receipt schema changes |
| F21-03 | Installed acceptance uses the verified packaged daemon rather than system SQLite | focused acceptance-evidence Node suite 6/6 passed | installed metadata evidence collection changes |
| F21-04 | Changed production boundaries remain warning-free and public-safe | focused meta-store/daemon Clippy, script syntax, diff and public guard passed | affected Rust/Node/public-boundary code changes |

No workspace, delivery matrix or previously valid installed product cell was
replayed. Native installed-main acceptance resumes only after F21 reaches main.

### installed v29 migration convergence repair F22 — 2026-07-27

The next native run passed build, install, COW and encrypted authority
inspection, then kept the same healthy control plane at
`core=migrating` until the first cold-start cell's 20-minute timeout. A focused
COW launch and native process sample localized the CPU-bound path to the
v32-to-v33 PDF reprocessing backfill: each source occurrence performed an
encrypted full scan of `resume_version`.

The backfill now creates a transaction-local lifecycle index over
`(source_revision_id, parse_version)`, performs indexed probes, and drops the
index before commit. The v33 schema/checksum remains unchanged. Exact
preservation hashing also streams ordinary tables in rowid order and
`WITHOUT ROWID` tables in primary-key order instead of sorting by every column.
When cleanup and a product cell both fail, the orchestrator preserves the
product failure rather than masking it.

| Row | Behavior boundary | Status | Re-run only when |
| --- | --- | --- | --- |
| F22-01 | PDF reprocessing backfill uses the bounded lookup and leaves no persistent schema artifact | exact new meta-store regression passed | v32-to-v33 backfill SQL or migration transaction changes |
| F22-02 | Exact source/staging digest is deterministic without a temporary sort | exact new rowid/WITHOUT ROWID backup regression passed | preservation witness or table-order logic changes |
| F22-03 | Existing encrypted v29 COW preservation remains exact | existing exact v29 migration test passed | current-store COW, forward chain or preservation logic changes |
| F22-04 | Cleanup never replaces an earlier product failure | exact Node orchestrator failure-composition test passed | acceptance cleanup/error composition changes |
| F22-05 | Authorized private v29 witness converges to valid current authority while preserving source ciphertext | passed in 26.769 seconds; source schema 29, current authority valid, source ciphertext unchanged | migration chain, SQLCipher backup or authority inspection changes |
| F22-06 | Changed production boundaries remain warning-free | focused meta-store/daemon Clippy, Node syntax and diff checks passed | affected Rust or Node code changes |

The pre-repair focused witness was stopped after 189 seconds while its sampled
stack remained in the quadratic backfill; its COW workspace was removed and
the authorized source hash was unchanged. No delivery matrix or unaffected
product cell was replayed. The next native run resumes at the previously failed
cold-start cell after F22 reaches main.

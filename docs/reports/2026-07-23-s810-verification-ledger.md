# S810 Test-Round Ledger

Status: active; R07, R08, R09 and R10 are closed. Native installed/manual
acceptance and the frozen-code release gates remain separate future rounds.

This is the authoritative record for deciding whether a test may run. It
prevents expensive repetition while keeping an auditable distinction between a
command that actually passed and a command that merely appears earlier in a
script.

## Operating protocol

1. A test round has an explicit, ordered set of cells before any command runs.
2. A cell may be carried forward only when a later change does not affect its
   recorded behavior boundary. Otherwise it becomes `invalidated`, never
   silently remains passed.
3. A failure opens a repair sub-round. The repair sub-round contains the failed
   cell, the smallest regression test that proves the fix, and any behavior
   directly affected by the fix. Unrelated passed cells remain carried.
4. A new round cannot open while the current round has an `in_progress`,
   `failed`, `unknown`, or unclassified `pending` cell. `not_run` cells outside
   the declared round are not silently imported into it.
5. `verify-local.sh` is an ordered release pipeline, not a development-time
   restart button. During active development its cells are executed and
   recorded individually. The whole script runs once only at a frozen-code
   gate or with an explicit decision to pay that cost.
6. “Passed” means an observed zero exit on the recorded behavior boundary.
   Script order alone is never evidence. A stopped `set -e` script marks all
   later cells `not_run`.
7. Beginning with R02, local execution truth is written by
   `./scripts/ci/verify-local.sh --parallel` under the ignored
   `.cache/verify-local-parallel/` directory. `state.json` carries only the
   latest reusable result per input fingerprint; each `runs/*.json` is an
   immutable round receipt with per-cell command, behavior, resource class,
   fingerprint, duration, wall time, and measured `speedup_vs_serial`. This
   Markdown report summarizes that local ledger
   and never invents historical results it cannot observe.

## State vocabulary

| Field | Values | Meaning |
| --- | --- | --- |
| Execution | `passed`, `failed`, `not_run`, `interrupted` | What actually happened in that invocation. |
| Reuse | `valid`, `invalidated`, `pending`, `not_applicable` | Whether the result may satisfy the next decision. |
| Round | `closed`, `repairing`, `blocked` | A round closes only when every declared cell is terminal and its required passing cells passed. |

## Evidence anchors

- Base commit: `30868881838b8785fb7f1ccf574cb8bb34c51d00`.
- Current worktree: dirty S810 hard-cut implementation; individual rows carry
  their own behavior boundary instead of treating the whole tree as one test
  surface.
- Historical P01 pipeline source: `scripts/ci/verify-local.sh`, SHA-256
  `20cab646663c38b6326e236200485bd7c6b49e73a3ed0efc35d748db151f8997`.
- Current parallel wrapper: `scripts/ci/verify-local.sh`, SHA-256
  `7c5882ec972559c7ee02269b31452046a1ccb300658fe601cb2cc5ecccd76518`.
- Current parallel runner + manifest: SHA-256
  `b6fc3a0a462d6198c181f9755842fb88fa48e792dccf3b99b2929aabb5561092`
  and `a9b348e396723bfe3799943d14df91028ea59a16496da2eb91d1c33884d06d53`.
- Current runner support module: SHA-256
  `7265aafaf1998d93b311c36370f81fd479b1ee296acbe4c939b17ce36827fcee`.
- Current runner self-test: SHA-256
  `bb59065da57e89f9a416be43045487b612002a8f5c1118f4189f75a74496ccc3`.

## Historical morning sequence — unreconciled

The original morning batch ran for more than ten hours and its first failure
was substantially later than P01-06. Its raw ordered transcript is not present
in this report after context compression, so this report must not claim an
exact first-failure cell or label its later cells `not_run`. The later P01
attempt below is **not** a replacement for that historical progress.

Until a raw terminal/log record is recovered, historical results are
`unknown`, not “unrun,” and cannot authorize a repeat by themselves. The new
local runner begins a fresh, durable ledger for all future rounds instead of
guessing at the morning sequence.

## Pipeline attempt P01 — later redundant `verify-local` attempt

This was the later repeat that should not have been used to restart the test
train. `verify-local.sh` uses `set -eu`; this particular invocation stopped at
P01-06, where
`s48_search_ipc::corrupted_published_generation_is_rebuilt_before_search`
failed. P01-01 through P01-05 completed in this later invocation, and P01-07
onward did not run **in this invocation only**. These rows do not describe the
morning batch. The later import IPC finding was discovered by a separate direct
execution of P01-11; it likewise does not prove P01-07 through P01-10 ran.

| Cell | Ordered command | Execution | Current reuse | Evidence / next rule |
| --- | --- | --- | --- | --- |
| P01-01 | `cargo metadata --no-deps --locked` | passed | valid | Carried into R01; no Cargo metadata change in R01. |
| P01-02 | `python3 scripts/ci/test-governance-contract-mutations.py` | passed | valid | Carried for the import repair; re-evaluate only for governance-contract changes. |
| P01-03 | `python3 scripts/ci/check-search-runtime-boundary.py --cargo "$CARGO_BIN"` | passed | valid | Carried for the import repair; search-runtime boundary was not edited by R01. |
| P01-04 | `cargo fmt --check` | passed | invalidated | R01 added/changed Rust sources. Targeted Rust formatting passed, but this full-workspace cell is not claimed green. |
| P01-05 | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | passed | invalidated | R01 changed CLI Rust code; a future frozen-code gate must run the affected Clippy scope. |
| P01-06 | `cargo test --workspace --exclude benchmark-runner --exclude embedder --exclude resume-cli --locked` | failed | pending | Failure was repaired by exact S48 coverage; the aggregate command itself has not been restarted and is not claimed passed. |
| P01-07 | `cargo test -p resume-cli --locked -- --test-threads=1` | not_run | pending | Not reached in P01; focused R01 CLI tests do not substitute for this full crate cell. |
| P01-08 | `cargo test -p embedder --locked -- --test-threads=1` | not_run | pending | Not reached in P01. |
| P01-09 | `cargo test -p benchmark-runner --locked -- --test-threads=1` | not_run | pending | Not reached in P01. |
| P01-10 | `./scripts/ci/check-cli-closed-loop.sh` | not_run | pending | Not reached in P01. |
| P01-11 | `./scripts/ci/check-daemon-closed-loop.sh` | passed separately in R01 | valid | The standalone R01 result proves its revised behavior; P01 itself did not reach this cell. |
| P01-12 | `./scripts/ci/check-daemon-incremental-import.sh` | not_run | pending | Not reached in P01. |
| P01-13 | `./scripts/ci/check-benchmark-smoke.sh` | not_run | pending | Not reached in P01. |
| P01-14 | `./scripts/ci/check-licenses.sh` | not_run | pending | Not reached in P01. |
| P01-15 | `./scripts/ci/check-runtime-bundle-policy.sh` | not_run | pending | Not reached in P01. |
| P01-16 | `./scripts/ci/check-runbooks.sh` | not_run | pending | Not reached in P01. |
| P01-17 | `./scripts/ci/check-current-stage-observability.sh` | not_run | pending | Not reached in P01. |
| P01-18 | `./scripts/ci/check-local-embedding-runtime.sh` | not_run | pending | Not reached in P01. |
| P01-19 | `./scripts/ci/check-local-ocr-runtime.sh` | not_run | pending | Not reached in P01. |
| P01-20 | `./scripts/ci/check-local-diagnostics-release-evidence.sh` | not_run | pending | Not reached in P01. |
| P01-21 | `./scripts/ci/check-local-quality-release-evidence.sh` | not_run | pending | Not reached in P01. |
| P01-22 | `./scripts/ci/check-current-stage-validation.sh` | not_run | pending | Not reached in P01. |
| P01-23 | `./scripts/ci/check-current-stage-handoff.sh` | not_run | pending | Not reached in P01. |
| P01-24 | `./scripts/ci/check-workflows.sh` | passed separately in R01 | valid | Revised daemon closed-loop contract was checked directly. |
| P01-25 | `./scripts/ci/check-release-readiness.sh` | not_run | pending | Not reached in P01. |
| P01-26 | `./scripts/ci/check-release-artifacts.sh` | not_run | pending | Not reached in P01. |
| P01-27 | `./scripts/ci/check-runtime-bundle-manifest.sh` | not_run | pending | Not reached in P01. |
| P01-28 | `./scripts/ci/check-runtime-bundle-payload.sh` | not_run | pending | Not reached in P01. |
| P01-29 | `./scripts/ci/check-release-publication-evidence.sh` | not_run | pending | Not reached in P01. |
| P01-30 | `./scripts/ci/check-signing-evidence.sh` | not_run | pending | Not reached in P01. |
| P01-31 | `./scripts/ci/check-notarization-evidence.sh` | not_run | pending | Not reached in P01. |
| P01-32 | `./scripts/ci/check-release-sbom.sh` | not_run | pending | Not reached in P01. |
| P01-33 | `./scripts/ci/check-runtime-bundle-sbom.sh` | not_run | pending | Not reached in P01. |
| P01-34 | `./scripts/ci/check-runtime-bundle-package.sh` | not_run | pending | Not reached in P01. |
| P01-35 | `./scripts/ci/check-macos-package.sh` | not_run | pending | Not reached in P01. |
| P01-36 | `./scripts/ci/check-macos-installer-evidence.sh` | not_run | pending | Not reached in P01. |
| P01-37 | `./scripts/ci/check-windows-package.sh` | not_run | pending | Not reached in P01. |
| P01-38 | `./scripts/ci/check-windows-installer-evidence.sh` | not_run | pending | Not reached in P01. |
| P01-39 | `./scripts/ci/check-windows-service-evidence.sh` | not_run | pending | Not reached in P01. |
| P01-40 | `./scripts/ci/guard-public-repo.sh` | not_run | pending | Not reached in P01. |

P01 is not closed. Its outstanding aggregate and release cells are not made due
by the focused import repair, and they must not be restarted merely because the
ledger exists.

## Focused repair round R01 — import IPC error projection

Trigger: the standalone P01-11 probe failed at `import ipc`. The daemon
returned a valid hard-cut capability rejection, but `resume-cli` collapsed it
to the generic `daemon import ipc returned an error` message. The repair only
touched the CLI import-error boundary and the stale closed-loop/guard contract.

| Cell | Required behavior | First result | Final result | Reuse | Invalidation boundary |
| --- | --- | --- | --- | --- | --- |
| R01-01 | `s47_import_ipc::import_ipc_reports_a_typed_unavailable_capability_without_leaking_context` | failed (RED) | passed | valid | CLI import error projection or its redaction contract. |
| R01-02 | `daemon_ipc_contract::tests::import_service_error_v2_accepts_only_exact_import_context` via `cargo test -p resume-cli --bin resume-cli ... --exact` | new | passed | valid | v2 parser, HTTP status parsing, or allowed import error combinations. |
| R01-03 | `s20_ipc::daemon_authenticates_and_queues_import_command_over_ipc` | carried evidence | passed | valid | Daemon import route or strict runtime attestation. |
| R01-04 | `./scripts/ci/check-daemon-closed-loop.sh` | failed at import, then delete premise | passed | valid | Capability gating, core IPC, or closed-loop script. |
| R01-05 | `./scripts/ci/check-workflows.sh` | new | passed | valid | CI script wiring or closed-loop contract guard. |
| R01-06 | Targeted `rustfmt --check`, `sh -n`, and `git diff --check` | new | passed | valid | Corresponding edited Rust, shell, or ledger files. |

R01 is **closed**. Its product-behavior evidence remains carried subject to
its stated invalidation boundaries. The old R01-05 workflow-script result is
superseded by R02 because the verification entrypoint itself changed.

## Test-tooling repair round R02 — resumable parallel verification

Trigger: the prior Markdown-only accounting could not reliably distinguish a
historical pass from a later rerun, and the serial release script could not
reuse an unchanged result. R02 adds a local-only manifest, input fingerprints,
resource locks, immutable round receipts, and an explicit parallel entrypoint.
It does not claim any product gate passed.

| Cell | Required behavior | Final result | Reuse / next rule |
| --- | --- | --- | --- |
| R02-01 | Runner self-test proves independent checks overlap, Cargo resources remain exclusive, passed inputs reuse, changed inputs rerun, failures retry, and dry-run never executes. | passed | Re-run when runner, support module, manifest, or self-test changes. |
| R02-02 | Python modules compile and the POSIX wrapper parses. | passed | Re-run when the corresponding Python or shell tooling changes. |
| R02-03 | Workflow guard requires the parallel runner, support module, manifest, self-test, and serial-gate self-test hook. | passed | Re-run when workflow guard or verification wiring changes. |

R02 is **closed**. No S810 product or installer test was opened by R02.

## Current product round R03 — user-authorized unfinished cells

Trigger: the user explicitly authorized continuation of the unfinished checks,
with the standing rule that prior valid evidence must not be repeated merely to
reconstruct the old serial sequence. R03 is a new current-source round: each
cell below will get a fingerprinted local receipt and is independent of the
unrecoverable historical morning transcript.

Execution uses the local parallel scheduler with ten worker slots and bounded
Cargo/runtime/packaging resources. Completion order is therefore a resource
scheduler result, not a claim about the original serial script order. A failure
does not cancel unrelated cells; it opens a focused repair sub-round and only
the affected cell(s) are rerun afterward.

Excluded by explicit reuse or deferral: P01-01 through P01-05, R01-04,
R02-01 through R02-03, P01-24, and the broad P01-06
`cargo-test-workspace-core` aggregate. The aggregate remains a
frozen-code/release gate and is not implied by R03.

| Cell | Parallel manifest ID | Status before execution | Result | Reuse / next rule |
| --- | --- | --- | --- | --- |
| R03-01 | `cargo-test-resume-cli` | declared | passed (1667.8s) | Valid at receipt fingerprint; rerun only if its inputs change. |
| R03-02 | `cargo-test-embedder` | declared | failed (340.7s) | R04 exact diagnosis and affected regression only. |
| R03-03 | `cargo-test-benchmark-runner` | declared | passed (553.7s) | Valid at receipt fingerprint; rerun only if its inputs change. |
| R03-04 | `cli-closed-loop` | declared | passed (73.2s) | Valid at receipt fingerprint; rerun only if CLI-loop inputs change. |
| R03-05 | `daemon-incremental-import` | declared | failed (102.1s) | R04 exact diagnosis and affected regression only. |
| R03-06 | `benchmark-smoke` | declared | failed (7.4s) | R04 exact diagnosis and affected regression only. |
| R03-07 | `licenses` | declared | passed (0.2s) | Valid at receipt fingerprint; rerun only if license inputs change. |
| R03-08 | `runtime-bundle-policy` | declared | passed (0.2s) | Valid at receipt fingerprint; rerun only if its policy inputs change. |
| R03-09 | `runbooks` | declared | passed (0.9s) | Valid at receipt fingerprint; rerun only if runbook inputs change. |
| R03-10 | `current-stage-observability` | declared | passed (0.4s) | Valid at receipt fingerprint; rerun only if observability inputs change. |
| R03-11 | `local-embedding-runtime` | declared | passed (1.4s) | Valid at receipt fingerprint; rerun only if runtime inputs change. |
| R03-12 | `local-ocr-runtime` | declared | passed (2.7s) | Valid at receipt fingerprint; rerun only if OCR/runtime inputs change. |
| R03-13 | `local-diagnostics-release-evidence` | declared | passed (4.4s) | Valid at receipt fingerprint; rerun only if diagnostics inputs change. |
| R03-14 | `local-quality-release-evidence` | declared | passed (406.5s) | Valid at receipt fingerprint; rerun only if quality/runtime inputs change. |
| R03-15 | `current-stage-validation` | declared | passed (26.4s) | Valid at receipt fingerprint; rerun only if validation/runtime inputs change. |
| R03-16 | `current-stage-handoff` | declared | passed (1.2s) | Valid at receipt fingerprint; rerun only if handoff inputs change. |
| R03-17 | `release-readiness` | declared | passed (3.5s) | Valid at receipt fingerprint; rerun only if readiness inputs change. |
| R03-18 | `release-artifacts` | declared | passed (0.4s) | Valid at receipt fingerprint; rerun only if artifact inputs change. |
| R03-19 | `runtime-bundle-manifest` | declared | passed (0.5s) | Valid at receipt fingerprint; rerun only if manifest inputs change. |
| R03-20 | `runtime-bundle-payload` | declared | passed (0.9s) | Valid at receipt fingerprint; rerun only if payload inputs change. |
| R03-21 | `release-publication-evidence` | declared | passed (1.8s) | Valid at receipt fingerprint; rerun only if publication inputs change. |
| R03-22 | `signing-evidence` | declared | passed (0.4s) | Valid at receipt fingerprint; rerun only if signing-evidence inputs change. |
| R03-23 | `notarization-evidence` | declared | passed (0.3s) | Valid at receipt fingerprint; rerun only if notarization-evidence inputs change. |
| R03-24 | `release-sbom` | declared | passed (1.2s) | Valid at receipt fingerprint; rerun only if release-SBOM inputs change. |
| R03-25 | `runtime-bundle-sbom` | declared | passed (1.2s) | Valid at receipt fingerprint; rerun only if runtime-SBOM inputs change. |
| R03-26 | `runtime-bundle-package` | declared | passed (0.6s) | Valid at receipt fingerprint; rerun only if packaging inputs change. |
| R03-27 | `macos-package` | declared | passed (9.4s) | Valid at receipt fingerprint; rerun only if macOS-package inputs change. |
| R03-28 | `macos-installer-evidence` | declared | passed (0.5s) | Valid at receipt fingerprint; rerun only if installer-evidence inputs change. |
| R03-29 | `windows-package` | declared | passed (0.0s) | Valid at receipt fingerprint; rerun only if Windows-package inputs change. |
| R03-30 | `windows-installer-evidence` | declared | passed (0.3s) | Valid at receipt fingerprint; rerun only if Windows-installer inputs change. |
| R03-31 | `windows-service-evidence` | declared | passed (0.3s) | Valid at receipt fingerprint; rerun only if Windows-service inputs change. |
| R03-32 | `public-repo-guard` | declared | passed (0.1s) | Valid at receipt fingerprint; rerun after a public-input change. |

R03 is **closed** with 29 passed and 3 failed cells. Its local immutable
receipt is `.cache/verify-local-parallel/runs/20260723T135256Z-05595a3d.json`:
wall time 1667.8s, summed work 3210.5s, and measured scheduling speedup 1.92x.
The failures have no reuse value. R04 is limited to exact diagnostic/repair
work for `cargo-test-embedder`, `daemon-incremental-import`, and
`benchmark-smoke`; the 29 passed cells must not be repeated unless their
declared input fingerprint becomes invalid.

## Focused repair round R04 — three failed R03 boundaries

R04 begins from the immutable R03 receipt. The three failures all execute
native child programs and Cargo commands; running them concurrently again
would reproduce the suspected shared macOS execution and Cargo-lock pressure
instead of isolating a product defect. Diagnostics therefore run one exact
failed boundary at a time. This is not a return to serial release verification:
the completed independent R03 cells retain their valid evidence.

| Cell | Exact diagnostic / regression boundary | Status | Rule |
| --- | --- | --- | --- |
| R04-01 | `cargo test -p embedder --locked --test resident -- --test-threads=1` | passed, 5/5 | R03 was a concurrent native-child startup failure, not an embedder regression. Re-run only for resident-worker code or native scheduling changes. |
| R04-02 | Strict S4 watcher regression and `./scripts/ci/check-daemon-incremental-import.sh` | passed | The old script invoked a bare daemon that hard-cut capability gating correctly blocked. It now owns an exact authenticated-runtime integration test. Re-run for incremental import, worker claim, runtime attestation, or script changes. |
| R04-03 | Capability-matrix/S50/S20/desktop exact tests and `./scripts/ci/check-benchmark-smoke.sh` | passed | Classifier unavailability no longer blocks index publication; portable smoke no longer presents a temporary executable as an attested production sidecar. Re-run for capability gates, batch protocol, smoke, or runtime contracts. |

R04 evidence details:

- R04-02 passed the new exact watcher test
  `foreground_import_watcher_requeues_completed_root_after_word_and_pdf_change_without_path_leak`,
  then the wrapper. The strengthened strict-runtime test
  `foreground_once_worker_processes_queued_import_task_from_persistent_scope`
  also passed after asserting atomic publication and searchability.
- R04-03 first failed red in the eight-runtime matrix, then passed after the
  classifier/index gate repair. The focused S50, S20, frontend validator, and
  Tauri validator tests passed; the final smoke passed with no production
  sidecar bypass.

R04 is **closed**. The three R03 failures are now resolved by their exact
affected checks. No unrelated R03 passed cell was rerun.

## Test-tooling repair round R05 — native runtime scheduling

Trigger: R03 showed that three ordinary Cargo slots can still overlap native
child startup and trigger a load-dependent macOS false failure. The manifest
now gives `cargo-test-resume-cli`, `cargo-test-embedder`,
`cargo-test-benchmark-runner`, and `daemon-incremental-import` a shared
`native-runtime` slot while retaining three normal Cargo slots for unrelated
work.

| Cell | Required behavior | Final result | Reuse / next rule |
| --- | --- | --- | --- |
| R05-01 | The runner serializes independent native-runtime checks without serializing unrelated checks or Cargo itself. | passed | Self-test now proves both Cargo and native-runtime exclusion, parallel overlap, reuse, invalidation, and failure retry. Re-run for runner, manifest, or self-test changes. |
| R05-02 | Manifest and workflow wiring accept the new resource class. | passed | `json.tool`, Python compilation, and `check-workflows.sh` passed. Re-run for verification wiring changes. |

R05 is **closed**. This is a scheduling correction only; it does not reopen
any product result.

## Carried focused evidence outside R01

| Cell | Behavior | Latest result | Reuse | Re-run only when |
| --- | --- | --- | --- | --- |
| C01 | Parent shutdown revokes generation before draining data plane | passed | valid | Parent-shutdown lifecycle changes. |
| C02 | Request-limit shutdown stops status updater before drain | passed | valid | Request-limit cleanup changes. |
| C03 | S48 corrupted published generation rebuilds before search | passed after repair | valid | Repair/search lifecycle changes. |
| C04 | S49 detail/hydrate retains exact selection across publications | passed after repair | valid | Detail hydration or request-limit cleanup changes. |

## Command-selection correction

One initial parser invocation omitted `--bin resume-cli`. Its intended unit
test passed, but Cargo then began launching unrelated integration-test binaries
with zero selected tests. It was interrupted and is not a passing checkpoint.
R01-02 is the only accepted parser result.

## Sequential continuation round R06

Trigger: the user explicitly directed the test train to continue at the next
fixed-order unfinished gate, rather than stopping at R05 or changing lanes to
installed acceptance. The current `verify-local.sh` order has an additional
runner self-test at position 3. Positions 1--4 retain valid behavior-specific
evidence; position 5 is the next uncompleted whole-workspace check after S810
source edits.

| Cell | Current script position | Command | Status | Rule |
| --- | --- | --- | --- | --- |
| R06-01 | 5 | `cargo fmt --check` | passed (8.2s) | Proceeded directly to R06-02. |
| R06-02 | 6 | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | passed (18.0s) | Proceeded to R06-03. |
| R06-03 | 7 | `cargo test --workspace --exclude benchmark-runner --exclude embedder --exclude resume-cli --locked` | interrupted_before_receipt | The first detached attempt lost its owner but left no completed receipt; it is re-queued exactly once in R07. |

## Full unattended continuation round R07

Trigger: the user explicitly directed that every remaining automated repository
check run in one keep-going batch, rather than stopping after the first
failure or treating only individual regressions as the next action. R07 starts
at the interrupted current position (7), includes all script positions 8--41,
and adds the deterministic desktop test/build/Tauri/internal-bundle checks.
It does not repeat completed positions 1--6.

The runner is started once with `--no-resume` for this declared suffix so each
R07 cell executes exactly once in this batch. It has no `--fail-fast` flag:
every schedulable cell runs even if another cell fails, and one immutable local
receipt records the full error set for joint triage.

R07 completed from
`.cache/verify-local-parallel/runs/20260723T153949Z-129ab3a5.json` in
5293.2 seconds of wall time. Thirty-eight cells passed and retain their receipt
fingerprints. Two cells failed:

| Cell | Manifest ID | Result | Repair boundary |
| --- | --- | --- | --- |
| R07-24 | `daemon-closed-loop` | failed at initial status IPC | Shared daemon health producer/consumer contract and this exact closed-loop cell only. |
| R07-40 | `desktop-macos-internal-bundle` | failed before build with `release_source_provenance_failed` | Retired exact-main artifact entry; replaced by the worktree-snapshot DMG boundary below. |

The 38 passing cells include the workspace-core, CLI, embedder,
benchmark-runner, incremental-import, desktop frontend/Tauri, packaging,
release-evidence, workflow, public-boundary and remaining repository checks.
They must not be replayed merely because either failed cell needed repair.
R07 is **closed** as a keep-going discovery round.

## Focused repair round R08 — shared daemon health contract

R08 replaces independently maintained status/capability tables with the
`daemon-contract` crate and one shared conformance fixture consumed by the
daemon producer, CLI, Tauri validator and TypeScript validator. The failed R07
closed-loop cell remains pending until its exact wrapper rerun; unrelated R07
cells remain carried.

| Cell | Exact boundary | Result | Re-run only when |
| --- | --- | --- | --- |
| R08-01 | `cargo test -p daemon-contract --locked` | passed, 4/4 | Shared health matrix or validator changes. |
| R08-02 | Exact daemon status-v3 producer fixture test | passed, 1/1 | Daemon status producer changes. |
| R08-03 | Exact CLI and Tauri status/capability validators | passed | Rust consumer projection changes. |
| R08-04 | `npx vitest run src/daemon.test.ts` | passed, 15/15 | TypeScript daemon validator changes. |
| R08-05 | `./scripts/ci/check-daemon-closed-loop.sh` | passed | Daemon bootstrap, shared health projection, or closed-loop script changes. |

R08 is **closed**. The original R07 initial-status failure is repaired without
replaying any other R07 cell.

## Test-tooling round R09 — source-aware reusable evidence

The local runner contract is hard-cut to manifest/ledger v2. Every cell records
its source authority, evidence lane, claim and artifact-production status.
Artifact-producing checks require a non-repository source authority and a
packaging resource. Existing v1 ledger results are carry-forward evidence only;
they do not acquire a stronger source or release claim.

| Cell | Boundary | Result |
| --- | --- | --- |
| R09-01 | `python3 scripts/ci/test-verify-local-parallel.py` | passed |
| R09-02 | Worktree DMG manifest selection/list validation | passed |
| R09-03 | Updated release-blocker runbook guard | passed |

## Focused artifact round R10 — immutable worktree DMG

R10 removes the v1/v2 macOS composition/install readers and the legacy 0.1.1
upgrade executor. Current installation supports only v3-bound first install or
same-version reinstall. A content-addressed COW snapshot binds the artifact to
`{authority, base_commit, source_tree_sha256}` and makes only a manual
`composition_only` claim.

Passed focused coverage includes the source identity/snapshot/wrapper tests,
v3 source bindings and deployment tests, v3 install/reinstall/journal/recovery
tests, exact COW receipt and installed-binding tests, and affected DMG contract
tests. The actual `desktop-macos-worktree-bundle` passed with:

- source authority `worktree_snapshot`;
- base commit `30868881838b8785fb7f1ccf574cb8bb34c51d00`;
- source tree
  `e96277992d5cb701b8b084afe0e73b9e9bd853c1b0d1c60ed3947d1353f3c3f6`;
- DMG SHA-256
  `1bdf2ee42f45a175a0dfd2ecd207a9229a28206a576e9722952ad82568cc57cd`;
- App composition digest
  `6bedc5c2db0f4a3c4cf2d672a5bbdb1e2093f7d3e485773fb65c8931c3890cab`;
- arm64, deep ad-hoc signature valid, hardened runtime enabled, exact
  embedding-only library-validation entitlement scope, zero build-machine path
  markers, and all three runtime packs digest-bound.

R10 is **closed** as `gui_manual` composition evidence. Gatekeeper rejection is
expected for this unnotarized internal-test profile. This result is not native
installed acceptance and does not reopen the 38 R07 passes.

## GUI/manual round R11 — installed worktree smoke

R11 installed the R10 artifact through the documented manual-test lane and
verified the copied App against the exact worktree source identity, composition
digest, reviewed desktop executable, icon and all bundled runtime manifests.
The previous installed App was moved to Trash and remains recoverable.

Two native launches then exercised both storage boundaries:

- The ordinary launch against the existing user data reached the authenticated
  control plane and displayed the typed unsupported-schema blocked state.
  Search authority was revoked and the existing data remained untouched.
- A guarded fresh-data launch atomically held the original data directory
  outside the live path, created a new empty directory, and restored the
  original directory with the same inode after normal App shutdown. The UI
  first showed `process_state=ready` with `core=initializing`, then reached
  Ready with embedding, OCR and classifier available. A synthetic keyword
  query completed against the empty corpus without semantic relaxation, and
  combined diagnostics reported all five redaction boundaries passing.

Normal quit left no desktop/daemon process or current control file behind. The
fresh v29 test directory and previous App are retained in Trash so the manual
operation remains recoverable.

R11 is **closed** as `gui_manual` installed-worktree evidence. It is not the
merged-clean-main v29 COW installed acceptance, signing/notarization evidence,
or the renewed two-hour soak, and it does not reopen any R07 passing cell.

## Focused quality round R12 — post-R07 Rust changes

R12 covers only Rust scopes changed after the R07 receipt:

| Cell | Boundary | Result |
| --- | --- | --- |
| R12-01 | `daemon-contract`, `resume-daemon` and `resume-cli` all-target/all-feature Clippy with warnings denied | passed |
| R12-02 | Desktop Tauri all-target Clippy with warnings denied | failed once on a test-only `CoreReason` re-export, then passed after removing the dead production re-export |
| R12-03 | Exact Rustfmt for the two touched Tauri files and `git diff --check` | passed |

R12 is **closed**. No Rust test, R07 passing cell, workspace-wide Clippy, or
package build was replayed for the import-only cleanup.

## Next-round gate

After the active focused round closes, a new product verification command requires a declared
round. When one is opened:

1. Record the next code change and compare it with the invalidation boundaries
   above.
2. Declare a new ordered round containing only the invalidated cells and new
   regression cells. Preserve every `valid` row without rerunning it.
3. If a declared cell fails, stop the round at that cell, record the bug, and
   open one repair sub-round. Do not restart unrelated earlier cells.
4. Use `./scripts/ci/verify-local.sh --parallel --jobs 10` for a new local
   round. It will reuse only an exact matching fingerprint and will write the
   reason for every executed, reused, failed, or fail-fast-skipped cell. The
   default manifest permits three concurrent ordinary Cargo cells and divides
   the ten configured Cargo build jobs and Rust test threads across them;
   native child/runtime checks additionally share one empirical safety slot,
   while packaging and runtime resources remain separately bounded.
5. A full serial `verify-local` pass remains a frozen-code/release gate. It
   must never be inferred from P01 or used as an automatic response to a small
   bug fix.

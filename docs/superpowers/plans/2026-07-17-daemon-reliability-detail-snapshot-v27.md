# Daemon Reliability And Detail Snapshot Correctness v29 Corrective Plan

The path retains `v27` only for linked issue-train continuity. This v29 plan is
the authoritative execution contract and does not authorize compatibility
readers, aliases or fallbacks.

## Ordered slices

1. **D1 IPC failure boundary**: extract server/connection/response error types,
   make all socket and request failures connection-local, supervise query and
   import services, and add deterministic reset tests.
2. **D2 identity and health**: bind endpoint/auth to one daemon instance, rotate
   credentials per generation, expose bounded status/diagnostics counters, and
   reject stale connection leases.
3. **R1 native supervisor**: replace WebView-driven startup with a single-owner
   Tauri actor, fixed backoff/circuit policy, typed lifecycle commands and
   idempotent tree shutdown.
4. **R2 evidence and UI state**: persist a bounded owner-only lifecycle receipt,
   export diagnostics even when the daemon is unavailable, and model lifecycle,
   service health and result freshness independently in React.
5. **C1-C3 coordinated data hard cut**: introduce immutable v28 storage,
   source revisions, active projection, atomic search publication, exact query
   leases and selection-bound detail/hydrate. These commits share one merge
   boundary so no supported build contains two searchability authorities.
6. **M1 repair migration**: copy only stable identity and authorization from
   v26/v27 into a validated copy-on-write v28 store, invalidate unprovable
   tasks/derived state/artifacts, reconcile exact unbounded root heads and fail
   closed when a source is unavailable.
7. **M2 convergence correction**: hold one data-directory processing lease,
   normalize orphaned running tasks before contract activation, serialize
   configured and migration task heads, defer OCR until first publication, and
   bound publication attempts plus cleanup failures across restart.
8. **M3 historical-data convergence**: enforce bounded and fair rule output at
   the producer, make persisted aggregates time-independent, sanitize embedded
   control bytes before content hashing, atomically stage every immutable
   ingest decision, and permit one restricted old-contract recovery from an
   unpublished runtime-invariant block.
9. **M4 exact metadata projection**: bind every query-visible document field to
   the exact active document/version/generation, publish that snapshot in the
   existing metadata/head/journal CAS, and split foreground FIFO publication
   from fail-fast reconciliation/migration/OCR maintenance acquisition. Model
   retained, same-version metadata and replacement actions explicitly so a
   rename rebuilds affected artifacts and cannot be swallowed as an exact
   no-op.
10. **P1 macOS installed-evidence hard cut**: bind the complete 0.1.1 bundle
    composition into an owner-only install receipt and make install, upgrade
    and uninstall crash-recoverable through one canonical phase journal,
    crash-released OS lock, durable partial promotion and tombstone GC. Reject
    0.1.0 as an upgrade trust root and use an authorized clean replacement that
    preserves Application Support.
11. **R3 cold-restart readiness correction**: make the published bound IPC
    control plane a prerequisite for combined-mode artifact maintenance, run
    recovery only in the supervised background service, and remove the
    generation-first-tick bypass of the durable completed-root rescan interval.
12. **R4 bounded artifact-repair correction**: hard-cut to COW v29, discard
    pre-manifest staging, validate current-only repair authority, make physical
    fulltext/vector publication locks fail fast with typed busy, persist a
    five-attempt repair ledger from failure completion, supervise retryable
    worker ticks without process exit, and persist the desktop restart window
    across App generations with explicit invalid-ledger diagnostics. Bind every
    publication to a persisted typed head/migration/artifact-repair authority;
    before physical cleanup, atomically record an immutable retirement intent
    and mark the journal query-ineligible. Replay at most 64 pending intents
    before a `repair_blocked` return, any new publication/attempt or broad GC.
    Completion uses only a transaction-authorized exact-artifact CAS; pending
    rows cannot be updated, deleted or replaced around that owner. Atomically
    settle and durably retain the exact attempt with the exact-head block if
    retirement cannot complete.
13. **V1 pre-deployment verification**: focused and broad tests, contract and
    privacy gates, production frontend/desktop builds and deterministic fault
    tests, excluding the long soak.
14. **D3 merged-main installed acceptance**: merge through the normal protected
    path, clean up the feature branch/worktree while preserving unrelated
    changes, sync local and remote `main`, build and install from that exact main,
    then use a temporary COW copy of the authorized local store to verify cold
    migration/recovery, Ready, strong-kill recovery, held fulltext/vector lock
    bounded retry, normal quit/reopen and redacted diagnostics. The repeatable
    native gate is `npm run acceptance:macos:installed-main --prefix
    apps/desktop -- ...`; it derives version, icon and source commit from a
    serially observed clean worktree whose HEAD equals freshly observed remote
    `main`, rejects torn or caller-supplied provenance assertions, and performs
    that read-only check before any stale-runtime cleanup. Under the run-wide
    lease it revalidates the same authority, builds from an isolated local clone
    of the exact commit, and rechecks lease plus live source authority before
    every system mutation. Bundle-composition, DMG and owner receipt v2 all bind
    that commit. The outer command itself runs the canonical
    release build and install/upgrade/reinstall transaction for exact version
    0.1.2; neither a pre-existing App nor a later source version can satisfy the
    gate. The gate holds the real macOS lifecycle
    lock for its full duration, uses an explicitly authorized v28 source and
    APFS `clonefile(2)` without a copy fallback, validates exact fulltext/vector
    `publication_busy` causes across two timed resident attempts, and requires
    one real fifth-attempt terminal block. Any non-applied publication must
    first have its typed-authority retirement intent durably recorded; a
    cleanup failure must settle that exact attempt and block only its exact
    head in one transaction before the harness releases the external lock or
    begins teardown. It binds persisted
    strong-kill evidence to the current receipt boundary, targets normal quit by
    the launched PID, and checks all four bundled native executables for
    residue. Source provenance requires the single canonical HTTPS origin plus
    a clean HEAD equal to freshly observed remote `main`; the authorized v28
    active-store manifest must have exactly four canonical records and one
    trailing LF. The combined desktop diagnostics export remains a separate
    native save-dialog check in this installed gate.
    Every v2 build, DMG, install, upgrade and installed-main verification uses
    absolute Apple system-tool paths with a closed environment and
    `shell:false`. The App and all four nested native executables must each have
    the exact ad-hoc hardened signature policy; only the embedding runtime may
    carry the single library-validation entitlement. Installed acceptance
    revalidates that five-target policy before every launch and at final
    teardown. Daemon diagnostics are accepted only with exact nested shapes,
    closed enums and bounded counters/latencies.
    DMG verification and install/upgrade consumption share one mounted-image
    lease: the image pathname identity, size and digest are checked before and
    after attach; the verified App is copied inside that same lease and is never
    remounted by pathname. Partial attach failure probes and detaches at most
    once. Bundle composition covers every regular App file except the code
    signature directory and the composition evidence itself, rejects links and
    irregular entries, and fails before hashing beyond its fixed file cap.
    Cold Ready is accepted only after the active v29 metadata file digest,
    fulltext/vector generation and projection/epoch identities agree. It then
    imports one fixed owner-only public synthetic canary over authenticated
    daemon IPC and requires a nonzero result at the exact current epoch before
    reusing that witness across kill and relaunch. Encrypted artifacts are
    streamed and matched to their manifest digest with owner/mode/inode/size
    stability. The two contention lanes run before a
    final normal quit/relaunch, Ready/search check, strict redacted diagnostics
    check, final quit and residue inspection. SIGINT/SIGTERM and next-run stale
    recovery operate under the lifecycle lock. A durable
    intent/pending/running marker binds PID, PGID, process start, executable and
    session authority; the guardian and stale recovery reap the entire exact
    authority group even if its leader and App have disappeared. COW cleanup
    validates the parent inode, renames the exact workspace into a random
    quarantine, revalidates identity and never deletes a pathname replacement.
15. **V2 frozen soak**: only after installed acceptance finds no new problem,
    freeze the exact code and run the 120-minute synthetic soak once. Any new
    deployed failure must first become a reproducible synthetic or installed
    regression, be fixed and reinstalled; the soak then restarts from zero.

## Fixed supervisor policy

- startup deadline 10 seconds;
- child observation every 100 ms;
- health probe every 5 seconds with a 2-second timeout;
- recycle only after three consecutive failed probes;
- five unexpected failures per rolling ten minutes;
- backoff 250 ms, 1 s, 4 s, 15 s, 30 s;
- reset after five continuous ready minutes;
- circuit open for five minutes, with one bounded manual half-open attempt.

Manual retry is valid only while the actor is `circuit_open`; it advances that
same failure history into the one half-open attempt. Every `blocked` reason is
sticky and ignores manual retry, so configuration, protocol, integrity,
ownership, supervisor and restart-ledger failures cannot be bypassed by the UI.

The rolling failure window, scheduled delay, circuit time and one-shot normal
shutdown start authority are persisted in one bounded owner-only ledger. A
normal Ready App shutdown does not erase failure history or count as a failure;
the next start atomically consumes its authority. Missing authority, corrupt
state, unsafe permissions, clock rollback or persistence failure blocks startup
and remains diagnosable without a live daemon.

Configuration, protocol, integrity and ownership conflicts enter `blocked`.
Normal App shutdown is not a failure. Search, import, detail and hydrate are
never replayed automatically.

## Delivery discipline

Work runs in an isolated worktree from the observed local HEAD. The two
pre-existing untracked research files are not copied, changed or deleted.
Normal slices stay within the repository PR budget; the coordinated v29 cut
uses an explicit scope exception and cannot auto-merge. Every slice records
focused red/green evidence and updates `PROGRESS.md`. No remote mutation occurs
until capability attestation is green.

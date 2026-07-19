# Daemon Reliability And Detail Snapshot Correctness v28 Corrective Plan

The path retains `v27` only for linked issue-train continuity. This v28 plan is
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
11. **V1 verification**: focused and broad tests, contract/privacy gates,
   synthetic soak, GUI evidence and target-platform native smoke.

## Fixed supervisor policy

- startup deadline 10 seconds;
- child observation every 100 ms;
- health probe every 5 seconds with a 2-second timeout;
- recycle only after three consecutive failed probes;
- five unexpected failures per rolling ten minutes;
- backoff 250 ms, 1 s, 4 s, 15 s, 30 s;
- reset after five continuous ready minutes;
- circuit open for five minutes, with one bounded manual half-open attempt.

Configuration, protocol, integrity and ownership conflicts enter `blocked`.
Normal App shutdown is not a failure. Search, import, detail and hydrate are
never replayed automatically.

## Delivery discipline

Work runs in an isolated worktree from the observed local HEAD. The two
pre-existing untracked research files are not copied, changed or deleted.
Normal slices stay within the repository PR budget; the coordinated v28 cut
uses an explicit scope exception and cannot auto-merge. Every slice records
focused red/green evidence and updates `PROGRESS.md`. No remote mutation occurs
until capability attestation is green.

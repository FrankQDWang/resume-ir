# Daemon Reliability And Detail Snapshot Correctness v27 Plan

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
5. **C1-C3 coordinated data hard cut**: introduce immutable v27 storage,
   source revisions, active projection, atomic search publication, exact query
   leases and selection-bound detail/hydrate. These commits share one merge
   boundary so no supported build contains two searchability authorities.
6. **M1 repair migration**: copy source identity to v27, invalidate unprovable
   v26 derived state, reconcile from authorized roots and fail closed when a
   source is unavailable.
7. **V1 verification**: focused and broad tests, contract/privacy gates,
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
Normal slices stay within the repository PR budget; the coordinated v27 cut
uses an explicit scope exception and cannot auto-merge. Every slice records
focused red/green evidence and updates `PROGRESS.md`. No remote mutation occurs
until capability attestation is green.

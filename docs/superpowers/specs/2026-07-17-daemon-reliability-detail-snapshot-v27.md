# Daemon Reliability And Detail Snapshot Correctness v29 Corrective Revision

## Status

The original v27 correctness train was approved for local implementation on
2026-07-17. Native testing subsequently proved that its first-publication
recovery did not converge on an installed legacy corpus. The v28 correction
made IPC publish before recovery but still allowed a physical snapshot lock to
block the resident worker indefinitely while health remained `repairing`. This
v29 corrective revision supersedes the v28 recovery acceptance claim while
retaining the already-versioned daemon, desktop and selection contracts. It is a
correctness train, not a stable-release or cross-platform completion claim.
The path retains `v27` only for linked issue-train continuity; this v29 title
and body are the authoritative contract and do not authorize an old reader.

## Confirmed failures

1. A client that resets a socket while the daemon writes a response can cause
   the accept loop to return, remove the endpoint manifest, stop the import
   worker, and terminate the process.
2. The desktop starts or reuses the daemon once but does not supervise the
   owned child after startup.
3. Search returns a version id, while detail and hydrate discard it and select
   the latest visible row by document id.
4. Normal parsing derives a resume-version id without the content hash and the
   store overwrites that row on conflict.
5. Import publishes document/version metadata before the new index head, so
   metadata, fulltext, vector and detail can observe different generations.
6. A production-shaped legacy rebuild exposed two additional invalid derived
   inputs: years-experience evidence could span more than the 4 KiB immutable
   mention bound, and parser text could retain embedded NUL control bytes.
   Sequential immutable writes then left partial version state behind after a
   later derived-row rejection.
7. Once that failure persisted `repair_blocked/runtime_invariant`, changing the
   processing contract identity alone could not reopen the unpublished rebuild;
   the activation seam accepted only `repairing/migration_rebuild`.
8. Immutable ingest staging still updated the mutable `document` row before
   publication. An active version A could therefore hydrate version identity
   and index data from A while exposing path, hashes and status staged for B;
   a failed publication left that mixed generation visible indefinitely.
9. One publication-session API combined generation-local serialization with
   fail-fast maintenance ownership. Making every caller wait fixed transient
   command failures but deadlocked migration/OCR competition barriers; making
   every caller fail fast made delete IPC race a 25 ms reconciler tick.
10. The first active-snapshot correction still classified a same-version file
    rename or move as an exact no-op. The mutable source path changed, but the
    active metadata snapshot and full-text file name remained old indefinitely,
    so detail could resolve an immutable body through an unavailable old path.
11. Installed strong-kill testing showed that a healthy 7,607-document
    generation could spend more than the fixed ten-second desktop startup
    deadline validating or recovering fulltext/vector artifacts before binding
    and publishing IPC. The supervisor therefore saw no process health surface
    and correctly classified every restart as `startup_timeout`.
12. The resident worker also replaced the configured completed-root rescan age
    with zero on the first tick of every process generation. A restart therefore
    forced an immediate full-corpus rescan and could create the interrupted
    publication that made the next cold start expensive.
13. Fulltext and vector publication used blocking operating-system locks. An
    abandoned or externally held lock therefore blocked the only resident
    maintenance worker forever while IPC continued to report `repairing`.
14. Artifact backoff was calculated from attempt start rather than failure
    completion, so a long failed attempt could make the next retry immediately
    due. A pre-manifest v29 target could also be reused after the authoritative
    v28 source advanced, promoting stale staged data.
15. The desktop restart budget lived only inside one supervisor actor. Closing
    and reopening the App reset the rolling failure window and could evade the
    fixed circuit policy; an invalid ledger had no explicit blocked reason.

The historical single unavailable document has no retained witness, so its
exact timeline is unknown. The structural failure modes above are independently
confirmed and sufficient to explain that class of outcome.

## Required invariants

- Connection, request, response and dependency failures are request-local.
  Only listener ownership, worker supervision and runtime invariants are
  process-fatal.
- A native supervisor is the sole owner of the desktop daemon process. It uses
  bounded restart policy and never replays business requests.
- In combined resident mode, a typed bound control-plane capability publishes
  the generation manifest and owns the listener before any search-artifact
  validation, recovery, rebuild or garbage collection can start. Existing
  immutable readers remain serviceable while that supervised maintenance runs.
- Completed-root rescan eligibility is based only on durable completion time
  and the configured interval. Process generation and worker tick are not
  scheduling inputs; migration rebuild reconciliation remains a separate
  explicit path.
- `ResumeVersion` is immutable and content-addressed. A stable id can never
  name different text or fields.
- Text normalization removes forbidden inline control bytes before hashing.
  Version-bound derived fields are deterministic, bounded at their producer,
  and staged with their document/revision/version/classification in one
  transaction.
- `ActiveSearchProjection` is the only searchability authority. Staged data is
  never queryable. It carries the complete query-visible document snapshot for
  its exact `(document, version, generation)` rather than hydrating mutable
  document state.
- Metadata projection, fulltext/vector heads and visible epoch change through
  one compare-and-swap publication plan.
- Search, detail and hydrate carry a closed `SearchSelection` containing
  document id, version id and visible epoch. There is no doc-only or latest-row
  fallback.
- All failure evidence is bounded and redacted. Daemon death cannot make
  lifecycle diagnostics unavailable.
- Publication ownership has two explicit contracts: foreground publication
  waits in one generation-local FIFO; reconciliation, migration and OCR
  maintenance use a non-queuing fail-fast acquisition. Physical fulltext and
  vector publication locks are also single-attempt `try_lock` operations and
  return typed `PublicationBusy`; no crate-internal wait or retry loop exists.
- Artifact repair is a durable closed state machine. It records failure
  completion time, retries at 1/4/15/30/60 seconds, and enters an explicit
  `repair_blocked` state after five failures instead of remaining repairing.
  Wall-clock rollback transactionally rebases the same attempt's deadline and
  cannot create an unbounded retry delay or reset the attempt count.
- Desktop restart history, scheduled backoff and circuit state survive App
  restart in an owner-only bounded ledger. Invalid ledger state fails closed
  with a typed lifecycle reason while diagnostics remain exportable.
- A macOS install, upgrade or uninstall writes an owner-only, canonical phase
  journal before every destructive rename, removal or receipt commit. Re-entry
  verifies the journal, App composition and receipt evidence and deterministically
  commits or rolls back; ambiguous or tampered state fails closed without
  deleting either version.

## Hard-cut contracts

- metadata schema `v29` with `SourceRevision`, immutable `ResumeVersion`,
  version-bound derived rows and `ActiveSearchProjection`;
- fulltext snapshot `v3` and vector snapshot `v4` with exact version identity;
- daemon discovery/auth `v2` with a per-generation instance id and token;
- search/detail/hydrate IPC `v3` with `SearchSelection`;
- daemon status `v2`, diagnostics `v3`, and one bounded IPC error envelope;
- desktop lifecycle and desktop diagnostics `v1`.

Both `daemon.status.v2` and `resume-ir.diagnostics.v3` carry required-nullable
`repair_reason` and `repair_progress`. The reason's closed enum is `migration_rebuild`,
`artifact_unavailable`, `source_unavailable`, and `runtime_invariant`; omission,
an unknown value, or a reason inconsistent with the metadata/query service
states is a contract failure. Ready and metadata-unavailable responses include
both keys with `null` rather than omitting them. Progress exposes only bounded
phase, attempt budget, retry delay and one closed `last_error_kind`. The exact
kinds are `fulltext_publication_busy`, `fulltext_failure`,
`vector_publication_busy`, `vector_failure`, `metadata_failure`, and
`interrupted`; no independently writable subsystem/reason fields or legacy
`last_error_class` alias exist. A blocked service returns action
`repair_required`, never an ordinary retry hint.

The macOS 0.1.2 App uses bundle-composition, DMG-composition and owner-install
receipt v2 contracts. The composition binds the exact merged-main
`source_commit` into its digest, and the DMG and installed receipt repeat and
match that commit. Normal readers are v2-only. The sole legacy admission is an
exact pinned 0.1.1 installed artifact admitted only as the predecessor of the
0.1.1-to-0.1.2 transaction. Because v1 never recorded a source commit, the
transition must not fabricate one or expose a generic v1/v2 compatibility
reader. Once the v2 receipt is durable, recovery converges forward and removes
the legacy receipt.

The v2 composition also binds the complete regular-file tree under the App,
excluding only `Contents/_CodeSignature/**` and its own composition evidence;
links, irregular entries, extra files, content drift and over-cap trees fail
closed. DMG verification owns one mounted-image lease from the pre-attach
pathname identity/size/digest through post-attach revalidation, App
verification and transaction-owned copy. Install and upgrade cannot remount
the pathname, and a failed or timed-out partial attach is probed and detached
once before returning.

Prepared search artifacts never cross the publication transaction boundary as
an ownable production value. Callers receive only a borrowed decision view;
the boundary consumes the prepared owner on every outcome, returning a
committed fence only for `Applied`. Cancellation, error and supersession first
atomically persist an immutable fulltext/vector retirement plan, the exact
CurrentHead/MigrationRebuild/ArtifactRepair authority and the query-ineligible
`Abandoned` journal state; only then may physical deletion begin. Per-artifact
completion is a transaction-authorized exact-artifact CAS; direct completion,
DELETE or REPLACE cannot bypass the pending gate. It is monotonic and
replayable. Any deferred, partial or
identity-mismatched retirement atomically settles the exact running attempt and
blocks only the head still named by that typed authority as
`runtime_invariant`; a newer migration contract, repair attempt or ready head
is never overwritten. No durable attempt remains `Running` and no next attempt
can start behind pending cleanup.

Old readers, dual writes, schema aliases, mutable-version updates and
`latest_visible_*` fallbacks are removed in the same merge boundary.

## Recovery and migration

Legacy derived version, mention, candidate-link, import-task and index data
cannot prove historical identity or exact first-publication completeness.
Migration therefore preserves only stable source/root/document identity and
authorization, builds and validates a copy-on-write v29 store, discards legacy
tasks and derived state, retires legacy fulltext/vector layouts under the
publication lock, enters `repairing`, and reconciles every active authorized
root from source. There is no dual reader, legacy alias or in-place schema
upgrade. Before manifest publication any existing v29 target is uncommitted
staging and is deleted, then recopied from the current authoritative v28 store.
Migration-private descriptor authority exists only inside the COW transaction;
every active v29 reopen exactly validates the permanent current-only trigger.

A data-directory-wide processing lease is acquired before contract activation
and held for the daemon generation or complete offline command. With that lease,
orphaned running tasks are normalized before activation; legacy task-lock
contention fails closed. Configured enqueue and migration reconciliation share
one SQLite `IMMEDIATE` per-root head coordinator, so concurrent requests cannot
leave a newer unclaimable task ahead of the exact rebuild task. The migration
purpose is persisted, every active root is scanned without a budget, and only
sealed dispositions from those exact completed task heads may enter the first
projection.

The first v29 generation crosses one typed all-root barrier. Its token binds
the inherited visible epoch and exact latest completed, non-cancelled task plus
a complete, non-exhausted, error-free scan scope for every active authorized
root; paused roots are atomically outside the publication set. The final
fulltext/vector/projection commit revalidates that token, lifecycle and heads in
the same immediate transaction. Any root/task/head, cancellation, scan or
projection change supersedes the publication. OCR is not claimed while the
first migration publication is incomplete; after `Ready`, the next worker tick
may claim and publish OCR through the normal exact-version boundary.

Publication attempts are durable across restart, use a closed failure class,
fixed bounded backoff from failure completion and a five-attempt budget. Every
failed snapshot attempt retires only its exact unpublished generation and its
generation-scoped staging/pin artifacts while holding the publication lock;
it cannot use broad garbage collection as failure cleanup. Invalid, deferred or
partial retirement fails closed as `runtime_invariant/repair_required` before
another attempt can begin, instead of accumulating artifacts or retrying
forever. This applies after validation to commit storage failures, projection
CAS supersession, OCR claim/publication supersession and migration
supersession: each path must first commit the typed-authority retirement intent
and `Abandoned` state, then retire only that generation. At reopen and before
new migration, artifact-repair or ordinary publication work, at most 64 pending
intents are replayed ahead of interrupted-journal recovery and broad GC; an
overflow fails closed. Replay also precedes a sticky `repair_blocked` return so
physical cleanup can converge without reopening business work. Pending intents
cannot be pruned, forged, deleted or replaced, and an exact terminal cleanup
attempt remains durable after outer failure settlement. An absent
artifact is accepted as replay-complete only under that exact durable intent
when the generation is not retained. Legacy v28 `preparing` or `validated`
journals migrate to `Abandoned` plus pending `may_exist` cleanup, never to a
fabricated complete record. Unreadable
sources enter sticky `repair_blocked/source_unavailable`; unsafe artifact,
cleanup or runtime invariants enter sticky
`repair_blocked/runtime_invariant`. Root ordering, retry timestamps, clock
rollback, cancellation, concurrent enqueue or restart cannot publish a partial
corpus or fabricate searchability.

Rule extraction owns the immutable entity-mention contract. Candidate storage
is bounded per field type while extraction runs, exact source occurrences remain
distinct, final retention is fair across field types, and only the final bounded
set is staged. Persisted years-experience aggregates use the union of closed
date ranges only; `PRESENT` never reads the wall clock, and derived aggregates
carry no source span. Inline control characters are normalized to stable word
boundaries before clean-text hashing, with the metadata store retaining a
fail-closed NUL check.

Immutable ingest staging is one SQLite `IMMEDIATE` transaction. A mention,
classification, identity or candidate-assignment failure rolls back the whole
document/revision/version stage. An unpublished
`repair_blocked/runtime_invariant` rebuild may be reopened only by an exact
processing-contract hard cut when generation is null, the prior active contract
is non-empty and different, and no task is running. That transaction invalidates
only old task-derived state, preserves source identity and immutable rows, and
returns to `repairing/migration_rebuild`. The same contract, source-unavailable
state, ready/generation-bearing state and running tasks remain sticky.

The mutable `document` row remains source-processing state and is not a search
hydration source. Every active projection seals the bounded query-visible
document fields alongside the exact document/version/generation. Publication
validates its version-to-source-revision content hash and byte size, copies a
retained version's prior active snapshot, and admits a replacement snapshot
only in the same `IMMEDIATE` transaction that swaps projection, journal, head
and visible epoch. Detail hydration, field filtering and retained full-text
rebuild read that exact active snapshot; there is no mutable-document fallback.
Each commit carries one explicit action per projection:
`RetainedUnchanged`, same-version `MetadataChanged`, or version-changing
`Replacement`. Shape and transition validation reject missing, reordered,
misclassified or no-op actions. A same-version path, file-name or timestamp
change builds a new artifact generation and atomically publishes the new
metadata snapshot while preserving immutable content identity; only a
byte-for-byte equal query-visible snapshot is an exact no-op.

The macOS 0.1.2 bundle retains the self-contained installation trust
root. Bundle composition binds the desktop executable, daemon/runtime native
payloads, runtime manifests and their resource bytes, icon, bundle identity,
version and target. The owner-only receipt binds the installed composition.
Install, upgrade and uninstall use a canonical, deny-unknown lifecycle journal
with durable phases and idempotent crash recovery. A validated owner-only lock
file is held by `/usr/bin/lockf`; a pipe-held capability makes process death
release the kernel lock. Transaction-owned partial trees are recursively
synced before atomic promotion, and every recursive deletion first becomes a
durable tombstone whose cleanup is repeatable after interruption. Journal
creation is compare-and-swap and every recovery/rollback entry requires the
live lock capability. Legacy 0.1.0 has no such evidence and is replaced cleanly
while preserving Application Support; it is not accepted by a compatibility
reader or promoted into a fabricated receipt.

## Acceptance

- deterministic reset coverage for status, search, batch, detail, hydrate and
  progress followed by a successful request to the same daemon;
- bounded supervisor recovery, crash-loop circuit opening, clean App shutdown
  and persistent privacy-safe lifecycle receipts and restart-window ledger;
- an externally held fulltext or vector publication lock returns typed busy
  without blocking status/search, then converges in the same daemon after the
  lock is released; five failures become `repair_required`;
- a pre-manifest staging target is never reused after the v28 source changes,
  and active v29 reopen rejects missing, legacy or migration-only authority;
- same source with different content produces different immutable versions;
- every staged/write/validate/commit fault keeps the prior projection and index
  usable;
- staging replacement B while A is active keeps A's complete document snapshot
  visible; an unrelated or failed publication preserves A, and one successful
  CAS switches every query-visible field to B;
- a same-version rename keeps the existing selection and immutable body but
  publishes a new generation/epoch whose detail path and full-text file name
  are both new; a failed CAS preserves the complete old generation;
- fulltext, field, semantic and hybrid hits agree on exact version identity;
- oversized/entity-flood, embedded-control and partial-stage fixtures converge
  without losing later high-value fields or persisting invalid text;
- an old blocked processing contract converges once after a hard cut, while the
  current blocked contract remains sticky across restart;
- target replacement yields typed stale selection and can never mix body pages;
- focused tests, workspace verification, privacy gates, 120-minute synthetic
  soak, and native macOS/Windows evidence remain separate claims.
- every macOS lifecycle phase supports crash/re-entry convergence; invalid
  journal permissions, unknown fields, digest drift or ambiguous filesystem
  state fail closed without losing an installed App or Application Support.
- v2 macOS evidence uses only absolute Apple system tools in a closed
  environment with `shell:false`; App plus four nested native executables all
  carry the exact ad-hoc hardened signature, and only the embedding runtime has
  the single allowed entitlement. Installed verification repeats this policy
  before every launch and at final teardown.
- installed daemon diagnostics accept only exact nested shapes, closed enums
  and bounded counts/latencies; unknown nested text or fields fail closed.
- the installed-main gate itself observes a fresh remote `main`, requires a
  serially stable clean equal local commit before any recovery mutation,
  revalidates it under the lifecycle lease, and builds the verified DMG from an
  isolated local clone of that exact commit. Lease and live source authority
  are rechecked before every install, upgrade, uninstall, import, signal, lock,
  clone and quit mutation. A pre-existing App, caller assertion, mutable
  checkout or later version cannot substitute for that build-to-install
  transition.
- the authorized schema-v28 source is cloned only with APFS `clonefile` and is
  proved unchanged. Cold acceptance verifies the v29 metadata file digest,
  fulltext/vector generation and projection/epoch agreement, then imports one
  fixed owner-only public synthetic canary through authenticated daemon IPC and
  requires a nonzero exact-current-epoch result without retaining query or
  result data. Ciphertext artifact bytes are streamed into the exact manifest
  SHA-256 while file identity, size, owner and mode remain stable; key material
  is owner-only and strictly bounded.
- strong-kill recovery precedes the fulltext and vector contention lanes. Only
  after both locks are released and the same daemon has recovered does the gate
  perform its final normal quit/relaunch, Ready/search witness, strict redacted
  diagnostics check, final quit and native-process residue check.
- SIGINT/SIGTERM and the next run recover only marker-bound COW workspaces and
  exact owned process identities under the lifecycle lock. Durable launch
  intent/pending/running state binds PID, PGID, process start, executable and
  session authority; guardian and stale recovery reap the whole exact process
  group even if the leader and App are already gone. Workspace deletion first
  validates its parent inode, atomically quarantines the exact inode and
  revalidates it, preserving any pathname replacement. No bundle-id-wide kill,
  unverified recursive deletion or private clone residue is accepted.
- merged-main installation acceptance precedes the soak; bundle, DMG, receipt,
  installed acceptance and the uninterrupted 120-minute soak bind one commit.
  Any deployed failure first becomes a repeatable regression and invalidates
  all prior non-soak/soak evidence before a new merge and installation.
- manual retry advances only `circuit_open` into its one half-open attempt; all
  `blocked` reasons remain blocked and spawn no child.

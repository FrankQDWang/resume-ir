# Daemon Reliability And Detail Snapshot Correctness v28 Corrective Revision

## Status

The original v27 correctness train was approved for local implementation on
2026-07-17. Native testing subsequently proved that its first-publication
recovery did not converge on an installed legacy corpus. This v28 corrective
revision supersedes the v27 metadata/migration acceptance claim while retaining
the already-versioned daemon, desktop and selection contracts. It is a
correctness train, not a stable-release or cross-platform completion claim.
The path retains `v27` only for linked issue-train continuity; this v28 title
and body are the authoritative contract and do not authorize a v27 reader.

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

The historical single unavailable document has no retained witness, so its
exact timeline is unknown. The structural failure modes above are independently
confirmed and sufficient to explain that class of outcome.

## Required invariants

- Connection, request, response and dependency failures are request-local.
  Only listener ownership, worker supervision and runtime invariants are
  process-fatal.
- A native supervisor is the sole owner of the desktop daemon process. It uses
  bounded restart policy and never replays business requests.
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
  maintenance use a non-queuing fail-fast acquisition. Neither contract is an
  implicit retry loop.
- A macOS install, upgrade or uninstall writes an owner-only, canonical phase
  journal before every destructive rename, removal or receipt commit. Re-entry
  verifies the journal, App composition and receipt evidence and deterministically
  commits or rolls back; ambiguous or tampered state fails closed without
  deleting either version.

## Hard-cut contracts

- metadata schema `v28` with `SourceRevision`, immutable `ResumeVersion`,
  version-bound derived rows and `ActiveSearchProjection`;
- fulltext snapshot `v2` and vector snapshot `v3` with exact version identity;
- daemon discovery/auth `v2` with a per-generation instance id and token;
- search/detail/hydrate IPC `v3` with `SearchSelection`;
- daemon status `v2`, diagnostics `v3`, and one bounded IPC error envelope;
- desktop lifecycle and desktop diagnostics `v1`.

Both `daemon.status.v2` and `resume-ir.diagnostics.v3` carry a required-nullable
`repair_reason`. Its closed enum is `migration_rebuild`,
`artifact_unavailable`, `source_unavailable`, and `runtime_invariant`; omission,
an unknown value, or a reason inconsistent with the metadata/query service
states is a contract failure. Ready and metadata-unavailable responses include
the key with `null` rather than omitting it.

Old readers, dual writes, schema aliases, mutable-version updates and
`latest_visible_*` fallbacks are removed in the same merge boundary.

## Recovery and migration

Legacy derived version, mention, candidate-link, import-task and index data
cannot prove historical identity or exact first-publication completeness.
Migration therefore preserves only stable source/root/document identity and
authorization, builds and validates a copy-on-write v28 store, discards legacy
tasks and derived state, retires legacy fulltext/vector layouts under the
publication lock, enters `repairing`, and reconciles every active authorized
root from source. There is no dual reader, legacy alias or in-place schema
upgrade.

A data-directory-wide processing lease is acquired before contract activation
and held for the daemon generation or complete offline command. With that lease,
orphaned running tasks are normalized before activation; legacy task-lock
contention fails closed. Configured enqueue and migration reconciliation share
one SQLite `IMMEDIATE` per-root head coordinator, so concurrent requests cannot
leave a newer unclaimable task ahead of the exact rebuild task. The migration
purpose is persisted, every active root is scanned without a budget, and only
sealed dispositions from those exact completed task heads may enter the first
projection.

The first v28 generation crosses one typed all-root barrier. Its token binds
the inherited visible epoch and exact latest completed, non-cancelled task plus
a complete, non-exhausted, error-free scan scope for every active authorized
root; paused roots are atomically outside the publication set. The final
fulltext/vector/projection commit revalidates that token, lifecycle and heads in
the same immediate transaction. Any root/task/head, cancellation, scan or
projection change supersedes the publication. OCR is not claimed while the
first migration publication is incomplete; after `Ready`, the next worker tick
may claim and publish OCR through the normal exact-version boundary.

Publication attempts are durable across restart, use a closed failure class,
fixed bounded backoff and a five-attempt budget. Every failed snapshot attempt
is cleaned while holding the publication lock; invalid or partial cleanup fails
closed instead of accumulating artifacts or retrying forever. Unreadable
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

The macOS 0.1.1 bundle establishes the first self-contained installation trust
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
  and persistent privacy-safe lifecycle receipts;
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

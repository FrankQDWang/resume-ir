# Daemon Reliability And Detail Snapshot Correctness v27

## Status

Approved for local implementation on 2026-07-17. The work is a correctness
train, not a release-completion claim. GitHub mutation is unavailable because
runtime capability attestation found invalid credentials and no API access.

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
- `ActiveSearchProjection` is the only searchability authority. Staged data is
  never queryable.
- Metadata projection, fulltext/vector heads and visible epoch change through
  one compare-and-swap publication plan.
- Search, detail and hydrate carry a closed `SearchSelection` containing
  document id, version id and visible epoch. There is no doc-only or latest-row
  fallback.
- All failure evidence is bounded and redacted. Daemon death cannot make
  lifecycle diagnostics unavailable.

## Hard-cut contracts

- metadata schema `v27` with `SourceRevision`, immutable `ResumeVersion`,
  version-bound derived rows and `ActiveSearchProjection`;
- fulltext snapshot `v2` and vector snapshot `v3` with exact version identity;
- daemon discovery/auth `v2` with a per-generation instance id and token;
- search/detail/hydrate IPC `v3` with `SearchSelection`;
- daemon status `v2`, diagnostics `v3`, and one bounded IPC error envelope;
- desktop lifecycle and desktop diagnostics `v1`.

Old readers, dual writes, schema aliases, mutable-version updates and
`latest_visible_*` fallbacks are removed in the same merge boundary.

## Recovery and migration

The v26-derived version, mention, candidate-link and index data cannot prove
historical identity. Migration therefore preserves source/root/document
identity, builds a copy-on-write v27 store, invalidates legacy derived
artifacts, enters `repairing`, and reconciles from authorized sources. The new
store becomes active only after validation. Unreadable sources remain
`repair_blocked`; the product must not fabricate searchability.

## Acceptance

- deterministic reset coverage for status, search, batch, detail, hydrate and
  progress followed by a successful request to the same daemon;
- bounded supervisor recovery, crash-loop circuit opening, clean App shutdown
  and persistent privacy-safe lifecycle receipts;
- same source with different content produces different immutable versions;
- every staged/write/validate/commit fault keeps the prior projection and index
  usable;
- fulltext, field, semantic and hybrid hits agree on exact version identity;
- target replacement yields typed stale selection and can never mix body pages;
- focused tests, workspace verification, privacy gates, 120-minute synthetic
  soak, and native macOS/Windows evidence remain separate claims.

# Persistent file-observation fast path

Issue: [#268](https://github.com/FrankQDWang/resume-ir/issues/268)

## Boundary

PR #267 made an already-proven no-op scan generation-stable and stopped
equivalent fulltext/vector publication. This slice acts earlier: it avoids
discovery content sampling, full-content reads, and SHA-256 work when a
previously successful source occurrence can be proven eligible for a bounded
metadata fast path. It does not change the scheduler, watcher reconciliation,
parsers, classification, OCR, embeddings, or search publication design.

Disk content remains authoritative. First import, uncertainty, contract drift,
and real change always use the strong read/hash path.

## Durable owner and lifecycle

Schema v35 adds `source_file_observation`, keyed by
`(root_id, relative_path)` and cascading with `source_occurrence`. The row is
not part of `Document`: it describes one physical occurrence, not deduplicated
content or a logical document. It binds the observation to the occurrence's
current immutable source revision.

An observation is written only after:

1. an opened-handle read completed with matching pre/post metadata;
2. the path still resolved to the same observation;
3. processing produced a successful source disposition;
4. the source occurrence was committed.

Failed, incomplete, or observation-less imports do not create a row. A v34 or
older database migrates with an empty table and therefore fails closed until
each occurrence completes one strong import. Rows survive daemon/store restart.

## Assurance model

`macos_stat_v1` requires all of:

- stable identity derived from device, inode, and birth time;
- exact byte size;
- nanosecond modification time;
- nanosecond change time.

The current observation must match the stored value, an opened file handle
before and after rerun evaluation, and a final path revalidation. Replacement,
rename, metadata ambiguity, unavailable identity/time, permission or metadata
I/O failure, and observed TOCTOU all fall back or retry without accepting a
stale revision.

These fields are not a cryptographic proof. Privileged tooling may be able to
forge every metadata witness. Each stable identity therefore receives a
deterministic staggered strong-audit deadline between 6 and 24 hours. An
audit-due file is fully read and SHA-256 hashed. The guarantee is immediate
detection for ordinary same-size/restored-mtime changes through change time,
plus a less-than-24-hour bound for the explicit all-metadata-forged residual
case.

Non-macOS builds return no eligible observation and use the strong path. They
are not current delivery targets.

## Processing contract

A matching observation supplies only the previously persisted strong content
digest. The existing `exact_rerun_decision` remains the sole processing gate
and still validates deletion, extension, size, source revision, schema/parse
version, classifier epoch, source-triage epoch, active projection,
classification, OCR job, and document status. Any rejection falls back to a
fresh strong read/hash.

## Aggregate evidence

`ImportIoMetrics` separates discovery content opens/sampled bytes, metadata
handle opens, full-content opens/bytes, strong hashes attempted/skipped,
persisted observations, fast-path hits, and a closed set of fallback counts.
These are bounded aggregates only; they contain no paths, names, text, raw
hashes, or private corpus material.

## Compatibility and rollback

v35 is a continuous copy-on-write migration from v34. Current binaries accept
v29-v34 as migration sources and open only exact v35 authority. Reverting the
fast-path consumer safely restores strong reads, but an older binary cannot
open the v35 authority in place. Binary downgrade requires restoring the
retained predecessor store or reimporting.

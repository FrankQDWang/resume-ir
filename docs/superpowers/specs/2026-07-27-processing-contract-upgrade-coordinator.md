# Processing-Contract Upgrade Coordinator

## Status

Approved for implementation on 2026-07-27. This contract root-fixes the missing
online processing-contract transition state machine. It does not revoke the
artifact-recovery primitives from PR #249, the control-plane front door from
PR #248, or the v32→v33 migration performance/diagnostic fixes from PR #245.

The forward-migration feature-train clause remains authoritative:

> Parser-contract changes enqueue old PDFs for low-priority reprocessing without
> blocking startup or existing search.

This document defines how that clause is realized for every
`ImportProcessingContract` dimension, not only PDF parser changes.

## Problem

`ImportProcessingContract` combines four identity fields:

- primary parser
- OCR parser
- derived schema
- classifier epoch

The only production activation path today is
`activate_migration_rebuild_contract`, which accepts unpublished
`repairing/migration_rebuild` or `repair_blocked/runtime_invariant` hard cuts,
deletes all import tasks on success, returns `Superseded` for Ready stores, and
is treated as success by daemon/CLI callers. Worker ticks re-invoke activation as
if it were a health check. Bootstrap failures collapse to `runtime_invariant`
and block every capability, including keyword search and detail.

That API matches unpublished full rebuild, not online version upgrade.

## Core model

Four authorities are independent:

```text
MetadataAuthority
  exact schema / continuous COW migration

SearchAuthority
  generation + visible_epoch + artifact health
  Ready | Repairing | Blocked

WriterAuthority
  DesiredProcessingContract
  CommittedWriterContract
  claim fence epoch
  ready | transitioning | unavailable | blocked

ReprocessingCampaign
  target contract + affected domain
  Planned → Queued → Running → Complete | Partial | Cancelled
```

Hard rules:

1. Do not equate generic contract transition with PDF reprocess.
2. Do not merge writer transition completion with reprocessing completion.
3. Do not patch search/write coupling by stuffing upgrade reasons into
   `CoreReason`. Add an independent `WriterHealth` axis.
4. Runtime temporary unavailability disables writers; it must never auto-commit a
   fallback processing contract.
5. Acceptance canaries prove behavior; they are not production state-machine
   events.

## Desired / Runtime / Committed separation

| Concept | Source | Rule |
|---|---|---|
| `DesiredProcessingContract` | verified install pack + model artifact + product version | sole legal target candidate |
| `RuntimeAvailability` | optional runtimes / classifier health | whether Desired may execute |
| `CommittedWriterContract` | durable store | write authority; only transition commits it |

If classifier startup fails, the binary must not derive and commit a
deterministic fallback epoch as a new target. Writer becomes `unavailable`;
existing SearchAuthority continues.

## Writer transition state machine

```text
Observed
  → ClaimsFenced
  → WorkersQuiesced
  → TargetCommitted
  → WriterReady
```

- Persist the claim fence before waiting for workers. Checking “no running task”
  then committing without a fence allows TOCTOU claims.
- `BlockedByRunningOwner` is a retryable attempt failure, not a business phase
  peer of `TargetCommitted`.
- `PersistedStateInvalid` is terminal.
- Writer may become ready after `TargetCommitted` and campaign materialization.
  It must not wait for the entire historical reprocessing campaign to finish.

### Old-task policy

| Class | Policy |
|---|---|
| old-contract running | finish, cancel, or normalize before target commit; never write across commit |
| old-contract queued | atomically retire and rebuild under target contract; preserve user intent and scan snapshot |
| completed | immutable |
| scheduled PDF/OCR jobs | bind transition/campaign id and full target contract |
| cancel / pause controls | remain allowed while fenced |

Silent per-task cancellation because “task contract ≠ current contract” is not a
migration mechanism.

## Reprocessing campaign state machine

```text
Planned → Queued → Running → Complete | Partial | Cancelled
```

Independent of writer readiness. SearchAuthority remains the third axis and is
derived from projection/artifact health, not from campaign progress.

If a publication witness is required, it must bind exact target contract, exact
task/campaign, base generation, expected visible epoch, and publication
fingerprint. A bare `visible_epoch` increment is insufficient.

## ContractDelta decision matrix

Coordinator computes the delta across all four contract fields, then selects a
strategy. Concurrent field changes take the broadest strategy. Unknown deltas
return `unsupported_transition`: keep old search, disable new writers.

| Delta | Strategy |
|---|---|
| primary (PDF) parser only | PDF reprocessing campaign (root-level semantics below) |
| OCR parser | invalidate old OCR jobs/cache and requeue under target |
| classifier epoch | reclassify all relevant versions; not PDF-only |
| derived schema | may require full derived-data rebuild |
| multiple fields | broadest applicable strategy |
| unknown | unsupported; no fallback commit |

### PDF reprocess honesty

First ship accepts **per-root low-priority full rescan**. Existing
`pdf_reprocess_job` scheduling that ends in root scans must not be described as
source-revision-selective PDF reprocessing.

Campaign/job rows must bind the full `processing_contract_id` plus
transition/campaign identity, not only `parser_contract`. Revision-level
selective reprocess is a later enhancement.

## WriterHealth and capability derivation

Control-plane snapshot becomes:

```text
core + runtimes + writer + capabilities
CapabilityMatrix::derive(core, runtimes, writer)
```

```text
WriterHealth {
  state: ready | transitioning | unavailable | blocked
  reason
  transition_phase
}
```

`UpgradeInProgress` must not be added to `CoreReason`.

`index_publication` semantics are split:

- public / uncoordinated writer admission
- coordinator-authorized publication
- search-authority artifact repair
- privacy deletion publication

Internal coordinated writes use an unforgeable `WriterAuthorityToken` (or
equivalent coordinator capability), not a boolean status bit.

## Route admission matrix

| Class | Examples | Rule |
|---|---|---|
| Always allowed | status, diagnostics, search, detail, progress, cancel, pause, preview close | independent of WriterReady |
| Writer ready required | new import, register, resume, manual scan, watcher rescan | `WriterHealth` ready |
| Special high priority | privacy deletion, search-authority artifact repair | independent mutation authority / token |
| Offline CLI | DirectImport, DirectDelete, Purge, TaskControl, OCR, DoctorRecovery | same coordinator/store transition API; no second activation path |

Routes that currently mutate without `text_import` (source-root register, legacy
migration, root control, root delete) are in scope for the matrix. Changing only
`CapabilityMatrix::derive` and `/imports` is insufficient.

## Writer priority ladder

Not one global barrier:

1. metadata authority recovery
2. search-authority artifact repair
3. privacy deletion / quiescence
4. writer-contract transition
5. watcher, rescan, ordinary import, OCR/reprocess

Failed writer transition must not permanently stall artifact repair or privacy
deletion. Privacy deletion holds independent mutation authority.

## Schema v34 and install boundary

Persisted transition, campaign, fence, and WriterAuthority state require an
exact schema bump to **v34**, registered in the continuous COW migration chain.
Side-table extension without a schema bump is forbidden.

Also required:

- product version advances with schema (distinct schemas must not share one
  product version such as `0.1.8`)
- DMG reinstall/rollback compatibility with DB schema
- once a v34 authority has been published, install transactions must not roll
  back to an app that can only read v33
- v33 predecessor retention, destruction, and privacy deletion rules

Hard-cut activation remains only for unpublished `migration_rebuild` and
`repair_blocked/runtime_invariant` full rebuilds. Ready online upgrades use the
new transition API and must not delete all import tasks.

## Transition receipt

Internal durable fields at least:

- `transition_id`
- source / target contract IDs
- desired product / schema identity
- source search generation and visible epoch
- phase, transition attempt, claim fence epoch
- running / queued / scheduled task counts
- failure class, retryability, retry_after
- campaign ID, target publication witness
- created / updated / completed timestamps

Cross-version rules:

- target not yet committed: may be replaced by a newer Desired target under
  explicit rules
- target committed: no rollback; finish current transition before starting next
- new binary encountering an in-progress transition from an older binary: closed
  compatibility table (`accept` / `continue` / `fail-closed`)

Public status/diagnostics expose opaque transition id, phase, and class only.
Full digests stay private.

## Activation result vocabulary

Replace silent `Superseded`-as-success with:

- `AlreadyActive`
- `TransitionRequired`
- `TransitionInProgress`
- `BlockedByRunningOwner` (retryable attempt failure)
- `PersistedStateInvalid` (terminal)

## Delivery slices

1. **A** — this spec and implementation plan
2. **B** — v34 schema, transition/campaign API, delta planner; dormant
3. **C** — `WriterHealth`, status/diagnostics bump, route matrix; default writer
   ready (behavior-equivalent)
4. **D** — UpgradeCoordinator, claim fence, remove old activation call sites
5. **E** — crash-point, structural fixtures, old-task migration tests
6. **F** — real installed acceptance with pre-upgrade canary and target-contract
   attestation

Do not land orchestration before WriterHealth semantics. Every intermediate
`main` must remain publishable: B dormant, C behavior-equivalent.

## Acceptance (Slice F)

Exact merged-main DMG against an authorized v29 APFS/COW clone:

1. **Pre-upgrade canary** — using the v29 writer, plant a synthetic legacy
   canary in the COW clone before launching current DMG. While reprocessing is
   incomplete, keyword search finds it, detail reads the original selection, and
   generation is not replaced by an empty authority.
2. **Cold start / online switch** — schema v34, four runtimes available, same
   daemon generation, receipt shows target committed, WriterReady admits a new
   canary import.
3. **Target-contract attestation** — path-redacted private receipt proves import
   task and completion bind target contract, publication includes the canary,
   and transition ids match. “Searchable” alone is insufficient.
4. **Restart** — target commit count does not increase, transition id unchanged,
   hard-cut/task-purge counts are zero, queued/task identity aggregates are not
   wholesale cleared.

Then freeze and run the existing 120-minute soak.

## Non-goals

- Wholesale revert of PR #249
- Treating worker-tick activation as a health check
- Source-revision-selective PDF reprocess in the first ship
- Windows/Linux delivery gates

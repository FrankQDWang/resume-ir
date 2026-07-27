# Processing-Contract Upgrade Coordinator Implementation Plan

Linked spec:
`docs/superpowers/specs/2026-07-27-processing-contract-upgrade-coordinator.md`

## Ordered slices

### A — Contract freeze

- Land the linked spec and this plan.
- Point active-goal / progress notes at the new contract when the execution owner
  updates goal pointers.
- No production code in this slice beyond docs.

### B — Schema v34 + dormant store API

1. Add `schema_v34` with tables for:
   - writer transition / receipt
   - claim fence epoch
   - reprocessing campaign
   - extended PDF/OCR campaign binding to full `processing_contract_id`
2. Register `v33 → v34` in the continuous COW forward-migration registry.
3. Make v34 the only current openable schema; v29–v33 migrate through the chain.
4. Advance product version in `apps/desktop/package.json` with the schema bump.
5. Implement store modules (new files; do not grow `lib.rs` with logic):
   - transition observe/begin/fence/quiesce/commit/fail APIs
   - ContractDelta planner
   - campaign materialize/progress APIs
6. Keep dormant: bootstrap/worker/CLI still use existing activation paths.
7. Focused tests: migration apply/validate, delta matrix, result vocabulary.

### C — WriterHealth + route matrix (behavior-equivalent)

1. Add `WriterHealth` to `daemon-contract`.
2. Change `CapabilityMatrix::derive(core, runtimes, writer)`.
3. Bump status/diagnostics producer and consumers together.
4. Default writer to `ready` so public capability outcomes match today when core
   is Ready and runtimes match.
5. Implement the full route admission matrix, including mutations that do not
   currently check `text_import`, and CLI offline mutation entry points behind
   the same gate types.
6. Contract tests must stay green with no intentional behavior change.

### D — Coordinator activation

1. Add `crates/daemon/src/upgrade_coordinator.rs`.
2. Bootstrap calls the coordinator after store open; claim fence precedes
   quiesce and target commit.
3. Remove worker-tick `activate_contract` and silent `Superseded`-as-success
   mappings from daemon/CLI.
4. Apply the writer priority ladder; privacy deletion and search-authority
   artifact repair keep independent authority tokens.
5. Writer-only failures update `WriterHealth` without blocking Ready search.
6. Hard-cut remains only for unpublished rebuild / repair_blocked paths.

### E — Crash and combination tests

1. Redacted structural fixtures derived from authorized v29 shape.
2. Crash points: after fence, before commit, after commit without materialize,
   restart idempotence.
3. Old queued retirement/rebuild; running tasks cannot cross commit.
4. In-progress transition compatibility table cases.

### F — Installed acceptance

1. Extend acceptance-matrix / installed scripts for:
   - pre-upgrade synthetic canary
   - target-contract attestation receipt
   - restart identity/purge counters
2. Run exact merged-main DMG against authorized v29 APFS/COW.
3. Freeze then soak only after F passes.

## Module homes

| Concern | Home |
|---|---|
| Orchestration | `crates/daemon/src/upgrade_coordinator.rs` |
| Transition/campaign store | `crates/meta-store/src/processing_contract_transition.rs` (+ campaign helper) |
| Delta planner | `crates/meta-store/src/contract_delta.rs` or import-pipeline sibling |
| WriterHealth | `crates/daemon-contract/src/health.rs` |
| Hard-cut API | retain in `import_processing_store.rs` for rebuild-only |

## Verification

Per slice focused tests first. Broad gates before installed acceptance:

```text
cargo test -p meta-store --locked
cargo test -p daemon-contract --locked
cargo test -p resume-daemon --locked
cargo test -p resume-cli --locked --test <focused>
./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
```

Installed acceptance and soak remain macOS-only and private-witness bound.

## Delivery discipline

- Do not open B before A is landed.
- Do not open D before C lands WriterHealth semantics.
- B dormant and C behavior-equivalent keep intermediate mains publishable.
- If C+D must merge, declare an explicit Scope Exception; never land D before C.

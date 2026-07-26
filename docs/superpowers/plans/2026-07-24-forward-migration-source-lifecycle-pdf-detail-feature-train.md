# Forward Migration, Source Lifecycle, PDF And Detail Feature Train Plan

This plan implements the linked 2026-07-24 feature-train specification. One
execution owner advances the six business versions in dependency order without
intermediate tests, builds, installers, Linux workflows or delivery pauses.
Validation starts only after every production, schema, contract, packaging and
UI change through v0.1.8 is complete.

## P0 — checkpoint and delivery foundation

1. Preserve the S810 tree as its own verified checkpoint.
2. Point active-goal and machine contracts at this spec and plan.
3. Make `apps/desktop/package.json.version` the product version authority;
   reference it from Tauri and derive all build/install evidence from it.
4. Add a feature-train ledger whose rows contain command, behavior boundary,
   input fingerprint, result, invalidating change and installed evidence.
5. Keep #217 as the umbrella record; do not push or trigger remote workflows
   while the atomic business implementation is still in progress.

## v0.1.3 — schema v30 migration slice

1. Record the v0.1.3 issue/branch boundary and bump the canonical version;
   describe registry, crash-point and byte-preservation cells in the deferred
   final ledger without executing them.
2. Introduce the private migration registry and COW staging/receipt owner
   modules; keep large meta-store and daemon orchestration files thin.
3. Implement v29→v30 manifest/history tables, exact migration recovery and a
   separate `resume-ir.metadata-initialization-receipt.v1` for crash-safe first
   publication of a fresh current store.
4. Version discovery/status/diagnostics/aggregate contracts atomically across
   daemon, CLI, Tauri and TypeScript consumers.
5. Record the intended migration/contract/packaging cells in the final ledger;
   defer execution until all six versions are implemented.

## v0.1.4 — schema v31 source truth slice

1. Add source-root, occurrence, revision and scan-snapshot schema/types.
2. Migrate and retire the desktop managed-root ledger in one validated
   transaction.
3. Route watcher, debounce, periodic and manual triggers through one per-root
   coordinator; atomically commit the task head with its scan snapshot and keep
   current root counts separate from historical progress and ETA.
4. Implement path-truth publication/deletion semantics and zero-change no-op.
5. Add the per-root progress UI and single start/rescan button; record the
   rename/move/delete/offline states for the final matrix without running them.

## v0.1.5 — schema v32 root deletion slice

1. Add deletion receipt/state and claim fences; record crash-recovery coverage
   in the deferred final ledger.
2. Publish search removal before physical cleanup and prove no half-delete.
3. Clean root-owned records and unreferenced artifacts transactionally; destroy
   a predecessor that contains deleted data.
4. Add bounded confirmation and progress UI. Add source-hash preservation and
   crash-recovery cases to the deferred final matrix.

## v0.1.6 — schema v33 PDFium/OCR slice

1. Freeze a reviewed PDFium source/build/runtime-pack contract for macOS and
   Windows. Package its license, source contract and GN arguments atomically,
   require bundle-composition v4 and DMG-composition v4 to bind all four
   runtime packs, and record tamper/package coverage in the deferred final
   ledger.
2. Add deferred PDF text-quality fixtures and replace production lopdf
   extraction.
3. Persist OCR page checkpoints and resumable low-priority serial scheduling.
4. Add reprocessing and cancellation semantics plus runtime/capability contract
   versions.
5. Add CJK, invisible/cropped/transparent/garbled inputs, restart resume,
   deletion cancellation and unchanged search-latency cases to the deferred
   final matrix.

## v0.1.7 — preview/detail slice

1. Extract detail drawer state/view modules and add resizable accessible width.
2. Add source-file authority and preview lease/range/close contracts; record
   bounded range and TTL coverage in the deferred final ledger.
3. Bundle a fixed PDF.js build and render only visible pages through range
   transport.
4. Add stale selection, wrong generation/hash, range overflow, window scope,
   lease close and zero unopened-preview import-cost cases to the deferred
   final matrix.

## v0.1.8 — reveal slice

1. Reuse source-file authority for a selection-only Tauri command.
2. Add the Rust opener plugin without JS guest permissions or a generic path
   command.
3. Record missing/replaced/symlink/reparse/unauthorized source and bounded-error
   cases in the deferred final ledger.
4. Add an installed Finder selection case using a synthetic file to the
   deferred final matrix.

## Final delivery

Freeze the complete business tree and run the macOS delivery matrix once,
continuing through failures so they can be analyzed together. Resume from the
immutable ledger; after repair, rerun only failed, fingerprint-invalidated and
previously unrun cells. Then run the public guard, release Tauri build, exact
DMG, installed-main APFS/COW acceptance, final Computer Use and the
uninterrupted 120-minute soak. No Linux lane runs. Reconcile the version history
into #217 only after those gates are complete.

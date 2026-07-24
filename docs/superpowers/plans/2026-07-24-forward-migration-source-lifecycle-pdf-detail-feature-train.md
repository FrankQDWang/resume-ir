# Forward Migration, Source Lifecycle, PDF And Detail Feature Train Plan

This plan implements the linked 2026-07-24 feature-train specification. One
execution owner advances one version at a time. A later version cannot start
until the prior issue has focused evidence, exact-commit DMG, installed manual
acceptance, completion comment and closure.

## P0 — checkpoint and delivery foundation

1. Preserve the S810 tree as its own verified checkpoint.
2. Point active-goal and machine contracts at this spec and plan.
3. Make `apps/desktop/package.json.version` the product version authority;
   reference it from Tauri and derive all build/install evidence from it.
4. Add a feature-train ledger whose rows contain command, behavior boundary,
   input fingerprint, result, invalidating change and installed evidence.
5. Keep #217 open as umbrella and open no feature issue until P0 passes its
   focused contract/version checks.

## v0.1.3 — schema v30 migration slice

1. Open the v0.1.3 issue and branch, bump the canonical version, and write
   failing registry, crash-point and byte-preservation tests.
2. Introduce the private migration registry and COW staging/receipt owner
   modules; keep large meta-store and daemon orchestration files thin.
3. Implement v29→v30 manifest/history tables and exact recovery policy.
4. Version discovery/status/diagnostics/aggregate contracts atomically across
   daemon, CLI, Tauri and TypeScript consumers.
5. Run only migration/contract/affected packaging cells, build and install the
   exact DMG, exercise direct v29→v30 and clean v30, record evidence, merge and
   close the issue.

## v0.1.4 — schema v31 source truth slice

1. Add source-root, occurrence, revision and scan-snapshot schema/types.
2. Migrate and retire the desktop managed-root ledger in one validated
   transaction.
3. Route watcher, debounce, periodic and manual triggers through one per-root
   coordinator with trustworthy completeness and ETA.
4. Implement path-truth publication/deletion semantics and zero-change no-op.
5. Add the per-root progress UI and single start/rescan button, then run focused
   source/coordinator/UI tests and installed rename/move/delete/offline states.

## v0.1.5 — schema v32 root deletion slice

1. Add deletion receipt/state, claim fences and crash recovery tests.
2. Publish search removal before physical cleanup and prove no half-delete.
3. Clean root-owned records and unreferenced artifacts transactionally; destroy
   a predecessor that contains deleted data.
4. Add bounded confirmation and progress UI. Verify source hashes are unchanged,
   install the exact DMG, reconcile and close.

## v0.1.6 — schema v33 PDFium/OCR slice

1. Freeze a reviewed PDFium source/build/runtime-pack contract for macOS and
   Windows and add tamper/package tests.
2. Add PDF text quality fixtures and replace production lopdf extraction.
3. Persist OCR page checkpoints and resumable low-priority serial scheduling.
4. Add reprocessing and cancellation semantics plus runtime/capability contract
   versions.
5. Verify CJK, invisible/cropped/transparent/garbled inputs, restart resume,
   deletion cancellation and unchanged search latency before installed
   acceptance.

## v0.1.7 — preview/detail slice

1. Extract detail drawer state/view modules and add resizable accessible width.
2. Add source-file authority and preview lease/range/close contracts with
   bounded range and TTL tests.
3. Bundle a fixed PDF.js build and render only visible pages through range
   transport.
4. Verify stale selection, wrong generation/hash, range overflow, window scope,
   lease close and zero unopened-preview import cost; install and accept.

## v0.1.8 — reveal slice

1. Reuse source-file authority for a selection-only Tauri command.
2. Add the Rust opener plugin without JS guest permissions or a generic path
   command.
3. Test missing/replaced/symlink/reparse/unauthorized sources and bounded error
   projection.
4. Install the exact DMG and use Finder to prove a synthetic file is selected.

## Final delivery

Freeze merged main and run `verify-local --parallel` once. Resume from the
immutable ledger after failures; rerun only failed or fingerprint-invalidated
cells. Then run public guard, release Tauri build, exact merged-main DMG,
installed-main APFS/COW acceptance, final Computer Use and the uninterrupted
120-minute soak. Reconcile all six issues into #217 and close #217 only when
those gates are complete.

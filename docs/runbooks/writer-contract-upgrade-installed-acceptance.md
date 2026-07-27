# Writer-contract upgrade installed acceptance (Slice F)

Private macOS-only gate. Requires an authorized v29 APFS/COW clone and an exact
merged-main DMG. Do not commit private paths, canary text, or digests.

## Preconditions

1. Exact clean `main` commit equals the DMG receipt commit.
2. Authorized `$RESUME_IR_INSTALLED_ACCEPTANCE_V29_ROOT` points at a preserved
   v29 authority (or the repo's configured private root equivalent).
3. No pre-existing App/daemon for this product generation.

## Phase 0 — Pre-upgrade canary (v29 writer)

Using a **v29-capable** binary against the COW clone only:

1. Import a synthetic legacy canary with a unique public token.
2. Confirm keyword search and detail against that canary.
3. Record generation + visible_epoch into a path-redacted private receipt.

## Phase 1 — Cold start on current DMG

1. Install exact current DMG into an isolated HOME.
2. Launch once; keep the same daemon generation alive.
3. Assert schema current = 34, four runtimes available.
4. **Before** PDF reprocessing completes: keyword search finds the pre-upgrade
   canary; detail reads the original selection; generation is not replaced by
   an empty authority.
5. Status `writer` shows target committed / writer ready (or transitioning with
   opaque transition id); keyword/detail remain available.

## Phase 2 — Target-contract attestation

1. Import a new synthetic canary after WriterReady.
2. Private attestation receipt must prove:
   - import task binding = target contract
   - completion binding = target contract
   - publication includes the canary
   - transition id matches status opaque id
3. “Searchable alone” is insufficient.

## Phase 3 — Restart idempotence

1. Quit and relaunch.
2. Assert:
   - target commit count unchanged
   - transition id unchanged
   - hard-cut / task-purge counters = 0
   - queued/task identity aggregates not wholesale cleared
3. Pre-upgrade and new canaries remain searchable.

Only after Phases 0–3 pass may the acceptance commit be frozen for the
120-minute soak.

# Daemon Bootstrap And Capability Hard Cut Implementation Plan

This plan implements the linked hard-cut specification under issue #217. It is
one correctness train with a scope exception, but each ordered slice must have
its own deterministic red/green evidence. No intermediate state is a release
candidate and no compatibility reader may be introduced to make a slice pass.

## Ordered slices

1. **B0 contract freeze**: switch the active spec/plan pointers; split v29 data
   correctness from bootstrap, lifecycle and capability acceptance; update the
   linked issue and machine guards before production edits.
2. **B1 v29-only owner open**: replace the runtime v28-to-v29 entry with exact
   existing-v29 validation plus empty-only current-v29 creation. Read existing
   keys without repair, reject every legacy authority without mutation, remove
   production migration/predecessor cleanup, and add byte-preservation tests.
3. **B2 launch-bound early control plane**: hard-cut discovery/auth to v3,
   require a desktop launch id, make the generation owner retain an `Arc` owner
   capability, bind and publish before store/runtime initialization, and serve
   authenticated in-memory status/diagnostics while a resident startup worker
   prepares the data plane.
4. **B3 runtime capability isolation**: move pack validation behind control
   publication, add the fixed runtime and operation-capability matrices, gate
   worker claim/publication before mutation, and preserve existing
   keyword/detail plus lexical hybrid behavior under optional-runtime faults.
   Build release sidecars before the daemon, compile their exact role/name/
   target and signature-neutral Mach-O payload digests into the daemon, and
   revalidate canonical executable identity immediately before spawn. Do not
   accept a caller-supplied or self-consistent adjacent manifest as authority.
5. **B4 supervisor/lifecycle hard cut**: delete pre-spawn probing and the
   persistent restart ledger, probe only through the launch-bound child, expose
   lifecycle v2, keep the fixed policy in process-local monotonic state, and
   make blocked/circuit retry explicit and typed.
6. **B5 bridge, UI and diagnostics**: hard-cut native/TypeScript projections to
   status v3, diagnostics v4, error v2 and lifecycle v2; revoke stale action
   authority on bridge/status failures; expose process/core/capability state and
   daemon-independent combined diagnostics.
7. **B6 installed acceptance and cleanup**: convert the macOS gate from v28
   migration to exact-v29 preservation, exercise stale/foreign discovery,
   strong kill, slow initialization and runtime fault injection, remove dead
   migration/ledger/contract code, then run broad, privacy, release, installed
   and frozen-soak gates.

## Implementation boundaries

- `main.rs` orchestrates only. Resident startup, runtime capabilities and
  status snapshots have separate modules and bounded messages.
- The control-plane thread owns the listener and current route state. A prepared
  runtime moves store/search handles into that owner; stores are not placed in
  `Arc<Mutex<_>>`.
- Status/diagnostics updates use bounded channels and cached typed projections.
  Worker terminal signals are not dropped or conflated with best-effort metric
  refreshes.
- A child owns its expected launch id and performs startup/heartbeat probes.
  Runtime-wide pre-probe is removed.
- Optional runtime errors are closed classifications. Raw paths, digests,
  manifests and subprocess text never enter IPC or desktop diagnostics.
- Existing search selection, immutable version and atomic publication logic is
  unchanged except for capability gating before new mutation.

## Required red/green commands

Focused commands are selected by each slice. The final train must include:

```text
cargo test -p meta-store --locked
cargo test -p resume-daemon --locked
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml --locked
npm test --prefix apps/desktop
npm run build --prefix apps/desktop
python3 scripts/ci/check-performance-contracts.py
python3 scripts/ci/check-autonomous-goal.py
./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
```

The target-platform release build and installed-main acceptance run only after
the focused and broad non-install gates pass. The two-hour soak starts only
after an exact installed build passes and restarts from zero after any deployed
regression.

## Delivery

One execution owner integrates the train. Storage and contract preparation may
land as bounded independent commits. Discovery/status/error/lifecycle producer
and consumer changes share one atomic merge boundary. The two existing
untracked research documents remain untouched. This scope exception cannot
auto-merge and cannot claim Windows native, signing, updater, scale or stable
release completion.

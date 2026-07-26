# Daemon Bootstrap And Capability Hard Cut

## Status

Approved for implementation on 2026-07-21 under issue #217. This contract
supersedes the bootstrap, desktop restart-ledger, optional-runtime and
v28-to-v29 installed-acceptance clauses in the 2026-07-17 v29 corrective
specification. Its immutable selection and atomic v29 publication invariants
remain authoritative.

This is a hard cut. There is no legacy discovery reader, restart-ledger reader,
runtime migration from a persisted pre-v29 store, dual write, compatibility
flag, request replay or doc-only detail fallback.

## Confirmed failure chain

1. Desktop preflight reads stale discovery/auth before spawning the child and
   maps every structured failure to `protocol_mismatch`, so the daemon that
   owns stale-file cleanup is never started.
2. A persistent restart-window JSON can block startup before spawn, while the
   UI retry command does nothing for `blocked`.
3. Release desktop validates embedding, OCR and classifier resources before
   spawning the daemon, so one optional pack removes status, keyword search,
   existing detail and diagnostics.
4. The ten-second supervisor deadline currently includes resident embedding,
   v29 open/migration, processing-contract activation and task normalization.
5. React retains an initial or previously trusted lifecycle snapshot when the
   Tauri bridge or strict decoder fails, presenting unrelated failures as
   `starting` or permitting stale Ready actions.
6. Post-spawn probing is not bound to the child being supervised and can
   temporarily adopt a still-reachable foreign discovery endpoint.

## Required architecture

### Storage authority

- Metadata schema remains exactly 29.
- An existing active store is opened only when its manifest, key, schema and
  current-v29 invariants validate. Opening it must not create or repair a key.
- A fresh v29 store may be created only when no manifest, metadata key, legacy
  database or migration authority exists.
- Persisted v27, v28 and unknown authorities fail with a typed unsupported
  schema result. Their bytes are not copied, upgraded, deleted or quarantined.
- The production v28-to-v29 migrator and predecessor cleanup are not reachable
  from owner open. Empty-store construction may reuse vetted DDL fragments only
  behind an initializer that proves the input database has schema version zero.

### Process and discovery authority

- The desktop supervisor generates a fresh 256-bit lowercase-hex `launch_id`
  for every spawn. Desktop-supervised daemon startup requires it.
- Discovery `resume-ir.daemon-ipc.v3` and auth
  `resume-ir.daemon-auth.v3` carry the same launch id, daemon-generated instance
  id and generation token.
- A business connection is bound to supervisor generation, launch id, instance
  id, address and token. Generation changes are never replayed.
- There is no pre-spawn discovery probe. A newly spawned child is the only
  process allowed to establish ownership. A live owner produces a typed
  ownership conflict; the supervisor never attaches to or kills it.
- A child probe accepts only the expected launch. Foreign discovery is treated
  as unavailable during startup. A protocol error is sticky only after it is
  attributable to the expected launch.
- After exclusive data-directory ownership, regular stale discovery/auth files
  are removed without parsing. Links, directories and other unsafe objects are
  not followed or removed and fail closed.

### Early authenticated control plane

The startup order is fixed:

1. validate bounded CLI shape;
2. acquire the data-directory owner;
3. acquire the launch-bound generation owner;
4. bind loopback and atomically publish discovery/auth;
5. enter the non-blocking authenticated control loop;
6. initialize current v29 storage, processing state, repair and optional
   runtimes in a resident startup worker;
7. activate data services on the same listener, instance and token.

The ten-second deadline covers steps 2-5 only. A valid authenticated status
response proves process readiness even when core services are initializing.
Status and diagnostics are rendered from a bounded in-memory snapshot and do
not read SQLite on the heartbeat path.

During initialization, authenticated status and diagnostics return 200. Every
business route returns a bounded `SERVICE_INITIALIZING` 503 without touching
storage. A persistent core failure leaves the control plane readable with
`core.state=blocked` and a closed redacted reason.

## Hard-cut public contracts

- discovery `resume-ir.daemon-ipc.v3`;
- auth `resume-ir.daemon-auth.v3`;
- status `daemon.status.v3`;
- diagnostics `resume-ir.diagnostics.v4`;
- unified error `resume-ir.error.v2`;
- desktop lifecycle `resume-ir.desktop-daemon-lifecycle.v2`;
- lifecycle receipt and desktop diagnostics v2;
- aggregate IPC `resume-ir.ipc.v4`;
- search/detail/hydrate success contracts remain v3;
- metadata remains v29.

`daemon.status.v3` has a fixed process/core/runtime/capability shape. Optional
runtimes are exactly embedding, OCR and classifier. Operation capabilities are
exactly keyword search, detail, semantic search, hybrid search, text import,
OCR import and index publication. Unknown fields and inconsistent state/reason
combinations are rejected.

`resume-ir.error.v2` adds `SERVICE_INITIALIZING`, `SERVICE_BLOCKED` and
`CAPABILITY_UNAVAILABLE` with closed action, capability and reason enums.
`SEMANTIC_DISABLED` continues to mean the product/storage contract disables
semantic search; it does not represent a missing runtime.

## Optional-runtime behavior

- Runtime pack validation happens in the daemon after control-plane
  publication. The desktop resolves only the daemon binary and bounded resource
  roots.
- A release-built embedding sidecar or PDF renderer is executable only when its
  canonical path has the exact reviewed role, runtime basename and target, and
  its Mach-O payload matches an identity compiled into that daemon generation.
  The payload digest uses the repository's
  `sha256_without_code_signature_v1` canonicalization so later inner signing
  may change only the code-signature blob; code, data, load-command, append or
  truncation drift remains invalid. A runtime-supplied or adjacent
  self-describing manifest is never a trust root. The final outer bundle
  composition and signature gates remain mandatory. Targets without an
  implemented reviewed executable-identity canonicalizer fail closed.
- Missing, invalid or failed embedding leaves current keyword/detail readable.
  Semantic search is unavailable; hybrid returns lexical results with a bounded
  partial reason. When the active publication contract requires vector output,
  import/index mutation is blocked before task claim.
- Missing or invalid OCR leaves text processing available when its other
  capabilities are satisfied. Scanned inputs remain unclaimed in the OCR
  backlog.
- Missing or invalid classifier preserves the active query generation and
  classifier epoch. New import, reclassification and OCR publication are
  blocked before claim; no implicit fallback epoch is activated.
- Installed runtime repair is recognized by a new daemon generation. Packaged
  builds remain fail-closed when required resources are absent or invalid.

## Desktop lifecycle and observability

Lifecycle v2 states are `starting`, `running`, `retry_wait`, `circuit_open` and
`blocked`. It carries one state-consistent transition reason, generation,
automatic restart attempt/limit, retry delay, heartbeat failures and last exit.
It has no restart-ledger fields.

Restart history is process-local monotonic state: five automatic restarts per
ten minutes, backoff 250 ms/1 s/4 s/15 s/30 s, five-minute circuit, and reset
after five continuous running minutes. App restart starts a new budget.
`blocked` retry performs a fresh launch-bound spawn; `circuit_open` permits one
half-open attempt; other states return `retry_not_allowed`.

The legacy restart ledger is never read or recreated. An exact regular legacy
file may be removed best-effort without parsing; all other outcomes are inert.
Lifecycle receipts are diagnostic output only and never restore policy.

React models lifecycle bridge readability, process lifecycle, core service,
optional runtime capability and result freshness independently. A bridge or
decoder failure revokes the last snapshot's action authority. Combined desktop
diagnostics remain exportable when the daemon is absent or blocked.

## Acceptance

- stale v1/v2/malformed discovery with no owner starts a launch-bound v3 daemon;
- a live or wrong-launch owner is never adopted, modified or killed;
- authenticated status is available within ten seconds and a deliberately slow
  core initialization later becomes ready without PID/listener/launch rotation;
- initialization and blocked routes do not access storage;
- exact v29 data, selections, projection/epoch and artifact heads are preserved;
- pre-v29, missing-key and bind failures preserve original bytes;
- legacy ledger content and permissions cannot affect startup;
- each optional runtime fault preserves the capabilities allowed above;
- bridge/status contract failure cannot present stale Ready or Starting;
- native macOS installed acceptance uses an authorized v29 COW copy, followed
  by the frozen 120-minute fault soak; Windows compile evidence is not native
  Windows completion.

All public evidence is bounded and redacted. No path, token, manifest body,
runtime stderr, resume text, raw query, candidate result or private digest may
enter git or GitHub prose.

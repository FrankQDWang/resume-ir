# Forward Migration, Source Lifecycle, PDF And Detail Feature Train

## Status

Approved for implementation on 2026-07-24 under umbrella issue #217. This
contract supersedes the v29-only storage policy in the 2026-07-21 daemon
bootstrap hard cut. The daemon bootstrap, launch binding, lifecycle v2,
selection tuple, atomic publication and privacy invariants remain authoritative
unless this document explicitly versions them.

The train has six ordered product versions delivered as one business
implementation. All production code, schemas, contracts, packaging declarations
and UI work are completed before any test, build, installer or native acceptance
run begins. The train does not run a Linux lane. After v0.1.8 business work is
complete, one resumable macOS matrix runs through every cell without stopping at
the first failure, then repairs the failures as one system and reruns only
failed, invalidated and previously unrun cells. The final merged-main installed
acceptance and 120-minute soak run after that matrix.

## Shared invariants

- `apps/desktop/package.json.version` is the only product version authority.
  Tauri, build plans, package names, install receipts, installed acceptance and
  release evidence derive from that manifest.
- Auth v3, desktop lifecycle v2, search/detail/hydrate v3 and
  `{doc_id, version_id, visible_epoch}` selection semantics do not change.
- There is no dual reader, dual write, downgrade read, request replay or
  compatibility flag.
- Paths, source bytes, tokens, raw system errors, resume text, private hashes
  and candidate results never enter public evidence or WebView state.
- Search remains read-only and never performs parsing, OCR, model inference or
  index publication.
- A focused passing test remains reusable until a later change touches its
  declared input fingerprint or behavior boundary.

## v0.1.3 — schema v30 forward migration

The current-store entry becomes a continuous registry of exact adjacent
migrations. Each entry declares `from`, `to`, `name`, checksum, apply and
validate. A binary may open its exact current schema or run the complete
registered chain beginning at v29. v27, v28, future schemas, missing keys and
damaged authorities fail closed without mutation.

Migration happens only after data-directory ownership:

1. open and validate the encrypted source read-only;
2. copy it to same-key encrypted COW staging;
3. apply each adjacent migration transactionally;
4. validate integrity, business summary, active heads, visible epoch and
   artifact digests;
5. sync the staged payload and atomically switch manifest authority;
6. retain one encrypted predecessor with a bounded migration receipt.

Crash recovery accepts only a fully validated new authority or resumes/cleans
the exact recorded staging operation. Source ciphertext is byte-identical after
every failed or interrupted point. Once the new current store receives writes,
the predecessor is explicit recovery material only and is never selected
automatically.

Creating the first empty current store uses the separate bounded
`resume-ir.metadata-initialization-receipt.v1`. It publishes neither key nor
manifest until that binary's current encrypted target has been fully
initialized, synced and validated (v30 in v0.1.3 and v33 after the complete
train). A crash can therefore only resume publication of that exact target or
remove the exact unpublished staging/key files; it cannot leave a partial
authority that permanently blocks first launch.

Discovery v5, status v5, diagnostics v9 and aggregate IPC v6 expose a bounded
`core=migrating` state after early control-plane publication. Business routes
return typed initializing responses and do not touch the unready store.

## v0.1.4 — schema v31 source-root truth

Daemon/meta-store is the sole durable source-root authority. The prior desktop
managed-root ledger is validated once, imported into the database and retired.
Tauri only performs native directory selection.

The domain is:

- `SourceRootId`: stable opaque identity, never a path;
- `SourceOccurrenceId`: root id plus normalized relative path;
- `SourceRevision`: immutable content revision of one occurrence;
- `ScanSnapshot`: trigger, phase, counts, completeness, rate and ETA evidence.

Path identity is authoritative. A rename or move deletes the old occurrence and
imports a new one. A removed file leaves the active projection. Same-path
content change creates a new revision and atomically replaces the old result.
Content hashes may reuse parsing/OCR/embedding artifacts but never preserve
business identity across paths. A zero-change scan produces no new version or
epoch.

Watcher events, debounce, the 300-second safety scan and manual scan share one
per-root coordinator. A root has at most one active scan and duplicate requests
coalesce. The canonical import-task head and its scan snapshot are committed in
one SQLite transaction, so a worker can never complete an unobservable scan;
orphaned active snapshots are closed before a replacement starts. Absence may
delete indexed occurrences only after a complete, in-budget directory
enumeration while the root is online. A per-file parse/classification failure
is counted but does not make the observed directory listing untrustworthy.
Offline, permission-denied, enumeration-error or partial scans report unknown
and retain the last trusted projection.

The final `source-roots.v2` projection separates current root counts from the
last scan's historical progress, so migrated data remains visible before its
first post-migration scan. Discovery v5, diagnostics v9 and aggregate IPC v6
expose the bounded endpoints and aggregates. The UI retains global totals and
gives each root a single button: “开始扫描”, then “重新扫描”, and “扫描中” while
active. It retains pause/resume monitoring and shows discovered, searchable,
non-resume, needs-review, OCR, failed, ignored, phase, progress, last sync,
watcher state and evidence-backed ETA.

## v0.1.5 — schema v32 root deletion

“删除目录及本地数据” never deletes source files. It uses a durable deletion
state machine:

1. mark the root deleting and fence watcher, scan and worker claims;
2. cancel or boundedly drain active work;
3. atomically publish a projection with the root removed;
4. remove its occurrences, revisions, classifications, OCR work/cache,
   embeddings, tasks and unreferenced content-addressed artifacts;
5. remove root authority;
6. complete only after a residual scan proves zero active references.

The operation resumes after a crash and cannot hide the root card while leaving
searchable results. Reference-counted artifacts survive while another active
root uses them. Privacy deletion destroys any retained predecessor containing
the deleted root before reporting completion. `source-roots.v2`, deletion
receipt v1 and diagnostics v7 are closed contracts.

After durable requested commits, IPC returns 202 before background deletion.
Until completion, closed reason `source_root_deleting` fences that path while
other roots/control remain available. Typed failures retry the receipt with
bounded backoff and no non-fatal stderr; panics are not recoverable.

## v0.1.6 — schema v33 PDFium and recoverable OCR

A reviewed, statically linked PDFium runtime becomes the only production PDF
text-object interpreter and OCR page renderer on macOS and Windows. The
production lopdf extraction and platform-split render paths are removed.

PDFium text passes application quality gates for render mode, alpha, CropBox,
geometry, Unicode, controls, repeated/high-entropy strings and invisible
overlays before classification. Rejected PDF text enters the persistent OCR
queue. OCR runs globally at concurrency one and low priority, checkpoints each
page in the existing cache, resumes after restart, cancels when the source
revision disappears and republishes only a fully reclassified
`resume_candidate`.

Parser-contract changes enqueue old PDFs for low-priority reprocessing without
blocking startup or existing search. Status v5, diagnostics v8 and error v3 add
the `pdfium` runtime and `pdf_import` capability. Release packaging must contain
the reviewed fourth runtime pack even though runtime degradation remains
observable. The installed pack contains the exact reviewed PDFium source
contract, Chromium license and GN arguments; the static archive is linked into
the daemon and renderer rather than duplicated in Resources. Bundle composition
v4 and DMG composition v4 both fail closed unless all four pack manifests and
their bound payloads are present.

## v0.1.7 — original PDF detail

The detail drawer becomes an independent module with pointer drag, keyboard
width adjustment and reset. PDF defaults to “原始简历”; structured filter fields
and extracted text remain auxiliary views. Field confidence supports filters
and explanation, not ordinary keyword or hybrid ranking.

`source-preview.v1` is a create/read-range/close lease contract. The daemon
accepts only the current selection, opens the exact source file, verifies size
and SHA-256 on the same handle, and issues a short opaque lease bound to
generation and selection. A bundled fixed PDF.js reads bounded 64 KiB ranges
and renders only visible pages with automatic streaming prefetch disabled.
Path, hash, lease secret and PDF bytes do not become React state, logs or
diagnostics. Missing/changed/stale sources disable preview while the published
structured detail remains visible. Diagnostics advances to v9.

## v0.1.8 — reveal source in the operating system

Every supported source format offers “在访达中显示” or “在文件资源管理器中显示”.
The WebView sends only the current selection. Shared `SourceFileAuthority`
revalidates the active revision, authorized root, normalized relative path,
regular-file type, symlink/reparse-point boundary, size and hash.

Rust invokes `tauri_plugin_opener::reveal_item_in_dir()` directly. No JS opener
guest, `opener:*` capability or generic open-path command exists. The closed
`source-reveal.v1` result is `revealed` or a bounded typed error. Missing,
renamed, replaced or unauthorized files fail closed until a watcher-created new
selection exists.

## GitHub and acceptance

#217 remains the durable umbrella record for the train. Version boundaries,
contracts, privacy boundaries and rollback policy remain explicit in the
history, but issue/PR delivery must not interrupt the atomic business
implementation or trigger remote Linux workflows before the product work is
complete.

After the complete v0.1.8 business tree is frozen, one resumable parallel
macOS matrix runs all required focused and delivery cells and records every
failure before repair begins. A repair invalidates only the cells whose input
or behavior boundary changed; all other passing evidence remains reusable.
Then run the public guard, release Tauri build, exact DMG, APFS/COW installed
acceptance, final Computer Use pass and a 120-minute fault soak. Windows remains
compile/contract/package scope only and is not native release evidence. No Linux
lane is part of this product train. #217 closes only after the final evidence is
reconciled.

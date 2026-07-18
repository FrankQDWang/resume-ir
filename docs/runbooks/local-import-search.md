# Local Import and Search Runbook

## Scope

Local-only operator workflow for proving the current `resume-ir` product loop:
discover local Word/PDF resumes, import them into the local engine, keep the
corpus incrementally managed, and run keyword, field, semantic, and hybrid
search from the local indexes.

Do not upload real resumes, raw query sets, local data directories, SQLite
databases, indexes, diagnostics packages, model caches, runtime binaries,
runtime manifests with local paths, or logs from private runs. Synthetic fixtures
are the only public reproduction input.

## Current-stage closure

Current-stage closure means the local import/search loop is usable and
reproducible with redacted evidence. Stable release remains blocked,
performance optimization is deferred, and this stage does not clear private
quality labeling, signing, notarization, platform lifecycle, model licensing,
or actual hardware fault drill blockers.

Do not chase P95/P99 in this stage. Do not require million-resume validation in
this stage. If a private local corpus is OCR-heavy or slow, record a redacted
aggregate BLOCKED handoff and move latency work to the follow-up performance
goal.

## Local paths

Use separate local directories for source resumes, engine data, and evidence:

```bash
resume_root="<local-private-resume-root>"
data_dir="<local-private-data-dir>"
out_dir="<local-private-evidence-dir>"
```

Keep all three outside the repository. Never commit these directories or any
file generated under them.

## Runtime preflight

OCR and semantic search require reviewed local runtimes. Run preflight before
private corpus access when the goal is a complete local validation flow:

```bash
resume-cli --data-dir "$data_dir" ocr preflight --json --ocr-lang eng
```

If tools are outside `PATH`, pin reviewed commands explicitly:

```bash
resume-cli --data-dir "$data_dir" ocr preflight --json \
  --ocr-lang eng \
  --tesseract-command "<local-tesseract-command>" \
  --pdftoppm-command "<local-pdftoppm-command>"
```

Embedding preflight must use a real local embedding command and a reviewed
model manifest:

```bash
resume-cli --data-dir "$data_dir" model preflight --json \
  --manifest "<local-model-manifest.json>" \
  --embedding-command "<local-embedding-command>" \
  --model-id "<reviewed-model-id>" \
  --dimension "<dimension>"
```

If OCR or embedding preflight fails, stop before importing private data for a
validation run and classify the run as OCR or embedding BLOCKED. Do not replace
these preflights with fake embedders or fake OCR outputs.

## Import

Specified-directory import is the normal path. The same command supports broad
scans by choosing a broad root such as a drive, volume, or home directory:

```bash
resume-cli --data-dir "$data_dir" import --root "$resume_root" --profile explicit
```

For discovery-style broad scans, use the discovery profile or preset:

```bash
resume-cli --data-dir "$data_dir" import --root "$resume_root" --profile discovery
resume-cli --data-dir "$data_dir" import --root-preset local-discovery --profile discovery
```

The `local-discovery` preset uses the local discovery roots configured by the
operator environment and should be treated as a convenience for whole-machine or
drive-level discovery. It is not a different product feature from root import.

Use bounded imports for large private corpora during investigation:

```bash
resume-cli --data-dir "$data_dir" import --root "$resume_root" --profile discovery --max-files 10000
```

## Incremental management

For day-to-day use, run the daemon in foreground while a platform service or UI
is not yet the primary operator surface:

Canonical import watcher prefix:
`resume-daemon --data-dir "$data_dir" run --foreground --work-imports --watch-import-roots`.

```bash
resume-daemon --data-dir "$data_dir" run --foreground \
  --work-imports \
  --watch-import-roots \
  --rescan-completed-imports \
  --work-ocr \
  --work-index \
  --ocr-lang eng \
  --embedding-command "<local-embedding-command>" \
  --embedding-model-id "<reviewed-model-id>" \
  --embedding-dimension "<dimension>"
```

`--watch-import-roots` requeues completed roots when local filesystem changes
are observed. `--rescan-completed-imports` gives a periodic safety net for
missed events. Existing imported documents are managed by content/text hashes,
document status, deleted flags, and index snapshots; rerunning import should not
be treated as a destructive rebuild.

Inspect local aggregate state without printing private paths:

```bash
resume-cli --data-dir "$data_dir" status --watch-import
resume-cli --data-dir "$data_dir" doctor
```

## Search

Run full-text search first to prove basic keyword retrieval:

```bash
resume-cli --data-dir "$data_dir" search "java backend" --mode fulltext --top-k 10
```

Run field-filtered search when structured extraction has produced high
confidence fields:

```bash
resume-cli --data-dir "$data_dir" search "platform engineer" \
  --mode fulltext \
  --skills-any "rust,java,postgres" \
  --years-experience-min 5 \
  --top-k 20
```

Run semantic search only after embedding preflight and an enabled atomic vector
snapshot are confirmed:

```bash
resume-cli --data-dir "$data_dir" search "distributed systems engineer" \
  --mode semantic \
  --embedding-command "<local-embedding-command>" \
  --model-id "<reviewed-model-id>" \
  --dimension "<dimension>" \
  --top-k 10
```

Run hybrid search for normal use after full-text and vector indexes are both
available:

```bash
resume-cli --data-dir "$data_dir" search "payments java platform" \
  --mode hybrid \
  --embedding-command "<local-embedding-command>" \
  --model-id "<reviewed-model-id>" \
  --dimension "<dimension>" \
  --skills-any "java,kafka,payment" \
  --top-k 10
```

Open a result detail by document id:

```bash
resume-cli --data-dir "$data_dir" detail --doc-id "<doc-id>"
```

Delete and purge are local management operations. They must keep indexes and
metadata consistent:

```bash
resume-cli --data-dir "$data_dir" delete --doc-id "<doc-id>"
resume-cli --data-dir "$data_dir" purge --deleted
```

## Redacted evidence

Diagnostics for handoff must be redacted and local:

```bash
resume-cli --data-dir "$data_dir" export-diagnostics --redact > "$out_dir/redacted-diagnostics.json"
```

Use the current-stage orchestrator for a reproducible local plan:

```bash
scripts/local/run-current-stage-validation.sh --dry-run \
  --resume-root "$resume_root" \
  --data-dir "$data_dir" \
  --out-dir "$out_dir" \
  [--query-set-trace-root "$RESUME_IR_QUERY_ARTIFACT_ROOT"] \
  --model-manifest "<local-model-manifest.json>" \
  --ocr-runtime-manifest "<local-ocr-runtime-manifest.json>" \
  --model-artifact "<local-model-artifact>" \
  --embedding-command "<local-embedding-command>" \
  --model-pack-id "<reviewed-model-pack-id>" \
  --model-id "<reviewed-model-id>" \
  --model-format "<model-format>" \
  --dimension "<dimension>" \
  --model-license "<model-license-id>" \
  --runtime-pack-id "<reviewed-runtime-pack-id>" \
  --language eng \
  --language-pack "<local-tessdata-file>" \
  --engine-license Apache-2.0 \
  --renderer-license "<installed-renderer-license>" \
  --language-license Apache-2.0
```

Use execute only when the operator explicitly authorizes local private corpus
access and local runtime artifacts are reviewed:

```bash
scripts/local/run-current-stage-validation.sh --execute \
  --validation-profile smoke \
  --resume-root "$resume_root" \
  --data-dir "$data_dir" \
  --out-dir "$out_dir" \
  [--query-set-trace-root "$RESUME_IR_QUERY_ARTIFACT_ROOT"] \
  --model-manifest "<local-model-manifest.json>" \
  --ocr-runtime-manifest "<local-ocr-runtime-manifest.json>" \
  --model-artifact "<local-model-artifact>" \
  --embedding-command "<local-embedding-command>" \
  --model-pack-id "<reviewed-model-pack-id>" \
  --model-id "<reviewed-model-id>" \
  --model-format "<model-format>" \
  --dimension "<dimension>" \
  --model-license "<model-license-id>" \
  --runtime-pack-id "<reviewed-runtime-pack-id>" \
  --language eng \
  --language-pack "<local-tessdata-file>" \
  --engine-license Apache-2.0 \
  --renderer-license "<installed-renderer-license>" \
  --language-license Apache-2.0 \
  --reviewed-model \
  --reviewed-ocr-runtime \
  --max-files 10000 \
  --max-queries 20 \
  --ocr-worker-ticks 1
```

The smoke profile proves wiring only. The full profile is for later evidence
when OCR/model licensing and local runtime review are complete and the operator
has time to run the private corpus flow. Neither profile permits committing or
uploading private outputs.

## Completion report

When closing this stage, report:

- import root mode used: explicit root, discovery root, or local-discovery;
- document counts and status counts from redacted aggregate output only;
- whether OCR preflight `runtime_probe` passed, failed, or was not run;
- whether embedding preflight `embedding_protocol` passed, failed, or was not run;
- which search modes were exercised: fulltext, field-filtered, semantic, hybrid;
- BLOCKED items: OCR backlog, embedding model review, private quality labels,
  signing/notarization, platform lifecycle, performance baseline, UI/manual
  testing;
- statement that performance optimization is deferred and million-resume
  validation is not a current-stage gate.

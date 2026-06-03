# OCR and Embedding Worker Runbook

## Scope

Local-only runbook for the configured OCR and embedding command workers. Do not
upload command input files, worker logs, model caches, vector snapshots,
databases, indexes, or local data directories. Synthetic fixtures are required
for public reproduction.

The product does not bundle a licensed OCR engine or embedding model yet. Those
remain BLOCKED until model and engine licenses are reviewed and distribution is
approved.

## Model Manifest Validation

Canonical local command form:
`resume-cli model validate-manifest --manifest <path>`.

Validate a reviewed local model pack before wiring it into an embedding or OCR
worker:

```bash
resume-cli --data-dir <local-data-dir> model validate-manifest \
  --manifest <local-model-manifest.json>
```

The manifest schema is `resume-ir.model-manifest.v1` with a `model_pack_id` and
one or more `models`. Each model entry must include `id`, `type`, `format`,
`artifact.path`, `artifact.sha256`, and a `license` object with `id` and
`reviewed: true`. Embedding models must also include `dim`.

The validator reads only local files, verifies artifact checksums, and blocks
unreviewed licenses. It must not print local paths, model bytes, or complete
digests.

## OCR Worker

Canonical local command form: `resume-cli ocr-worker --once`.

Foreground one-shot OCR worker:

```bash
resume-cli --data-dir <local-data-dir> ocr-worker --once \
  --command <local-ocr-command>
```

Daemon one-shot OCR worker:

```bash
resume-daemon --data-dir <local-data-dir> run --foreground --once \
  --work-ocr-once \
  --ocr-command <local-ocr-command>
```

Daemon loop with status IPC:

```bash
resume-daemon --data-dir <local-data-dir> run --foreground \
  --work-ocr \
  --ocr-command <local-ocr-command> \
  --ipc-listen 127.0.0.1:0
```

If a command crashes or returns malformed output, the worker must not print OCR
stdout, OCR stderr, input bytes, or paths. The document should remain
`OcrRequired`, the job should be `FailedRetryable`, and the OCR cache should
record a retryable failure without text. Validate with:

```bash
cargo test -p resume-cli --test s15_ocr_handoff --locked
cargo test -p resume-daemon --test s50_ocr_worker --locked
```

## Embedding Worker

Foreground one-shot embedding worker:

```bash
resume-cli --data-dir <local-data-dir> embed-worker --once \
  --command <local-embedding-command> \
  --model-id <reviewed-model-id> \
  --dimension <dimension>
```

Daemon one-shot embedding worker:

```bash
resume-daemon --data-dir <local-data-dir> run --foreground --once \
  --work-embeddings-once \
  --embedding-command <local-embedding-command> \
  --embedding-model-id <reviewed-model-id> \
  --embedding-dimension <dimension>
```

Use only reviewed local commands. Do not use commands that call a network API or
download model weights at runtime. Do not upload model outputs or vector
snapshots.

## Recovery Checks

After a worker failure:

```bash
resume-cli --data-dir <local-data-dir> status
resume-cli --data-dir <local-data-dir> doctor
resume-cli --data-dir <local-data-dir> export-diagnostics --redact
```

The output should show retryable queues without raw resume text, complete paths,
command paths, OCR text, or vector values.

## Known Blockers

- real PDF page rendering for OCR is not complete
- multi-page scanned PDF OCR is not complete
- OCR bounding boxes are not persisted
- licensed model distribution is BLOCKED
- Windows command process-tree validation is not complete
- macOS and Windows service-level worker validation is not complete

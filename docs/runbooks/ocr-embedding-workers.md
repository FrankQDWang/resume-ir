# OCR and Embedding Worker Runbook

## Scope

Local-only runbook for the configured OCR and embedding command workers. Do not
upload command input files, worker logs, model caches, vector snapshots,
databases, indexes, or local data directories. Synthetic fixtures are required
for public reproduction.

The product does not bundle a licensed OCR engine or embedding model yet. Those
remain BLOCKED until OCR runtime, model, and language-pack licenses are reviewed
and distribution is approved.

## External runtime decision

Tesseract/tessdata is the preferred external OCR runtime for the current stage.
It is treated as a local command runtime with reviewed checksum and license
evidence, not as an opaque bundled dependency. Poppler/pdftoppm is an accepted
user-installed external PDF renderer and may be configured by command path, but
it is not bundled by default.

## PDF Renderer License Boundary

The MIT project may call a user-installed Poppler command such as `pdftoppm`
through a local subprocess boundary. That does not make the repository's Rust
code a Poppler distribution, and it keeps the default install path compatible
with the current MIT public repository goal. Do not bundle Poppler/pdftoppm by
default in product installers.

If a future release bundles Poppler binaries, that release becomes a separate
GPL-family distribution review item. The installer/release evidence must record
the exact installed Poppler license from the selected distribution, include the
required license/source-offer materials, and pass legal review before the
release blocker can be cleared. Runtime manifests should record the exact
installed Poppler license, version, artifact checksum, and reviewed status
instead of assuming Poppler is MIT-licensed.

This is no longer an unresolved runtime-choice blocker. Current engineering
work is dependency detection, local manifest validation, checksum/license
recording, fail-closed operator errors, and redacted diagnostics. If the
operator does not have Tesseract, tessdata, or pdftoppm installed, commands
should report the missing dependency and remediation without printing local
paths, raw resume text, OCR text, page images, command stderr, model caches, or
index contents.

PDFium remains the preferred future permissive-license bundled renderer candidate if the product later needs an included PDF renderer. MuPDF and
Ghostscript can be evaluated as external command adapters, but their
AGPL/commercial license posture is not a better default for this MIT repository.

## OCR Runtime Preflight

Canonical local command form:
`resume-cli ocr preflight --json`.

Before running OCR workers, check that the local external OCR runtime is
discoverable without printing command paths:

```bash
resume-cli --data-dir <local-data-dir> ocr preflight --json \
  --ocr-lang eng
```

If the operator keeps tools outside `PATH`, pass explicit local command paths:

```bash
resume-cli --data-dir <local-data-dir> ocr preflight --json \
  --ocr-lang eng \
  --tesseract-command <local-tesseract-command> \
  --pdftoppm-command <local-pdftoppm-command>
```

The JSON schema is `ocr-runtime-preflight.v1`. The command exits nonzero when
`pdftoppm`, `tesseract`, or the requested Tesseract language pack is missing or
unknown, and it prints remediation such as installing Poppler/pdftoppm,
Tesseract/tessdata, or the requested language pack. Output must keep paths as
`<redacted>` and must not print command paths, OCR text, page images, Tesseract
language dumps, model caches, indexes, or local data directories.

## OCR Runtime Manifest Validation

Canonical local draft command form:
`resume-cli ocr draft-manifest --out <path>`.

After dependency preflight, create a local-only manifest draft from the selected
external commands and language pack:

```bash
resume-cli --data-dir <local-data-dir> ocr draft-manifest \
  --out <local-ocr-runtime-manifest.json> \
  --runtime-pack-id <reviewed-runtime-pack-id> \
  --tesseract-command <local-tesseract-command> \
  --pdftoppm-command <local-pdftoppm-command> \
  --language eng \
  --language-pack <local-tessdata-file> \
  --engine-license Apache-2.0 \
  --renderer-license <installed-poppler-license> \
  --language-license Apache-2.0 \
  --reviewed
```

The draft command writes the manifest to the local `--out` file and keeps stdout
redacted. The manifest file itself contains local artifact paths because the
validator must read those files to verify checksums. Do not commit, upload, or
paste this manifest unless it has been separately reviewed and stripped of local
paths. Omit `--reviewed` when legal review is not complete; subsequent
validation and release-readiness intake must then fail closed.

Canonical local command form:
`resume-cli ocr validate-manifest --manifest <path>`.

Validate a reviewed local OCR runtime pack before wiring it into OCR workers:

```bash
resume-cli --data-dir <local-data-dir> ocr validate-manifest \
  --manifest <local-ocr-runtime-manifest.json>
```

The manifest schema is `resume-ir.ocr-runtime-manifest.v1` with a
`runtime_pack_id` and one or more `components`. Each component entry must
include `id`, `kind`, `engine`, `version`, `artifact.path`, `artifact.sha256`,
and a `license` object with `id` and `reviewed: true`. Supported component
kinds are `ocr-engine`, `pdf-renderer`, and `ocr-language-pack`. Optional
`languages` entries must include `id`, `artifact.path`, `artifact.sha256`, and
a reviewed license.

The validator reads only local files, verifies artifact checksums, and blocks
unreviewed licenses. It must not print local paths, runtime bytes, language pack
bytes, or complete digests.

## Embedding Runtime Preflight

Canonical local command form:
`resume-cli model preflight --json`.

Before running embedding workers or semantic search, verify the reviewed model
manifest and local embedding command without printing paths:

```bash
resume-cli --data-dir <local-data-dir> model preflight --json \
  --manifest <local-model-manifest.json> \
  --embedding-command <local-embedding-command> \
  --model-id <reviewed-model-id> \
  --dimension <dimension>
```

The JSON schema is `embedding-runtime-preflight.v1`. The command validates the
model manifest checksum/license evidence, confirms that the requested embedding
model id and dimension are present, and exits nonzero when the embedding command
is missing or not executable. It must not execute a network API, download model
weights, print command paths, print model bytes, print embedding vectors, or
include model caches, indexes, or local data directories.

## Model Manifest Validation

Canonical local draft command form:
`resume-cli model draft-manifest --out <path>`.

After selecting a local offline embedding artifact, create a local-only model
manifest draft:

```bash
resume-cli --data-dir <local-data-dir> model draft-manifest \
  --out <local-model-manifest.json> \
  --model-pack-id <reviewed-model-pack-id> \
  --model-id <reviewed-model-id> \
  --model-type embedding \
  --dimension <dimension> \
  --format <model-format> \
  --artifact <local-model-artifact> \
  --license <model-license-id> \
  --reviewed
```

The draft command writes the manifest to the local `--out` file and keeps stdout
redacted. The manifest file itself contains the local artifact path because the
validator must read the model file to verify its checksum. Do not commit,
upload, or paste this manifest unless it has been separately reviewed and
stripped of local paths.

Omit `--reviewed` when model weight license review is not complete. Validation,
preflight, release-readiness, vector-quality, and private benchmark gates must
then fail closed.

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

- licensed OCR runtime and language-pack distribution is BLOCKED until reviewed
  runtime manifests are available and approved
- licensed model distribution is BLOCKED
- full non-English OCR quality validation is not complete
- full-library scanned resume OCR proof beyond bounded witness budgets is not
  complete
- real large-corpus OCR throughput proof is not complete
- Windows command process-tree validation is not complete
- macOS and Windows service-level worker validation is not complete

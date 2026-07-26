# OCR and Atomic Search Publication Runbook

## Scope

Local-only runbook for the configured OCR worker and daemon-owned atomic search
publication. Do not upload command input files, runtime logs, model caches, vector snapshots,
databases, indexes, or local data directories. Synthetic fixtures are required
for public reproduction.

The desktop product requires reviewed classifier, embedding, OCR and PDFium
runtime packs. A release remains blocked until the exact target packs and their
installer composition receipts are complete.

## Bundled-first runtime decision

The product runtime is bundled: installers include reviewed OCR and PDFium
packs with exact license, checksum, source/build identity, notices and
upgrade/rollback evidence. All evidence must record `runtime_distribution_mode` and
`runtime_package_binaries_included` so a local external command is never
mistaken for an included product runtime. The privacy sentinel
`runtime_binaries_included` must remain false because local evidence packages
must not contain runtime binary contents.
This bundled-first policy applies to every supported desktop target.

Tesseract plus tessdata is the OCR engine/language-pack stack. A pinned
statically linked PDFium build is the only desktop PDF text and page-rendering runtime on
macOS and Windows. There is no Poppler compatibility path in the desktop
product.

An External override is a local developer or diagnostic lane only. It never
replaces the reviewed bundled runtime, never satisfies installer composition,
and never becomes release evidence.
Use `scripts/release/create-runtime-bundle-manifest.sh` during release dry-runs
to produce the redacted `release.runtime_bundle.v1` manifest for reviewed local
runtime artifacts. The distribution uses
`LicenseRef-resume-ir-mixed-runtime-bundle`; each component still carries its
own exact license identity. Each component records its reviewed status and
source-offer identity. The manifest records basenames, byte counts, sha256
hashes, license IDs, sources, notices, and source identity evidence only; it
does not commit or upload runtime binaries.

For the current-stage private 10k validation flow, use
`scripts/local/run-current-stage-validation.sh` as the orchestration entrypoint.
Its dry-run prints a redacted local-only plan; execute mode runs the same
preflight, manifest, OCR and atomic search-publication, benchmark, diagnostics,
and release-readiness steps locally without uploading evidence.

## PDF Renderer License Boundary

The repository source license and the PDFium component license are recorded
independently. Each product target must carry the exact PDFium root license,
source commit, dependency revision, GN arguments, static-pack digest and final
executable identity. Missing, partial, symlinked or mismatched pack content
fails closed and cannot be treated as a release composition.

## OCR and PDFium runtime admission

Desktop runtime admission is daemon-owned and happens after the authenticated
control plane is available. The daemon validates the immutable runtime-pack
root supplied by the desktop launcher:

- the OCR pack contains the target-native Tesseract executable and exact
  `eng`/`chi_sim` tessdata described by `runtime-pack.json`;
- the PDFium pack contains the target-native
  `resume-pdf-render-runtime`, the pinned source contract and the exact PDFium
  root license;
- every executable identity is computed from the target-native canonical
  executable payload, every data file is hashed, and dependency closure must
  match the pack contract;
- missing, modified, symlinked, non-regular or wrong-target content reports the
  corresponding optional runtime as unavailable without blocking keyword
  search or details.

macOS pack preparation uses:

```bash
cd apps/desktop
npm run build:macos:pdfium
npm run build:macos:ocr
```

The PDFium source builder requires a complete Xcode installation selected by
an explicit `DEVELOPER_DIR` or by `xcode-select`; Command Line Tools alone are
not accepted. Its preflight runs before source synchronization and leaves the
global developer-directory selection unchanged.

Windows pack preparation runs only on a native Windows x64/MSVC host:

```powershell
cd apps/desktop
npm run build:windows:pdfium
npm run build:windows:ocr
```

The Windows OCR builder does not use a Linux container, cross-emulation or a
downloaded prebuilt executable. Both targets stage the same four reviewed
runtime capabilities: classifier, embedding, OCR and PDFium.

Runtime-pack manifests and source contracts may contain pinned public source
identities and checksums, but no local paths. Runtime bytes, source checkouts,
OCR pages, model caches and installed pack paths remain local and are never
included in diagnostics or public evidence.

## Embedding Runtime Preflight

Canonical local command form:
`resume-cli model preflight --json`.

Before enabling vector publication or semantic search, verify the reviewed
model manifest and local embedding command without printing paths:

```bash
resume-cli --data-dir <local-data-dir> model preflight --json \
  --manifest <local-model-manifest.json> \
  --embedding-command <local-embedding-command> \
  --model-id <reviewed-model-id> \
  --dimension <dimension>
```

The JSON schema is `embedding-runtime-preflight.v1`. The command validates the
model manifest checksum/license evidence, confirms that the requested embedding
model id and dimension are present, then runs one synthetic local protocol probe
through the configured command. The JSON includes `embedding_protocol` with
`passed`, `failed`, or `not_run`. It exits nonzero when the embedding command is
missing, not executable, or does not speak `resume-ir-embedding-v1` for the
requested model and dimension. It must not execute a network API, download model
weights, print command paths, print model bytes, print embedding vectors, print
synthetic probe text, or include model caches, indexes, or local data
directories.

### Local lightweight hash adapter

The repository includes `scripts/local/embedding-runtime-hash.py` as the
lightweight local smoke fallback for operator bring-up. It has no third party
dependency, reads the private input file from
`RESUME_IR_EMBEDDING_INPUT_PATH`, verifies `RESUME_IR_EMBEDDING_MODEL_ID` and
`RESUME_IR_EMBEDDING_DIMENSION`, and writes only the
`resume-ir-embedding-v1` stdout protocol. It is a lexical hashing vectorizer,
not a semantic model and not a production-representative embedding baseline.

Use `resume-ir-hash-embedding-v1` with a fixed dimension such as `256` when the
goal is protocol smoke or local import/query harness observation before a real
reviewed local model is available. Do not use hash output to make vector
quality, semantic search, D10K/W1, profile optimization, scale, or
`goal_complete` claims. The model artifact can be the executable adapter script
itself, recorded with the repository license in a local-only reviewed model
manifest. Generated manifests contain local artifact paths and must stay local.

### Local multilingual E5 ONNX adapter

The recommended lightweight semantic runtime for the next local private
baseline is `scripts/local/embedding-runtime-e5-onnx.py` with
`intfloat/multilingual-e5-small`. The model is MIT licensed, multilingual,
384-dimensional, and capped at 512 tokens. It is a real embedding model; model
files and tokenizer files still remain external local runtime artifacts and
must not be committed or uploaded.

The adapter requires a local model directory in `RESUME_IR_E5_MODEL_DIR` with
tokenizer files and an ONNX file at `onnx/model.onnx` by default. To use another
repo-local ONNX filename such as an optimized export, set
`RESUME_IR_E5_ONNX_FILE=<relative-onnx-file>`. The command loads with
local-files-only behavior, forces CPU ONNX Runtime by default, prefixes the
input whose id is exactly `query` with `query: `, prefixes all document inputs
with `passage: `, average-pools the ONNX hidden states, L2-normalizes the
vectors, and writes only `resume-ir-embedding-v1`.

Prepare the Python runtime in a private local environment:

```bash
uv venv .cache/resume-ir-e5-onnx-py312 --python <python-3.12>
uv pip install --python .cache/resume-ir-e5-onnx-py312/bin/python \
  numpy onnxruntime transformers
PATH=<repo>/.cache/resume-ir-e5-onnx-py312/bin:$PATH
```

Create the reviewed local model manifest from the exact local ONNX artifact:

```bash
resume-cli --data-dir <local-data-dir> model draft-manifest \
  --out <local-model-manifest.json> \
  --model-pack-id intfloat-multilingual-e5-small-onnx-local \
  --model-id intfloat/multilingual-e5-small \
  --model-type embedding \
  --dimension 384 \
  --format onnx \
  --artifact <local-e5-model-dir>/onnx/model.onnx \
  --license MIT \
  --reviewed

resume-cli --data-dir <local-data-dir> model validate-manifest \
  --manifest <local-model-manifest.json>
```

Then preflight the local command without downloading at runtime:

```bash
RESUME_IR_E5_MODEL_DIR=<local-e5-model-dir> \
resume-cli --data-dir <local-data-dir> model preflight --json \
  --manifest <local-model-manifest.json> \
  --embedding-command scripts/local/embedding-runtime-e5-onnx.py \
  --model-id intfloat/multilingual-e5-small \
  --dimension 384
```

If `transformers`, `onnxruntime`, `numpy`, tokenizer files, the ONNX artifact,
or reviewed manifest evidence are unavailable, the embedding runtime remains
external/legal/runtime BLOCKED. The failure path must not print local model
paths, model bytes, raw resume text, raw query text, vectors, cache locations,
or diagnostics payloads.

### Local sentence-transformers adapter

The repository includes
`scripts/local/embedding-runtime-sentence-transformers.py` as a reproducible
external command adapter for local sentence-transformers models. It reads the
private input file path from `RESUME_IR_EMBEDDING_INPUT_PATH`, verifies
`RESUME_IR_EMBEDDING_MODEL_ID` and `RESUME_IR_EMBEDDING_DIMENSION`, loads a
locally cached sentence-transformers model, and writes only the
`resume-ir-embedding-v1` stdout protocol. It must not print local paths, raw
resume text, model bytes, cache locations, or embedding input payloads.

Current-stage smoke validation may alternatively use
`sentence-transformers/all-MiniLM-L6-v2` with dimension `384` after the
operator has reviewed the current model card/license and recorded a local model
manifest. The model remains an external local runtime artifact; do not commit or
upload model weights, model caches, generated manifests with local paths, vector
snapshots, SQLite databases, query sets, or diagnostics.

Use the local manifest preparation helper to turn an already cached
sentence-transformers snapshot into a reviewed local model manifest. The helper
does not download weights. It reads the local Hugging Face cache, requires the
model card license to match the requested license, uses `model.safetensors` as
the checksum-bearing artifact, calls `resume-cli model draft-manifest`, validates
the manifest, and prints only redacted status output:

```bash
scripts/local/prepare-local-embedding-model-manifest.sh \
  --out <local-model-manifest.json> \
  --model-id sentence-transformers/all-MiniLM-L6-v2 \
  --model-pack-id sentence-transformers-all-MiniLM-L6-v2-local \
  --dimension 384 \
  --license Apache-2.0
```

If the local cache, model card, `model.safetensors`, or license match is
missing, the helper exits nonzero and the embedding runtime remains
external/legal/runtime BLOCKED. The generated manifest contains local artifact
paths and must stay local.

By default, the adapter loads with local-files-only behavior so preflight and
worker execution do not implicitly download model weights. To intentionally
prepare a local cache, run the download in a private local environment first,
then switch back to offline execution:

```bash
uv venv .cache/resume-ir-embedding-runtime-py312 --python <python-3.12>
uv pip install --python .cache/resume-ir-embedding-runtime-py312/bin/python \
  sentence-transformers
PATH=<repo>/.cache/resume-ir-embedding-runtime-py312/bin:$PATH \
RESUME_IR_SENTENCE_TRANSFORMERS_ALLOW_DOWNLOAD=1 \
RESUME_IR_SENTENCE_TRANSFORMERS_MODEL=sentence-transformers/all-MiniLM-L6-v2 \
RESUME_IR_EMBEDDING_INPUT_PATH=<synthetic-local-input> \
RESUME_IR_EMBEDDING_MODEL_ID=sentence-transformers/all-MiniLM-L6-v2 \
RESUME_IR_EMBEDDING_DIMENSION=384 \
scripts/local/embedding-runtime-sentence-transformers.py
```

Use the adapter as the embedding command after the model is locally available:

```bash
resume-cli --data-dir <local-data-dir> model preflight --json \
  --manifest <local-model-manifest.json> \
  --embedding-command scripts/local/embedding-runtime-sentence-transformers.py \
  --model-id sentence-transformers/all-MiniLM-L6-v2 \
  --dimension 384
```

For `model draft-manifest`, use a local reviewed descriptor or actual model
artifact file that represents the selected local model cache and records the
reviewed license/checksum evidence. The product has not bundled that model; if
the model card, artifact checksum, or license review is not confirmed, leave the
manifest unreviewed and treat embedding runtime as BLOCKED.

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

## Atomic Search Publication

Vector construction is part of the same publication boundary as metadata,
full-text, and the active document-to-version projection. There is no separate
embedding job queue or mutable vector backfill worker. A publication failure
leaves the previous active projection and both index generations readable.

For a bounded local reconcile after a direct CLI import, run the daemon with the
reviewed resident embedding runtime:

```bash
resume-daemon --data-dir <local-data-dir> run --foreground --once \
  --work-index-once \
  --embedding-command <local-embedding-command> \
  --embedding-model-id <reviewed-model-id> \
  --embedding-dimension <dimension>
```

Normal import and OCR daemon runs use the same three embedding options. New or
changed immutable resume versions are embedded before the metadata CAS publishes
the new active projection; unchanged exact-version vectors are retained.

Use only reviewed local commands. Do not use commands that call a network API or
download model weights at runtime. Do not upload model outputs or vector
snapshots.

## Recovery Checks

After an OCR or atomic publication failure:

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
- macOS and Windows service-level runtime validation is not complete

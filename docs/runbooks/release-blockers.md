# Release Blockers Runbook

## Scope

Local-only release-readiness runbook. Do not upload real resumes, local data
directories, diagnostics, logs, indexes, model caches, tokens, or signing
material. Synthetic fixtures are the only public reproduction input.

This repository is not ready for stable release while any BLOCKED item below is
unresolved.

## Current-stage boundary

The current goal is a reproducible local baseline, not final latency tuning.
Completion evidence for this stage is a local 10k validation baseline over the
available private corpus, observable aggregate metrics, and a repeatable
operator workflow. P95/P99 reduction, stricter latency targets, and external
100k/1M real-corpus validation belong to the deferred performance-optimization goal.

OCR runtime selection is no longer an open product blocker for this stage:
Tesseract/tessdata is the accepted external OCR engine direction, and
Poppler/pdftoppm is accepted only as a user-installed external PDF renderer.
The current-stage work is runtime manifests, checksum/license records,
dependency detection, fail-closed errors, and runbooks.

Signing and notarization are release-credential blockers. This repository must
provide scripts, CI secret interfaces, fail-closed gates, and documentation, but
real certificates, private keys, Apple Developer credentials, Windows signing
credentials, and notarization credentials are human-provided release inputs.

Embedding runtime work must use a real local offline runtime path with a model
manifest, checksum, license record, and failure guidance. If a model weight
license is not reviewed, mark the model as external/legal blocked; do not use a
placeholder model claim to clear release evidence.

## Current-Stage Local Validation Flow

Use the current-stage validation script to generate a redacted operator plan
before touching private resumes. The dry-run emits
`resume-ir.current-stage-validation-plan.v1` with privacy boundary
`local_only_redacted_plan`, placeholder paths, the ordered local commands, and
`performance_optimization_deferred: true`. It does not scan, import, OCR,
embed, benchmark, or read the private corpus:

```bash
scripts/local/run-current-stage-validation.sh --dry-run \
  --resume-root <private-local-root> \
  --data-dir <local-data-dir> \
  --out-dir <local-evidence-dir> \
  --query-set <local-query-set.jsonl> \
  --model-manifest <local-model-manifest.json> \
  --ocr-runtime-manifest <local-ocr-runtime-manifest.json> \
  --model-artifact <local-model-artifact> \
  --embedding-command <local-embedding-command> \
  --model-pack-id <reviewed-model-pack-id> \
  --model-id <reviewed-local-model-id> \
  --model-format <model-format> \
  --dimension <dimension> \
  --model-license <model-license-id> \
  --runtime-pack-id <reviewed-runtime-pack-id> \
  --tesseract-command <local-tesseract-command> \
  --pdftoppm-command <local-pdftoppm-command> \
  --language eng \
  --language-pack <local-tessdata-file> \
  --engine-license Apache-2.0 \
  --renderer-license <installed-poppler-license> \
  --language-license Apache-2.0 \
  --dataset-manifest-sha256 <sha256> \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10
```

Run execute mode only on the operator's machine and keep every generated file
local. The script performs OCR/model preflight, drafts local manifests, validates
reviewed manifests, imports the selected root, runs bounded OCR and embedding
worker loops, writes `benchmark-corpus-summary.local.json`, writes the private
query baseline report, runs the current-stage baseline shape gate, exports
redacted diagnostics, and feeds the local evidence into `release-readiness`.
At the end it also writes
`current-stage-validation-evidence.json` with schema
`resume-ir.current-stage-validation-evidence.v1` and privacy boundary
`local_only_redacted_evidence_manifest`. That manifest contains step statuses,
input digests, output file digests, the `release-readiness` exit code, and
privacy sentinels only. It must not contain local paths, raw resume text, raw
query text, report bodies, model bytes, runtime binaries, indexes, or SQLite
data.
After the execute run writes the manifest, operators may pass it back to
`release-readiness` with
`--current-stage-evidence current-stage-validation-evidence.json` to validate
the redacted manifest schema, required step statuses, SHA-256 output digests,
and privacy sentinels without exposing the local evidence directory or report
bodies.
Add `--reviewed-model` and `--reviewed-ocr-runtime` only after the selected
model weights, OCR engine, renderer, and language pack have actually been
reviewed; otherwise validation must fail closed.

```bash
scripts/local/run-current-stage-validation.sh --execute \
  --resume-root <private-local-root> \
  --data-dir <local-data-dir> \
  --out-dir <local-evidence-dir> \
  --query-set <local-query-set.jsonl> \
  --model-manifest <local-model-manifest.json> \
  --ocr-runtime-manifest <local-ocr-runtime-manifest.json> \
  --model-artifact <local-model-artifact> \
  --embedding-command <local-embedding-command> \
  --model-pack-id <reviewed-model-pack-id> \
  --model-id <reviewed-local-model-id> \
  --model-format <model-format> \
  --dimension <dimension> \
  --model-license <model-license-id> \
  --runtime-pack-id <reviewed-runtime-pack-id> \
  --tesseract-command <local-tesseract-command> \
  --pdftoppm-command <local-pdftoppm-command> \
  --language eng \
  --language-pack <local-tessdata-file> \
  --engine-license Apache-2.0 \
  --renderer-license <installed-poppler-license> \
  --language-license Apache-2.0 \
  --dataset-manifest-sha256 <sha256> \
  --reviewed-model \
  --reviewed-ocr-runtime \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10
```

The current-stage baseline shape gate intentionally uses
`--max-p95-ms 86400000` so a slow 10k private-corpus benchmark records an
observed baseline instead of turning this goal into an endless latency tuning
loop. P95/P99 reduction, strict `--max-p95-ms 200`, and 100k/1M validation are
deferred to the follow-up performance-optimization goal. Do not commit or
upload the query set, local manifests, benchmark reports, diagnostics, indexes,
SQLite databases, model caches, runtime binaries, or raw resumes.

## Current BLOCKED Items

- signing certificates are not available for production installers
- notarization credentials are not available for macOS release artifacts
- Windows MSI install, upgrade, uninstall, and rollback are not proven
- Windows service install, start, stop, status, uninstall, rollback, and recovery
  are not proven
- macOS signed pkg/dmg install, upgrade, uninstall, and rollback are not proven
- private real-corpus hot-index hybrid benchmark baseline over the available
  local corpus is not available
- private business labeled field-quality evidence is not available
- private business labeled dedupe-quality evidence is not available
- private business labeled vector-quality evidence is not available
- private real-corpus OCR throughput evidence is not available
- reviewed OCR runtime manifest/dependency evidence for the selected external
  OCR direction is not available
- a reviewed licensed embedding model is not selected or distributed
- Windows and macOS cross-platform validation are not complete
- redacted diagnostics evidence from `export-diagnostics --redact` has not been
  reviewed through release-readiness
- hardware fault drills for actual ENOSPC, service-level daemon kill,
  battery-mode, and external-drive disconnect are not proven on release
  platforms

## Pre-Release Local Gate

Run the local gate before any public push:

```bash
./scripts/ci/verify-local.sh
./scripts/ci/guard-public-repo.sh
```

The explicit stable-release readiness gate must remain blocked until every
release criterion has current local evidence:

```bash
resume-cli --data-dir <local-data-dir> release-readiness --json
```

After local redacted aggregate reports have been generated and reviewed, feed
them into the readiness gate as evidence inputs. Reviewed model/OCR manifests
can be supplied only after their artifacts, checksums, and licenses have been
validated locally:

```bash
resume-cli --data-dir <local-data-dir> release-readiness --json \
  --benchmark-report private-benchmark-local.json \
  --field-quality-report private-field-quality.json \
  --dedupe-quality-report private-dedupe-quality.json \
  --vector-quality-report private-vector-quality.json \
  --ocr-throughput-report private-ocr-throughput.json \
  --model-manifest local-model-manifest.json \
  --ocr-runtime-manifest local-ocr-runtime-manifest.json \
  --diagnostics-report redacted-diagnostics.json \
  --current-stage-evidence current-stage-validation-evidence.json \
  --release-artifact-manifest release-artifacts.json \
  --release-sbom release-sbom.json \
  --macos-package-manifest macos-package.json \
  --windows-package-manifest windows-package.json \
  --signing-evidence signing-evidence.json \
  --notarization-evidence notarization-evidence.json \
  --macos-installer-evidence macos-installer-evidence.json \
  --windows-installer-evidence windows-installer-evidence.json \
  --windows-service-evidence windows-service-evidence.json
```

Passing these local evidence inputs marks only the corresponding local evidence
items as `provided_evidence`; aggregate reports and redacted diagnostics evidence
are marked `redacted_local_aggregate`, and reviewed model/OCR manifests are marked
`reviewed_local_manifest`. Blocked signing, notarization, macOS installer,
Windows installer, Windows service, release artifact, release SBOM, macOS
package, and Windows package dry-run manifests are marked
`blocked_release_evidence_manifest`. The current-stage validation evidence
manifest is marked `local_only_redacted_evidence_manifest`; it records the
local operator flow, input/output digests, step statuses, and privacy sentinels,
but it does not replace the benchmark, quality, model, OCR runtime, signing,
notarization, installer, platform, diagnostics, or hardware-drill evidence
items. The labels are:

- signing automation evidence
- notarization automation evidence
- release artifact manifest evidence
- release SBOM evidence
- macOS package manifest evidence
- Windows package manifest evidence
- macOS installer automation evidence
- Windows installer automation evidence
- Windows service automation evidence
- current-stage validation evidence manifest

Those automation and dry-run manifest evidence entries prove only that
fail-closed automation, schema checks, redacted artifact inventory, and redacted
SBOM generation exist; they do not clear signing, notarization, installer
lifecycle, service lifecycle, GitHub Release upload, or cross-platform release
blockers. The command must still fail closed while signing, notarization,
installer lifecycle, cross-platform release validation, hardware fault-drill
blockers, or any missing local evidence remain unresolved. Do not upload or
commit generated reports or manifests unless they have been separately reviewed
to contain no raw resume text, filenames, local paths, queries, labels, sample
IDs, document IDs, vectors, page images, secrets, diagnostics, indexes, model
files, OCR runtime binaries, or model caches.

Generate the diagnostics report from the same local data directory used for the
current validation run:

```bash
resume-cli --data-dir <local-data-dir> export-diagnostics --redact \
  > redacted-diagnostics.json
resume-cli --data-dir <local-data-dir> release-readiness --json \
  --diagnostics-report redacted-diagnostics.json
```

The release-readiness diagnostics intake validates only `diagnostics.v1`
redacted local aggregate diagnostics: top-level `redacted: true`, redacted path,
query, and resume-text sentinels, `evidence_level: "local_aggregate_only"`, and
the expected aggregate diagnostic scope. It rejects reports with common local
path or secret markers and never prints the report path or report body.

Release dry-runs must also produce a blocked signing evidence manifest, not a
fake signature result. The manifest schema is `release.signing_evidence.v1` and
must contain only artifact names, byte counts, hashes, and blocked signing
evidence status. It must not contain private keys, certificate passwords,
signing tokens, local paths, resume data, diagnostics, indexes, or model caches.

```bash
scripts/release/create-artifact-manifest.sh \
  --version v0.0.0 \
  --target-dir target/release \
  --out-dir release-dry-run

scripts/release/create-signing-evidence.sh \
  --version v0.0.0 \
  --artifact-manifest release-dry-run/release-artifacts.json \
  --out-dir release-dry-run
```

This signing evidence manifest is a fail-closed release evidence validator. It
does not sign artifacts, does not validate a certificate chain, does not prove
private key custody, and cannot clear the signing certificates blocker until
production signing certificates and per-artifact signature verification evidence
exist.

macOS package dry-runs must also produce a blocked notarization evidence
manifest. The manifest schema is `release.notarization_evidence.v1` and must
contain only macOS package artifact names, byte counts, hashes, and blocked
notarization evidence status. It must not contain notary credentials, notary
passwords, local paths, resume data, diagnostics, indexes, or model caches.

```bash
scripts/release/create-notarization-evidence.sh \
  --version v0.0.0 \
  --macos-package-manifest macos-package-dry-run/macos-package.json \
  --out-dir macos-package-dry-run
```

This notarization evidence manifest is a fail-closed release evidence
validator. It does not submit artifacts through `notarytool`, staple
notarization tickets, validate Gatekeeper with `spctl`, or clear the macOS
notarization blocker until Apple Developer ID credentials and per-artifact
notarization ticket/Gatekeeper evidence exist.

Run the benchmark smoke only as smoke evidence, not as production performance
proof:

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  synthetic-query --documents 24 --queries 6 --top-k 5 --json
```

Run the synthetic benchmark gate against a local or nightly benchmark artifact:

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  gate --report benchmark-smoke.json --allow-synthetic \
  --min-documents 1000 --min-queries 100 \
  --max-p95-ms 500 --max-zero-result-queries 0
```

The installed binary form is `resume-benchmark gate --report
benchmark-smoke.json`.

The explicit `--allow-synthetic` flag is required for synthetic smoke artifacts.
Synthetic query reports must include `generation_mode: "streaming"` so larger
local synthetic pressure runs do not require pre-collecting the full synthetic
document set in memory. Do not treat a passing synthetic gate as 100k or 1M
real-corpus proof.

Run private real-corpus benchmark baseline checks only against local redacted
aggregate reports. The current product goal requires a reproducible baseline,
observability metrics, and a local validation workflow over the available
private corpus; it does not require looping on P95/P99 latency reduction in this
goal. The report must use `dataset_kind: "private-real-corpus"`,
`target_claim: "benchmark_baseline_observed"`, `corpus_origin:
"private_local"`, `privacy_boundary: "redacted_local_aggregate"`,
`query_mode: "hybrid"`, `retrieval_layers: "fulltext+field+vector+rrf"`,
`hot_index: true`, explicit aggregate `searchable_document_count` and
`vector_indexed_document_count` hot-index coverage fields, false hot-path OCR/
parsing/heavy-model-inference booleans, false raw-data/path/query booleans, and
sha256 digests for the local dataset manifest, query set, reviewed embedding
model manifest, and redacted `benchmark-corpus-summary` preflight. It must also
have internally consistent aggregate metrics: hot-index coverage counts are
non-zero and no larger than `document_count`, latency samples equal query count,
zero-result queries do not exceed query count, total hits do not exceed
`query_count * top_k`, latency percentiles P50/P95/P99 are present and ordered,
`query_total_ms` is positive, and reported QPS matches `query_count /
(query_total_ms / 1000)` within rounding tolerance. Do not upload reports if
they contain raw resume text, local paths, queries, sample IDs, or filenames.

The current local private corpus is approximately ten thousand resumes, not a
100k or 1M corpus. Local release-readiness therefore requires a redacted
hot-index hybrid baseline over the available private corpus with at least 8000
local documents, at least 8000 hot-searchable documents, at least 8000
vector-indexed documents, and 500 query latency samples. P95/P99 reduction and
external 100k/1M scale validation move to the follow-up performance
optimization goal; do not keep rerunning this goal solely because the baseline
latency is above the eventual product target.

Generate the private query benchmark report locally only after the target
private corpus has been imported, indexed, and warmed, and after the local query
command has been reviewed to run hot hybrid search without OCR, parsing, or
heavy model inference on the query path. Prefer the product-owned
`resume-cli benchmark-query-protocol` command over private wrapper scripts; it
returns only the benchmark protocol and runs the query through the normal
product hybrid search path. The query-set JSONL stays local and may contain raw
private queries; the benchmark runner passes each query through an owner-only
temporary file path in `RESUME_IR_QUERY_INPUT_PATH` plus
`RESUME_IR_QUERY_TOP_K` and `RESUME_IR_QUERY_MODE=hybrid`, and must return only
`resume-ir-query-v1` plus `hits=<n>` on stdout. Do not upload the query-set, the
report, or command wrappers unless they have been separately reviewed to contain
no raw queries, filenames, local paths, tokens, or resume data.
If a wrapper is still needed, it must delegate through
`resume-cli benchmark-query-protocol` or pass the query file through
`resume-cli search --query-file "$RESUME_IR_QUERY_INPUT_PATH" --mode hybrid`
instead of putting the raw query in argv; wrapper stdout must still be reduced
to the benchmark protocol only.

Before generating the benchmark report, capture local hot-index corpus coverage
as a redacted aggregate summary:

```bash
resume-cli --data-dir <local-data-dir> benchmark-corpus-summary --json \
  > benchmark-corpus-summary.local.json
```

Pass the summary file directly to `private-query`; do not hand-copy counts.
The benchmark runner will reject summaries that do not prove full hot-index
coverage and will emit only the summary file's SHA-256 digest in the benchmark
report. The summary is local evidence only and must not contain raw resume
text, local paths, queries, filenames, sample IDs, or document IDs.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  private-query \
  --query-set private-query-set.jsonl \
  --command resume-cli \
  --command-arg --data-dir --command-arg <local-data-dir> \
  --command-arg benchmark-query-protocol \
  --command-arg --embedding-command --command-arg <embedding-command> \
  --command-arg --model-id --command-arg <model-id> \
  --command-arg --dimension --command-arg <dim> \
  --corpus-summary benchmark-corpus-summary.local.json \
  --max-queries 500 --top-k 10 \
  --dataset-manifest-sha256 <sha256> \
  --query-set-sha256 <sha256> \
  --model-manifest-sha256 <sha256> \
  --json > private-benchmark-local.json
```

The strict gate below remains available for the follow-up performance
optimization goal and should not be used as this goal's completion blocker:

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  gate --report private-benchmark-local.json \
  --require-private-real-corpus \
  --min-documents 8000 --min-queries 500 \
  --max-p95-ms 200 --max-zero-result-queries 0
```

These gates are release evidence validators. They do not create, upload, or
sanitize private benchmark reports and cannot clear the benchmark blocker until
representative local private-corpus evidence exists.

Run private business field-quality gates only against local redacted aggregate
reports. The report must use `dataset_kind: "private-business-labeled"`,
`target_claim: "field_quality_target_met"`, `corpus_origin: "private_local"`,
`privacy_boundary: "redacted_local_aggregate"`, `field_taxonomy:
"resume-ir.fields.v1"`, false raw-data/path/field-value/sample-ID booleans, and
sha256 digests for both the dataset and annotation manifests. It must include
production field metrics for name, email, phone, school, school_tier, degree,
major, company, title, location, skill, certificate, date ranges, and years
experience. Every production field metric must have positive labeled support
(`true_positive + false_negative > 0`), and the reported precision, recall, and
F1 must match the aggregate counts within rounding tolerance. Do not upload
reports if they contain raw resume text, local paths, field values, sample IDs,
filenames, or notes.

Generate the private field-quality aggregate report locally from a reviewed
business-labeled JSONL dataset. The JSONL may contain raw resume text, sample
IDs, and field labels, so it must stay local. The generated report is aggregate
only and still must be reviewed before any upload or public commit.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  field-quality \
  --dataset private-field-quality.jsonl \
  --private-business-labeled \
  --dataset-manifest-sha256 <sha256> \
  --annotation-manifest-sha256 <sha256> \
  --json > private-field-quality.json
```

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  field-gate --report private-field-quality.json \
  --require-private-business-labeled \
  --min-samples 1000 \
  --min-precision 0.93 --min-recall 0.93 --min-f1 0.93
```

This gate is a release evidence validator. It does not create, upload, label,
or sanitize private field-quality reports and cannot clear the field extraction
quality blocker until representative local business labels and aggregate field
metrics exist.

Run private business dedupe-quality gates only against local redacted aggregate
reports. The report must use `dataset_kind: "private-business-labeled"`,
`target_claim: "dedupe_quality_target_met"`, `corpus_origin: "private_local"`,
`privacy_boundary: "redacted_local_aggregate"`, `dedupe_taxonomy:
"resume-ir.dedupe.v1"`, false raw-data/path/profile-value/sample-ID/document-ID
booleans, and sha256 digests for both the dataset and annotation manifests. Do
not upload reports if they contain names, schools, companies, skills, document
IDs, sample IDs, filenames, local paths, raw resume text, or notes. The labeled
JSONL can contain raw profile values and identifiers only while it stays in a
reviewed local private workspace; do not commit, upload, or archive that JSONL.
The aggregate pair counts must be internally consistent:
`pair_count == true_positive + false_positive + false_negative + true_negative`,
`positive_pair_count == true_positive + false_negative`, and
`predicted_duplicate_pairs == true_positive + false_positive`. The reported
precision, recall, and F1 must match those aggregate counts within rounding
tolerance.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  dedupe-quality --dataset private-dedupe-quality.jsonl \
  --private-business-labeled \
  --dataset-manifest-sha256 <sha256> \
  --annotation-manifest-sha256 <sha256> \
  --json > private-dedupe-quality.json
```

Review the generated report before any release evidence upload or public commit.
It must be aggregate-only and must not contain sample IDs, document IDs, local
paths, profile values, or raw resume text.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  dedupe-gate --report private-dedupe-quality.json \
  --require-private-business-labeled \
  --min-pairs 1000 --min-positive-pairs 100 \
  --min-precision 0.90 --min-recall 0.90 --min-f1 0.90
```

This workflow creates only a local redacted aggregate report and validates its
release-evidence shape. It does not upload reports, create labels, review
labeling quality, or clear the dedupe quality blocker until representative local
business labels and aggregate dedupe metrics exist.

Run private business vector-quality gates only against local redacted aggregate
reports. The report must use `dataset_kind: "private-business-labeled"`,
`target_claim: "vector_quality_target_met"`, `corpus_origin: "private_local"`,
`privacy_boundary: "redacted_local_aggregate"`, `vector_taxonomy:
"resume-ir.vector-quality.v1"`, false raw-query/candidate-text/path/sample-ID/
candidate-ID/vector booleans, and sha256 digests for the dataset, annotation,
and model manifests (`dataset_manifest_sha256`,
`annotation_manifest_sha256`, and `model_manifest_sha256`). Do not upload
reports if they contain raw queries,
candidate text, resume text, candidate IDs, sample IDs, filenames, local paths,
vectors, command paths, model paths, or notes. The labeled JSONL can contain raw
queries, candidate text, sample IDs, and candidate IDs only while it stays in a
reviewed local private workspace; do not commit, upload, or archive that JSONL.
Private vector-quality release reports must also have feasible aggregate
retrieval counts: `sample_count > 0`, `candidate_count > 0`, `top_k > 0`,
`candidate_count >= sample_count`, `top_k <= candidate_count`,
`zero_recall_queries <= sample_count`, and `recall_at_k` must not exceed the
maximum possible recall implied by `zero_recall_queries` within rounding
tolerance.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  vector-quality --dataset private-vector-quality.jsonl \
  --command <reviewed-local-embedding-command> \
  --model-id <reviewed-local-model-id> \
  --dimension <n> \
  --private-business-labeled \
  --dataset-manifest-sha256 <sha256> \
  --annotation-manifest-sha256 <sha256> \
  --model-manifest-sha256 <sha256> \
  --top-k 10 \
  --json > private-vector-quality.json
```

Review the generated report before any release evidence upload or public commit.
It must be aggregate-only and must not contain raw queries, candidate text,
sample IDs, candidate IDs, vectors, local paths, command paths, model paths, or
raw resume text.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  vector-gate --report private-vector-quality.json \
  --require-private-business-labeled \
  --min-samples 1000 \
  --min-recall-at-k 0.90 --min-mrr 0.85 --min-ndcg-at-k 0.90 \
  --max-zero-recall-queries 0
```

This workflow creates only a local redacted aggregate report and validates its
release-evidence shape. It does not upload reports, create labels, review
labeling quality, approve model licensing, or clear the vector quality blocker
until representative local business labels, a reviewed model manifest, and
aggregate semantic retrieval metrics exist.

Run private real-corpus OCR throughput gates only against local redacted
aggregate reports. The report must use `dataset_kind: "private-real-corpus"`,
`target_claim: "ocr_throughput_target_met"`, `corpus_origin:
"private_local"`, `privacy_boundary: "redacted_local_aggregate"`, false raw
OCR text/page image/path/document-ID/page-ID/command-path booleans, and sha256
digests for the dataset, OCR runtime, renderer, and language-pack manifests
(`dataset_manifest_sha256`, `ocr_runtime_manifest_sha256`,
`renderer_manifest_sha256`, and `language_pack_manifest_sha256`). Do not upload
reports if they contain raw OCR text, page images, resume text, filenames,
local paths, document IDs, page IDs, command paths, runtime paths, or notes.
Small or under-threshold diagnostic reports should use
`target_claim: "not_evaluated"`; the private OCR throughput command emits
`ocr_throughput_target_met` only when the built-in release OCR page, P95,
throughput, and run-budget thresholds are met.
Private OCR throughput reports must also include `total_ms` so
`pages_per_second` can be recomputed, per-document failure aggregates
(`failed_document_count`, `render_failure_count`, and `ocr_failure_count`), and
the total-run budget flag `run_budget_exhausted`. Release evidence must satisfy
`page_count > 0`, `document_count > 0`, `scanned_document_count > 0`,
`scanned_document_count <= document_count`, `scanned_document_count <=
page_count`, `failed_document_count <= document_count`, `render_failure_count +
ocr_failure_count == failed_document_count`, `page_latency_ms.samples ==
page_count`, `total_ms > 0`, `pages_per_second == page_count / (total_ms /
1000)` within rounding tolerance, and `run_budget_exhausted: false`.

Generate the private OCR throughput report locally. The command reads only local
PDF files under the requested root, runs the configured renderer plus OCR engine,
and prints only aggregate redacted JSON. Do not commit or upload the generated
report unless it has been separately reviewed to contain no raw OCR text, page
images, local paths, filenames, document IDs, page IDs, command paths, runtime
paths, or notes.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  private-ocr-throughput \
  --root <private-local-root> \
  --pdftoppm-command <pdftoppm> \
  --tesseract-command <tesseract> \
  --max-documents 900 --max-pages 500 --pages-per-document 1 \
  --page-timeout-ms 30000 --max-run-ms <release-budget-ms> \
  --render-dpi 150 --ocr-lang eng+chi_sim \
  --dataset-manifest-sha256 <sha256> \
  --ocr-runtime-manifest-sha256 <sha256> \
  --renderer-manifest-sha256 <sha256> \
  --language-pack-manifest-sha256 <sha256> \
  --json > private-ocr-throughput.json
```

Small private smoke reports can prove command wiring, but they do not clear the
release blocker. Representative evidence should set `--max-documents` above the
minimum page count so isolated corrupt, encrypted, render-failed, or OCR-failed
PDFs are counted in the redacted failure aggregates without aborting the whole
run. A report with `run_budget_exhausted: true`, or a report that misses the
latency/throughput thresholds below, is diagnostic local evidence only and
cannot clear the release blocker. Stable-release OCR throughput evidence needs
the representative page count and reviewed manifests required by the gate below.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  ocr-gate --report private-ocr-throughput.json \
  --require-private-real-corpus \
  --min-pages 500 --max-p95-ms 1000 --min-pages-per-second 1
```

This gate is a release evidence validator. It does not run OCR, upload, label,
or sanitize private OCR throughput reports and cannot clear the OCR throughput
blocker until representative local scanned-resume runs, reviewed runtime/
renderer/language-pack manifests, and aggregate latency/throughput metrics
exist.

Generate a local release dry-run manifest only after release binaries have been
built:

```bash
scripts/release/create-artifact-manifest.sh \
  --version v0.1.0 \
  --target-dir target/release \
  --out-dir release-dry-run
scripts/release/create-sbom.sh \
  --version v0.1.0 \
  --out-dir release-dry-run
```

The generated `release-artifacts.json` records binary names, byte counts, and
sha256 hashes. The generated `release-sbom.json` is a redacted SPDX 2.3 package
inventory derived from locked Cargo metadata. These dry-run files are not an
installer, signature, notarization ticket, or GitHub Release upload, and they
must not contain local paths or runtime data.

On macOS only, generate unsigned pkg/dmg dry-run artifacts after release
binaries have been built:

```bash
scripts/release/create-macos-package.sh \
  --version v0.1.0 \
  --target-dir target/release \
  --out-dir release-dry-run
```

The generated `macos-package.json` records only artifact filenames, byte counts,
hashes, unsigned status, and still-blocked release steps. The pkg/dmg files are
local evidence only. They are not signed, not notarized, not uploaded, and do
not prove install, upgrade, uninstall, rollback, or Gatekeeper behavior.

Release dry-runs must also produce a blocked macOS installer lifecycle evidence
manifest. The manifest schema is `release.macos_installer_evidence.v1` and must
contain only pkg/dmg artifact names, byte counts, hashes, planned installer and
LaunchAgent lifecycle actions, blocked evidence status, and the macOS package
manifest digest. It must not contain installer tokens, administrator passwords,
local paths, raw installer logs, resume data, diagnostics, indexes, or model
caches.

```bash
scripts/release/create-macos-installer-evidence.sh \
  --version v0.1.0 \
  --macos-package-manifest release-dry-run/macos-package.json \
  --out-dir release-dry-run
```

This macOS installer evidence manifest is a fail-closed release evidence
validator. It does not run `installer`, mount or install a dmg, install,
upgrade, uninstall, prove rollback, start/stop a LaunchAgent, or clear the macOS
installer lifecycle blocker until administrator-elevated release-runner
evidence exists.

Generate the macOS installer lifecycle dry-run operator plan without executing
installer commands:

```bash
scripts/release/run-macos-installer-lifecycle.sh \
  --version v0.1.0 \
  --macos-package-manifest release-dry-run/macos-package.json \
  --out release-dry-run/macos-installer-lifecycle-dry-run.json \
  --dry-run
```

The generated `macos-installer-lifecycle-dry-run.json` has schema
`release.macos_installer_lifecycle_plan.v1`. It records only artifact
filenames, the macOS package manifest digest, planned install, upgrade,
uninstall, rollback, LaunchAgent start, and LaunchAgent stop actions, plus the
commands that a release runner must execute later. It must not contain local
paths, administrator passwords, installer logs, resume data, diagnostics,
indexes, or model caches. It is an operator plan only and does not clear the
macOS installer lifecycle blocker.

On Windows only, generate an unsigned MSI dry-run artifact after release
binaries have been built and the WiX .NET tool is installed:

```powershell
dotnet tool install --global wix --version 6.0.2
scripts/release/create-windows-package.ps1 `
  -Version v0.1.0 `
  -TargetDir target/release `
  -OutDir release-dry-run
```

The generated `windows-package.json` records only artifact filenames, byte
counts, hashes, unsigned status, MSI kind, and still-blocked release steps. The
MSI file is local evidence only. It is not signed, not uploaded, and does not
prove install, upgrade, uninstall, rollback, Windows service registration, or
service lifecycle behavior.

Release dry-runs must also produce a blocked Windows installer lifecycle
evidence manifest. The manifest schema is
`release.windows_installer_evidence.v1` and must contain only MSI artifact
names, byte counts, hashes, planned installer lifecycle actions, blocked
evidence status, and the Windows package manifest digest. It must not contain
installer tokens, administrator passwords, local paths, raw installer logs,
resume data, diagnostics, indexes, or model caches.

```bash
scripts/release/create-windows-installer-evidence.sh \
  --version v0.1.0 \
  --windows-package-manifest release-dry-run/windows-package.json \
  --out-dir release-dry-run
```

This Windows installer evidence manifest is a fail-closed release evidence
validator. It does not run `msiexec`, install, upgrade, repair, uninstall,
prove rollback, or clear the Windows installer lifecycle blocker until
administrator-elevated release-runner evidence exists.

Generate the Windows installer lifecycle dry-run operator plan without
executing MSI commands:

```powershell
scripts/release/run-windows-installer-lifecycle.ps1 `
  -Version v0.1.0 `
  -WindowsPackageManifest release-dry-run/windows-package.json `
  -Out release-dry-run/windows-installer-lifecycle-dry-run.json `
  -DryRun
```

The generated `windows-installer-lifecycle-dry-run.json` has schema
`release.windows_installer_lifecycle_plan.v1`. It records only artifact
filenames, the Windows package manifest digest, planned install, upgrade,
repair, uninstall, and rollback actions, plus the `msiexec.exe` command that a
release runner must execute later. It must not contain local paths,
administrator passwords, installer logs, resume data, diagnostics, indexes, or
model caches. It is an operator plan only and does not clear the Windows
installer lifecycle blocker.

Generate local Windows Service dry-run evidence without registering a service:

```bash
resume-cli --data-dir <local-data-dir> service install \
  --platform windows-service \
  --daemon-binary <path-to-resume-daemon.exe> \
  --dry-run
resume-cli --data-dir <local-data-dir> service status \
  --platform windows-service \
  --dry-run
resume-cli --data-dir <local-data-dir> service start \
  --platform windows-service \
  --dry-run
resume-cli --data-dir <local-data-dir> service stop \
  --platform windows-service \
  --dry-run
resume-cli --data-dir <local-data-dir> service uninstall \
  --platform windows-service \
  --dry-run
```

These dry-runs are redacted command-plan evidence only. They do not prove
Windows service registration, service recovery, rollback, upgrade behavior, or
administrator-elevated install/uninstall.

Release dry-runs must also produce a blocked Windows Service lifecycle evidence
manifest. The manifest schema is `release.windows_service_evidence.v1` and must
contain only MSI artifact names, byte counts, hashes, planned lifecycle actions,
blocked evidence status, and the Windows package manifest digest. It must not
contain service tokens, administrator passwords, local paths, raw service logs,
resume data, diagnostics, indexes, or model caches.

```bash
scripts/release/create-windows-service-evidence.sh \
  --version v0.1.0 \
  --windows-package-manifest release-dry-run/windows-package.json \
  --out-dir release-dry-run
```

This Windows Service evidence manifest is a fail-closed release evidence
validator. It does not register a service, start/stop/query it, configure
recovery, uninstall it, prove rollback, or clear the Windows service lifecycle
blocker until administrator-elevated release-runner evidence exists.

Validate any proposed local model pack before worker configuration:

```bash
resume-cli --data-dir <local-data-dir> model validate-manifest \
  --manifest <local-model-manifest.json>
```

This command is governance evidence only. A valid manifest does not by itself
complete licensed model selection, model quality evaluation, distribution
approval, or production performance proof.
After review, pass the same manifest to
`resume-cli release-readiness --model-manifest <local-model-manifest.json>` so
the release gate can validate checksum/license evidence without printing local
paths or model contents.

Validate any proposed local OCR runtime pack before worker configuration:

```bash
resume-cli --data-dir <local-data-dir> ocr validate-manifest \
  --manifest <local-ocr-runtime-manifest.json>
```

This command is governance evidence only. The current OCR direction is
Tesseract plus tessdata as an accepted Apache-2.0 external OCR runtime, with
Poppler `pdftoppm` called as a user-installed external PDF renderer and not bundled by default. A valid OCR runtime manifest must record checksums and
reviewed licenses for the local OCR engine, tessdata language packs, and
renderer dependency, and the product must keep dependency detection plus
fail-closed operator guidance in place. A valid manifest does not by itself
complete non-English OCR quality validation, platform installer validation, or
production OCR throughput proof.

Poppler/pdftoppm is operationally strong and widely packaged, but its licensing
and distribution review must stay separate from this MIT repository's default
release artifacts. It is acceptable as an external command discovered on the
operator's machine or explicitly configured by path. Do not bundle Poppler
binaries into default installers until legal review approves the exact binary
source, license notices, source-offer obligations, and installer composition.
PDFium remains the preferred future permissive-license bundled renderer
candidate if the product later needs an included PDF renderer. MuPDF and
Ghostscript are viable external command alternatives in some deployments, but
their AGPL/commercial licensing posture is not a better default for a permissive
MIT distribution.

After review, pass the same manifest to
`resume-cli release-readiness --ocr-runtime-manifest
<local-ocr-runtime-manifest.json>` so the release gate can validate checksum,
engine, renderer, language-pack, and license evidence without printing local
paths or runtime contents.

## Stable Release Exit Criteria

Stable release requires current evidence for:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- `cargo test --workspace --locked`
- redacted diagnostics without raw resume text or complete paths
- Windows install, upgrade, uninstall, service start, service stop, and rollback
- macOS install, upgrade, uninstall, LaunchAgent start, LaunchAgent stop, signing,
  and notarization
- 100k and 1M hot-index hybrid benchmark runs on representative hardware
- private real-corpus OCR throughput gate with reviewed OCR runtime, renderer,
  and language-pack manifests
- OCR and embedding model license review
- OCR runtime manifest checksum validation
- model pack manifest checksum validation

If any item is missing, keep the release blocked and update `PROGRESS.md` with
the exact missing evidence and owner.

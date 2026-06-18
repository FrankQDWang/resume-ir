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

OCR runtime direction is bundled-first with external override. Tesseract/
tessdata is the accepted OCR engine/language-pack direction. PDFium is the
preferred bundled PDF renderer candidate when quality is sufficient; Poppler/
pdftoppm is acceptable for bundled-first runtime packaging when the release
distribution records GPL-compatible license evidence, source-offer obligations,
notices, checksums, and installer composition. The current-stage work is
runtime manifests, checksum/license records, dependency detection, fail-closed
errors, and runbooks.

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

The execute flow first performs OCR and embedding runtime preflight, drafts
local OCR/model manifests, and validates those manifests before reading the
private resume root. OCR preflight must run a synthetic local PDF render plus
Tesseract TSV probe and record `runtime_probe: "passed"` before private corpus
access continues. Embedding preflight must run a synthetic local
`resume-ir-embedding-v1` protocol probe and record `embedding_protocol: "passed"`
before private corpus access continues. If runtime preflight or manifest
validation fails, execute mode stops before scanning the private corpus or
copying a private query set.
Those pre-corpus failures write `current-stage-blocked-summary.json` with
`private_corpus_read: false`, `blocked_category: "ocr"` for OCR runtime
failures or `blocked_category: "embedding"` for model/protocol failures, the
blocked preflight or manifest step, and basename-only digests for local runtime
probe outputs and manifests that were produced. The summary must not include
the private resume root, query bodies, benchmark reports, diagnostics, indexes,
or SQLite data because none of those steps may run before runtime preflight is
accepted.
Caller-supplied OCR/model manifest digests are checked against the generated
local manifests before private corpus scanning continues. Caller-supplied query
set digests are checked against the generated or locally copied query set before
private query benchmarking starts.
After runtime preflight succeeds, execute mode generates a local redacted
dataset manifest with `resume-cli privacy dataset-manifest`. The manifest schema
is `resume-ir.dataset-manifest.v1` and its privacy boundary is
`local_only_redacted_dataset_manifest`. It records only aggregate counts,
supported-extension counts, budget state, and a corpus fingerprint; it must not
contain local paths, file names, raw resume text, per-file hashes, indexes,
SQLite data, diagnostics, or model/runtime caches. Operators may pass
`--dataset-manifest-sha256 <sha256>` only as an optional consistency check; if
omitted, execute mode computes the digest from
`<local-evidence-dir>/dataset-manifest.local.json`.
If dataset manifest generation or private corpus import fails, execute mode
writes `current-stage-blocked-summary.json` with
`blocked_category: "import/parser"`, the blocked dataset/import step, and
`private_corpus_read: true`, then stops before OCR workers, embedding workers,
query-set generation, benchmarks, diagnostics, or release-readiness. The
summary records only basename-only digests for runtime preflight outputs, the
redacted dataset manifest when present, and import stdout; it must not include
resume paths, filenames, raw parsed text, query bodies, benchmark reports,
indexes, SQLite data, or diagnostics.

For repeated local validation on an already imported private data directory,
operators may pass `--reuse-imported-corpus --reuse-dataset-manifest
<prior-redacted-dataset-manifest.json>`. This mode is explicit opt-in. It
validates and copies the prior `resume-ir.dataset-manifest.v1` redacted
manifest into the new local evidence directory, checks any caller-supplied
dataset-manifest digest against that copied file, writes a redacted
`dataset-manifest.stdout.txt`, then runs `resume-cli status` against the
existing local data directory and records that aggregate status under
`import.stdout.txt`. It skips `privacy dataset-manifest --root ...` and
`import --root ...`, so it is useful for continuing bounded OCR/embedding/query
smoke after a full private corpus import has already completed locally. It must
not be used to claim a new corpus scan/import happened, and it still leaves full
current-stage evidence blocked unless the later hot-index, 500-query, OCR
throughput, diagnostics, fault, and release-readiness evidence all pass.

The default `--validation-profile full` is the only profile intended to produce
`resume-ir.current-stage-validation-evidence.v1` for `release-readiness
--current-stage-evidence`. The `--validation-profile smoke` profile is a
bounded local command-wiring proof for situations where the local private corpus
is dominated by OCR-required files and full OCR would make the current
interaction run for too long. Smoke still performs runtime preflight, manifest
validation, import, bounded OCR/embedding workers, query-set generation,
private-query benchmark protocol, a low-floor benchmark gate, and redacted
diagnostics. It then runs the safe synthetic `fault-simulate --case
disk-space-low --json` smoke probe and writes `current-stage-smoke-summary.json`
with schema `resume-ir.current-stage-smoke-summary.v1`. Smoke output is
explicitly not release-readiness evidence, must not be passed as proof of the
10k/8000-document baseline, and must keep full baseline, 500-query baseline,
P95/P99 optimization, 100k/1M validation, and stable release readiness marked
not complete or BLOCKED. The synthetic fault probe only proves that local
diagnostic/fault evidence wiring can produce `fault-simulation.v1`; it does not
clear actual ENOSPC, daemon kill, process crash, power-loss, or hardware
fault-drill release blockers.

If `--query-set <local-query-set.jsonl>` is omitted, execute mode drafts a
local private query set after import/OCR/embedding work by running
`resume-cli benchmark-query-set draft`. The generated JSONL schema is
`resume-ir.query-set.jsonl.v1` and its privacy boundary is
`local_only_private_query_set`. The query-set file may contain private query
terms derived from high-confidence non-contact fields and must stay local under
the evidence directory; stdout and the current-stage evidence manifest include
only counts, basenames, and SHA-256 digests. The draft command excludes names,
emails, phones, local paths, filenames, raw resume text, document IDs, and
sample IDs derived from source data.

The smoke profile passes `--allow-keyword-fallback` to the local query-set
draft command. That fallback is only for proving the current-stage wiring when
a tiny OCR-heavy sample has searchable text but too few high-confidence
non-contact field mentions. It still writes only a local private JSONL query
set and redacted stdout. The full profile does not use the fallback: the full
500-query baseline remains blocked until the local corpus can produce the
required field-backed query set.

The smoke profile also passes
`--allow-partial-hot-index-for-smoke` to `resume-benchmark private-query`.
This lets a bounded wiring witness continue when only a subset of the imported
documents became searchable and vector-indexed within the smoke worker budget.
The benchmark report still carries redacted aggregate `document_count`,
`searchable_document_count`, and `vector_indexed_document_count`; the full
profile does not use the flag and remains blocked until the required hot-index
coverage floor is met.
The private benchmark evidence validator follows the selected validation
profile: full profile still requires the 8000-document searchable/vector floors
and 500 latency samples, while smoke profile requires non-zero searchable,
vector, and query-sample evidence plus the same privacy, protocol, and internal
consistency checks.

`benchmark-corpus-summary.local.json` also carries redacted aggregate
`document_status_counts`, `ingest_job_status_counts`,
`ingest_job_kind_status_counts`, and `ingest_job_failure_counts`. Use those
counts to classify current-stage blockers such as OCR backlog, retryable OCR
failures, queued index work, or parser/import gaps without reading local paths,
document IDs, query text, raw resume text, report bodies, indexes, SQLite data,
or diagnostics.
In full-profile execute mode, if the bounded OCR worker leaves OCR-required
documents and the hot index is still not fully covered, the script writes
`redacted-diagnostics.json` with `export-diagnostics --redact`, runs `doctor`
against the same local data directory, then stops before query-set generation
and writes `current-stage-blocked-summary.json` with
`blocked_step: "ocr_worker_bounded_loop"`, `blocked_category: "ocr"`, and
`blocked_reason: "ocr_backlog_exceeds_current_stage_budget"`. The blocked
summary records the diagnostics and doctor outputs only by basename and SHA-256
digest. This is the expected current-stage handoff for an OCR-heavy private corpus; it is not
release-clearing evidence and must not be passed to `--current-stage-evidence`
or used to claim the 10k/8000-document baseline is complete. It may be passed to
`release-readiness --current-stage-blocked-summary` only as non-clearing
operator handoff evidence; the full baseline and stable release blockers must
remain blocked.
The smoke and benchmark-blocked summaries copy those safe counts under
`corpus_summary_observability`, so a handoff can classify blockers from the
summary itself. The full release-readiness evidence manifest still records only
the corpus-summary basename and digest, not the report body.

Execute mode automatically writes `current-stage-handoff.json` after it writes a
local smoke summary, blocked summary, or full current-stage evidence manifest.
To rebuild or inspect that committed-safe operator handoff manually, run:

```bash
scripts/local/summarize-current-stage-validation.py \
  --input <local-evidence-dir>/current-stage-smoke-summary.json \
  --out <local-evidence-dir>/current-stage-handoff.json
```

The output schema is `resume-ir.current-stage-handoff.v1` with privacy boundary
`local_only_redacted_handoff`. It copies only structured status, preflight
probe statuses, redacted aggregate observability counts, completed step names,
must-not-upload categories, not-complete/BLOCKED items, and a `next_action`
object. For blocked summaries, `next_action` names the blocker category and the
next rerun target, while explicitly preserving the current-stage boundary: do
not chase P95/P99 optimization and do not require million-resume validation in
this stage. It fails closed if the input contains private markers or local path
shapes, or if the input schema does not use its required source privacy boundary
(`local_only_redacted_aggregate_summary` for smoke,
`local_only_redacted_blocked_summary` for blocked handoff, and
`local_only_redacted_evidence_manifest` for full evidence). The handoff report
is for operator continuity only: it is not release-readiness evidence, not a
substitute for the full current-stage validation evidence manifest, and not
proof that the complete product is done. Pass the blocked summary itself, not
the handoff report, to `--current-stage-blocked-summary` when release-readiness
needs structured non-clearing blocked-state context.

```bash
scripts/local/run-current-stage-validation.sh --dry-run \
  --validation-profile full \
  --resume-root <private-local-root> \
  --data-dir <local-data-dir> \
  --out-dir <local-evidence-dir> \
  [--query-set <local-query-set.jsonl>] \
  [--reuse-imported-corpus --reuse-dataset-manifest <prior-redacted-dataset-manifest.json>] \
  --model-manifest <local-model-manifest.json> \
  --ocr-runtime-manifest <local-ocr-runtime-manifest.json> \
  --model-artifact <local-model-artifact> \
  --embedding-command <local-embedding-command> \
  [--embedding-runtime-bin-dir <local-runtime-bin-dir>] \
  --model-pack-id <reviewed-model-pack-id> \
  --model-id <reviewed-local-model-id> \
  --model-format <model-format> \
  --dimension <dimension> \
  --model-license <model-license-id> \
  --runtime-pack-id <reviewed-runtime-pack-id> \
  [--tesseract-command <local-tesseract-command>] \
  [--pdftoppm-command <local-pdftoppm-command>] \
  --language eng \
  --language-pack <local-tessdata-file> \
  --engine-license Apache-2.0 \
  --renderer-license <installed-poppler-license> \
  --language-license Apache-2.0 \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10 \
  [--private-query-timeout-ms <query-command-timeout-ms>] \
  [--ocr-jobs-per-tick <bounded-ocr-job-budget>]
```

For Tesseract combined languages such as `eng+chi_sim`, pass repeated
`--language-pack <lang>=<local-tessdata-file>` entries so the local OCR runtime
manifest records every tessdata checksum and reviewed license separately.
When `--tesseract-command` or `--pdftoppm-command` is omitted,
`run-current-stage-validation.sh` uses `command -v tesseract` and
`command -v pdftoppm` on the operator machine, then still records only redacted
command templates in plan, summary, and handoff outputs. Pass explicit command
paths only when pinning a non-`PATH` runtime or reviewing an exact local binary.
If the embedding command depends on a local Python or tool runtime that is not
the default shell runtime, pass `--embedding-runtime-bin-dir
<local-runtime-bin-dir>` instead of relying on an operator-modified `PATH`.
Execute mode prepends that directory only for child commands. The dry-run plan,
smoke summary, blocked summary, and full evidence manifest record only
`embedding_runtime_bin_dir_configured: true|false`; they must never contain the
local runtime path.

Run execute mode only on the operator's machine and keep every generated file
local. The script performs OCR/model preflight, drafts local manifests, validates
reviewed manifests, imports the selected root, runs bounded OCR and embedding
worker loops, writes `benchmark-corpus-summary.local.json`, writes the private
query baseline report, runs the current-stage baseline shape gate, writes the
private OCR throughput baseline report, runs the current-stage OCR throughput
baseline gate, exports redacted diagnostics, runs `doctor` against the same
local data directory, and feeds the local evidence into the local safe synthetic
fault probe before feeding the redacted local evidence into `release-readiness`.
At the end it also writes
`current-stage-validation-evidence.json` with schema
`resume-ir.current-stage-validation-evidence.v1` and privacy boundary
`local_only_redacted_evidence_manifest`. That manifest contains step statuses,
input digests, `preflight_probes` with `ocr_runtime_probe: "passed"` and
`embedding_protocol: "passed"`, redacted `corpus_summary_observability`
aggregate counts copied from `benchmark-corpus-summary.local.json`, explicit
`full_baseline_satisfied: true` and `release_readiness_evidence: true` flags
for handoff generation, output file digests, the `release-readiness` exit code,
and privacy sentinels only. It must not contain local paths, raw
resume text, raw query text, report bodies, model bytes, runtime binaries,
indexes, or SQLite data.
After the execute run writes the manifest, operators may pass it back to
`release-readiness` with
`--current-stage-evidence current-stage-validation-evidence.json` to validate
the redacted manifest schema, the complete local-flow step statuses, required
basename-only output files with SHA-256 digests, and privacy sentinels without
exposing the local evidence directory or report bodies. The required output
inventory includes the dataset manifest, query set, OCR/model preflight logs,
bounded worker stdout, corpus summary, private benchmark report, benchmark gate
stdout, private OCR throughput report, OCR throughput gate stdout, redacted
diagnostics, `doctor.out`, `fault-simulation-storage-low.json`, and
release-readiness stdout/stderr digests.
The `redacted_outputs` inventory must contain exactly those expected basenames;
unknown extra files are rejected even when their names are basename-only.
The `steps` array must exactly match the ordered local validation flow; duplicate
step IDs or unknown extra steps are rejected.
The manifest is accepted only when `max_files >= 8000`, `max_queries >= 500`,
`release_readiness_exit == 1`, `preflight_probes.ocr_runtime_probe == "passed"`,
`preflight_probes.embedding_protocol == "passed"`, and the dataset, query-set,
model-manifest, and OCR-runtime-manifest input digests match the corresponding
basename-only output digests.
When `--current-stage-evidence` is supplied in the same `release-readiness`
invocation as `--benchmark-report`, `--ocr-throughput-report`,
`--diagnostics-report`, `--model-manifest`, or `--ocr-runtime-manifest`,
`release-readiness` recomputes the supplied file's SHA-256 and requires it to
match the current-stage manifest entry for the corresponding local output or
input digest. A mismatch is a fail-closed evidence-bundle error and must not
print local paths.
Add `--reviewed-model` and `--reviewed-ocr-runtime` only after the selected
model weights, OCR engine, renderer, and language pack have actually been
reviewed; otherwise validation must fail closed.

If the full profile reaches the private benchmark report but the baseline shape
gate fails, execute mode writes `current-stage-blocked-summary.json` with schema
`resume-ir.current-stage-blocked-summary.v1` and privacy boundary
`local_only_redacted_blocked_summary`, then exits non-zero before
`release-readiness`. That file records the blocked step/category/reason, input
digests, preflight probe statuses, completed step statuses,
`corpus_summary_observability` aggregate counts, and basename-only output
digests. It is not release-clearing evidence and must not be passed to
`--current-stage-evidence`; after review it may be passed to
`--current-stage-blocked-summary` only to record a structured blocked handoff in
release-readiness without clearing the private real-corpus baseline blocker. It
exists so the next operator can see whether the failure was benchmark
coverage/query/gate related without exposing local paths, private query text,
report bodies, indexes, or diagnostics.
When the baseline shape gate fails, treat the full current-stage baseline as
not complete and continue from the blocked summary rather than reading private
reports directly.

If private OCR throughput generation or the current-stage OCR throughput
baseline gate fails after the baseline shape gate has passed, execute mode
writes `current-stage-blocked-summary.json` with `blocked_category: "ocr"` and
either `blocked_step: "private_ocr_throughput_baseline"` or
`blocked_step: "ocr_throughput_baseline_gate"`, then stops before diagnostics
and release-readiness. That summary records aggregate corpus observability,
the configured `ocr_throughput_min_pages`, and basename-only digests for the
OCR throughput report and gate stdout. It does not include OCR text, rendered
page images, local paths, document IDs, page IDs, command paths, report bodies,
indexes, or SQLite data. This gate proves a reproducible current-stage
baseline; strict P95/P99 and pages-per-second reduction remains the follow-up
performance optimization goal.

If local query-set generation fails before the private benchmark can run,
execute mode also writes `current-stage-blocked-summary.json` with
`blocked_step: "query_set_draft"`, `blocked_category: "query-set"`, and
`blocked_reason: "query_set_draft_failed"`. That summary includes the same
redacted corpus observability counts and the query-set draft stdout digest, but
does not include the query-set file, query bodies, local paths, or benchmark
reports.
If the private query baseline command itself fails, execute mode writes the same
blocked summary schema with `blocked_step: "private_query_baseline"`,
`blocked_category: "benchmark"`, and
`blocked_reason: "private_query_baseline_failed"`, then stops before the
benchmark gate and release-readiness intake. That summary records only digests
for the query set and partial benchmark stdout file plus aggregate corpus
observability; it does not include query bodies, benchmark report bodies, local
paths, indexes, or diagnostics.
If redacted diagnostics export fails after the baseline gate has run, execute
mode writes `current-stage-blocked-summary.json` with
`blocked_step: "redacted_diagnostics"`, `blocked_category: "diagnostics"`, and
`blocked_reason: "redacted_diagnostics_failed"`, then stops before
release-readiness. That summary records aggregate corpus observability and file
digests up to the failed diagnostics output, not diagnostic bodies, local
paths, query text, indexes, or SQLite data.
If the safe synthetic fault simulation smoke or local-safe suite fails after
redacted diagnostics,
execute mode writes `current-stage-blocked-summary.json` with
`blocked_step: "fault_simulation_smoke"`, `blocked_category:
"fault-injection"`, and either
`blocked_reason: "fault_simulation_smoke_failed"` or
`blocked_reason: "fault_simulation_suite_failed"`, then stops before
release-readiness. That summary records aggregate corpus observability plus
basename-only digests through `fault-simulation-storage-low.json` and
`fault-simulation-suite-local-safe.json`; it does not include local paths,
diagnostic bodies, query text, raw resume text, indexes, SQLite data, or
scratch directory contents. The local-safe suite is passed the explicit
`resume-daemon` binary path so it can run the redacted synthetic daemon
kill/restart probe instead of guessing host paths, and execute mode creates a
local scratch-only OCR crash fixture so the aggregate suite can prove the
redacted OCR engine-failure path. Passing synthetic fault probes remain local
wiring and crash-recovery checks only; they do not clear the separate
release-platform hardware fault-drill blocker.
If `release-readiness` rejects the local evidence inputs themselves after the
baseline gate, redacted diagnostics, and fault simulation probes pass, execute mode writes the same
blocked summary schema with `blocked_step: "release_readiness_intake"`,
`blocked_category: "release-readiness"`, and
`blocked_reason: "release_readiness_evidence_failed_validation"`, then stops
before writing `current-stage-validation-evidence.json`. That summary records
only aggregate corpus observability and basename-only digests through
`release-readiness.json` and `release-readiness.stderr.txt`; it is not stable
release evidence and must not be uploaded with private reports, diagnostics,
indexes, SQLite data, or local paths.

```bash
scripts/local/run-current-stage-validation.sh --execute \
  --validation-profile full \
  --resume-root <private-local-root> \
  --data-dir <local-data-dir> \
  --out-dir <local-evidence-dir> \
  [--query-set <local-query-set.jsonl>] \
  [--reuse-imported-corpus --reuse-dataset-manifest <prior-redacted-dataset-manifest.json>] \
  --model-manifest <local-model-manifest.json> \
  --ocr-runtime-manifest <local-ocr-runtime-manifest.json> \
  --model-artifact <local-model-artifact> \
  --embedding-command <local-embedding-command> \
  [--embedding-runtime-bin-dir <local-runtime-bin-dir>] \
  --model-pack-id <reviewed-model-pack-id> \
  --model-id <reviewed-local-model-id> \
  --model-format <model-format> \
  --dimension <dimension> \
  --model-license <model-license-id> \
  --runtime-pack-id <reviewed-runtime-pack-id> \
  [--tesseract-command <local-tesseract-command>] \
  [--pdftoppm-command <local-pdftoppm-command>] \
  --language eng \
  --language-pack <local-tessdata-file> \
  --engine-license Apache-2.0 \
  --renderer-license <installed-poppler-license> \
  --language-license Apache-2.0 \
  [--dataset-manifest-sha256 <sha256>] \
  --reviewed-model \
  --reviewed-ocr-runtime \
  --max-files 10000 \
  --max-queries 500 \
  --top-k 10 \
  [--private-query-timeout-ms <query-command-timeout-ms>] \
  [--ocr-jobs-per-tick <bounded-ocr-job-budget>]
```

For bounded local command-wiring validation, use smoke mode and keep all outputs
local. The summary records only redacted aggregate status and output digests; it
does not write `current-stage-validation-evidence.json` and does not run
`release-readiness`:

```bash
scripts/local/run-current-stage-validation.sh --execute \
  --validation-profile smoke \
  --resume-root <private-local-root-or-small-local-sample-root> \
  --data-dir <local-data-dir> \
  --out-dir <local-evidence-dir> \
  [--query-set <local-query-set.jsonl>] \
  --model-manifest <local-model-manifest.json> \
  --ocr-runtime-manifest <local-ocr-runtime-manifest.json> \
  --model-artifact <local-model-artifact> \
  --embedding-command <local-embedding-command> \
  [--embedding-runtime-bin-dir <local-runtime-bin-dir>] \
  --model-pack-id <reviewed-model-pack-id> \
  --model-id <reviewed-local-model-id> \
  --model-format <model-format> \
  --dimension <dimension> \
  --model-license <model-license-id> \
  --runtime-pack-id <reviewed-runtime-pack-id> \
  [--tesseract-command <local-tesseract-command>] \
  [--pdftoppm-command <local-pdftoppm-command>] \
  --language eng \
  --language-pack <local-tessdata-file> \
  --engine-license Apache-2.0 \
  --renderer-license <installed-poppler-license> \
  --language-license Apache-2.0 \
  --reviewed-model \
  --reviewed-ocr-runtime \
  --max-files <bounded-file-count> \
  --max-queries <bounded-query-count> \
  --top-k 10 \
  [--private-query-timeout-ms <query-command-timeout-ms>] \
  [--ocr-jobs-per-tick <bounded-ocr-job-budget>]
```

Use `--ocr-jobs-per-tick` to increase how many queued OCR documents the
bounded worker may claim during one foreground worker tick. The default is `1`
for compatibility; raising it is a current-stage baseline throughput control,
not a P95/P99 query-latency optimization claim.

Use `--private-query-timeout-ms` to set the benchmark runner's per-query command
budget for local `resume-cli benchmark-query-protocol` invocations. The default
is `30000`; raising it is a current-stage validation command budget for cold or
OCR-heavy local data directories, not a claim that query P95/P99 latency has
been optimized.

Smoke mode passes `--allow-smoke-confidence` to the benchmark gate because a
bounded local wiring run may have `percentile_confidence: "smoke"`. Full
current-stage and release gates must not use that flag.

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
- GitHub Release publication is not approved or proven with a working GitHub
  Actions release token, Git credential path, and artifact upload evidence
- private real-corpus hot-index hybrid benchmark baseline over the available
  local corpus is not available
- private business labeled field-quality evidence is not available
- private business labeled dedupe-quality evidence is not available
- private business labeled vector-quality evidence is not available
- private real-corpus OCR baseline evidence is not available
- reviewed OCR/PDF runtime manifest/dependency evidence for the selected
  bundled-first distribution mode is not available
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

Every entry under `blockers[]` must be actionable, not just descriptive prose.
Besides `label`, `status: "blocked"`, and `detail`, each blocker must include a
`blocked_dependency` object with `kind`, `needed_from`, and `summary`, plus a
`next_action` string. These fields are the machine-readable source for final
reports and handoffs: they say whether the missing evidence is a human release
credential, release publication approval, a release-platform transcript, a
private labeled quality dataset, a reviewed runtime/model manifest, local
current-stage evidence, redacted local diagnostics evidence, or actual
hardware-drill evidence.

The JSON report must include `goal_gap_matrix` with schema
`resume-ir.goal-gap-matrix.v1`. That matrix is the product-level P0-P6 gap view:
P0/P1 can show local implementation covered by CI, P2/P3/P4 can show local
implementation present while quality/runtime/baseline evidence remains blocked,
P5 can show local release automation implementation complete while real signing,
notarization, administrator-elevated platform transcripts, and release approval
remain blocked, and P6 remains not complete until the full current-stage
baseline, quality datasets, hardware drills, and later external scale validation
exist. The matrix must keep `complete_product: false`,
`current_stage: "baseline_not_complete"`, and the completion statement that the
complete product is not complete while any row is blocked or not_complete.

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
  --release-publication-evidence release-publication-evidence.json \
  --github-release-publication-gate github-release-publication-gate.json \
  --macos-package-manifest macos-package.json \
  --windows-package-manifest windows-package.json \
  --signing-evidence signing-evidence.json \
  --notarization-evidence notarization-evidence.json \
  --macos-installer-evidence macos-installer-evidence.json \
  --windows-installer-evidence windows-installer-evidence.json \
  --windows-service-evidence windows-service-evidence.json \
  --macos-installer-lifecycle-plan macos-installer-lifecycle-dry-run.json \
  --windows-installer-lifecycle-plan windows-installer-lifecycle-dry-run.json \
  --windows-service-lifecycle-plan windows-service-lifecycle-dry-run.json \
  --hardware-fault-evidence hardware-fault-drills.json
```

For local quality evidence preparation, use the operator helper below only with
reviewed private business labeled datasets that remain on the local machine.
The helper is fail-closed: without `--reviewed` it exits with
`quality review is incomplete`, and it prints only redacted status output.

```bash
scripts/local/prepare-local-quality-release-evidence.sh --reviewed \
  --out-dir <local-evidence-dir> \
  --field-dataset private-field-quality.jsonl \
  --field-dataset-manifest-sha256 <sha256> \
  --field-annotation-manifest-sha256 <sha256> \
  --dedupe-dataset private-dedupe-quality.jsonl \
  --dedupe-dataset-manifest-sha256 <sha256> \
  --dedupe-annotation-manifest-sha256 <sha256> \
  --vector-dataset private-vector-quality.jsonl \
  --vector-dataset-manifest-sha256 <sha256> \
  --vector-annotation-manifest-sha256 <sha256> \
  --embedding-command <reviewed-local-embedding-command> \
  --model-id <reviewed-local-model-id> \
  --dimension <n> \
  --model-manifest-sha256 <sha256> \
  --resume-cli resume-cli \
  --data-dir <local-data-dir>
```

If the full current-stage execute flow stops before producing
`current-stage-validation-evidence.json`, validate the redacted blocked summary
as non-clearing handoff context instead:

```bash
resume-cli --data-dir <local-data-dir> release-readiness --json \
  --current-stage-blocked-summary current-stage-blocked-summary.json
```

This records `current-stage blocked handoff` under `provided_evidence` with
privacy boundary `local_only_redacted_blocked_summary`; it does not clear the
private real-corpus baseline, OCR throughput, diagnostics, model, runtime,
quality, platform, signing, or hardware fault-drill blockers.
When a blocked handoff is supplied in the same `release-readiness` invocation
as `--diagnostics-report`, `--model-manifest`, or `--ocr-runtime-manifest`,
`release-readiness` recomputes those files' SHA-256 digests and requires them
to match the blocked summary's redacted output or input digest entries. A
mismatch is a fail-closed evidence-bundle error and must not print local paths.
`--current-stage-blocked-summary` and `--current-stage-evidence` are mutually exclusive.
Passing both is a release-readiness input error because one claims a blocked
handoff while the other claims the full local validation evidence manifest
exists.

Passing these local evidence inputs marks only the corresponding local evidence
items as `provided_evidence`; aggregate reports and redacted diagnostics evidence
are marked `redacted_local_aggregate`, and reviewed model/OCR manifests are marked
`reviewed_local_manifest`. Blocked signing, notarization, macOS installer,
Windows installer, Windows service, installer lifecycle plans, Windows Service
lifecycle plan, release artifact, release SBOM, macOS package, and Windows
package dry-run manifests are marked
`blocked_release_evidence_manifest`. The current-stage validation evidence
manifest is marked `local_only_redacted_evidence_manifest`; it records the
local operator flow, input/output digests, step statuses, and privacy sentinels,
and the current-stage blocked handoff is marked
`local_only_redacted_blocked_summary`; neither replaces the benchmark, quality,
model, OCR runtime, signing, notarization, installer, platform, diagnostics, or
hardware-drill evidence items. The labels are:

- signing automation evidence
- notarization automation evidence
- release artifact manifest evidence
- release SBOM evidence
- macOS package manifest evidence
- Windows package manifest evidence
- macOS installer automation evidence
- Windows installer automation evidence
- Windows service automation evidence
- macOS installer lifecycle plan evidence
- Windows installer lifecycle plan evidence
- Windows service lifecycle plan evidence
- hardware fault drills
- current-stage validation evidence manifest
- current-stage blocked handoff

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

Actual hardware fault-drill evidence is separate from the dry-run and
simulation paths. `--hardware-fault-evidence hardware-fault-drills.json`
accepts only `release.hardware_fault_drills.v1` summaries with
`evidence_boundary: "redacted_release_hardware_fault_drills"` and
`execution_mode: "actual_release_platform_drill"`. The file must be produced
after running actual ENOSPC, service-level daemon kill, battery-mode, and
external-drive disconnect drills on dedicated release-platform test machines or
VMs, with both macOS and Windows marked `passed` for each drill. It may contain
only build/artifact digests, transcript/diagnostics SHA-256 digests, redaction
booleans, cleanup confirmation, and the must-not-upload list. The top-level
report, every drill entry, and nested platform status objects must contain only
the defined schema fields; unknown payload fields such as transcript paths or
ad hoc log pointers are rejected. Do not pass `fault-simulate` output, dry-run
operator plans, raw logs, local paths, raw resume text, diagnostic packages,
tokens, indexes, SQLite databases, model caches, or release-runner machine
paths as hardware fault evidence.

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
the expected aggregate diagnostic scope. The report may contain only the
defined top-level aggregate diagnostics fields emitted by
`export-diagnostics --redact`; unknown top-level payload fields are rejected so
raw logs, samples, or ad hoc diagnostic details cannot be smuggled into release
evidence. It rejects reports with common local path or secret markers and never
prints the report path or report body.

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

scripts/release/create-release-publication-evidence.sh \
  --version v0.0.0 \
  --artifact-manifest release-dry-run/release-artifacts.json \
  --out-dir release-dry-run

scripts/release/publish-github-release.sh \
  --dry-run \
  --version v0.0.0 \
  --repo FrankQDWang/resume-ir \
  --artifact-manifest release-dry-run/release-artifacts.json \
  --publication-evidence release-dry-run/release-publication-evidence.json \
  --out-dir release-dry-run
```

This signing evidence manifest is a fail-closed release evidence validator. It
does not sign artifacts, does not validate a certificate chain, does not prove
private key custody, and cannot clear the signing certificates blocker until
production signing certificates and per-artifact signature verification evidence
exist.

The GitHub Release publication evidence manifest is also fail-closed. Its schema
is `release.publication_evidence.v1`; release-readiness records it as
`GitHub Release publication automation evidence` with
`blocked_release_evidence_manifest`. It proves only that the dry-run has a
structured blocker for human release approval, GitHub Actions release-token
availability, release creation, and artifact upload evidence. It must not call
GitHub APIs, read tokens, create a release, upload artifacts, include local
paths, or clear the `GitHub Release publication` blocker.

`scripts/release/publish-github-release.sh` is the fail-closed publication entry
point. Dry-run mode writes `release.github_publication_gate.v1` to
`github-release-publication-gate.json`; release-readiness records it as
`GitHub Release publication gate evidence` with
`blocked_release_evidence_manifest`. It must not call GitHub, read tokens,
create releases, or upload artifacts. Execute mode requires all of the following
before it can publish: `--execute`, `--approve-release`, an `--artifact-dir`,
`gh`, and either `GITHUB_TOKEN` or `GH_TOKEN`. The release workflow runs only
dry-run mode by default. The gate's planned steps must include release creation,
artifact upload, and `gh_release_download_verify` so uploaded artifacts are
proved retrievable before publication can count as evidence. Real publication
remains blocked until a human approves the release and supplies the CI secret
interface.

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
`query_protocol: "resume-ir-query-v1"`, `query_mode: "hybrid"`,
`retrieval_layers: "fulltext+field+vector+rrf"`, `hot_index: true`, explicit
aggregate `searchable_document_count` and `vector_indexed_document_count`
hot-index coverage fields, false hot-path OCR/parsing/heavy-model-inference
booleans, false raw-data/path/query booleans, and sha256 digests for the local
dataset manifest, query set, reviewed embedding model manifest, and redacted
`benchmark-corpus-summary` preflight. It must also have internally consistent
aggregate metrics: hot-index coverage counts are non-zero and no larger than
`document_count`, latency samples equal query count, zero-result queries do not
exceed query count, total hits do not exceed `query_count * top_k`, latency
percentiles P50/P95/P99 are present and ordered, `query_total_ms` is positive,
and reported QPS matches `query_count / (query_total_ms / 1000)` within
rounding tolerance. Do not upload reports if they contain raw resume text, local
paths, queries, sample IDs, or filenames.

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
`resume-ir-query-v1`, `mode=hybrid`, `layers=fulltext+field+vector+rrf`,
`top_k=<n>`, and `hits=<n>` on stdout. Do not upload the query-set, the report,
or command wrappers unless they have been separately reviewed to contain no raw
queries, filenames, local paths, tokens, or resume data.
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
F1 must match the aggregate counts within rounding tolerance. The field-level
counts, overall counts, `expected_mentions`, and `predicted_mentions` must also
be internally consistent so an aggregate report cannot claim a larger labeled
surface than its per-field evidence proves. Do not upload reports if they
contain raw resume text, local paths, field values, sample IDs, filenames, or
notes.

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
`target_claim: "ocr_throughput_baseline_observed"` or
`target_claim: "ocr_throughput_target_met"`, `corpus_origin:
"private_local"`, `privacy_boundary: "redacted_local_aggregate"`, false raw
OCR text/page image/path/document-ID/page-ID/command-path booleans, and sha256
digests for the dataset, OCR runtime, renderer, and language-pack manifests
(`dataset_manifest_sha256`, `ocr_runtime_manifest_sha256`,
`renderer_manifest_sha256`, and `language_pack_manifest_sha256`). Do not upload
reports if they contain raw OCR text, page images, resume text, filenames,
local paths, document IDs, page IDs, command paths, runtime paths, or notes.
Small, under-page-floor, or run-budget-exhausted diagnostic reports should use
`target_claim: "not_evaluated"`. The private OCR throughput command emits
`ocr_throughput_baseline_observed` when the representative page floor and
run-budget checks pass but the strict OCR throughput target is not met, and
emits `ocr_throughput_target_met` only when the built-in release OCR page, P95,
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
current-stage OCR baseline evidence blocker. Representative evidence should set
`--max-documents` above the minimum page count so isolated corrupt, encrypted,
render-failed, or OCR-failed PDFs are counted in the redacted failure aggregates
without aborting the whole run. A report with `run_budget_exhausted: true` is
diagnostic local evidence only. A representative report that misses the
latency/throughput thresholds below can clear the current-stage OCR baseline
evidence gate with `ocr_throughput_baseline_observed`, but it does not clear the
follow-up strict performance optimization goal.

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  ocr-gate --report private-ocr-throughput.json \
  --current-stage-baseline \
  --require-private-real-corpus \
  --min-pages 500
```

The strict OCR throughput target remains a separate follow-up performance gate:

```bash
cargo run -p benchmark-runner --bin resume-benchmark --locked -- \
  ocr-gate --report private-ocr-throughput.json \
  --require-private-real-corpus \
  --min-pages 500 --max-p95-ms 1000 --min-pages-per-second 1
```

These gates are evidence validators. They do not run OCR, upload, label, or
sanitize private OCR throughput reports. The strict `ocr-gate` threshold form
above remains the follow-up performance validator; current-stage
release-readiness intake accepts representative baseline evidence with observed
P50/P95/P99 latency and pages-per-second metrics, reviewed runtime/
renderer/language-pack manifests, and no run-budget exhaustion.

Generate a local release dry-run manifest only after release binaries have been
built:

```bash
scripts/release/create-runtime-bundle-manifest.sh \
  --version v0.1.0 \
  --runtime-pack-id reviewed-runtime-pack \
  --distribution-license GPL-3.0-or-later \
  --source-offer reviewed-source-offer.txt \
  --notice reviewed-NOTICE.txt \
  --component tesseract\|ocr-engine\|Apache-2.0\|https://github.com/tesseract-ocr/tesseract\|<local-tesseract-binary> \
  --component eng-tessdata\|ocr-language-pack\|Apache-2.0\|https://github.com/tesseract-ocr/tessdata\|<local-eng-traineddata> \
  --component pdf-renderer\|pdf-renderer\|<reviewed-renderer-license>\|<reviewed-renderer-source>\|<local-pdf-renderer-binary> \
  --out-dir release-dry-run \
  --reviewed
scripts/release/create-artifact-manifest.sh \
  --version v0.1.0 \
  --target-dir target/release \
  --out-dir release-dry-run \
  --runtime-bundle-manifest release-dry-run/runtime-bundle-manifest.json
scripts/release/create-sbom.sh \
  --version v0.1.0 \
  --out-dir release-dry-run \
  --runtime-bundle-manifest release-dry-run/runtime-bundle-manifest.json
```

The generated `runtime-bundle-manifest.json` uses schema
`release.runtime_bundle.v1` and records only component basenames, byte counts,
sha256 hashes, reviewed licenses, sources, notices, and source-offer evidence.
It does not copy runtime binaries into the repository. The generated
`release-artifacts.json` records binary names, byte counts, sha256 hashes, and
the runtime bundle manifest digest under `runtime_bundle_manifests`. The
generated `release-sbom.json` is a redacted SPDX 2.3 package inventory derived
from locked Cargo metadata plus reviewed runtime packages from
`runtime-bundle-manifest.json`. Runtime packages record component names,
checksums, reviewed licenses, sources, and source-offer digests without local
paths or raw runtime binary contents. These dry-run files are not an installer,
signature, notarization ticket, or GitHub Release upload, and they must not
contain local paths, raw runtime binary contents, local data, diagnostics,
model caches, or resume data.

On macOS only, generate unsigned pkg/dmg dry-run artifacts after release
binaries have been built:

```bash
scripts/release/create-macos-package.sh \
  --version v0.1.0 \
  --target-dir target/release \
  --out-dir release-dry-run \
  --runtime-bundle-manifest release-dry-run/runtime-bundle-manifest.json \
  --runtime-bundle-dir <reviewed-runtime-bundle-dir>
```

The generated `macos-package.json` records only artifact filenames, byte counts,
hashes, unsigned status, and still-blocked release steps. When a reviewed
runtime bundle is supplied, it also records `runtime_payload` with schema
`release.runtime_package_payload.v1`, the runtime bundle manifest digest,
component basenames, checksums, reviewed licenses, sources, and the runtime
install location. The pkg/dmg files are local evidence only. They are not
signed, not notarized, not uploaded, and do not prove install, upgrade,
uninstall, rollback, or Gatekeeper behavior.

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
  -OutDir release-dry-run `
  -RuntimeBundleManifest release-dry-run/runtime-bundle-manifest.json `
  -RuntimeBundleDir <reviewed-runtime-bundle-dir>
```

The generated `windows-package.json` records only artifact filenames, byte
counts, hashes, unsigned status, MSI kind, and still-blocked release steps.
When a reviewed runtime bundle is supplied, it also records
`runtime_payload` with schema `release.runtime_package_payload.v1`, the runtime
bundle manifest digest, component basenames, checksums, reviewed licenses,
sources, and the runtime install location. The MSI file is local evidence only.
It is not signed, not uploaded, and does not prove install, upgrade, uninstall,
rollback, Windows service registration, or service lifecycle behavior.

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

Release dry-runs must also write a Windows Service lifecycle dry-run operator
plan:

```powershell
scripts/release/run-windows-service-lifecycle.ps1 `
  -Version v0.1.0 `
  -WindowsPackageManifest release-dry-run/windows-package.json `
  -Out release-dry-run/windows-service-lifecycle-dry-run.json `
  -DryRun
```

The generated plan schema is
`release.windows_service_lifecycle_plan.v1`. It records the Windows package
manifest digest, MSI artifact basenames, planned install/start/status/stop/
recovery/uninstall/rollback actions, the `sc.exe` command boundary, required
administrator approval, and blocked release steps. It must not contain service
tokens, administrator passwords, local paths, raw service logs, resume data,
diagnostics, indexes, model caches, or runtime artifacts. This is an operator
plan only; it does not register, start, stop, query, recover, uninstall, roll
back, or otherwise clear the Windows service lifecycle blocker.

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

This command is governance evidence only. The OCR direction is bundled-first
runtime packaging with external override. A valid OCR runtime manifest must
record checksums, reviewed licenses, `runtime_distribution_mode`,
`runtime_package_binaries_included`, component source, and installer source for
the OCR engine, tessdata language packs, and renderer dependency. The privacy
sentinel `runtime_binaries_included` must remain false in redacted evidence
because evidence packages must not contain runtime binary contents. Tesseract plus
tessdata remains the preferred Apache-2.0 OCR stack. PDFium is the preferred
bundled renderer candidate if quality and platform evidence are sufficient.
Poppler/pdftoppm is operationally strong and widely packaged; if it is bundled,
the release distribution must use a GPL-compatible license such as
GPL-3.0-or-later and include license notices, source-offer obligations, SBOM
entries, exact binary source, artifact checksums, and installer composition
evidence. A valid manifest does not by itself complete non-English OCR quality
validation, platform installer validation, or production OCR throughput proof.
Tesseract plus tessdata is still the preferred OCR engine and language-pack
pair inside either bundled or external runtime mode.

MuPDF and Ghostscript are viable external command alternatives or
commercial-license options in some deployments, but their AGPL/commercial
licensing posture needs explicit release review before bundling.

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
- current-stage private hot-index hybrid benchmark baseline over the available
  local corpus with aggregate P50/P95/P99 metrics
- external 100k and 1M hot-index hybrid benchmark runs on representative
  hardware as follow-up performance-optimization evidence
- private real-corpus OCR throughput gate with reviewed OCR runtime, renderer,
  and language-pack manifests
- OCR and embedding model license review
- OCR runtime manifest checksum validation
- model pack manifest checksum validation

If any item is missing, keep the release blocked and update `PROGRESS.md` with
the exact missing evidence and owner.

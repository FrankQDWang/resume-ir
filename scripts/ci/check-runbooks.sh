#!/usr/bin/env sh
set -eu

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

require_file() {
  if [ ! -f "$1" ]; then
    fail "missing required runbook: $1"
  fi
}

require_text() {
  file="$1"
  text="$2"
  if ! grep -Fq -- "$text" "$file"; then
    fail "runbook $file is missing required text: $text"
  fi
}

diagnostics_runbook="docs/runbooks/diagnostics-redaction.md"
fault_runbook="docs/runbooks/fault-injection.md"
worker_runbook="docs/runbooks/ocr-embedding-workers.md"
release_runbook="docs/runbooks/release-blockers.md"
goal_doc="GOAL.md"
license_doc="02_execution_plan_执行方案/08_依赖许可与参考资料.md"

for file in "$diagnostics_runbook" "$fault_runbook" "$worker_runbook" "$release_runbook" "$goal_doc" "$license_doc"; do
  require_file "$file"
done

for file in "$diagnostics_runbook" "$fault_runbook" "$worker_runbook" "$release_runbook"; do
  require_text "$file" "Local-only"
  require_text "$file" "Do not upload"
  require_text "$file" "Synthetic fixtures"
done

require_text "$diagnostics_runbook" "resume-cli export-diagnostics --redact"
require_text "$diagnostics_runbook" "release-readiness --json"
require_text "$diagnostics_runbook" "--diagnostics-report redacted-diagnostics.json"
require_text "$diagnostics_runbook" "redacted diagnostics evidence"
require_text "$diagnostics_runbook" "raw resume text"
require_text "$diagnostics_runbook" "complete paths"
require_text "$fault_runbook" "resume-cli fault-simulate --case disk-space-low"
require_text "$fault_runbook" "resume-cli fault-simulate --case permission-denied"
require_text "$fault_runbook" "resume-cli fault-simulate --case file-lock"
require_text "$fault_runbook" "resume-cli fault-simulate --case index-snapshot-corrupt"
require_text "$fault_runbook" "resume-cli fault-simulate --case model-checksum"
require_text "$fault_runbook" "resume-cli fault-simulate --case daemon-kill"
require_text "$fault_runbook" "resume-cli fault-simulate --case ocr-crash"
require_text "$fault_runbook" "resume-cli fault-simulate --case battery-mode"
require_text "$fault_runbook" "resume-cli fault-simulate --case external-drive-disconnect"
require_text "$fault_runbook" "real hardware drill: blocked"
require_text "$worker_runbook" "resume-cli ocr-worker --once"
require_text "$worker_runbook" "resume-cli ocr preflight --json"
require_text "$worker_runbook" "ocr-runtime-preflight.v1"
require_text "$worker_runbook" "--tesseract-command <local-tesseract-command>"
require_text "$worker_runbook" "--pdftoppm-command <local-pdftoppm-command>"
require_text "$worker_runbook" "exits nonzero"
require_text "$worker_runbook" "paths as"
require_text "$worker_runbook" "resume-cli ocr draft-manifest --out"
require_text "$worker_runbook" "--runtime-pack-id <reviewed-runtime-pack-id>"
require_text "$worker_runbook" "--language-pack <local-tessdata-file>"
require_text "$worker_runbook" "Omit \`--reviewed\` when legal review is not complete"
require_text "$worker_runbook" "resume-cli ocr validate-manifest --manifest"
require_text "$worker_runbook" "resume-cli model preflight --json"
require_text "$worker_runbook" "embedding-runtime-preflight.v1"
require_text "$worker_runbook" "embedding_protocol"
require_text "$worker_runbook" "resume-ir-embedding-v1"
require_text "$worker_runbook" "--embedding-command <local-embedding-command>"
require_text "$worker_runbook" "--model-id <reviewed-model-id>"
require_text "$worker_runbook" "resume-cli model draft-manifest --out"
require_text "$worker_runbook" "--model-pack-id <reviewed-model-pack-id>"
require_text "$worker_runbook" "--artifact <local-model-artifact>"
require_text "$worker_runbook" "Omit \`--reviewed\` when model weight license review is not complete"
require_text "$worker_runbook" "resume-cli model validate-manifest --manifest"
require_text "$worker_runbook" "resume-daemon"
require_text "$worker_runbook" "FailedRetryable"
require_text "$release_runbook" "BLOCKED"
require_text "$release_runbook" "resume-benchmark gate"
require_text "$release_runbook" 'generation_mode: "streaming"'
require_text "$release_runbook" "field-gate"
require_text "$release_runbook" "--require-private-business-labeled"
require_text "$release_runbook" 'target_claim: "field_quality_target_met"'
require_text "$release_runbook" 'field_taxonomy:'
require_text "$release_runbook" "dedupe-gate"
require_text "$release_runbook" 'target_claim: "dedupe_quality_target_met"'
require_text "$release_runbook" 'dedupe_taxonomy:'
require_text "$release_runbook" "vector-gate"
require_text "$release_runbook" 'target_claim: "vector_quality_target_met"'
require_text "$release_runbook" 'vector_taxonomy:'
require_text "$release_runbook" "model_manifest_sha256"
require_text "$release_runbook" "ocr-gate --report private-ocr-throughput.json"
require_text "$release_runbook" 'target_claim: "ocr_throughput_target_met"'
require_text "$release_runbook" "ocr_runtime_manifest_sha256"
require_text "$release_runbook" "renderer_manifest_sha256"
require_text "$release_runbook" "language_pack_manifest_sha256"
require_text "$release_runbook" 'query_mode: "hybrid"'
require_text "$release_runbook" 'retrieval_layers:'
require_text "$release_runbook" "hot_index: true"
require_text "$release_runbook" "heavy-model-inference"
require_text "$release_runbook" 'target_claim: "benchmark_baseline_observed"'
require_text "$release_runbook" "follow-up performance"
require_text "$release_runbook" "resume-cli --data-dir <local-data-dir> ocr validate-manifest"
require_text "$release_runbook" "resume-cli --data-dir <local-data-dir> model validate-manifest"
require_text "$release_runbook" 'embedding_protocol: "passed"'
require_text "$release_runbook" "Tesseract plus tessdata"
require_text "$release_runbook" "Apache-2.0 external OCR runtime"
require_text "$release_runbook" 'Poppler `pdftoppm`'
require_text "$release_runbook" "not bundled by default"
require_text "$release_runbook" "Current-stage boundary"
require_text "$release_runbook" "local 10k validation baseline"
require_text "$release_runbook" "deferred performance-optimization goal"
require_text "$release_runbook" "resume-cli privacy dataset-manifest"
require_text "$release_runbook" "resume-ir.dataset-manifest.v1"
require_text "$release_runbook" "local_only_redacted_dataset_manifest"
require_text "$release_runbook" "contain local paths, file names, raw resume text, per-file hashes"
require_text "$release_runbook" "resume-cli benchmark-query-set draft"
require_text "$release_runbook" "resume-ir.query-set.jsonl.v1"
require_text "$release_runbook" "local_only_private_query_set"
require_text "$release_runbook" "The draft command excludes names"
require_text "$release_runbook" "emails, phones, local paths, filenames"
require_text "$release_runbook" "Signing and notarization are release-credential blockers"
require_text "$release_runbook" "--model-manifest local-model-manifest.json"
require_text "$release_runbook" "--ocr-runtime-manifest local-ocr-runtime-manifest.json"
require_text "$release_runbook" "reviewed_local_manifest"
require_text "$release_runbook" "signing"
require_text "$release_runbook" "notarization"
require_text "$release_runbook" "Windows"
require_text "$release_runbook" "macOS"
require_text "$worker_runbook" "External runtime decision"
require_text "$worker_runbook" "Tesseract/tessdata is the preferred external OCR runtime"
require_text "$worker_runbook" "Poppler/pdftoppm is an accepted"
require_text "$worker_runbook" "user-installed external PDF renderer"
require_text "$worker_runbook" "MIT project may call a user-installed Poppler command"
require_text "$worker_runbook" "Do not bundle Poppler/pdftoppm"
require_text "$worker_runbook" "default in product installers"
require_text "$worker_runbook" "exact installed Poppler license"
require_text "$worker_runbook" "reviewed status"
require_text "$worker_runbook" "PDFium remains the preferred future permissive-license bundled renderer candidate"
require_text "$goal_doc" "当前阶段不要求极致性能优化"
require_text "$goal_doc" "真实本机 1 万份本地验证流程"
require_text "$goal_doc" "性能极致优化"
require_text "$license_doc" "Poppler/pdftoppm"
require_text "$license_doc" "外部命令"
require_text "$license_doc" "不默认打包"
require_text "$license_doc" "PDFium"
require_text "$license_doc" "后续内置渲染器候选"

printf '%s\n' "runbook check passed"

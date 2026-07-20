import assert from "node:assert/strict";
import { chmod, mkdtemp, readFile, realpath, rm, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  SYNTHETIC_CANARY_FILE_NAME,
  SYNTHETIC_CANARY_TOKEN,
  canaryImportCompleted,
  createSyntheticCanary,
  syntheticCanaryImportRequest,
  validateCanaryImportResponse,
  validateCanarySearchResponse,
} from "./synthetic-canary.mjs";

const epoch = 12;

function readyStatus(overrides = {}) {
  return {
    schema_version: "daemon.status.v2",
    status: "ok",
    visible_epoch: epoch,
    process_state: "ready",
    service_state: "ready",
    services: { metadata: "ready", query: "ready" },
    repair_reason: null,
    repair_progress: null,
    error: null,
    ipc: {
      accepted: 1,
      completed: 1,
      client_disconnect: 0,
      request_failure: 0,
      response_failure: 0,
    },
    indexed_documents: 5,
    searchable_documents: 5,
    partial_documents: 0,
    failed_retryable: 0,
    failed_permanent: 0,
    recovery_queue_depth: 0,
    ocr_queue_depth: 0,
    ocr_jobs_queued: 0,
    ocr_page_budget_blocked: 0,
    ocr_remediation: "none",
    ocr_language_unavailable: 0,
    ocr_language_remediation: "none",
    embedding_queue_depth: 0,
    entity_mentions: 0,
    import_tasks_queued: 0,
    import_tasks_recoverable: 0,
    import_tasks_cancelled: 0,
    import_scan_scopes: 1,
    import_scan_errors: 0,
    query_latency: {
      sample_count: 0,
      p50_ms: null,
      p95_ms: null,
      p99_ms: null,
      last_result_count: null,
      raw_queries: "<redacted>",
    },
    latest_import_scan: {
      scan_profile: "explicit",
      files_discovered: 1,
      ignored_entries: 0,
      scan_errors: 0,
      searchable_documents: 1,
      ocr_required_documents: 0,
      ocr_jobs_queued: 0,
      failed_documents: 0,
      deleted_documents: 0,
      scan_budget_observed: 1,
      scan_budget_limit: 1,
      scan_budget_exhausted: false,
    },
    active_profile: "balanced",
    index_health: "ready",
    snapshot_present: true,
    ...overrides,
  };
}

function searchResponse(results) {
  return {
    schema_version: "resume-ir.search-response.v3",
    request_id: "installed-main-synthetic-witness",
    status: "ok",
    visible_epoch: epoch,
    query_mode: "fulltext",
    partial: false,
    partial_reasons: [],
    latency_ms: 1,
    stage_latency_ms: {
      parse: 0,
      prefilter: 0,
      bm25: 1,
      ann: 0,
      fusion: 0,
      bulk_hydrate: 0,
      snippet: 0,
    },
    search_index: "available",
    result_count: results.length,
    results,
  };
}

test("creates one owner-only fixed synthetic canary outside the cloned data directory", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-canary-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const workspace = {
    root,
    home: root,
    dataDir: path.join(root, "Library", "Application Support", "local.resume-ir.desktop"),
  };
  const canary = await createSyntheticCanary(workspace);
  assert.equal(path.basename(canary.file), SYNTHETIC_CANARY_FILE_NAME);
  assert.equal(path.dirname(canary.file), canary.root);
  assert.equal(canary.root.startsWith(`${root}${path.sep}`), true);
  assert.equal(canary.root.startsWith(`${workspace.dataDir}${path.sep}`), false);
  assert.equal((await stat(canary.root)).mode & 0o777, 0o700);
  assert.equal((await stat(canary.file)).mode & 0o777, 0o600);
  assert.match(await readFile(canary.file, "utf8"), new RegExp(SYNTHETIC_CANARY_TOKEN));
  assert.deepEqual(syntheticCanaryImportRequest(canary), {
    roots: [canary.root],
    profile: "explicit",
    max_files: 1,
  });
});

test("accepts only the exact one-root import receipt and completed one-file publication", () => {
  const taskId = `imp_${"a".repeat(32)}`;
  assert.deepEqual(
    validateCanaryImportResponse({
      schema_version: "daemon.import.v1",
      status: "accepted",
      accepted_roots: 1,
      new_tasks: 1,
      task_ids: [taskId],
      scan_profile: "explicit",
      scan_file_limit: 1,
    }),
    { taskId },
  );
  assert.equal(canaryImportCompleted(readyStatus(), epoch - 1), true);
  assert.equal(canaryImportCompleted(readyStatus(), epoch), false);
  assert.equal(
    canaryImportCompleted(
      readyStatus({
        latest_import_scan: {
          ...readyStatus().latest_import_scan,
          searchable_documents: 0,
        },
      }),
      epoch - 1,
    ),
    false,
  );
});

test("zero-hit, wrong-file, and stale-epoch search responses are not witnesses", () => {
  assert.throws(
    () => validateCanarySearchResponse(searchResponse([]), epoch),
    /search_witness_invalid/,
  );
  const result = {
    rank: 1,
    selection: {
      doc_id: `doc_${"1".repeat(32)}`,
      version_id: `ver_${"2".repeat(32)}`,
      visible_epoch: epoch,
    },
    file_name: SYNTHETIC_CANARY_FILE_NAME,
    snippet: SYNTHETIC_CANARY_TOKEN,
  };
  assert.equal(
    validateCanarySearchResponse(searchResponse([result]), epoch).result_count,
    1,
  );
  assert.throws(
    () =>
      validateCanarySearchResponse(
        searchResponse([{ ...result, file_name: "unrelated.txt" }]),
        epoch,
      ),
    /search_witness_invalid/,
  );
  assert.throws(
    () => validateCanarySearchResponse(searchResponse([result]), epoch + 1),
    /search_witness_invalid/,
  );
});

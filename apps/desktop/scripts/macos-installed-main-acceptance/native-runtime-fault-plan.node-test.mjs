import assert from "node:assert/strict";
import test from "node:test";

import {
  REQUIRED_FAULT_CELLS,
  RUNTIME_FAULT_CASES,
  runtimeFaultCase,
  runtimeFaultStatusMatches,
} from "./native-runtime-fault-plan.mjs";

const RUNTIMES = ["embedding", "ocr", "classifier"];

function capabilitiesFor(runtimes) {
  const available = (name) => runtimes[name].state === "available";
  const embedding = available("embedding");
  const ocr = available("ocr");
  const classifier = available("classifier");
  const importReason = !classifier
    ? "classifier_unavailable"
    : "embedding_unavailable";
  return {
    keyword_search: { state: "available", reason: null },
    detail: { state: "available", reason: null },
    semantic_search: embedding
      ? { state: "available", reason: null }
      : { state: "unavailable", reason: "embedding_unavailable" },
    hybrid_search: embedding
      ? { state: "available", reason: null }
      : { state: "degraded", reason: "embedding_unavailable" },
    text_import: classifier && embedding
      ? { state: "available", reason: null }
      : { state: "unavailable", reason: importReason },
    ocr_import: classifier && embedding && ocr
      ? { state: "available", reason: null }
      : {
          state: "unavailable",
          reason: !classifier
            ? "classifier_unavailable"
            : !embedding
              ? "embedding_unavailable"
              : "ocr_unavailable",
        },
    index_publication: classifier && embedding
      ? { state: "available", reason: null }
      : { state: "unavailable", reason: importReason },
  };
}

function projectedStatus(definition) {
  const optionalRuntimes = Object.fromEntries(
    RUNTIMES.map((name) => [
      name,
      definition.expectedReasons[name]
        ? { state: "unavailable", reason: definition.expectedReasons[name] }
        : { state: "available", reason: null },
    ]),
  );
  return {
    schema_version: "daemon.status.v3",
    status: "ok",
    process_state: "ready",
    core: { state: "ready", reason: null },
    optional_runtimes: optionalRuntimes,
    capabilities: capabilitiesFor(optionalRuntimes),
    error: null,
    repair_progress: null,
    indexed_documents: 1,
    searchable_documents: 1,
    partial_documents: 0,
    visible_epoch: 1,
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
    import_scan_scopes: 0,
    import_scan_errors: 0,
    query_latency: {
      sample_count: 0,
      p50_ms: null,
      p95_ms: null,
      p99_ms: null,
      last_result_count: null,
      raw_queries: "<redacted>",
    },
    latest_import_scan: null,
    active_profile: "balanced",
    index_health: "ready",
    snapshot_present: true,
    ipc: {
      accepted: 0,
      completed: 0,
      client_disconnect: 0,
      request_failure: 0,
      response_failure: 0,
    },
  };
}

test("fault plan covers missing, invalid, start-failed, and representative combinations", () => {
  assert.equal(new Set(REQUIRED_FAULT_CELLS).size, REQUIRED_FAULT_CELLS.length);
  assert.equal(REQUIRED_FAULT_CELLS[0], "slow_initialization");
  for (const runtimeName of RUNTIMES) {
    for (const reason of ["missing", "invalid", "start_failed"]) {
      const definition = runtimeFaultCase(`${runtimeName}_${reason}`);
      assert.equal(definition.expectedReasons[runtimeName], reason);
    }
  }
  for (const cell of [
    "embedding_ocr_missing",
    "embedding_classifier_invalid",
    "all_runtimes_missing",
  ]) {
    assert.ok(Object.keys(runtimeFaultCase(cell).expectedReasons).length >= 2);
  }
  assert.equal(
    runtimeFaultCase("classifier_start_failed").evidenceSource,
    "deterministic_contract_projection",
  );
  assert.ok(
    RUNTIME_FAULT_CASES.filter(({ evidenceSource }) => evidenceSource === "installed_app")
      .length > 0,
  );
});

test("every installed fault definition matches only its exact closed runtime matrix", () => {
  for (const definition of RUNTIME_FAULT_CASES) {
    const exact = projectedStatus(definition);
    assert.equal(runtimeFaultStatusMatches(exact, definition), true, definition.cell);
    const runtimeName = Object.keys(definition.expectedReasons)[0];
    const wrong = structuredClone(exact);
    wrong.optional_runtimes[runtimeName].reason =
      wrong.optional_runtimes[runtimeName].reason === "missing"
        ? "invalid"
        : "missing";
    assert.equal(runtimeFaultStatusMatches(wrong, definition), false, definition.cell);
    const unknown = structuredClone(exact);
    unknown.optional_runtimes.extra = { state: "available", reason: null };
    assert.equal(runtimeFaultStatusMatches(unknown, definition), false);
    const illegalCapability = structuredClone(exact);
    illegalCapability.capabilities.keyword_search = {
      state: "unavailable",
      reason: "embedding_unavailable",
    };
    assert.equal(runtimeFaultStatusMatches(illegalCapability, definition), false);
    assert.equal(
      runtimeFaultStatusMatches({ ...exact, private_debug: true }, definition),
      false,
    );
  }
});

import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { diagnostics } from "./fixtures.mjs";
import { readActiveStoreManifest } from "./filesystem-cow.mjs";
import {
  captureLifecycleReceiptBoundary,
  contentionStatus,
  initializingStatus,
  optionalRuntimeFaultStatus,
  persistentBlockedStatus,
  readDaemonConnection,
  readyStatus,
  requestJsonPostServiceUnavailable,
  validDaemonStatus,
  validateDaemonDiagnostics,
  validateLifecycleReceipt,
  validateLifecycleReceiptBoundary,
} from "./ipc-contracts.mjs";

const LAUNCH = "a".repeat(64);
const INSTANCE = "b".repeat(64);
const TOKEN = "c".repeat(64);

function allCapabilities(state, reason) {
  const capability = { state, reason };
  return {
    keyword_search: { ...capability },
    detail: { ...capability },
    semantic_search: { ...capability },
    hybrid_search: { ...capability },
    text_import: { ...capability },
    ocr_import: { ...capability },
    index_publication: { ...capability },
  };
}

function status(overrides = {}) {
  return {
    schema_version: "daemon.status.v3",
    status: "ok",
    process_state: "ready",
    core: { state: "ready", reason: null },
    optional_runtimes: {
      embedding: { state: "available", reason: null },
      ocr: { state: "available", reason: null },
      classifier: { state: "available", reason: null },
    },
    capabilities: allCapabilities("available", null),
    error: null,
    repair_progress: null,
    indexed_documents: 4,
    searchable_documents: 4,
    partial_documents: 0,
    visible_epoch: 7,
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
    latest_import_scan: null,
    active_profile: "balanced",
    index_health: "ready",
    snapshot_present: true,
    ipc: {
      accepted: 1,
      completed: 1,
      client_disconnect: 0,
      request_failure: 0,
      response_failure: 0,
    },
    ...overrides,
  };
}

function retryStatus(kind, attempt) {
  return status({
    status: "repairing",
    core: { state: "repairing", reason: "artifact_unavailable" },
    capabilities: allCapabilities("initializing", "core_initializing"),
    error: {
      code: "SERVICE_INITIALIZING",
      action: "wait_for_service",
      capability: null,
      reason: "artifact_unavailable",
    },
    repair_progress: {
      phase: "retry_wait",
      attempt,
      max_attempts: 5,
      retry_after_ms: 500,
      last_error_kind: `${kind}_publication_busy`,
    },
  });
}

function blockedStatus(kind) {
  return status({
    status: "blocked",
    core: { state: "blocked", reason: "runtime_invariant" },
    capabilities: allCapabilities("blocked", "core_blocked"),
    error: {
      code: "SERVICE_BLOCKED",
      action: "repair_required",
      capability: null,
      reason: "runtime_invariant",
    },
    repair_progress: {
      phase: "blocked",
      attempt: 5,
      max_attempts: 5,
      retry_after_ms: null,
      last_error_kind: `${kind}_publication_busy`,
    },
  });
}

function missingRuntimeStatus(runtimeName) {
  const capabilities = allCapabilities("available", null);
  if (runtimeName === "embedding") {
    capabilities.semantic_search = {
      state: "unavailable",
      reason: "embedding_unavailable",
    };
    capabilities.hybrid_search = {
      state: "degraded",
      reason: "embedding_unavailable",
    };
    for (const name of ["text_import", "ocr_import", "index_publication"]) {
      capabilities[name] = {
        state: "unavailable",
        reason: "embedding_unavailable",
      };
    }
  } else if (runtimeName === "ocr") {
    capabilities.ocr_import = {
      state: "unavailable",
      reason: "ocr_unavailable",
    };
  } else {
    for (const name of ["text_import", "ocr_import", "index_publication"]) {
      capabilities[name] = {
        state: "unavailable",
        reason: "classifier_unavailable",
      };
    }
  }
  return status({
    optional_runtimes: {
      embedding: {
        state: runtimeName === "embedding" ? "unavailable" : "available",
        reason: runtimeName === "embedding" ? "missing" : null,
      },
      ocr: {
        state: runtimeName === "ocr" ? "unavailable" : "available",
        reason: runtimeName === "ocr" ? "missing" : null,
      },
      classifier: {
        state: runtimeName === "classifier" ? "unavailable" : "available",
        reason: runtimeName === "classifier" ? "missing" : null,
      },
    },
    capabilities,
  });
}

function receipt(events) {
  return {
    schema_version: "resume-ir.desktop-daemon-lifecycle-receipt.v2",
    persistence_state: "ready",
    dropped_event_count: 0,
    events,
  };
}

function lifecycleEvent(at, state, transitionReason, generation, overrides = {}) {
  return {
    at_unix_ms: at,
    state,
    transition_reason: transitionReason,
    generation,
    automatic_restart_attempt: 0,
    automatic_restart_limit: 5,
    retry_after_ms: null,
    heartbeat_failures: 0,
    last_exit: null,
    ...overrides,
  };
}

test("status v3 distinguishes ready, repairing, and blocked without daemon restart", () => {
  assert.equal(readyStatus(status()), true);
  assert.equal(validDaemonStatus(status()), true);
  assert.equal(readyStatus({ ...status(), schema_version: "daemon.status.v2" }), false);
  assert.equal(validDaemonStatus({ ...status(), private_debug: true }), false);
  assert.equal(
    validDaemonStatus({
      ...status(),
      capabilities: {
        ...status().capabilities,
        keyword_search: {
          state: "unavailable",
          reason: "embedding_unavailable",
        },
      },
    }),
    false,
  );
  assert.equal(contentionStatus(retryStatus("fulltext", 1), "fulltext", 1), true);
  assert.equal(persistentBlockedStatus(blockedStatus("vector"), "vector"), true);
  assert.equal(contentionStatus({ ...retryStatus("fulltext", 1), private_debug: true }, "fulltext", 1), false);
});

async function serviceResponse(statusCode, body) {
  const server = http.createServer((_request, response) => {
    response.writeHead(statusCode, { "Content-Type": "application/json" });
    response.end(body);
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  return {
    close: () => new Promise((resolve) => server.close(resolve)),
    url: new URL(`http://127.0.0.1:${address.port}/imports`),
  };
}

test("service-unavailable POST helper accepts only exact HTTP 503 JSON", async (context) => {
  const expected = {
    schema_version: "resume-ir.error.v2",
    status: "error",
    error: {
      code: "CAPABILITY_UNAVAILABLE",
      action: "select_supported_mode",
      capability: "text_import",
      reason: "classifier_unavailable",
    },
  };
  const unavailable = await serviceResponse(503, JSON.stringify(expected));
  context.after(unavailable.close);
  assert.deepEqual(
    await requestJsonPostServiceUnavailable(
      unavailable.url,
      TOKEN,
      { roots: ["/private/synthetic"] },
    ),
    expected,
  );

  const success = await serviceResponse(202, JSON.stringify(expected));
  context.after(success.close);
  await assert.rejects(
    requestJsonPostServiceUnavailable(
      success.url,
      TOKEN,
      { roots: ["/private/synthetic"] },
    ),
    /daemon_response_invalid/,
  );

  const malformed = await serviceResponse(503, "not-json");
  context.after(malformed.close);
  await assert.rejects(
    requestJsonPostServiceUnavailable(
      malformed.url,
      TOKEN,
      { roots: ["/private/synthetic"] },
    ),
    /daemon_response_invalid/,
  );
});

test("slow initialization and every missing optional runtime require exact matrices", () => {
  const initializing = status({
    status: "initializing",
    core: { state: "initializing", reason: "metadata_initializing" },
    optional_runtimes: {
      embedding: { state: "initializing", reason: null },
      ocr: { state: "initializing", reason: null },
      classifier: { state: "initializing", reason: null },
    },
    capabilities: allCapabilities("initializing", "core_initializing"),
    error: {
      code: "SERVICE_INITIALIZING",
      action: "wait_for_service",
      capability: null,
      reason: "metadata_initializing",
    },
  });
  assert.equal(initializingStatus(initializing), true);
  for (const runtimeName of ["embedding", "ocr", "classifier"]) {
    const observed = missingRuntimeStatus(runtimeName);
    assert.equal(optionalRuntimeFaultStatus(observed, runtimeName), true);
    assert.equal(
      optionalRuntimeFaultStatus(
        {
          ...observed,
          optional_runtimes: {
            ...observed.optional_runtimes,
            [runtimeName]: { state: "unavailable", reason: "invalid" },
          },
        },
        runtimeName,
      ),
      false,
    );
  }
});

test("diagnostics v4 validates exact health and privacy matrices", () => {
  assert.equal(validateDaemonDiagnostics(diagnostics()).schema_version, "resume-ir.diagnostics.v4");
  assert.throws(
    () => validateDaemonDiagnostics(diagnostics({ schema_version: "resume-ir.diagnostics.v3" })),
    /diagnostics_contract_invalid/,
  );
  assert.throws(
    () => validateDaemonDiagnostics(diagnostics({ contains_queries: true })),
    /diagnostics_contract_invalid/,
  );
  const retry = retryStatus("fulltext", 1);
  assert.equal(
    validateDaemonDiagnostics(diagnostics({
      core: retry.core,
      capabilities: retry.capabilities,
      error: retry.error,
      repair_progress: retry.repair_progress,
    }), [], { phase: "retry_wait", kind: "fulltext", attempt: 1 }).core.state,
    "repairing",
  );

  const embeddingFaultCapabilities = allCapabilities("available", null);
  embeddingFaultCapabilities.semantic_search = { state: "unavailable", reason: "embedding_unavailable" };
  embeddingFaultCapabilities.hybrid_search = { state: "degraded", reason: "embedding_unavailable" };
  for (const operation of ["text_import", "ocr_import", "index_publication"]) {
    embeddingFaultCapabilities[operation] = { state: "unavailable", reason: "embedding_unavailable" };
  }
  const embeddingFault = diagnostics({
    optional_runtimes: {
      embedding: { state: "unavailable", reason: "invalid" },
      ocr: { state: "available", reason: null },
      classifier: { state: "available", reason: null },
    },
    capabilities: embeddingFaultCapabilities,
  });
  assert.equal(validateDaemonDiagnostics(embeddingFault).capabilities.hybrid_search.state, "degraded");
  assert.throws(
    () => validateDaemonDiagnostics({
      ...embeddingFault,
      capabilities: {
        ...embeddingFaultCapabilities,
        text_import: { state: "available", reason: null },
      },
    }),
    /diagnostics_contract_invalid/,
  );
});

test("lifecycle receipt v2 proves one post-boundary retry and new generation", () => {
  const before = receipt([
    lifecycleEvent(100, "starting", "initial_start", 0),
    lifecycleEvent(200, "running", "control_plane_ready", 1),
  ]);
  const source = `${JSON.stringify(before)}\n`;
  const boundary = captureLifecycleReceiptBoundary({
    capturedAtUnixMs: 250,
    source,
    value: before,
  });
  const after = receipt([
    ...before.events,
    lifecycleEvent(300, "retry_wait", "child_exited", 1, {
      automatic_restart_attempt: 1,
      retry_after_ms: 250,
      last_exit: "child_exited",
    }),
    lifecycleEvent(600, "running", "control_plane_ready", 2, {
      automatic_restart_attempt: 1,
      last_exit: "child_exited",
    }),
  ]);
  const afterSource = `${JSON.stringify(after)}\n`;
  assert.equal(
    validateLifecycleReceiptBoundary({ boundary, source: afterSource, value: after }),
    after,
  );
  assert.equal(validateLifecycleReceipt(after), after);
  assert.throws(
    () => validateLifecycleReceipt({ ...after, schema_version: "resume-ir.desktop-daemon-lifecycle-receipt.v1" }),
    /lifecycle_receipt_invalid/,
  );
});

test("active store reader accepts only exact private v29 authority", async (context) => {
  const root = await realpath(await mkdtemp(path.join(os.tmpdir(), "resume-ir-v29-manifest-")));
  context.after(() => rm(root, { recursive: true, force: true }));
  const digest = "1".repeat(64);
  const file = path.join(root, "metadata-active.v1");
  await writeFile(file, `resume-ir.metadata-active.v1\nfile=metadata-v29-${digest.slice(0, 16)}.sqlite3\nschema=29\ndigest=${digest}\n`, { mode: 0o600 });
  await chmod(file, 0o600);
  assert.deepEqual(await readActiveStoreManifest(root), {
    fileName: `metadata-v29-${digest.slice(0, 16)}.sqlite3`,
    schema: 29,
    digest,
  });
  await writeFile(file, `resume-ir.metadata-active.v1\nfile=metadata-v28-${digest.slice(0, 16)}.sqlite3\nschema=28\ndigest=${digest}\n`, { mode: 0o600 });
  await assert.rejects(
    readActiveStoreManifest(root),
    /active_store_manifest_invalid/,
  );
});

test("discovery/auth v3 require one launch, instance, token, and loopback origin", async (context) => {
  const root = await realpath(await mkdtemp(path.join(os.tmpdir(), "resume-ir-v3-discovery-")));
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(root, { recursive: true });
  const endpoints = {
    schema_version: "resume-ir.daemon-ipc.v3",
    launch_id: LAUNCH,
    instance_id: INSTANCE,
    owner_mode: "desktop_supervised",
    status: "http://127.0.0.1:4312/status",
    diagnostics: "http://127.0.0.1:4312/diagnostics",
    imports: "http://127.0.0.1:4312/imports",
    import_cancel: "http://127.0.0.1:4312/imports/cancel",
    import_control: "http://127.0.0.1:4312/imports/control",
    import_progress: "http://127.0.0.1:4312/imports/progress",
    search: "http://127.0.0.1:4312/search",
    search_batch: "http://127.0.0.1:4312/search/batch",
    details: "http://127.0.0.1:4312/details",
    delete: "http://127.0.0.1:4312/delete",
  };
  const auth = {
    schema_version: "resume-ir.daemon-auth.v3",
    launch_id: LAUNCH,
    instance_id: INSTANCE,
    token: TOKEN,
  };
  await writeFile(path.join(root, "ipc.endpoints.json"), JSON.stringify(endpoints), { mode: 0o600 });
  await writeFile(path.join(root, "ipc.auth"), JSON.stringify(auth), { mode: 0o600 });
  await chmod(path.join(root, "ipc.endpoints.json"), 0o600);
  await chmod(path.join(root, "ipc.auth"), 0o600);
  const connection = await readDaemonConnection(root);
  assert.equal(connection.launchId, LAUNCH);
  assert.equal(connection.instanceId, INSTANCE);

  await writeFile(path.join(root, "ipc.auth"), JSON.stringify({ ...auth, launch_id: "d".repeat(64) }), { mode: 0o600 });
  await chmod(path.join(root, "ipc.auth"), 0o600);
  await assert.rejects(readDaemonConnection(root), /daemon_discovery_invalid/);
});

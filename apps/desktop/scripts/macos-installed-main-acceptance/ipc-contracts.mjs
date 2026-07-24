import http from "node:http";
import { createHash } from "node:crypto";
import path from "node:path";

import {
  AUTH_FILE,
  CONTENTION_ERROR_KINDS,
  DIGEST,
  ENDPOINT_FILE,
  HTTP_TIMEOUT_MS,
  MAX_DIAGNOSTICS_BYTES,
  MAX_HTTP_BYTES,
  AcceptanceError,
  asAcceptanceError,
  exactKeys,
  fail,
  throwIfAborted,
} from "./core.mjs";
import { readPrivateJson } from "./filesystem-cow.mjs";

const CORE_REASONS = [
  "metadata_initializing",
  "migration_rebuild",
  "artifact_unavailable",
  "source_unavailable",
  "runtime_invariant",
  "unsupported_store_schema",
  "metadata_unavailable",
];
const RUNTIME_NAMES = ["embedding", "ocr", "classifier"];
const CAPABILITY_NAMES = [
  "keyword_search",
  "detail",
  "semantic_search",
  "hybrid_search",
  "text_import",
  "ocr_import",
  "index_publication",
];
const STATUS_KEYS = [
  "schema_version", "status", "process_state", "core", "optional_runtimes",
  "capabilities", "error", "repair_progress", "indexed_documents",
  "searchable_documents", "partial_documents", "visible_epoch",
  "failed_retryable", "failed_permanent", "recovery_queue_depth",
  "ocr_queue_depth", "ocr_jobs_queued", "ocr_page_budget_blocked",
  "ocr_remediation", "ocr_language_unavailable", "ocr_language_remediation",
  "embedding_queue_depth", "entity_mentions", "import_tasks_queued",
  "import_tasks_recoverable", "import_tasks_cancelled", "import_scan_scopes",
  "import_scan_errors", "query_latency", "latest_import_scan", "active_profile",
  "index_health", "snapshot_present", "ipc",
];
const REPAIR_PHASES = [
  "queued", "migration_rebuild", "source_unavailable", "rebuilding",
  "retry_wait", "blocked",
];
const REPAIR_ERROR_KINDS = [
  "fulltext_publication_busy", "fulltext_failure", "vector_publication_busy",
  "vector_failure", "metadata_failure", "interrupted",
];

function endpointPath(value, expectedPath) {
  let url;
  try {
    url = new URL(value);
  } catch {
    fail("daemon_discovery_invalid");
  }
  if (
    url.protocol !== "http:" || url.hostname !== "127.0.0.1" ||
    url.username !== "" || url.password !== "" || url.search !== "" ||
    url.hash !== "" || url.pathname !== expectedPath ||
    !/^[1-9][0-9]{0,4}$/.test(url.port) || Number(url.port) > 65_535
  ) {
    fail("daemon_discovery_invalid");
  }
  return url;
}

export async function readDaemonConnection(dataDir) {
  const manifestFile = path.join(dataDir, ENDPOINT_FILE);
  const authFile = path.join(dataDir, AUTH_FILE);
  const before = await readPrivateJson(manifestFile);
  const auth = await readPrivateJson(authFile);
  const after = await readPrivateJson(manifestFile);
  const endpointKeys = [
    "schema_version", "launch_id", "instance_id", "owner_mode", "status",
    "diagnostics", "imports", "import_cancel", "import_control",
    "import_progress", "search", "search_batch", "details", "delete",
  ];
  if (
    before.source !== after.source || !exactKeys(before.value, endpointKeys) ||
    !exactKeys(auth.value, ["schema_version", "launch_id", "instance_id", "token"]) ||
    before.value.schema_version !== "resume-ir.daemon-ipc.v3" ||
    auth.value.schema_version !== "resume-ir.daemon-auth.v3" ||
    before.value.owner_mode !== "desktop_supervised" ||
    !DIGEST.test(before.value.launch_id ?? "") ||
    auth.value.launch_id !== before.value.launch_id ||
    !DIGEST.test(before.value.instance_id ?? "") ||
    auth.value.instance_id !== before.value.instance_id ||
    !DIGEST.test(auth.value.token ?? "")
  ) {
    fail("daemon_discovery_invalid");
  }
  const routes = [
    ["status", "/status"], ["diagnostics", "/diagnostics"],
    ["imports", "/imports"], ["import_cancel", "/imports/cancel"],
    ["import_control", "/imports/control"],
    ["import_progress", "/imports/progress"], ["search", "/search"],
    ["search_batch", "/search/batch"], ["details", "/details"],
    ["delete", "/delete"],
  ];
  const urls = Object.fromEntries(
    routes.map(([key, expected]) => [key, endpointPath(before.value[key], expected)]),
  );
  if (!Object.values(urls).every((url) => url.origin === urls.status.origin)) {
    fail("daemon_discovery_invalid");
  }
  return Object.freeze({
    launchId: before.value.launch_id,
    instanceId: before.value.instance_id,
    token: auth.value.token,
    urls,
  });
}

async function requestJsonWithBody(
  url,
  token,
  { body, expectedStatus = 200, method = "GET", signal, timeoutMs = HTTP_TIMEOUT_MS } = {},
) {
  const payload = body === undefined ? undefined : JSON.stringify(body);
  if (
    !["GET", "POST"].includes(method) ||
    ![200, 202, 503].includes(expectedStatus) ||
    (method === "GET" && payload !== undefined) ||
    (payload !== undefined && Buffer.byteLength(payload, "utf8") > 64 * 1024)
  ) {
    fail("daemon_request_invalid");
  }
  throwIfAborted(signal);
  return new Promise((resolve, reject) => {
    let abortListener;
    const settle = (callback, value) => {
      if (signal && abortListener) signal.removeEventListener("abort", abortListener);
      callback(value);
    };
    const request = http.request(
      url,
      {
        headers: {
          ...(token === undefined ? {} : { Authorization: `Bearer ${token}` }),
          ...(payload === undefined ? {} : {
            "Content-Length": Buffer.byteLength(payload, "utf8"),
            "Content-Type": "application/json",
          }),
        },
        method,
      },
      (response) => {
        const chunks = [];
        let bytes = 0;
        response.on("data", (chunk) => {
          bytes += chunk.length;
          if (bytes > MAX_HTTP_BYTES) {
            request.destroy();
            settle(reject, new AcceptanceError("daemon_response_invalid"));
            return;
          }
          chunks.push(chunk);
        });
        response.on("end", () => {
          if (response.statusCode !== expectedStatus) {
            settle(reject, new AcceptanceError("daemon_response_invalid"));
            return;
          }
          try {
            settle(resolve, JSON.parse(Buffer.concat(chunks, bytes).toString("utf8")));
          } catch {
            settle(reject, new AcceptanceError("daemon_response_invalid"));
          }
        });
      },
    );
    request.setTimeout(timeoutMs, () =>
      request.destroy(new AcceptanceError("daemon_response_timeout")));
    request.once("error", (error) => settle(reject, asAcceptanceError(error)));
    abortListener = () => request.destroy(new AcceptanceError("acceptance_interrupted"));
    signal?.addEventListener("abort", abortListener, { once: true });
    if (signal?.aborted) return abortListener();
    if (payload !== undefined) request.write(payload);
    request.end();
  });
}

export async function requestJson(url, token, timeoutMs = HTTP_TIMEOUT_MS, signal) {
  return requestJsonWithBody(url, token, { signal, timeoutMs });
}

export async function requestJsonPost(url, token, body, { signal } = {}) {
  return requestJsonWithBody(url, token, { body, method: "POST", signal });
}

export async function requestJsonPostAccepted(url, token, body, { signal } = {}) {
  return requestJsonWithBody(url, token, {
    body,
    expectedStatus: 202,
    method: "POST",
    signal,
  });
}

export async function requestJsonPostServiceUnavailable(
  url,
  token,
  body,
  { signal } = {},
) {
  return requestJsonWithBody(url, token, {
    body,
    expectedStatus: 503,
    method: "POST",
    signal,
  });
}

function boundedCount(value, maximum = Number.MAX_SAFE_INTEGER) {
  return Number.isSafeInteger(value) && value >= 0 && value <= maximum;
}

function nullableCount(value, maximum) {
  return value === null || boundedCount(value, maximum);
}

function validIpc(value) {
  return exactKeys(value, [
    "accepted", "completed", "client_disconnect", "request_failure", "response_failure",
  ]) && Object.values(value).every((count) => boundedCount(count));
}

function validCore(core) {
  if (!exactKeys(core, ["state", "reason"])) return false;
  if (core.state === "ready") return core.reason === null;
  if (core.state === "initializing") return core.reason === "metadata_initializing";
  if (core.state === "repairing") {
    return ["migration_rebuild", "artifact_unavailable"].includes(core.reason);
  }
  return ["degraded", "blocked"].includes(core.state) &&
    ["source_unavailable", "runtime_invariant", "unsupported_store_schema", "metadata_unavailable"].includes(core.reason);
}

function validRuntime(runtime) {
  return exactKeys(runtime, ["state", "reason"]) &&
    ((["initializing", "available"].includes(runtime.state) && runtime.reason === null) ||
      (runtime.state === "unavailable" &&
        ["missing", "invalid", "start_failed", "not_configured"].includes(runtime.reason)));
}

function validCapability(capability) {
  return exactKeys(capability, ["state", "reason"]) &&
    ((capability.state === "available" && capability.reason === null) ||
      (capability.state === "initializing" && capability.reason === "core_initializing") ||
      (["degraded", "unavailable", "blocked"].includes(capability.state) &&
        ["core_blocked", "embedding_unavailable", "ocr_unavailable", "classifier_unavailable"].includes(capability.reason)));
}

function validHealth(value) {
  return value.process_state === "ready" && validCore(value.core) &&
    exactKeys(value.optional_runtimes, RUNTIME_NAMES) &&
    RUNTIME_NAMES.every((name) => validRuntime(value.optional_runtimes[name])) &&
    exactKeys(value.capabilities, CAPABILITY_NAMES) &&
    CAPABILITY_NAMES.every((name) => validCapability(value.capabilities[name])) &&
    capabilityMatrixMatches(value.core, value.optional_runtimes, value.capabilities);
}

function capabilityIs(value, state, reason) {
  return value.state === state && value.reason === reason;
}

function capabilityMatrixMatches(core, runtimes, capabilities) {
  if (["initializing", "repairing"].includes(core.state)) {
    return CAPABILITY_NAMES.every((name) =>
      capabilityIs(capabilities[name], "initializing", "core_initializing"));
  }
  if (["degraded", "blocked"].includes(core.state)) {
    return CAPABILITY_NAMES.every((name) =>
      capabilityIs(capabilities[name], "blocked", "core_blocked"));
  }
  if (!capabilityIs(capabilities.keyword_search, "available", null) ||
      !capabilityIs(capabilities.detail, "available", null)) return false;
  const embedding = runtimes.embedding.state === "available";
  const classifier = runtimes.classifier.state === "available";
  const ocr = runtimes.ocr.state === "available";
  return capabilityIs(capabilities.semantic_search,
    embedding ? "available" : "unavailable",
    embedding ? null : "embedding_unavailable") &&
    capabilityIs(capabilities.hybrid_search,
      embedding ? "available" : "degraded",
      embedding ? null : "embedding_unavailable") &&
    capabilityIs(capabilities.text_import,
      classifier && embedding ? "available" : "unavailable",
      !classifier ? "classifier_unavailable" : !embedding ? "embedding_unavailable" : null) &&
    capabilityIs(capabilities.ocr_import,
      classifier && embedding && ocr ? "available" : "unavailable",
      !classifier ? "classifier_unavailable" : !embedding ? "embedding_unavailable" : !ocr ? "ocr_unavailable" : null) &&
    capabilityIs(capabilities.index_publication,
      classifier && embedding ? "available" : "unavailable",
      !classifier ? "classifier_unavailable" : !embedding ? "embedding_unavailable" : null);
}

function validServiceError(error, core) {
  if (core.state === "ready") return error === null;
  if (!exactKeys(error, ["code", "action", "capability", "reason"]) ||
      error.capability !== null || error.reason !== core.reason) return false;
  if (["initializing", "repairing"].includes(core.state)) {
    return error.code === "SERVICE_INITIALIZING" && error.action === "wait_for_service";
  }
  return error.code === "SERVICE_BLOCKED" &&
    error.action === (core.state === "degraded" ? "retry" : "repair_required");
}

function validRepairProgress(progress, coreState) {
  if (progress === null) return coreState !== "repairing";
  return exactKeys(progress, [
    "phase", "attempt", "max_attempts", "retry_after_ms", "last_error_kind",
  ]) && REPAIR_PHASES.includes(progress.phase) &&
    nullableCount(progress.attempt, 5) &&
    (progress.max_attempts === null || progress.max_attempts === 5) &&
    nullableCount(progress.retry_after_ms, 60_000) &&
    (progress.last_error_kind === null || REPAIR_ERROR_KINDS.includes(progress.last_error_kind));
}

function validStatusShape(value) {
  if (!exactKeys(value, STATUS_KEYS) || value.schema_version !== "daemon.status.v3" ||
      !validHealth(value) || !validServiceError(value.error, value.core) ||
      !validRepairProgress(value.repair_progress, value.core.state) || !validIpc(value.ipc)) {
    return false;
  }
  const expectedStatus = {
    initializing: "initializing", ready: "ok", repairing: "repairing",
    degraded: "degraded", blocked: "blocked",
  }[value.core.state];
  return value.status === expectedStatus;
}

export function validDaemonStatus(value) {
  return validStatusShape(value);
}

export function readyStatus(status) {
  return validStatusShape(status) && status.core.state === "ready" &&
    RUNTIME_NAMES.every((name) => status.optional_runtimes[name].state === "available") &&
    CAPABILITY_NAMES.every((name) => status.capabilities[name].state === "available") &&
    status.repair_progress === null && status.error === null &&
    status.index_health === "ready" && status.snapshot_present === true;
}

export function initializingStatus(status) {
  return (
    validStatusShape(status) &&
    status.core.state === "initializing" &&
    status.core.reason === "metadata_initializing" &&
    status.error.code === "SERVICE_INITIALIZING" &&
    status.error.action === "wait_for_service"
  );
}

export function optionalRuntimeFaultStatus(status, runtimeName) {
  if (
    !RUNTIME_NAMES.includes(runtimeName) ||
    !validStatusShape(status) ||
    status.core.state !== "ready" ||
    status.error !== null ||
    status.repair_progress !== null ||
    status.index_health !== "ready" ||
    status.snapshot_present !== true
  ) {
    return false;
  }
  return RUNTIME_NAMES.every((name) => {
    const runtime = status.optional_runtimes[name];
    return name === runtimeName
      ? runtime.state === "unavailable" && runtime.reason === "missing"
      : runtime.state === "available" && runtime.reason === null;
  });
}

function exactContentionProgress(progress, kind, phase, attempt) {
  return CONTENTION_ERROR_KINDS[kind] !== undefined &&
    exactKeys(progress, [
      "phase", "attempt", "max_attempts", "retry_after_ms", "last_error_kind",
    ]) && progress.phase === phase && progress.attempt === attempt &&
    progress.max_attempts === 5 && progress.last_error_kind === CONTENTION_ERROR_KINDS[kind];
}

export function contentionStatus(status, kind, expectedAttempt) {
  const progress = status?.repair_progress;
  return validStatusShape(status) && status.core.state === "repairing" &&
    status.core.reason === "artifact_unavailable" &&
    Number.isSafeInteger(expectedAttempt) && expectedAttempt >= 1 && expectedAttempt < 5 &&
    exactContentionProgress(progress, kind, "retry_wait", expectedAttempt) &&
    boundedCount(progress.retry_after_ms, 60_000) &&
    status.error.code === "SERVICE_INITIALIZING" &&
    status.error.action === "wait_for_service";
}

export function persistentBlockedStatus(status, kind) {
  const progress = status?.repair_progress;
  return validStatusShape(status) && status.core.state === "blocked" &&
    status.core.reason === "runtime_invariant" &&
    exactContentionProgress(progress, kind, "blocked", 5) &&
    progress.retry_after_ms === null && status.error.code === "SERVICE_BLOCKED" &&
    status.error.action === "repair_required";
}

function scanForPrivateStrings(value, forbidden) {
  const visit = (candidate) => {
    if (typeof candidate === "string") {
      if (
        candidate.startsWith("/") || /^[A-Za-z]:[\\/]/.test(candidate) ||
        candidate.startsWith("file:") ||
        forbidden.some((secret) => secret && candidate.includes(secret))
      ) fail("diagnostics_privacy_invalid");
      return;
    }
    if (Array.isArray(candidate)) return candidate.forEach(visit);
    if (candidate !== null && typeof candidate === "object") {
      Object.values(candidate).forEach(visit);
    }
  };
  visit(value);
}

function diagnosticsPhaseValid(value, expected) {
  if (expected.phase === "ready") return value.core.state === "ready";
  if (expected.phase === "retry_wait") {
    return value.core.state === "repairing" && value.core.reason === "artifact_unavailable" &&
      exactContentionProgress(value.repair_progress, expected.kind, "retry_wait", expected.attempt);
  }
  if (expected.phase === "blocked") {
    return value.core.state === "blocked" && value.core.reason === "runtime_invariant" &&
      exactContentionProgress(value.repair_progress, expected.kind, "blocked", 5);
  }
  return false;
}

export function validateDaemonDiagnostics(
  value,
  forbidden = [],
  expected = { phase: "ready" },
) {
  const serialized = JSON.stringify(value);
  const metricsKeys = [
    "ipc", "indexed_documents", "searchable_documents", "partial_documents",
    "ocr_queue_depth", "embedding_queue_depth", "recovery_queue_depth",
    "import_tasks_queued", "import_tasks_recoverable", "import_tasks_cancelled",
    "query_latency",
  ];
  const errorKeys = [
    "failed_retryable", "failed_permanent", "import_scan_errors",
    "ocr_page_budget_blocked", "ocr_language_unavailable", "scan_error_buckets",
  ];
  const latency = value?.metrics?.query_latency;
  const buckets = value?.error_counts?.scan_error_buckets;
  if (
    typeof serialized !== "string" || Buffer.byteLength(serialized, "utf8") > MAX_DIAGNOSTICS_BYTES ||
    !exactKeys(value, [
      "schema_version", "privacy_boundary", "contains_raw_resume_text",
      "contains_queries", "contains_resume_paths", "contains_candidate_results",
      "contains_snippet_text", "visible_epoch", "evidence_lane", "evidence_status",
      "process_state", "core", "optional_runtimes", "capabilities",
      "repair_progress", "error", "metrics", "error_counts", "benchmark_refs",
    ]) || value.schema_version !== "resume-ir.diagnostics.v4" ||
    value.privacy_boundary !== "redacted_local_aggregate" ||
    [value.contains_raw_resume_text, value.contains_queries, value.contains_resume_paths,
      value.contains_candidate_results, value.contains_snippet_text].some(Boolean) ||
    value.evidence_lane !== "gui_manual" || value.evidence_status !== "unaccepted" ||
    !boundedCount(value.visible_epoch) || !validHealth(value) ||
    !validServiceError(value.error, value.core) ||
    !validRepairProgress(value.repair_progress, value.core.state) ||
    !diagnosticsPhaseValid(value, expected) || !exactKeys(value.metrics, metricsKeys) ||
    !validIpc(value.metrics.ipc) ||
    !metricsKeys.filter((key) => !["ipc", "query_latency"].includes(key))
      .every((key) => boundedCount(value.metrics[key])) ||
    !exactKeys(latency, ["sample_count", "p50_ms", "p95_ms", "p99_ms", "last_result_count"]) ||
    !boundedCount(latency.sample_count) ||
    ![latency.p50_ms, latency.p95_ms, latency.p99_ms].every((entry) =>
      entry === null || (typeof entry === "number" && Number.isFinite(entry) && entry >= 0 && entry <= 3_600_000)) ||
    !nullableCount(latency.last_result_count) || !exactKeys(value.error_counts, errorKeys) ||
    !errorKeys.filter((key) => key !== "scan_error_buckets")
      .every((key) => boundedCount(value.error_counts[key])) ||
    !Array.isArray(buckets) || buckets.length > 16 ||
    !buckets.every((bucket) =>
      exactKeys(bucket, ["class", "operation", "count"]) &&
      ["permission_denied", "source_unavailable", "locked_or_unreadable", "io"].includes(bucket.class) &&
      ["normalize_path", "read_directory", "read_metadata", "fingerprint"].includes(bucket.operation) &&
      boundedCount(bucket.count)) ||
    !Array.isArray(value.benchmark_refs) || value.benchmark_refs.length !== 0
  ) {
    fail("diagnostics_contract_invalid");
  }
  scanForPrivateStrings(value, forbidden);
  return value;
}

function lifecycleReasonValid(event) {
  const reasonByState = {
    starting: ["initial_start", "automatic_retry", "manual_retry", "half_open_retry"],
    running: ["control_plane_ready"],
    retry_wait: ["child_exited", "startup_timeout", "heartbeat_timeout", "start_failed", "control_plane_failure"],
    circuit_open: ["restart_budget_exhausted"],
    blocked: ["configuration_invalid", "runtime_integrity", "protocol_mismatch", "ownership_conflict", "supervisor_unavailable"],
  };
  return reasonByState[event.state]?.includes(event.transition_reason) === true;
}

function validateLifecycleReceiptShape(value, forbidden) {
  const exits = [
    "child_exited", "startup_timeout", "heartbeat_timeout", "start_failed",
    "control_plane_failure",
  ];
  if (
    !exactKeys(value, ["schema_version", "persistence_state", "dropped_event_count", "events"]) ||
    value.schema_version !== "resume-ir.desktop-daemon-lifecycle-receipt.v2" ||
    value.persistence_state !== "ready" || !boundedCount(value.dropped_event_count) ||
    !Array.isArray(value.events) || value.events.length > 16 ||
    !value.events.every((event) =>
      exactKeys(event, [
        "at_unix_ms", "state", "transition_reason", "generation",
        "automatic_restart_attempt", "automatic_restart_limit", "retry_after_ms",
        "heartbeat_failures", "last_exit",
      ]) && boundedCount(event.at_unix_ms) && event.at_unix_ms > 0 &&
      lifecycleReasonValid(event) && boundedCount(event.generation) &&
      boundedCount(event.automatic_restart_attempt, 5) &&
      event.automatic_restart_limit === 5 && nullableCount(event.retry_after_ms, 300_000) &&
      boundedCount(event.heartbeat_failures, 3) &&
      (event.last_exit === null || exits.includes(event.last_exit))) ||
    value.events.some((event, index) =>
      index > 0 && event.at_unix_ms < value.events[index - 1].at_unix_ms)
  ) {
    fail("lifecycle_receipt_invalid");
  }
  scanForPrivateStrings(value, forbidden);
  return value;
}

function historicalRecoveryExists(value) {
  return value.events.some((event, index) =>
    event.state === "retry_wait" && event.transition_reason === "child_exited" &&
    event.last_exit === "child_exited" && event.automatic_restart_attempt >= 1 &&
    value.events.slice(index + 1).some((running) =>
      running.state === "running" && running.generation === event.generation + 1 &&
      running.last_exit === "child_exited" &&
      running.automatic_restart_attempt >= event.automatic_restart_attempt &&
      running.at_unix_ms >= event.at_unix_ms));
}

export function captureLifecycleReceiptBoundary({
  capturedAtUnixMs,
  source,
  value,
  forbidden = [],
}) {
  validateLifecycleReceiptShape(value, forbidden);
  const latestRunning = value.events.findLast((event) => event.state === "running");
  if (!boundedCount(capturedAtUnixMs) || capturedAtUnixMs <= 0 || !latestRunning ||
      latestRunning.generation < 1 || capturedAtUnixMs < latestRunning.at_unix_ms) {
    fail("lifecycle_receipt_boundary_invalid");
  }
  return Object.freeze({
    capturedAtUnixMs,
    cursor: Object.freeze({
      droppedEventCount: value.dropped_event_count,
      eventCount: value.events.length,
    }),
    digest: createHash("sha256").update(source).digest("hex"),
    eventIdentities: new Set(value.events.map((event) => JSON.stringify(event))),
    generation: latestRunning.generation,
    latestEventAtUnixMs: value.events.at(-1)?.at_unix_ms ?? 0,
  });
}

export function validateLifecycleReceiptBoundary({ boundary, forbidden = [], source, value }) {
  validateLifecycleReceiptShape(value, forbidden);
  if (
    boundary === null || typeof boundary !== "object" ||
    createHash("sha256").update(source).digest("hex") === boundary.digest ||
    value.dropped_event_count < boundary.cursor.droppedEventCount ||
    (value.dropped_event_count === boundary.cursor.droppedEventCount &&
      value.events.length <= boundary.cursor.eventCount)
  ) fail("lifecycle_receipt_boundary_invalid");
  const retryIndex = value.events.findIndex((event) =>
    !boundary.eventIdentities.has(JSON.stringify(event)) &&
    event.state === "retry_wait" && event.transition_reason === "child_exited" &&
    event.last_exit === "child_exited" && event.generation === boundary.generation &&
    event.at_unix_ms >= boundary.capturedAtUnixMs &&
    event.at_unix_ms >= boundary.latestEventAtUnixMs);
  if (retryIndex < 0) fail("lifecycle_receipt_boundary_invalid");
  const retry = value.events[retryIndex];
  const running = value.events.slice(retryIndex + 1).find((event) =>
    !boundary.eventIdentities.has(JSON.stringify(event)) && event.state === "running" &&
    event.transition_reason === "control_plane_ready" &&
    event.last_exit === "child_exited" && event.generation === retry.generation + 1 &&
    event.automatic_restart_attempt >= retry.automatic_restart_attempt &&
    event.at_unix_ms >= retry.at_unix_ms);
  if (!running) fail("lifecycle_receipt_boundary_invalid");
  return value;
}

export function validateLifecycleReceipt(value, forbidden = []) {
  validateLifecycleReceiptShape(value, forbidden);
  if (!historicalRecoveryExists(value)) fail("lifecycle_receipt_invalid");
  return value;
}

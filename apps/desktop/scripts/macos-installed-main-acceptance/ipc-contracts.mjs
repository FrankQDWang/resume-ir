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

function endpointPath(value, expectedPath) {
  let url;
  try {
    url = new URL(value);
  } catch {
    fail("daemon_discovery_invalid");
  }
  if (
    url.protocol !== "http:" ||
    url.hostname !== "127.0.0.1" ||
    url.username !== "" ||
    url.password !== "" ||
    url.search !== "" ||
    url.hash !== "" ||
    url.pathname !== expectedPath ||
    !/^[1-9][0-9]{0,4}$/.test(url.port) ||
    Number(url.port) > 65_535
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
    "schema_version",
    "instance_id",
    "owner_mode",
    "status",
    "diagnostics",
    "imports",
    "import_cancel",
    "import_control",
    "import_progress",
    "search",
    "search_batch",
    "details",
    "delete",
  ];
  if (
    before.source !== after.source ||
    !exactKeys(before.value, endpointKeys) ||
    !exactKeys(auth.value, ["schema_version", "instance_id", "token"]) ||
    before.value.schema_version !== "resume-ir.daemon-ipc.v2" ||
    before.value.owner_mode !== "desktop_supervised" ||
    auth.value.schema_version !== "resume-ir.daemon-auth.v2" ||
    !DIGEST.test(before.value.instance_id ?? "") ||
    auth.value.instance_id !== before.value.instance_id ||
    !DIGEST.test(auth.value.token ?? "")
  ) {
    fail("daemon_discovery_invalid");
  }
  const routes = [
    ["status", "/status"],
    ["diagnostics", "/diagnostics"],
    ["imports", "/imports"],
    ["import_cancel", "/imports/cancel"],
    ["import_control", "/imports/control"],
    ["import_progress", "/imports/progress"],
    ["search", "/search"],
    ["search_batch", "/search/batch"],
    ["details", "/details"],
    ["delete", "/delete"],
  ];
  const urls = Object.fromEntries(
    routes.map(([key, expected]) => [
      key,
      endpointPath(before.value[key], expected),
    ]),
  );
  const origin = urls.status.origin;
  if (!Object.values(urls).every((url) => url.origin === origin)) {
    fail("daemon_discovery_invalid");
  }
  return Object.freeze({
    instanceId: before.value.instance_id,
    token: auth.value.token,
    urls,
  });
}

async function requestJsonWithBody(
  url,
  token,
  {
    body,
    expectedStatus = 200,
    method = "GET",
    signal,
    timeoutMs = HTTP_TIMEOUT_MS,
  } = {},
) {
  const payload = body === undefined ? undefined : JSON.stringify(body);
  if (
    !["GET", "POST"].includes(method) ||
    ![200, 202].includes(expectedStatus) ||
    (method === "GET" && payload !== undefined) ||
    (payload !== undefined && Buffer.byteLength(payload, "utf8") > 64 * 1024)
  ) {
    fail("daemon_request_invalid");
  }
  throwIfAborted(signal);
  return new Promise((resolve, reject) => {
    let abortListener;
    const settle = (callback, value) => {
      if (signal && abortListener) {
        signal.removeEventListener("abort", abortListener);
      }
      callback(value);
    };
    const request = http.request(
      url,
      {
        headers: {
          ...(token === undefined
            ? {}
            : { Authorization: `Bearer ${token}` }),
          ...(payload === undefined
            ? {}
            : {
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
            settle(
              resolve,
              JSON.parse(Buffer.concat(chunks, bytes).toString("utf8")),
            );
          } catch {
            settle(reject, new AcceptanceError("daemon_response_invalid"));
          }
        });
      },
    );
    request.setTimeout(timeoutMs, () => {
      request.destroy(new AcceptanceError("daemon_response_timeout"));
    });
    request.once("error", (error) =>
      settle(reject, asAcceptanceError(error)),
    );
    abortListener = () =>
      request.destroy(new AcceptanceError("acceptance_interrupted"));
    signal?.addEventListener("abort", abortListener, { once: true });
    if (signal?.aborted) {
      abortListener();
      return;
    }
    if (payload !== undefined) request.write(payload);
    request.end();
  });
}

export async function requestJson(
  url,
  token,
  timeoutMs = HTTP_TIMEOUT_MS,
  signal,
) {
  return requestJsonWithBody(url, token, { signal, timeoutMs });
}

export async function requestJsonPost(url, token, body, { signal } = {}) {
  return requestJsonWithBody(url, token, { body, method: "POST", signal });
}

export async function requestJsonPostAccepted(
  url,
  token,
  body,
  { signal } = {},
) {
  return requestJsonWithBody(url, token, {
    body,
    expectedStatus: 202,
    method: "POST",
    signal,
  });
}

const DIAGNOSTICS_REPAIR_PHASES = [
  "queued",
  "migration_rebuild",
  "source_unavailable",
  "rebuilding",
  "retry_wait",
  "blocked",
];
const DIAGNOSTICS_REPAIR_ERROR_KINDS = [
  "fulltext_publication_busy",
  "fulltext_failure",
  "vector_publication_busy",
  "vector_failure",
  "metadata_failure",
  "interrupted",
];
const DIAGNOSTICS_REPAIR_REASONS = [
  "migration_rebuild",
  "artifact_unavailable",
  "source_unavailable",
  "runtime_invariant",
];
const DIAGNOSTICS_ERROR_CODES = [
  "REPAIRING",
  "METADATA_UNAVAILABLE",
  "QUERY_SERVICE_UNAVAILABLE",
];
const DIAGNOSTICS_ERROR_ACTIONS = [
  "wait_for_repair",
  "retry",
  "repair_required",
];

function boundedCount(candidate, maximum = Number.MAX_SAFE_INTEGER) {
  return (
    Number.isSafeInteger(candidate) && candidate >= 0 && candidate <= maximum
  );
}

function nullableBoundedCount(candidate, maximum) {
  return candidate === null || boundedCount(candidate, maximum);
}

function diagnosticsNestedContractValid(value) {
  const services = value?.services;
  const progress = value?.repair_progress;
  const error = value?.error;
  return (
    exactKeys(services, ["metadata", "query"]) &&
    ["ready", "unavailable"].includes(services.metadata) &&
    ["ready", "repairing", "unavailable"].includes(services.query) &&
    (value.repair_reason === null ||
      DIAGNOSTICS_REPAIR_REASONS.includes(value.repair_reason)) &&
    (progress === null ||
      (exactKeys(progress, [
        "phase",
        "attempt",
        "max_attempts",
        "retry_after_ms",
        "last_error_kind",
      ]) &&
        DIAGNOSTICS_REPAIR_PHASES.includes(progress.phase) &&
        nullableBoundedCount(progress.attempt, 5) &&
        (progress.max_attempts === null || progress.max_attempts === 5) &&
        nullableBoundedCount(progress.retry_after_ms, 60_000) &&
        (progress.last_error_kind === null ||
          DIAGNOSTICS_REPAIR_ERROR_KINDS.includes(
            progress.last_error_kind,
          )))) &&
    (error === null ||
      (exactKeys(error, ["code", "action"]) &&
        DIAGNOSTICS_ERROR_CODES.includes(error.code) &&
        DIAGNOSTICS_ERROR_ACTIONS.includes(error.action)))
  );
}

export function readyStatus(status) {
  return (
    status?.schema_version === "daemon.status.v2" &&
    status.status === "ok" &&
    status.process_state === "ready" &&
    status.service_state === "ready" &&
    status.services?.metadata === "ready" &&
    status.services?.query === "ready" &&
    status.repair_reason === null &&
    status.repair_progress === null &&
    status.error === null &&
    status.index_health === "ready" &&
    status.snapshot_present === true
  );
}

function exactContentionProgress(progress, kind, phase, attempt) {
  const errorKind = CONTENTION_ERROR_KINDS[kind];
  if (!errorKind) return false;
  return (
    exactKeys(progress, [
      "phase",
      "attempt",
      "max_attempts",
      "retry_after_ms",
      "last_error_kind",
    ]) &&
    progress.phase === phase &&
    progress.attempt === attempt &&
    progress.max_attempts === 5 &&
    progress.last_error_kind === errorKind
  );
}

export function contentionStatus(status, kind, expectedAttempt) {
  const progress = status?.repair_progress;
  return (
    status?.schema_version === "daemon.status.v2" &&
    status.status === "repairing" &&
    status.process_state === "ready" &&
    status.service_state === "repairing" &&
    status.services?.metadata === "ready" &&
    status.services?.query === "repairing" &&
    status.repair_reason === "artifact_unavailable" &&
    Number.isSafeInteger(expectedAttempt) &&
    expectedAttempt >= 1 &&
    expectedAttempt < 5 &&
    exactContentionProgress(progress, kind, "retry_wait", expectedAttempt) &&
    Number.isSafeInteger(progress.retry_after_ms) &&
    progress.retry_after_ms >= 0 &&
    progress.retry_after_ms <= 60_000 &&
    status.error?.code === "REPAIRING" &&
    status.error?.action === "wait_for_repair"
  );
}

export function persistentBlockedStatus(status, kind) {
  const progress = status?.repair_progress;
  return (
    status?.schema_version === "daemon.status.v2" &&
    status.status === "degraded" &&
    status.process_state === "ready" &&
    status.service_state === "degraded" &&
    status.services?.metadata === "ready" &&
    status.services?.query === "unavailable" &&
    status.repair_reason === "runtime_invariant" &&
    exactContentionProgress(progress, kind, "blocked", 5) &&
    progress.retry_after_ms === null &&
    status.error?.code === "QUERY_SERVICE_UNAVAILABLE" &&
    status.error?.action === "repair_required"
  );
}

function scanForPrivateStrings(value, forbidden) {
  const visit = (candidate) => {
    if (typeof candidate === "string") {
      if (
        candidate.startsWith("/") ||
        /^[A-Za-z]:[\\/]/.test(candidate) ||
        candidate.startsWith("file:") ||
        forbidden.some((secret) => secret && candidate.includes(secret))
      ) {
        fail("diagnostics_privacy_invalid");
      }
      return;
    }
    if (Array.isArray(candidate)) {
      candidate.forEach(visit);
      return;
    }
    if (candidate !== null && typeof candidate === "object") {
      Object.values(candidate).forEach(visit);
    }
  };
  visit(value);
}

function diagnosticsServiceStateValid(value, expected) {
  if (expected.phase === "ready") {
    return (
      value.process_state === "ready" &&
      value.service_state === "ready" &&
      value.services?.metadata === "ready" &&
      value.services?.query === "ready" &&
      value.repair_reason === null &&
      value.repair_progress === null &&
      value.error === null
    );
  }
  if (expected.phase === "retry_wait") {
    const progress = value.repair_progress;
    return (
      value.process_state === "ready" &&
      value.service_state === "repairing" &&
      value.services?.metadata === "ready" &&
      value.services?.query === "repairing" &&
      value.repair_reason === "artifact_unavailable" &&
      exactContentionProgress(
        progress,
        expected.kind,
        "retry_wait",
        expected.attempt,
      ) &&
      Number.isSafeInteger(progress.retry_after_ms) &&
      progress.retry_after_ms >= 0 &&
      progress.retry_after_ms <= 60_000 &&
      value.error?.code === "REPAIRING" &&
      value.error?.action === "wait_for_repair"
    );
  }
  if (expected.phase === "blocked") {
    return (
      value.process_state === "ready" &&
      value.service_state === "degraded" &&
      value.services?.metadata === "ready" &&
      value.services?.query === "unavailable" &&
      value.repair_reason === "runtime_invariant" &&
      exactContentionProgress(
        value.repair_progress,
        expected.kind,
        "blocked",
        5,
      ) &&
      value.repair_progress.retry_after_ms === null &&
      value.error?.code === "QUERY_SERVICE_UNAVAILABLE" &&
      value.error?.action === "repair_required"
    );
  }
  return false;
}

export function validateDaemonDiagnostics(
  value,
  forbidden = [],
  expected = { phase: "ready" },
) {
  const serialized = JSON.stringify(value);
  const count = (candidate) => boundedCount(candidate);
  const optionalCount = (candidate) => candidate === null || count(candidate);
  const optionalLatency = (candidate) =>
    candidate === null ||
    (typeof candidate === "number" &&
      Number.isFinite(candidate) &&
      candidate >= 0 &&
      candidate <= 3_600_000);
  const metricsKeys = [
    "ipc",
    "indexed_documents",
    "searchable_documents",
    "partial_documents",
    "ocr_queue_depth",
    "embedding_queue_depth",
    "recovery_queue_depth",
    "import_tasks_queued",
    "import_tasks_recoverable",
    "import_tasks_cancelled",
    "query_latency",
  ];
  const errorCountKeys = [
    "failed_retryable",
    "failed_permanent",
    "import_scan_errors",
    "ocr_page_budget_blocked",
    "ocr_language_unavailable",
    "scan_error_buckets",
  ];
  const ipc = value?.metrics?.ipc;
  const latency = value?.metrics?.query_latency;
  const buckets = value?.error_counts?.scan_error_buckets;
  if (
    typeof serialized !== "string" ||
    Buffer.byteLength(serialized, "utf8") > MAX_DIAGNOSTICS_BYTES ||
    !exactKeys(value, [
      "schema_version",
      "privacy_boundary",
      "contains_raw_resume_text",
      "contains_queries",
      "contains_resume_paths",
      "contains_candidate_results",
      "contains_snippet_text",
      "visible_epoch",
      "evidence_lane",
      "evidence_status",
      "process_state",
      "service_state",
      "services",
      "repair_reason",
      "repair_progress",
      "error",
      "metrics",
      "error_counts",
      "benchmark_refs",
    ]) ||
    value?.schema_version !== "resume-ir.diagnostics.v3" ||
    value.privacy_boundary !== "redacted_local_aggregate" ||
    value.contains_raw_resume_text !== false ||
    value.contains_queries !== false ||
    value.contains_resume_paths !== false ||
    value.contains_candidate_results !== false ||
    value.contains_snippet_text !== false ||
    !count(value.visible_epoch) ||
    value.evidence_lane !== "gui_manual" ||
    value.evidence_status !== "unaccepted" ||
    !diagnosticsNestedContractValid(value) ||
    !diagnosticsServiceStateValid(value, expected) ||
    !exactKeys(value.metrics, metricsKeys) ||
    !exactKeys(ipc, [
      "accepted",
      "completed",
      "client_disconnect",
      "request_failure",
      "response_failure",
    ]) ||
    !Object.values(ipc).every(count) ||
    !metricsKeys
      .filter((key) => !["ipc", "query_latency"].includes(key))
      .every((key) => count(value.metrics[key])) ||
    !exactKeys(latency, [
      "sample_count",
      "p50_ms",
      "p95_ms",
      "p99_ms",
      "last_result_count",
    ]) ||
    !count(latency.sample_count) ||
    ![latency.p50_ms, latency.p95_ms, latency.p99_ms].every(optionalLatency) ||
    !optionalCount(latency.last_result_count) ||
    !exactKeys(value.error_counts, errorCountKeys) ||
    !errorCountKeys
      .filter((key) => key !== "scan_error_buckets")
      .every((key) => count(value.error_counts[key])) ||
    !Array.isArray(buckets) ||
    buckets.length > 16 ||
    !buckets.every(
      (bucket) =>
        exactKeys(bucket, ["class", "operation", "count"]) &&
        [
          "permission_denied",
          "source_unavailable",
          "locked_or_unreadable",
          "io",
        ].includes(bucket.class) &&
        [
          "normalize_path",
          "read_directory",
          "read_metadata",
          "fingerprint",
        ].includes(bucket.operation) &&
        count(bucket.count),
    ) ||
    !Array.isArray(value.benchmark_refs) ||
    value.benchmark_refs.length !== 0
  ) {
    fail("diagnostics_contract_invalid");
  }
  scanForPrivateStrings(value, forbidden);
  return value;
}

function validateLifecycleReceiptShape(value, forbidden) {
  const count = (candidate) =>
    Number.isSafeInteger(candidate) && candidate >= 0;
  const nullableEnum = (candidate, values) =>
    candidate === null || values.includes(candidate);
  if (
    !exactKeys(value, [
      "schema_version",
      "persistence_state",
      "dropped_event_count",
      "events",
    ]) ||
    value?.schema_version !== "resume-ir.desktop-daemon-lifecycle-receipt.v1" ||
    value.persistence_state !== "ready" ||
    !Number.isSafeInteger(value.dropped_event_count) ||
    value.dropped_event_count < 0 ||
    !Array.isArray(value.events) ||
    value.events.length > 16 ||
    !value.events.every(
      (event) =>
        exactKeys(event, [
          "at_unix_ms",
          "state",
          "generation",
          "restart_attempt",
          "restart_budget",
          "retry_delay_ms",
          "consecutive_heartbeat_failures",
          "blocked_reason",
          "last_exit",
          "restart_ledger_reason",
        ]) &&
        count(event.at_unix_ms) &&
        event.at_unix_ms > 0 &&
        ["starting", "ready", "recovering", "circuit_open", "blocked"].includes(
          event.state,
        ) &&
        count(event.generation) &&
        count(event.restart_attempt) &&
        event.restart_attempt <= 5 &&
        event.restart_budget === 5 &&
        (event.retry_delay_ms === null ||
          (count(event.retry_delay_ms) && event.retry_delay_ms <= 300_000)) &&
        count(event.consecutive_heartbeat_failures) &&
        event.consecutive_heartbeat_failures <= 3 &&
        nullableEnum(event.blocked_reason, [
          "configuration_invalid",
          "runtime_integrity",
          "protocol_mismatch",
          "ownership_conflict",
          "supervisor_unavailable",
          "restart_ledger_invalid",
        ]) &&
        nullableEnum(event.last_exit, [
          "child_exited",
          "startup_timeout",
          "heartbeat_timeout",
          "start_failed",
          "control_plane_failure",
        ]) &&
        nullableEnum(event.restart_ledger_reason, [
          "invalid_format",
          "unsafe_file",
          "oversized",
          "read_unavailable",
          "clock_invalid",
          "persistence_unavailable",
        ]),
    )
  ) {
    fail("lifecycle_receipt_invalid");
  }
  if (
    value.events.some(
      (event, index) =>
        index > 0 && event.at_unix_ms < value.events[index - 1].at_unix_ms,
    )
  ) {
    fail("lifecycle_receipt_invalid");
  }
  scanForPrivateStrings(value, forbidden);
  return value;
}

function historicalRecoveryExists(value) {
  return value.events.some(
    (event, recoveryIndex) =>
      event.state === "recovering" &&
      event.last_exit === "child_exited" &&
      event.restart_attempt >= 1 &&
      value.events
        .slice(recoveryIndex + 1)
        .some(
          (ready) =>
            ready.state === "ready" &&
            ready.last_exit === "child_exited" &&
            ready.generation === event.generation + 1 &&
            ready.restart_attempt >= event.restart_attempt &&
            ready.at_unix_ms >= event.at_unix_ms,
        ),
  );
}

export function captureLifecycleReceiptBoundary({
  capturedAtUnixMs,
  source,
  value,
  forbidden = [],
}) {
  validateLifecycleReceiptShape(value, forbidden);
  const latestReady = value.events.findLast((event) => event.state === "ready");
  if (
    !Number.isSafeInteger(capturedAtUnixMs) ||
    capturedAtUnixMs <= 0 ||
    !latestReady ||
    latestReady.generation < 1 ||
    capturedAtUnixMs < latestReady.at_unix_ms
  ) {
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
    generation: latestReady.generation,
    latestEventAtUnixMs: value.events.at(-1)?.at_unix_ms ?? 0,
  });
}

export function validateLifecycleReceiptBoundary({
  boundary,
  forbidden = [],
  source,
  value,
}) {
  validateLifecycleReceiptShape(value, forbidden);
  if (
    boundary === null ||
    typeof boundary !== "object" ||
    createHash("sha256").update(source).digest("hex") === boundary.digest ||
    value.dropped_event_count < boundary.cursor.droppedEventCount ||
    (value.dropped_event_count === boundary.cursor.droppedEventCount &&
      value.events.length <= boundary.cursor.eventCount)
  ) {
    fail("lifecycle_receipt_boundary_invalid");
  }
  const recoveryIndex = value.events.findIndex(
    (event) =>
      !boundary.eventIdentities.has(JSON.stringify(event)) &&
      event.state === "recovering" &&
      event.last_exit === "child_exited" &&
      event.generation === boundary.generation &&
      event.at_unix_ms >= boundary.capturedAtUnixMs &&
      event.at_unix_ms >= boundary.latestEventAtUnixMs,
  );
  if (recoveryIndex < 0) fail("lifecycle_receipt_boundary_invalid");
  const recovery = value.events[recoveryIndex];
  const ready = value.events.slice(recoveryIndex + 1).find(
    (event) =>
      !boundary.eventIdentities.has(JSON.stringify(event)) &&
      event.state === "ready" &&
      event.last_exit === "child_exited" &&
      event.generation === recovery.generation + 1 &&
      event.restart_attempt >= recovery.restart_attempt &&
      event.at_unix_ms >= recovery.at_unix_ms,
  );
  if (!ready) fail("lifecycle_receipt_boundary_invalid");
  return value;
}

export function validateLifecycleReceipt(value, forbidden) {
  validateLifecycleReceiptShape(value, forbidden);
  if (!historicalRecoveryExists(value)) fail("lifecycle_receipt_invalid");
  return value;
}

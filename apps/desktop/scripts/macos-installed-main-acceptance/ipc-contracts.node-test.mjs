import assert from "node:assert/strict";
import http from "node:http";
import {
  chmod,
  mkdtemp,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  readVerifiedPrivateText,
  readActiveStoreManifest,
} from "./filesystem-cow.mjs";
import {
  captureLifecycleReceiptBoundary,
  contentionStatus,
  persistentBlockedStatus,
  readDaemonConnection,
  requestJsonPost,
  validateDaemonDiagnostics,
  validateLifecycleReceipt,
  validateLifecycleReceiptBoundary,
} from "./ipc-contracts.mjs";
import { diagnostics } from "./fixtures.mjs";

test("sends the bounded search witness as an authenticated JSON POST", async (context) => {
  const observed = [];
  const server = http.createServer((request, response) => {
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => {
      observed.push({
        authorization: request.headers.authorization,
        body: Buffer.concat(chunks).toString("utf8"),
        method: request.method,
        url: request.url,
      });
      response.writeHead(200, { "Content-Type": "application/json" });
      response.end('{"status":"ok"}');
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  context.after(() => new Promise((resolve) => server.close(resolve)));
  const address = server.address();
  const body = { schema_version: "synthetic.search.v1", top_k: 1 };
  assert.deepEqual(
    await requestJsonPost(
      new URL(`http://127.0.0.1:${address.port}/search`),
      "a".repeat(64),
      body,
    ),
    { status: "ok" },
  );
  assert.deepEqual(observed, [
    {
      authorization: `Bearer ${"a".repeat(64)}`,
      body: JSON.stringify(body),
      method: "POST",
      url: "/search",
    },
  ]);
});

test("validates diagnostics privacy flags and rejects path-shaped payloads", () => {
  const value = diagnostics();
  assert.equal(validateDaemonDiagnostics(value), value);
  assert.throws(
    () =>
      validateDaemonDiagnostics(
        diagnostics({ private_debug: "/Users/example/private-resume.pdf" }),
      ),
    /diagnostics_contract_invalid/,
  );
  assert.throws(
    () =>
      validateDaemonDiagnostics(diagnostics({ contains_resume_paths: true })),
    /diagnostics_contract_invalid/,
  );
  assert.throws(
    () =>
      validateDaemonDiagnostics(diagnostics({ benchmark_refs: ["synthetic"] })),
    /diagnostics_contract_invalid/,
  );
});

test("rejects unknown diagnostics nested keys without relying on path scanning", () => {
  const progress = {
    phase: "retry_wait",
    attempt: 1,
    max_attempts: 5,
    retry_after_ms: 1_000,
    last_error_kind: "fulltext_publication_busy",
  };
  const retry = diagnostics({
    service_state: "repairing",
    services: { metadata: "ready", query: "repairing" },
    repair_reason: "artifact_unavailable",
    repair_progress: progress,
    error: { code: "REPAIRING", action: "wait_for_repair" },
  });
  const retryExpected = {
    attempt: 1,
    kind: "fulltext",
    phase: "retry_wait",
  };

  assert.throws(
    () =>
      validateDaemonDiagnostics(
        diagnostics({
          services: { metadata: "ready", query: "ready", worker: "ready" },
        }),
      ),
    /diagnostics_contract_invalid/,
  );
  assert.throws(
    () =>
      validateDaemonDiagnostics(
        {
          ...retry,
          repair_progress: { ...progress, subsystem: "fulltext" },
        },
        [],
        retryExpected,
      ),
    /diagnostics_contract_invalid/,
  );
  assert.throws(
    () =>
      validateDaemonDiagnostics(
        {
          ...retry,
          error: {
            code: "REPAIRING",
            action: "wait_for_repair",
            detail: "synthetic-detail",
          },
        },
        [],
        retryExpected,
      ),
    /diagnostics_contract_invalid/,
  );
});

test("rejects invalid diagnostics enums and out-of-contract numeric bounds", () => {
  const retry = (overrides = {}) =>
    diagnostics({
      service_state: "repairing",
      services: { metadata: "ready", query: "repairing" },
      repair_reason: "artifact_unavailable",
      repair_progress: {
        phase: "retry_wait",
        attempt: 1,
        max_attempts: 5,
        retry_after_ms: 1_000,
        last_error_kind: "fulltext_publication_busy",
        ...overrides,
      },
      error: { code: "REPAIRING", action: "wait_for_repair" },
    });

  assert.throws(
    () =>
      validateDaemonDiagnostics(
        retry({ last_error_kind: "fulltext" }),
        [],
        { attempt: 1, kind: "fulltext", phase: "retry_wait" },
      ),
    /diagnostics_contract_invalid/,
  );
  assert.throws(
    () =>
      validateDaemonDiagnostics(retry({ attempt: 6 }), [], {
        attempt: 6,
        kind: "fulltext",
        phase: "retry_wait",
      }),
    /diagnostics_contract_invalid/,
  );
  const value = diagnostics();
  assert.throws(
    () =>
      validateDaemonDiagnostics({
        ...value,
        metrics: {
          ...value.metrics,
          query_latency: {
            ...value.metrics.query_latency,
            p95_ms: 3_600_001,
          },
        },
      }),
    /diagnostics_contract_invalid/,
  );
});

test("requires persisted child-exit recovery followed by the next ready generation", () => {
  const event = (overrides) => ({
    at_unix_ms: 1_000,
    state: "recovering",
    generation: 4,
    restart_attempt: 1,
    restart_budget: 5,
    retry_delay_ms: 0,
    consecutive_heartbeat_failures: 0,
    blocked_reason: null,
    last_exit: "child_exited",
    restart_ledger_reason: null,
    ...overrides,
  });
  const receipt = {
    schema_version: "resume-ir.desktop-daemon-lifecycle-receipt.v1",
    persistence_state: "ready",
    dropped_event_count: 0,
    events: [
      event({}),
      event({ at_unix_ms: 1_001, state: "ready", generation: 5 }),
    ],
  };
  assert.equal(validateLifecycleReceipt(receipt, []), receipt);
  assert.throws(
    () =>
      validateLifecycleReceipt(
        { ...receipt, persistence_state: "recovered_corrupt", events: [] },
        [],
      ),
    /lifecycle_receipt_invalid/,
  );
  assert.throws(
    () =>
      validateLifecycleReceipt(
        {
          ...receipt,
          events: [event({}), event({ at_unix_ms: 999, state: "ready" })],
        },
        [],
      ),
    /lifecycle_receipt_invalid/,
  );
});

test("binds lifecycle recovery to the receipt captured before this kill", () => {
  const event = (overrides) => ({
    at_unix_ms: 1_000,
    state: "ready",
    generation: 4,
    restart_attempt: 0,
    restart_budget: 5,
    retry_delay_ms: null,
    consecutive_heartbeat_failures: 0,
    blocked_reason: null,
    last_exit: null,
    restart_ledger_reason: null,
    ...overrides,
  });
  const before = {
    schema_version: "resume-ir.desktop-daemon-lifecycle-receipt.v1",
    persistence_state: "ready",
    dropped_event_count: 0,
    events: [event({})],
  };
  const beforeSource = `${JSON.stringify(before)}\n`;
  const boundary = captureLifecycleReceiptBoundary({
    capturedAtUnixMs: 2_000,
    source: beforeSource,
    value: before,
  });
  const after = {
    ...before,
    events: [
      ...before.events,
      event({
        at_unix_ms: 2_001,
        state: "recovering",
        restart_attempt: 1,
        retry_delay_ms: 1_000,
        last_exit: "child_exited",
      }),
      event({
        at_unix_ms: 3_001,
        generation: 5,
        restart_attempt: 1,
        last_exit: "child_exited",
      }),
    ],
  };
  assert.equal(
    validateLifecycleReceiptBoundary({
      boundary,
      source: `${JSON.stringify(after)}\n`,
      value: after,
    }),
    after,
  );
  assert.throws(
    () =>
      validateLifecycleReceiptBoundary({
        boundary,
        source: beforeSource,
        value: before,
      }),
    /lifecycle_receipt_boundary_invalid/,
  );
  const historical = {
    ...after,
    events: after.events.map((item, index) =>
      index === 1 ? { ...item, at_unix_ms: 1_500 } : item,
    ),
  };
  assert.throws(
    () =>
      validateLifecycleReceiptBoundary({
        boundary,
        source: `${JSON.stringify(historical)}\n`,
        value: historical,
      }),
    /lifecycle_receipt_boundary_invalid/,
  );
});

test("requires exact publication-busy error kinds for retry and blocked states", () => {
  const progress = (overrides = {}) => ({
    phase: "retry_wait",
    attempt: 1,
    max_attempts: 5,
    retry_after_ms: 1_000,
    last_error_kind: "fulltext_publication_busy",
    ...overrides,
  });
  const retry = {
    schema_version: "daemon.status.v2",
    status: "repairing",
    process_state: "ready",
    service_state: "repairing",
    services: { metadata: "ready", query: "repairing" },
    repair_reason: "artifact_unavailable",
    repair_progress: progress(),
    error: { code: "REPAIRING", action: "wait_for_repair" },
  };
  assert.equal(contentionStatus(retry, "fulltext", 1), true);
  assert.equal(contentionStatus(retry, "vector", 1), false);
  assert.equal(
    contentionStatus(
      {
        ...retry,
        repair_progress: {
          ...progress(),
          last_error_kind: "fulltext_storage",
        },
      },
      "fulltext",
      1,
    ),
    false,
  );
  const { last_error_kind: _removed, ...broadProgress } = progress();
  assert.equal(
    contentionStatus(
      {
        ...retry,
        repair_progress: { ...broadProgress, last_error_class: "fulltext" },
      },
      "fulltext",
      1,
    ),
    false,
  );

  const blocked = {
    ...retry,
    status: "degraded",
    service_state: "degraded",
    services: { metadata: "ready", query: "unavailable" },
    repair_reason: "runtime_invariant",
    repair_progress: progress({
      phase: "blocked",
      attempt: 5,
      retry_after_ms: null,
    }),
    error: {
      code: "QUERY_SERVICE_UNAVAILABLE",
      action: "repair_required",
    },
  };
  assert.equal(persistentBlockedStatus(blocked, "fulltext"), true);
  assert.equal(
    persistentBlockedStatus(
      { ...blocked, repair_reason: "artifact_unavailable" },
      "fulltext",
    ),
    false,
  );

  const retryDiagnostics = diagnostics({
    process_state: "ready",
    service_state: "repairing",
    services: { metadata: "ready", query: "repairing" },
    repair_reason: "artifact_unavailable",
    repair_progress: progress(),
    error: { code: "REPAIRING", action: "wait_for_repair" },
  });
  assert.equal(
    validateDaemonDiagnostics(retryDiagnostics, [], {
      attempt: 1,
      kind: "fulltext",
      phase: "retry_wait",
    }),
    retryDiagnostics,
  );
  assert.throws(
    () =>
      validateDaemonDiagnostics(
        diagnostics({
          ...retryDiagnostics,
          repair_progress: progress({
            last_error_kind: "vector_publication_busy",
          }),
        }),
        [],
        { attempt: 1, kind: "fulltext", phase: "retry_wait" },
      ),
    /diagnostics_contract_invalid/,
  );
  const blockedDiagnostics = diagnostics({
    process_state: "ready",
    service_state: "degraded",
    services: { metadata: "ready", query: "unavailable" },
    repair_reason: "runtime_invariant",
    repair_progress: progress({
      phase: "blocked",
      attempt: 5,
      retry_after_ms: null,
    }),
    error: {
      code: "QUERY_SERVICE_UNAVAILABLE",
      action: "repair_required",
    },
  });
  assert.equal(
    validateDaemonDiagnostics(blockedDiagnostics, [], {
      kind: "fulltext",
      phase: "blocked",
    }),
    blockedDiagnostics,
  );
  assert.throws(
    () =>
      validateDaemonDiagnostics(
        {
          ...blockedDiagnostics,
          repair_progress: {
            ...blockedDiagnostics.repair_progress,
            last_error_kind: "fulltext_failure",
          },
        },
        [],
        { kind: "fulltext", phase: "blocked" },
      ),
    /diagnostics_contract_invalid/,
  );
});

test("parses strict private v28 manifests and rejects permissive files", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-manifest-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const manifest = path.join(root, "metadata-active.v1");
  const records = [
    "resume-ir.metadata-active.v1",
    "file=metadata-v28-1111111111111111.sqlite3",
    "schema=28",
    `digest=${"1".repeat(64)}`,
  ];
  const canonical = `${records.join("\n")}\n`;
  await writeFile(manifest, canonical, { mode: 0o600 });
  assert.deepEqual(await readActiveStoreManifest(root), {
    fileName: "metadata-v28-1111111111111111.sqlite3",
    schema: 28,
    digest: "1".repeat(64),
  });
  for (const invalid of [
    canonical.slice(0, -1),
    `${canonical}\n`,
    `${canonical}extra=forbidden\n`,
    canonical.replaceAll("\n", "\r\n"),
    canonical.replace("schema=28", "schema=29"),
  ]) {
    await writeFile(manifest, invalid);
    await assert.rejects(
      readActiveStoreManifest(root),
      /active_store_manifest_invalid/,
    );
  }
  await writeFile(manifest, canonical);
  await chmod(manifest, 0o644);
  await assert.rejects(readActiveStoreManifest(root), /private_file_invalid/);
});

test("reads only a generation-consistent owner-only loopback discovery pair", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-connection-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const instanceId = "1".repeat(64);
  const token = "2".repeat(64);
  const origin = "http://127.0.0.1:43111";
  const endpoint = {
    schema_version: "resume-ir.daemon-ipc.v2",
    instance_id: instanceId,
    owner_mode: "desktop_supervised",
    status: `${origin}/status`,
    diagnostics: `${origin}/diagnostics`,
    imports: `${origin}/imports`,
    import_cancel: `${origin}/imports/cancel`,
    import_control: `${origin}/imports/control`,
    import_progress: `${origin}/imports/progress`,
    search: `${origin}/search`,
    search_batch: `${origin}/search/batch`,
    details: `${origin}/details`,
    delete: `${origin}/delete`,
  };
  await writeFile(
    path.join(root, "ipc.endpoints.json"),
    JSON.stringify(endpoint),
    {
      mode: 0o600,
    },
  );
  await writeFile(
    path.join(root, "ipc.auth"),
    JSON.stringify({
      schema_version: "resume-ir.daemon-auth.v2",
      instance_id: instanceId,
      token,
    }),
    { mode: 0o600 },
  );

  const connection = await readDaemonConnection(root);
  assert.equal(connection.instanceId, instanceId);
  assert.equal(connection.urls.status.pathname, "/status");
  await chmod(path.join(root, "ipc.auth"), 0o644);
  await assert.rejects(readDaemonConnection(root), /private_file_invalid/);
});

test("rejects an owner-file pathname replacement after the verified open", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-owner-race-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const file = path.join(root, "owner.json");
  const replacement = path.join(root, "replacement.json");
  const displaced = path.join(root, "displaced.json");
  await writeFile(file, '{"generation":"first"}', { mode: 0o600 });
  await writeFile(replacement, '{"generation":"second"}', { mode: 0o600 });

  await assert.rejects(
    readVerifiedPrivateText(file, 1_024, {
      afterVerifiedOpen: async () => {
        await rename(file, displaced);
        await rename(replacement, file);
      },
    }),
    /private_file_invalid/,
  );
});

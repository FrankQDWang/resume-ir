import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  chmod,
  mkdir,
  mkdtemp,
  realpath,
  rm,
  unlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  captureV29LogicalAuthority,
  validateInstalledReadyArtifacts,
  validateInstalledRecoveryEvidence,
} from "./acceptance-evidence.mjs";
import { diagnostics } from "./fixtures.mjs";

const STORE_DIGEST = "1".repeat(64);
const PROJECTION_DIGEST = `sha256:${"2".repeat(64)}`;
const GENERATION = "generation-ready";
const SNAPSHOT_PAYLOAD = "encrypted";
const SNAPSHOT_ARTIFACT_DIGEST = `sha256:${createHash("sha256")
  .update(SNAPSHOT_PAYLOAD)
  .digest("hex")}`;

function readyStatus(overrides = {}) {
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
    capabilities: {
      keyword_search: { state: "available", reason: null },
      detail: { state: "available", reason: null },
      semantic_search: { state: "available", reason: null },
      hybrid_search: { state: "available", reason: null },
      text_import: { state: "available", reason: null },
      ocr_import: { state: "available", reason: null },
      index_publication: { state: "available", reason: null },
    },
    repair_progress: null,
    error: null,
    ipc: {
      accepted: 4,
      completed: 4,
      client_disconnect: 0,
      request_failure: 0,
      response_failure: 0,
    },
    visible_epoch: 7,
    indexed_documents: 4,
    searchable_documents: 4,
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
    entity_mentions: 4,
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
    ...overrides,
  };
}

function searchResponse(overrides = {}) {
  return {
    schema_version: "resume-ir.search-response.v3",
    request_id: "installed-main-synthetic-witness",
    status: "ok",
    visible_epoch: 7,
    query_mode: "fulltext",
    partial: false,
    partial_reasons: [],
    latency_ms: 1,
    stage_latency_ms: {
      parse: 0.1,
      prefilter: 0.1,
      bm25: 0.2,
      ann: 0,
      fusion: 0,
      bulk_hydrate: 0.1,
      snippet: 0.1,
    },
    search_index: "available",
    result_count: 0,
    results: [],
    ...overrides,
  };
}

function metadataAuthority(overrides = {}) {
  const generation = overrides.generation ?? GENERATION;
  const values = {
    generation,
    visibleEpoch: 7,
    projectionDigest: PROJECTION_DIGEST,
    vectorProjectionDigest: PROJECTION_DIGEST,
    ...overrides,
  };
  const fields = [
    values.generation,
    values.visibleEpoch,
    "ready",
    "ready",
    values.projectionDigest,
    values.generation,
    "fulltext.snapshot.v3",
    "tantivy.fulltext.v3",
    4,
    values.projectionDigest,
    `sha256:${"3".repeat(64)}`,
    values.generation,
    "vector.snapshot.v4",
    "hnsw-vector.v4",
    "disabled",
    "",
    -1,
    4,
    `sha256:${"5".repeat(64)}`,
    0,
    0,
    values.vectorProjectionDigest,
    `sha256:${"3".repeat(64)}`,
  ];
  return async (command, args, options) => {
    assert.equal(command, "/usr/bin/sqlite3");
    assert.equal(args.at(-2).endsWith(".sqlite3"), true);
    assert.match(args.at(-1), /search_projection_state/);
    assert.equal(options.env.HOME, "/var/empty");
    return {
      status: 0,
      stdout: `${fields.join("\t")}\n`,
      stderr: "",
      timedOut: false,
      overflow: false,
    };
  };
}

async function writePrivate(file, value) {
  await writeFile(file, value, { mode: 0o600 });
  await chmod(file, 0o600);
}

async function writeGeneration(root, kind, generation = GENERATION, overrides = {}) {
  const directory = path.join(root, "snapshots", generation);
  await mkdir(directory, { recursive: true, mode: 0o700 });
  await chmod(root, 0o700);
  await chmod(path.join(root, "snapshots"), 0o700);
  await chmod(directory, 0o700);
  const common = {
    generation,
    projection_digest: PROJECTION_DIGEST,
    logical_content_digest: `sha256:${"3".repeat(64)}`,
    artifact_digest: SNAPSHOT_ARTIFACT_DIGEST,
  };
  if (kind === "fulltext") {
    await writePrivate(
      path.join(directory, "snapshot-manifest.json"),
      `${JSON.stringify({
        schema_version: "fulltext.snapshot.v3",
        index_schema: "tantivy.fulltext.v3",
        encrypted_snapshot: "resume-ir-fulltext-snapshot-encrypted-v3",
        document_count: 4,
        ...common,
        ...overrides,
      })}\n`,
    );
    await writePrivate(
      path.join(directory, "fulltext.snapshot.enc"),
      SNAPSHOT_PAYLOAD,
    );
    await writePrivate(path.join(directory, "fulltext.snapshot.key-v3"), "a".repeat(64));
    return;
  }
  await writePrivate(
    path.join(directory, "snapshot-manifest.json"),
    JSON.stringify({
      schema_version: "vector.snapshot.v4",
      index_schema: "hnsw-vector.v4",
      model_id: null,
      dimension: null,
      vector_count: 0,
      projection_count: 4,
      vector_document_count: 0,
      coverage_digest: `sha256:${"5".repeat(64)}`,
      search_backend: "hnsw_ann",
      encryption: "xchacha20poly1305.v1",
      ...common,
      ...overrides,
    }),
  );
  await writePrivate(
    path.join(directory, "vector.snapshot.enc"),
    SNAPSHOT_PAYLOAD,
  );
  await writePrivate(path.join(directory, "vector.snapshot.key-v4"), "a".repeat(64));
}

async function fixture(context) {
  const dataDir = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-ready-evidence-")),
  );
  context.after(() => rm(dataDir, { recursive: true, force: true }));
  await chmod(dataDir, 0o700);
  const metadataFile = `metadata-v29-${STORE_DIGEST.slice(0, 16)}.sqlite3`;
  await writePrivate(
    path.join(dataDir, "metadata-active.v1"),
    [
      "resume-ir.metadata-active.v1",
      `file=${metadataFile}`,
      "schema=29",
      `digest=${STORE_DIGEST}`,
      "",
    ].join("\n"),
  );
  await writePrivate(path.join(dataDir, metadataFile), "SQLite format 3\0synthetic");
  await writeGeneration(path.join(dataDir, "search-index"), "fulltext");
  await writeGeneration(path.join(dataDir, "vector-index"), "vector");
  return { dataDir, metadataFile };
}

test("binds the active v29 metadata file, one exact generation pair, and a bounded search witness", async (context) => {
  const { dataDir } = await fixture(context);
  assert.deepEqual(
    await validateInstalledRecoveryEvidence({
      dataDir,
      diagnostics: diagnostics(),
      runTool: metadataAuthority(),
      search: searchResponse(),
      status: readyStatus({
        latest_import_scan: {
          scan_profile: "explicit",
          files_discovered: 4,
          ignored_entries: 1,
          scan_errors: 0,
          searchable_documents: 4,
          ocr_required_documents: 0,
          ocr_jobs_queued: 0,
          failed_documents: 0,
          deleted_documents: 0,
          scan_budget_observed: null,
          scan_budget_limit: null,
          scan_budget_exhausted: false,
        },
      }),
    }),
    {
      generationAgreement: true,
      metadataArtifactBound: true,
      searchWitness: true,
    },
  );
});

test("validates cold Ready and artifact evidence without a search witness", async (context) => {
  const { dataDir } = await fixture(context);
  const expectedV29Authority = await captureV29LogicalAuthority({
    dataDir,
    runTool: metadataAuthority(),
  });
  assert.deepEqual(
    await validateInstalledReadyArtifacts({
      dataDir,
      diagnostics: diagnostics(),
      expectedV29Authority,
      runTool: metadataAuthority(),
      status: readyStatus(),
    }),
    {
      generationAgreement: true,
      metadataArtifactBound: true,
    },
  );
  await assert.rejects(
    validateInstalledReadyArtifacts({
      dataDir,
      diagnostics: diagnostics(),
      expectedV29Authority: { ...expectedV29Authority, visibleEpoch: 6 },
      runTool: metadataAuthority(),
      status: readyStatus(),
    }),
    /v29_logical_authority_changed/,
  );
});

test("rejects missing or stale metadata and generation artifacts", async (context) => {
  const missing = await fixture(context);
  await unlink(path.join(missing.dataDir, missing.metadataFile));
  await assert.rejects(
    validateInstalledRecoveryEvidence({
      dataDir: missing.dataDir,
      diagnostics: diagnostics(),
      runTool: metadataAuthority(),
      search: searchResponse(),
      status: readyStatus(),
    }),
    /metadata_artifact_invalid/,
  );

  const stale = await fixture(context);
  await writeGeneration(
    path.join(stale.dataDir, "search-index"),
    "fulltext",
    "generation-stale",
  );
  await assert.rejects(
    validateInstalledRecoveryEvidence({
      dataDir: stale.dataDir,
      diagnostics: diagnostics(),
      runTool: metadataAuthority({ generation: "generation-stale" }),
      search: searchResponse(),
      status: readyStatus(),
    }),
    /search_artifact_generation_invalid/,
  );
});

test("rejects projection/count/epoch inconsistencies without exposing result content", async (context) => {
  const projection = await fixture(context);
  await writeGeneration(
    path.join(projection.dataDir, "vector-index"),
    "vector",
    GENERATION,
    { projection_digest: `sha256:${"9".repeat(64)}` },
  );
  await assert.rejects(
    validateInstalledRecoveryEvidence({
      dataDir: projection.dataDir,
      diagnostics: diagnostics(),
      runTool: metadataAuthority(),
      search: searchResponse(),
      status: readyStatus(),
    }),
    /search_artifact_projection_mismatch/,
  );

  const epoch = await fixture(context);
  await assert.rejects(
    validateInstalledRecoveryEvidence({
      dataDir: epoch.dataDir,
      diagnostics: diagnostics({ visible_epoch: 8 }),
      runTool: metadataAuthority(),
      search: searchResponse(),
      status: readyStatus(),
    }),
    /ready_evidence_epoch_mismatch/,
  );
});

test("rejects fulltext or vector bytes that do not match the manifest artifact digest", async (context) => {
  for (const [root, file] of [
    ["search-index", "fulltext.snapshot.enc"],
    ["vector-index", "vector.snapshot.enc"],
  ]) {
    const { dataDir } = await fixture(context);
    await writePrivate(
      path.join(dataDir, root, "snapshots", GENERATION, file),
      "tampered encrypted snapshot",
    );

    await assert.rejects(
      validateInstalledRecoveryEvidence({
        dataDir,
        diagnostics: diagnostics(),
        runTool: metadataAuthority(),
        search: searchResponse(),
        status: readyStatus(),
      }),
      /search_artifact_digest_mismatch/,
    );
  }
});

test("rejects snapshot keys above the explicit small-file bound", async (context) => {
  const { dataDir } = await fixture(context);
  await writePrivate(
    path.join(
      dataDir,
      "search-index",
      "snapshots",
      GENERATION,
      "fulltext.snapshot.key-v3",
    ),
    "k".repeat(129),
  );

  await assert.rejects(
    validateInstalledRecoveryEvidence({
      dataDir,
      diagnostics: diagnostics(),
      runTool: metadataAuthority(),
      search: searchResponse(),
      status: readyStatus(),
    }),
    /search_artifact_manifest_invalid/,
  );
});

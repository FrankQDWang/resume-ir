import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import path from "node:path";

import { CLOSED_SYSTEM_TOOL_ENV } from "../macos-system-tools.mjs";
import { toolSucceeded } from "./bounded-process.mjs";
import { TOOL_TIMEOUT_MS, exactKeys, fail } from "./core.mjs";
import {
  readActiveStoreManifest,
  readPrivateJson,
  requirePrivateFile,
  requireSecureDirectory,
} from "./filesystem-cow.mjs";
import { validateDaemonDiagnostics } from "./ipc-contracts.mjs";

const CONTENT_DIGEST = /^sha256:[a-f0-9]{64}$/;
const GENERATION = /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/;
const DOCUMENT_ID = /^doc_[a-f0-9]{32}$/;
const VERSION_ID = /^ver_[a-f0-9]{32}$/;
const MAX_METADATA_BYTES = 64 * 1024 * 1024 * 1024;
const MAX_MANIFEST_BYTES = 4 * 1024;
const MAX_ENCRYPTED_SNAPSHOT_BYTES = 64 * 1024 * 1024 * 1024;
const MAX_SNAPSHOT_KEY_BYTES = 128;
const MAX_DOCUMENTS = 10_000_000;
const HASH_CHUNK_BYTES = 64 * 1024;
const SEARCH_REQUEST_ID = "installed-main-synthetic-witness";
export const SYNTHETIC_CANARY_TOKEN =
  "resumeirinstalledmaincanary7f3d9b2c";
const SQLITE3 = "/usr/bin/sqlite3";
const METADATA_AUTHORITY_SQL = `
SELECT head.generation,
       head.visible_epoch,
       head.service_state,
       publication.state,
       publication.projection_digest,
       publication.fulltext_generation,
       publication.fulltext_manifest_schema,
       publication.fulltext_index_schema,
       publication.fulltext_document_count,
       publication.fulltext_projection_digest,
       publication.fulltext_logical_content_digest,
       publication.vector_generation,
       publication.vector_manifest_schema,
       publication.vector_index_schema,
       publication.vector_mode,
       hex(COALESCE(publication.vector_model_id, '')),
       COALESCE(publication.vector_dimension, -1),
       publication.vector_projection_count,
       publication.vector_coverage_digest,
       publication.vector_count,
       publication.vector_document_count,
       publication.vector_projection_digest,
       publication.vector_logical_content_digest
FROM search_projection_state AS head
JOIN search_publication_journal AS publication
  ON publication.generation = head.generation
WHERE head.state_key = 'default';`.trim();

function boundedCount(value, maximum = Number.MAX_SAFE_INTEGER) {
  return Number.isSafeInteger(value) && value >= 0 && value <= maximum;
}

function boundedLatency(value) {
  return (
    typeof value === "number" &&
    Number.isFinite(value) &&
    value >= 0 &&
    value <= 60_000
  );
}

function validIpcCounts(value) {
  return (
    exactKeys(value, [
      "accepted",
      "completed",
      "client_disconnect",
      "request_failure",
      "response_failure",
    ]) && Object.values(value).every((count) => boundedCount(count))
  );
}

function validQueryLatency(value) {
  return (
    exactKeys(value, [
      "sample_count",
      "p50_ms",
      "p95_ms",
      "p99_ms",
      "last_result_count",
      "raw_queries",
    ]) &&
    boundedCount(value.sample_count) &&
    [value.p50_ms, value.p95_ms, value.p99_ms].every(
      (latency) => latency === null || boundedLatency(latency),
    ) &&
    (value.last_result_count === null ||
      boundedCount(value.last_result_count, MAX_DOCUMENTS)) &&
    value.raw_queries === "<redacted>"
  );
}

export function validateReadyStatus(value) {
  const countKeys = [
    "visible_epoch",
    "indexed_documents",
    "searchable_documents",
    "partial_documents",
    "failed_retryable",
    "failed_permanent",
    "recovery_queue_depth",
    "ocr_queue_depth",
    "ocr_jobs_queued",
    "ocr_page_budget_blocked",
    "ocr_language_unavailable",
    "embedding_queue_depth",
    "entity_mentions",
    "import_tasks_queued",
    "import_tasks_recoverable",
    "import_tasks_cancelled",
    "import_scan_scopes",
    "import_scan_errors",
  ];
  const latest = value?.latest_import_scan;
  if (
    !exactKeys(value, [
      "schema_version",
      "status",
      "process_state",
      "service_state",
      "services",
      "repair_reason",
      "repair_progress",
      "error",
      "ipc",
      "visible_epoch",
      "indexed_documents",
      "searchable_documents",
      "partial_documents",
      "failed_retryable",
      "failed_permanent",
      "recovery_queue_depth",
      "ocr_queue_depth",
      "ocr_jobs_queued",
      "ocr_page_budget_blocked",
      "ocr_remediation",
      "ocr_language_unavailable",
      "ocr_language_remediation",
      "embedding_queue_depth",
      "entity_mentions",
      "import_tasks_queued",
      "import_tasks_recoverable",
      "import_tasks_cancelled",
      "import_scan_scopes",
      "import_scan_errors",
      "query_latency",
      "latest_import_scan",
      "active_profile",
      "index_health",
      "snapshot_present",
    ]) ||
    value.schema_version !== "daemon.status.v2" ||
    value.status !== "ok" ||
    value.process_state !== "ready" ||
    value.service_state !== "ready" ||
    !exactKeys(value.services, ["metadata", "query"]) ||
    value.services.metadata !== "ready" ||
    value.services.query !== "ready" ||
    value.repair_reason !== null ||
    value.repair_progress !== null ||
    value.error !== null ||
    !validIpcCounts(value.ipc) ||
    !countKeys.every((key) => boundedCount(value[key], MAX_DOCUMENTS)) ||
    value.visible_epoch < 1 ||
    value.searchable_documents > value.indexed_documents ||
    value.partial_documents > value.indexed_documents ||
    typeof value.ocr_remediation !== "string" ||
    value.ocr_remediation.length > 256 ||
    typeof value.ocr_language_remediation !== "string" ||
    value.ocr_language_remediation.length > 256 ||
    !validQueryLatency(value.query_latency) ||
    (latest !== null &&
      (!exactKeys(latest, [
        "scan_profile",
        "files_discovered",
        "ignored_entries",
        "scan_errors",
        "searchable_documents",
        "ocr_required_documents",
        "ocr_jobs_queued",
        "failed_documents",
        "deleted_documents",
        "scan_budget_observed",
        "scan_budget_limit",
        "scan_budget_exhausted",
      ]) ||
        !["explicit", "discovery"].includes(latest.scan_profile) ||
        !Object.entries(latest)
          .filter(
            ([key]) =>
              ![
                "scan_profile",
                "scan_budget_observed",
                "scan_budget_limit",
                "scan_budget_exhausted",
              ].includes(key),
          )
          .every(([, count]) => boundedCount(count, MAX_DOCUMENTS)) ||
        ![latest.scan_budget_observed, latest.scan_budget_limit].every(
          (count) => count === null || boundedCount(count, MAX_DOCUMENTS),
        ) ||
        typeof latest.scan_budget_exhausted !== "boolean")) ||
    value.active_profile !== "balanced" ||
    value.index_health !== "ready" ||
    value.snapshot_present !== true
  ) {
    fail("ready_status_contract_invalid");
  }
  return value;
}

export function validateSyntheticSearchResponse(value) {
  const results = value?.results;
  const stages = value?.stage_latency_ms;
  if (
    !exactKeys(value, [
      "schema_version",
      "request_id",
      "status",
      "visible_epoch",
      "query_mode",
      "partial",
      "partial_reasons",
      "latency_ms",
      "stage_latency_ms",
      "search_index",
      "result_count",
      "results",
    ]) ||
    value.schema_version !== "resume-ir.search-response.v3" ||
    value.request_id !== SEARCH_REQUEST_ID ||
    value.status !== "ok" ||
    !boundedCount(value.visible_epoch) ||
    value.visible_epoch < 1 ||
    value.query_mode !== "fulltext" ||
    value.partial !== false ||
    !Array.isArray(value.partial_reasons) ||
    value.partial_reasons.length !== 0 ||
    !boundedLatency(value.latency_ms) ||
    !exactKeys(stages, [
      "parse",
      "prefilter",
      "bm25",
      "ann",
      "fusion",
      "bulk_hydrate",
      "snippet",
    ]) ||
    !Object.values(stages).every(boundedLatency) ||
    value.search_index !== "available" ||
    !boundedCount(value.result_count, 1) ||
    !Array.isArray(results) ||
    results.length !== value.result_count ||
    !results.every(
      (result, index) =>
        exactKeys(result, ["rank", "selection", "file_name", "snippet"]) &&
        result.rank === index + 1 &&
        exactKeys(result.selection, [
          "doc_id",
          "version_id",
          "visible_epoch",
        ]) &&
        DOCUMENT_ID.test(result.selection.doc_id ?? "") &&
        VERSION_ID.test(result.selection.version_id ?? "") &&
        result.selection.visible_epoch === value.visible_epoch &&
        typeof result.file_name === "string" &&
        Buffer.byteLength(result.file_name, "utf8") <= 4_096 &&
        typeof result.snippet === "string" &&
        Buffer.byteLength(result.snippet, "utf8") <= 64 * 1024,
    )
  ) {
    fail("search_witness_invalid");
  }
  return value;
}

async function validateMetadataArtifact(dataDir) {
  let before;
  try {
    before = await readActiveStoreManifest(dataDir);
  } catch {
    fail("metadata_artifact_invalid");
  }
  const expectedName = `metadata-v29-${before.digest.slice(0, 16)}.sqlite3`;
  if (before.schema !== 29 || before.fileName !== expectedName) {
    fail("metadata_artifact_invalid");
  }
  const file = path.join(dataDir, before.fileName);
  let metadata;
  try {
    metadata = await requirePrivateFile(file, {
      maxBytes: MAX_METADATA_BYTES,
    });
  } catch {
    fail("metadata_artifact_invalid");
  }
  let current;
  let resolved;
  try {
    [current, resolved] = await Promise.all([lstat(file), realpath(file)]);
  } catch {
    fail("metadata_artifact_invalid");
  }
  if (
    resolved !== file ||
    current.dev !== metadata.dev ||
    current.ino !== metadata.ino ||
    current.size !== metadata.size ||
    metadata.size < 1 ||
    metadata.size > MAX_METADATA_BYTES
  ) {
    fail("metadata_artifact_invalid");
  }
  return Object.freeze({ file, manifest: before, metadata });
}

function parseMetadataAuthorityLine(source) {
  if (
    typeof source !== "string" ||
    !source.endsWith("\n") ||
    source.slice(0, -1).includes("\n")
  ) {
    fail("metadata_authority_invalid");
  }
  const fields = source.slice(0, -1).split("\t");
  if (fields.length !== 23) fail("metadata_authority_invalid");
  const number = (index) => {
    const value = Number(fields[index]);
    if (!boundedCount(value, MAX_DOCUMENTS)) {
      fail("metadata_authority_invalid");
    }
    return value;
  };
  const modelHex = fields[15];
  if (
    modelHex.length > 512 ||
    modelHex.length % 2 !== 0 ||
    !/^[A-F0-9]*$/.test(modelHex)
  ) {
    fail("metadata_authority_invalid");
  }
  const modelId = Buffer.from(modelHex, "hex").toString("utf8");
  if (Buffer.from(modelId, "utf8").toString("hex").toUpperCase() !== modelHex) {
    fail("metadata_authority_invalid");
  }
  const dimension = Number(fields[16]);
  const authority = {
    generation: fields[0],
    visibleEpoch: number(1),
    serviceState: fields[2],
    publicationState: fields[3],
    projectionDigest: fields[4],
    fulltextGeneration: fields[5],
    fulltextManifestSchema: fields[6],
    fulltextIndexSchema: fields[7],
    fulltextDocumentCount: number(8),
    fulltextProjectionDigest: fields[9],
    fulltextLogicalContentDigest: fields[10],
    vectorGeneration: fields[11],
    vectorManifestSchema: fields[12],
    vectorIndexSchema: fields[13],
    vectorMode: fields[14],
    vectorModelId: modelId === "" ? null : modelId,
    vectorDimension: dimension === -1 ? null : dimension,
    vectorProjectionCount: number(17),
    vectorCoverageDigest: fields[18],
    vectorCount: number(19),
    vectorDocumentCount: number(20),
    vectorProjectionDigest: fields[21],
    vectorLogicalContentDigest: fields[22],
  };
  if (
    !GENERATION.test(authority.generation) ||
    authority.visibleEpoch < 1 ||
    authority.serviceState !== "ready" ||
    authority.publicationState !== "ready" ||
    !CONTENT_DIGEST.test(authority.projectionDigest) ||
    authority.fulltextGeneration !== authority.generation ||
    authority.fulltextManifestSchema !== "fulltext.snapshot.v3" ||
    authority.fulltextIndexSchema !== "tantivy.fulltext.v3" ||
    !CONTENT_DIGEST.test(authority.fulltextProjectionDigest) ||
    !CONTENT_DIGEST.test(authority.fulltextLogicalContentDigest) ||
    authority.vectorGeneration !== authority.generation ||
    authority.vectorManifestSchema !== "vector.snapshot.v4" ||
    authority.vectorIndexSchema !== "hnsw-vector.v4" ||
    !["disabled", "enabled"].includes(authority.vectorMode) ||
    (authority.vectorMode === "disabled" &&
      (authority.vectorModelId !== null ||
        authority.vectorDimension !== null ||
        authority.vectorCount !== 0 ||
        authority.vectorDocumentCount !== 0)) ||
    (authority.vectorMode === "enabled" &&
      (authority.vectorModelId === null ||
        authority.vectorModelId.length > 128 ||
        !Number.isSafeInteger(authority.vectorDimension) ||
        authority.vectorDimension < 1 ||
        authority.vectorDimension > 65_536)) ||
    !CONTENT_DIGEST.test(authority.vectorCoverageDigest) ||
    !CONTENT_DIGEST.test(authority.vectorProjectionDigest) ||
    !CONTENT_DIGEST.test(authority.vectorLogicalContentDigest)
  ) {
    fail("metadata_authority_invalid");
  }
  return Object.freeze(authority);
}

async function readMetadataAuthority(file, runTool) {
  if (typeof runTool !== "function") fail("metadata_authority_invalid");
  const result = await runTool(
    SQLITE3,
    [
      "-batch",
      "-readonly",
      "-noheader",
      "-separator",
      "\t",
      file,
      METADATA_AUTHORITY_SQL,
    ],
    { env: CLOSED_SYSTEM_TOOL_ENV, timeoutMs: TOOL_TIMEOUT_MS },
  );
  if (!toolSucceeded(result) || result.stderr !== "") {
    fail("metadata_authority_invalid");
  }
  return parseMetadataAuthorityLine(result.stdout);
}

function parseFulltextManifest(value, generation) {
  if (
    !exactKeys(value, [
      "schema_version",
      "index_schema",
      "encrypted_snapshot",
      "generation",
      "document_count",
      "projection_digest",
      "logical_content_digest",
      "artifact_digest",
    ]) ||
    value.schema_version !== "fulltext.snapshot.v3" ||
    value.index_schema !== "tantivy.fulltext.v3" ||
    value.encrypted_snapshot !==
      "resume-ir-fulltext-snapshot-encrypted-v3" ||
    value.generation !== generation ||
    !boundedCount(value.document_count, MAX_DOCUMENTS) ||
    !CONTENT_DIGEST.test(value.projection_digest ?? "") ||
    !CONTENT_DIGEST.test(value.logical_content_digest ?? "") ||
    !CONTENT_DIGEST.test(value.artifact_digest ?? "")
  ) {
    fail("search_artifact_manifest_invalid");
  }
  return value;
}

function parseVectorManifest(value, generation) {
  const modelDisabled = value?.model_id === null && value?.dimension === null;
  const modelEnabled =
    typeof value?.model_id === "string" &&
    value.model_id.length > 0 &&
    value.model_id.length <= 256 &&
    Number.isSafeInteger(value.dimension) &&
    value.dimension > 0 &&
    value.dimension <= 65_536;
  if (
    !exactKeys(value, [
      "schema_version",
      "index_schema",
      "generation",
      "model_id",
      "dimension",
      "vector_count",
      "projection_count",
      "vector_document_count",
      "projection_digest",
      "coverage_digest",
      "logical_content_digest",
      "artifact_digest",
      "search_backend",
      "encryption",
    ]) ||
    value.schema_version !== "vector.snapshot.v4" ||
    value.index_schema !== "hnsw-vector.v4" ||
    value.generation !== generation ||
    (!modelDisabled && !modelEnabled) ||
    !boundedCount(value.vector_count, MAX_DOCUMENTS) ||
    !boundedCount(value.projection_count, MAX_DOCUMENTS) ||
    !boundedCount(value.vector_document_count, MAX_DOCUMENTS) ||
    value.vector_document_count > value.vector_count ||
    value.vector_document_count > value.projection_count ||
    (modelDisabled &&
      (value.vector_count !== 0 || value.vector_document_count !== 0)) ||
    !CONTENT_DIGEST.test(value.projection_digest ?? "") ||
    !CONTENT_DIGEST.test(value.coverage_digest ?? "") ||
    !CONTENT_DIGEST.test(value.logical_content_digest ?? "") ||
    !CONTENT_DIGEST.test(value.artifact_digest ?? "") ||
    value.search_backend !== "hnsw_ann" ||
    value.encryption !== "xchacha20poly1305.v1"
  ) {
    fail("search_artifact_manifest_invalid");
  }
  return value;
}

async function hashPrivateSnapshot(file) {
  let expected;
  let resolved;
  let handle;
  try {
    [expected, resolved] = await Promise.all([lstat(file), realpath(file)]);
    if (
      resolved !== path.resolve(file) ||
      !expected.isFile() ||
      expected.isSymbolicLink() ||
      expected.uid !== process.getuid() ||
      expected.nlink !== 1 ||
      (expected.mode & 0o777) !== 0o600 ||
      expected.size < 1 ||
      expected.size > MAX_ENCRYPTED_SNAPSHOT_BYTES ||
      !Number.isInteger(constants.O_NOFOLLOW)
    ) {
      fail("search_artifact_file_invalid");
    }
    handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
    const openedBefore = await handle.stat();
    if (
      !openedBefore.isFile() ||
      openedBefore.dev !== expected.dev ||
      openedBefore.ino !== expected.ino ||
      openedBefore.size !== expected.size ||
      openedBefore.uid !== expected.uid ||
      openedBefore.nlink !== expected.nlink ||
      (openedBefore.mode & 0o777) !== 0o600
    ) {
      fail("search_artifact_file_invalid");
    }

    const digest = createHash("sha256");
    const chunk = Buffer.allocUnsafe(HASH_CHUNK_BYTES);
    let position = 0;
    while (position < expected.size) {
      const length = Math.min(chunk.length, expected.size - position);
      const { bytesRead } = await handle.read(chunk, 0, length, position);
      if (bytesRead < 1 || bytesRead > length) {
        fail("search_artifact_file_invalid");
      }
      digest.update(chunk.subarray(0, bytesRead));
      position += bytesRead;
    }

    const [openedAfter, current, currentResolved] = await Promise.all([
      handle.stat(),
      lstat(file),
      realpath(file),
    ]);
    if (
      position !== expected.size ||
      !openedAfter.isFile() ||
      openedAfter.dev !== expected.dev ||
      openedAfter.ino !== expected.ino ||
      openedAfter.size !== expected.size ||
      openedAfter.uid !== expected.uid ||
      openedAfter.nlink !== expected.nlink ||
      (openedAfter.mode & 0o777) !== 0o600 ||
      !current.isFile() ||
      current.isSymbolicLink() ||
      current.dev !== expected.dev ||
      current.ino !== expected.ino ||
      current.size !== expected.size ||
      current.uid !== expected.uid ||
      current.nlink !== expected.nlink ||
      (current.mode & 0o777) !== 0o600 ||
      current.size < 1 ||
      current.size > MAX_ENCRYPTED_SNAPSHOT_BYTES ||
      currentResolved !== resolved
    ) {
      fail("search_artifact_file_invalid");
    }
    return `sha256:${digest.digest("hex")}`;
  } catch {
    fail("search_artifact_file_invalid");
  } finally {
    await handle?.close().catch(() => {});
  }
}

async function readExactGeneration(dataDir, kind, generation) {
  const root = path.join(
    dataDir,
    kind === "fulltext" ? "search-index" : "vector-index",
  );
  const snapshots = path.join(root, "snapshots");
  await requireSecureDirectory(root, { privateMode: true });
  await requireSecureDirectory(snapshots, { privateMode: true });
  if (!GENERATION.test(generation)) fail("search_artifact_generation_invalid");
  const directory = path.join(snapshots, generation);
  try {
    await requireSecureDirectory(directory, { privateMode: true });
  } catch {
    fail("search_artifact_generation_invalid");
  }
  const manifestFile = path.join(directory, "snapshot-manifest.json");
  const manifest = await readPrivateJson(manifestFile, MAX_MANIFEST_BYTES);
  if (kind === "fulltext" && !manifest.source.endsWith("\n")) {
    fail("search_artifact_manifest_invalid");
  }
  const manifestValue =
    kind === "fulltext"
      ? parseFulltextManifest(manifest.value, generation)
      : parseVectorManifest(manifest.value, generation);
  const encryptedFile = path.join(
    directory,
    kind === "fulltext" ? "fulltext.snapshot.enc" : "vector.snapshot.enc",
  );
  const keyFile = path.join(
    directory,
    kind === "fulltext"
      ? "fulltext.snapshot.key-v3"
      : "vector.snapshot.key-v4",
  );
  let artifactDigest;
  try {
    [artifactDigest] = await Promise.all([
      hashPrivateSnapshot(encryptedFile),
      requirePrivateFile(keyFile, { maxBytes: MAX_SNAPSHOT_KEY_BYTES }),
    ]);
  } catch {
    fail("search_artifact_manifest_invalid");
  }
  if (artifactDigest !== manifestValue.artifact_digest) {
    fail("search_artifact_digest_mismatch");
  }
  return manifestValue;
}

async function validateReadyArtifacts(
  { dataDir, runTool },
  ready,
  redacted,
) {
  if (
    redacted.visible_epoch !== ready.visible_epoch ||
    redacted.metrics.indexed_documents !== ready.indexed_documents ||
    redacted.metrics.searchable_documents !== ready.searchable_documents ||
    redacted.metrics.partial_documents !== ready.partial_documents
  ) {
    fail("ready_evidence_epoch_mismatch");
  }
  const metadataBefore = await validateMetadataArtifact(dataDir);
  const authority = await readMetadataAuthority(metadataBefore.file, runTool);
  const [fulltext, vector] = await Promise.all([
    readExactGeneration(dataDir, "fulltext", authority.fulltextGeneration),
    readExactGeneration(dataDir, "vector", authority.vectorGeneration),
  ]);
  const metadataAfter = await validateMetadataArtifact(dataDir);
  if (
    metadataAfter.manifest.schema !== metadataBefore.manifest.schema ||
    metadataAfter.manifest.fileName !== metadataBefore.manifest.fileName ||
    metadataAfter.manifest.digest !== metadataBefore.manifest.digest ||
    metadataAfter.metadata.dev !== metadataBefore.metadata.dev ||
    metadataAfter.metadata.ino !== metadataBefore.metadata.ino ||
    metadataAfter.metadata.size !== metadataBefore.metadata.size
  ) {
    fail("metadata_artifact_invalid");
  }
  if (
    authority.visibleEpoch !== ready.visible_epoch ||
    authority.projectionDigest !== authority.fulltextProjectionDigest ||
    authority.projectionDigest !== authority.vectorProjectionDigest ||
    authority.fulltextGeneration !== authority.vectorGeneration ||
    fulltext.generation !== authority.generation ||
    vector.generation !== authority.generation
  ) {
    fail("search_artifact_generation_invalid");
  }
  if (
    fulltext.projection_digest !== authority.projectionDigest ||
    vector.projection_digest !== authority.projectionDigest ||
    fulltext.logical_content_digest !==
      authority.fulltextLogicalContentDigest ||
    vector.logical_content_digest !== authority.vectorLogicalContentDigest ||
    vector.coverage_digest !== authority.vectorCoverageDigest ||
    fulltext.document_count !== authority.fulltextDocumentCount ||
    vector.projection_count !== authority.vectorProjectionCount ||
    vector.vector_count !== authority.vectorCount ||
    vector.vector_document_count !== authority.vectorDocumentCount ||
    fulltext.document_count !== ready.searchable_documents ||
    vector.projection_count !== ready.searchable_documents ||
    (authority.vectorMode === "disabled" &&
      (vector.model_id !== null || vector.dimension !== null)) ||
    (authority.vectorMode === "enabled" &&
      (vector.model_id !== authority.vectorModelId ||
        vector.dimension !== authority.vectorDimension))
  ) {
    fail("search_artifact_projection_mismatch");
  }
  return Object.freeze({
    generationAgreement: true,
    metadataArtifactBound: true,
  });
}

export async function validateInstalledReadyArtifacts({
  dataDir,
  diagnostics,
  runTool,
  status,
}) {
  const ready = validateReadyStatus(status);
  const redacted = validateDaemonDiagnostics(diagnostics);
  return validateReadyArtifacts({ dataDir, runTool }, ready, redacted);
}

export async function validateInstalledRecoveryEvidence({
  dataDir,
  diagnostics,
  runTool,
  search,
  status,
}) {
  const ready = validateReadyStatus(status);
  const redacted = validateDaemonDiagnostics(diagnostics);
  const witness = validateSyntheticSearchResponse(search);
  if (witness.visible_epoch !== ready.visible_epoch) {
    fail("ready_evidence_epoch_mismatch");
  }
  const artifactEvidence = await validateReadyArtifacts(
    { dataDir, runTool },
    ready,
    redacted,
  );
  return Object.freeze({
    ...artifactEvidence,
    searchWitness: true,
  });
}

export const SYNTHETIC_SEARCH_REQUEST = Object.freeze({
  schema_version: "resume-ir.ipc-request.v3",
  request_id: SEARCH_REQUEST_ID,
  client_capability: "codex_validation",
  deadline_ms: 5_000,
  payload: Object.freeze({
    query: SYNTHETIC_CANARY_TOKEN,
    mode: "fulltext",
    top_k: 1,
  }),
});

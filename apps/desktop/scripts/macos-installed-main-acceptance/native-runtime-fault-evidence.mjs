import { constants } from "node:fs";
import { chmod, copyFile, lstat, mkdir, realpath } from "node:fs/promises";
import path from "node:path";

import { CLOSED_SYSTEM_TOOL_ENV } from "../macos-system-tools.mjs";
import { SYNTHETIC_CANARY_TOKEN } from "./acceptance-evidence.mjs";
import { toolSucceeded } from "./bounded-process.mjs";
import {
  TOOL_TIMEOUT_MS,
  exactKeys,
  fail,
} from "./core.mjs";
import { readActiveStoreManifest } from "./filesystem-cow.mjs";
import {
  requestJsonPost,
  requestJsonPostAccepted,
  requestJsonPostServiceUnavailable,
} from "./ipc-contracts.mjs";
import { SYNTHETIC_CANARY_FILE_NAME } from "./synthetic-canary.mjs";

const OCR_FIXTURE = path.join(
  "tests",
  "fixtures",
  "resumes",
  "synthetic-scanned-resume.pdf",
);
const OCR_FAULT_ROOT = ".resume-ir-optional-runtime-ocr-fixture";
const CLASSIFIER_EPOCH = /^[a-z0-9_]{1,64}$/;
const DOCUMENT_ID = /^doc_[a-f0-9]{32}$/;
const VERSION_ID = /^ver_[a-f0-9]{32}$/;
const IMPORT_TASK_ID = /^imp_[a-f0-9]{32}$/;
const SQLITE3 = "/usr/bin/sqlite3";
const MAX_LATENCY_MS = 60_000;
const MAX_TEXT_BYTES = 64 * 1024;
const STAGE_KEYS = [
  "parse",
  "prefilter",
  "bm25",
  "ann",
  "fusion",
  "bulk_hydrate",
  "snippet",
];

function searchRequest(requestId, mode) {
  return {
    schema_version: "resume-ir.ipc-request.v3",
    request_id: requestId,
    client_capability: "codex_validation",
    deadline_ms: 5_000,
    payload: {
      query: SYNTHETIC_CANARY_TOKEN,
      mode,
      top_k: 1,
    },
  };
}

function sameSelection(actual, expected) {
  return (
    validSelection(actual) &&
    actual.doc_id === expected.doc_id &&
    actual.version_id === expected.version_id &&
    actual.visible_epoch === expected.visible_epoch
  );
}

function validSelection(selection) {
  return (
    exactKeys(selection, ["doc_id", "version_id", "visible_epoch"]) &&
    DOCUMENT_ID.test(selection.doc_id ?? "") &&
    VERSION_ID.test(selection.version_id ?? "") &&
    Number.isSafeInteger(selection.visible_epoch) &&
    selection.visible_epoch >= 1
  );
}

function boundedLatency(value) {
  return Number.isFinite(value) && value >= 0 && value <= MAX_LATENCY_MS;
}

function validateSearchWitness(
  response,
  requestId,
  responseMode,
  expectedSelection,
) {
  const hit = response?.results?.[0];
  if (
    !exactKeys(response, [
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
    response.schema_version !== "resume-ir.search-response.v3" ||
    response.request_id !== requestId ||
    response.status !== "ok" ||
    !Number.isSafeInteger(response.visible_epoch) ||
    response.visible_epoch < 1 ||
    response.query_mode !== responseMode ||
    typeof response.partial !== "boolean" ||
    !Array.isArray(response.partial_reasons) ||
    response.partial_reasons.length > 4 ||
    !response.partial_reasons.every((reason) =>
      [
        "search_index_not_ready",
        "deadline_exceeded",
        "embedding_runtime_unavailable",
      ].includes(reason),
    ) ||
    !boundedLatency(response.latency_ms) ||
    !exactKeys(response.stage_latency_ms, STAGE_KEYS) ||
    !Object.values(response.stage_latency_ms).every(boundedLatency) ||
    response.search_index !== "available" ||
    response.result_count !== 1 ||
    !Array.isArray(response.results) ||
    response.results.length !== 1 ||
    !exactKeys(hit, ["rank", "selection", "file_name", "snippet"]) ||
    hit.rank !== 1 ||
    !validSelection(hit.selection) ||
    hit.selection.visible_epoch !== response.visible_epoch ||
    hit.file_name !== SYNTHETIC_CANARY_FILE_NAME ||
    typeof hit.snippet !== "string" ||
    Buffer.byteLength(hit.snippet, "utf8") > MAX_TEXT_BYTES ||
    (expectedSelection !== null &&
      !sameSelection(hit.selection, expectedSelection)) ||
    (responseMode === "keyword" &&
      (response.partial !== false || response.partial_reasons.length !== 0))
  ) {
    fail("optional_runtime_data_plane_invalid");
  }
  return response;
}

export async function captureSyntheticFaultWitness(connection, signal) {
  const response = await requestJsonPost(
    connection.urls.search,
    connection.token,
    searchRequest("runtime-fault-witness", "fulltext"),
    { signal },
  );
  const validated = validateSearchWitness(
    response,
    "runtime-fault-witness",
    "keyword",
    null,
  );
  if (validated.result_count !== 1) fail("optional_runtime_data_plane_invalid");
  return Object.freeze({ ...response.results[0].selection });
}

export async function validateEmbeddingFaultDataPlane(
  connection,
  expectedSelection,
  signal,
) {
  const keyword = await requestJsonPost(
    connection.urls.search,
    connection.token,
    searchRequest("runtime-fault-keyword", "fulltext"),
    { signal },
  );
  validateSearchWitness(
    keyword,
    "runtime-fault-keyword",
    "keyword",
    expectedSelection,
  );
  const hybrid = await requestJsonPost(
    connection.urls.search,
    connection.token,
    searchRequest("runtime-fault-hybrid", "hybrid"),
    { signal },
  );
  validateSearchWitness(
    hybrid,
    "runtime-fault-hybrid",
    "hybrid",
    expectedSelection,
  );
  if (
    hybrid.partial !== true ||
    JSON.stringify(hybrid.partial_reasons) !==
      JSON.stringify(["embedding_runtime_unavailable"])
  ) {
    fail("optional_runtime_data_plane_invalid");
  }
  const detail = await requestJsonPost(
    connection.urls.details,
    connection.token,
    {
      schema_version: "resume-ir.detail-request.v3",
      request_id: "runtime-fault-detail",
      selection: expectedSelection,
    },
    { signal },
  );
  if (
    !exactKeys(detail, [
      "schema_version",
      "request_id",
      "selection",
      "status",
      "document",
      "limits",
    ]) ||
    detail?.schema_version !== "resume-ir.detail-response.v3" ||
    detail?.request_id !== "runtime-fault-detail" ||
    detail?.status !== "ok" ||
    !sameSelection(detail.selection, expectedSelection) ||
    !exactKeys(detail.document, [
      "source_byte_size",
      "parse_version",
      "schema_version",
      "language_set",
      "page_count",
      "quality_score",
      "fields_truncated",
      "fields",
      "snippet",
    ]) ||
    !Number.isSafeInteger(detail.document.source_byte_size) ||
    detail.document.source_byte_size < 0 ||
    typeof detail.document.parse_version !== "string" ||
    typeof detail.document.schema_version !== "string" ||
    !Array.isArray(detail.document.language_set) ||
    (detail.document.page_count !== null &&
      (!Number.isSafeInteger(detail.document.page_count) ||
        detail.document.page_count < 1)) ||
    (detail.document.quality_score !== null &&
      (!Number.isFinite(detail.document.quality_score) ||
        detail.document.quality_score < 0 ||
        detail.document.quality_score > 1)) ||
    typeof detail.document.fields_truncated !== "boolean" ||
    !Array.isArray(detail.document.fields) ||
    !detail.document.fields.every(
      (field) =>
        exactKeys(field, ["type", "value", "confidence"]) &&
        typeof field.type === "string" &&
        typeof field.value === "string" &&
        Number.isFinite(field.confidence),
    ) ||
    typeof detail.document.snippet !== "string" ||
    Buffer.byteLength(detail.document.snippet, "utf8") > MAX_TEXT_BYTES ||
    !exactKeys(detail.limits, ["max_fields", "max_response_bytes"]) ||
    detail.limits.max_fields !== 256 ||
    detail.limits.max_response_bytes !== 1024 * 1024 ||
    detail.document.fields.length > detail.limits.max_fields
  ) {
    fail("optional_runtime_data_plane_invalid");
  }
  return Object.freeze({
    detailAvailable: true,
    hybridLexicalPartial: true,
    keywordAvailable: true,
    selectionPreserved: true,
  });
}

export async function createOcrFaultFixture(workspace, repoRoot) {
  const source = path.join(repoRoot, OCR_FIXTURE);
  const root = path.join(workspace.root, OCR_FAULT_ROOT);
  const destination = path.join(root, "synthetic-scanned-resume.pdf");
  if (
    !path.isAbsolute(repoRoot) ||
    !path.isAbsolute(workspace?.root ?? "") ||
    path.relative(repoRoot, source) !== OCR_FIXTURE
  ) {
    fail("optional_runtime_fixture_invalid");
  }
  try {
    await mkdir(root, { mode: 0o700 });
    await chmod(root, 0o700);
    const [resolvedSource, sourceMetadata] = await Promise.all([
      realpath(source),
      lstat(source),
    ]);
    if (
      resolvedSource !== source ||
      !sourceMetadata.isFile() ||
      sourceMetadata.isSymbolicLink() ||
      sourceMetadata.size < 1
    ) {
      fail("optional_runtime_fixture_invalid");
    }
    await copyFile(source, destination, constants.COPYFILE_EXCL);
    await chmod(destination, 0o600);
    const [resolvedRoot, resolvedFile, rootMetadata, metadata] =
      await Promise.all([
        realpath(root),
        realpath(destination),
        lstat(root),
        lstat(destination),
      ]);
    if (
      resolvedRoot !== root ||
      resolvedFile !== destination ||
      !rootMetadata.isDirectory() ||
      rootMetadata.isSymbolicLink() ||
      (rootMetadata.mode & 0o777) !== 0o700 ||
      !metadata.isFile() ||
      metadata.isSymbolicLink() ||
      metadata.size < 1 ||
      metadata.nlink !== 1 ||
      metadata.uid !== process.getuid() ||
      (metadata.mode & 0o777) !== 0o600
    ) {
      fail("optional_runtime_fixture_invalid");
    }
  } catch (error) {
    if (error?.code === "optional_runtime_fixture_invalid") throw error;
    fail("optional_runtime_fixture_invalid");
  }
  return Object.freeze({
    request: Object.freeze({
      roots: Object.freeze([root]),
      profile: "explicit",
      max_files: 1,
    }),
    root,
  });
}

export async function submitImportExpectedRejected(connection, request, signal) {
  const response = await requestJsonPostServiceUnavailable(
    connection.urls.imports,
    connection.token,
    request,
    { signal },
  );
  if (
    !exactKeys(response, ["schema_version", "status", "error"]) ||
    response.schema_version !== "resume-ir.error.v2" ||
    response.status !== "error" ||
    !exactKeys(response.error, ["code", "action", "capability", "reason"]) ||
    response.error.code !== "CAPABILITY_UNAVAILABLE" ||
    response.error.action !== "select_supported_mode" ||
    response.error.capability !== "text_import" ||
    response.error.reason !== "classifier_unavailable"
  ) {
    fail("optional_runtime_claim_gate_invalid");
  }
}

export async function submitOcrBacklogImport(connection, request, signal) {
  const response = await requestJsonPostAccepted(
    connection.urls.imports,
    connection.token,
    request,
    { signal },
  );
  if (
    !exactKeys(response, [
      "schema_version",
      "status",
      "accepted_roots",
      "new_tasks",
      "task_ids",
      "scan_profile",
      "scan_file_limit",
    ]) ||
    response?.schema_version !== "daemon.import.v1" ||
    response?.status !== "accepted" ||
    response?.accepted_roots !== 1 ||
    response?.new_tasks !== 1 ||
    !Array.isArray(response?.task_ids) ||
    response.task_ids.length !== 1 ||
    !IMPORT_TASK_ID.test(response.task_ids[0] ?? "") ||
    response.scan_profile !== "explicit" ||
    response.scan_file_limit !== 1
  ) {
    fail("optional_runtime_claim_gate_invalid");
  }
}

export async function readActiveClassifierEpoch(dataDir, runTool) {
  const manifest = await readActiveStoreManifest(dataDir);
  const result = await runTool(
    SQLITE3,
    [
      "-batch",
      "-readonly",
      "-noheader",
      path.join(dataDir, manifest.fileName),
      "SELECT classifier_epoch FROM search_publication_journal WHERE generation = (SELECT generation FROM search_projection_state WHERE state_key = 'default');",
    ],
    { env: CLOSED_SYSTEM_TOOL_ENV, timeoutMs: TOOL_TIMEOUT_MS },
  );
  const epoch = result?.stdout?.endsWith("\n")
    ? result.stdout.slice(0, -1)
    : "";
  if (
    !toolSucceeded(result) ||
    result.stderr !== "" ||
    result.stdout.slice(0, -1).includes("\n") ||
    !CLASSIFIER_EPOCH.test(epoch)
  ) {
    fail("optional_runtime_classifier_epoch_invalid");
  }
  return epoch;
}

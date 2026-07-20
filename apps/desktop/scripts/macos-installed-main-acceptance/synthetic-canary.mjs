import { constants } from "node:fs";
import { chmod, lstat, mkdir, open, realpath } from "node:fs/promises";
import path from "node:path";

import {
  SYNTHETIC_CANARY_TOKEN,
  validateReadyStatus,
  validateSyntheticSearchResponse,
} from "./acceptance-evidence.mjs";
import { exactKeys, fail } from "./core.mjs";

export { SYNTHETIC_CANARY_TOKEN };

export const SYNTHETIC_CANARY_FILE_NAME =
  "resume-ir-installed-main-synthetic-canary.txt";
const CANARY_DIRECTORY_NAME = ".resume-ir-installed-main-canary";
const IMPORT_TASK_ID = /^imp_[a-f0-9]{32}$/;
const CANARY_BODY = Object.freeze(
  [
    "SUMMARY",
    "Synthetic Acceptance Candidate",
    "EXPERIENCE",
    `Built ${SYNTHETIC_CANARY_TOKEN} systems`,
    "EDUCATION",
    "Synthetic University",
    "SKILLS",
    SYNTHETIC_CANARY_TOKEN,
    "",
  ].join("\n"),
);

function isDirectChild(parent, child) {
  return path.dirname(child) === parent && child.startsWith(`${parent}${path.sep}`);
}

export async function createSyntheticCanary(workspace) {
  if (
    typeof workspace?.root !== "string" ||
    typeof workspace?.home !== "string" ||
    typeof workspace?.dataDir !== "string" ||
    workspace.root !== workspace.home ||
    !path.isAbsolute(workspace.root) ||
    !path.isAbsolute(workspace.dataDir)
  ) {
    fail("synthetic_canary_invalid");
  }
  let resolvedWorkspace;
  try {
    resolvedWorkspace = await realpath(workspace.root);
  } catch {
    fail("synthetic_canary_invalid");
  }
  if (resolvedWorkspace !== workspace.root) fail("synthetic_canary_invalid");
  const root = path.join(resolvedWorkspace, CANARY_DIRECTORY_NAME);
  const file = path.join(root, SYNTHETIC_CANARY_FILE_NAME);
  let handle;
  try {
    await mkdir(root, { mode: 0o700 });
    await chmod(root, 0o700);
    handle = await open(
      file,
      constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | constants.O_NOFOLLOW,
      0o600,
    );
    await handle.writeFile(CANARY_BODY, "utf8");
    await handle.sync();
    const metadata = await handle.stat();
    await handle.close();
    handle = undefined;
    const [resolvedRoot, resolvedFile, rootMetadata, fileMetadata] =
      await Promise.all([realpath(root), realpath(file), lstat(root), lstat(file)]);
    if (
      resolvedRoot !== root ||
      resolvedFile !== file ||
      !isDirectChild(resolvedWorkspace, root) ||
      !isDirectChild(root, file) ||
      resolvedFile.startsWith(`${workspace.dataDir}${path.sep}`) ||
      !rootMetadata.isDirectory() ||
      rootMetadata.isSymbolicLink() ||
      (rootMetadata.mode & 0o777) !== 0o700 ||
      !fileMetadata.isFile() ||
      fileMetadata.isSymbolicLink() ||
      (fileMetadata.mode & 0o777) !== 0o600 ||
      fileMetadata.nlink !== 1 ||
      fileMetadata.dev !== metadata.dev ||
      fileMetadata.ino !== metadata.ino ||
      fileMetadata.size !== metadata.size ||
      metadata.size !== Buffer.byteLength(CANARY_BODY, "utf8")
    ) {
      fail("synthetic_canary_invalid");
    }
  } catch (error) {
    await handle?.close().catch(() => {});
    if (error?.code === "synthetic_canary_invalid") throw error;
    fail("synthetic_canary_invalid");
  }
  return Object.freeze({ file, root });
}

export function syntheticCanaryImportRequest(canary) {
  if (
    typeof canary?.root !== "string" ||
    !path.isAbsolute(canary.root) ||
    path.basename(canary.file ?? "") !== SYNTHETIC_CANARY_FILE_NAME ||
    path.dirname(canary.file) !== canary.root
  ) {
    fail("synthetic_canary_invalid");
  }
  return Object.freeze({
    roots: Object.freeze([canary.root]),
    profile: "explicit",
    max_files: 1,
  });
}

export function validateCanaryImportResponse(value) {
  if (
    !exactKeys(value, [
      "schema_version",
      "status",
      "accepted_roots",
      "new_tasks",
      "task_ids",
      "scan_profile",
      "scan_file_limit",
    ]) ||
    value.schema_version !== "daemon.import.v1" ||
    value.status !== "accepted" ||
    value.accepted_roots !== 1 ||
    value.new_tasks !== 1 ||
    !Array.isArray(value.task_ids) ||
    value.task_ids.length !== 1 ||
    !IMPORT_TASK_ID.test(value.task_ids[0] ?? "") ||
    value.scan_profile !== "explicit" ||
    value.scan_file_limit !== 1
  ) {
    fail("synthetic_canary_import_invalid");
  }
  return Object.freeze({ taskId: value.task_ids[0] });
}

export function canaryImportCompleted(value, previousEpoch) {
  let status;
  try {
    status = validateReadyStatus(value);
  } catch {
    return false;
  }
  const latest = status.latest_import_scan;
  return (
    Number.isSafeInteger(previousEpoch) &&
    previousEpoch >= 1 &&
    status.visible_epoch > previousEpoch &&
    status.import_tasks_queued === 0 &&
    status.import_tasks_recoverable === 0 &&
    exactKeys(latest, [
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
    ]) &&
    latest.scan_profile === "explicit" &&
    latest.files_discovered === 1 &&
    latest.ignored_entries === 0 &&
    latest.scan_errors === 0 &&
    latest.searchable_documents === 1 &&
    latest.ocr_required_documents === 0 &&
    latest.ocr_jobs_queued === 0 &&
    latest.failed_documents === 0 &&
    latest.deleted_documents === 0 &&
    latest.scan_budget_observed === 1 &&
    latest.scan_budget_limit === 1 &&
    latest.scan_budget_exhausted === false
  );
}

export function validateCanarySearchResponse(value, expectedEpoch) {
  const response = validateSyntheticSearchResponse(value);
  if (
    response.visible_epoch !== expectedEpoch ||
    response.result_count !== 1 ||
    response.results[0]?.file_name !== SYNTHETIC_CANARY_FILE_NAME ||
    response.results[0]?.selection?.visible_epoch !== expectedEpoch
  ) {
    fail("search_witness_invalid");
  }
  return response;
}

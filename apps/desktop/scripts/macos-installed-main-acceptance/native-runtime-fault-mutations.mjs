import { constants } from "node:fs";
import { chmod, copyFile, open } from "node:fs/promises";
import path from "node:path";

import { toolSucceeded } from "./bounded-process.mjs";
import { fail } from "./core.mjs";
import {
  fileSha256,
  permissionFailure,
  requireSafeFile,
  sameIdentity,
} from "./native-fault-file-ops.mjs";

const INVALID_FAULT_BYTES = Buffer.from(
  "resume-ir-installed-acceptance-invalid-runtime-v1\n",
  "utf8",
);
export const INVALID_FAULT_SHA256 =
  "b2a0f43101d251a4214538f0187f4ecd253065c4eb4bbb3dad030b853c2fc25a";

export function runtimeFaultTargets(appBundle, expectedExecutables) {
  return Object.freeze({
    classifierModel: path.join(
      appBundle,
      "Contents",
      "Resources",
      "classifier",
      "runtime-pack",
      "linear-promotion-model.json",
    ),
    embedding: expectedExecutables.embedding_runtime,
    ocrEngine: path.join(
      appBundle,
      "Contents",
      "Resources",
      "ocr",
      "runtime-pack",
      "tesseract",
    ),
    pdfRenderer: expectedExecutables.pdf_renderer,
  });
}

export function targetForMutation(mutation, targets) {
  const exact = {
    classifierModel: { executable: false, file: targets.classifierModel },
    embedding: { executable: true, file: targets.embedding },
    ocrEngine: { executable: true, file: targets.ocrEngine },
    pdfRenderer: { executable: true, file: targets.pdfRenderer },
  }[mutation.target];
  if (!exact) fail("installed_fault_cell_invalid");
  return {
    activation: mutation.activation,
    executable: exact.executable,
    target: exact.file,
  };
}

export async function systemDenyExecution(runTool, username, file) {
  if (!/^[A-Za-z0-9._-]{1,128}$/.test(username)) {
    fail("installed_fault_activation_failed");
  }
  const result = await runTool(
    "/bin/chmod",
    ["+a", `user:${username} deny execute`, file],
    { timeoutMs: 10_000 },
  );
  if (!toolSucceeded(result) || result.stdout !== "" || result.stderr !== "") {
    if (result?.status === 1) fail("installed_fault_permission_denied");
    fail("installed_fault_activation_failed");
  }
}

export async function writeInvalidReplacement(mutation) {
  let handle;
  try {
    handle = await open(
      mutation.target,
      constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | constants.O_NOFOLLOW,
      mutation.identity.mode & 0o777,
    );
    await handle.writeFile(INVALID_FAULT_BYTES);
    await handle.sync();
    await handle.close();
    handle = null;
  } catch (error) {
    await handle?.close().catch(() => {});
    if (permissionFailure(error)) fail("installed_fault_permission_denied");
    fail("installed_fault_activation_failed");
  }
  const replacement = await requireSafeFile(mutation.target, mutation);
  if (
    sameIdentity(replacement, mutation.identity) ||
    (await fileSha256(mutation.target)) !== INVALID_FAULT_SHA256
  ) {
    fail("installed_fault_activation_failed");
  }
}

export async function writeStartFailureReplacement(mutation, denyExecution) {
  try {
    await copyFile(mutation.backup, mutation.target, constants.COPYFILE_EXCL);
    await chmod(mutation.target, mutation.identity.mode & 0o777);
  } catch (error) {
    if (permissionFailure(error)) fail("installed_fault_permission_denied");
    fail("installed_fault_activation_failed");
  }
  await denyExecution(mutation.target);
  const replacement = await requireSafeFile(mutation.target, mutation);
  if (
    sameIdentity(replacement, mutation.identity) ||
    (await fileSha256(mutation.target)) !== mutation.digest
  ) {
    fail("installed_fault_activation_failed");
  }
}

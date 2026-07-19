import {
  acquireLifecycleLock,
  prepareLifecycleLockFile,
  releaseLifecycleLock,
  requireLifecycleLockCapability,
} from "./macos-lifecycle-lock.mjs";
import { prepareOwnerEvidenceDirectory } from "./macos-owner-evidence-store.mjs";

function executionError() {
  return new Error("macOS lifecycle execution is invalid");
}

export async function runWithMacosLifecycleLock({
  applicationSupportRoot,
  resolveApplicationSupportRoot,
  lifecycleLockTestRuntime,
  execute,
}) {
  if (
    typeof resolveApplicationSupportRoot !== "function" ||
    typeof execute !== "function"
  ) {
    throw executionError();
  }
  const resolvedApplicationSupport =
    applicationSupportRoot ?? (await resolveApplicationSupportRoot());
  const lockFile = await prepareLifecycleLockFile({
    applicationSupportRoot: resolvedApplicationSupport,
    prepareEvidenceDirectory: prepareOwnerEvidenceDirectory,
  });
  const capability = await acquireLifecycleLock({
    lockFile,
    testRuntime: lifecycleLockTestRuntime,
  });
  try {
    requireLifecycleLockCapability(capability);
    return await execute({
      applicationSupportRoot: resolvedApplicationSupport,
      lifecycleLockCapability: capability,
    });
  } finally {
    await releaseLifecycleLock(capability);
  }
}

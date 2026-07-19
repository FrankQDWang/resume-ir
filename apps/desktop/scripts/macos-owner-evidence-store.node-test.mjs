import assert from "node:assert/strict";
import { mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { prepareOwnerEvidenceDirectory } from "./macos-owner-evidence-store.mjs";

test("durably publishes the first owner evidence directory", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-owner-evidence-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const applicationSupportRoot = path.join(
    root,
    "Library",
    "Application Support",
  );
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  const evidenceDirectory = path.join(
    applicationSupportRoot,
    "local.resume-ir.desktop",
  );
  const synced = [];

  assert.equal(
    await prepareOwnerEvidenceDirectory(applicationSupportRoot, {
      syncDirectory: async (directory) => synced.push(directory),
    }),
    evidenceDirectory,
  );
  assert.deepEqual(synced, [evidenceDirectory, applicationSupportRoot]);
});

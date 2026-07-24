import assert from "node:assert/strict";
import { chmod, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { AUTH_FILE, ENDPOINT_FILE } from "./core.mjs";
import { readPrivateJson } from "./filesystem-cow.mjs";
import {
  prepareForeignControlFixture,
  prepareStaleControlFixture,
  validateForeignControlPreserved,
  waitForControlReplacement,
} from "./native-control-cells.mjs";

function endpoints(origin, launchId, instanceId) {
  return {
    schema_version: "resume-ir.daemon-ipc.v3",
    launch_id: launchId,
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
}

async function replaceWithV3(dataDir) {
  const launchId = "c".repeat(64);
  const instanceId = "d".repeat(64);
  const token = "e".repeat(64);
  const origin = "http://127.0.0.1:43123";
  await writeFile(
    path.join(dataDir, ENDPOINT_FILE),
    `${JSON.stringify(endpoints(origin, launchId, instanceId))}\n`,
    { mode: 0o600 },
  );
  await writeFile(
    path.join(dataDir, AUTH_FILE),
    `${JSON.stringify({
      schema_version: "resume-ir.daemon-auth.v3",
      launch_id: launchId,
      instance_id: instanceId,
      token,
    })}\n`,
    { mode: 0o600 },
  );
  await Promise.all([
    chmod(path.join(dataDir, ENDPOINT_FILE), 0o600),
    chmod(path.join(dataDir, AUTH_FILE), 0o600),
  ]);
  return launchId;
}

async function privateDirectory(context) {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-control-cell-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

test("stale v2 discovery must be replaced by an owned v3 candidate", async (context) => {
  const dataDir = await privateDirectory(context);
  const fixture = await prepareStaleControlFixture(dataDir);
  assert.equal(
    (await readPrivateJson(path.join(dataDir, ENDPOINT_FILE))).value
      .schema_version,
    "resume-ir.daemon-ipc.v2",
  );
  const launchId = await replaceWithV3(dataDir);
  let ownershipChecks = 0;
  const replacement = await waitForControlReplacement(
    fixture,
    async (candidate) => {
      ownershipChecks += 1;
      return candidate.launchId === launchId;
    },
    undefined,
    1_000,
  );
  assert.equal(replacement.launchId, launchId);
  assert.equal(ownershipChecks, 1);
});

test("foreign v3 listener remains live and unprobed while owned discovery replaces it", async (context) => {
  const dataDir = await privateDirectory(context);
  const fixture = await prepareForeignControlFixture(dataDir);
  context.after(() => fixture.close());
  validateForeignControlPreserved(fixture);

  const launchId = await replaceWithV3(dataDir);
  const replacement = await waitForControlReplacement(
    fixture,
    async (candidate) => candidate.launchId === launchId,
    undefined,
    1_000,
  );

  assert.equal(replacement.launchId, launchId);
  assert.equal(validateForeignControlPreserved(fixture), true);
  await fixture.close();
});

import assert from "node:assert/strict";
import { constants } from "node:fs";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  acquireExistingLock,
  cloneDirectoryTreeCow,
  createCowCloneWorkspace,
  listRecoverableWorkspaces,
  pathsOverlap,
  persistWorkspaceMarker,
  readActiveStoreManifest,
  releaseExistingLock,
  requireApfsDirectory,
  requireSecureDirectory,
  safeRemoveWorkspace,
  validateWorkspaceMarker,
  workspaceMarker,
} from "./filesystem-cow.mjs";
import { WORKSPACE_PREFIX } from "./core.mjs";
import { COMPOSITION, DMG, HEAD } from "./fixtures.mjs";

const darwinTest = process.platform === "darwin" ? test : test.skip;

test("v1 launch markers require the exact authority-anchor state shape", () => {
  const runId = "a".repeat(64);
  const authority = "b".repeat(64);
  const startTime = "Mon Jul 20 12:34:56 2026";
  const guardian = {
    pid: 7_001,
    pgid: 7_001,
    start_time: startTime,
    executable: process.execPath,
    session_authority: authority,
  };
  const running = workspaceMarker(runId, {
    state: "app_running",
    session_authority: authority,
    guardian,
    authority_anchor: {
      pid: 7_002,
      pgid: guardian.pgid,
      start_time: startTime,
      executable: process.execPath,
      session_authority: authority,
    },
    application: {
      pid: 7_003,
      pgid: guardian.pgid,
      start_time: startTime,
      executable: "/Applications/resume-ir.app/Contents/MacOS/resume-desktop",
      session_authority: authority,
    },
  });
  assert.equal(validateWorkspaceMarker(running).state, "app_running");
  const legacy = { ...running };
  delete legacy.authority_anchor;
  assert.throws(() => validateWorkspaceMarker(legacy), /workspace_marker_invalid/);
  assert.throws(
    () => validateWorkspaceMarker({ ...running, unknown: true }),
    /workspace_marker_invalid/,
  );
  assert.throws(
    () =>
      validateWorkspaceMarker({
        ...running,
        state: "launch_pending",
        application: null,
      }),
    /workspace_marker_invalid/,
  );
});

test("stale discovery and removal require an exact owner marker and run id", async (context) => {
  const temporaryParent = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-marker-cleanup-")),
  );
  context.after(() => rm(temporaryParent, { recursive: true, force: true }));
  await chmod(temporaryParent, 0o700);
  const currentRunId = "1".repeat(64);
  const staleRunId = "2".repeat(64);
  const current = path.join(temporaryParent, `${WORKSPACE_PREFIX}current`);
  const stale = path.join(temporaryParent, `${WORKSPACE_PREFIX}stale`);
  const unowned = path.join(temporaryParent, `${WORKSPACE_PREFIX}unowned`);
  for (const directory of [current, stale, unowned]) {
    await mkdir(directory, { mode: 0o700 });
    await chmod(directory, 0o700);
  }
  await persistWorkspaceMarker(current, workspaceMarker(currentRunId));
  await persistWorkspaceMarker(stale, workspaceMarker(staleRunId));
  await writeFile(path.join(unowned, "unrelated"), "synthetic", {
    mode: 0o600,
  });

  const discovered = await listRecoverableWorkspaces(
    temporaryParent,
    currentRunId,
  );
  assert.deepEqual(
    discovered.map(({ root, runId }) => [root, runId]),
    [[stale, staleRunId]],
  );
  await assert.rejects(
    safeRemoveWorkspace(stale, temporaryParent, currentRunId),
    /cleanup_failed/,
  );
  assert.equal((await stat(stale)).isDirectory(), true);
  await safeRemoveWorkspace(stale, temporaryParent, staleRunId);
  await assert.rejects(stat(stale), /ENOENT/);
  assert.equal((await stat(current)).isDirectory(), true);
  assert.equal((await stat(unowned)).isDirectory(), true);
});

test("cleanup quarantines by verified inode and never deletes a pathname replacement", async (context) => {
  const temporaryParent = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-quarantine-cleanup-")),
  );
  context.after(() => rm(temporaryParent, { recursive: true, force: true }));
  await chmod(temporaryParent, 0o700);
  const runId = "7".repeat(64);
  const root = path.join(temporaryParent, `${WORKSPACE_PREFIX}victim`);
  const originalMoved = path.join(temporaryParent, "verified-original-moved");
  const attacker = path.join(temporaryParent, "attacker-replacement");
  await Promise.all([
    mkdir(root, { mode: 0o700 }),
    mkdir(attacker, { mode: 0o700 }),
  ]);
  await Promise.all([chmod(root, 0o700), chmod(attacker, 0o700)]);
  await persistWorkspaceMarker(root, workspaceMarker(runId));
  await writeFile(path.join(attacker, "must-survive"), "synthetic", {
    mode: 0o600,
  });

  await assert.rejects(
    safeRemoveWorkspace(root, temporaryParent, runId, {
      afterVerifiedRoot: async () => {
        await rename(root, originalMoved);
        await rename(attacker, root);
      },
    }),
    /cleanup_failed/,
  );
  const candidates = await readdir(temporaryParent);
  const survivorBodies = await Promise.all(
    candidates.map((name) =>
      readFile(path.join(temporaryParent, name, "must-survive"), "utf8").catch(
        () => null,
      ),
    ),
  );
  assert.equal(survivorBodies.includes("synthetic"), true);
  assert.equal((await stat(originalMoved)).isDirectory(), true);
});

test("rejects either source/temporary ancestor direction before locking or writing", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-overlap-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const source = path.join(root, "source");
  const temporaryInsideSource = path.join(source, "temporary");
  await mkdir(temporaryInsideSource, { recursive: true, mode: 0o700 });
  await chmod(source, 0o700);
  await chmod(temporaryInsideSource, 0o700);
  assert.equal(pathsOverlap(source, temporaryInsideSource), true);
  assert.equal(pathsOverlap(root, source), true);
  assert.equal(pathsOverlap(source, path.join(root, "sibling")), false);
  const before = (await readdir(source, { recursive: true })).sort();
  const rootBefore = (await readdir(root, { recursive: true })).sort();
  let lockAttempted = false;
  for (const temporaryParent of [temporaryInsideSource, root]) {
    await assert.rejects(
      createCowCloneWorkspace({
        authorizedSourceDataDir: source,
        temporaryParent,
        expectedComposition: {},
        acquireLock: async () => {
          lockAttempted = true;
        },
        requireApfs: async () => {},
        runTool: async () => {
          throw new Error("clone must not run");
        },
      }),
      /apfs_clone_unavailable/,
    );
  }
  assert.equal(lockAttempted, false);
  assert.deepEqual((await readdir(source, { recursive: true })).sort(), before);
  assert.deepEqual((await readdir(root, { recursive: true })).sort(), rootBefore);
});

test("creates the test HOME with forced per-file COW while leaving the source unchanged", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-cow-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const source = path.join(root, "authorized-source");
  const temporaryParent = path.join(root, "temporary");
  await Promise.all([
    mkdir(source, { mode: 0o755 }),
    mkdir(temporaryParent, { mode: 0o700 }),
  ]);
  await chmod(source, 0o755);
  const manifestBody = [
    "resume-ir.metadata-active.v1",
    "file=metadata-v28-1111111111111111.sqlite3",
    "schema=28",
    `digest=${"1".repeat(64)}`,
    "",
  ].join("\n");
  await writeFile(path.join(source, "metadata-active.v1"), manifestBody, {
    mode: 0o644,
  });
  await writeFile(path.join(source, "data-directory-owner.lock"), "", {
    mode: 0o600,
  });
  await writeFile(path.join(source, "search-publication.lock"), "", {
    mode: 0o600,
  });
  await writeFile(path.join(source, "index-publication.lock"), "", {
    mode: 0o644,
  });
  await writeFile(
    path.join(source, "resume-ir.install-receipt.v2.json"),
    `${JSON.stringify({
      schema_version: "resume-ir.macos-install-receipt.v2",
      bundle_id: "local.resume-ir.desktop",
      version: "0.1.2",
      target_triple: "aarch64-apple-darwin",
      source_commit: HEAD,
      composition_digest: COMPOSITION,
      dmg_sha256: DMG,
    })}\n`,
    { mode: 0o600 },
  );
  const calls = [];
  const runTool = async (command, args, toolOptions) => {
    calls.push([command, [...args], { ...toolOptions }]);
    assert.equal(command, process.execPath);
    assert.equal(args[1], "--internal-cow-clone");
    await cloneDirectoryTreeCow(args[2], args[3], {
      cloneFile: async (sourceFile, destinationFile, mode) => {
        assert.equal(mode, constants.COPYFILE_FICLONE_FORCE);
        await copyFile(sourceFile, destinationFile);
      },
    });
    return {
      status: 0,
      stdout: "resume-ir.apfs-cow-clone.complete.v1\n",
      stderr: "",
      timedOut: false,
      overflow: false,
    };
  };
  const lockCalls = [];
  const manifestReads = [];
  const workspace = await createCowCloneWorkspace({
    authorizedSourceDataDir: source,
    temporaryParent,
    expectedComposition: {
      bundle_id: "local.resume-ir.desktop",
      version: "0.1.2",
      target_triple: "aarch64-apple-darwin",
      source_commit: HEAD,
      composition_digest: COMPOSITION,
    },
    runTool,
    requireApfs: async () => {},
    readManifest: async (directory, readOptions) => {
      manifestReads.push(directory);
      return readActiveStoreManifest(directory, readOptions);
    },
    acquireLock: async (file) => {
      lockCalls.push(["acquire", file]);
      return { file };
    },
    releaseLock: async ({ file }) => lockCalls.push(["release", file]),
  });

  assert.equal(workspace.sourceSchema, 28);
  assert.equal(
    await readFile(path.join(workspace.dataDir, "metadata-active.v1"), "utf8"),
    manifestBody,
  );
  assert.equal(
    await readFile(path.join(source, "metadata-active.v1"), "utf8"),
    manifestBody,
  );
  assert.equal((await stat(workspace.dataDir)).mode & 0o777, 0o755);
  assert.equal(
    (await stat(path.join(workspace.dataDir, "metadata-active.v1"))).mode &
      0o777,
    0o644,
  );
  assert.equal(
    (await stat(path.join(workspace.dataDir, "index-publication.lock"))).mode &
      0o777,
    0o644,
  );
  assert.equal(
    calls.some(
      ([command, args]) =>
        command === process.execPath &&
        args[1] === "--internal-cow-clone" &&
        args[2] === source &&
        args[3] === workspace.dataDir,
    ),
    true,
  );
  assert.deepEqual(
    lockCalls.map(([operation]) => operation),
    ["acquire", "release"],
  );
  assert.deepEqual(manifestReads, [source, workspace.dataDir, source]);
});

test("requires the exact APFS statfs type", async () => {
  const calls = [];
  await requireApfsDirectory("/synthetic/apfs", {
    statfsTool: async (...args) => {
      calls.push(args);
      return { type: 0x1an };
    },
  });
  assert.deepEqual(calls, [["/synthetic/apfs", { bigint: true }]]);
  await assert.rejects(
    requireApfsDirectory("/synthetic/not-apfs", {
      statfsTool: async () => ({ type: 0x11an }),
    }),
    /apfs_clone_unavailable/,
  );
});

darwinTest(
  "recognizes the native temporary volume through Node statfs",
  async () => {
    const root = await realpath(os.tmpdir());
    await requireApfsDirectory(root);
  },
);

darwinTest(
  "rejects an explicit non-canonical temporary-directory alias",
  async () => {
    const alias = os.tmpdir();
    const canonical = await realpath(alias);
    if (alias !== canonical) {
      await assert.rejects(requireSecureDirectory(alias), /directory_invalid/);
    }
  },
);

test("fails closed when any per-file forced COW clone is unavailable", async (context) => {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-cow-force-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  await chmod(root, 0o700);
  const source = path.join(root, "source");
  const destinationParent = path.join(root, "destination");
  await Promise.all([
    mkdir(source, { mode: 0o700 }),
    mkdir(destinationParent, { mode: 0o700 }),
  ]);
  await writeFile(path.join(source, "one"), "one", { mode: 0o600 });
  await writeFile(path.join(source, "two"), "two", { mode: 0o600 });
  const modes = [];
  let calls = 0;
  await assert.rejects(
    cloneDirectoryTreeCow(source, path.join(destinationParent, "clone"), {
      cloneFile: async (sourceFile, destinationFile, mode) => {
        modes.push(mode);
        calls += 1;
        if (calls === 2) throw new Error("synthetic fallback would occur");
        await copyFile(sourceFile, destinationFile);
      },
    }),
    /apfs_clone_failed/,
  );
  assert.deepEqual(
    modes,
    Array(modes.length).fill(constants.COPYFILE_FICLONE_FORCE),
  );
});

darwinTest(
  "uses the no-fallback clonefile mode on a synthetic APFS fixture",
  async (context) => {
    const root = await realpath(
      await mkdtemp(path.join(os.tmpdir(), "resume-ir-cow-native-test-")),
    );
    context.after(() => rm(root, { recursive: true, force: true }));
    await chmod(root, 0o700);
    const source = path.join(root, "source");
    const destinationParent = path.join(root, "destination");
    await Promise.all([
      mkdir(source, { mode: 0o700 }),
      mkdir(destinationParent, { mode: 0o700 }),
    ]);
    await writeFile(path.join(source, "payload"), "synthetic-only", {
      mode: 0o600,
    });
    const destination = path.join(destinationParent, "clone");
    assert.deepEqual(await cloneDirectoryTreeCow(source, destination), {
      clonedFiles: 1,
    });
    assert.equal(
      await readFile(path.join(destination, "payload"), "utf8"),
      "synthetic-only",
    );
  },
);

darwinTest(
  "acquires and releases the real BSD flock holder",
  async (context) => {
    const root = await realpath(
      await mkdtemp(path.join(os.tmpdir(), "resume-ir-flock-holder-test-")),
    );
    context.after(() => rm(root, { recursive: true, force: true }));
    await chmod(root, 0o700);
    const lock = path.join(root, "snapshot-publication.lock");
    await writeFile(lock, "", { mode: 0o600 });

    const capability = await acquireExistingLock(lock);
    assert.equal(capability.released, false);
    await releaseExistingLock(capability);
    assert.equal(capability.released, true);
  },
);

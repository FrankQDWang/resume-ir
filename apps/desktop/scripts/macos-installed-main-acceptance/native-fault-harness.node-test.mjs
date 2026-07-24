import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  symlink,
  unlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { AcceptanceError } from "./core.mjs";
import { createNativeAcceptanceCells } from "./native-acceptance-cells.mjs";
import {
  REQUIRED_FAULT_CELLS,
  SLOW_RUNTIME_STOP_MS,
  createNativeFaultHarnessForTesting,
} from "./native-fault-harness.mjs";
import {
  INSTALLED_FAULT_RECOVERY_AUTHORITY_SCHEMA,
} from "./native-fault-recovery-authority.mjs";
import { backupPath } from "./native-fault-file-ops.mjs";
import {
  projectedRuntimeFaultEvidence,
  runtimeFaultCase,
} from "./native-runtime-fault-plan.mjs";

const ATOMIC_RENAME_EXCLUSIVE = fileURLToPath(
  new URL("./atomic-rename-exclusive.rb", import.meta.url),
);

async function installedFixture(t) {
  const created = await mkdtemp(
    path.join(os.tmpdir(), "resume-ir-fault-harness-"),
  );
  const root = await realpath(created);
  t.after(() => rm(root, { force: true, recursive: true }));
  const appBundle = path.join(root, "resume-ir.app");
  const macos = path.join(appBundle, "Contents", "MacOS");
  const classifier = path.join(
    appBundle,
    "Contents",
    "Resources",
    "classifier",
    "runtime-pack",
  );
  const ocr = path.join(
    appBundle,
    "Contents",
    "Resources",
    "ocr",
    "runtime-pack",
  );
  await mkdir(macos, { recursive: true, mode: 0o755 });
  await mkdir(classifier, { recursive: true, mode: 0o755 });
  await mkdir(ocr, { recursive: true, mode: 0o755 });
  const executablePaths = {
    desktop: path.join(macos, "resume-desktop"),
    daemon: path.join(macos, "resume-daemon"),
    embedding_runtime: path.join(macos, "resume-embedding-runtime"),
    pdf_renderer: path.join(macos, "resume-pdf-render-runtime"),
  };
  for (const [role, file] of Object.entries(executablePaths)) {
    await writeFile(file, `exact-${role}\n`, { mode: 0o755 });
  }
  const classifierModel = path.join(classifier, "linear-promotion-model.json");
  await writeFile(classifierModel, '{"model":"exact"}\n', { mode: 0o644 });
  const ocrEngine = path.join(ocr, "tesseract");
  await writeFile(ocrEngine, "exact-ocr-engine\n", { mode: 0o755 });
  return { appBundle, classifierModel, executablePaths, macos, ocrEngine };
}

async function acceptanceBackups(parent) {
  return (await readdir(parent)).filter((entry) =>
    entry.endsWith(".resume-ir-installed-acceptance-backup-v1"),
  );
}

function declaration(fixture, cell) {
  return {
    cell,
    dataDir: path.join(fixture.appBundle, "acceptance-data"),
    executablePaths: fixture.executablePaths,
  };
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function fixtureRecoveryAuthority(fixture) {
  return Object.freeze({
    schema: INSTALLED_FAULT_RECOVERY_AUTHORITY_SCHEMA,
    compositionDigest: "a".repeat(64),
    targets: Object.freeze([
      Object.freeze({
        file: "Contents/MacOS/resume-embedding-runtime",
        digest: "sha256_without_code_signature_v1",
        sha256: sha256(await readFile(fixture.executablePaths.embedding_runtime)),
      }),
      Object.freeze({
        file: "Contents/MacOS/resume-pdf-render-runtime",
        digest: "sha256_without_code_signature_v1",
        sha256: sha256(await readFile(fixture.executablePaths.pdf_renderer)),
      }),
      Object.freeze({
        file: "Contents/Resources/ocr/runtime-pack/tesseract",
        digest: "sha256",
        sha256: sha256(await readFile(fixture.ocrEngine)),
      }),
      Object.freeze({
        file:
          "Contents/Resources/classifier/runtime-pack/linear-promotion-model.json",
        digest: "sha256",
        sha256: sha256(await readFile(fixture.classifierModel)),
      }),
    ]),
  });
}

test("prepare is declarative and each missing cell atomically moves then restores only its exact target", async (t) => {
  const fixture = await installedFixture(t);
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  const cases = [
    ["embedding_missing", fixture.executablePaths.embedding_runtime],
    ["ocr_missing", fixture.executablePaths.pdf_renderer],
    ["classifier_missing", fixture.classifierModel],
  ];

  assert.deepEqual(harness.supportedCells, REQUIRED_FAULT_CELLS);
  for (const [cell, target] of cases) {
    const original = await readFile(target);
    const originalIdentity = await lstat(target);
    const handle = await harness.prepare(declaration(fixture, cell));
    assert.deepEqual(await readFile(target), original, `${cell} prepare mutated`);
    assert.deepEqual(await acceptanceBackups(path.dirname(target)), []);

    await harness.activate(handle);
    await assert.rejects(lstat(target), { code: "ENOENT" });
    const backups = await acceptanceBackups(path.dirname(target));
    assert.equal(backups.length, 1);
    const backupIdentity = await lstat(path.join(path.dirname(target), backups[0]));
    assert.equal(backupIdentity.ino, originalIdentity.ino);

    await harness.restore(handle, { requireCompleted: true });
    assert.deepEqual(await readFile(target), original);
    assert.equal((await lstat(target)).ino, originalIdentity.ino);
    assert.deepEqual(await acceptanceBackups(path.dirname(target)), []);
    await harness.restore(handle, { requireCompleted: true });
  }
});

test("invalid cells install a deterministic replacement and restore exact original bytes", async (t) => {
  const fixture = await installedFixture(t);
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  const cases = [
    ["embedding_invalid", fixture.executablePaths.embedding_runtime],
    ["ocr_invalid", fixture.executablePaths.pdf_renderer],
    ["classifier_invalid", fixture.classifierModel],
  ];

  for (const [cell, target] of cases) {
    const original = await readFile(target);
    const originalIdentity = await lstat(target);
    const handle = await harness.prepare(declaration(fixture, cell));
    await harness.activate(handle);
    assert.notDeepEqual(await readFile(target), original);
    assert.notEqual((await lstat(target)).ino, originalIdentity.ino);
    assert.equal((await acceptanceBackups(path.dirname(target))).length, 1);

    await harness.restore(handle, { requireCompleted: true });
    assert.deepEqual(await readFile(target), original);
    assert.equal((await lstat(target)).ino, originalIdentity.ino);
    assert.deepEqual(await acceptanceBackups(path.dirname(target)), []);
  }
});

test("embedding and OCR start-failed cells preserve attested bytes but deny execution", async (t) => {
  const fixture = await installedFixture(t);
  const denied = [];
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
    denyExecution: async (file) => denied.push(file),
  });
  const cases = [
    ["embedding_start_failed", fixture.executablePaths.embedding_runtime],
    ["ocr_start_failed", fixture.ocrEngine],
  ];

  for (const [cell, target] of cases) {
    const original = await readFile(target);
    const originalIdentity = await lstat(target);
    const handle = await harness.prepare(declaration(fixture, cell));
    await harness.activate(handle);
    assert.deepEqual(await readFile(target), original);
    assert.notEqual((await lstat(target)).ino, originalIdentity.ino);
    assert.equal(denied.at(-1), target);

    await harness.restore(handle, { requireCompleted: true });
    assert.deepEqual(await readFile(target), original);
    assert.equal((await lstat(target)).ino, originalIdentity.ino);
  }
});

test("representative combined cells mutate and restore every declared target", async (t) => {
  const fixture = await installedFixture(t);
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  const originals = new Map(
    await Promise.all(
      [
        fixture.executablePaths.embedding_runtime,
        fixture.executablePaths.pdf_renderer,
        fixture.classifierModel,
      ].map(async (file) => [file, await readFile(file)]),
    ),
  );
  const handle = await harness.prepare(
    declaration(fixture, "all_runtimes_missing"),
  );
  await harness.activate(handle);
  for (const file of originals.keys()) {
    await assert.rejects(lstat(file), { code: "ENOENT" });
  }
  await harness.restore(handle, { requireCompleted: true });
  for (const [file, bytes] of originals) {
    assert.deepEqual(await readFile(file), bytes);
  }
});

test("classifier start failure is explicitly projected because no native post-attestation boundary exists", async (t) => {
  const fixture = await installedFixture(t);
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  const before = await readFile(fixture.classifierModel);
  const handle = await harness.prepare(
    declaration(fixture, "classifier_start_failed"),
  );
  await harness.activate(handle);
  assert.deepEqual(await readFile(fixture.classifierModel), before);
  await harness.restore(handle, { requireCompleted: true });

  assert.equal(
    runtimeFaultCase("classifier_start_failed").evidenceSource,
    "deterministic_contract_projection",
  );
  assert.deepEqual(projectedRuntimeFaultEvidence("classifier_start_failed"), {
    cell: "classifier_start_failed",
    evidenceSource: "deterministic_contract_projection",
    expectedReasons: { classifier: "start_failed" },
    nativeMutationApplied: false,
    projectionReason: "post_attestation_failure_surface_absent",
  });
});

test(
  "the macOS rename helper is atomic and refuses to overwrite its destination",
  { skip: process.platform !== "darwin" },
  async (t) => {
    const created = await mkdtemp(
      path.join(os.tmpdir(), "resume-ir-exclusive-rename-"),
    );
    const root = await realpath(created);
    t.after(() => rm(root, { force: true, recursive: true }));
    const source = path.join(root, "source");
    const destination = path.join(root, "destination");
    await writeFile(source, "first\n");
    assert.equal(
      spawnSync("/usr/bin/ruby", [
        ATOMIC_RENAME_EXCLUSIVE,
        source,
        destination,
      ]).status,
      0,
    );
    assert.equal(await readFile(destination, "utf8"), "first\n");

    await writeFile(source, "second\n");
    assert.equal(
      spawnSync("/usr/bin/ruby", [
        ATOMIC_RENAME_EXCLUSIVE,
        source,
        destination,
      ]).status,
      74,
    );
    assert.equal(await readFile(source, "utf8"), "second\n");
    assert.equal(await readFile(destination, "utf8"), "first\n");
  },
);

test("rejects path substitution and unsafe targets without creating a backup", async (t) => {
  const fixture = await installedFixture(t);
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  await assert.rejects(
    harness.prepare({
      cell: "embedding_missing",
      dataDir: path.join(fixture.appBundle, "acceptance-data"),
      executablePaths: {
        ...fixture.executablePaths,
        embedding_runtime: path.join(fixture.macos, "lookalike"),
      },
    }),
    /installed_fault_target_invalid/,
  );

  const outside = path.join(path.dirname(fixture.appBundle), "outside-runtime");
  await writeFile(outside, "outside\n", { mode: 0o755 });
  await unlink(fixture.executablePaths.embedding_runtime);
  await symlink(outside, fixture.executablePaths.embedding_runtime);
  const handle = await harness.prepare(
    declaration(fixture, "embedding_missing"),
  );
  await assert.rejects(harness.activate(handle), (error) => {
    return (
      error instanceof AcceptanceError &&
      error.code === "installed_fault_target_unsafe"
    );
  });
  assert.deepEqual(await acceptanceBackups(fixture.macos), []);
});

test("reports insufficient installed-App ownership as a typed permission failure", async (t) => {
  const fixture = await installedFixture(t);
  const observedUid = (await lstat(fixture.executablePaths.embedding_runtime)).uid;
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
    uid: observedUid + 1,
  });
  const handle = await harness.prepare(
    declaration(fixture, "embedding_missing"),
  );
  await assert.rejects(
    harness.activate(handle),
    /installed_fault_permission_denied/,
  );
  assert.deepEqual(await acceptanceBackups(fixture.macos), []);
});

test("a deterministic backup is crash-recoverable and restore remains idempotent", async (t) => {
  const fixture = await installedFixture(t);
  const recoveryAuthority = await fixtureRecoveryAuthority(fixture);
  const first = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  const original = await readFile(fixture.executablePaths.pdf_renderer);
  const handle = await first.prepare(declaration(fixture, "ocr_missing"));
  await first.activate(handle);
  assert.equal((await acceptanceBackups(fixture.macos)).length, 1);

  const recovered = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
    loadRecoveryAuthority: async () => recoveryAuthority,
  });
  await recovered.recover();
  assert.deepEqual(await readFile(fixture.executablePaths.pdf_renderer), original);
  assert.deepEqual(await acceptanceBackups(fixture.macos), []);
  await recovered.recover();
  await first.restore(handle);
  await first.restore(handle);
});

test("crash recovery fails closed when the deterministic backup bytes changed", async (t) => {
  const fixture = await installedFixture(t);
  const recoveryAuthority = await fixtureRecoveryAuthority(fixture);
  const first = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  const handle = await first.prepare(
    declaration(fixture, "embedding_missing"),
  );
  await first.activate(handle);
  const [backup] = await acceptanceBackups(fixture.macos);
  await writeFile(path.join(fixture.macos, backup), "tampered-backup\n", {
    mode: 0o755,
  });

  const recovered = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
    loadRecoveryAuthority: async () => recoveryAuthority,
  });
  await assert.rejects(recovered.recover(), /installed_fault_backup_changed/);
  await assert.rejects(lstat(fixture.executablePaths.embedding_runtime), {
    code: "ENOENT",
  });
  assert.equal((await acceptanceBackups(fixture.macos)).length, 1);
});

test("crash recovery rejects arbitrary bytes whose backup name self-reports their hash", async (t) => {
  const fixture = await installedFixture(t);
  const recoveryAuthority = await fixtureRecoveryAuthority(fixture);
  const target = fixture.executablePaths.embedding_runtime;
  await unlink(target);
  const forged = Buffer.from("forged-runtime\n");
  const forgedBackup = backupPath(target, sha256(forged));
  await writeFile(forgedBackup, forged, { mode: 0o755 });

  const recovered = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
    loadRecoveryAuthority: async () => recoveryAuthority,
  });
  await assert.rejects(recovered.recover(), /installed_fault_backup_untrusted/);
  await assert.rejects(lstat(target), { code: "ENOENT" });
  assert.deepEqual(await readFile(forgedBackup), forged);
});

test("crash recovery requires the owner receipt bound composition authority", async (t) => {
  const fixture = await installedFixture(t);
  const first = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  const handle = await first.prepare(declaration(fixture, "classifier_missing"));
  await first.activate(handle);

  const recovered = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  await assert.rejects(
    recovered.recover(),
    /installed_fault_recovery_authority_invalid/,
  );
  await assert.rejects(lstat(fixture.classifierModel), { code: "ENOENT" });
});

function exactRuntimeProcesses(executablePaths, dataDir, pid = 8_101) {
  return [
    {
      pid: 7_001,
      ppid: 6_001,
      pgid: 7_001,
      command:
        `${executablePaths.desktop} ` +
        "--resume-ir-acceptance-session-authority=" +
        "a".repeat(64),
    },
    {
      pid: 8_001,
      ppid: 7_001,
      pgid: 7_001,
      command:
        `${executablePaths.daemon} --data-dir ${dataDir} ` +
        "run --foreground",
    },
    {
      pid,
      ppid: 8_001,
      pgid: 7_001,
      command: executablePaths.embedding_runtime,
    },
  ];
}

test("slow monitor verifies the exact child identity, stops for eleven seconds, and continues it", async (t) => {
  const fixture = await installedFixture(t);
  const signals = [];
  const delays = [];
  const dataDir = path.join(fixture.appBundle, "acceptance-data");
  const processes = exactRuntimeProcesses(fixture.executablePaths, dataDir);
  let inspections = 0;
  let stopped;
  const stoppedPromise = new Promise((resolve) => {
    stopped = resolve;
  });
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
    delay: async (milliseconds) => {
      delays.push(milliseconds);
    },
    killProcess: (pid, signal) => {
      signals.push([pid, signal]);
      if (signal === "SIGSTOP") stopped();
    },
    listProcesses: async () => {
      inspections += 1;
      return inspections === 1 ? [] : processes;
    },
    readStartTime: async () => "Mon Jan  1 00:00:00 2024",
  });
  const handle = await harness.prepare(
    declaration(fixture, "slow_initialization"),
  );
  await harness.activate(handle);
  await stoppedPromise;
  await harness.restore(handle, { requireCompleted: true });
  assert.ok(delays.includes(SLOW_RUNTIME_STOP_MS));
  assert.deepEqual(signals, [
    [8_101, "SIGSTOP"],
    [8_101, "SIGCONT"],
  ]);
});

test("activation fails closed instead of stopping a pre-existing installed runtime", async (t) => {
  const fixture = await installedFixture(t);
  const dataDir = path.join(fixture.appBundle, "acceptance-data");
  const signals = [];
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
    killProcess: (pid, signal) => signals.push([pid, signal]),
    listProcesses: async () =>
      exactRuntimeProcesses(fixture.executablePaths, dataDir),
  });
  const handle = await harness.prepare(
    declaration(fixture, "slow_initialization"),
  );
  await assert.rejects(
    harness.activate(handle),
    /installed_runtime_already_running/,
  );
  assert.deepEqual(signals, []);
});

test("slow monitor times out when this launch never starts the exact embedding process", async (t) => {
  const fixture = await installedFixture(t);
  let clock = 0;
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
    delay: async () => {
      clock += 2;
    },
    listProcesses: async () => [],
    monitorTimeoutMs: 3,
    now: () => clock,
  });
  const handle = await harness.prepare(
    declaration(fixture, "slow_initialization"),
  );
  await harness.activate(handle);
  await new Promise((resolve) => setImmediate(resolve));
  await assert.rejects(
    harness.restore(handle, { requireCompleted: true }),
    /slow_initialization_monitor_timeout/,
  );
});

test("cleanup sends SIGCONT before cancelling an in-flight slow monitor", async (t) => {
  const fixture = await installedFixture(t);
  const signals = [];
  const dataDir = path.join(fixture.appBundle, "acceptance-data");
  let inspections = 0;
  let stopped;
  const stoppedPromise = new Promise((resolve) => {
    stopped = resolve;
  });
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
    delay: () => new Promise(() => {}),
    killProcess: (pid, signal) => {
      signals.push([pid, signal]);
      if (signal === "SIGSTOP") stopped();
    },
    listProcesses: async () => {
      inspections += 1;
      return inspections === 1
        ? []
        : exactRuntimeProcesses(fixture.executablePaths, dataDir);
    },
  });
  const handle = await harness.prepare(
    declaration(fixture, "slow_initialization"),
  );
  await harness.activate(handle);
  await stoppedPromise;
  await harness.restore(handle);
  assert.deepEqual(signals, [
    [8_101, "SIGSTOP"],
    [8_101, "SIGCONT"],
  ]);
});

test("a prepared but never activated cell cannot claim completion", async (t) => {
  const fixture = await installedFixture(t);
  const harness = createNativeFaultHarnessForTesting({
    appBundle: fixture.appBundle,
  });
  const handle = await harness.prepare(
    declaration(fixture, "classifier_missing"),
  );
  await assert.rejects(
    harness.restore(handle, { requireCompleted: true }),
    /installed_fault_not_activated/,
  );
});

test("native-cell cleanup reports a restore failure instead of hiding it", async () => {
  const executablePaths = {
    desktop: "/synthetic/resume-desktop",
    daemon: "/synthetic/resume-daemon",
    embedding_runtime: "/synthetic/resume-embedding-runtime",
    pdf_renderer: "/synthetic/resume-pdf-render-runtime",
  };
  const harness = Object.freeze({
    supportedCells: REQUIRED_FAULT_CELLS,
    async activate() {},
    async prepare() {
      return Object.freeze({});
    },
    async recover() {},
    async restore() {
      throw new Error("synthetic restore failure");
    },
  });
  const cells = createNativeAcceptanceCells({
    faultHarness: harness,
    getBindings: () => ({ executablePaths }),
    now: Date.now,
    options: { authorizedSourceDataDir: "/synthetic/source" },
    requireMutationAuthority: async () => {},
    runTool: async () => {},
  });
  await cells.prepareFaultCell(
    { dataDir: "/synthetic/workspace" },
    "embedding_missing",
  );
  await assert.rejects(cells.cleanup(), /cleanup_failed/);
});

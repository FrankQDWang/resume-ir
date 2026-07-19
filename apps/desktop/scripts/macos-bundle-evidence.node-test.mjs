import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  BUNDLE_COMPOSITION_FILE,
  createBundleComposition,
  verifyBundleComposition,
  writeBundleComposition,
} from "./macos-bundle-composition.mjs";
import {
  createInstallReceipt,
  installReceiptPath,
  persistInstallReceipt,
  readInstallReceipt,
  removeInstallReceipt,
} from "./macos-install-receipt.mjs";

const TARGET = "aarch64-apple-darwin";
const VERSION = "0.1.1";

function syntheticMachO(payload) {
  const suffix = Buffer.from(payload);
  const header = Buffer.alloc(32);
  header.writeUInt32LE(0xfeedfacf, 0);
  header.writeUInt32LE(0x0100000c, 4);
  return Buffer.concat([header, suffix]);
}

function sha256(body) {
  return createHash("sha256").update(body).digest("hex");
}

async function writeExecutable(file, body) {
  await writeFile(file, body);
  await chmod(file, 0o755);
}

async function bundleFixture(context) {
  const temporary = await mkdtemp(path.join(os.tmpdir(), "resume-ir-bundle-evidence-"));
  const root = await realpath(temporary);
  context.after(() => rm(root, { recursive: true, force: true }));
  const appBundle = path.join(root, "resume-ir.app");
  const contents = path.join(appBundle, "Contents");
  const macos = path.join(contents, "MacOS");
  const resources = path.join(contents, "Resources");
  await mkdir(macos, { recursive: true });
  for (const directory of [
    "classifier/runtime-pack",
    "embedding/runtime-pack",
    "ocr/runtime-pack",
  ]) {
    await mkdir(path.join(resources, directory), { recursive: true });
  }
  await writeFile(
    path.join(contents, "Info.plist"),
    [
      '<?xml version="1.0" encoding="UTF-8"?>',
      '<plist version="1.0"><dict>',
      "<key>CFBundleIdentifier</key><string>local.resume-ir.desktop</string>",
      `<key>CFBundleShortVersionString</key><string>${VERSION}</string>`,
      "<key>CFBundleDisplayName</key><string>resume-ir</string>",
      "<key>CFBundleIconFile</key><string>icon.icns</string>",
      "<key>CFBundleExecutable</key><string>resume-desktop</string>",
      "</dict></plist>",
    ].join(""),
  );
  for (const executable of [
    "resume-desktop",
    "resume-daemon",
    "resume-embedding-runtime",
    "resume-pdf-render-runtime",
  ]) {
    await writeExecutable(
      path.join(macos, executable),
      syntheticMachO(`synthetic-${executable}`),
    );
  }
  for (const pack of ["classifier", "embedding", "ocr"]) {
    const packDirectory = path.join(resources, pack, "runtime-pack");
    const payload = Buffer.from(`synthetic-${pack}-payload`);
    await writeFile(path.join(packDirectory, "payload.bin"), payload);
    await writeFile(
      path.join(packDirectory, "runtime-pack.json"),
      `${JSON.stringify({
        schema_version: `synthetic.${pack}.v1`,
        files: [
          {
            role: "payload",
            file: "payload.bin",
            bytes: payload.length,
            sha256: sha256(payload),
          },
        ],
      })}\n`,
    );
  }
  await writeFile(path.join(resources, "icon.icns"), "synthetic-approved-icon");
  return { appBundle, resources, root };
}

test("writes and verifies one canonical version-bound bundle composition", async (context) => {
  const fixture = await bundleFixture(context);
  const expected = await createBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
  });
  await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
  });
  const verified = await verifyBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    expectedVersion: VERSION,
  });

  assert.deepEqual(verified, expected);
  assert.equal(verified.executables.length, 4);
  assert.equal(verified.runtime_manifests.length, 3);
  assert.match(verified.composition_digest, /^[a-f0-9]{64}$/);
  const manifestBody = await readFile(
    path.join(fixture.resources, BUNDLE_COMPOSITION_FILE),
    "utf8",
  );
  assert.equal(manifestBody, `${JSON.stringify(verified)}\n`);
});

test("fails closed on missing, non-canonical, unknown, or tampered bundle evidence", async (context) => {
  const fixture = await bundleFixture(context);
  await assert.rejects(
    verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
    }),
    /bundle composition evidence is unavailable/,
  );

  await writeBundleComposition({ appBundle: fixture.appBundle, targetTriple: TARGET });
  const evidence = path.join(fixture.resources, BUNDLE_COMPOSITION_FILE);
  const manifest = JSON.parse(await readFile(evidence, "utf8"));
  await chmod(evidence, 0o600);
  await writeFile(evidence, `${JSON.stringify({ ...manifest, unknown: true })}\n`);
  await assert.rejects(
    verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
    }),
    /bundle composition evidence is invalid/,
  );

  await rm(evidence);
  await writeBundleComposition({ appBundle: fixture.appBundle, targetTriple: TARGET });
  await writeExecutable(
    path.join(fixture.appBundle, "Contents", "MacOS", "resume-daemon"),
    syntheticMachO("tampered-daemon"),
  );
  await assert.rejects(
    verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
    }),
    /bundle composition payload does not match/,
  );

  await writeExecutable(
    path.join(fixture.appBundle, "Contents", "MacOS", "resume-daemon"),
    syntheticMachO("synthetic-resume-daemon"),
  );
  await writeFile(
    path.join(fixture.resources, "classifier", "runtime-pack", "payload.bin"),
    "tampered-classifier-payload",
  );
  await assert.rejects(
    verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
    }),
    /runtime pack does not match manifest/,
  );
});

test("rejects bundle payloads reached through an internal directory symlink", async (context) => {
  const fixture = await bundleFixture(context);
  const linkedRuntime = path.join(fixture.resources, "ocr");
  const externalRuntime = path.join(fixture.root, "external-ocr");
  await rm(linkedRuntime, { recursive: true });
  await mkdir(path.join(externalRuntime, "runtime-pack"), { recursive: true });
  const payload = Buffer.from("external-payload");
  await writeFile(path.join(externalRuntime, "runtime-pack", "payload.bin"), payload);
  await writeFile(
    path.join(externalRuntime, "runtime-pack", "runtime-pack.json"),
    `${JSON.stringify({
      schema_version: "synthetic.ocr.v1",
      files: [
        {
          role: "payload",
          file: "payload.bin",
          bytes: payload.length,
          sha256: sha256(payload),
        },
      ],
    })}\n`,
  );
  await symlink(externalRuntime, linkedRuntime);

  await assert.rejects(
    createBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
    }),
    /bundle composition runtime manifest is invalid/,
  );
});

test("rejects an unbound fifth native payload", async (context) => {
  const fixture = await bundleFixture(context);
  await writeExecutable(
    path.join(fixture.appBundle, "Contents", "MacOS", "unexpected-helper"),
    syntheticMachO("unexpected-helper"),
  );
  await assert.rejects(
    createBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
    }),
    /bundle composition native payload is invalid/,
  );
});

test("persists and reloads an owner-only receipt bound to the verified composition", async (context) => {
  const fixture = await bundleFixture(context);
  const applicationSupportRoot = path.join(fixture.root, "Library", "Application Support");
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  const composition = await createBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
  });
  const receipt = createInstallReceipt({
    composition,
    dmgSha256: sha256("synthetic-dmg"),
  });

  await persistInstallReceipt({ applicationSupportRoot, receipt });
  assert.deepEqual(await readInstallReceipt({ applicationSupportRoot }), receipt);
  const receiptFile = installReceiptPath(applicationSupportRoot);
  assert.equal((await stat(receiptFile)).mode & 0o077, 0);
  assert.equal(await readFile(receiptFile, "utf8"), `${JSON.stringify(receipt)}\n`);
});

test("receipt replacement is atomic and rejects drift or unsafe roots", async (context) => {
  const fixture = await bundleFixture(context);
  const applicationSupportRoot = path.join(fixture.root, "Library", "Application Support");
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  const composition = await createBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
  });
  const first = createInstallReceipt({
    composition,
    dmgSha256: sha256("first-dmg"),
  });
  const second = createInstallReceipt({
    composition: { ...composition, version: "0.1.2" },
    dmgSha256: sha256("second-dmg"),
  });
  await persistInstallReceipt({ applicationSupportRoot, receipt: first });
  let persistSyncCalls = 0;
  await assert.rejects(
    persistInstallReceipt({
      applicationSupportRoot,
      receipt: second,
      operations: {
        syncDirectory: async () => {
          persistSyncCalls += 1;
          if (persistSyncCalls === 1) throw new Error("synthetic fsync failure");
        },
      },
    }),
    /install receipt could not be persisted/,
  );
  assert.deepEqual(await readInstallReceipt({ applicationSupportRoot }), first);

  await persistInstallReceipt({ applicationSupportRoot, receipt: second });
  assert.deepEqual(await readInstallReceipt({ applicationSupportRoot }), second);

  let removeSyncCalls = 0;
  await assert.rejects(
    removeInstallReceipt({
      applicationSupportRoot,
      operations: {
        syncDirectory: async () => {
          removeSyncCalls += 1;
          if (removeSyncCalls === 1) throw new Error("synthetic fsync failure");
        },
      },
    }),
    /install receipt could not be removed/,
  );
  assert.deepEqual(await readInstallReceipt({ applicationSupportRoot }), second);

  const body = JSON.parse(await readFile(installReceiptPath(applicationSupportRoot), "utf8"));
  await writeFile(
    installReceiptPath(applicationSupportRoot),
    `${JSON.stringify({ ...body, extra: true })}\n`,
  );
  await assert.rejects(
    readInstallReceipt({ applicationSupportRoot }),
    /install receipt is invalid/,
  );
  await assert.rejects(
    persistInstallReceipt({ applicationSupportRoot: fixture.root, receipt: first }),
    /application support root is invalid/,
  );
});

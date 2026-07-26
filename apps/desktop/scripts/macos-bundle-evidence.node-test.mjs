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
import { createServer } from "node:net";
import path from "node:path";
import test from "node:test";

import {
  BUNDLE_COMPOSITION_FILE,
  createBundleComposition,
  readBundleCompositionEvidence,
  verifyBundleComposition,
  writeBundleComposition,
} from "./macos-bundle-composition.mjs";
import {
  createInstallReceipt,
  installReceiptPath,
  persistInstallReceipt,
  readInstallReceipt,
  removeInstallReceipt,
  verifyInstallReceipt,
} from "./macos-install-receipt.mjs";
import {
  createInstalledFaultRecoveryAuthority,
  readInstalledFaultRecoveryAuthority,
} from "./macos-installed-main-acceptance/native-fault-recovery-authority.mjs";

const TARGET = "aarch64-apple-darwin";
const VERSION = "0.1.2";
const SOURCE_COMMIT = "0123456789abcdef0123456789abcdef01234567";
const SOURCE = Object.freeze({
  authority: "worktree_snapshot",
  base_commit: SOURCE_COMMIT,
  source_tree_sha256: "a".repeat(64),
});
const verifySyntheticSignaturePolicy = async () => ({
  code_signature: "ad_hoc_valid",
  hardened_runtime: true,
  library_validation_entitlement_scope: "embedding_runtime_only",
});

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

async function bundleFixture(
  context,
  temporaryPrefix = path.join(os.tmpdir(), "resume-ir-bundle-evidence-"),
) {
  const temporary = await mkdtemp(temporaryPrefix);
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
    const files = [
      {
        role: "payload",
        file: "payload.bin",
        bytes: payload.length,
        sha256: sha256(payload),
      },
    ];
    if (pack === "classifier") {
      const model = Buffer.from('{"model":"synthetic"}\n');
      await writeFile(
        path.join(packDirectory, "linear-promotion-model.json"),
        model,
      );
      files.push({
        role: "model",
        file: "linear-promotion-model.json",
        bytes: model.length,
        sha256: sha256(model),
      });
    } else if (pack === "ocr") {
      const engine = Buffer.from("synthetic-tesseract-runtime");
      await writeExecutable(path.join(packDirectory, "tesseract"), engine);
      files.push({
        role: "executable",
        file: "tesseract",
        bytes: engine.length,
        sha256: sha256(engine),
      });
    }
    await writeFile(
      path.join(packDirectory, "runtime-pack.json"),
      `${JSON.stringify({
        schema_version: `synthetic.${pack}.v1`,
        files,
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
    source: SOURCE,
  });
  await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const signatureDirectory = path.join(
    fixture.appBundle,
    "Contents",
    "_CodeSignature",
  );
  await mkdir(signatureDirectory);
  await writeFile(path.join(signatureDirectory, "CodeResources"), "signature-slot");
  const verified = await verifyBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    expectedVersion: VERSION,
    expectedSource: SOURCE,
    verifySignaturePolicy: verifySyntheticSignaturePolicy,
  });

  assert.deepEqual(verified, expected);
  assert.equal(verified.executables.length, 4);
  assert.equal(verified.runtime_manifests.length, 3);
  assert.equal(verified.app_files.length, 14);
  assert.equal(
    verified.app_files.some(({ file }) => file === "Contents/Info.plist"),
    true,
  );
  assert.equal(
    verified.app_files.some(({ file }) =>
      file.endsWith(BUNDLE_COMPOSITION_FILE),
    ),
    false,
  );
  assert.match(verified.app_tree_digest, /^[a-f0-9]{64}$/);
  assert.match(verified.composition_digest, /^[a-f0-9]{64}$/);
  const manifestBody = await readFile(
    path.join(fixture.resources, BUNDLE_COMPOSITION_FILE),
    "utf8",
  );
  assert.equal(manifestBody, `${JSON.stringify(verified)}\n`);
});

test("binds v3 composition verification to the exact signature policy", async (context) => {
  const fixture = await bundleFixture(context);
  await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const request = {
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    expectedVersion: VERSION,
    expectedSource: SOURCE,
  };
  for (const verifySignaturePolicy of [
    undefined,
    async () => ({
      code_signature: "ad_hoc_valid",
      hardened_runtime: true,
    }),
    async () => ({
      ...(await verifySyntheticSignaturePolicy()),
      unknown_policy_field: "not-allowed",
    }),
  ]) {
    await assert.rejects(
      verifyBundleComposition({ ...request, verifySignaturePolicy }),
      /bundle signature policy/,
    );
  }
});

test("fails closed on missing, non-canonical, unknown, or tampered bundle evidence", async (context) => {
  const fixture = await bundleFixture(context);
  await assert.rejects(
    verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
      expectedSource: SOURCE,
      verifySignaturePolicy: verifySyntheticSignaturePolicy,
    }),
    /bundle composition evidence is unavailable/,
  );

  await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const evidence = path.join(fixture.resources, BUNDLE_COMPOSITION_FILE);
  const manifest = JSON.parse(await readFile(evidence, "utf8"));
  await chmod(evidence, 0o600);
  await writeFile(evidence, `${JSON.stringify({ ...manifest, unknown: true })}\n`);
  await assert.rejects(
    verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
      expectedSource: SOURCE,
      verifySignaturePolicy: verifySyntheticSignaturePolicy,
    }),
    /bundle composition evidence is invalid/,
  );

  await rm(evidence);
  await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  await writeExecutable(
    path.join(fixture.appBundle, "Contents", "MacOS", "resume-daemon"),
    syntheticMachO("tampered-daemon"),
  );
  await assert.rejects(
    verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
      expectedSource: SOURCE,
      verifySignaturePolicy: verifySyntheticSignaturePolicy,
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
      expectedSource: SOURCE,
      verifySignaturePolicy: verifySyntheticSignaturePolicy,
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
      source: SOURCE,
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
      source: SOURCE,
    }),
    /bundle composition native payload is invalid/,
  );
});

test("binds every regular App file and rejects unbound resource drift", async (context) => {
  const fixture = await bundleFixture(context);
  const frontend = path.join(fixture.resources, "frontend");
  const frontendAsset = path.join(frontend, "index.html");
  await mkdir(frontend);
  await writeFile(frontendAsset, "verified-frontend");
  await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });

  await writeFile(frontendAsset, "mutated-frontend");
  await assert.rejects(
    verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
      expectedSource: SOURCE,
      verifySignaturePolicy: verifySyntheticSignaturePolicy,
    }),
    /bundle composition payload does not match/,
  );
});

test("orders the complete App file tree by UTF-8 bytes", async (context) => {
  const fixture = await bundleFixture(context);
  const byteEarlier = "\uE000.bin";
  const byteLater = "\u{10000}.bin";
  await writeFile(path.join(fixture.resources, byteLater), "later");
  await writeFile(path.join(fixture.resources, byteEarlier), "earlier");

  const composition = await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const unicodeFiles = composition.app_files
    .map(({ file }) => file)
    .filter((file) => file.endsWith(byteEarlier) || file.endsWith(byteLater));

  assert.deepEqual(unicodeFiles, [
    `Contents/Resources/${byteEarlier}`,
    `Contents/Resources/${byteLater}`,
  ]);
  assert.deepEqual(
    await verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
      expectedSource: SOURCE,
      verifySignaturePolicy: verifySyntheticSignaturePolicy,
    }),
    composition,
  );
});

test("rejects unsafe entries inside the excluded code-signature subtree", async (context) => {
  const fixture = await bundleFixture(context);
  await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const signatureDirectory = path.join(
    fixture.appBundle,
    "Contents",
    "_CodeSignature",
  );
  await mkdir(signatureDirectory);
  await symlink(
    path.join(fixture.resources, "icon.icns"),
    path.join(signatureDirectory, "unsafe-link"),
  );

  await assert.rejects(
    verifyBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      expectedVersion: VERSION,
      expectedSource: SOURCE,
      verifySignaturePolicy: verifySyntheticSignaturePolicy,
    }),
    /bundle composition App file tree is invalid/,
  );
});

test(
  "rejects irregular entries in the complete App file tree",
  { skip: process.platform === "win32" },
  async (context) => {
    const fixture = await bundleFixture(context, "/tmp/ri-bundle-");
    const socketPath = path.join(fixture.resources, "irregular.sock");
    const server = createServer();
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(socketPath, resolve);
    });
    try {
      await assert.rejects(
        createBundleComposition({
          appBundle: fixture.appBundle,
          targetTriple: TARGET,
          source: SOURCE,
        }),
        /bundle composition App file tree is invalid/,
      );
    } finally {
      await new Promise((resolve) => server.close(resolve));
    }
  },
);

test("rejects file 4097 at the cap before validating or hashing it", async (context) => {
  const fixture = await bundleFixture(context);
  const baseline = await createBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const capDirectory = path.join(fixture.appBundle, "Contents", "zz-cap");
  await mkdir(capDirectory);
  const extraFileCount = 4096 - baseline.app_files.length + 1;
  for (let start = 0; start < extraFileCount; start += 128) {
    await Promise.all(
      Array.from(
        { length: Math.min(128, extraFileCount - start) },
        (_, offset) => {
          const index = start + offset;
          const body = index === extraFileCount - 1 ? "" : "x";
          return writeFile(
            path.join(capDirectory, `file-${index.toString().padStart(4, "0")}`),
            body,
          );
        },
      ),
    );
  }

  await assert.rejects(
    createBundleComposition({
      appBundle: fixture.appBundle,
      targetTriple: TARGET,
      source: SOURCE,
    }),
    /bundle composition App file cap exceeded/,
  );
});

test("installed receipt rejects added App files after evidence regeneration and ad-hoc resign", async (context) => {
  const fixture = await bundleFixture(context);
  const original = await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const receipt = createInstallReceipt({
    composition: original,
    dmgSha256: sha256("verified-dmg"),
  });
  await writeFile(path.join(fixture.resources, "injected.js"), "injected");
  await rm(path.join(fixture.resources, BUNDLE_COMPOSITION_FILE));
  await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const resigned = await verifyBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    expectedVersion: VERSION,
    expectedSource: SOURCE,
    verifySignaturePolicy: verifySyntheticSignaturePolicy,
  });

  assert.notEqual(resigned.composition_digest, original.composition_digest);
  assert.throws(
    () => verifyInstallReceipt({ receipt, composition: resigned }),
    /install receipt does not match bundle composition/,
  );
});

test("persists and reloads an owner-only receipt bound to the verified composition", async (context) => {
  const fixture = await bundleFixture(context);
  const applicationSupportRoot = path.join(fixture.root, "Library", "Application Support");
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  const composition = await createBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
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

test("crash recovery authority is bound to canonical composition evidence and the owner-only receipt", async (context) => {
  const fixture = await bundleFixture(context);
  const applicationSupportRoot = path.join(
    fixture.root,
    "Library",
    "Application Support",
  );
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  const composition = await writeBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const receipt = createInstallReceipt({
    composition,
    dmgSha256: sha256("synthetic-dmg"),
  });
  await persistInstallReceipt({ applicationSupportRoot, receipt });

  assert.deepEqual(
    await readBundleCompositionEvidence({ appBundle: fixture.appBundle }),
    composition,
  );
  assert.deepEqual(
    await readInstalledFaultRecoveryAuthority({
      appBundle: fixture.appBundle,
      applicationSupportRoot,
    }),
    createInstalledFaultRecoveryAuthority(composition),
  );

  await writeFile(
    installReceiptPath(applicationSupportRoot),
    `${JSON.stringify({ ...receipt, composition_digest: "f".repeat(64) })}\n`,
    { mode: 0o600 },
  );
  await assert.rejects(
    readInstalledFaultRecoveryAuthority({
      appBundle: fixture.appBundle,
      applicationSupportRoot,
    }),
    /installed fault recovery authority is invalid/,
  );
});

test("receipt replacement is atomic and rejects drift or unsafe roots", async (context) => {
  const fixture = await bundleFixture(context);
  const applicationSupportRoot = path.join(fixture.root, "Library", "Application Support");
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  const composition = await createBundleComposition({
    appBundle: fixture.appBundle,
    targetTriple: TARGET,
    source: SOURCE,
  });
  const first = createInstallReceipt({
    composition,
    dmgSha256: sha256("first-dmg"),
  });
  const second = createInstallReceipt({
    composition: { ...composition, version: "0.1.3" },
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

  await assert.rejects(
    persistInstallReceipt({
      applicationSupportRoot,
      receipt: first,
      expectedReceipt: first,
    }),
    /install receipt does not match expected transaction/,
  );
  assert.deepEqual(await readInstallReceipt({ applicationSupportRoot }), second);

  await persistInstallReceipt({
    applicationSupportRoot,
    receipt: first,
    expectedReceipt: second,
  });
  assert.deepEqual(await readInstallReceipt({ applicationSupportRoot }), first);

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
  assert.deepEqual(await readInstallReceipt({ applicationSupportRoot }), first);

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

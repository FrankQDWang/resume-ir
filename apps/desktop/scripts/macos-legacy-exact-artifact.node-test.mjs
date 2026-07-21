import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  appendFile,
  chmod,
  lstat,
  mkdtemp,
  mkdir,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  createInstallReceiptEvidence,
  createInstallReceipt,
} from "./macos-install-receipt.mjs";
import {
  LEGACY_EXACT_BUNDLE_ID,
  LEGACY_EXACT_COMPOSITION_DIGEST,
  LEGACY_EXACT_DMG_SHA256,
  LEGACY_EXACT_TARGET,
  LEGACY_EXACT_VERSION,
  LEGACY_INSTALL_RECEIPT_SCHEMA,
  legacyExactInstallReceiptPath,
  readInstallReceiptSet,
  removeLegacyExactInstallReceipt,
  validateLegacyExactInstallReceipt,
  verifyLegacyExactBundleComposition,
  verifyPinnedLegacyBundlePayload,
} from "./macos-legacy-exact-artifact.mjs";
import { prepareOwnerEvidenceDirectory } from "./macos-owner-evidence-store.mjs";

const SOURCE_COMMIT = "0123456789abcdef0123456789abcdef01234567";

function sha256(body) {
  return createHash("sha256").update(body).digest("hex");
}

function syntheticMachO(payload, cpuType = 0x0100000c) {
  const header = Buffer.alloc(32);
  header.writeUInt32LE(0xfeedfacf, 0);
  header.writeUInt32LE(cpuType, 4);
  return Buffer.concat([header, Buffer.from(payload)]);
}

async function writeExecutable(file, body) {
  await writeFile(file, body);
  await chmod(file, 0o755);
}

function legacyReceipt() {
  return {
    schema_version: LEGACY_INSTALL_RECEIPT_SCHEMA,
    bundle_id: LEGACY_EXACT_BUNDLE_ID,
    version: LEGACY_EXACT_VERSION,
    target_triple: LEGACY_EXACT_TARGET,
    composition_digest: LEGACY_EXACT_COMPOSITION_DIGEST,
    dmg_sha256: LEGACY_EXACT_DMG_SHA256,
  };
}

function currentReceipt() {
  return createInstallReceipt({
    composition: {
      bundle_id: LEGACY_EXACT_BUNDLE_ID,
      version: "0.1.2",
      target_triple: LEGACY_EXACT_TARGET,
      source_commit: SOURCE_COMMIT,
      composition_digest: "a".repeat(64),
    },
    dmgSha256: "b".repeat(64),
  });
}

async function fixture(context) {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-legacy-receipt-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const applicationSupportRoot = path.join(root, "Library", "Application Support");
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  await prepareOwnerEvidenceDirectory(applicationSupportRoot);
  return { root, applicationSupportRoot };
}

async function syntheticV1BundleFixture(context) {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-legacy-bundle-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const appBundle = path.join(root, "resume-ir.app");
  const contents = path.join(appBundle, "Contents");
  const macos = path.join(contents, "MacOS");
  const resources = path.join(contents, "Resources");
  await mkdir(macos, { recursive: true });
  await writeFile(
    path.join(contents, "Info.plist"),
    [
      '<?xml version="1.0" encoding="UTF-8"?>',
      '<plist version="1.0"><dict>',
      "<key>CFBundleIdentifier</key><string>local.resume-ir.desktop</string>",
      "<key>CFBundleShortVersionString</key><string>0.1.1</string>",
      "<key>CFBundleDisplayName</key><string>resume-ir</string>",
      "<key>CFBundleIconFile</key><string>icon.icns</string>",
      "<key>CFBundleExecutable</key><string>resume-desktop</string>",
      "</dict></plist>",
    ].join(""),
  );
  const executableContracts = [
    { role: "desktop", file: "resume-desktop" },
    { role: "daemon", file: "resume-daemon" },
    { role: "embedding_runtime", file: "resume-embedding-runtime" },
    { role: "pdf_renderer", file: "resume-pdf-render-runtime" },
  ];
  const executables = [];
  for (const contract of executableContracts) {
    const body = syntheticMachO(`synthetic-${contract.file}`);
    await writeExecutable(path.join(macos, contract.file), body);
    executables.push({ ...contract, sha256: sha256(body) });
  }
  const runtimeManifests = [];
  for (const role of ["classifier", "embedding", "ocr"]) {
    const directory = path.join(resources, role, "runtime-pack");
    await mkdir(directory, { recursive: true });
    const payload = Buffer.from(`synthetic-${role}-payload`);
    const source = `${JSON.stringify({
      schema_version: `synthetic.${role}.v1`,
      files: [
        {
          role: "payload",
          file: "payload.bin",
          bytes: payload.length,
          sha256: sha256(payload),
        },
      ],
    })}\n`;
    await writeFile(path.join(directory, "payload.bin"), payload);
    await writeFile(path.join(directory, "runtime-pack.json"), source);
    runtimeManifests.push({
      role,
      file: `${role}/runtime-pack/runtime-pack.json`,
      sha256: sha256(source),
    });
  }
  const icon = Buffer.from("synthetic-approved-v1-icon");
  await writeFile(path.join(resources, "icon.icns"), icon);
  const body = {
    schema_version: "resume-ir.macos-bundle-composition.v1",
    bundle_id: "local.resume-ir.desktop",
    version: "0.1.1",
    target_triple: "aarch64-apple-darwin",
    mach_o_digest: "sha256_without_code_signature_v1",
    file_digest: "sha256",
    executables,
    runtime_manifests: runtimeManifests,
    icon: { file: "icon.icns", sha256: sha256(icon) },
  };
  const composition = {
    ...body,
    composition_digest: sha256(JSON.stringify(body)),
  };
  await writeFile(
    path.join(resources, "resume-ir.bundle-composition.v1.json"),
    `${JSON.stringify(composition)}\n`,
  );
  return {
    appBundle,
    composition,
    daemon: path.join(macos, "resume-daemon"),
  };
}

async function writeLegacy(applicationSupportRoot, receipt = legacyReceipt()) {
  await writeFile(
    legacyExactInstallReceiptPath(applicationSupportRoot),
    `${JSON.stringify(receipt)}\n`,
    { mode: 0o600 },
  );
}

test("recognizes only the exact 0.1.1 predecessor receipt", () => {
  assert.deepEqual(validateLegacyExactInstallReceipt(legacyReceipt()), legacyReceipt());
  for (const change of [
    { composition_digest: "0".repeat(64) },
    { dmg_sha256: "0".repeat(64) },
    { version: "0.1.0" },
    { source_commit: SOURCE_COMMIT },
  ]) {
    assert.throws(
      () => validateLegacyExactInstallReceipt({ ...legacyReceipt(), ...change }),
      /legacy exact artifact is invalid/,
    );
  }
});

test("rejects a recomputed but non-pinned synthetic v1 composition", async (context) => {
  const values = await syntheticV1BundleFixture(context);
  const verifySignaturePolicy = async () => ({
    code_signature: "ad_hoc_valid",
    hardened_runtime: true,
    library_validation_entitlement_scope: "embedding_runtime_only",
  });
  assert.notEqual(
    values.composition.composition_digest,
    LEGACY_EXACT_COMPOSITION_DIGEST,
  );
  await assert.rejects(
    verifyPinnedLegacyBundlePayload({
      appBundle: values.appBundle,
      targetTriple: "aarch64-apple-darwin",
      expectedVersion: "0.1.1",
      composition: values.composition,
      platform: "darwin",
      verifySignaturePolicy,
    }),
    /legacy exact artifact is invalid/,
  );
});

test("rejects executable mutation in an exact installed v1 App clone", async (context) => {
  const installed = "/Applications/resume-ir.app";
  if (process.platform !== "darwin") {
    context.skip("requires macOS exact-v1 installed App evidence");
    return;
  }
  try {
    await lstat(installed);
  } catch {
    context.skip("exact-v1 installed App evidence is unavailable");
    return;
  }
  const installedInfo = await readFile(
    path.join(installed, "Contents", "Info.plist"),
    "utf8",
  );
  if (
    !/<key>\s*CFBundleShortVersionString\s*<\/key>\s*<string>\s*0\.1\.1\s*<\/string>/.test(
      installedInfo,
    )
  ) {
    context.skip("installed App is not the exact-v1 predecessor");
    return;
  }
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-exact-v1-clone-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const appBundle = path.join(root, "resume-ir.app");
  const copy = spawnSync("/bin/cp", ["-cR", installed, appBundle], {
    encoding: "utf8",
    shell: false,
  });
  assert.equal(copy.status, 0, `${copy.stdout}\n${copy.stderr}`);
  const verification = {
    appBundle,
    targetTriple: "aarch64-apple-darwin",
    expectedVersion: "0.1.1",
  };
  assert.equal(
    (await verifyLegacyExactBundleComposition(verification))
      .composition_digest,
    LEGACY_EXACT_COMPOSITION_DIGEST,
  );
  await assert.rejects(
    verifyLegacyExactBundleComposition({
      ...verification,
      verifySignaturePolicy: async () => ({
        code_signature: "ad_hoc_valid",
        hardened_runtime: true,
      }),
    }),
    /legacy exact artifact is invalid/,
  );

  await appendFile(
    path.join(appBundle, "Contents", "MacOS", "resume-daemon"),
    "mutated",
  );
  await assert.rejects(
    verifyLegacyExactBundleComposition(verification),
    /legacy exact artifact is invalid/,
  );
});

test("classifies only legacy_only, both_valid, and current_only", async (context) => {
  const { applicationSupportRoot } = await fixture(context);
  await writeLegacy(applicationSupportRoot);
  assert.equal(
    (await readInstallReceiptSet({ applicationSupportRoot })).state,
    "legacy_only",
  );

  await createInstallReceiptEvidence({
    applicationSupportRoot,
    receipt: currentReceipt(),
  });
  assert.equal(
    (await readInstallReceiptSet({ applicationSupportRoot })).state,
    "both_valid",
  );

  await removeLegacyExactInstallReceipt({
    applicationSupportRoot,
    expectedReceipt: legacyReceipt(),
  });
  assert.equal(
    (await readInstallReceiptSet({ applicationSupportRoot })).state,
    "current_only",
  );
});

test("fails closed without deleting mismatched or absent evidence", async (context) => {
  const { applicationSupportRoot } = await fixture(context);
  await assert.rejects(
    readInstallReceiptSet({ applicationSupportRoot }),
    /install receipt set is invalid/,
  );

  const mismatched = { ...legacyReceipt(), dmg_sha256: "0".repeat(64) };
  await writeLegacy(applicationSupportRoot, mismatched);
  await assert.rejects(
    readInstallReceiptSet({ applicationSupportRoot }),
    /legacy exact install receipt is invalid/,
  );
  assert.equal(
    await readFile(legacyExactInstallReceiptPath(applicationSupportRoot), "utf8"),
    `${JSON.stringify(mismatched)}\n`,
  );
});

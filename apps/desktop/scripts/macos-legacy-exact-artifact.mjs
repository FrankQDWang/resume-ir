import { createHash } from "node:crypto";
import { lstat, open, readFile, readdir, realpath } from "node:fs/promises";
import path from "node:path";

import { readInstallReceipt } from "./macos-install-receipt.mjs";
import { verifyMacosInternalTestSignaturePolicy } from "./verify-macos-dmg.mjs";
import {
  executablePayloadSha256,
  sha256,
} from "./verify-bundled-sidecar.mjs";
import {
  ownerEvidencePath,
  readOwnerEvidence,
  removeOwnerEvidence,
} from "./macos-owner-evidence-store.mjs";

export const LEGACY_EXACT_VERSION = "0.1.1";
export const LEGACY_EXACT_TARGET = "aarch64-apple-darwin";
export const LEGACY_EXACT_BUNDLE_ID = "local.resume-ir.desktop";
export const LEGACY_EXACT_COMPOSITION_DIGEST =
  "18a2d41769f6e2fcc6cc504085b40f25ec185a27109eac525e551513ec5801c6";
export const LEGACY_EXACT_DMG_SHA256 =
  "363ce8d5db7c120a05fc7c282a9f9b6a8e1173f3175c308839dfb1440867c780";
export const LEGACY_BUNDLE_COMPOSITION_FILE =
  "resume-ir.bundle-composition.v1.json";
export const LEGACY_BUNDLE_COMPOSITION_SCHEMA =
  "resume-ir.macos-bundle-composition.v1";
export const LEGACY_INSTALL_RECEIPT_FILE =
  "resume-ir.install-receipt.v1.json";
export const LEGACY_INSTALL_RECEIPT_SCHEMA =
  "resume-ir.macos-install-receipt.v1";

const MAX_BUNDLE_EVIDENCE_BYTES = 16 * 1024;
const MAX_RECEIPT_BYTES = 4 * 1024;
const MAX_INFO_PLIST_BYTES = 64 * 1024;
const MAX_RUNTIME_MANIFEST_BYTES = 1024 * 1024;
const MAX_BOUND_FILE_BYTES = 512 * 1024 * 1024;
const MAX_RUNTIME_FILES = 128;
const DIGEST = /^[a-f0-9]{64}$/;
const EXPECTED_DISPLAY_NAME = "resume-ir";
const EXPECTED_MAIN_EXECUTABLE = "resume-desktop";
const EXECUTABLES = Object.freeze([
  Object.freeze({ role: "desktop", file: "resume-desktop" }),
  Object.freeze({ role: "daemon", file: "resume-daemon" }),
  Object.freeze({ role: "embedding_runtime", file: "resume-embedding-runtime" }),
  Object.freeze({ role: "pdf_renderer", file: "resume-pdf-render-runtime" }),
]);
const RUNTIME_MANIFESTS = Object.freeze([
  Object.freeze({
    role: "classifier",
    file: "classifier/runtime-pack/runtime-pack.json",
  }),
  Object.freeze({
    role: "embedding",
    file: "embedding/runtime-pack/runtime-pack.json",
  }),
  Object.freeze({ role: "ocr", file: "ocr/runtime-pack/runtime-pack.json" }),
]);

function legacyError(message = "legacy exact artifact is invalid") {
  return new Error(message);
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value)) === JSON.stringify(expected)
  );
}

function validateEntry(entry, expected) {
  return (
    exactKeys(entry, ["role", "file", "sha256"]) &&
    entry.role === expected.role &&
    entry.file === expected.file &&
    DIGEST.test(entry.sha256)
  );
}

function compositionBody(value) {
  const {
    schema_version,
    bundle_id,
    version,
    target_triple,
    mach_o_digest,
    file_digest,
    executables,
    runtime_manifests,
    icon,
  } = value;
  return {
    schema_version,
    bundle_id,
    version,
    target_triple,
    mach_o_digest,
    file_digest,
    executables,
    runtime_manifests,
    icon,
  };
}

function digestCompositionBody(value) {
  return createHash("sha256")
    .update(JSON.stringify(compositionBody(value)))
    .digest("hex");
}

function validateLegacyBundleCompositionShape(value) {
  if (
    !exactKeys(value, [
      "schema_version",
      "bundle_id",
      "version",
      "target_triple",
      "mach_o_digest",
      "file_digest",
      "executables",
      "runtime_manifests",
      "icon",
      "composition_digest",
    ]) ||
    value.schema_version !== LEGACY_BUNDLE_COMPOSITION_SCHEMA ||
    value.bundle_id !== LEGACY_EXACT_BUNDLE_ID ||
    value.version !== LEGACY_EXACT_VERSION ||
    value.target_triple !== LEGACY_EXACT_TARGET ||
    value.mach_o_digest !== "sha256_without_code_signature_v1" ||
    value.file_digest !== "sha256" ||
    !Array.isArray(value.executables) ||
    value.executables.length !== EXECUTABLES.length ||
    !value.executables.every((entry, index) =>
      validateEntry(entry, EXECUTABLES[index]),
    ) ||
    !Array.isArray(value.runtime_manifests) ||
    value.runtime_manifests.length !== RUNTIME_MANIFESTS.length ||
    !value.runtime_manifests.every((entry, index) =>
      validateEntry(entry, RUNTIME_MANIFESTS[index]),
    ) ||
    !exactKeys(value.icon, ["file", "sha256"]) ||
    value.icon.file !== "icon.icns" ||
    !DIGEST.test(value.icon.sha256) ||
    !DIGEST.test(value.composition_digest) ||
    digestCompositionBody(value) !== value.composition_digest
  ) {
    throw legacyError();
  }
  return value;
}

export function validateLegacyExactBundleComposition(value) {
  const composition = validateLegacyBundleCompositionShape(value);
  if (composition.composition_digest !== LEGACY_EXACT_COMPOSITION_DIGEST) {
    throw legacyError();
  }
  return composition;
}

async function requireBoundFile(file) {
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([lstat(file), realpath(file)]);
  } catch {
    throw legacyError();
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    resolved !== path.resolve(file) ||
    metadata.size === 0 ||
    metadata.size > MAX_BOUND_FILE_BYTES
  ) {
    throw legacyError();
  }
  return metadata;
}

function safeRelativePath(file) {
  return (
    typeof file === "string" &&
    file.length > 0 &&
    file.length <= 256 &&
    file === file.replaceAll("\\", "/") &&
    !path.posix.isAbsolute(file) &&
    file
      .split("/")
      .every((part) => part.length > 0 && part !== "." && part !== "..")
  );
}

async function recursiveFiles(root, relative = "") {
  let entries;
  try {
    entries = await readdir(path.join(root, relative), {
      withFileTypes: true,
    });
  } catch {
    throw legacyError();
  }
  const files = [];
  for (const entry of entries) {
    const child = path.posix.join(relative, entry.name);
    if (entry.isSymbolicLink()) throw legacyError();
    if (entry.isDirectory()) {
      files.push(...(await recursiveFiles(root, child)));
    } else if (entry.isFile()) {
      files.push(child);
    } else {
      throw legacyError();
    }
  }
  return files.sort();
}

async function requireArm64MachO(file) {
  let handle;
  try {
    handle = await open(file, "r");
    const header = Buffer.alloc(8);
    const { bytesRead } = await handle.read(header, 0, header.length, 0);
    if (
      bytesRead !== header.length ||
      header.readUInt32LE(0) !== 0xfeedfacf ||
      header.readUInt32LE(4) !== 0x0100000c
    ) {
      throw legacyError();
    }
  } catch (error) {
    if (error?.message === "legacy exact artifact is invalid") throw error;
    throw legacyError();
  } finally {
    await handle?.close().catch(() => {});
  }
}

function isNativeRuntimeEntry(entry) {
  return (
    entry.role === "runtime_library" ||
    entry.role === "engine_library" ||
    entry.role === "engine_binary" ||
    entry.executable === true ||
    entry.file.endsWith(".dylib")
  );
}

async function verifyRuntimePack(manifestFile) {
  const metadata = await requireBoundFile(manifestFile);
  if (metadata.size > MAX_RUNTIME_MANIFEST_BYTES) throw legacyError();
  let manifest;
  try {
    manifest = JSON.parse(await readFile(manifestFile, "utf8"));
  } catch {
    throw legacyError();
  }
  if (
    manifest === null ||
    typeof manifest !== "object" ||
    Array.isArray(manifest) ||
    !Array.isArray(manifest.files) ||
    manifest.files.length === 0 ||
    manifest.files.length > MAX_RUNTIME_FILES
  ) {
    throw legacyError();
  }
  const files = new Set();
  for (const entry of manifest.files) {
    if (
      entry === null ||
      typeof entry !== "object" ||
      Array.isArray(entry) ||
      typeof entry.role !== "string" ||
      entry.role.length === 0 ||
      entry.role.length > 64 ||
      !safeRelativePath(entry.file) ||
      files.has(entry.file) ||
      !Number.isSafeInteger(entry.bytes) ||
      entry.bytes <= 0 ||
      entry.bytes > MAX_BOUND_FILE_BYTES ||
      !DIGEST.test(entry.sha256)
    ) {
      throw legacyError();
    }
    files.add(entry.file);
    const payload = path.join(path.dirname(manifestFile), entry.file);
    const payloadMetadata = await requireBoundFile(payload);
    if (
      payloadMetadata.size !== entry.bytes ||
      (await sha256(payload)) !== entry.sha256
    ) {
      throw legacyError();
    }
    if (entry.executable === true && (payloadMetadata.mode & 0o111) === 0) {
      throw legacyError();
    }
    if (isNativeRuntimeEntry(entry)) await requireArm64MachO(payload);
  }
  const expectedFiles = ["runtime-pack.json", ...files].sort();
  if (
    JSON.stringify(await recursiveFiles(path.dirname(manifestFile))) !==
    JSON.stringify(expectedFiles)
  ) {
    throw legacyError();
  }
}

function plistString(source, field) {
  const escaped = field.replaceAll(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const matches = [
    ...source.matchAll(
      new RegExp(
        `<key>\\s*${escaped}\\s*<\\/key>\\s*<string>\\s*([^<]+?)\\s*<\\/string>`,
        "g",
      ),
    ),
  ];
  if (matches.length !== 1) throw legacyError();
  return matches[0][1];
}

async function verifyLegacyIdentity(appBundle) {
  const infoPlist = path.join(appBundle, "Contents", "Info.plist");
  const metadata = await requireBoundFile(infoPlist);
  if (metadata.size > MAX_INFO_PLIST_BYTES) throw legacyError();
  let source;
  try {
    source = await readFile(infoPlist, "utf8");
  } catch {
    throw legacyError();
  }
  if (
    plistString(source, "CFBundleIdentifier") !== LEGACY_EXACT_BUNDLE_ID ||
    plistString(source, "CFBundleShortVersionString") !==
      LEGACY_EXACT_VERSION ||
    plistString(source, "CFBundleDisplayName") !== EXPECTED_DISPLAY_NAME ||
    plistString(source, "CFBundleIconFile") !== "icon.icns" ||
    plistString(source, "CFBundleExecutable") !== EXPECTED_MAIN_EXECUTABLE
  ) {
    throw legacyError();
  }
}

async function resolveLegacyApp(appBundle) {
  if (!path.isAbsolute(appBundle)) throw legacyError();
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([
      lstat(appBundle),
      realpath(appBundle),
    ]);
  } catch {
    throw legacyError();
  }
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    resolved !== path.resolve(appBundle)
  ) {
    throw legacyError();
  }
  return resolved;
}

export async function verifyPinnedLegacyBundlePayload({
  appBundle,
  targetTriple,
  expectedVersion,
  composition,
  platform = process.platform,
  runner,
  verifySignaturePolicy = verifyMacosInternalTestSignaturePolicy,
}) {
  if (
    targetTriple !== LEGACY_EXACT_TARGET ||
    expectedVersion !== LEGACY_EXACT_VERSION ||
    typeof verifySignaturePolicy !== "function"
  ) {
    throw legacyError();
  }
  const pinned = validateLegacyExactBundleComposition(composition);
  const resolvedApp = await resolveLegacyApp(appBundle);
  await verifyLegacyIdentity(resolvedApp);
  const macos = path.join(resolvedApp, "Contents", "MacOS");
  const resources = path.join(resolvedApp, "Contents", "Resources");
  let macosEntries;
  try {
    macosEntries = (await readdir(macos)).sort();
  } catch {
    throw legacyError();
  }
  if (
    JSON.stringify(macosEntries) !==
    JSON.stringify(EXECUTABLES.map(({ file }) => file).sort())
  ) {
    throw legacyError();
  }
  const executables = [];
  for (const entry of EXECUTABLES) {
    const file = path.join(macos, entry.file);
    const metadata = await requireBoundFile(file);
    if ((metadata.mode & 0o111) === 0) throw legacyError();
    await requireArm64MachO(file);
    let digest;
    try {
      digest = await executablePayloadSha256(file);
    } catch {
      throw legacyError();
    }
    executables.push({ ...entry, sha256: digest });
  }
  const runtimeManifests = [];
  for (const entry of RUNTIME_MANIFESTS) {
    const file = path.join(resources, entry.file);
    await verifyRuntimePack(file);
    runtimeManifests.push({ ...entry, sha256: await sha256(file) });
  }
  const icon = path.join(resources, "icon.icns");
  await requireBoundFile(icon);
  const actualBase = {
    schema_version: LEGACY_BUNDLE_COMPOSITION_SCHEMA,
    bundle_id: LEGACY_EXACT_BUNDLE_ID,
    version: LEGACY_EXACT_VERSION,
    target_triple: LEGACY_EXACT_TARGET,
    mach_o_digest: "sha256_without_code_signature_v1",
    file_digest: "sha256",
    executables,
    runtime_manifests: runtimeManifests,
    icon: { file: "icon.icns", sha256: await sha256(icon) },
  };
  const actual = {
    ...actualBase,
    composition_digest: digestCompositionBody(actualBase),
  };
  if (JSON.stringify(actual) !== JSON.stringify(pinned)) throw legacyError();
  let signature;
  try {
    signature = await verifySignaturePolicy({
      appBundle: resolvedApp,
      platform,
      runner,
    });
  } catch {
    throw legacyError();
  }
  if (
    !exactKeys(signature, [
      "code_signature",
      "hardened_runtime",
      "library_validation_entitlement_scope",
    ]) ||
    signature.code_signature !== "ad_hoc_valid" ||
    signature.hardened_runtime !== true ||
    signature.library_validation_entitlement_scope !==
      "embedding_runtime_only"
  ) {
    throw legacyError();
  }
  return pinned;
}

export async function verifyLegacyExactBundleComposition({
  appBundle,
  targetTriple,
  expectedVersion,
  platform = process.platform,
  runner,
  verifySignaturePolicy = verifyMacosInternalTestSignaturePolicy,
}) {
  if (
    !path.isAbsolute(appBundle) ||
    targetTriple !== LEGACY_EXACT_TARGET ||
    expectedVersion !== LEGACY_EXACT_VERSION
  ) {
    throw legacyError();
  }
  let appMetadata;
  let resolvedApp;
  try {
    [appMetadata, resolvedApp] = await Promise.all([
      lstat(appBundle),
      realpath(appBundle),
    ]);
  } catch {
    throw legacyError();
  }
  if (
    !appMetadata.isDirectory() ||
    appMetadata.isSymbolicLink() ||
    resolvedApp !== path.resolve(appBundle)
  ) {
    throw legacyError();
  }
  const file = path.join(
    resolvedApp,
    "Contents",
    "Resources",
    LEGACY_BUNDLE_COMPOSITION_FILE,
  );
  let metadata;
  let source;
  let composition;
  try {
    metadata = await lstat(file);
    source = await readFile(file, "utf8");
    composition = validateLegacyExactBundleComposition(JSON.parse(source));
  } catch (error) {
    if (error?.message === "legacy exact artifact is invalid") throw error;
    throw legacyError();
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > MAX_BUNDLE_EVIDENCE_BYTES ||
    `${JSON.stringify(composition)}\n` !== source
  ) {
    throw legacyError();
  }
  return verifyPinnedLegacyBundlePayload({
    appBundle: resolvedApp,
    targetTriple,
    expectedVersion,
    composition,
    platform,
    runner,
    verifySignaturePolicy,
  });
}

export function validateLegacyExactInstallReceipt(receipt) {
  if (
    !exactKeys(receipt, [
      "schema_version",
      "bundle_id",
      "version",
      "target_triple",
      "composition_digest",
      "dmg_sha256",
    ]) ||
    receipt.schema_version !== LEGACY_INSTALL_RECEIPT_SCHEMA ||
    receipt.bundle_id !== LEGACY_EXACT_BUNDLE_ID ||
    receipt.version !== LEGACY_EXACT_VERSION ||
    receipt.target_triple !== LEGACY_EXACT_TARGET ||
    receipt.composition_digest !== LEGACY_EXACT_COMPOSITION_DIGEST ||
    receipt.dmg_sha256 !== LEGACY_EXACT_DMG_SHA256
  ) {
    throw legacyError();
  }
  return receipt;
}

export function legacyExactInstallReceiptPath(applicationSupportRoot) {
  return ownerEvidencePath(applicationSupportRoot, LEGACY_INSTALL_RECEIPT_FILE);
}

export async function readLegacyExactInstallReceipt({
  applicationSupportRoot,
  allowMissing = false,
}) {
  const evidence = await readOwnerEvidence({
    applicationSupportRoot,
    fileName: LEGACY_INSTALL_RECEIPT_FILE,
    maxBytes: MAX_RECEIPT_BYTES,
    validate: validateLegacyExactInstallReceipt,
    label: "legacy exact install receipt",
    allowMissing,
  });
  return evidence?.value;
}

export async function removeLegacyExactInstallReceipt({
  applicationSupportRoot,
  expectedReceipt,
  operations = {},
}) {
  return removeOwnerEvidence({
    applicationSupportRoot,
    fileName: LEGACY_INSTALL_RECEIPT_FILE,
    expectedValue: validateLegacyExactInstallReceipt(expectedReceipt),
    maxBytes: MAX_RECEIPT_BYTES,
    validate: validateLegacyExactInstallReceipt,
    label: "legacy exact install receipt",
    operations,
  });
}

export async function readInstallReceiptSet({
  applicationSupportRoot,
  readLegacyReceipt = readLegacyExactInstallReceipt,
  readCurrentReceipt = readInstallReceipt,
}) {
  const [legacyReceipt, currentReceipt] = await Promise.all([
    readLegacyReceipt({ applicationSupportRoot, allowMissing: true }),
    readCurrentReceipt({ applicationSupportRoot, allowMissing: true }),
  ]);
  if (legacyReceipt && currentReceipt) {
    if (currentReceipt.version !== "0.1.2") throw legacyError();
    return Object.freeze({
      state: "both_valid",
      legacy_receipt: legacyReceipt,
      current_receipt: currentReceipt,
    });
  }
  if (legacyReceipt) {
    return Object.freeze({
      state: "legacy_only",
      legacy_receipt: legacyReceipt,
      current_receipt: null,
    });
  }
  if (currentReceipt) {
    return Object.freeze({
      state: "current_only",
      legacy_receipt: null,
      current_receipt: currentReceipt,
    });
  }
  throw legacyError("install receipt set is invalid");
}

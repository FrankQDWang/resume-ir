import { createHash } from "node:crypto";
import { lstat, open, readFile, readdir, realpath } from "node:fs/promises";
import path from "node:path";

import {
  executablePayloadSha256,
  sha256,
} from "./verify-bundled-sidecar.mjs";

export const BUNDLE_COMPOSITION_FILE = "resume-ir.bundle-composition.v1.json";
export const BUNDLE_COMPOSITION_SCHEMA = "resume-ir.macos-bundle-composition.v1";

const EXPECTED_BUNDLE_ID = "local.resume-ir.desktop";
const EXPECTED_DISPLAY_NAME = "resume-ir";
const EXPECTED_ICON_FILE = "icon.icns";
const EXPECTED_MAIN_EXECUTABLE = "resume-desktop";
const SUPPORTED_TARGET = "aarch64-apple-darwin";
const MAX_INFO_PLIST_BYTES = 64 * 1024;
const MAX_EVIDENCE_BYTES = 16 * 1024;
const MAX_RUNTIME_MANIFEST_BYTES = 1024 * 1024;
const MAX_BOUND_FILE_BYTES = 512 * 1024 * 1024;
const MAX_RUNTIME_FILES = 128;
const DIGEST = /^[a-f0-9]{64}$/;
const VERSION = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;
const MIN_VERSION = [0, 1, 1];

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
  Object.freeze({
    role: "ocr",
    file: "ocr/runtime-pack/runtime-pack.json",
  }),
]);

function evidenceError(message = "bundle composition evidence is invalid") {
  return new Error(message);
}

function supportedVersion(version) {
  if (!VERSION.test(version ?? "")) return false;
  const parts = version.split(".").map(Number);
  if (parts.some((part) => !Number.isSafeInteger(part))) return false;
  for (let index = 0; index < parts.length; index += 1) {
    if (parts[index] !== MIN_VERSION[index]) {
      return parts[index] > MIN_VERSION[index];
    }
  }
  return true;
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value)) === JSON.stringify(expected)
  );
}

function digestBody(value) {
  return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}

function baseComposition(composition) {
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
  } = composition;
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

function validateEntry(entry, expected) {
  return (
    exactKeys(entry, ["role", "file", "sha256"]) &&
    entry.role === expected.role &&
    entry.file === expected.file &&
    DIGEST.test(entry.sha256)
  );
}

function validateCompositionShape(value) {
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
    value.schema_version !== BUNDLE_COMPOSITION_SCHEMA ||
    value.bundle_id !== EXPECTED_BUNDLE_ID ||
    !supportedVersion(value.version) ||
    value.target_triple !== SUPPORTED_TARGET ||
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
    value.icon.file !== EXPECTED_ICON_FILE ||
    !DIGEST.test(value.icon.sha256) ||
    !DIGEST.test(value.composition_digest) ||
    digestBody(baseComposition(value)) !== value.composition_digest
  ) {
    throw evidenceError();
  }
  return value;
}

async function requireBoundFile(file, message) {
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([lstat(file), realpath(file)]);
  } catch {
    throw evidenceError(message);
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    resolved !== path.resolve(file) ||
    metadata.size === 0 ||
    metadata.size > MAX_BOUND_FILE_BYTES
  ) {
    throw evidenceError(message);
  }
}

function safeRelativePath(file) {
  return (
    typeof file === "string" &&
    file.length > 0 &&
    file.length <= 256 &&
    file === file.replaceAll("\\", "/") &&
    !path.posix.isAbsolute(file) &&
    file.split("/").every((part) => part.length > 0 && part !== "." && part !== "..")
  );
}

async function recursiveFiles(root, relative = "") {
  let entries;
  try {
    entries = await readdir(path.join(root, relative), { withFileTypes: true });
  } catch {
    throw evidenceError("bundle composition runtime pack is invalid");
  }
  const files = [];
  for (const entry of entries) {
    const child = path.posix.join(relative, entry.name);
    if (entry.isSymbolicLink()) {
      throw evidenceError("bundle composition runtime pack is invalid");
    }
    if (entry.isDirectory()) {
      files.push(...(await recursiveFiles(root, child)));
    } else if (entry.isFile()) {
      files.push(child);
    } else {
      throw evidenceError("bundle composition runtime pack is invalid");
    }
  }
  return files.sort();
}

async function verifyRuntimePack(manifestFile) {
  await requireBoundFile(
    manifestFile,
    "bundle composition runtime manifest is invalid",
  );
  const metadata = await lstat(manifestFile);
  if (metadata.size > MAX_RUNTIME_MANIFEST_BYTES) {
    throw evidenceError("bundle composition runtime manifest is invalid");
  }
  let manifest;
  try {
    manifest = JSON.parse(await readFile(manifestFile, "utf8"));
  } catch {
    throw evidenceError("bundle composition runtime manifest is invalid");
  }
  if (
    manifest === null ||
    typeof manifest !== "object" ||
    Array.isArray(manifest) ||
    !Array.isArray(manifest.files) ||
    manifest.files.length === 0 ||
    manifest.files.length > MAX_RUNTIME_FILES
  ) {
    throw evidenceError("bundle composition runtime manifest is invalid");
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
      throw evidenceError("bundle composition runtime manifest is invalid");
    }
    files.add(entry.file);
    const payload = path.join(path.dirname(manifestFile), entry.file);
    await requireBoundFile(payload, "bundle composition runtime pack is invalid");
    const payloadMetadata = await lstat(payload);
    if (
      payloadMetadata.size !== entry.bytes ||
      (await sha256(payload)) !== entry.sha256
    ) {
      throw evidenceError("bundle composition runtime pack does not match manifest");
    }
  }
  const expectedFiles = ["runtime-pack.json", ...files].sort();
  if (
    JSON.stringify(await recursiveFiles(path.dirname(manifestFile))) !==
    JSON.stringify(expectedFiles)
  ) {
    throw evidenceError("bundle composition runtime pack is invalid");
  }
}

async function resolveAppBundle(appBundle) {
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([
      lstat(appBundle),
      realpath(appBundle),
    ]);
  } catch {
    throw evidenceError("bundle composition App is invalid");
  }
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw evidenceError("bundle composition App is invalid");
  }
  return resolved;
}

function plistString(source, field) {
  const escaped = field.replaceAll(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const matches = [
    ...source.matchAll(
      new RegExp(`<key>\\s*${escaped}\\s*<\\/key>\\s*<string>\\s*([^<]+?)\\s*<\\/string>`, "g"),
    ),
  ];
  if (matches.length !== 1) throw evidenceError("App identity evidence is invalid");
  return matches[0][1];
}

async function readIdentity(appBundle) {
  const infoPlist = path.join(appBundle, "Contents", "Info.plist");
  await requireBoundFile(infoPlist, "App identity evidence is invalid");
  const metadata = await lstat(infoPlist);
  if (metadata.size > MAX_INFO_PLIST_BYTES) {
    throw evidenceError("App identity evidence is invalid");
  }
  let source;
  try {
    source = await readFile(infoPlist, "utf8");
  } catch {
    throw evidenceError("App identity evidence is invalid");
  }
  const identity = {
    bundleId: plistString(source, "CFBundleIdentifier"),
    version: plistString(source, "CFBundleShortVersionString"),
    displayName: plistString(source, "CFBundleDisplayName"),
    iconFile: plistString(source, "CFBundleIconFile"),
    executable: plistString(source, "CFBundleExecutable"),
  };
  if (
    identity.bundleId !== EXPECTED_BUNDLE_ID ||
    !supportedVersion(identity.version) ||
    identity.displayName !== EXPECTED_DISPLAY_NAME ||
    identity.iconFile !== EXPECTED_ICON_FILE ||
    identity.executable !== EXPECTED_MAIN_EXECUTABLE
  ) {
    throw evidenceError("App identity evidence is invalid");
  }
  return identity;
}

function validateArguments(appBundle, targetTriple) {
  if (
    !path.isAbsolute(appBundle) ||
    targetTriple !== SUPPORTED_TARGET
  ) {
    throw evidenceError("bundle composition arguments are invalid");
  }
}

export async function createBundleComposition({ appBundle, targetTriple }) {
  validateArguments(appBundle, targetTriple);
  const resolvedAppBundle = await resolveAppBundle(appBundle);
  const identity = await readIdentity(resolvedAppBundle);
  const macos = path.join(resolvedAppBundle, "Contents", "MacOS");
  const resources = path.join(resolvedAppBundle, "Contents", "Resources");
  let macosEntries;
  try {
    macosEntries = (await readdir(macos)).sort();
  } catch {
    throw evidenceError("bundle composition native payload is invalid");
  }
  const expectedMacosEntries = EXECUTABLES.map(({ file }) => file).sort();
  if (JSON.stringify(macosEntries) !== JSON.stringify(expectedMacosEntries)) {
    throw evidenceError("bundle composition native payload is invalid");
  }
  const executables = [];
  for (const entry of EXECUTABLES) {
    const file = path.join(macos, entry.file);
    await requireBoundFile(file, "bundle composition native payload is invalid");
    let digest;
    try {
      digest = await executablePayloadSha256(file);
    } catch {
      throw evidenceError("bundle composition native payload is invalid");
    }
    executables.push({ ...entry, sha256: digest });
  }
  const runtimeManifests = [];
  for (const entry of RUNTIME_MANIFESTS) {
    const file = path.join(resources, entry.file);
    await verifyRuntimePack(file);
    runtimeManifests.push({ ...entry, sha256: await sha256(file) });
  }
  const iconPath = path.join(resources, EXPECTED_ICON_FILE);
  await requireBoundFile(iconPath, "bundle composition icon is invalid");
  const base = {
    schema_version: BUNDLE_COMPOSITION_SCHEMA,
    bundle_id: identity.bundleId,
    version: identity.version,
    target_triple: targetTriple,
    mach_o_digest: "sha256_without_code_signature_v1",
    file_digest: "sha256",
    executables,
    runtime_manifests: runtimeManifests,
    icon: { file: EXPECTED_ICON_FILE, sha256: await sha256(iconPath) },
  };
  return { ...base, composition_digest: digestBody(base) };
}

export async function writeBundleComposition({ appBundle, targetTriple }) {
  const composition = await createBundleComposition({ appBundle, targetTriple });
  const resolvedAppBundle = await resolveAppBundle(appBundle);
  const file = path.join(
    resolvedAppBundle,
    "Contents",
    "Resources",
    BUNDLE_COMPOSITION_FILE,
  );
  let handle;
  try {
    handle = await open(file, "wx", 0o444);
    await handle.writeFile(`${JSON.stringify(composition)}\n`, "utf8");
    await handle.sync();
  } catch {
    throw evidenceError("bundle composition evidence could not be created");
  } finally {
    await handle?.close().catch(() => {});
  }
  return composition;
}

export async function verifyBundleComposition({
  appBundle,
  targetTriple,
  expectedVersion,
}) {
  validateArguments(appBundle, targetTriple);
  const resolvedAppBundle = await resolveAppBundle(appBundle);
  const file = path.join(
    resolvedAppBundle,
    "Contents",
    "Resources",
    BUNDLE_COMPOSITION_FILE,
  );
  let metadata;
  try {
    metadata = await lstat(file);
  } catch {
    throw evidenceError("bundle composition evidence is unavailable");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > MAX_EVIDENCE_BYTES
  ) {
    throw evidenceError();
  }
  let source;
  let stored;
  try {
    source = await readFile(file, "utf8");
    stored = validateCompositionShape(JSON.parse(source));
  } catch (error) {
    if (error?.message === "bundle composition evidence is invalid") throw error;
    throw evidenceError();
  }
  if (`${JSON.stringify(stored)}\n` !== source) throw evidenceError();
  if (
    stored.target_triple !== targetTriple ||
    (expectedVersion !== undefined && stored.version !== expectedVersion)
  ) {
    throw evidenceError("bundle composition identity does not match");
  }
  const actual = await createBundleComposition({ appBundle, targetTriple });
  if (JSON.stringify(stored) !== JSON.stringify(actual)) {
    throw evidenceError("bundle composition payload does not match");
  }
  return stored;
}

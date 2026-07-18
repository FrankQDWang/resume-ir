import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { lstat, open, readFile, readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import os from "node:os";
import path from "node:path";

import { validateRuntimePackManifest } from "./prepare-sidecar.mjs";
import { verifyOcrResourcePack } from "./ocr-pack.mjs";

const APPLE_TARGETS = new Set(["aarch64-apple-darwin"]);
const SIDECARS = [
  "resume-daemon",
  "resume-embedding-runtime",
  "resume-pdf-render-runtime",
];
const LC_CODE_SIGNATURE = 0x1d;
const LC_SEGMENT_64 = 0x19;
const MAX_SIDECAR_BYTES = 256 * 1024 * 1024;
const MAX_LOAD_COMMANDS = 4096;

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

async function executablePayloadSha256(file) {
  const bytes = await readFile(file);
  if (
    bytes.length < 32 ||
    bytes.length > MAX_SIDECAR_BYTES ||
    bytes.readUInt32LE(0) !== 0xfeedfacf
  ) {
    throw new Error("bundled native component payload is invalid");
  }
  const commandCount = bytes.readUInt32LE(16);
  const commandBytes = bytes.readUInt32LE(20);
  const commandEnd = 32 + commandBytes;
  if (
    commandCount > MAX_LOAD_COMMANDS ||
    commandEnd > bytes.length ||
    commandCount * 8 > commandBytes
  ) {
    throw new Error("bundled native component payload is invalid");
  }
  let offset = 32;
  let signature;
  const linkeditCommands = [];
  for (let index = 0; index < commandCount; index += 1) {
    if (offset + 8 > commandEnd) {
      throw new Error("bundled native component payload is invalid");
    }
    const command = bytes.readUInt32LE(offset);
    const size = bytes.readUInt32LE(offset + 4);
    if (size < 8 || offset + size > commandEnd) {
      throw new Error("bundled native component payload is invalid");
    }
    if (command === LC_CODE_SIGNATURE) {
      if (signature || size !== 16) {
        throw new Error("bundled native component payload is invalid");
      }
      signature = {
        commandOffset: offset,
        dataOffset: bytes.readUInt32LE(offset + 8),
        dataSize: bytes.readUInt32LE(offset + 12),
      };
    }
    if (
      command === LC_SEGMENT_64 &&
      size >= 72 &&
      bytes.subarray(offset + 8, offset + 24).toString("utf8").replaceAll("\0", "") ===
        "__LINKEDIT"
    ) {
      linkeditCommands.push(offset);
    }
    offset += size;
  }
  if (offset !== commandEnd) {
    throw new Error("bundled native component payload is invalid");
  }
  if (!signature) return createHash("sha256").update(bytes).digest("hex");
  if (
    signature.dataSize === 0 ||
    signature.dataOffset < commandEnd ||
    signature.dataOffset + signature.dataSize !== bytes.length
  ) {
    throw new Error("bundled native component payload is invalid");
  }
  const unsignedPayload = Buffer.from(bytes.subarray(0, signature.dataOffset));
  unsignedPayload.writeUInt32LE(0, signature.commandOffset + 8);
  unsignedPayload.writeUInt32LE(0, signature.commandOffset + 12);
  for (const commandOffset of linkeditCommands) {
    unsignedPayload.writeBigUInt64LE(0n, commandOffset + 32);
    unsignedPayload.writeBigUInt64LE(0n, commandOffset + 48);
  }
  return createHash("sha256").update(unsignedPayload).digest("hex");
}

async function machOArchitecture(file) {
  const handle = await open(file, "r");
  try {
    const header = Buffer.alloc(8);
    const { bytesRead } = await handle.read(header, 0, header.length, 0);
    if (bytesRead !== header.length || header.readUInt32LE(0) !== 0xfeedfacf) {
      throw new Error("bundled native component is not a supported 64-bit Mach-O binary");
    }
    const cpuType = header.readUInt32LE(4);
    if (cpuType === 0x0100000c) return "arm64";
    if (cpuType === 0x01000007) return "x86_64";
    throw new Error("bundled native component Mach-O architecture is not supported");
  } finally {
    await handle.close();
  }
}

async function containsAnyMarker(file, prefixes) {
  const markers = [
    ...new Set(
      prefixes
        .filter((prefix) => typeof prefix === "string" && path.isAbsolute(prefix))
        .flatMap((prefix) => [prefix, prefix.replaceAll("\\", "/")]),
    ),
  ].map((prefix) => Buffer.from(prefix));
  const overlap = Math.max(0, ...markers.map((marker) => marker.length - 1));
  let tail = Buffer.alloc(0);
  for await (const chunk of createReadStream(file)) {
    const bytes = Buffer.concat([tail, chunk]);
    if (markers.some((marker) => bytes.indexOf(marker) !== -1)) return true;
    tail = overlap === 0 ? Buffer.alloc(0) : bytes.subarray(-overlap);
  }
  return false;
}

export async function verifyBundledSidecar({
  repoRoot,
  targetTriple,
  appBundle,
  expectedManifest = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "embedding",
    targetTriple,
    "runtime-pack.json",
  ),
  expectedOcrManifest = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "ocr",
    targetTriple,
    "runtime-pack.json",
  ),
  buildMachineIdentityPrefixes = [repoRoot, os.homedir()],
}) {
  if (!APPLE_TARGETS.has(targetTriple)) {
    throw new Error("bundle verification target is not supported");
  }
  if (!path.isAbsolute(repoRoot) || !path.isAbsolute(appBundle)) {
    throw new Error("bundle verification paths must be absolute");
  }

  const macosDirectory = path.join(appBundle, "Contents", "MacOS");
  const expectedArchitecture =
    targetTriple === "aarch64-apple-darwin" ? "arm64" : "x86_64";

  const macosEntries = await readdir(macosDirectory);
  for (const sidecarName of SIDECARS) {
    const matchingEntries = macosEntries.filter((entry) =>
      entry.startsWith(sidecarName),
    );
    if (matchingEntries.length !== 1 || matchingEntries[0] !== sidecarName) {
      throw new Error(`bundle must contain exactly one ${sidecarName} sidecar`);
    }
    const staged = path.join(
      repoRoot,
      "target",
      "tauri-sidecars",
      `${sidecarName}-${targetTriple}`,
    );
    const bundled = path.join(macosDirectory, sidecarName);
    let stagedMetadata;
    let bundledMetadata;
    try {
      [stagedMetadata, bundledMetadata] = await Promise.all([
        lstat(staged),
        lstat(bundled),
      ]);
    } catch {
      throw new Error(`required ${sidecarName} sidecar is missing from bundle composition`);
    }
    if (
      !stagedMetadata.isFile() ||
      !bundledMetadata.isFile() ||
      stagedMetadata.isSymbolicLink() ||
      bundledMetadata.isSymbolicLink() ||
      stagedMetadata.size === 0 ||
      bundledMetadata.size === 0 ||
      stagedMetadata.size > MAX_SIDECAR_BYTES ||
      bundledMetadata.size > MAX_SIDECAR_BYTES ||
      (stagedMetadata.mode & 0o111) === 0 ||
      (bundledMetadata.mode & 0o111) === 0
    ) {
      throw new Error(`${sidecarName} sidecar bundle composition is invalid`);
    }
    const [stagedDigest, bundledDigest] = await Promise.all([
      executablePayloadSha256(staged),
      executablePayloadSha256(bundled),
    ]);
    if (stagedDigest !== bundledDigest) {
      throw new Error(`bundled ${sidecarName} sidecar does not match the staged binary`);
    }
    if ((await machOArchitecture(bundled)) !== expectedArchitecture) {
      throw new Error(`${sidecarName} sidecar architecture does not match the target`);
    }
    if (await containsAnyMarker(bundled, buildMachineIdentityPrefixes)) {
      throw new Error(`${sidecarName} contains a build-machine identity path marker`);
    }
  }

  const stagedPack = path.join(
    repoRoot,
    "target",
    "tauri-resources",
    "embedding-runtime-pack",
  );
  const bundledPack = path.join(
    appBundle,
    "Contents",
    "Resources",
    "embedding",
    "runtime-pack",
  );
  let stagedPackMetadata;
  let bundledPackMetadata;
  try {
    [stagedPackMetadata, bundledPackMetadata] = await Promise.all([
      lstat(stagedPack),
      lstat(bundledPack),
    ]);
  } catch {
    throw new Error("required embedding resource directory is missing");
  }
  if (
    !stagedPackMetadata.isDirectory() ||
    !bundledPackMetadata.isDirectory() ||
    stagedPackMetadata.isSymbolicLink() ||
    bundledPackMetadata.isSymbolicLink()
  ) {
    throw new Error("embedding resource directory must be a regular non-symlink directory");
  }
  let expected;
  let stagedManifest;
  let bundledManifest;
  try {
    expected = validateRuntimePackManifest(
      JSON.parse(await readFile(expectedManifest, "utf8")),
    );
    stagedManifest = validateRuntimePackManifest(
      JSON.parse(await readFile(path.join(stagedPack, "runtime-pack.json"), "utf8")),
    );
    bundledManifest = validateRuntimePackManifest(
      JSON.parse(await readFile(path.join(bundledPack, "runtime-pack.json"), "utf8")),
    );
  } catch {
    throw new Error("required embedding resource manifest is missing or invalid");
  }
  const expectedJson = JSON.stringify(expected);
  if (
    JSON.stringify(stagedManifest) !== expectedJson ||
    JSON.stringify(bundledManifest) !== expectedJson
  ) {
    throw new Error("embedding resource manifest does not match reviewed composition");
  }
  const expectedEntries = ["runtime-pack.json", ...expected.files.map(({ file }) => file)].sort();
  const [stagedEntries, bundledEntries] = await Promise.all([
    readdir(stagedPack),
    readdir(bundledPack),
  ]);
  if (
    JSON.stringify(stagedEntries.sort()) !== JSON.stringify(expectedEntries) ||
    JSON.stringify(bundledEntries.sort()) !== JSON.stringify(expectedEntries)
  ) {
    throw new Error("embedding resource pack must contain exactly the reviewed files");
  }
  let resourceBytes = 0;
  for (const entry of expected.files) {
    const stagedFile = path.join(stagedPack, entry.file);
    const bundledFile = path.join(bundledPack, entry.file);
    const [stagedMetadata, bundledMetadata] = await Promise.all([
      lstat(stagedFile),
      lstat(bundledFile),
    ]);
    if (
      !stagedMetadata.isFile() ||
      !bundledMetadata.isFile() ||
      stagedMetadata.isSymbolicLink() ||
      bundledMetadata.isSymbolicLink() ||
      stagedMetadata.size !== entry.bytes ||
      bundledMetadata.size !== entry.bytes
    ) {
      throw new Error(`embedding resource ${entry.role} bundle composition is invalid`);
    }
    const [stagedDigest, bundledDigest] = await Promise.all([
      sha256(stagedFile),
      sha256(bundledFile),
    ]);
    if (stagedDigest !== entry.sha256 || bundledDigest !== entry.sha256) {
      throw new Error(`embedding resource ${entry.role} does not match reviewed bytes`);
    }
    if (
      entry.role === "runtime_library" &&
      (await machOArchitecture(bundledFile)) !== expectedArchitecture
    ) {
      throw new Error("embedding runtime library architecture does not match the target");
    }
    if (await containsAnyMarker(bundledFile, buildMachineIdentityPrefixes)) {
      throw new Error(`embedding resource ${entry.role} contains a build-machine identity path marker`);
    }
    resourceBytes += bundledMetadata.size;
  }
  if (
    await containsAnyMarker(
      path.join(bundledPack, "runtime-pack.json"),
      buildMachineIdentityPrefixes,
    )
  ) {
    throw new Error("embedding resource manifest contains a build-machine identity path marker");
  }

  const stagedOcrPack = path.join(
    repoRoot,
    "target",
    "tauri-resources",
    "ocr-runtime-pack",
  );
  const bundledOcrPack = path.join(
    appBundle,
    "Contents",
    "Resources",
    "ocr",
    "runtime-pack",
  );
  let stagedOcrManifest;
  let bundledOcrManifest;
  try {
    [stagedOcrManifest, bundledOcrManifest] = await Promise.all([
      verifyOcrResourcePack({
        directory: stagedOcrPack,
        expectedManifest: expectedOcrManifest,
      }),
      verifyOcrResourcePack({
        directory: bundledOcrPack,
        expectedManifest: expectedOcrManifest,
      }),
    ]);
  } catch {
    throw new Error("required OCR resource pack is missing or invalid");
  }
  if (JSON.stringify(stagedOcrManifest) !== JSON.stringify(bundledOcrManifest)) {
    throw new Error("OCR resource manifests do not match");
  }
  let ocrResourceBytes = 0;
  for (const entry of bundledOcrManifest.files) {
    const bundledFile = path.join(bundledOcrPack, entry.file);
    const metadata = await lstat(bundledFile);
    if (entry.executable && (metadata.mode & 0o111) === 0) {
      throw new Error("bundled OCR engine is not executable");
    }
    if (await containsAnyMarker(bundledFile, buildMachineIdentityPrefixes)) {
      throw new Error(`OCR resource ${entry.role} contains a build-machine identity path marker`);
    }
    ocrResourceBytes += metadata.size;
  }
  if (
    await containsAnyMarker(
      path.join(bundledOcrPack, "runtime-pack.json"),
      buildMachineIdentityPrefixes,
    )
  ) {
    throw new Error("OCR resource manifest contains a build-machine identity path marker");
  }

  return {
    schema_version: "resume-ir.desktop-bundle-composition.v1",
    target_triple: targetTriple,
    daemon_sidecar_count: 1,
    embedding_sidecar_count: 1,
    pdf_renderer_sidecar_count: 1,
    embedding_resource_file_count: expectedEntries.length,
    embedding_resource_bytes: resourceBytes,
    ocr_resource_file_count: bundledOcrManifest.files.length + 1,
    ocr_resource_bytes: ocrResourceBytes,
    digest_match: true,
    executable: true,
    architecture: expectedArchitecture,
    path_scan_scope: "repo_root_and_builder_home",
    build_machine_identity_path_markers: 0,
  };
}

function parseArguments(args) {
  const values = new Map();
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (!["--target", "--app-bundle"].includes(key) || !value) {
      throw new Error("invalid bundle verification arguments");
    }
    values.set(key, value);
  }
  return {
    appBundle: values.get("--app-bundle"),
    targetTriple: values.get("--target"),
  };
}

async function main() {
  const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
  const arguments_ = parseArguments(process.argv.slice(2));
  const receipt = await verifyBundledSidecar({ repoRoot, ...arguments_ });
  console.log(JSON.stringify(receipt));
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    console.error(`verify-bundled-sidecar: ${error.message}`);
    process.exitCode = 1;
  });
}

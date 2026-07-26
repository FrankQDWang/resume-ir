import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  open,
  readFile,
  readdir,
  rename,
  rm,
} from "node:fs/promises";
import path from "node:path";

const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const ROLE_COUNTS = new Map([
  ["engine_binary", 1],
  ["engine_library", 15],
  ["language_eng", 1],
  ["language_chi_sim", 1],
  ["engine_config", 1],
  ["license_text", 10],
  ["third_party_notice", 1],
]);

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

function safeRelativePath(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 256 &&
    value === value.replaceAll("\\", "/") &&
    !path.posix.isAbsolute(value) &&
    value.split("/").every((part) => part.length > 0 && part !== "." && part !== "..")
  );
}

export function validateOcrRuntimePackManifest(manifest) {
  if (
    !manifest ||
    manifest.schema_version !== "resume-ir.desktop-ocr-runtime-pack.v1" ||
    manifest.runtime_pack_id !==
      "tesseract-5.5.2-tessdata-fast-4.1.0-macos-arm64-r1" ||
    manifest.target_triple !== "aarch64-apple-darwin" ||
    manifest.engine !== "tesseract" ||
    manifest.engine_version !== "5.5.2" ||
    manifest.renderer !== "macos-pdfkit-coregraphics" ||
    JSON.stringify(manifest.languages) !== JSON.stringify(["eng", "chi_sim"]) ||
    manifest.network_access !== "disabled" ||
    manifest.license_reviewed !== true ||
    manifest.third_party_notice !== "THIRD-PARTY-NOTICES.json" ||
    !Array.isArray(manifest.files) ||
    manifest.files.length !== [...ROLE_COUNTS.values()].reduce((sum, count) => sum + count, 0)
  ) {
    throw new Error("OCR runtime manifest contract is invalid");
  }
  const roles = new Map();
  const files = new Set();
  for (const entry of manifest.files) {
    if (
      !entry ||
      !ROLE_COUNTS.has(entry.role) ||
      !safeRelativePath(entry.file) ||
      files.has(entry.file) ||
      !Number.isSafeInteger(entry.bytes) ||
      entry.bytes <= 0 ||
      !SHA256_PATTERN.test(entry.sha256) ||
      typeof entry.executable !== "boolean"
    ) {
      throw new Error("OCR runtime manifest file contract is invalid");
    }
    files.add(entry.file);
    roles.set(entry.role, (roles.get(entry.role) ?? 0) + 1);
  }
  for (const [role, count] of ROLE_COUNTS) {
    if (roles.get(role) !== count) {
      throw new Error("OCR runtime manifest role set is incomplete");
    }
  }
  const exactRoleFiles = new Map([
    ["engine_binary", "tesseract"],
    ["language_eng", "tessdata/eng.traineddata"],
    ["language_chi_sim", "tessdata/chi_sim.traineddata"],
    ["engine_config", "tessdata/configs/tsv"],
    ["third_party_notice", "THIRD-PARTY-NOTICES.json"],
  ]);
  for (const [role, file] of exactRoleFiles) {
    const entry = manifest.files.find((candidate) => candidate.role === role);
    if (entry?.file !== file || entry.executable !== (role === "engine_binary")) {
      throw new Error("OCR runtime manifest fixed file contract is invalid");
    }
  }
  if (
    manifest.files.some((entry) =>
      entry.role === "engine_library"
        ? !entry.file.startsWith("lib/") || !entry.file.endsWith(".dylib") || entry.executable
        : entry.role === "license_text"
          ? !entry.file.startsWith("LICENSES/") || !entry.file.endsWith(".txt") || entry.executable
          : false,
    )
  ) {
    throw new Error("OCR runtime manifest scoped file contract is invalid");
  }
  return manifest;
}

async function directRegularFile(file, label) {
  let metadata;
  try {
    metadata = await lstat(file);
  } catch {
    throw new Error(`${label} is missing`);
  }
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    throw new Error(`${label} must be a regular non-symlink file`);
  }
  return metadata;
}

async function arm64MachO(file) {
  const handle = await open(file, "r");
  try {
    const header = Buffer.alloc(8);
    const { bytesRead } = await handle.read(header, 0, header.length, 0);
    return (
      bytesRead === header.length &&
      header.readUInt32LE(0) === 0xfeedfacf &&
      header.readUInt32LE(4) === 0x0100000c
    );
  } finally {
    await handle.close();
  }
}

async function recursiveEntries(root, relative = "") {
  const entries = await readdir(path.join(root, relative), { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const child = path.posix.join(relative, entry.name);
    if (entry.isSymbolicLink()) throw new Error("OCR resource pack must not contain symlinks");
    if (entry.isDirectory()) files.push(...(await recursiveEntries(root, child)));
    else if (entry.isFile()) files.push(child);
    else throw new Error("OCR resource pack contains an unsupported entry");
  }
  return files.sort();
}

async function validatedSourcePack(sourceRoot, expectedManifest) {
  const rootMetadata = await lstat(sourceRoot).catch(() => undefined);
  if (!rootMetadata?.isDirectory() || rootMetadata.isSymbolicLink()) {
    throw new Error("OCR resource source must be a regular directory");
  }
  await directRegularFile(expectedManifest, "expected OCR manifest");
  const sourceManifestPath = path.join(sourceRoot, "runtime-pack.json");
  await directRegularFile(sourceManifestPath, "source OCR manifest");
  let expected;
  let source;
  try {
    expected = validateOcrRuntimePackManifest(
      JSON.parse(await readFile(expectedManifest, "utf8")),
    );
    source = validateOcrRuntimePackManifest(
      JSON.parse(await readFile(sourceManifestPath, "utf8")),
    );
  } catch (error) {
    if (error instanceof SyntaxError) throw new Error("OCR runtime manifest is not valid JSON");
    throw error;
  }
  if (JSON.stringify(source) !== JSON.stringify(expected)) {
    throw new Error("OCR runtime source does not match reviewed manifest");
  }
  const expectedEntries = ["runtime-pack.json", ...expected.files.map(({ file }) => file)].sort();
  if (JSON.stringify(await recursiveEntries(sourceRoot)) !== JSON.stringify(expectedEntries)) {
    throw new Error("OCR runtime source must contain exactly the reviewed files");
  }
  for (const entry of expected.files) {
    const sourceFile = path.join(sourceRoot, entry.file);
    const metadata = await directRegularFile(sourceFile, `OCR resource ${entry.role}`);
    if (metadata.size !== entry.bytes || (await sha256(sourceFile)) !== entry.sha256) {
      throw new Error(`OCR resource ${entry.role} does not match manifest`);
    }
    if (
      ["engine_binary", "engine_library"].includes(entry.role) &&
      !(await arm64MachO(sourceFile))
    ) {
      throw new Error(`OCR resource ${entry.role} architecture is invalid`);
    }
  }
  return expected;
}

export async function stageOcrResourcePack(plan) {
  const expected = await validatedSourcePack(plan.sourcePackRoot, plan.expectedManifest);
  const parent = path.dirname(plan.destination);
  const temporary = path.join(parent, `${path.basename(plan.destination)}.tmp-${process.pid}-${Date.now()}`);
  const backup = path.join(parent, `${path.basename(plan.destination)}.old-${process.pid}-${Date.now()}`);
  await mkdir(parent, { recursive: true });
  await rm(temporary, { recursive: true, force: true });
  await mkdir(temporary, { mode: 0o700 });
  try {
    await copyFile(plan.expectedManifest, path.join(temporary, "runtime-pack.json"));
    await chmod(path.join(temporary, "runtime-pack.json"), 0o644);
    for (const entry of expected.files) {
      const destination = path.join(temporary, entry.file);
      await mkdir(path.dirname(destination), { recursive: true });
      await copyFile(path.join(plan.sourcePackRoot, entry.file), destination);
      await chmod(destination, entry.executable ? 0o755 : 0o644);
    }
    await validatedSourcePack(temporary, plan.expectedManifest);
    let previous = false;
    try {
      await rename(plan.destination, backup);
      previous = true;
    } catch (error) {
      if (!error || error.code !== "ENOENT") throw error;
    }
    try {
      await rename(temporary, plan.destination);
    } catch (error) {
      if (previous) await rename(backup, plan.destination);
      throw error;
    }
    await rm(backup, { recursive: true, force: true });
  } finally {
    await rm(temporary, { recursive: true, force: true });
    await rm(backup, { recursive: true, force: true });
  }
  return {
    schema_version: "resume-ir.ocr-resource-stage.v1",
    target_triple: plan.targetTriple,
    resource_file_count: expected.files.length + 1,
  };
}

export async function verifyOcrResourcePack({ directory, expectedManifest }) {
  return validatedSourcePack(directory, expectedManifest);
}

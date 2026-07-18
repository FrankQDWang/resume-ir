import { createHash } from "node:crypto";
import { createReadStream, lstatSync, readFileSync } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  readFile,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

import { inspectWindowsPeDynamicLibrary } from "./windows-pe.mjs";

const TARGET = "x86_64-pc-windows-msvc";
const CONTRACT_SCHEMA = "resume-ir.windows-embedding-source-contract.v1";
const PACK_SCHEMA = "resume-ir.embedding-runtime-pack.v1";
const PACK_ID = "intfloat-multilingual-e5-small-qint8-r1";
const SOURCE_REPOSITORY = "https://github.com/microsoft/onnxruntime";
const SOURCE_TAG = "v1.24.4";
const SOURCE_COMMIT = "2d924974ef147392ced8409d36bd6d2e7fcc8a74";
const BUILD_ARGUMENTS = [
  "--config",
  "Release",
  "--build_shared_lib",
  "--enable_msvc_static_runtime",
  "--skip_submodule_sync",
  "--parallel",
  "1",
  "--update",
  "--build",
  "--test",
];
const FORBIDDEN_IMPORT_PREFIXES = [
  "MSVCP",
  "VCRUNTIME",
  "CONCRT",
  "UCRTBASE",
  "API-MS-WIN-CRT-",
  "ONNXRUNTIME_PROVIDERS_",
];
const SYSTEM_IMPORTS = new Set([
  "ADVAPI32.DLL",
  "API-MS-WIN-CORE-PATH-L1-1-0.DLL",
  "BCRYPT.DLL",
  "COMBASE.DLL",
  "CRYPT32.DLL",
  "DBGHELP.DLL",
  "DWRITE.DLL",
  "DXCORE.DLL",
  "DXGI.DLL",
  "GDI32.DLL",
  "IPHLPAPI.DLL",
  "KERNEL32.DLL",
  "NORMALIZ.DLL",
  "NTDLL.DLL",
  "OLE32.DLL",
  "OLEAUT32.DLL",
  "POWRPROF.DLL",
  "PSAPI.DLL",
  "RPCRT4.DLL",
  "SETUPAPI.DLL",
  "SHELL32.DLL",
  "SHLWAPI.DLL",
  "USER32.DLL",
  "VERSION.DLL",
  "WINHTTP.DLL",
  "WINMM.DLL",
  "WS2_32.DLL",
]);
const MODEL_ROLES = new Set([
  "model",
  "tokenizer",
  "model_config",
  "special_tokens_map",
  "tokenizer_config",
]);
const SHA256 = /^[a-f0-9]{64}$/;

function exactKeys(value, keys) {
  return (
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...keys].sort())
  );
}

function sameArray(left, right) {
  return Array.isArray(left) && JSON.stringify(left) === JSON.stringify(right);
}

function validFileIdentity(value, file, maxBytes) {
  return (
    exactKeys(value, ["file", "bytes", "sha256"]) &&
    value.file === file &&
    Number.isSafeInteger(value.bytes) &&
    value.bytes > 0 &&
    value.bytes <= maxBytes &&
    SHA256.test(value.sha256)
  );
}

export function validateWindowsEmbeddingSourceContract(contract) {
  const runtime = contract?.onnxruntime;
  const model = contract?.model_assets;
  const prebuilt = runtime?.official_prebuilt_observation;
  if (
    !exactKeys(contract, [
      "schema_version",
      "target_triple",
      "runtime_pack_schema",
      "runtime_pack_id",
      "product_runtime_network_access",
      "onnxruntime",
      "model_assets",
    ]) ||
    contract.schema_version !== CONTRACT_SCHEMA ||
    contract.target_triple !== TARGET ||
    contract.runtime_pack_schema !== PACK_SCHEMA ||
    contract.runtime_pack_id !== PACK_ID ||
    contract.product_runtime_network_access !== "disabled" ||
    !exactKeys(runtime, [
      "version",
      "api_version",
      "source_repository",
      "source_tag",
      "source_commit",
      "license",
      "provenance_schema",
      "build_arguments",
      "provider",
      "telemetry",
      "required_export",
      "forbidden_import_prefixes",
      "source_license_file",
      "source_notices_file",
      "official_prebuilt_observation",
    ]) ||
    runtime.version !== "1.24.4" ||
    runtime.api_version !== 24 ||
    runtime.source_repository !== SOURCE_REPOSITORY ||
    runtime.source_tag !== SOURCE_TAG ||
    runtime.source_commit !== SOURCE_COMMIT ||
    runtime.license !== "MIT" ||
    runtime.provenance_schema !== "resume-ir.onnxruntime-windows-build-provenance.v2" ||
    !sameArray(runtime.build_arguments, BUILD_ARGUMENTS) ||
    runtime.provider !== "cpu" ||
    runtime.telemetry !== false ||
    runtime.required_export !== "OrtGetApiBase" ||
    !sameArray(runtime.forbidden_import_prefixes, FORBIDDEN_IMPORT_PREFIXES) ||
    !validFileIdentity(runtime.source_license_file, "LICENSE", 64 * 1024) ||
    !validFileIdentity(
      runtime.source_notices_file,
      "ThirdPartyNotices.txt",
      4 * 1024 * 1024,
    ) ||
    !exactKeys(prebuilt, [
      "asset_name",
      "bytes",
      "sha256",
      "imports_dynamic_msvc_runtime",
      "accepted_as_self_contained",
    ]) ||
    prebuilt.asset_name !== "onnxruntime-win-x64-1.24.4.zip" ||
    prebuilt.bytes !== 74_442_783 ||
    prebuilt.sha256 !==
      "d2319fddfb6ea4db99ccc4b60c85c517bcd855721f5daa6a06d40d7cb2ee2357" ||
    prebuilt.imports_dynamic_msvc_runtime !== true ||
    prebuilt.accepted_as_self_contained !== false ||
    !exactKeys(model, [
      "source_manifest_schema",
      "source_runtime_pack_id",
      "source_manifest_sha256",
      "model_id",
      "upstream_model_id",
      "upstream_revision",
      "upstream_model_file",
      "quantization",
      "dimension",
      "license",
    ]) ||
    model.source_manifest_schema !== PACK_SCHEMA ||
    model.source_runtime_pack_id !== PACK_ID ||
    !SHA256.test(model.source_manifest_sha256) ||
    model.model_id !== PACK_ID ||
    model.upstream_model_id !== "intfloat/multilingual-e5-small" ||
    model.upstream_revision !== "614241f622f53c4eeff9890bdc4f31cfecc418b3" ||
    model.upstream_model_file !== "onnx/model_qint8_avx512_vnni.onnx" ||
    model.quantization !== "dynamic_int8" ||
    model.dimension !== 384 ||
    model.license !== "MIT"
  ) {
    throw new Error("Windows embedding source contract is invalid");
  }
  return contract;
}

export function readWindowsEmbeddingSourceContract(file) {
  if (!path.isAbsolute(file)) throw new Error("Windows embedding source contract path is invalid");
  let metadata;
  try {
    metadata = lstatSync(file);
  } catch {
    throw new Error("Windows embedding source contract is missing");
  }
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > 64 * 1024) {
    throw new Error("Windows embedding source contract file is invalid");
  }
  try {
    return validateWindowsEmbeddingSourceContract(JSON.parse(readFileSync(file, "utf8")));
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("Windows embedding source contract is not valid JSON");
    }
    throw error;
  }
}

export function inspectWindowsPeImage(buffer) {
  return inspectWindowsPeDynamicLibrary(buffer);
}

async function directFile(file, label, maxBytes) {
  let metadata;
  try {
    metadata = await lstat(file);
  } catch {
    throw new Error(`${label} is missing`);
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > maxBytes
  ) {
    throw new Error(`${label} must be a bounded regular non-symlink file`);
  }
  return metadata;
}

async function directDirectory(directory, label) {
  let metadata;
  try {
    metadata = await lstat(directory);
  } catch {
    throw new Error(`${label} is missing`);
  }
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw new Error(`${label} must be a regular directory`);
  }
}

async function sha256File(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

function validSystemImport(name) {
  return SYSTEM_IMPORTS.has(name);
}

export async function validateWindowsEmbeddingRuntimeArtifact({ runtimeRoot, contract }) {
  validateWindowsEmbeddingSourceContract(contract);
  if (!path.isAbsolute(runtimeRoot)) throw new Error("Windows embedding runtime root is invalid");
  await directDirectory(runtimeRoot, "Windows embedding runtime root");
  const dll = path.join(runtimeRoot, "onnxruntime.dll");
  const license = path.join(runtimeRoot, contract.onnxruntime.source_license_file.file);
  const notices = path.join(runtimeRoot, contract.onnxruntime.source_notices_file.file);
  const provenanceFile = path.join(runtimeRoot, "build-provenance.json");
  const dllMetadata = await directFile(dll, "Windows embedding runtime DLL", 256 * 1024 * 1024);
  await directFile(license, "Windows embedding runtime license", 64 * 1024);
  await directFile(notices, "Windows embedding runtime notices", 4 * 1024 * 1024);
  await directFile(provenanceFile, "Windows embedding build provenance", 64 * 1024);
  const image = inspectWindowsPeImage(await readFile(dll));
  if (!image.exports.includes(contract.onnxruntime.required_export)) {
    throw new Error("Windows embedding runtime required export is missing");
  }
  for (const imported of image.imports) {
    if (
      contract.onnxruntime.forbidden_import_prefixes.some((prefix) =>
        imported.startsWith(prefix),
      ) ||
      !validSystemImport(imported)
    ) {
      throw new Error("Windows embedding runtime dependency closure is not self-contained");
    }
  }
  for (const [file, identity, label] of [
    [license, contract.onnxruntime.source_license_file, "license"],
    [notices, contract.onnxruntime.source_notices_file, "notices"],
  ]) {
    const metadata = await lstat(file);
    if (metadata.size !== identity.bytes || (await sha256File(file)) !== identity.sha256) {
      throw new Error(`Windows embedding runtime ${label} does not match source contract`);
    }
  }
  let provenance;
  try {
    provenance = JSON.parse(await readFile(provenanceFile, "utf8"));
  } catch {
    throw new Error("Windows embedding build provenance is invalid");
  }
  const dllSha256 = await sha256File(dll);
  if (
    !exactKeys(provenance, [
      "schema_version",
      "target_triple",
      "source_repository",
      "source_tag",
      "source_commit",
      "version",
      "api_version",
      "build_arguments",
      "provider",
      "telemetry",
      "source_tree_clean",
      "builder_platform",
      "builder_architecture",
      "python_version",
      "visual_studio_version",
      "msvc_toolset_version",
      "windows_sdk_version",
      "cmake_version",
      "tests_passed",
      "artifact_file",
      "artifact_bytes",
      "artifact_sha256",
    ]) ||
    provenance.schema_version !== contract.onnxruntime.provenance_schema ||
    provenance.target_triple !== TARGET ||
    provenance.source_repository !== SOURCE_REPOSITORY ||
    provenance.source_tag !== SOURCE_TAG ||
    provenance.source_commit !== SOURCE_COMMIT ||
    provenance.version !== contract.onnxruntime.version ||
    provenance.api_version !== contract.onnxruntime.api_version ||
    !sameArray(provenance.build_arguments, BUILD_ARGUMENTS) ||
    provenance.provider !== "cpu" ||
    provenance.telemetry !== false ||
    provenance.source_tree_clean !== true ||
    provenance.builder_platform !== "windows" ||
    provenance.builder_architecture !== "x86_64" ||
    ![
      provenance.python_version,
      provenance.visual_studio_version,
      provenance.msvc_toolset_version,
      provenance.windows_sdk_version,
      provenance.cmake_version,
    ].every(
      (value) =>
        typeof value === "string" && /^\d+(?:\.\d+){1,4}$/.test(value),
    ) ||
    provenance.tests_passed !== true ||
    provenance.artifact_file !== "onnxruntime.dll" ||
    provenance.artifact_bytes !== dllMetadata.size ||
    provenance.artifact_sha256 !== dllSha256
  ) {
    throw new Error("Windows embedding build provenance does not match artifact");
  }
  return Object.freeze({ dll, dllBytes: dllMetadata.size, dllSha256, image, license, notices, provenance });
}

function validateModelManifest(manifest, contract) {
  if (
    manifest?.schema_version !== PACK_SCHEMA ||
    manifest.runtime_pack_id !== PACK_ID ||
    manifest.model_id !== contract.model_assets.model_id ||
    manifest.upstream_model_id !== contract.model_assets.upstream_model_id ||
    manifest.upstream_revision !== contract.model_assets.upstream_revision ||
    manifest.upstream_model_file !== contract.model_assets.upstream_model_file ||
    manifest.quantization !== contract.model_assets.quantization ||
    manifest.dimension !== contract.model_assets.dimension ||
    manifest.provider !== "cpu" ||
    manifest.network_access !== "disabled" ||
    manifest.license_reviewed !== true ||
    manifest.model_license !== contract.model_assets.license ||
    manifest.onnxruntime_license !== "MIT" ||
    !Array.isArray(manifest.files) ||
    manifest.files.length !== 6 ||
    manifest.files.filter(({ role }) => role === "runtime_library").length !== 1
  ) {
    throw new Error("Windows embedding model source manifest is invalid");
  }
  const entries = manifest.files.filter(({ role }) => MODEL_ROLES.has(role));
  if (
    entries.length !== MODEL_ROLES.size ||
    new Set(entries.map(({ role }) => role)).size !== MODEL_ROLES.size ||
    new Set(entries.map(({ file }) => file)).size !== MODEL_ROLES.size ||
    entries.some(
      (entry) =>
        !exactKeys(entry, ["role", "file", "bytes", "sha256"]) ||
        path.basename(entry.file) !== entry.file ||
        !Number.isSafeInteger(entry.bytes) ||
        entry.bytes <= 0 ||
        !SHA256.test(entry.sha256),
    )
  ) {
    throw new Error("Windows embedding model asset manifest is invalid");
  }
  return entries;
}

export async function assembleWindowsEmbeddingPack({
  contractFile,
  runtimeRoot,
  modelPackRoot,
  destination,
}) {
  if (
    ![contractFile, runtimeRoot, modelPackRoot, destination].every(
      (value) => typeof value === "string" && path.isAbsolute(value),
    )
  ) {
    throw new Error("Windows embedding assembly paths must be absolute");
  }
  const containsPath = (parent, child) => {
    const relative = path.relative(path.resolve(parent), path.resolve(child));
    return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
  };
  if (
    containsPath(runtimeRoot, destination) ||
    containsPath(destination, runtimeRoot) ||
    containsPath(modelPackRoot, destination) ||
    containsPath(destination, modelPackRoot) ||
    containsPath(destination, contractFile)
  ) {
    throw new Error("Windows embedding assembly inputs and destination must not overlap");
  }
  const contract = readWindowsEmbeddingSourceContract(contractFile);
  const runtime = await validateWindowsEmbeddingRuntimeArtifact({ runtimeRoot, contract });
  await directDirectory(modelPackRoot, "Windows embedding model pack root");
  const modelManifestFile = path.join(modelPackRoot, "runtime-pack.json");
  await directFile(modelManifestFile, "Windows embedding model source manifest", 64 * 1024);
  if ((await sha256File(modelManifestFile)) !== contract.model_assets.source_manifest_sha256) {
    throw new Error("Windows embedding model source manifest is not the reviewed input");
  }
  let modelManifest;
  try {
    modelManifest = JSON.parse(await readFile(modelManifestFile, "utf8"));
  } catch {
    throw new Error("Windows embedding model source manifest is invalid");
  }
  const modelEntries = validateModelManifest(modelManifest, contract);
  for (const entry of modelEntries) {
    const file = path.join(modelPackRoot, entry.file);
    const metadata = await directFile(file, `Windows embedding model asset ${entry.role}`, 256 * 1024 * 1024);
    if (metadata.size !== entry.bytes || (await sha256File(file)) !== entry.sha256) {
      throw new Error(`Windows embedding model asset ${entry.role} does not match manifest`);
    }
  }
  const manifest = {
    ...Object.fromEntries(
      Object.entries(modelManifest).filter(([key]) => key !== "files"),
    ),
    files: [
      {
        role: "runtime_library",
        file: "onnxruntime.dll",
        bytes: runtime.dllBytes,
        sha256: runtime.dllSha256,
      },
      ...modelEntries,
    ],
  };
  const parent = path.dirname(destination);
  const temporary = path.join(parent, `${path.basename(destination)}.tmp-${process.pid}-${Date.now()}`);
  const backup = path.join(parent, `${path.basename(destination)}.old-${process.pid}-${Date.now()}`);
  await mkdir(parent, { recursive: true });
  await rm(temporary, { recursive: true, force: true });
  await mkdir(temporary, { mode: 0o700 });
  try {
    await copyFile(runtime.dll, path.join(temporary, "onnxruntime.dll"));
    for (const entry of modelEntries) {
      await copyFile(path.join(modelPackRoot, entry.file), path.join(temporary, entry.file));
    }
    await copyFile(runtime.license, path.join(temporary, "ONNXRUNTIME-LICENSE.txt"));
    await copyFile(
      runtime.notices,
      path.join(temporary, "ONNXRUNTIME-THIRD-PARTY-NOTICES.txt"),
    );
    await writeFile(
      path.join(temporary, "build-provenance.json"),
      `${JSON.stringify(runtime.provenance, null, 2)}\n`,
    );
    await writeFile(
      path.join(temporary, "source-contract.json"),
      `${JSON.stringify(contract, null, 2)}\n`,
    );
    await writeFile(
      path.join(temporary, "runtime-pack.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
    );
    for (const file of [
      "onnxruntime.dll",
      ...modelEntries.map(({ file }) => file),
      "ONNXRUNTIME-LICENSE.txt",
      "ONNXRUNTIME-THIRD-PARTY-NOTICES.txt",
      "build-provenance.json",
      "source-contract.json",
      "runtime-pack.json",
    ]) {
      await chmod(path.join(temporary, file), 0o644);
    }
    let previous = false;
    try {
      await rename(destination, backup);
      previous = true;
    } catch (error) {
      if (!error || error.code !== "ENOENT") throw error;
    }
    try {
      await rename(temporary, destination);
    } catch (error) {
      if (previous) await rename(backup, destination);
      throw error;
    }
    await rm(backup, { recursive: true, force: true });
  } finally {
    await rm(temporary, { recursive: true, force: true });
    await rm(backup, { recursive: true, force: true });
  }
  return Object.freeze({
    schema_version: "resume-ir.windows-embedding-pack-assembly.v1",
    target_triple: TARGET,
    resource_file_count: modelEntries.length + 6,
    runtime_import_count: runtime.image.imports.length,
  });
}

function parseArguments(args) {
  const values = {};
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (!value || !["--runtime-root", "--model-pack-root", "--destination"].includes(key)) {
      throw new Error("Windows embedding assembly arguments are invalid");
    }
    values[key.slice(2).replaceAll("-", "_")] = value;
  }
  if (Object.keys(values).length !== 3) {
    throw new Error("Windows embedding assembly arguments are incomplete");
  }
  return values;
}

async function main() {
  const args = parseArguments(process.argv.slice(2));
  const contractFile = fileURLToPath(
    new URL("../resources/embedding/x86_64-pc-windows-msvc/source-contract.json", import.meta.url),
  );
  const result = await assembleWindowsEmbeddingPack({
    contractFile,
    runtimeRoot: args.runtime_root,
    modelPackRoot: args.model_pack_root,
    destination: args.destination,
  });
  console.log(
    `assembled reviewed Windows embedding pack (${result.resource_file_count} files)`,
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`windows-embedding-pack: ${error.message}`);
    process.exitCode = 1;
  });
}

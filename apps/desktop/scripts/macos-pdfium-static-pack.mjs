import { createHash } from "node:crypto";
import { createReadStream, lstatSync, readFileSync } from "node:fs";
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
  stat,
  writeFile,
} from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const TARGET = "aarch64-apple-darwin";
const CONTRACT_SCHEMA = "resume-ir.macos-pdfium-source-contract.v1";
const PACK_SCHEMA = "resume-ir.pdfium-static-build-pack.v1";
const INSTALLED_PACK_SCHEMA = "resume-ir.pdfium-static-runtime-pack.v1";
const PDFIUM_COMMIT = "91b9d569b34be4f38eed7b3c49b227356c3aadad";
const BUILD_REVISION = "f394ab2c993283e94680ca13db98b99927868e98";
const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const EXPECTED_ARGUMENTS = Object.freeze([
  'target_os="mac"',
  'target_cpu="arm64"',
  "is_debug=false",
  "is_official_build=true",
  "chrome_pgo_phase=0",
  "clang_use_unsafe_buffers_plugin=false",
  "use_thin_lto=false",
  "is_component_build=false",
  "pdf_is_standalone=true",
  "pdf_is_complete_lib=true",
  "pdf_enable_v8=false",
  "pdf_enable_xfa=false",
  "pdf_use_skia=false",
  "pdf_use_partition_alloc=false",
  "pdf_bundle_freetype=true",
  "use_custom_libcxx=false",
  "symbol_level=0",
]);

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

export function validateMacosPdfiumSourceContract(contract) {
  const wrapper = contract?.wrapper;
  const pdfium = contract?.pdfium;
  const license = pdfium?.source_license_file;
  const pack = contract?.pack;
  if (
    !exactKeys(contract, [
      "schema_version",
      "target_triple",
      "product_runtime_network_access",
      "wrapper",
      "pdfium",
      "pack",
    ]) ||
    contract.schema_version !== CONTRACT_SCHEMA ||
    contract.target_triple !== TARGET ||
    contract.product_runtime_network_access !== "disabled" ||
    !exactKeys(wrapper, [
      "dependency",
      "dependency_version",
      "dependency_feature",
      "link_mode",
      "workspace_unsafe_code_allowed",
    ]) ||
    wrapper.dependency !== "pdfium-render" ||
    wrapper.dependency_version !== "0.9.3" ||
    wrapper.dependency_feature !== "pdfium_7881" ||
    wrapper.link_mode !== "complete-static-library" ||
    wrapper.workspace_unsafe_code_allowed !== false ||
    !exactKeys(pdfium, [
      "release",
      "source_repository",
      "source_commit",
      "source_build_dependency_revision",
      "license",
      "source_license_file",
      "gn_arguments",
      "build_target",
      "static_library_file",
    ]) ||
    pdfium.release !== "chromium/7881" ||
    pdfium.source_repository !== "https://pdfium.googlesource.com/pdfium.git" ||
    pdfium.source_commit !== PDFIUM_COMMIT ||
    pdfium.source_build_dependency_revision !== BUILD_REVISION ||
    pdfium.license !== "LicenseRef-PDFium-Root-LICENSE" ||
    !exactKeys(license, ["file", "bytes", "sha256"]) ||
    license.file !== "LICENSE" ||
    license.bytes !== 12_896 ||
    license.sha256 !== "1fe9dea718fbd75cf149adaf4d8a22a4335604d964ddb76d1b45383dec8668c9" ||
    !sameArray(pdfium.gn_arguments, EXPECTED_ARGUMENTS) ||
    pdfium.build_target !== "pdfium" ||
    pdfium.static_library_file !== "obj/libpdfium.a" ||
    !exactKeys(pack, ["schema_version", "library_file", "license_file", "args_file"]) ||
    pack.schema_version !== PACK_SCHEMA ||
    pack.library_file !== "libpdfium.a" ||
    pack.license_file !== "LICENSE" ||
    pack.args_file !== "args.gn"
  ) {
    throw new Error("macOS PDFium source contract is invalid");
  }
  return contract;
}

export function readMacosPdfiumSourceContract(file) {
  if (!path.isAbsolute(file)) {
    throw new Error("macOS PDFium source contract path is invalid");
  }
  let metadata;
  try {
    metadata = lstatSync(file);
  } catch {
    throw new Error("macOS PDFium source contract is missing");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > 32 * 1024
  ) {
    throw new Error("macOS PDFium source contract file is invalid");
  }
  try {
    return validateMacosPdfiumSourceContract(JSON.parse(readFileSync(file, "utf8")));
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("macOS PDFium source contract is not valid JSON");
    }
    throw error;
  }
}

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

async function directFile(file, label) {
  const metadata = await lstat(file).catch(() => undefined);
  if (!metadata?.isFile() || metadata.isSymbolicLink() || metadata.size === 0) {
    throw new Error(`${label} must be a non-empty regular file`);
  }
  return metadata;
}

function normalizedArguments(text) {
  return text
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("#"));
}

async function assertStaticArchive(file) {
  const handle = await open(file, "r");
  try {
    const header = Buffer.alloc(8);
    const { bytesRead } = await handle.read(header, 0, header.length, 0);
    if (bytesRead !== header.length || header.toString("ascii") !== "!<arch>\n") {
      throw new Error("PDFium library is not a static archive");
    }
  } finally {
    await handle.close();
  }
}

function validPackEntry(entry, expectedFile) {
  return (
    exactKeys(entry, ["file", "bytes", "sha256"]) &&
    entry.file === expectedFile &&
    Number.isSafeInteger(entry.bytes) &&
    entry.bytes > 0 &&
    SHA256_PATTERN.test(entry.sha256)
  );
}

function validatePackManifest(manifest, contract) {
  if (
    !exactKeys(manifest, [
      "schema_version",
      "target_triple",
      "source_commit",
      "source_build_dependency_revision",
      "gn_arguments",
      "library",
      "license",
      "args",
    ]) ||
    manifest.schema_version !== PACK_SCHEMA ||
    manifest.target_triple !== TARGET ||
    manifest.source_commit !== contract.pdfium.source_commit ||
    manifest.source_build_dependency_revision !==
      contract.pdfium.source_build_dependency_revision ||
    !sameArray(manifest.gn_arguments, contract.pdfium.gn_arguments) ||
    !validPackEntry(manifest.library, contract.pack.library_file) ||
    !validPackEntry(manifest.license, contract.pack.license_file) ||
    manifest.license.bytes !== contract.pdfium.source_license_file.bytes ||
    manifest.license.sha256 !== contract.pdfium.source_license_file.sha256 ||
    !validPackEntry(manifest.args, contract.pack.args_file)
  ) {
    throw new Error("macOS PDFium static pack manifest is invalid");
  }
  return manifest;
}

export async function verifyMacosPdfiumStaticPack({ directory, sourceContract }) {
  const contract = readMacosPdfiumSourceContract(sourceContract);
  const root = await lstat(directory).catch(() => undefined);
  if (!root?.isDirectory() || root.isSymbolicLink()) {
    throw new Error("macOS PDFium static pack is missing");
  }
  const entries = (await readdir(directory)).sort();
  const expected = [
    contract.pack.args_file,
    contract.pack.library_file,
    contract.pack.license_file,
    "runtime-pack.json",
  ].sort();
  if (JSON.stringify(entries) !== JSON.stringify(expected)) {
    throw new Error("macOS PDFium static pack contains unexpected entries");
  }
  const manifestFile = path.join(directory, "runtime-pack.json");
  await directFile(manifestFile, "PDFium pack manifest");
  let manifest;
  try {
    manifest = validatePackManifest(
      JSON.parse(await readFile(manifestFile, "utf8")),
      contract,
    );
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("macOS PDFium static pack manifest is not valid JSON");
    }
    throw error;
  }
  for (const entry of [manifest.library, manifest.license, manifest.args]) {
    const file = path.join(directory, entry.file);
    const metadata = await directFile(file, `PDFium pack ${entry.file}`);
    if (metadata.size !== entry.bytes || (await sha256(file)) !== entry.sha256) {
      throw new Error(`PDFium pack ${entry.file} does not match its manifest`);
    }
  }
  await assertStaticArchive(path.join(directory, contract.pack.library_file));
  if (
    !sameArray(
      normalizedArguments(await readFile(path.join(directory, contract.pack.args_file), "utf8")),
      contract.pdfium.gn_arguments,
    )
  ) {
    throw new Error("PDFium pack GN arguments do not match the reviewed contract");
  }
  return { contract, manifest, libraryDirectory: directory };
}

export async function stageMacosPdfiumRuntimePack({
  destination,
  directory,
  sourceContract,
}) {
  for (const value of [destination, directory, sourceContract]) {
    if (!path.isAbsolute(value)) {
      throw new Error("PDFium installed pack paths must be absolute");
    }
  }
  const verified = await verifyMacosPdfiumStaticPack({ directory, sourceContract });
  const parent = path.dirname(destination);
  const temporary = path.join(
    parent,
    `${path.basename(destination)}.tmp-${process.pid}-${Date.now()}`,
  );
  const backup = path.join(
    parent,
    `${path.basename(destination)}.old-${process.pid}-${Date.now()}`,
  );
  await mkdir(parent, { recursive: true });
  await rm(temporary, { recursive: true, force: true });
  await mkdir(temporary, { mode: 0o700 });
  try {
    const payloads = [
      {
        role: "license",
        source: path.join(directory, verified.contract.pack.license_file),
        file: "LICENSE",
      },
      {
        role: "build_arguments",
        source: path.join(directory, verified.contract.pack.args_file),
        file: "args.gn",
      },
      {
        role: "source_contract",
        source: sourceContract,
        file: "source-contract.json",
      },
    ];
    const files = [];
    for (const payload of payloads) {
      const target = path.join(temporary, payload.file);
      await copyFile(payload.source, target);
      await chmod(target, 0o644);
      const metadata = await directFile(target, `installed PDFium ${payload.role}`);
      files.push({
        role: payload.role,
        file: payload.file,
        bytes: metadata.size,
        sha256: await sha256(target),
      });
    }
    const installedManifest = {
      schema_version: INSTALLED_PACK_SCHEMA,
      runtime_pack_id: "pdfium-chromium-7881-static-arm64-v1",
      target_triple: TARGET,
      link_mode: "static",
      source_commit: verified.contract.pdfium.source_commit,
      source_build_dependency_revision:
        verified.contract.pdfium.source_build_dependency_revision,
      product_runtime_network_access: "disabled",
      files,
    };
    await writeFile(
      path.join(temporary, "runtime-pack.json"),
      `${JSON.stringify(installedManifest, null, 2)}\n`,
      { mode: 0o644 },
    );

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
    schema_version: "resume-ir.pdfium-static-runtime-stage.v1",
    target_triple: TARGET,
    resource_file_count: 4,
  });
}

export async function verifyMacosPdfiumRuntimePack({
  directory,
  sourceContract,
}) {
  for (const value of [directory, sourceContract]) {
    if (!path.isAbsolute(value)) {
      throw new Error("PDFium installed pack paths must be absolute");
    }
  }
  const contract = readMacosPdfiumSourceContract(sourceContract);
  const root = await lstat(directory).catch(() => undefined);
  if (!root?.isDirectory() || root.isSymbolicLink()) {
    throw new Error("installed PDFium runtime pack is missing");
  }
  const entries = (await readdir(directory)).sort();
  if (
    !sameArray(
      entries,
      ["LICENSE", "args.gn", "runtime-pack.json", "source-contract.json"].sort(),
    )
  ) {
    throw new Error("installed PDFium runtime pack contains unexpected entries");
  }
  const manifestFile = path.join(directory, "runtime-pack.json");
  const manifestMetadata = await directFile(
    manifestFile,
    "installed PDFium runtime manifest",
  );
  if (manifestMetadata.size > 1024 * 1024) {
    throw new Error("installed PDFium runtime manifest is invalid");
  }
  let manifest;
  try {
    manifest = JSON.parse(await readFile(manifestFile, "utf8"));
  } catch {
    throw new Error("installed PDFium runtime manifest is invalid");
  }
  const expectedFiles = [
    ["license", "LICENSE"],
    ["build_arguments", "args.gn"],
    ["source_contract", "source-contract.json"],
  ];
  if (
    !exactKeys(manifest, [
      "schema_version",
      "runtime_pack_id",
      "target_triple",
      "link_mode",
      "source_commit",
      "source_build_dependency_revision",
      "product_runtime_network_access",
      "files",
    ]) ||
    manifest.schema_version !== INSTALLED_PACK_SCHEMA ||
    manifest.runtime_pack_id !== "pdfium-chromium-7881-static-arm64-v1" ||
    manifest.target_triple !== TARGET ||
    manifest.link_mode !== "static" ||
    manifest.source_commit !== contract.pdfium.source_commit ||
    manifest.source_build_dependency_revision !==
      contract.pdfium.source_build_dependency_revision ||
    manifest.product_runtime_network_access !== "disabled" ||
    !Array.isArray(manifest.files) ||
    manifest.files.length !== expectedFiles.length ||
    manifest.files[0]?.bytes !== contract.pdfium.source_license_file.bytes ||
    manifest.files[0]?.sha256 !== contract.pdfium.source_license_file.sha256
  ) {
    throw new Error("installed PDFium runtime manifest is invalid");
  }
  let resourceBytes = 0;
  for (let index = 0; index < expectedFiles.length; index += 1) {
    const [role, file] = expectedFiles[index];
    const entry = manifest.files[index];
    if (
      !exactKeys(entry, ["role", "file", "bytes", "sha256"]) ||
      entry.role !== role ||
      entry.file !== file ||
      !Number.isSafeInteger(entry.bytes) ||
      entry.bytes <= 0 ||
      !SHA256_PATTERN.test(entry.sha256)
    ) {
      throw new Error("installed PDFium runtime manifest is invalid");
    }
    const payload = path.join(directory, file);
    const metadata = await directFile(payload, `installed PDFium ${role}`);
    if (metadata.size !== entry.bytes || (await sha256(payload)) !== entry.sha256) {
      throw new Error("installed PDFium runtime pack does not match its manifest");
    }
    resourceBytes += metadata.size;
  }
  const bundledContract = readMacosPdfiumSourceContract(
    path.join(directory, "source-contract.json"),
  );
  if (
    bundledContract.pdfium.source_commit !== contract.pdfium.source_commit ||
    bundledContract.pdfium.source_build_dependency_revision !==
      contract.pdfium.source_build_dependency_revision ||
    !sameArray(bundledContract.pdfium.gn_arguments, contract.pdfium.gn_arguments) ||
    !sameArray(
      normalizedArguments(await readFile(path.join(directory, "args.gn"), "utf8")),
      contract.pdfium.gn_arguments,
    )
  ) {
    throw new Error("installed PDFium runtime source identity is invalid");
  }
  return Object.freeze({
    manifest,
    resourceBytes,
    resourceFileCount: entries.length,
  });
}

function gitOutput(checkout, args) {
  const result = spawnSync("git", ["-C", checkout, ...args], {
    encoding: "utf8",
    shell: false,
  });
  if (result.error || result.status !== 0) {
    throw new Error("unable to verify PDFium source checkout");
  }
  return result.stdout.trim();
}

export async function assembleMacosPdfiumStaticPack({
  checkout,
  buildOutput,
  destination,
  sourceContract,
}) {
  for (const value of [checkout, buildOutput, destination, sourceContract]) {
    if (!path.isAbsolute(value)) throw new Error("PDFium pack paths must be absolute");
  }
  const contract = readMacosPdfiumSourceContract(sourceContract);
  if (
    gitOutput(checkout, ["rev-parse", "HEAD"]) !== contract.pdfium.source_commit ||
    gitOutput(checkout, ["status", "--porcelain", "--untracked-files=no"]) !== ""
  ) {
    throw new Error("PDFium source checkout is not the exact clean reviewed commit");
  }
  const sourceLicense = path.join(checkout, contract.pdfium.source_license_file.file);
  const sourceArgs = path.join(buildOutput, contract.pack.args_file);
  const sourceLibrary = path.join(buildOutput, contract.pdfium.static_library_file);
  const licenseMetadata = await directFile(sourceLicense, "PDFium source license");
  if (
    licenseMetadata.size !== contract.pdfium.source_license_file.bytes ||
    (await sha256(sourceLicense)) !== contract.pdfium.source_license_file.sha256
  ) {
    throw new Error("PDFium source license does not match the reviewed commit");
  }
  await directFile(sourceArgs, "PDFium GN arguments");
  await directFile(sourceLibrary, "PDFium static library");
  await assertStaticArchive(sourceLibrary);
  if (
    !sameArray(
      normalizedArguments(await readFile(sourceArgs, "utf8")),
      contract.pdfium.gn_arguments,
    )
  ) {
    throw new Error("PDFium build arguments do not match the reviewed contract");
  }
  const lipo = spawnSync("lipo", ["-info", sourceLibrary], {
    encoding: "utf8",
    shell: false,
  });
  if (lipo.error || lipo.status !== 0 || !/\barm64\b/u.test(lipo.stdout)) {
    throw new Error("PDFium static library is not an arm64 macOS archive");
  }

  const parent = path.dirname(destination);
  const temporary = path.join(
    parent,
    `${path.basename(destination)}.tmp-${process.pid}-${Date.now()}`,
  );
  const backup = path.join(
    parent,
    `${path.basename(destination)}.old-${process.pid}-${Date.now()}`,
  );
  await mkdir(parent, { recursive: true });
  await rm(temporary, { recursive: true, force: true });
  await mkdir(temporary, { mode: 0o700 });
  try {
    const files = [
      [sourceLibrary, contract.pack.library_file],
      [sourceLicense, contract.pack.license_file],
      [sourceArgs, contract.pack.args_file],
    ];
    for (const [source, name] of files) {
      await copyFile(source, path.join(temporary, name));
      await chmod(path.join(temporary, name), 0o644);
    }
    const entry = async (file) => {
      const metadata = await stat(path.join(temporary, file));
      return { file, bytes: metadata.size, sha256: await sha256(path.join(temporary, file)) };
    };
    const manifest = {
      schema_version: PACK_SCHEMA,
      target_triple: TARGET,
      source_commit: contract.pdfium.source_commit,
      source_build_dependency_revision: contract.pdfium.source_build_dependency_revision,
      gn_arguments: contract.pdfium.gn_arguments,
      library: await entry(contract.pack.library_file),
      license: await entry(contract.pack.license_file),
      args: await entry(contract.pack.args_file),
    };
    await writeFile(
      path.join(temporary, "runtime-pack.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
      { mode: 0o644 },
    );
    await verifyMacosPdfiumStaticPack({
      directory: temporary,
      sourceContract,
    });
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
  return verifyMacosPdfiumStaticPack({ directory: destination, sourceContract });
}

function parseArguments(args) {
  const parsed = {};
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (!value || !["--checkout", "--build-output", "--destination", "--contract"].includes(key)) {
      throw new Error("invalid macOS PDFium pack arguments");
    }
    parsed[key.slice(2).replace("-", "_")] = value;
  }
  return parsed;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const args = parseArguments(process.argv.slice(2));
  assembleMacosPdfiumStaticPack({
    checkout: args.checkout,
    buildOutput: args.build_output,
    destination: args.destination,
    sourceContract: args.contract,
  }).catch((error) => {
    console.error(`macos-pdfium-static-pack: ${error.message}`);
    process.exitCode = 1;
  });
}

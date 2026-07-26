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
import path from "node:path";

const TARGET = "x86_64-pc-windows-msvc";
const CONTRACT_SCHEMA = "resume-ir.windows-pdf-renderer-source-contract.v1";
const PACK_SCHEMA = "resume-ir.pdfium-static-build-pack.v1";
const INSTALLED_PACK_SCHEMA = "resume-ir.pdfium-static-runtime-pack.v1";
const INSTALLED_PACK_ID = "pdfium-chromium-7881-static-x64-v1";
const PDFIUM_SOURCE = "https://pdfium.googlesource.com/pdfium.git";
const PDFIUM_COMMIT = "91b9d569b34be4f38eed7b3c49b227356c3aadad";
const BUILD_REVISION = "f394ab2c993283e94680ca13db98b99927868e98";
const GN_ARGUMENTS = [
  'target_os="win"',
  'target_cpu="x64"',
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
];
const BUILD_TARGETS = ["pdfium", "pdfium_unittests", "pdfium_embeddertests"];
const ENVIRONMENT = [
  "RESUME_IR_PDF_RENDER_INPUT_PATH",
  "RESUME_IR_PDF_RENDER_PAGE_NO",
  "RESUME_IR_PDF_RENDER_DPI",
];
const FORBIDDEN_IMPORT_PREFIXES = [
  "MSVCP",
  "VCRUNTIME",
  "CONCRT",
  "UCRTBASE",
  "API-MS-WIN-CRT-",
];
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

export function validateWindowsPdfRendererSourceContract(contract) {
  const rejected = contract?.rejected_platform_api;
  const wrapper = contract?.wrapper;
  const protocol = contract?.protocol;
  const exitCodes = protocol?.exit_codes;
  const pdfium = contract?.pdfium;
  const license = pdfium?.source_license_file;
  const pack = contract?.pack;
  if (
    !exactKeys(contract, [
      "schema_version",
      "target_triple",
      "product_runtime_network_access",
      "rejected_platform_api",
      "wrapper",
      "protocol",
      "pdfium",
      "pack",
    ]) ||
    contract.schema_version !== CONTRACT_SCHEMA ||
    contract.target_triple !== TARGET ||
    contract.product_runtime_network_access !== "disabled" ||
    !exactKeys(rejected, [
      "name",
      "official_support_document",
      "desktop_package_identity_required",
      "target_installer_identity",
      "accepted",
    ]) ||
    rejected.name !== "Windows.Data.Pdf" ||
    rejected.official_support_document !==
      "https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/winrt-api-desktop-app-support" ||
    rejected.desktop_package_identity_required !== true ||
    rejected.target_installer_identity !== "tauri-nsis-unpackaged-win32" ||
    rejected.accepted !== false ||
    !exactKeys(wrapper, [
      "crate",
      "dependency",
      "dependency_version",
      "dependency_feature",
      "cargo_feature",
      "link_mode",
      "workspace_unsafe_code_allowed",
    ]) ||
    wrapper.crate !== "resume-pdf-render-runtime" ||
    wrapper.dependency !== "pdfium-render" ||
    wrapper.dependency_version !== "0.9.3" ||
    wrapper.dependency_feature !== "pdfium_7881" ||
    wrapper.cargo_feature !== "windows-static-pdfium" ||
    wrapper.link_mode !== "complete-static-library" ||
    wrapper.workspace_unsafe_code_allowed !== false ||
    !exactKeys(protocol, [
      "arguments",
      "environment",
      "input_max_bytes",
      "path_max_utf16_units",
      "page_min",
      "page_max",
      "dpi_min",
      "dpi_max",
      "dimension_max_pixels",
      "page_max_pixels",
      "stdout_max_bytes",
      "stdout_format",
      "stderr",
      "exit_codes",
    ]) ||
    !sameArray(protocol.arguments, []) ||
    !sameArray(protocol.environment, ENVIRONMENT) ||
    protocol.input_max_bytes !== 256 * 1024 * 1024 ||
    protocol.path_max_utf16_units !== 32_767 ||
    protocol.page_min !== 1 ||
    protocol.page_max !== 512 ||
    protocol.dpi_min !== 72 ||
    protocol.dpi_max !== 600 ||
    protocol.dimension_max_pixels !== 10_000 ||
    protocol.page_max_pixels !== 10_000_000 ||
    protocol.stdout_max_bytes !== 32 * 1024 * 1024 ||
    protocol.stdout_format !== "ppm-p6-rgb8" ||
    protocol.stderr !== "bounded-generic-only" ||
    !exactKeys(exitCodes, ["success", "unavailable", "invalid_request", "resource_limit"]) ||
    exitCodes.success !== 0 ||
    exitCodes.unavailable !== 1 ||
    exitCodes.invalid_request !== 2 ||
    exitCodes.resource_limit !== 3 ||
    !exactKeys(pdfium, [
      "release",
      "source_repository",
      "source_commit",
      "source_build_dependency_revision",
      "license",
      "source_license_file",
      "build_provenance_schema",
      "gn_arguments",
      "build_targets",
      "static_library_file",
      "final_binary_file",
      "required_final_dependency_closure",
      "forbidden_final_import_prefixes",
    ]) ||
    pdfium.release !== "chromium/7881" ||
    pdfium.source_repository !== PDFIUM_SOURCE ||
    pdfium.source_commit !== PDFIUM_COMMIT ||
    pdfium.source_build_dependency_revision !== BUILD_REVISION ||
    pdfium.license !== "LicenseRef-PDFium-Root-LICENSE" ||
    !exactKeys(license, ["file", "bytes", "sha256"]) ||
    license.file !== "LICENSE" ||
    license.bytes !== 12_896 ||
    license.sha256 !== "1fe9dea718fbd75cf149adaf4d8a22a4335604d964ddb76d1b45383dec8668c9" ||
    pdfium.build_provenance_schema !== "resume-ir.pdfium-windows-build-provenance.v1" ||
    !sameArray(pdfium.gn_arguments, GN_ARGUMENTS) ||
    !sameArray(pdfium.build_targets, BUILD_TARGETS) ||
    pdfium.static_library_file !== "pdfium.lib" ||
    pdfium.final_binary_file !== "resume-pdf-render-runtime.exe" ||
    pdfium.required_final_dependency_closure !== "windows-system-dlls-only" ||
    !sameArray(pdfium.forbidden_final_import_prefixes, FORBIDDEN_IMPORT_PREFIXES) ||
    !exactKeys(pack, ["schema_version", "library_file", "license_file", "args_file"]) ||
    pack.schema_version !== "resume-ir.pdfium-static-build-pack.v1" ||
    pack.library_file !== "pdfium.lib" ||
    pack.license_file !== "LICENSE" ||
    pack.args_file !== "args.gn"
  ) {
    throw new Error("Windows PDF renderer source contract is invalid");
  }
  return contract;
}

export function readWindowsPdfRendererSourceContract(file) {
  if (!path.isAbsolute(file)) {
    throw new Error("Windows PDF renderer source contract path is invalid");
  }
  let metadata;
  try {
    metadata = lstatSync(file);
  } catch {
    throw new Error("Windows PDF renderer source contract is missing");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > 64 * 1024
  ) {
    throw new Error("Windows PDF renderer source contract file is invalid");
  }
  try {
    return validateWindowsPdfRendererSourceContract(JSON.parse(readFileSync(file, "utf8")));
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("Windows PDF renderer source contract is not valid JSON");
    }
    throw error;
  }
}

async function directFile(file, label) {
  const metadata = await lstat(file).catch(() => undefined);
  if (!metadata?.isFile() || metadata.isSymbolicLink() || metadata.size === 0) {
    throw new Error(`${label} must be a non-empty regular file`);
  }
  return metadata;
}

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

function normalizedArguments(text) {
  return text
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("#"));
}

async function assertStaticLibrary(file) {
  const handle = await open(file, "r");
  try {
    const header = Buffer.alloc(8);
    const { bytesRead } = await handle.read(header, 0, header.length, 0);
    if (bytesRead !== header.length || header.toString("ascii") !== "!<arch>\n") {
      throw new Error("Windows PDFium library is not a COFF archive");
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
    SHA256.test(entry.sha256)
  );
}

function validateStaticPackManifest(manifest, contract) {
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
    throw new Error("Windows PDFium static pack manifest is invalid");
  }
  return manifest;
}

export async function verifyWindowsPdfiumStaticPack({ directory, sourceContract }) {
  const contract = readWindowsPdfRendererSourceContract(sourceContract);
  const root = await lstat(directory).catch(() => undefined);
  if (!root?.isDirectory() || root.isSymbolicLink()) {
    throw new Error("Windows PDFium static pack is missing");
  }
  const expected = [
    contract.pack.args_file,
    contract.pack.library_file,
    contract.pack.license_file,
    "runtime-pack.json",
  ].sort();
  if (!sameArray((await readdir(directory)).sort(), expected)) {
    throw new Error("Windows PDFium static pack contains unexpected entries");
  }
  let manifest;
  try {
    manifest = validateStaticPackManifest(
      JSON.parse(await readFile(path.join(directory, "runtime-pack.json"), "utf8")),
      contract,
    );
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("Windows PDFium static pack manifest is not valid JSON");
    }
    throw error;
  }
  for (const entry of [manifest.library, manifest.license, manifest.args]) {
    const file = path.join(directory, entry.file);
    const metadata = await directFile(file, `Windows PDFium pack ${entry.file}`);
    if (metadata.size !== entry.bytes || (await sha256(file)) !== entry.sha256) {
      throw new Error(`Windows PDFium pack ${entry.file} does not match its manifest`);
    }
  }
  await assertStaticLibrary(path.join(directory, contract.pack.library_file));
  if (
    !sameArray(
      normalizedArguments(await readFile(path.join(directory, contract.pack.args_file), "utf8")),
      contract.pdfium.gn_arguments,
    )
  ) {
    throw new Error("Windows PDFium GN arguments do not match the reviewed contract");
  }
  return Object.freeze({ contract, manifest, libraryDirectory: directory });
}

export async function assembleWindowsPdfiumStaticPack({
  library,
  license,
  args,
  destination,
  sourceContract,
}) {
  for (const value of [library, license, args, destination, sourceContract]) {
    if (!path.isAbsolute(value)) {
      throw new Error("Windows PDFium pack paths must be absolute");
    }
  }
  const contract = readWindowsPdfRendererSourceContract(sourceContract);
  const sourceFiles = [
    [library, contract.pack.library_file],
    [license, contract.pack.license_file],
    [args, contract.pack.args_file],
  ];
  for (const [file, name] of sourceFiles) {
    await directFile(file, `Windows PDFium source ${name}`);
  }
  await assertStaticLibrary(library);
  if (
    (await stat(license)).size !== contract.pdfium.source_license_file.bytes ||
    (await sha256(license)) !== contract.pdfium.source_license_file.sha256 ||
    !sameArray(
      normalizedArguments(await readFile(args, "utf8")),
      contract.pdfium.gn_arguments,
    )
  ) {
    throw new Error("Windows PDFium source artifacts do not match the reviewed contract");
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
    for (const [source, name] of sourceFiles) {
      await copyFile(source, path.join(temporary, name));
      await chmod(path.join(temporary, name), 0o644);
    }
    const identity = async (file) => {
      const metadata = await stat(path.join(temporary, file));
      return {
        file,
        bytes: metadata.size,
        sha256: await sha256(path.join(temporary, file)),
      };
    };
    await writeFile(
      path.join(temporary, "runtime-pack.json"),
      `${JSON.stringify(
        {
          schema_version: PACK_SCHEMA,
          target_triple: TARGET,
          source_commit: contract.pdfium.source_commit,
          source_build_dependency_revision:
            contract.pdfium.source_build_dependency_revision,
          gn_arguments: contract.pdfium.gn_arguments,
          library: await identity(contract.pack.library_file),
          license: await identity(contract.pack.license_file),
          args: await identity(contract.pack.args_file),
        },
        null,
        2,
      )}\n`,
      { mode: 0o644 },
    );
    await verifyWindowsPdfiumStaticPack({
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
  return verifyWindowsPdfiumStaticPack({ directory: destination, sourceContract });
}

export async function stageWindowsPdfiumRuntimePack({
  destination,
  directory,
  sourceContract,
}) {
  for (const value of [destination, directory, sourceContract]) {
    if (!path.isAbsolute(value)) {
      throw new Error("Windows PDFium runtime-pack paths must be absolute");
    }
  }
  const verified = await verifyWindowsPdfiumStaticPack({ directory, sourceContract });
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
      ["license", path.join(directory, verified.contract.pack.license_file), "LICENSE"],
      [
        "build_arguments",
        path.join(directory, verified.contract.pack.args_file),
        "args.gn",
      ],
      ["source_contract", sourceContract, "source-contract.json"],
    ];
    const files = [];
    for (const [role, source, file] of payloads) {
      const target = path.join(temporary, file);
      await copyFile(source, target);
      await chmod(target, 0o644);
      const metadata = await directFile(target, `Windows PDFium runtime ${role}`);
      files.push({ role, file, bytes: metadata.size, sha256: await sha256(target) });
    }
    await writeFile(
      path.join(temporary, "runtime-pack.json"),
      `${JSON.stringify(
        {
          schema_version: INSTALLED_PACK_SCHEMA,
          runtime_pack_id: INSTALLED_PACK_ID,
          target_triple: TARGET,
          link_mode: "static",
          source_commit: verified.contract.pdfium.source_commit,
          source_build_dependency_revision:
            verified.contract.pdfium.source_build_dependency_revision,
          product_runtime_network_access: "disabled",
          files,
        },
        null,
        2,
      )}\n`,
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

export async function verifyWindowsPdfiumRuntimePack({
  directory,
  sourceContract,
}) {
  const contract = readWindowsPdfRendererSourceContract(sourceContract);
  const root = await lstat(directory).catch(() => undefined);
  if (!root?.isDirectory() || root.isSymbolicLink()) {
    throw new Error("installed Windows PDFium runtime pack is missing");
  }
  const entries = (await readdir(directory)).sort();
  if (
    !sameArray(
      entries,
      ["LICENSE", "args.gn", "runtime-pack.json", "source-contract.json"].sort(),
    )
  ) {
    throw new Error("installed Windows PDFium runtime pack contains unexpected entries");
  }
  let manifest;
  try {
    manifest = JSON.parse(
      await readFile(path.join(directory, "runtime-pack.json"), "utf8"),
    );
  } catch {
    throw new Error("installed Windows PDFium runtime manifest is invalid");
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
    manifest.runtime_pack_id !== INSTALLED_PACK_ID ||
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
    throw new Error("installed Windows PDFium runtime manifest is invalid");
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
      !SHA256.test(entry.sha256)
    ) {
      throw new Error("installed Windows PDFium runtime manifest is invalid");
    }
    const payload = path.join(directory, file);
    const metadata = await directFile(payload, `installed Windows PDFium ${role}`);
    if (metadata.size !== entry.bytes || (await sha256(payload)) !== entry.sha256) {
      throw new Error("installed Windows PDFium runtime pack does not match its manifest");
    }
    resourceBytes += metadata.size;
  }
  const bundledContract = readWindowsPdfRendererSourceContract(
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
    throw new Error("installed Windows PDFium runtime source identity is invalid");
  }
  return Object.freeze({
    manifest,
    resourceBytes,
    resourceFileCount: entries.length,
  });
}

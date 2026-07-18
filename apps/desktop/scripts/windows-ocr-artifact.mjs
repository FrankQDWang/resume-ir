import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  readFile,
  readdir,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";

import { inspectWindowsPeExecutable } from "./windows-pe.mjs";
import {
  readWindowsOcrSourceContract,
  validateWindowsOcrSourceContract,
} from "./windows-ocr-pack.mjs";

const TARGET = "x86_64-pc-windows-msvc";
const PACK_SCHEMA = "resume-ir.desktop-ocr-runtime-pack.v1";
const SHA256 = /^[a-f0-9]{64}$/;
const VERSION = /^\d+(?:\.\d+){1,4}$/;
const SYSTEM_IMPORTS = new Set([
  "ADVAPI32.DLL",
  "BCRYPT.DLL",
  "COMBASE.DLL",
  "CRYPT32.DLL",
  "DBGHELP.DLL",
  "DWRITE.DLL",
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
  "SHELL32.DLL",
  "SHLWAPI.DLL",
  "USER32.DLL",
  "VERSION.DLL",
  "WINHTTP.DLL",
  "WINMM.DLL",
  "WS2_32.DLL",
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

function validIdentity(value, maxBytes) {
  return (
    exactKeys(value, ["bytes", "sha256"]) &&
    Number.isSafeInteger(value.bytes) &&
    value.bytes > 0 &&
    value.bytes <= maxBytes &&
    SHA256.test(value.sha256)
  );
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

async function recursiveFiles(root, relative = "") {
  const files = [];
  const entries = await readdir(path.join(root, relative), { withFileTypes: true });
  for (const entry of entries) {
    const child = path.posix.join(relative, entry.name);
    if (entry.isSymbolicLink()) {
      throw new Error("Windows OCR artifact inputs must not contain symlinks");
    }
    if (entry.isDirectory()) files.push(...(await recursiveFiles(root, child)));
    else if (entry.isFile()) files.push(child);
    else throw new Error("Windows OCR artifact inputs contain an unsupported entry");
  }
  return files.sort();
}

async function exactRoot(root, label, expectedFiles) {
  await directDirectory(root, label);
  if (JSON.stringify(await recursiveFiles(root)) !== JSON.stringify([...expectedFiles].sort())) {
    throw new Error(`${label} must contain exactly the reviewed files`);
  }
}

async function sha256File(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

async function verifyIdentity(file, identity, label, maxBytes) {
  const metadata = await directFile(file, label, maxBytes);
  if (metadata.size !== identity.bytes || (await sha256File(file)) !== identity.sha256) {
    throw new Error(`${label} does not match the reviewed identity`);
  }
  return metadata;
}

export function validateWindowsOcrDependencyClosure(imports, contract) {
  validateWindowsOcrSourceContract(contract);
  if (
    !Array.isArray(imports) ||
    imports.length === 0 ||
    imports.length > 128 ||
    new Set(imports).size !== imports.length ||
    imports.some((name) =>
      typeof name !== "string" ||
      name !== name.toUpperCase() ||
      !/^[A-Z0-9_.-]+$/.test(name) ||
      contract.final_artifact.forbidden_import_prefixes.some((prefix) =>
        name.startsWith(prefix),
      ) ||
      !SYSTEM_IMPORTS.has(name),
    )
  ) {
    throw new Error("Windows OCR dependency closure is not self-contained");
  }
  return imports;
}

function validSourceProvenance(value, source) {
  return (
    exactKeys(value, [
      "version",
      "source_repository",
      "source_tag",
      "source_commit",
      "cmake_generator",
      "cmake_arguments",
      "source_tree_clean",
    ]) &&
    value.version === source.version &&
    value.source_repository === source.source_repository &&
    value.source_tag === source.source_tag &&
    value.source_commit === source.source_commit &&
    value.cmake_generator === source.cmake_generator &&
    sameArray(value.cmake_arguments, source.cmake_arguments) &&
    value.source_tree_clean === true
  );
}

export function validateWindowsOcrBuildProvenance({
  provenance,
  contract,
  artifactBytes,
  artifactSha256,
  imports,
}) {
  validateWindowsOcrSourceContract(contract);
  validateWindowsOcrDependencyClosure(imports, contract);
  if (
    !exactKeys(provenance, [
      "schema_version",
      "target_triple",
      "tesseract",
      "leptonica",
      "msvc_runtime",
      "msvc_toolset_version",
      "windows_sdk_version",
      "cmake_version",
      "ninja_version",
      "tests_passed",
      "artifact_file",
      "artifact_bytes",
      "artifact_sha256",
      "artifact_imports",
    ]) ||
    provenance.schema_version !== contract.tesseract.build_provenance_schema ||
    provenance.target_triple !== TARGET ||
    !validSourceProvenance(provenance.tesseract, contract.tesseract) ||
    !validSourceProvenance(provenance.leptonica, contract.leptonica) ||
    provenance.msvc_runtime !== "static" ||
    ![
      provenance.msvc_toolset_version,
      provenance.windows_sdk_version,
      provenance.cmake_version,
      provenance.ninja_version,
    ].every((value) => typeof value === "string" && VERSION.test(value)) ||
    provenance.tests_passed !== true ||
    provenance.artifact_file !== contract.final_artifact.file ||
    provenance.artifact_bytes !== artifactBytes ||
    provenance.artifact_sha256 !== artifactSha256 ||
    !sameArray(provenance.artifact_imports, imports)
  ) {
    throw new Error("Windows OCR build provenance does not match artifact");
  }
  return provenance;
}

function manifestEntry(role, file, { bytes, sha256 }, executable = false) {
  return { role, file, bytes, sha256, executable };
}

export function createWindowsOcrRuntimePackManifest({
  contract,
  artifactIdentity,
  noticeIdentity,
}) {
  validateWindowsOcrSourceContract(contract);
  if (
    !validIdentity(artifactIdentity, contract.final_artifact.max_bytes) ||
    !validIdentity(noticeIdentity, 64 * 1024)
  ) {
    throw new Error("Windows OCR runtime pack identities are invalid");
  }
  const [eng, chiSim] = contract.traineddata.files;
  const tsv = contract.tesseract.engine_config_file;
  return {
    schema_version: PACK_SCHEMA,
    runtime_pack_id: contract.runtime_pack_id,
    target_triple: TARGET,
    engine: "tesseract",
    engine_version: contract.tesseract.version,
    renderer: "windows-pdfium-static",
    languages: [...contract.protocol.languages],
    network_access: "disabled",
    license_reviewed: true,
    third_party_notice: "THIRD-PARTY-NOTICES.json",
    files: [
      manifestEntry("engine_binary", "tesseract.exe", artifactIdentity, true),
      manifestEntry("language_eng", "tessdata/eng.traineddata", eng),
      manifestEntry("language_chi_sim", "tessdata/chi_sim.traineddata", chiSim),
      manifestEntry("engine_config", "tessdata/configs/tsv", tsv),
      manifestEntry(
        "license_text",
        "LICENSES/Tesseract-Apache-2.0.txt",
        contract.tesseract.source_license_file,
      ),
      manifestEntry(
        "license_text",
        "LICENSES/Leptonica-BSD-2-Clause.txt",
        contract.leptonica.source_license_file,
      ),
      manifestEntry(
        "license_text",
        "LICENSES/tessdata-fast-Apache-2.0.txt",
        contract.traineddata.source_license_file,
      ),
      manifestEntry(
        "third_party_notice",
        "THIRD-PARTY-NOTICES.json",
        noticeIdentity,
      ),
    ],
  };
}

function thirdPartyNotices(contract) {
  const components = [
    ["Tesseract", contract.tesseract, "LICENSES/Tesseract-Apache-2.0.txt"],
    ["Leptonica", contract.leptonica, "LICENSES/Leptonica-BSD-2-Clause.txt"],
    ["tessdata_fast", contract.traineddata, "LICENSES/tessdata-fast-Apache-2.0.txt"],
  ].map(([name, source, licenseFile]) => ({
    name,
    version: source.version,
    source_repository: source.source_repository,
    source_tag: source.source_tag,
    source_commit: source.source_commit,
    license: source.license,
    license_file: licenseFile,
  }));
  return Buffer.from(
    `${JSON.stringify(
      {
        schema_version: "resume-ir.windows-ocr-third-party-notices.v1",
        components,
      },
      null,
      2,
    )}\n`,
  );
}

async function validateRuntimeRoot(runtimeRoot, contract) {
  await exactRoot(runtimeRoot, "Windows OCR runtime root", [
    "LICENSE",
    "build-provenance.json",
    "leptonica-license.txt",
    "tesseract.exe",
  ]);
  const executable = path.join(runtimeRoot, "tesseract.exe");
  const metadata = await directFile(
    executable,
    "Windows OCR executable",
    contract.final_artifact.max_bytes,
  );
  const body = await readFile(executable);
  const artifactSha256 = createHash("sha256").update(body).digest("hex");
  const image = inspectWindowsPeExecutable(body);
  validateWindowsOcrDependencyClosure(image.imports, contract);
  await verifyIdentity(
    path.join(runtimeRoot, "LICENSE"),
    contract.tesseract.source_license_file,
    "Windows OCR Tesseract license",
    64 * 1024,
  );
  await verifyIdentity(
    path.join(runtimeRoot, "leptonica-license.txt"),
    contract.leptonica.source_license_file,
    "Windows OCR Leptonica license",
    64 * 1024,
  );
  const provenanceFile = path.join(runtimeRoot, "build-provenance.json");
  await directFile(provenanceFile, "Windows OCR build provenance", 64 * 1024);
  let provenance;
  try {
    provenance = JSON.parse(await readFile(provenanceFile, "utf8"));
  } catch {
    throw new Error("Windows OCR build provenance is invalid");
  }
  validateWindowsOcrBuildProvenance({
    provenance,
    contract,
    artifactBytes: metadata.size,
    artifactSha256,
    imports: image.imports,
  });
  return { executable, metadata, artifactSha256, image, provenance };
}

async function validateDataRoot(dataRoot, contract) {
  await exactRoot(dataRoot, "Windows OCR traineddata root", [
    "LICENSE",
    "chi_sim.traineddata",
    "configs/tsv",
    "eng.traineddata",
  ]);
  for (const [file, identity, label] of [
    ["eng.traineddata", contract.traineddata.files[0], "English traineddata"],
    ["chi_sim.traineddata", contract.traineddata.files[1], "Chinese traineddata"],
    ["configs/tsv", contract.tesseract.engine_config_file, "TSV config"],
    ["LICENSE", contract.traineddata.source_license_file, "tessdata license"],
  ]) {
    await verifyIdentity(
      path.join(dataRoot, file),
      identity,
      `Windows OCR ${label}`,
      8 * 1024 * 1024,
    );
  }
}

function containsPath(parent, child) {
  const relative = path.relative(path.resolve(parent), path.resolve(child));
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

export async function assembleWindowsOcrPack({
  contractFile,
  runtimeRoot,
  dataRoot,
  destination,
}) {
  if (
    ![contractFile, runtimeRoot, dataRoot, destination].every(
      (value) => typeof value === "string" && path.isAbsolute(value),
    )
  ) {
    throw new Error("Windows OCR assembly paths must be absolute");
  }
  if (
    containsPath(runtimeRoot, destination) ||
    containsPath(destination, runtimeRoot) ||
    containsPath(dataRoot, destination) ||
    containsPath(destination, dataRoot) ||
    containsPath(destination, contractFile)
  ) {
    throw new Error("Windows OCR assembly inputs and destination must not overlap");
  }
  const contract = readWindowsOcrSourceContract(contractFile);
  const runtime = await validateRuntimeRoot(runtimeRoot, contract);
  await validateDataRoot(dataRoot, contract);
  const noticeBody = thirdPartyNotices(contract);
  const noticeIdentity = {
    bytes: noticeBody.length,
    sha256: createHash("sha256").update(noticeBody).digest("hex"),
  };
  const manifest = createWindowsOcrRuntimePackManifest({
    contract,
    artifactIdentity: {
      bytes: runtime.metadata.size,
      sha256: runtime.artifactSha256,
    },
    noticeIdentity,
  });
  const parent = path.dirname(destination);
  const suffix = `${process.pid}-${Date.now()}`;
  const temporary = path.join(parent, `${path.basename(destination)}.tmp-${suffix}`);
  const backup = path.join(parent, `${path.basename(destination)}.old-${suffix}`);
  await mkdir(parent, { recursive: true });
  await directDirectory(parent, "Windows OCR assembly destination parent");
  await rm(temporary, { recursive: true, force: true });
  await mkdir(path.join(temporary, "tessdata", "configs"), {
    recursive: true,
    mode: 0o700,
  });
  await mkdir(path.join(temporary, "LICENSES"), { mode: 0o700 });
  let backupPresent = false;
  try {
    for (const [source, target] of [
      [runtime.executable, "tesseract.exe"],
      [path.join(dataRoot, "eng.traineddata"), "tessdata/eng.traineddata"],
      [path.join(dataRoot, "chi_sim.traineddata"), "tessdata/chi_sim.traineddata"],
      [path.join(dataRoot, "configs", "tsv"), "tessdata/configs/tsv"],
      [path.join(runtimeRoot, "LICENSE"), "LICENSES/Tesseract-Apache-2.0.txt"],
      [path.join(runtimeRoot, "leptonica-license.txt"), "LICENSES/Leptonica-BSD-2-Clause.txt"],
      [path.join(dataRoot, "LICENSE"), "LICENSES/tessdata-fast-Apache-2.0.txt"],
    ]) {
      await copyFile(source, path.join(temporary, target));
    }
    for (const [file, body] of [
      ["THIRD-PARTY-NOTICES.json", noticeBody],
      ["build-provenance.json", `${JSON.stringify(runtime.provenance, null, 2)}\n`],
      ["source-contract.json", `${JSON.stringify(contract, null, 2)}\n`],
      ["runtime-pack.json", `${JSON.stringify(manifest, null, 2)}\n`],
    ]) {
      await writeFile(path.join(temporary, file), body);
    }
    for (const entry of manifest.files) {
      await chmod(path.join(temporary, entry.file), entry.executable ? 0o755 : 0o644);
    }
    for (const file of ["build-provenance.json", "source-contract.json", "runtime-pack.json"]) {
      await chmod(path.join(temporary, file), 0o644);
    }
    await exactRoot(temporary, "Windows OCR candidate pack", [
      ...manifest.files.map(({ file }) => file),
      "build-provenance.json",
      "runtime-pack.json",
      "source-contract.json",
    ]);
    for (const entry of manifest.files) {
      await verifyIdentity(
        path.join(temporary, entry.file),
        entry,
        `Windows OCR candidate ${entry.role}`,
        contract.final_artifact.max_bytes,
      );
    }
    try {
      await rename(destination, backup);
      backupPresent = true;
    } catch (error) {
      if (!error || error.code !== "ENOENT") {
        throw new Error("Windows OCR candidate replacement failed");
      }
    }
    try {
      await rename(temporary, destination);
    } catch {
      if (backupPresent) {
        try {
          await rename(backup, destination);
          backupPresent = false;
        } catch {
          throw new Error("Windows OCR candidate replacement rollback failed");
        }
      }
      throw new Error("Windows OCR candidate replacement failed");
    }
    if (backupPresent) {
      await rm(backup, { recursive: true, force: true });
      backupPresent = false;
    }
  } catch (error) {
    if (
      error instanceof Error &&
      (error.message.startsWith("Windows OCR") || error.message.startsWith("Windows PE"))
    ) {
      throw error;
    }
    throw new Error("Windows OCR candidate assembly failed");
  } finally {
    await rm(temporary, { recursive: true, force: true });
    if (!backupPresent) await rm(backup, { recursive: true, force: true });
  }
  return Object.freeze({
    schema_version: "resume-ir.windows-ocr-pack-assembly.v1",
    target_triple: TARGET,
    resource_file_count: manifest.files.length + 1,
    review_file_count: 2,
    runtime_import_count: runtime.image.imports.length,
  });
}

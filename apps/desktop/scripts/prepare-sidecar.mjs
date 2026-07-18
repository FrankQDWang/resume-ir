import { createHash } from "node:crypto";
import {
  chmodSync,
  createReadStream,
  lstatSync,
  mkdirSync,
  readFileSync,
} from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  readFile,
  rename,
  rm,
  stat,
} from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import os from "node:os";
import path from "node:path";

import { stageClassifierResourcePack } from "./classifier-pack.mjs";
import { stageOcrResourcePack } from "./ocr-pack.mjs";
import { readWindowsEmbeddingSourceContract } from "./windows-embedding-pack.mjs";
import { readWindowsOcrSourceContract } from "./windows-ocr-pack.mjs";
import { readWindowsPdfRendererSourceContract } from "./windows-pdf-renderer.mjs";

const SUPPORTED_TARGET_TRIPLES = new Set([
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-pc-windows-msvc",
]);
const WINDOWS_TARGET_TRIPLE = "x86_64-pc-windows-msvc";
const EMBEDDING_RESOURCE_TARGETS = new Set(["aarch64-apple-darwin"]);
const EXPECTED_PACK_ROLES = new Set([
  "runtime_library",
  "model",
  "tokenizer",
  "model_config",
  "special_tokens_map",
  "tokenizer_config",
]);
const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const WINDOWS_PROCESS_OWNERS = Object.freeze([
  "desktop_daemon",
  "embedding_one_shot",
  "embedding_resident",
  "ocr_custom_engine",
  "ocr_tesseract",
  "pdf_custom_renderer",
  "pdf_pdftoppm",
]);

export function defaultSidecarBuildTargetDir() {
  return process.platform === "win32"
    ? path.join(os.tmpdir(), "resume-ir-tauri-sidecar-build")
    : path.join(path.sep, "tmp", "resume-ir-tauri-sidecar-build");
}

export function createSidecarPlan({
  repoRoot,
  buildTargetDir = path.join(repoRoot, "target"),
  targetTriple,
  debug,
  packageName = "resume-daemon",
  binaryName = "resume-daemon",
}) {
  if (typeof targetTriple !== "string" || targetTriple.length === 0) {
    throw new Error("target triple is required");
  }
  if (!SUPPORTED_TARGET_TRIPLES.has(targetTriple)) {
    throw new Error("target triple is not supported");
  }
  if (!path.isAbsolute(repoRoot)) {
    throw new Error("repository root must be absolute");
  }
  if (!path.isAbsolute(buildTargetDir)) {
    throw new Error("sidecar build target must be absolute");
  }
  if (![packageName, binaryName].every((value) => /^[a-z0-9-]+$/.test(value))) {
    throw new Error("sidecar package and binary names are invalid");
  }

  const windows = targetTriple.endsWith("-windows-msvc");
  const extension = windows ? ".exe" : "";
  const profile = debug ? "debug" : "release";
  const cargoArgs = [
    "build",
    "--manifest-path",
    path.join(repoRoot, "Cargo.toml"),
    "-p",
    packageName,
    "--bin",
    binaryName,
    "--locked",
    "--target",
    targetTriple,
    "--target-dir",
    buildTargetDir,
  ];
  if (!debug) cargoArgs.push("--release");

  return Object.freeze({
    buildKind: "cargo",
    buildTargetDir,
    cargoArgs: Object.freeze(cargoArgs),
    destination: path.join(
      repoRoot,
      "target",
      "tauri-sidecars",
      `${binaryName}-${targetTriple}${extension}`,
    ),
    binaryName,
    packageName,
    profile,
    repoRoot,
    source: path.join(
      buildTargetDir,
      targetTriple,
      profile,
      `${binaryName}${extension}`,
    ),
    targetTriple,
    windows,
  });
}

export function createPdfRendererPlan({ repoRoot, buildTargetDir, targetTriple, debug }) {
  if (targetTriple !== "aarch64-apple-darwin") {
    throw new Error("PDF renderer target is not supported");
  }
  if (![repoRoot, buildTargetDir].every(path.isAbsolute)) {
    throw new Error("PDF renderer build paths must be absolute");
  }
  const profile = debug ? "debug" : "release";
  const binaryName = "resume-pdf-render-runtime";
  const sourceFile = path.join(
    repoRoot,
    "apps",
    "desktop",
    "native",
    "macos",
    "pdf_render_runtime.m",
  );
  const source = path.join(buildTargetDir, targetTriple, profile, binaryName);
  return Object.freeze({
    binaryName,
    buildKind: "clang",
    buildTargetDir,
    clangArgs: Object.freeze([
      "clang",
      debug ? "-O0" : "-O2",
      "-fobjc-arc",
      "-arch",
      "arm64",
      "-mmacosx-version-min=13.0",
      "-framework",
      "Foundation",
      "-framework",
      "CoreGraphics",
      sourceFile,
      "-o",
      source,
    ]),
    destination: path.join(
      repoRoot,
      "target",
      "tauri-sidecars",
      `${binaryName}-${targetTriple}`,
    ),
    profile,
    repoRoot,
    source,
    sourceFile,
    targetTriple,
    windows: false,
  });
}

export function createDesktopCompositionPlan({
  repoRoot,
  buildTargetDir = path.join(repoRoot, "target"),
  targetTriple,
  debug,
  sourcePackRoot = path.join(repoRoot, ".cache", "resume-ir-native-e5-qint8-pack"),
  sourceOcrPackRoot = path.join(repoRoot, ".cache", "resume-ir-macos-ocr-runtime-pack"),
  sourceClassifierPackRoot = path.join(
    repoRoot,
    ".cache",
    "resume-ir-classifier-model-pack",
  ),
  expectedManifest = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "embedding",
    targetTriple ?? "missing-target",
    "runtime-pack.json",
  ),
  expectedOcrManifest = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "ocr",
    targetTriple ?? "missing-target",
    "runtime-pack.json",
  ),
  expectedClassifierManifest = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "classifier",
    targetTriple ?? "missing-target",
    "runtime-pack.json",
  ),
  processContainmentContract = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "process-containment",
    WINDOWS_TARGET_TRIPLE,
    "contract.json",
  ),
  windowsEmbeddingSourceContract = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "embedding",
    WINDOWS_TARGET_TRIPLE,
    "source-contract.json",
  ),
  windowsPdfRendererSourceContract = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "pdf-renderer",
    WINDOWS_TARGET_TRIPLE,
    "source-contract.json",
  ),
  windowsOcrSourceContract = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "ocr",
    WINDOWS_TARGET_TRIPLE,
    "source-contract.json",
  ),
}) {
  if (targetTriple === WINDOWS_TARGET_TRIPLE) {
    readWindowsProcessContainmentContract(processContainmentContract);
    readWindowsEmbeddingSourceContract(windowsEmbeddingSourceContract);
    readWindowsOcrSourceContract(windowsOcrSourceContract);
    readWindowsPdfRendererSourceContract(windowsPdfRendererSourceContract);
    throw new Error(
      "Windows desktop composition is incomplete: reviewed static-CRT x64 embedding, static Tesseract OCR, static PDFium renderer, and process-containment contracts are present; real reviewed embedding/Tesseract/PDFium artifacts, expected pack manifests, final PE dependency closure, and native evidence are required; refusing a partial NSIS build",
    );
  }
  if (!EMBEDDING_RESOURCE_TARGETS.has(targetTriple)) {
    throw new Error("embedding resource target is not supported");
  }
  if (
    ![
      sourcePackRoot,
      expectedManifest,
      sourceOcrPackRoot,
      expectedOcrManifest,
      sourceClassifierPackRoot,
      expectedClassifierManifest,
    ].every(path.isAbsolute)
  ) {
    throw new Error("desktop resource paths must be absolute");
  }
  const sidecarOptions = { repoRoot, buildTargetDir, targetTriple, debug };
  return Object.freeze({
    sidecars: Object.freeze([
      createSidecarPlan(sidecarOptions),
      createSidecarPlan({
        ...sidecarOptions,
        packageName: "resume-embedding-runtime",
        binaryName: "resume-embedding-runtime",
      }),
      createPdfRendererPlan(sidecarOptions),
    ]),
    ocrResourcePack: Object.freeze({
      destination: path.join(repoRoot, "target", "tauri-resources", "ocr-runtime-pack"),
      expectedManifest: expectedOcrManifest,
      sourcePackRoot: sourceOcrPackRoot,
      targetTriple,
    }),
    classifierResourcePack: Object.freeze({
      destination: path.join(
        repoRoot,
        "target",
        "tauri-resources",
        "classifier-model-pack",
      ),
      expectedManifest: expectedClassifierManifest,
      sourcePackRoot: sourceClassifierPackRoot,
      targetTriple,
    }),
    resourcePack: Object.freeze({
      destination: path.join(
        repoRoot,
        "target",
        "tauri-resources",
        "embedding-runtime-pack",
      ),
      expectedManifest,
      sourcePackRoot,
      targetTriple,
    }),
    targetTriple,
  });
}

export function validateWindowsProcessContainmentContract(contract) {
  if (
    !contract ||
    contract.schema_version !== "resume-ir.windows-process-containment.v1" ||
    contract.target_triple !== WINDOWS_TARGET_TRIPLE ||
    contract.minimum_windows_build !== 10240 ||
    contract.wrapper_crate !== "process-containment" ||
    contract.job_limit !== "kill_on_job_close" ||
    contract.breakaway_allowed !== false ||
    contract.spawn_failure_mode !== "fail_closed_and_reaped" ||
    contract.workspace_unsafe_code_allowed !== false ||
    !Array.isArray(contract.covered_spawn_owners) ||
    contract.covered_spawn_owners.length !== WINDOWS_PROCESS_OWNERS.length ||
    contract.covered_spawn_owners.some(
      (owner, index) => owner !== WINDOWS_PROCESS_OWNERS[index],
    )
  ) {
    throw new Error("Windows process containment contract is invalid");
  }
  return contract;
}

function readWindowsProcessContainmentContract(file) {
  if (!path.isAbsolute(file)) {
    throw new Error("Windows process containment contract path is invalid");
  }
  let metadata;
  try {
    metadata = lstatSync(file);
  } catch {
    throw new Error("Windows process containment contract is missing");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > 16 * 1024
  ) {
    throw new Error("Windows process containment contract file is invalid");
  }
  try {
    return validateWindowsProcessContainmentContract(
      JSON.parse(readFileSync(file, "utf8")),
    );
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("Windows process containment contract is not valid JSON");
    }
    throw error;
  }
}

export async function stageBuiltSidecar(plan) {
  const label =
    plan.binaryName === "resume-daemon"
      ? "daemon"
      : plan.binaryName === "resume-embedding-runtime"
        ? "embedding runtime"
        : "PDF renderer runtime";
  let sourceMetadata;
  try {
    sourceMetadata = await stat(plan.source);
  } catch {
    throw new Error(`built ${label} sidecar is missing`);
  }
  if (!sourceMetadata.isFile()) {
    throw new Error(`built ${label} sidecar is not a file`);
  }
  if (sourceMetadata.size === 0) {
    throw new Error(`built ${label} sidecar is empty`);
  }

  const destinationDir = path.dirname(plan.destination);
  const destinationName = path.basename(plan.destination);
  await mkdir(destinationDir, { recursive: true });
  const stalePrefix = `${destinationName}.tmp-`;
  const temporary = path.join(
    destinationDir,
    `${stalePrefix}${process.pid}-${Date.now()}`,
  );
  try {
    await copyFile(plan.source, temporary);
    if (!plan.windows) await chmod(temporary, 0o755);
    try {
      await rename(temporary, plan.destination);
    } catch (error) {
      if (!error || !["EEXIST", "EPERM"].includes(error.code)) throw error;
      await rm(plan.destination, { force: true });
      await rename(temporary, plan.destination);
    }
  } finally {
    await rm(temporary, { force: true });
  }
}

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

export function validateRuntimePackManifest(manifest) {
  if (
    !manifest ||
    manifest.schema_version !== "resume-ir.embedding-runtime-pack.v1" ||
    manifest.runtime_pack_id !== "intfloat-multilingual-e5-small-qint8-r1" ||
    manifest.model_id !== manifest.runtime_pack_id ||
    manifest.dimension !== 384 ||
    manifest.provider !== "cpu" ||
    manifest.network_access !== "disabled" ||
    manifest.license_reviewed !== true ||
    manifest.model_license !== "MIT" ||
    manifest.onnxruntime_license !== "MIT" ||
    manifest.quantization !== "dynamic_int8" ||
    !Array.isArray(manifest.files) ||
    manifest.files.length !== EXPECTED_PACK_ROLES.size
  ) {
    throw new Error("embedding runtime manifest contract is invalid");
  }
  const roles = new Set();
  const files = new Set();
  for (const entry of manifest.files) {
    if (
      !entry ||
      !EXPECTED_PACK_ROLES.has(entry.role) ||
      roles.has(entry.role) ||
      typeof entry.file !== "string" ||
      entry.file.length === 0 ||
      path.basename(entry.file) !== entry.file ||
      files.has(entry.file) ||
      !Number.isSafeInteger(entry.bytes) ||
      entry.bytes <= 0 ||
      typeof entry.sha256 !== "string" ||
      !SHA256_PATTERN.test(entry.sha256)
    ) {
      throw new Error("embedding runtime manifest file contract is invalid");
    }
    roles.add(entry.role);
    files.add(entry.file);
  }
  if ([...EXPECTED_PACK_ROLES].some((role) => !roles.has(role))) {
    throw new Error("embedding runtime manifest role set is incomplete");
  }
  return manifest;
}

async function readDirectRegularFile(file, label) {
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

export async function stageEmbeddingResourcePack(plan) {
  let rootMetadata;
  try {
    rootMetadata = await lstat(plan.sourcePackRoot);
  } catch {
    throw new Error("embedding resource source is missing");
  }
  if (!rootMetadata.isDirectory() || rootMetadata.isSymbolicLink()) {
    throw new Error("embedding resource source must be a regular directory");
  }
  await readDirectRegularFile(plan.expectedManifest, "expected embedding manifest");
  const sourceManifestPath = path.join(plan.sourcePackRoot, "runtime-pack.json");
  await readDirectRegularFile(sourceManifestPath, "source embedding manifest");
  let expected;
  let source;
  try {
    expected = validateRuntimePackManifest(
      JSON.parse(await readFile(plan.expectedManifest, "utf8")),
    );
    source = validateRuntimePackManifest(
      JSON.parse(await readFile(sourceManifestPath, "utf8")),
    );
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("embedding runtime manifest is not valid JSON");
    }
    throw error;
  }
  if (JSON.stringify(source) !== JSON.stringify(expected)) {
    throw new Error("embedding runtime source does not match reviewed manifest");
  }

  for (const entry of expected.files) {
    const sourceFile = path.join(plan.sourcePackRoot, entry.file);
    const metadata = await readDirectRegularFile(
      sourceFile,
      `embedding resource ${entry.role}`,
    );
    if (metadata.size !== entry.bytes || (await sha256(sourceFile)) !== entry.sha256) {
      throw new Error(`embedding resource ${entry.role} does not match manifest`);
    }
  }

  const parent = path.dirname(plan.destination);
  const temporary = path.join(
    parent,
    `${path.basename(plan.destination)}.tmp-${process.pid}-${Date.now()}`,
  );
  const backup = path.join(
    parent,
    `${path.basename(plan.destination)}.old-${process.pid}-${Date.now()}`,
  );
  await mkdir(parent, { recursive: true });
  await rm(temporary, { recursive: true, force: true });
  await mkdir(temporary, { mode: 0o700 });
  try {
    await copyFile(plan.expectedManifest, path.join(temporary, "runtime-pack.json"));
    await chmod(path.join(temporary, "runtime-pack.json"), 0o644);
    for (const entry of expected.files) {
      const destination = path.join(temporary, entry.file);
      await copyFile(path.join(plan.sourcePackRoot, entry.file), destination);
      await chmod(destination, entry.role === "runtime_library" ? 0o755 : 0o644);
    }
    const copiedManifest = validateRuntimePackManifest(
      JSON.parse(await readFile(path.join(temporary, "runtime-pack.json"), "utf8")),
    );
    if (JSON.stringify(copiedManifest) !== JSON.stringify(expected)) {
      throw new Error("staged embedding manifest does not match reviewed composition");
    }
    for (const entry of expected.files) {
      const copiedFile = path.join(temporary, entry.file);
      const metadata = await readDirectRegularFile(
        copiedFile,
        `staged embedding resource ${entry.role}`,
      );
      if (metadata.size !== entry.bytes || (await sha256(copiedFile)) !== entry.sha256) {
        throw new Error(`staged embedding resource ${entry.role} does not match manifest`);
      }
    }
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
  return Object.freeze({
    schema_version: "resume-ir.embedding-resource-stage.v1",
    target_triple: plan.targetTriple,
    resource_file_count: expected.files.length + 1,
  });
}

export function runSidecarBuild(plan, runner = spawnSync) {
  prepareBuildTargetDirectory(plan);
  const result = runner("cargo", plan.cargoArgs, {
    cwd: plan.repoRoot,
    shell: false,
    stdio: "inherit",
  });
  if (result.error || result.status !== 0) {
    const label = plan.binaryName === "resume-daemon" ? "daemon" : "embedding runtime";
    throw new Error(`${label} sidecar build failed`);
  }
}

export function runPdfRendererBuild(plan, runner = spawnSync) {
  prepareBuildTargetDirectory(plan);
  mkdirSync(path.dirname(plan.source), { mode: 0o700, recursive: true });
  const result = runner("xcrun", plan.clangArgs, {
    cwd: plan.repoRoot,
    shell: false,
    stdio: "inherit",
  });
  if (result.error || result.status !== 0) {
    throw new Error("PDF renderer sidecar build failed");
  }
}

function prepareBuildTargetDirectory(plan) {
  try {
    mkdirSync(plan.buildTargetDir, { mode: 0o700, recursive: true });
    const metadata = lstatSync(plan.buildTargetDir);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      throw new Error("sidecar build target is not a secure directory");
    }
    if (
      process.platform !== "win32" &&
      typeof process.getuid === "function" &&
      metadata.uid !== process.getuid()
    ) {
      throw new Error("sidecar build target is not owned by the current user");
    }
    if (process.platform !== "win32") chmodSync(plan.buildTargetDir, 0o700);
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("sidecar build target")) {
      throw error;
    }
    throw new Error("unable to prepare sidecar build target");
  }
}

function parseArguments(args) {
  let targetTriple;
  let debug;
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === "--target") {
      targetTriple = args[index + 1];
      index += 1;
    } else if (argument === "--debug") {
      debug = true;
    } else if (argument === "--release") {
      debug = false;
    } else {
      throw new Error("unsupported prepare-sidecar argument");
    }
  }
  return { debug, targetTriple };
}

function debugFromEnvironment(value) {
  if (value === undefined || value === "false") return false;
  if (value === "true") return true;
  throw new Error("TAURI_ENV_DEBUG must be true or false");
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
  const targetTriple = options.targetTriple ?? process.env.TAURI_ENV_TARGET_TRIPLE;
  const debug = options.debug ?? debugFromEnvironment(process.env.TAURI_ENV_DEBUG);
  const plan = createDesktopCompositionPlan({
    repoRoot,
    buildTargetDir: defaultSidecarBuildTargetDir(),
    targetTriple,
    debug,
  });
  for (const sidecar of plan.sidecars) {
    if (sidecar.buildKind === "cargo") runSidecarBuild(sidecar);
    else runPdfRendererBuild(sidecar);
    await stageBuiltSidecar(sidecar);
  }
  await stageEmbeddingResourcePack(plan.resourcePack);
  await stageOcrResourcePack(plan.ocrResourcePack);
  await stageClassifierResourcePack(plan.classifierResourcePack);
  console.log(
    `prepared bundled desktop runtime composition for ${plan.targetTriple}`,
  );
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    console.error(`prepare-sidecar: ${error.message}`);
    process.exitCode = 1;
  });
}

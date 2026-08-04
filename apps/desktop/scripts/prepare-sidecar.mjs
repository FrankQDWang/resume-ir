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
  writeFile,
} from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import os from "node:os";
import path from "node:path";

import { stageClassifierResourcePack } from "./classifier-pack.mjs";
import {
  readMacosPdfiumSourceContract,
  stageMacosPdfiumRuntimePack,
  verifyMacosPdfiumStaticPack,
} from "./macos-pdfium-static-pack.mjs";
import { stageOcrResourcePack } from "./ocr-pack.mjs";
import { readWindowsEmbeddingSourceContract } from "./windows-embedding-pack.mjs";
import { readWindowsOcrSourceContract } from "./windows-ocr-pack.mjs";
import {
  readWindowsPdfRendererSourceContract,
  stageWindowsPdfiumRuntimePack,
  verifyWindowsPdfiumStaticPack,
} from "./windows-pdf-renderer.mjs";
import { stageRuntimeExecutableAttestation } from "./runtime-executable-attestation.mjs";

const SUPPORTED_TARGET_TRIPLES = new Set([
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-pc-windows-msvc",
]);
const WINDOWS_TARGET_TRIPLE = "x86_64-pc-windows-msvc";
const WINDOWS_STATIC_CRT_RUSTFLAGS = "-C\u001ftarget-feature=+crt-static";
const EMBEDDING_RESOURCE_TARGETS = new Set(["aarch64-apple-darwin"]);
const EXPECTED_PACK_ROLES = new Set([
  "runtime_library",
  "model",
  "tokenizer",
  "model_config",
  "special_tokens_map",
  "tokenizer_config",
]);
const COREML_PACK_SCHEMA = "resume-ir.coreml-embedding-runtime-pack.v1";
const COREML_PACK_ID = "intfloat-multilingual-e5-small-coreml-fp16-r1";
const COREML_WORKER_BINARY = "resume-coreml-embedding-worker";
const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const RUNTIME_ATTESTATION_ENV =
  "RESUME_IR_RUNTIME_EXECUTABLE_ATTESTATION";
const PDFIUM_STATIC_LIB_ENVS = new Map([
  [
    "aarch64-apple-darwin",
    "PDFIUM_STATIC_LIB_PATH_aarch64_apple_darwin",
  ],
  [
    WINDOWS_TARGET_TRIPLE,
    "PDFIUM_STATIC_LIB_PATH_x86_64_pc_windows_msvc",
  ],
]);
const RUNTIME_EXECUTABLE_ROLES = Object.freeze([
  Object.freeze({ role: "embedding_runtime", binaryName: "resume-embedding-runtime" }),
  Object.freeze({ role: "pdf_renderer", binaryName: "resume-pdf-render-runtime" }),
]);
const WINDOWS_PROCESS_OWNERS = Object.freeze([
  "desktop_daemon",
  "embedding_one_shot",
  "embedding_resident",
  "ocr_custom_engine",
  "ocr_tesseract",
  "pdfium",
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
  buildEnvironment = {},
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
  if (
    !buildEnvironment ||
    typeof buildEnvironment !== "object" ||
    Array.isArray(buildEnvironment) ||
    Object.entries(buildEnvironment).some(
      ([name, value]) =>
        ![PDFIUM_STATIC_LIB_ENVS.get(targetTriple), "CARGO_ENCODED_RUSTFLAGS"].includes(name) ||
        typeof value !== "string" ||
        (name === PDFIUM_STATIC_LIB_ENVS.get(targetTriple)
          ? !path.isAbsolute(value)
          : value !== WINDOWS_STATIC_CRT_RUSTFLAGS),
    )
  ) {
    throw new Error("sidecar build environment is invalid");
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
    buildEnvironment: Object.freeze({ ...buildEnvironment }),
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

export function createPdfRendererPlan({
  repoRoot,
  buildTargetDir,
  targetTriple,
  debug,
  buildEnvironment = {},
}) {
  if (!PDFIUM_STATIC_LIB_ENVS.has(targetTriple)) {
    throw new Error("PDF renderer target is not supported");
  }
  return createSidecarPlan({
    repoRoot,
    buildTargetDir,
    targetTriple,
    debug,
    buildEnvironment,
    packageName: "resume-pdf-render-runtime",
    binaryName: "resume-pdf-render-runtime",
  });
}

export function createDesktopCompositionPlan({
  repoRoot,
  buildTargetDir = path.join(repoRoot, "target"),
  targetTriple,
  debug,
  sourcePackRoot = path.join(repoRoot, ".cache", "resume-ir-native-e5-qint8-pack"),
  sourceCoreMlPackRoot = path.join(repoRoot, ".cache", "resume-ir-coreml-runtime-pack"),
  sourceOcrPackRoot = path.join(repoRoot, ".cache", "resume-ir-macos-ocr-runtime-pack"),
  sourceClassifierPackRoot = path.join(
    repoRoot,
    ".cache",
    "resume-ir-classifier-model-pack",
  ),
  sourcePdfiumPackRoot = path.join(
    repoRoot,
    ".cache",
    "resume-ir-macos-pdfium-static-pack",
  ),
  sourceWindowsPackRoot = path.join(
    repoRoot,
    ".cache",
    "resume-ir-windows-embedding-runtime-pack",
  ),
  sourceWindowsOcrPackRoot = path.join(
    repoRoot,
    ".cache",
    "resume-ir-windows-ocr-runtime-pack",
  ),
  sourceWindowsPdfiumPackRoot = path.join(
    repoRoot,
    ".cache",
    "resume-ir-windows-pdfium-static-pack",
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
  expectedCoreMlManifest = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "embedding",
    "aarch64-apple-darwin",
    "coreml-runtime-pack.json",
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
  macosPdfiumSourceContract = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "pdf-renderer",
    "aarch64-apple-darwin",
    "source-contract.json",
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
  windowsClassifierManifest = path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "classifier",
    WINDOWS_TARGET_TRIPLE,
    "runtime-pack.json",
  ),
}) {
  if (targetTriple === WINDOWS_TARGET_TRIPLE) {
    readWindowsProcessContainmentContract(processContainmentContract);
    readWindowsEmbeddingSourceContract(windowsEmbeddingSourceContract);
    readWindowsOcrSourceContract(windowsOcrSourceContract);
    readWindowsPdfRendererSourceContract(windowsPdfRendererSourceContract);
    if (
      ![
        sourceWindowsPackRoot,
        sourceWindowsOcrPackRoot,
        sourceClassifierPackRoot,
        sourceWindowsPdfiumPackRoot,
        windowsClassifierManifest,
      ].every(path.isAbsolute)
    ) {
      throw new Error("Windows desktop resource paths must be absolute");
    }
    const buildEnvironment = {
      CARGO_ENCODED_RUSTFLAGS: WINDOWS_STATIC_CRT_RUSTFLAGS,
      [PDFIUM_STATIC_LIB_ENVS.get(targetTriple)]: sourceWindowsPdfiumPackRoot,
    };
    const sidecarOptions = { repoRoot, buildTargetDir, targetTriple, debug };
    return createCompositionPlan({
      ...sidecarOptions,
      buildEnvironment,
      sourcePackRoot: sourceWindowsPackRoot,
      expectedManifest: path.join(sourceWindowsPackRoot, "runtime-pack.json"),
      sourceOcrPackRoot: sourceWindowsOcrPackRoot,
      expectedOcrManifest: path.join(sourceWindowsOcrPackRoot, "runtime-pack.json"),
      sourceClassifierPackRoot,
      expectedClassifierManifest: windowsClassifierManifest,
      sourcePdfiumPackRoot: sourceWindowsPdfiumPackRoot,
      pdfiumSourceContract: windowsPdfRendererSourceContract,
    });
  }
  if (!EMBEDDING_RESOURCE_TARGETS.has(targetTriple)) {
    throw new Error("embedding resource target is not supported");
  }
  readMacosPdfiumSourceContract(macosPdfiumSourceContract);
  if (
    ![
      sourcePackRoot,
      expectedManifest,
      sourceOcrPackRoot,
      expectedOcrManifest,
      sourceClassifierPackRoot,
      expectedClassifierManifest,
      sourcePdfiumPackRoot,
      macosPdfiumSourceContract,
      sourceCoreMlPackRoot,
      expectedCoreMlManifest,
    ].every(path.isAbsolute)
  ) {
    throw new Error("desktop resource paths must be absolute");
  }
  const buildEnvironment = {
    [PDFIUM_STATIC_LIB_ENVS.get(targetTriple)]: sourcePdfiumPackRoot,
  };
  return createCompositionPlan({
    repoRoot,
    buildTargetDir,
    targetTriple,
    debug,
    buildEnvironment,
    sourcePackRoot,
    expectedManifest,
    sourceOcrPackRoot,
    expectedOcrManifest,
    sourceClassifierPackRoot,
    expectedClassifierManifest,
    sourcePdfiumPackRoot,
    pdfiumSourceContract: macosPdfiumSourceContract,
    sourceCoreMlPackRoot,
    expectedCoreMlManifest,
  });
}

function createCompositionPlan({
  repoRoot,
  buildTargetDir,
  targetTriple,
  debug,
  buildEnvironment,
  sourcePackRoot,
  expectedManifest,
  sourceOcrPackRoot,
  expectedOcrManifest,
  sourceClassifierPackRoot,
  expectedClassifierManifest,
  sourcePdfiumPackRoot,
  pdfiumSourceContract,
  sourceCoreMlPackRoot,
  expectedCoreMlManifest,
}) {
  const sidecarOptions = { repoRoot, buildTargetDir, targetTriple, debug };
  return Object.freeze({
    sidecars: Object.freeze([
      createSidecarPlan({
        ...sidecarOptions,
        buildEnvironment,
      }),
      createSidecarPlan({
        ...sidecarOptions,
        buildEnvironment:
          targetTriple === WINDOWS_TARGET_TRIPLE
            ? { CARGO_ENCODED_RUSTFLAGS: WINDOWS_STATIC_CRT_RUSTFLAGS }
            : {},
        packageName: "resume-embedding-runtime",
        binaryName: "resume-embedding-runtime",
      }),
      createPdfRendererPlan({
        ...sidecarOptions,
        buildEnvironment,
      }),
    ]),
    pdfiumStaticPack: Object.freeze({
      directory: sourcePdfiumPackRoot,
      sourceContract: pdfiumSourceContract,
    }),
    pdfiumResourcePack: Object.freeze({
      destination: path.join(
        repoRoot,
        "target",
        "tauri-resources",
        "pdfium-static-runtime-pack",
      ),
      directory: sourcePdfiumPackRoot,
      sourceContract: pdfiumSourceContract,
    }),
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
    runtimeExecutableAttestation: Object.freeze({
      destination: path.join(
        repoRoot,
        "target",
        "tauri-sidecars",
        `runtime-executable-attestation-${targetTriple}.json`,
      ),
      profile: debug ? "debug" : "release",
      targetTriple,
    }),
    coreMlWorker:
      targetTriple === "aarch64-apple-darwin"
        ? Object.freeze({
            source: path.join(repoRoot, "scripts", "local", "coreml-resident-worker.swift"),
            destination: path.join(
              repoRoot,
              "target",
              "tauri-sidecars",
              `${COREML_WORKER_BINARY}-${targetTriple}`,
            ),
            targetTriple,
          })
        : null,
    coreMlResourcePack:
      targetTriple === "aarch64-apple-darwin"
        ? Object.freeze({
            destination: path.join(
              repoRoot,
              "target",
              "tauri-resources",
              "embedding-runtime-pack",
              "coreml",
            ),
            expectedManifest: expectedCoreMlManifest,
            sourcePackRoot: sourceCoreMlPackRoot,
            targetTriple,
          })
        : null,
    targetTriple,
  });
}

export function sidecarsInAttestedBuildOrder(plan) {
  const byName = new Map(plan.sidecars.map((sidecar) => [sidecar.binaryName, sidecar]));
  const runtimeSidecars = RUNTIME_EXECUTABLE_ROLES.map(({ binaryName }) => {
    const sidecar = byName.get(binaryName);
    if (!sidecar) throw new Error(`required ${binaryName} build is missing`);
    return sidecar;
  });
  const daemon = byName.get("resume-daemon");
  if (!daemon || byName.size !== RUNTIME_EXECUTABLE_ROLES.length + 1) {
    throw new Error("attested sidecar build role set is invalid");
  }
  return Object.freeze({
    runtimeSidecars: Object.freeze(runtimeSidecars),
    daemon,
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

export function validateCoreMlRuntimePackManifest(manifest) {
  const expectedRoles = new Set([
    "interactive_analytics",
    "interactive_core_data",
    "interactive_metadata",
    "interactive_model",
    "interactive_weights",
    "bulk_analytics",
    "bulk_core_data",
    "bulk_metadata",
    "bulk_model",
    "bulk_weights",
  ]);
  if (
    !manifest ||
    manifest.schema_version !== COREML_PACK_SCHEMA ||
    manifest.runtime_pack_id !== COREML_PACK_ID ||
    manifest.model_id !== COREML_PACK_ID ||
    manifest.upstream_model_id !== "intfloat/multilingual-e5-small" ||
    manifest.upstream_revision !== "614241f622f53c4eeff9890bdc4f31cfecc418b3" ||
    manifest.dimension !== 384 ||
    manifest.provider !== "coreml" ||
    manifest.compute_units !== "all" ||
    manifest.network_access !== "disabled" ||
    manifest.license_reviewed !== true ||
    manifest.model_license !== "MIT" ||
    manifest.target_triple !== "aarch64-apple-darwin" ||
    JSON.stringify(manifest.fixed_shapes) !== JSON.stringify(["B1x512", "B4x512"]) ||
    !Array.isArray(manifest.files) ||
    manifest.files.length !== expectedRoles.size
  ) {
    throw new Error("Core ML runtime manifest contract is invalid");
  }
  const roles = new Set();
  const files = new Set();
  for (const entry of manifest.files) {
    if (
      !entry ||
      !expectedRoles.has(entry.role) ||
      roles.has(entry.role) ||
      typeof entry.file !== "string" ||
      !entry.file.startsWith(entry.role.startsWith("interactive_") ? "e5-b1x512.mlmodelc/" : "e5-b4x512.mlmodelc/") ||
      path.isAbsolute(entry.file) ||
      entry.file.split("/").some((part) => part.length === 0 || part === "." || part === "..") ||
      files.has(entry.file) ||
      !Number.isSafeInteger(entry.bytes) ||
      entry.bytes <= 0 ||
      typeof entry.sha256 !== "string" ||
      !SHA256_PATTERN.test(entry.sha256)
    ) {
      throw new Error("Core ML runtime manifest file contract is invalid");
    }
    roles.add(entry.role);
    files.add(entry.file);
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

export async function stageCoreMlResourcePack(plan) {
  const sourceMetadata = await lstat(plan.sourcePackRoot).catch(() => null);
  if (!sourceMetadata?.isDirectory() || sourceMetadata.isSymbolicLink()) {
    throw new Error("Core ML resource source must be a regular directory");
  }
  const expectedPath = plan.expectedManifest;
  const sourceManifestPath = path.join(plan.sourcePackRoot, "runtime-pack.json");
  await readDirectRegularFile(expectedPath, "expected Core ML manifest");
  await readDirectRegularFile(sourceManifestPath, "source Core ML manifest");
  let expected;
  let source;
  try {
    expected = validateCoreMlRuntimePackManifest(
      JSON.parse(await readFile(expectedPath, "utf8")),
    );
    source = validateCoreMlRuntimePackManifest(
      JSON.parse(await readFile(sourceManifestPath, "utf8")),
    );
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("Core ML runtime manifest is not valid JSON");
    }
    throw error;
  }
  if (JSON.stringify(source) !== JSON.stringify(expected)) {
    throw new Error("Core ML resource source does not match reviewed manifest");
  }
  for (const entry of expected.files) {
    const sourceFile = path.join(plan.sourcePackRoot, entry.file);
    const metadata = await readDirectRegularFile(sourceFile, `Core ML resource ${entry.role}`);
    if (metadata.size !== entry.bytes || (await sha256(sourceFile)) !== entry.sha256) {
      throw new Error(`Core ML resource ${entry.role} does not match manifest`);
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
    await copyFile(expectedPath, path.join(temporary, "runtime-pack.json"));
    await chmod(path.join(temporary, "runtime-pack.json"), 0o644);
    for (const entry of expected.files) {
      const destination = path.join(temporary, entry.file);
      await mkdir(path.dirname(destination), { recursive: true });
      await copyFile(path.join(plan.sourcePackRoot, entry.file), destination);
      await chmod(destination, 0o644);
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
    schema_version: "resume-ir.coreml-resource-stage.v1",
    target_triple: plan.targetTriple,
    resource_file_count: expected.files.length + 1,
  });
}

export async function buildCoreMlWorker(plan, runner = spawnSync) {
  const source = lstatSync(plan.source);
  if (!source.isFile() || source.isSymbolicLink()) {
    throw new Error("Core ML worker source must be a regular file");
  }
  await mkdir(path.dirname(plan.destination), { recursive: true });
  const temporary = `${plan.destination}.tmp-${process.pid}-${Date.now()}`;
  try {
    const result = runner(
      "xcrun",
      [
        "swiftc",
        "-parse-as-library",
        "-O",
        "-whole-module-optimization",
        "-warnings-as-errors",
        plan.source,
        "-o",
        temporary,
      ],
      { shell: false, stdio: "inherit" },
    );
    if (result.error || result.status !== 0) {
      throw new Error("Core ML worker build failed");
    }
    await chmod(temporary, 0o755);
    await rename(temporary, plan.destination);
  } finally {
    await rm(temporary, { force: true });
  }
}

export function runSidecarBuild(plan, runner = spawnSync, environment = process.env) {
  prepareBuildTargetDirectory(plan);
  const env = environmentForSidecarBuild(plan, environment);
  const result = runner("cargo", plan.cargoArgs, {
    cwd: plan.repoRoot,
    env,
    shell: false,
    stdio: "inherit",
  });
  if (result.error || result.status !== 0) {
    const label = plan.binaryName === "resume-daemon"
      ? "daemon"
      : plan.binaryName === "resume-embedding-runtime"
        ? "embedding runtime"
        : "PDFium renderer runtime";
    throw new Error(`${label} sidecar build failed`);
  }
}

function environmentForSidecarBuild(plan, environment) {
  if (
    environment === null ||
    typeof environment !== "object" ||
    Array.isArray(environment)
  ) {
    throw new Error("sidecar build environment is invalid");
  }
  const cleanEnvironment = { ...environment };
  const inheritedAttestation = cleanEnvironment[RUNTIME_ATTESTATION_ENV];
  delete cleanEnvironment[RUNTIME_ATTESTATION_ENV];
  delete cleanEnvironment.PDFIUM_STATIC_LIB_PATH;
  delete cleanEnvironment.PDFIUM_DYNAMIC_LIB_PATH;
  for (const name of Object.keys(cleanEnvironment)) {
    if (name.startsWith("PDFIUM_STATIC_LIB_PATH_")) delete cleanEnvironment[name];
  }
  Object.assign(cleanEnvironment, plan.buildEnvironment);
  if (plan.binaryName !== "resume-daemon") return cleanEnvironment;
  if (typeof inheritedAttestation !== "string" || !path.isAbsolute(inheritedAttestation)) {
    throw new Error("daemon sidecar build requires an absolute runtime executable attestation");
  }
  return { ...cleanEnvironment, [RUNTIME_ATTESTATION_ENV]: inheritedAttestation };
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

export async function buildAttestedSidecars(
  plan,
  {
    cargoRunner = spawnSync,
    coreMlRunner = spawnSync,
    environment = process.env,
  } = {},
) {
  if (plan.targetTriple === WINDOWS_TARGET_TRIPLE) {
    await verifyWindowsPdfiumStaticPack(plan.pdfiumStaticPack);
  } else {
    await verifyMacosPdfiumStaticPack(plan.pdfiumStaticPack);
  }
  const { daemon, runtimeSidecars } = sidecarsInAttestedBuildOrder(plan);
  for (const sidecar of runtimeSidecars) {
    runSidecarBuild(sidecar, cargoRunner, environment);
    await stageBuiltSidecar(sidecar);
  }
  if (plan.coreMlWorker) {
    await buildCoreMlWorker(plan.coreMlWorker, coreMlRunner);
  }
  const attestationPath = await stageRuntimeExecutableAttestation(
    plan.runtimeExecutableAttestation,
    runtimeSidecars,
  );
  runSidecarBuild(daemon, cargoRunner, {
    ...environment,
    [RUNTIME_ATTESTATION_ENV]: attestationPath,
  });
  await stageBuiltSidecar(daemon);
  return attestationPath;
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
  await buildAttestedSidecars(plan);
  await stageEmbeddingResourcePack(plan.resourcePack);
  if (plan.coreMlResourcePack) {
    await stageCoreMlResourcePack(plan.coreMlResourcePack);
  }
  await stageOcrResourcePack(plan.ocrResourcePack);
  await stageClassifierResourcePack(plan.classifierResourcePack);
  if (plan.targetTriple === WINDOWS_TARGET_TRIPLE) {
    await stageWindowsPdfiumRuntimePack(plan.pdfiumResourcePack);
  } else {
    await stageMacosPdfiumRuntimePack(plan.pdfiumResourcePack);
  }
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

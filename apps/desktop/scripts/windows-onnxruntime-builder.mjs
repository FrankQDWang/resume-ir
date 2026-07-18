import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  open,
  readFile,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

import {
  readWindowsEmbeddingSourceContract,
  validateWindowsEmbeddingRuntimeArtifact,
} from "./windows-embedding-pack.mjs";

const TARGET = "x86_64-pc-windows-msvc";
const SOURCE_COMMIT = "2d924974ef147392ced8409d36bd6d2e7fcc8a74";
const PLAN_SCHEMA = "resume-ir.windows-onnxruntime-build-plan.v1";
const RECEIPT_SCHEMA = "resume-ir.windows-onnxruntime-build.v1";
const MAX_PATH_LENGTH = 1024;
const MAX_INSPECT_OUTPUT = 64 * 1024;
const MAX_BUILD_MILLISECONDS = 4 * 60 * 60 * 1000;
const MAX_DLL_BYTES = 256 * 1024 * 1024;

function normalizedVersion(value) {
  const match = String(value).match(/\d+(?:\.\d+){1,4}/);
  return match?.[0];
}

function versionAtLeast(value, floor) {
  const parts = value.split(".").map(Number);
  const required = floor.split(".").map(Number);
  for (let index = 0; index < Math.max(parts.length, required.length); index += 1) {
    const current = parts[index] ?? 0;
    const minimum = required[index] ?? 0;
    if (current !== minimum) return current > minimum;
  }
  return true;
}

function containsPath(parent, child) {
  const relative = path.relative(path.resolve(parent), path.resolve(child));
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function safeAbsolutePath(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= MAX_PATH_LENGTH &&
    path.isAbsolute(value) &&
    !/[\0\r\n]/.test(value)
  );
}

export function createWindowsOnnxRuntimeBuildPlan({
  sourceRoot,
  destination,
  contractFile = fileURLToPath(
    new URL(
      "../resources/embedding/x86_64-pc-windows-msvc/source-contract.json",
      import.meta.url,
    ),
  ),
  platform = process.platform,
  architecture = process.arch,
  environment = process.env,
}) {
  if (platform !== "win32" || architecture !== "x64") {
    throw new Error("Windows ONNX Runtime builder requires native Windows x64");
  }
  if (![sourceRoot, destination, contractFile].every(safeAbsolutePath)) {
    throw new Error("Windows ONNX Runtime build paths are invalid");
  }
  const normalizedSourceRoot = path.resolve(sourceRoot);
  const normalizedDestination = path.resolve(destination);
  if (
    containsPath(normalizedSourceRoot, normalizedDestination) ||
    containsPath(normalizedDestination, normalizedSourceRoot)
  ) {
    throw new Error("Windows ONNX Runtime build paths overlap");
  }
  const visualStudioVersion = normalizedVersion(environment.VSCMD_VER);
  const msvcToolsetVersion = normalizedVersion(environment.VCToolsVersion);
  const windowsSdkVersion = normalizedVersion(environment.WindowsSDKVersion);
  if (
    !visualStudioVersion?.startsWith("17.") ||
    !msvcToolsetVersion?.startsWith("14.") ||
    !windowsSdkVersion?.startsWith("10.")
  ) {
    throw new Error("Windows ONNX Runtime builder requires VS2022 Developer Prompt");
  }
  const contract = readWindowsEmbeddingSourceContract(contractFile);
  const buildRoot = path.join(normalizedSourceRoot, "build", "Windows");
  const buildScript = path.join(
    normalizedSourceRoot,
    "tools",
    "ci_build",
    "build.py",
  );
  return Object.freeze({
    schemaVersion: PLAN_SCHEMA,
    targetTriple: TARGET,
    sourceRoot: normalizedSourceRoot,
    destination: normalizedDestination,
    contract,
    buildRoot,
    buildScript,
    command: "python",
    args: Object.freeze([
      buildScript,
      "--build_dir",
      buildRoot,
      ...contract.onnxruntime.build_arguments,
    ]),
    artifactSource: path.join(
      buildRoot,
      "Release",
      "Release",
      "onnxruntime.dll",
    ),
    visualStudioVersion,
    msvcToolsetVersion,
    windowsSdkVersion,
  });
}

async function directRegularFile(file, maxBytes, message) {
  let metadata;
  try {
    metadata = await lstat(file);
  } catch {
    throw new Error(message);
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > maxBytes
  ) {
    throw new Error(message);
  }
  return metadata;
}

async function directDirectory(directory, message) {
  let metadata;
  try {
    metadata = await lstat(directory);
  } catch {
    throw new Error(message);
  }
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw new Error(message);
  }
}

async function sha256(file) {
  return createHash("sha256").update(await readFile(file)).digest("hex");
}

async function inspectSourceTree(plan, inspect) {
  await directDirectory(plan.sourceRoot, "Windows ONNX Runtime source root is invalid");
  await directRegularFile(
    plan.buildScript,
    4 * 1024 * 1024,
    "Windows ONNX Runtime official build script is missing",
  );
  for (const [file, identity] of [
    [path.join(plan.sourceRoot, plan.contract.onnxruntime.source_license_file.file), plan.contract.onnxruntime.source_license_file],
    [path.join(plan.sourceRoot, plan.contract.onnxruntime.source_notices_file.file), plan.contract.onnxruntime.source_notices_file],
  ]) {
    const metadata = await directRegularFile(
      file,
      4 * 1024 * 1024,
      "Windows ONNX Runtime source legal file is invalid",
    );
    if (metadata.size !== identity.bytes || (await sha256(file)) !== identity.sha256) {
      throw new Error("Windows ONNX Runtime source legal identity drifted");
    }
  }
  const request = (command, args) =>
    inspect({ command, args, cwd: plan.sourceRoot, maxBytes: MAX_INSPECT_OUTPUT });
  const pythonVersion = normalizedVersion(await request("python", ["--version"]));
  const cmakeVersion = normalizedVersion(await request("cmake", ["--version"]));
  const commit = (await request("git", ["rev-parse", "HEAD"])).trim();
  const status = await request("git", ["status", "--porcelain", "--untracked-files=all"]);
  const submodules = (await request("git", ["submodule", "status", "--recursive"]))
    .trimEnd()
    .split(/\r?\n/)
    .filter(Boolean);
  if (
    !pythonVersion ||
    !versionAtLeast(pythonVersion, "3.10") ||
    !cmakeVersion ||
    !versionAtLeast(cmakeVersion, "3.28") ||
    commit !== SOURCE_COMMIT ||
    status !== "" ||
    submodules.length === 0 ||
    submodules.length > 512 ||
    submodules.some((line) => !/^ [a-f0-9]{40} /.test(line))
  ) {
    throw new Error("Windows ONNX Runtime source or toolchain identity is invalid");
  }
  return Object.freeze({ pythonVersion, cmakeVersion });
}

async function prepareBuildRoot(plan) {
  const parent = path.dirname(plan.buildRoot);
  await mkdir(parent, { recursive: true });
  await directDirectory(parent, "Windows ONNX Runtime build parent is invalid");
  try {
    const metadata = await lstat(plan.buildRoot);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      throw new Error("Windows ONNX Runtime build root is invalid");
    }
    await rm(plan.buildRoot, { recursive: true, force: true });
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
}

async function optionalExistingDestination(plan) {
  try {
    await directDirectory(
      plan.destination,
      "Windows ONNX Runtime destination is invalid",
    );
    await validateWindowsEmbeddingRuntimeArtifact({
      runtimeRoot: plan.destination,
      contract: plan.contract,
    });
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    try {
      await lstat(plan.destination);
    } catch (missing) {
      if (missing?.code === "ENOENT") return false;
    }
    throw error;
  }
}

function buildProvenance(plan, tools, artifact) {
  return {
    schema_version: plan.contract.onnxruntime.provenance_schema,
    target_triple: TARGET,
    source_repository: plan.contract.onnxruntime.source_repository,
    source_tag: plan.contract.onnxruntime.source_tag,
    source_commit: plan.contract.onnxruntime.source_commit,
    version: plan.contract.onnxruntime.version,
    api_version: plan.contract.onnxruntime.api_version,
    build_arguments: plan.contract.onnxruntime.build_arguments,
    provider: "cpu",
    telemetry: false,
    source_tree_clean: true,
    builder_platform: "windows",
    builder_architecture: "x86_64",
    python_version: tools.pythonVersion,
    visual_studio_version: plan.visualStudioVersion,
    msvc_toolset_version: plan.msvcToolsetVersion,
    windows_sdk_version: plan.windowsSdkVersion,
    cmake_version: tools.cmakeVersion,
    tests_passed: true,
    artifact_file: "onnxruntime.dll",
    artifact_bytes: artifact.bytes,
    artifact_sha256: artifact.sha256,
  };
}

function receipt(validated, plan) {
  const value = {
    schema_version: RECEIPT_SCHEMA,
    target_triple: TARGET,
    source_commit: SOURCE_COMMIT,
    profile: "Release",
    tests_passed: true,
    dependency_closure: "windows-system-dlls-only",
    artifact_count: 1,
    artifacts: [
      {
        role: "runtime_library",
        file: "onnxruntime.dll",
        bytes: validated.dllBytes,
        sha256: validated.dllSha256,
        import_count: validated.image.imports.length,
      },
    ],
    toolchain: {
      visual_studio_version: plan.visualStudioVersion,
      msvc_toolset_version: plan.msvcToolsetVersion,
      windows_sdk_version: plan.windowsSdkVersion,
    },
  };
  if (Buffer.byteLength(JSON.stringify(value)) >= 4096) {
    throw new Error("Windows ONNX Runtime build receipt exceeds its bound");
  }
  return Object.freeze(value);
}

async function publishRuntime(plan, tools, { beforePromote = () => {} } = {}) {
  const artifact = await directRegularFile(
    plan.artifactSource,
    MAX_DLL_BYTES,
    "Windows ONNX Runtime build artifact is missing",
  );
  const artifactIdentity = {
    bytes: artifact.size,
    sha256: await sha256(plan.artifactSource),
  };
  const parent = path.dirname(plan.destination);
  await mkdir(parent, { recursive: true });
  await directDirectory(parent, "Windows ONNX Runtime destination parent is invalid");
  const lockFile = path.join(parent, ".windows-onnxruntime-builder.lock");
  let lock;
  try {
    lock = await open(lockFile, "wx", 0o600);
  } catch {
    throw new Error("Windows ONNX Runtime publish is already active");
  }
  let stage;
  let backup;
  let existing = false;
  let promoted = false;
  let result;
  let rollbackFailed = false;
  try {
    stage = await mkdtemp(path.join(parent, ".windows-onnxruntime-stage-"));
    backup = path.join(stage, "backup");
    const candidate = path.join(stage, "candidate");
    await mkdir(candidate);
    await copyFile(plan.artifactSource, path.join(candidate, "onnxruntime.dll"));
    await copyFile(
      path.join(plan.sourceRoot, plan.contract.onnxruntime.source_license_file.file),
      path.join(candidate, "LICENSE"),
    );
    await copyFile(
      path.join(plan.sourceRoot, plan.contract.onnxruntime.source_notices_file.file),
      path.join(candidate, "ThirdPartyNotices.txt"),
    );
    await writeFile(
      path.join(candidate, "build-provenance.json"),
      `${JSON.stringify(buildProvenance(plan, tools, artifactIdentity), null, 2)}\n`,
    );
    const candidateEvidence = await validateWindowsEmbeddingRuntimeArtifact({
      runtimeRoot: candidate,
      contract: plan.contract,
    });
    existing = await optionalExistingDestination(plan);
    if (existing) await rename(plan.destination, backup);
    await beforePromote();
    await rename(candidate, plan.destination);
    promoted = true;
    const destinationEvidence = await validateWindowsEmbeddingRuntimeArtifact({
      runtimeRoot: plan.destination,
      contract: plan.contract,
    });
    if (
      destinationEvidence.dllBytes !== candidateEvidence.dllBytes ||
      destinationEvidence.dllSha256 !== candidateEvidence.dllSha256
    ) {
      throw new Error("Windows ONNX Runtime published artifact drifted");
    }
    result = receipt(destinationEvidence, plan);
  } catch {
    if (promoted) {
      try {
        await rm(plan.destination, { recursive: true, force: true });
      } catch {
        rollbackFailed = true;
      }
    }
    if (existing) {
      try {
        await rm(plan.destination, { recursive: true, force: true });
        await rename(backup, plan.destination);
      } catch {
        rollbackFailed = true;
      }
    }
    throw new Error(
      rollbackFailed
        ? "Windows ONNX Runtime publish and rollback failed"
        : "Windows ONNX Runtime publish failed",
    );
  } finally {
    if (stage) await rm(stage, { recursive: true, force: true });
    try {
      await lock.close();
    } finally {
      await rm(lockFile, { force: true });
    }
  }
  return result;
}

async function runBoundedCommand({ command, args, cwd, maxBytes }) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      shell: false,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    const chunks = [];
    let bytes = 0;
    let exceeded = false;
    const consume = (chunk) => {
      bytes += chunk.length;
      if (bytes > maxBytes) {
        exceeded = true;
        child.kill();
      } else {
        chunks.push(chunk);
      }
    };
    child.stdout.on("data", consume);
    child.stderr.on("data", consume);
    child.once("error", () => reject(new Error("build-host command failed")));
    child.once("close", (code) => {
      if (code === 0 && !exceeded) resolve(Buffer.concat(chunks).toString("utf8"));
      else reject(new Error("build-host command failed"));
    });
  });
}

async function runOfficialBuild({ command, args, cwd }) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd,
      shell: false,
      stdio: "ignore",
      windowsHide: true,
    });
    const timer = setTimeout(() => child.kill(), MAX_BUILD_MILLISECONDS);
    child.once("error", () => {
      clearTimeout(timer);
      reject(new Error("Windows ONNX Runtime official build failed"));
    });
    child.once("close", (code) => {
      clearTimeout(timer);
      if (code === 0) resolve();
      else reject(new Error("Windows ONNX Runtime official build failed"));
    });
  });
}

export async function buildWindowsOnnxRuntime({
  sourceRoot,
  destination,
  contractFile,
  platform = process.platform,
  architecture = process.arch,
  environment = process.env,
  inspect = runBoundedCommand,
  runBuild = runOfficialBuild,
  beforePromote,
}) {
  const plan = createWindowsOnnxRuntimeBuildPlan({
    sourceRoot,
    destination,
    contractFile,
    platform,
    architecture,
    environment,
  });
  const tools = await inspectSourceTree(plan, inspect);
  await prepareBuildRoot(plan);
  await runBuild({ command: plan.command, args: plan.args, cwd: plan.sourceRoot });
  return publishRuntime(plan, tools, { beforePromote });
}

function parseArguments(args) {
  if (
    args.length !== 4 ||
    args[0] !== "--source-root" ||
    args[2] !== "--destination" ||
    !args[1] ||
    !args[3]
  ) {
    throw new Error("Windows ONNX Runtime builder arguments are invalid");
  }
  return { sourceRoot: args[1], destination: args[3] };
}

async function main() {
  const inputs = parseArguments(process.argv.slice(2));
  const result = await buildWindowsOnnxRuntime(inputs);
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`windows-onnxruntime-builder: ${error.message}`);
    process.exitCode = 1;
  });
}

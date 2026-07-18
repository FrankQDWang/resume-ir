import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  open,
  readFile,
  readdir,
  rename,
  rm,
} from "node:fs/promises";
import { fileURLToPath } from "node:url";
import os from "node:os";
import path from "node:path";

import { createTauriBuildEnvironment } from "./run-tauri.mjs";
import { inspectWindowsPeExecutable } from "./windows-pe.mjs";

const RUNTIME_TARGET = "x86_64-pc-windows-gnu";
const BUNDLE_TARGET = "x86_64-pc-windows-msvc";
const PLAN_SCHEMA = "resume-ir.windows-daemon-sidecar-build-plan.v1";
const RECEIPT_SCHEMA = "resume-ir.windows-daemon-sidecar-build.v1";
const DEFAULT_BUILD_TARGET_DIR = path.join(
  path.sep,
  "tmp",
  "resume-ir-windows-gnu-daemon-build",
);
const MAX_ARTIFACT_BYTES = 64 * 1024 * 1024;
const MAX_BUILD_OUTPUT_BYTES = 64 * 1024;
const STATIC_CRT_ENCODED_RUSTFLAGS =
  "-D\u001fwarnings\u001f-C\u001ftarget-feature=+crt-static";
const CARGO_ZIGBUILD_VERSION = "0.23.0";
const CARGO_ZIGBUILD_VERSION_OUTPUT = `cargo-zigbuild ${CARGO_ZIGBUILD_VERSION}`;
const ZIG_VERSION = "0.16.0";
const FORBIDDEN_IMPORT_PREFIXES =
  "CONCRT LIBCRYPTO LIBGCC LIBGOMP LIBSSL LIBWINPTHREAD MSVCP UCRTBASE VCOMP VCRUNTIME".split(
    " ",
  );
const WINDOWS_10_SYSTEM_IMPORTS = new Set(
  (
    "ADVAPI32.DLL API-MS-WIN-CORE-SYNCH-L1-2-0.DLL " +
    "API-MS-WIN-CRT-CONVERT-L1-1-0.DLL API-MS-WIN-CRT-ENVIRONMENT-L1-1-0.DLL " +
    "API-MS-WIN-CRT-FILESYSTEM-L1-1-0.DLL API-MS-WIN-CRT-HEAP-L1-1-0.DLL " +
    "API-MS-WIN-CRT-LOCALE-L1-1-0.DLL API-MS-WIN-CRT-MATH-L1-1-0.DLL " +
    "API-MS-WIN-CRT-PRIVATE-L1-1-0.DLL API-MS-WIN-CRT-RUNTIME-L1-1-0.DLL " +
    "API-MS-WIN-CRT-STDIO-L1-1-0.DLL API-MS-WIN-CRT-STRING-L1-1-0.DLL " +
    "API-MS-WIN-CRT-TIME-L1-1-0.DLL API-MS-WIN-CRT-UTILITY-L1-1-0.DLL " +
    "BCRYPT.DLL BCRYPTPRIMITIVES.DLL CRYPT32.DLL KERNEL32.DLL NTDLL.DLL " +
    "OLEAUT32.DLL PDH.DLL POWRPROF.DLL PSAPI.DLL SHELL32.DLL USER32.DLL WS2_32.DLL"
  ).split(" "),
);

function isWithin(parent, candidate) {
  const relative = path.relative(parent, candidate);
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

export function createWindowsDaemonSidecarBuildPlan({
  repoRoot,
  homeDirectory = os.homedir(),
  buildTargetDir = DEFAULT_BUILD_TARGET_DIR,
}) {
  if (
    ![repoRoot, homeDirectory, buildTargetDir].every(path.isAbsolute) ||
    path.parse(buildTargetDir).root === buildTargetDir
  ) {
    throw new Error("Windows daemon sidecar build paths are invalid");
  }
  const normalizedRepoRoot = path.resolve(repoRoot);
  const normalizedHomeDirectory = path.resolve(homeDirectory);
  const normalizedBuildTargetDir = path.resolve(buildTargetDir);
  if (
    isWithin(normalizedRepoRoot, normalizedBuildTargetDir) ||
    isWithin(normalizedHomeDirectory, normalizedBuildTargetDir)
  ) {
    throw new Error("Windows daemon sidecar build target is not isolated");
  }
  const file = `resume-daemon-${BUNDLE_TARGET}.exe`;
  const environment = createTauriBuildEnvironment({
    environment: {
      CARGO_ENCODED_RUSTFLAGS: STATIC_CRT_ENCODED_RUSTFLAGS,
      CARGO_TARGET_DIR: normalizedBuildTargetDir,
      CARGO_TERM_COLOR: "never",
      SOURCE_DATE_EPOCH: "0",
      ZERO_AR_DATE: "1",
    },
    repoRoot: normalizedRepoRoot,
    homeDirectory: normalizedHomeDirectory,
  });
  return Object.freeze({
    schemaVersion: PLAN_SCHEMA,
    runtimeTargetTriple: RUNTIME_TARGET,
    bundleTargetTriple: BUNDLE_TARGET,
    cargoZigbuildVersion: CARGO_ZIGBUILD_VERSION,
    zigVersion: ZIG_VERSION,
    command: "cargo",
    args: Object.freeze([
      "zigbuild",
      "--quiet",
      "--locked",
      "--release",
      "--target",
      RUNTIME_TARGET,
      "-p",
      "resume-daemon",
    ]),
    environment: Object.freeze(environment),
    repoRoot: normalizedRepoRoot,
    homeDirectory: normalizedHomeDirectory,
    buildTargetDir: normalizedBuildTargetDir,
    artifact: Object.freeze({
      role: "daemon",
      file,
      source: path.join(
        normalizedBuildTargetDir,
        RUNTIME_TARGET,
        "release",
        "resume-daemon.exe",
      ),
      destination: path.join(
        normalizedRepoRoot,
        "target",
        "tauri-sidecars",
        file,
      ),
      maxBytes: MAX_ARTIFACT_BYTES,
    }),
  });
}

async function boundedRegularFile(file, maxBytes, missingMessage) {
  let metadata;
  try {
    metadata = await lstat(file);
  } catch {
    throw new Error(missingMessage);
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > maxBytes
  ) {
    throw new Error("Windows daemon sidecar artifact is invalid");
  }
  return metadata;
}

async function optionalExistingArtifact(file, maxBytes) {
  try {
    const metadata = await lstat(file);
    if (
      !metadata.isFile() ||
      metadata.isSymbolicLink() ||
      metadata.size === 0 ||
      metadata.size > maxBytes
    ) {
      throw new Error("Windows daemon sidecar destination is invalid");
    }
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

function buildMachineIdentityMarkers({ repoRoot, homeDirectory }) {
  const markers = new Set([repoRoot, homeDirectory]);
  const homeName = path.basename(homeDirectory);
  if (homeName.length >= 3) markers.add(homeName);
  for (const marker of [...markers]) markers.add(marker.replaceAll("/", "\\"));
  return [...markers].filter(Boolean);
}

function containsBuildMachineIdentity(body, buildPaths) {
  return buildMachineIdentityMarkers(buildPaths).some((marker) =>
    [Buffer.from(marker, "utf8"), Buffer.from(marker, "utf16le")].some(
      (encoded) => body.includes(encoded),
    ),
  );
}

async function validateArtifact(file, expected, buildPaths) {
  const metadata = await boundedRegularFile(
    file,
    expected.maxBytes,
    "Windows daemon sidecar artifact is missing",
  );
  const body = await readFile(file);
  const image = inspectWindowsPeExecutable(body);
  if (
    image.imports.some(
      (name) =>
        FORBIDDEN_IMPORT_PREFIXES.some((prefix) => name.startsWith(prefix)) ||
        !WINDOWS_10_SYSTEM_IMPORTS.has(name),
    )
  ) {
    throw new Error("Windows daemon sidecar dependency closure is not self-contained");
  }
  if (containsBuildMachineIdentity(body, buildPaths)) {
    throw new Error("Windows daemon sidecar contains build-machine identity");
  }
  return Object.freeze({
    role: expected.role,
    file: expected.file,
    bytes: metadata.size,
    sha256: createHash("sha256").update(body).digest("hex"),
    import_count: image.imports.length,
  });
}

function receipt(artifact) {
  const value = {
    schema_version: RECEIPT_SCHEMA,
    runtime_target_triple: RUNTIME_TARGET,
    bundle_target_triple: BUNDLE_TARGET,
    profile: "release",
    cargo_zigbuild_version: CARGO_ZIGBUILD_VERSION,
    zig_version: ZIG_VERSION,
    artifact_count: 1,
    dependency_closure: "windows-10-system-dlls-only",
    sqlcipher_openssl_linkage: "static",
    build_machine_identity_path_markers: 0,
    artifacts: [artifact],
  };
  if (Buffer.byteLength(JSON.stringify(value)) >= 4096) {
    throw new Error("Windows daemon sidecar receipt exceeds its bound");
  }
  return Object.freeze(value);
}

export async function promoteWindowsDaemonSidecar(
  plan,
  { beforePromote = () => {} } = {},
) {
  if (
    plan?.schemaVersion !== PLAN_SCHEMA ||
    plan.runtimeTargetTriple !== RUNTIME_TARGET ||
    plan.bundleTargetTriple !== BUNDLE_TARGET ||
    plan.artifact?.role !== "daemon"
  ) {
    throw new Error("Windows daemon sidecar build plan is invalid");
  }
  const buildPaths = {
    repoRoot: plan.repoRoot,
    homeDirectory: plan.homeDirectory,
  };
  const sourceEvidence = await validateArtifact(
    plan.artifact.source,
    plan.artifact,
    buildPaths,
  );
  const destinationRoot = path.dirname(plan.artifact.destination);
  await mkdir(destinationRoot, { recursive: true });
  const lockFile = path.join(destinationRoot, ".windows-daemon-sidecar.lock");
  let lock;
  try {
    lock = await open(lockFile, "wx", 0o600);
  } catch {
    throw new Error("Windows daemon sidecar staging is already active");
  }
  let stage;
  let backup;
  let existing = false;
  let promoted = false;
  let result;
  let rollbackFailed = false;
  try {
    stage = await mkdtemp(path.join(destinationRoot, ".windows-daemon-stage-"));
    const candidate = path.join(stage, "candidate.exe");
    backup = path.join(stage, "backup.exe");
    await copyFile(plan.artifact.source, candidate);
    const candidateEvidence = await validateArtifact(
      candidate,
      plan.artifact,
      buildPaths,
    );
    if (
      candidateEvidence.bytes !== sourceEvidence.bytes ||
      candidateEvidence.sha256 !== sourceEvidence.sha256
    ) {
      throw new Error("Windows daemon sidecar staged artifact drifted");
    }
    existing = await optionalExistingArtifact(
      plan.artifact.destination,
      plan.artifact.maxBytes,
    );
    if (existing) await rename(plan.artifact.destination, backup);
    await beforePromote();
    await rename(candidate, plan.artifact.destination);
    promoted = true;
    const destinationEvidence = await validateArtifact(
      plan.artifact.destination,
      plan.artifact,
      buildPaths,
    );
    if (
      destinationEvidence.bytes !== sourceEvidence.bytes ||
      destinationEvidence.sha256 !== sourceEvidence.sha256
    ) {
      throw new Error("Windows daemon sidecar promoted artifact drifted");
    }
    result = receipt(destinationEvidence);
  } catch {
    if (promoted) {
      try {
        await rm(plan.artifact.destination, { force: true });
      } catch {
        rollbackFailed = true;
      }
    }
    if (existing) {
      try {
        await rm(plan.artifact.destination, { force: true });
        await rename(backup, plan.artifact.destination);
      } catch {
        rollbackFailed = true;
      }
    }
    throw new Error(
      rollbackFailed
        ? "Windows daemon sidecar staging and rollback failed"
        : "Windows daemon sidecar staging failed",
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

async function prepareBuildTargetDirectory(buildTargetDir) {
  try {
    await mkdir(buildTargetDir, { mode: 0o700, recursive: true });
    const metadata = await lstat(buildTargetDir);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      throw new Error("Windows daemon sidecar build target is not secure");
    }
    if (typeof process.getuid === "function" && metadata.uid !== process.getuid()) {
      throw new Error("Windows daemon sidecar build target is not owned");
    }
    await chmod(buildTargetDir, 0o700);
    for (const entry of await readdir(buildTargetDir)) {
      await rm(path.join(buildTargetDir, entry), { recursive: true, force: true });
    }
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("Windows daemon")) {
      throw error;
    }
    throw new Error("Windows daemon sidecar build target cannot be prepared");
  }
}

async function runBoundedCommand({ command, args, cwd, environment, maxBytes }) {
  return new Promise((resolve, reject) => {
    const childEnvironment = { ...process.env, ...environment };
    delete childEnvironment.RUSTFLAGS;
    const child = spawn(command, args, {
      cwd,
      env: childEnvironment,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    const chunks = [];
    let outputBytes = 0;
    let outputExceeded = false;
    const consume = (chunk) => {
      outputBytes += chunk.length;
      if (outputBytes > maxBytes) {
        outputExceeded = true;
        child.kill("SIGTERM");
      } else {
        chunks.push(chunk);
      }
    };
    child.stdout.on("data", consume);
    child.stderr.on("data", consume);
    child.once("error", () => reject(new Error("bounded command failed")));
    child.once("close", (code) => {
      if (code === 0 && !outputExceeded) {
        resolve(Buffer.concat(chunks).toString("utf8").trim());
      } else {
        reject(new Error("bounded command failed"));
      }
    });
  });
}

async function inspectTool(request) {
  return runBoundedCommand({ ...request, environment: {}, maxBytes: 1024 });
}

async function runCargoZigbuild(request) {
  try {
    await runBoundedCommand({
      ...request,
      maxBytes: MAX_BUILD_OUTPUT_BYTES,
    });
  } catch {
    throw new Error("Windows daemon sidecar build failed");
  }
}

export async function buildWindowsDaemonSidecar({
  repoRoot,
  homeDirectory = os.homedir(),
  buildTargetDir = DEFAULT_BUILD_TARGET_DIR,
  inspect = inspectTool,
  runBuild = runCargoZigbuild,
}) {
  const plan = createWindowsDaemonSidecarBuildPlan({
    repoRoot,
    homeDirectory,
    buildTargetDir,
  });
  const cargoZigbuildVersion = await inspect({
    command: "cargo-zigbuild",
    args: ["--version"],
    cwd: plan.repoRoot,
  });
  const zigVersion = await inspect({
    command: "zig",
    args: ["version"],
    cwd: plan.repoRoot,
  });
  if (
    cargoZigbuildVersion !== CARGO_ZIGBUILD_VERSION_OUTPUT ||
    zigVersion !== ZIG_VERSION
  ) {
    throw new Error("Windows daemon sidecar toolchain version is invalid");
  }
  await prepareBuildTargetDirectory(plan.buildTargetDir);
  await runBuild({
    command: plan.command,
    args: plan.args,
    cwd: plan.repoRoot,
    environment: plan.environment,
  });
  return promoteWindowsDaemonSidecar(plan);
}

async function main() {
  if (process.argv.length !== 2) {
    throw new Error("Windows daemon sidecar builder does not accept arguments");
  }
  const repoRoot = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "../../..",
  );
  const result = await buildWindowsDaemonSidecar({ repoRoot });
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`windows-daemon-sidecar: ${error.message}`);
    process.exitCode = 1;
  });
}

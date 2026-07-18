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
} from "node:fs/promises";
import { fileURLToPath } from "node:url";
import os from "node:os";
import path from "node:path";

import { createTauriBuildEnvironment } from "./run-tauri.mjs";
import { inspectWindowsPeExecutable } from "./windows-pe.mjs";

const TARGET = "x86_64-pc-windows-msvc";
const PLAN_SCHEMA = "resume-ir.windows-embedding-sidecar-build-plan.v1";
const RECEIPT_SCHEMA = "resume-ir.windows-embedding-sidecar-build.v1";
const MAX_ARTIFACT_BYTES = 64 * 1024 * 1024;
const MAX_BUILD_OUTPUT_BYTES = 64 * 1024;
const STATIC_CRT_ENCODED_RUSTFLAGS = "-C\u001ftarget-feature=+crt-static";
const CARGO_XWIN_VERSION = "0.22.0";
const CARGO_XWIN_VERSION_OUTPUT = `cargo-xwin-xwin ${CARGO_XWIN_VERSION}`;
const FORBIDDEN_IMPORT_PREFIXES = [
  "MSVCP",
  "VCRUNTIME",
  "CONCRT",
  "UCRTBASE",
  "API-MS-WIN-CRT-",
  "LIBGOMP",
  "VCOMP",
];
const SYSTEM_IMPORTS = new Set([
  "ADVAPI32.DLL",
  "API-MS-WIN-CORE-SYNCH-L1-2-0.DLL",
  "BCRYPT.DLL",
  "BCRYPTPRIMITIVES.DLL",
  "COMBASE.DLL",
  "CRYPT32.DLL",
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
  "SECUR32.DLL",
  "SHELL32.DLL",
  "SHLWAPI.DLL",
  "USER32.DLL",
  "USERENV.DLL",
  "VERSION.DLL",
  "WINHTTP.DLL",
  "WINMM.DLL",
  "WINTRUST.DLL",
  "WS2_32.DLL",
]);

export function createWindowsEmbeddingSidecarBuildPlan({
  repoRoot,
  homeDirectory = os.homedir(),
}) {
  if (!path.isAbsolute(repoRoot) || !path.isAbsolute(homeDirectory)) {
    throw new Error("Windows embedding sidecar repository root is invalid");
  }
  const file = `resume-embedding-runtime-${TARGET}.exe`;
  const environment = createTauriBuildEnvironment({
    environment: { CARGO_ENCODED_RUSTFLAGS: STATIC_CRT_ENCODED_RUSTFLAGS },
    repoRoot,
    homeDirectory,
  });
  return Object.freeze({
    schema_version: PLAN_SCHEMA,
    target_triple: TARGET,
    command: "cargo",
    cargo_xwin_version: CARGO_XWIN_VERSION,
    environment: Object.freeze(environment),
    args: Object.freeze([
      "xwin",
      "build",
      "--quiet",
      "--locked",
      "--release",
      "--target",
      TARGET,
      "-p",
      "resume-embedding-runtime",
    ]),
    repoRoot,
    homeDirectory,
    artifact: Object.freeze({
      role: "embedding_runtime",
      file,
      source: path.join(
        repoRoot,
        "target",
        TARGET,
        "release",
        "resume-embedding-runtime.exe",
      ),
      destination: path.join(repoRoot, "target", "tauri-sidecars", file),
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
    throw new Error("Windows embedding sidecar artifact is invalid");
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
      throw new Error("Windows embedding sidecar destination is invalid");
    }
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

function buildMachineIdentityMarkers({ repoRoot, homeDirectory }) {
  const markers = new Set(["/Users/", repoRoot, homeDirectory]);
  const homeName = path.basename(homeDirectory);
  if (homeName.length >= 3) markers.add(homeName);
  for (const marker of [...markers]) {
    markers.add(marker.replaceAll("/", "\\"));
  }
  return [...markers].filter(Boolean);
}

function containsBuildMachineIdentity(body, buildPaths) {
  return buildMachineIdentityMarkers(buildPaths).some((marker) =>
    [Buffer.from(marker, "utf8"), Buffer.from(marker, "utf16le")].some((encoded) =>
      body.includes(encoded),
    ),
  );
}

async function validateArtifact(file, expected, buildPaths) {
  const metadata = await boundedRegularFile(
    file,
    expected.maxBytes,
    "Windows embedding sidecar artifact is missing",
  );
  const body = await readFile(file);
  const image = inspectWindowsPeExecutable(body);
  if (
    image.imports.some(
      (name) =>
        FORBIDDEN_IMPORT_PREFIXES.some((prefix) => name.startsWith(prefix)) ||
        !SYSTEM_IMPORTS.has(name),
    )
  ) {
    throw new Error("Windows embedding sidecar dependency closure is not self-contained");
  }
  if (containsBuildMachineIdentity(body, buildPaths)) {
    throw new Error("Windows embedding sidecar contains build-machine identity");
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
    target_triple: TARGET,
    profile: "release",
    cargo_xwin_version: CARGO_XWIN_VERSION,
    artifact_count: 1,
    dependency_closure: "windows-system-dlls-only",
    build_machine_identity_path_markers: 0,
    artifacts: [artifact],
  };
  if (Buffer.byteLength(JSON.stringify(value)) >= 4096) {
    throw new Error("Windows embedding sidecar receipt exceeds its bound");
  }
  return Object.freeze(value);
}

export async function promoteWindowsEmbeddingSidecar(
  plan,
  { beforePromote = () => {} } = {},
) {
  if (
    plan?.schema_version !== PLAN_SCHEMA ||
    plan.target_triple !== TARGET ||
    plan.artifact?.role !== "embedding_runtime"
  ) {
    throw new Error("Windows embedding sidecar build plan is invalid");
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
  const lockFile = path.join(destinationRoot, ".windows-embedding-sidecar.lock");
  let lock;
  try {
    lock = await open(lockFile, "wx", 0o600);
  } catch {
    throw new Error("Windows embedding sidecar staging is already active");
  }
  let stage;
  let backup;
  let existing = false;
  let promoted = false;
  let result;
  let rollbackFailed = false;
  try {
    stage = await mkdtemp(
      path.join(destinationRoot, ".windows-embedding-sidecar-stage-"),
    );
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
      throw new Error("Windows embedding sidecar staged artifact drifted");
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
      throw new Error("Windows embedding sidecar promoted artifact drifted");
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
        ? "Windows embedding sidecar staging and rollback failed"
        : "Windows embedding sidecar staging failed",
    );
  } finally {
    if (stage) await rm(stage, { recursive: true, force: true });
    await lock.close();
    await rm(lockFile, { force: true });
  }
  return result;
}

async function runCargoXwin({ command, args, cwd, environment }) {
  await new Promise((resolve, reject) => {
    const childEnvironment = { ...process.env, ...environment };
    delete childEnvironment.RUSTFLAGS;
    const child = spawn(command, args, {
      cwd,
      env: childEnvironment,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    let outputBytes = 0;
    let outputExceeded = false;
    const consume = (chunk) => {
      outputBytes += chunk.length;
      if (outputBytes > MAX_BUILD_OUTPUT_BYTES && !outputExceeded) {
        outputExceeded = true;
        child.kill("SIGTERM");
      }
    };
    child.stdout.on("data", consume);
    child.stderr.on("data", consume);
    child.once("error", () =>
      reject(new Error("Windows embedding sidecar build failed")),
    );
    child.once("close", (code) => {
      if (code === 0 && !outputExceeded) resolve();
      else reject(new Error("Windows embedding sidecar build failed"));
    });
  });
}

async function inspectCargoXwin({ command, cwd }) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, ["xwin", "--version"], {
      cwd,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    const chunks = [];
    let outputBytes = 0;
    let outputExceeded = false;
    const consume = (chunk) => {
      outputBytes += chunk.length;
      if (outputBytes > 1024) {
        outputExceeded = true;
        child.kill("SIGTERM");
      } else {
        chunks.push(chunk);
      }
    };
    child.stdout.on("data", consume);
    child.stderr.on("data", consume);
    child.once("error", () =>
      reject(new Error("Windows embedding sidecar toolchain is unavailable")),
    );
    child.once("close", (code) => {
      if (code !== 0 || outputExceeded) {
        reject(new Error("Windows embedding sidecar toolchain is unavailable"));
        return;
      }
      resolve(Buffer.concat(chunks).toString("utf8").trim());
    });
  });
}

export async function buildWindowsEmbeddingSidecar({
  repoRoot,
  homeDirectory = os.homedir(),
  inspectTool = inspectCargoXwin,
  runBuild = runCargoXwin,
}) {
  const plan = createWindowsEmbeddingSidecarBuildPlan({ repoRoot, homeDirectory });
  if (
    (await inspectTool({ command: plan.command, cwd: plan.repoRoot })) !==
    CARGO_XWIN_VERSION_OUTPUT
  ) {
    throw new Error("Windows embedding sidecar toolchain version is invalid");
  }
  await runBuild({
    command: plan.command,
    args: plan.args,
    cwd: plan.repoRoot,
    environment: plan.environment,
  });
  return promoteWindowsEmbeddingSidecar(plan);
}

async function main() {
  if (process.argv.length !== 2) {
    throw new Error("Windows embedding sidecar builder does not accept arguments");
  }
  const repoRoot = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "../../..",
  );
  const result = await buildWindowsEmbeddingSidecar({ repoRoot });
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(`windows-embedding-sidecar: ${error.message}`);
    process.exitCode = 1;
  });
}

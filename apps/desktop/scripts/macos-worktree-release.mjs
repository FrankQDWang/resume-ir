import { spawnSync } from "node:child_process";
import { constants } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  open,
  readFile,
  realpath,
  rm,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { stageImmutableRuntimePacks } from "./macos-installed-main-acceptance/immutable-runtime-packs.mjs";
import {
  createMacosInternalTestPlan,
  resolveMacosTestReleasePaths,
} from "./macos-test-release.mjs";
import {
  validateSourceIdentity,
  verifyImmutableSnapshotSource,
} from "./macos-source-identity.mjs";
import { sha256 } from "./verify-bundled-sidecar.mjs";
import { createImmutableWorktreeSnapshot } from "./macos-worktree-build-source.mjs";

const MAX_JSON_BYTES = 64 * 1024;
const TARGET_TRIPLE = "aarch64-apple-darwin";

function releaseError() {
  return new Error("macOS worktree release failed");
}

async function requirePrivateDirectory(directory) {
  try {
    await mkdir(directory, { recursive: true, mode: 0o700 });
    await chmod(directory, 0o700);
    const [metadata, resolved] = await Promise.all([
      lstat(directory),
      realpath(directory),
    ]);
    if (
      !metadata.isDirectory() ||
      metadata.isSymbolicLink() ||
      resolved !== directory ||
      (metadata.mode & 0o777) !== 0o700
    ) {
      throw releaseError();
    }
  } catch (error) {
    if (error?.message === "macOS worktree release failed") throw error;
    throw releaseError();
  }
}

async function readBoundedJson(file) {
  let source;
  try {
    source = await readFile(file, "utf8");
  } catch {
    throw releaseError();
  }
  if (Buffer.byteLength(source, "utf8") > MAX_JSON_BYTES) throw releaseError();
  try {
    return JSON.parse(source);
  } catch {
    throw releaseError();
  }
}

function npmCliForNode(nodeExecutable) {
  return path.resolve(
    path.dirname(nodeExecutable),
    "..",
    "lib",
    "node_modules",
    "npm",
    "bin",
    "npm-cli.js",
  );
}

function defaultInstallDependencies({ frontendRoot, environment }) {
  return spawnSync(
    process.execPath,
    [
      npmCliForNode(process.execPath),
      "ci",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
    ],
    {
      cwd: frontendRoot,
      env: environment,
      shell: false,
      stdio: "ignore",
    },
  );
}

function defaultRunRelease({ script, frontendRoot, environment }) {
  return spawnSync(process.execPath, [script], {
    cwd: frontendRoot,
    encoding: "utf8",
    env: environment,
    maxBuffer: MAX_JSON_BYTES,
    shell: false,
  });
}

function parseReleaseReceipt(result, source) {
  if (
    result?.error ||
    result?.status !== 0 ||
    typeof result.stdout !== "string" ||
    result.stderr !== "" ||
    Buffer.byteLength(result.stdout, "utf8") > MAX_JSON_BYTES ||
    !result.stdout.endsWith("\n") ||
    result.stdout.slice(0, -1).includes("\n")
  ) {
    throw releaseError();
  }
  let receipt;
  try {
    receipt = JSON.parse(result.stdout);
  } catch {
    throw releaseError();
  }
  if (
    receipt?.schema_version !== "resume-ir.macos-dmg-composition.v3" ||
    JSON.stringify(receipt.source) !== JSON.stringify(source) ||
    receipt.target_triple !== TARGET_TRIPLE ||
    !/^[a-f0-9]{64}$/.test(receipt.dmg_sha256 ?? "") ||
    receipt.release_claim !== "composition_only"
  ) {
    throw releaseError();
  }
  return receipt;
}

async function publishArtifact({
  artifactRoot,
  dmg,
  receipt,
  source,
  version,
}) {
  await requirePrivateDirectory(artifactRoot);
  const artifactName =
    `resume-ir_${version}_aarch64_${source.source_tree_sha256.slice(0, 12)}.dmg`;
  const artifact = path.join(artifactRoot, artifactName);
  const existing = await lstat(artifact).catch((error) => {
    if (error?.code === "ENOENT") return undefined;
    throw releaseError();
  });
  if (existing) {
    if (
      !existing.isFile() ||
      existing.isSymbolicLink() ||
      (await sha256(artifact)) !== receipt.dmg_sha256
    ) {
      throw releaseError();
    }
  } else {
    try {
      await copyFile(
        dmg,
        artifact,
        constants.COPYFILE_EXCL | constants.COPYFILE_FICLONE,
      );
      await chmod(artifact, 0o444);
      if ((await sha256(artifact)) !== receipt.dmg_sha256) throw releaseError();
    } catch (error) {
      await rm(artifact, { force: true }).catch(() => {});
      if (error?.message === "macOS worktree release failed") throw error;
      throw releaseError();
    }
  }
  const artifactReceipt = Object.freeze({
    schema_version: "resume-ir.macos-worktree-artifact.v1",
    source,
    artifact_file: artifactName,
    dmg_sha256: receipt.dmg_sha256,
    composition_receipt: receipt,
  });
  const receiptFile = `${artifact}.json`;
  const body = `${JSON.stringify(artifactReceipt)}\n`;
  if (Buffer.byteLength(body, "utf8") > MAX_JSON_BYTES) throw releaseError();
  let handle;
  try {
    handle = await open(receiptFile, "wx", 0o444);
    await handle.writeFile(body, "utf8");
    await handle.sync();
  } catch (error) {
    if (error?.code !== "EEXIST") throw releaseError();
    const existingBody = await readFile(receiptFile, "utf8").catch(() => "");
    if (existingBody !== body) throw releaseError();
  } finally {
    await handle?.close().catch(() => {});
  }
  return Object.freeze({ artifact, receipt: artifactReceipt });
}

export async function buildMacosWorktreeRelease({
  repoRoot,
  environment = process.env,
  platform = process.platform,
  artifactRoot = path.join(repoRoot, ".cache", "macos-manual-artifacts"),
  cacheRoot = path.join(repoRoot, ".cache", "macos-worktree-build"),
  createSnapshot = createImmutableWorktreeSnapshot,
  installDependencies = defaultInstallDependencies,
  runRelease = defaultRunRelease,
  stageRuntimePacks = stageImmutableRuntimePacks,
}) {
  if (
    platform !== "darwin" ||
    !path.isAbsolute(repoRoot ?? "") ||
    !path.isAbsolute(cacheRoot ?? "") ||
    !path.isAbsolute(artifactRoot ?? "")
  ) {
    throw releaseError();
  }
  const snapshot = await createSnapshot({ repoRoot, cacheRoot });
  const source = validateSourceIdentity(snapshot.source);
  await verifyImmutableSnapshotSource({
    repoRoot: snapshot.repoRoot,
    expected: source,
  });
  await stageRuntimePacks({
    immutableRepoRoot: snapshot.repoRoot,
    sourceRepoRoot: repoRoot,
  });
  const frontendRoot = path.join(snapshot.repoRoot, "apps", "desktop");
  const npmCache = path.join(cacheRoot, "npm-cache");
  const cargoTargetDir = path.join(cacheRoot, "tauri-target");
  await Promise.all([
    requirePrivateDirectory(npmCache),
    requirePrivateDirectory(cargoTargetDir),
  ]);
  const buildEnvironment = {
    ...environment,
    CARGO_TARGET_DIR: cargoTargetDir,
    NPM_CONFIG_CACHE: npmCache,
    RESUME_IR_MACOS_SOURCE_IDENTITY: JSON.stringify(source),
  };
  const install = await installDependencies({
    frontendRoot,
    environment: buildEnvironment,
  });
  if (install?.error || install?.status !== 0) throw releaseError();

  const script = path.join(
    frontendRoot,
    "scripts",
    "macos-test-release.mjs",
  );
  const paths = resolveMacosTestReleasePaths(pathToFileURL(script).href);
  const [baseConfig, platformConfig] = await Promise.all([
    readBoundedJson(paths.baseConfig),
    readBoundedJson(paths.platformConfig),
  ]);
  const plan = createMacosInternalTestPlan({
    frontendRoot,
    platform: "darwin",
    baseConfig,
    platformConfig,
    cargoTargetDir,
  });
  const result = await runRelease({
    script,
    frontendRoot,
    environment: buildEnvironment,
    plan,
    source,
  });
  const receipt = parseReleaseReceipt(result, source);
  if ((await sha256(plan.dmg)) !== receipt.dmg_sha256) throw releaseError();
  await verifyImmutableSnapshotSource({
    repoRoot: snapshot.repoRoot,
    expected: source,
  });
  return publishArtifact({
    artifactRoot,
    dmg: plan.dmg,
    receipt,
    source,
    version: baseConfig.version,
  });
}

export function resolveMacosWorktreeRepoRoot(scriptUrl = import.meta.url) {
  return path.resolve(fileURLToPath(new URL("../../..", scriptUrl)));
}

async function main() {
  if (process.argv.length !== 2) throw releaseError();
  const repoRoot = resolveMacosWorktreeRepoRoot();
  const result = await buildMacosWorktreeRelease({ repoRoot });
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch(() => {
    process.stdout.write(
      '{"schema_version":"resume-ir.macos-worktree-artifact-failure.v1","outcome":"failed"}\n',
    );
    process.exitCode = 1;
  });
}

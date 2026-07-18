import { spawnSync } from "node:child_process";
import {
  copyFile,
  cp,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rename,
  rm,
  symlink,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  APPLE_TOOL_TIMEOUT_MS,
  MAX_DMG_BYTES,
  validateMountedDmgLayout,
  verifyMacosDmg,
  verifyMacosInternalTestEntitlements,
} from "./verify-macos-dmg.mjs";

const TARGET_TRIPLE = "aarch64-apple-darwin";
const PRODUCT_NAME = "resume-ir";
const MAX_ENTITLEMENT_BYTES = 16 * 1024;
const MAX_TOOL_OUTPUT_BYTES = 64 * 1024;
const LIBRARY_VALIDATION_ENTITLEMENT =
  "com.apple.security.cs.disable-library-validation";
const CREDENTIAL_VARIABLES = [
  "APPLE_API_ISSUER",
  "APPLE_API_KEY",
  "APPLE_API_KEY_PATH",
  "APPLE_ID",
  "APPLE_PASSWORD",
  "APPLE_TEAM_ID",
  "APPLE_CERTIFICATE",
  "APPLE_CERTIFICATE_PASSWORD",
  "KEYCHAIN_PASSWORD",
];

function defaultRunner(command, args, options) {
  return spawnSync(command, args, {
    ...options,
    shell: false,
    stdio: "inherit",
  });
}

function defaultToolRunner(command, args) {
  return spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: MAX_TOOL_OUTPUT_BYTES,
    timeout: APPLE_TOOL_TIMEOUT_MS,
    shell: false,
    windowsHide: true,
  });
}

function succeeded(result) {
  return !result?.error && result?.status === 0;
}

async function validateEntitlementFile(entitlements) {
  let metadata;
  try {
    metadata = await lstat(entitlements);
  } catch {
    throw new Error("macOS internal-test entitlement file is invalid");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > MAX_ENTITLEMENT_BYTES
  ) {
    throw new Error("macOS internal-test entitlement file is invalid");
  }
  let source;
  try {
    source = await readFile(entitlements, "utf8");
  } catch {
    throw new Error("macOS internal-test entitlement file is invalid");
  }
  const keys = [
    ...source.matchAll(/<key>\s*([^<]+?)\s*<\/key>/g),
  ].map((match) => match[1]);
  const escaped = LIBRARY_VALIDATION_ENTITLEMENT.replaceAll(".", "\\.");
  if (
    keys.length !== 1 ||
    keys[0] !== LIBRARY_VALIDATION_ENTITLEMENT ||
    !new RegExp(
      `<key>\\s*${escaped}\\s*<\\/key>\\s*<true\\s*\\/>`,
    ).test(source)
  ) {
    throw new Error("macOS internal-test entitlement file is invalid");
  }
}

async function validateDmgFile(dmg, errorMessage) {
  let metadata;
  try {
    metadata = await lstat(dmg);
  } catch {
    throw new Error(errorMessage);
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > MAX_DMG_BYTES
  ) {
    throw new Error(errorMessage);
  }
}

async function mountedFilesystemAt(mountDirectory) {
  try {
    const [mountMetadata, parentMetadata] = await Promise.all([
      lstat(mountDirectory),
      lstat(path.dirname(mountDirectory)),
    ]);
    return mountMetadata.dev !== parentMetadata.dev;
  } catch {
    return false;
  }
}

async function detachOverlay({
  attached,
  attachAttempted,
  mountDirectory,
  runner,
  mountProbe,
}) {
  if (!attached && (!attachAttempted || !(await mountProbe(mountDirectory)))) {
    return undefined;
  }
  let result;
  try {
    result = await runner("hdiutil", ["detach", mountDirectory]);
  } catch {
    result = undefined;
  }
  if (succeeded(result)) return undefined;
  if (!(await mountProbe(mountDirectory))) return undefined;
  try {
    await runner("hdiutil", ["detach", mountDirectory, "-force"]);
  } catch {
    // The bounded recovery attempt is cleanup-only. A source mount that
    // needed forced detachment is never promoted.
  }
  if (await mountProbe(mountDirectory)) {
    return new Error("macOS internal-test DMG detach or cleanup failed");
  }
  return attached
    ? new Error("macOS internal-test DMG detach or cleanup failed")
    : undefined;
}

async function removeRewriteArtifacts(workspace, replacementDmg) {
  let cleanupFailed = false;
  for (const target of [workspace, replacementDmg]) {
    try {
      await rm(target, { recursive: target === workspace, force: true });
    } catch {
      cleanupFailed = true;
    }
  }
  return cleanupFailed
    ? new Error("macOS internal-test DMG detach or cleanup failed")
    : undefined;
}

function isStrictDescendant(root, candidate) {
  const relative = path.relative(root, candidate);
  return (
    relative.length > 0 &&
    relative !== ".." &&
    !relative.startsWith(`..${path.sep}`) &&
    !path.isAbsolute(relative)
  );
}

async function copyOptionalFile(source, destination, copyOne) {
  try {
    const metadata = await lstat(source);
    if (!metadata.isFile() || metadata.isSymbolicLink()) {
      throw new Error("invalid optional file");
    }
  } catch (error) {
    if (error?.code === "ENOENT") return;
    throw new Error("macOS internal-test DMG staging failed");
  }
  try {
    await copyOne(source, destination);
  } catch {
    throw new Error("macOS internal-test DMG staging failed");
  }
}

export async function stageMountedDmg({
  mountDirectory,
  appBundle,
  stagingDirectory,
  operations = { copyFile, cp, symlink },
}) {
  try {
    await mkdir(stagingDirectory);
    await copyOptionalFile(
      path.join(mountDirectory, ".DS_Store"),
      path.join(stagingDirectory, ".DS_Store"),
      operations.copyFile,
    );
    await operations.copyFile(
      path.join(mountDirectory, ".VolumeIcon.icns"),
      path.join(stagingDirectory, ".VolumeIcon.icns"),
    );
    await operations.cp(
      appBundle,
      path.join(stagingDirectory, "resume-ir.app"),
      {
        recursive: true,
        dereference: false,
        errorOnExist: true,
        force: false,
        preserveTimestamps: true,
        verbatimSymlinks: true,
      },
    );
    await operations.symlink(
      "/Applications",
      path.join(stagingDirectory, "Applications"),
    );
    return await validateMountedDmgLayout({
      mountDirectory: stagingDirectory,
    });
  } catch (error) {
    if (
      error instanceof Error &&
      error.message === "macOS internal-test DMG staging failed"
    ) {
      throw error;
    }
    throw new Error("macOS internal-test DMG staging failed");
  }
}

async function resolveNativeSigningTargets(appBundle) {
  const contents = path.join(appBundle, "Contents");
  const macosDirectory = path.join(contents, "MacOS");
  const infoPlist = path.join(contents, "Info.plist");
  const executableNames = [
    "resume-desktop",
    "resume-daemon",
    "resume-embedding-runtime",
    "resume-pdf-render-runtime",
  ];
  const components = [
    { target: appBundle, kind: "directory" },
    { target: contents, kind: "directory" },
    { target: macosDirectory, kind: "directory" },
    ...executableNames.map((name) => ({
      target: path.join(macosDirectory, name),
      kind: "executable",
    })),
  ];
  try {
    for (const { target, kind } of components) {
      const metadata = await lstat(target);
      if (
        metadata.isSymbolicLink() ||
        (kind === "directory" && !metadata.isDirectory()) ||
        (kind === "executable" &&
          (!metadata.isFile() ||
            metadata.size === 0 ||
            (metadata.mode & 0o111) === 0))
      ) {
        throw new Error("invalid component");
      }
    }
    const infoMetadata = await lstat(infoPlist);
    if (
      !infoMetadata.isFile() ||
      infoMetadata.isSymbolicLink() ||
      infoMetadata.size === 0 ||
      infoMetadata.size > 64 * 1024
    ) {
      throw new Error("invalid Info.plist");
    }
    const infoSource = await readFile(infoPlist, "utf8");
    if (
      !/<key>\s*CFBundleExecutable\s*<\/key>\s*<string>\s*resume-desktop\s*<\/string>/.test(
        infoSource,
      )
    ) {
      throw new Error("invalid main executable");
    }
    const [appRealPath, ...executableRealPaths] = await Promise.all([
      realpath(appBundle),
      ...executableNames.map((name) =>
        realpath(path.join(macosDirectory, name)),
      ),
    ]);
    if (
      executableRealPaths.some(
        (executable) => !isStrictDescendant(appRealPath, executable),
      )
    ) {
      throw new Error("escaped component");
    }
  } catch {
    throw new Error("macOS internal-test native signing path is invalid");
  }
  return Object.freeze({
    embeddingRuntime: path.join(
      macosDirectory,
      "resume-embedding-runtime",
    ),
  });
}

export async function applyMacosInternalTestEntitlements({
  dmg,
  entitlements,
  platform = process.platform,
  runner = defaultToolRunner,
  mountProbe = mountedFilesystemAt,
}) {
  if (
    platform !== "darwin" ||
    !path.isAbsolute(dmg) ||
    !path.isAbsolute(entitlements)
  ) {
    throw new Error("macOS internal-test entitlement arguments are invalid");
  }
  await Promise.all([
    validateDmgFile(dmg, "macOS internal-test DMG is invalid"),
    validateEntitlementFile(entitlements),
  ]);

  let workspace;
  try {
    workspace = await mkdtemp(
      path.join(path.dirname(dmg), ".resume-ir-macos-entitlements-"),
    );
  } catch {
    throw new Error("macOS internal-test entitlement workspace is unavailable");
  }
  const mountDirectory = path.join(workspace, "mount");
  const stagingDirectory = path.join(workspace, "staging");
  const replacementDmg = `${workspace}.dmg`;
  let attached = false;
  let attachAttempted = false;
  let operationError;
  let entitlementReceipt;
  try {
    await mkdir(mountDirectory);
    attachAttempted = true;
    const attach = await runner("hdiutil", [
      "attach",
      dmg,
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mountDirectory,
    ]);
    if (!succeeded(attach)) {
      throw new Error("macOS internal-test DMG attach failed");
    }
    attached = true;
    let mountedAppBundle;
    try {
      mountedAppBundle = await validateMountedDmgLayout({
        mountDirectory,
        allowFseventsd: true,
      });
    } catch (error) {
      if (error?.message === "DMG transient metadata is invalid") {
        throw new Error(
          "macOS internal-test DMG transient metadata is invalid",
        );
      }
      throw error;
    }
    const appBundle = await stageMountedDmg({
      mountDirectory,
      appBundle: mountedAppBundle,
      stagingDirectory,
    });
    const { embeddingRuntime } = await resolveNativeSigningTargets(appBundle);
    const signRuntime = await runner("codesign", [
      "--force",
      "--sign",
      "-",
      "--options",
      "runtime",
      "--timestamp=none",
      "--entitlements",
      entitlements,
      embeddingRuntime,
    ]);
    if (!succeeded(signRuntime)) {
      throw new Error("macOS internal-test entitlement signing failed");
    }
    const signApp = await runner("codesign", [
      "--force",
      "--sign",
      "-",
      "--options",
      "runtime",
      "--timestamp=none",
      appBundle,
    ]);
    if (!succeeded(signApp)) {
      throw new Error("macOS internal-test entitlement signing failed");
    }
    const signature = await runner("codesign", [
      "--verify",
      "--deep",
      "--strict",
      "--verbose=2",
      appBundle,
    ]);
    if (!succeeded(signature)) {
      throw new Error("macOS internal-test entitlement signing failed");
    }
    entitlementReceipt = await verifyMacosInternalTestEntitlements({
      appBundle,
      platform,
      runner,
    });
  } catch (error) {
    operationError =
      error instanceof Error && error.message.startsWith("macOS internal-test")
        ? error
        : new Error("macOS internal-test entitlement processing failed");
  }

  const detachError = await detachOverlay({
    attached,
    attachAttempted,
    mountDirectory,
    runner,
    mountProbe,
  });
  if (detachError || operationError) {
    const cleanupError = await removeRewriteArtifacts(workspace, replacementDmg);
    if (detachError || cleanupError) throw detachError ?? cleanupError;
    throw operationError;
  }

  let rebuildError;
  try {
    const create = await runner("hdiutil", [
      "create",
      "-quiet",
      "-volname",
      PRODUCT_NAME,
      "-srcfolder",
      stagingDirectory,
      "-fs",
      "HFS+",
      "-format",
      "UDZO",
      "-imagekey",
      "zlib-level=9",
      "-ov",
      replacementDmg,
    ]);
    if (!succeeded(create)) {
      throw new Error("macOS internal-test DMG rebuild failed");
    }
    await validateDmgFile(
      replacementDmg,
      "macOS internal-test DMG rebuild failed",
    );
  } catch {
    rebuildError = new Error("macOS internal-test DMG rebuild failed");
  }
  if (rebuildError) {
    const cleanupError = await removeRewriteArtifacts(workspace, replacementDmg);
    if (cleanupError) throw cleanupError;
    throw rebuildError;
  }

  let cleanupError;
  try {
    await rm(workspace, { recursive: true, force: true });
  } catch {
    cleanupError = new Error("macOS internal-test DMG detach or cleanup failed");
  }
  if (cleanupError) {
    await rm(replacementDmg, { force: true }).catch(() => {});
    throw cleanupError;
  }
  try {
    await rename(replacementDmg, dmg);
  } catch {
    await rm(replacementDmg, { force: true }).catch(() => {});
    throw new Error("macOS internal-test DMG replacement failed");
  }
  return entitlementReceipt;
}

async function readBoundedJson(file) {
  const source = await readFile(file, "utf8");
  if (Buffer.byteLength(source, "utf8") > 64 * 1024) {
    throw new Error("macOS test release config is invalid");
  }
  try {
    return JSON.parse(source);
  } catch {
    throw new Error("macOS test release config is invalid");
  }
}

export function resolveMacosTestReleasePaths(scriptUrl = import.meta.url) {
  const frontendRoot = fileURLToPath(new URL("..", scriptUrl));
  return Object.freeze({
    repoRoot: fileURLToPath(new URL("../../..", scriptUrl)),
    frontendRoot,
    runTauri: fileURLToPath(new URL("./run-tauri.mjs", scriptUrl)),
    baseConfig: path.join(frontendRoot, "src-tauri", "tauri.conf.json"),
    platformConfig: path.join(
      frontendRoot,
      "src-tauri",
      "tauri.macos.conf.json",
    ),
  });
}

export function createMacosInternalTestEnvironment(environment) {
  const sanitized = { ...environment, APPLE_SIGNING_IDENTITY: "-" };
  for (const variable of CREDENTIAL_VARIABLES) delete sanitized[variable];
  return sanitized;
}

export function createMacosInternalTestPlan({
  frontendRoot,
  platform = process.platform,
  baseConfig,
  platformConfig,
}) {
  const version = baseConfig?.version;
  const macOS = platformConfig?.bundle?.macOS;
  if (
    platform !== "darwin" ||
    !path.isAbsolute(frontendRoot) ||
    baseConfig?.productName !== PRODUCT_NAME ||
    typeof version !== "string" ||
    !/^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(version) ||
    !Array.isArray(platformConfig?.bundle?.targets) ||
    platformConfig.bundle.targets.length !== 1 ||
    platformConfig.bundle.targets[0] !== "dmg" ||
    macOS?.signingIdentity !== "-" ||
    macOS?.hardenedRuntime !== true
  ) {
    throw new Error("macOS test release config is invalid");
  }
  return Object.freeze({
    targetTriple: TARGET_TRIPLE,
    tauriArguments: Object.freeze([
      "build",
      "--target",
      TARGET_TRIPLE,
      "--bundles",
      "dmg",
      "--ci",
    ]),
    dmg: path.join(
      frontendRoot,
      "src-tauri",
      "target",
      TARGET_TRIPLE,
      "release",
      "bundle",
      "dmg",
      `${PRODUCT_NAME}_${version}_aarch64.dmg`,
    ),
    entitlements: path.join(
      frontendRoot,
      "src-tauri",
      "entitlements.internal-test.plist",
    ),
  });
}

function internalTestCandidatePath(dmg) {
  const parsed = path.parse(dmg);
  return path.join(
    parsed.dir,
    `.${parsed.name}.internal-test-candidate${parsed.ext}`,
  );
}

async function removeReleaseArtifacts(paths) {
  try {
    for (const target of paths) await rm(target, { force: true });
  } catch {
    throw new Error("macOS internal-test artifact cleanup failed");
  }
}

export async function buildMacosInternalTestRelease({
  repoRoot,
  frontendRoot,
  runTauri,
  baseConfig,
  platformConfig,
  environment = process.env,
  platform = process.platform,
  runner = defaultRunner,
  applyEntitlements = applyMacosInternalTestEntitlements,
  verifyDmg = verifyMacosDmg,
}) {
  if (!path.isAbsolute(repoRoot) || !path.isAbsolute(runTauri)) {
    throw new Error("macOS test release paths are invalid");
  }
  const plan = createMacosInternalTestPlan({
    frontendRoot,
    platform,
    baseConfig,
    platformConfig,
  });
  const candidateDmg = internalTestCandidatePath(plan.dmg);
  await removeReleaseArtifacts([plan.dmg, candidateDmg]);
  let build;
  try {
    build = runner(process.execPath, [runTauri, ...plan.tauriArguments], {
      cwd: frontendRoot,
      env: createMacosInternalTestEnvironment(environment),
    });
  } catch {
    build = undefined;
  }
  if (build?.error || build?.status !== 0) {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw new Error("macOS internal-test build failed");
  }
  try {
    await rename(plan.dmg, candidateDmg);
  } catch {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw new Error("macOS internal-test build artifact is invalid");
  }
  let receipt;
  let verificationError;
  try {
    const entitlementReceipt = await applyEntitlements({
      dmg: candidateDmg,
      entitlements: plan.entitlements,
      platform,
    });
    if (
      entitlementReceipt?.library_validation_entitlement_scope !==
      "embedding_runtime_only"
    ) {
      throw new Error("macOS internal-test entitlement verification failed");
    }
    receipt = await verifyDmg({
      repoRoot,
      targetTriple: plan.targetTriple,
      dmg: candidateDmg,
      platform,
    });
    if (
      receipt?.schema_version !== "resume-ir.macos-dmg-composition.v1" ||
      receipt?.distribution_signature !== "accepted" ||
      receipt?.distribution_profile !== "internal_test" ||
      receipt?.code_signature !== "ad_hoc_valid" ||
      receipt?.hardened_runtime !== true ||
      receipt?.library_validation_entitlement_scope !==
        "embedding_runtime_only" ||
      receipt?.notarization !== "not_requested" ||
      receipt?.tester_allow_list_required !== true ||
      !/^[a-f0-9]{64}$/.test(receipt?.dmg_sha256 ?? "") ||
      receipt?.digest_match !== true ||
      receipt?.release_claim !== "composition_only"
    ) {
      throw new Error("macOS internal-test verification failed");
    }
  } catch (error) {
    verificationError = error;
  }
  if (verificationError) {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw verificationError;
  }
  try {
    await rename(candidateDmg, plan.dmg);
  } catch {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw new Error("macOS internal-test artifact promotion failed");
  }
  return receipt;
}

async function main() {
  if (process.argv.length !== 2) {
    throw new Error("macOS test release does not accept arguments");
  }
  const paths = resolveMacosTestReleasePaths();
  const [baseConfig, platformConfig] = await Promise.all([
    readBoundedJson(paths.baseConfig),
    readBoundedJson(paths.platformConfig),
  ]);
  const receipt = await buildMacosInternalTestRelease({
    ...paths,
    baseConfig,
    platformConfig,
  });
  console.log(JSON.stringify(receipt));
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    console.error(`macos-test-release: ${error.message}`);
    process.exitCode = 1;
  });
}

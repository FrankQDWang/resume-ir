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
  verifyMacosInternalTestSignaturePolicy,
} from "./verify-macos-dmg.mjs";
import {
  verifyBundleComposition,
  writeBundleComposition,
} from "./macos-bundle-composition.mjs";
import {
  captureSourceIdentity,
  validateSourceIdentity,
  verifyImmutableSnapshotSource,
} from "./macos-source-identity.mjs";
import {
  MACOS_SYSTEM_TOOLS,
  runClosedSystemTool,
} from "./macos-system-tools.mjs";

const TARGET_TRIPLE = "aarch64-apple-darwin";
const PRODUCT_NAME = "resume-ir";
const DESKTOP_EXECUTABLE = "resume-desktop";
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
const SOURCE_IDENTITY_ENV = "RESUME_IR_MACOS_SOURCE_IDENTITY";

export const MACOS_TEST_RELEASE_FAILURE_SCHEMA =
  "resume-ir.macos-test-release-failure.v1";
export const MACOS_TEST_RELEASE_ERROR_CODES = Object.freeze([
  "release_artifact_cleanup_failed",
  "release_artifact_promotion_failed",
  "release_build_artifact_invalid",
  "release_build_tool_failed",
  "release_contract_invalid",
  "release_dmg_verification_failed",
  "release_entitlement_failed",
  "release_internal_failure",
  "release_postbuild_source_failed",
  "release_source_changed",
  "release_source_provenance_failed",
]);

export class MacosTestReleaseError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "MacosTestReleaseError";
    this.code = code;
  }
}

function releaseError(code, message) {
  return new MacosTestReleaseError(code, message);
}

export function runSilentReleaseBuild(command, args, options) {
  return spawnSync(command, args, {
    ...options,
    shell: false,
    stdio: "ignore",
  });
}

function defaultRunner(command, args, options) {
  return runSilentReleaseBuild(command, args, options);
}

function defaultToolRunner(command, args) {
  return runClosedSystemTool(command, args, {
    encoding: "utf8",
    maxBuffer: MAX_TOOL_OUTPUT_BYTES,
    timeout: APPLE_TOOL_TIMEOUT_MS,
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
  if (!attached) {
    if (!attachAttempted || !(await mountProbe(mountDirectory))) {
      return undefined;
    }
    let result;
    try {
      result = await runner(MACOS_SYSTEM_TOOLS.hdiutil, [
        "detach",
        mountDirectory,
        "-force",
      ]);
    } catch {
      result = undefined;
    }
    if (succeeded(result) || !(await mountProbe(mountDirectory))) {
      return undefined;
    }
    return new Error("macOS internal-test DMG detach or cleanup failed");
  }
  let result;
  try {
    result = await runner(MACOS_SYSTEM_TOOLS.hdiutil, [
      "detach",
      mountDirectory,
    ]);
  } catch {
    result = undefined;
  }
  if (succeeded(result)) return undefined;
  if (!(await mountProbe(mountDirectory))) return undefined;
  try {
    await runner(MACOS_SYSTEM_TOOLS.hdiutil, [
      "detach",
      mountDirectory,
      "-force",
    ]);
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
  source,
  platform = process.platform,
  runner = defaultToolRunner,
  mountProbe = mountedFilesystemAt,
}) {
  let validatedSource;
  try {
    validatedSource = validateSourceIdentity(source);
  } catch {
    throw new Error("macOS internal-test entitlement arguments are invalid");
  }
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
    const attach = await runner(MACOS_SYSTEM_TOOLS.hdiutil, [
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
    const signRuntime = await runner(MACOS_SYSTEM_TOOLS.codesign, [
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
    await writeBundleComposition({
      appBundle,
      targetTriple: TARGET_TRIPLE,
      source: validatedSource,
    });
    const signApp = await runner(MACOS_SYSTEM_TOOLS.codesign, [
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
    const signature = await runner(MACOS_SYSTEM_TOOLS.codesign, [
      "--verify",
      "--deep",
      "--strict",
      "--verbose=2",
      appBundle,
    ]);
    if (!succeeded(signature)) {
      throw new Error("macOS internal-test entitlement signing failed");
    }
    const entitlementScope = await verifyMacosInternalTestEntitlements({
      appBundle,
      platform,
      runner,
    });
    const composition = await verifyBundleComposition({
      appBundle,
      targetTriple: TARGET_TRIPLE,
      expectedSource: validatedSource,
      verifySignaturePolicy: ({ appBundle: boundAppBundle }) =>
        verifyMacosInternalTestSignaturePolicy({
          appBundle: boundAppBundle,
          platform,
          runner,
        }),
    });
    entitlementReceipt = {
      ...entitlementScope,
      app_composition_digest: composition.composition_digest,
    };
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
    const create = await runner(MACOS_SYSTEM_TOOLS.hdiutil, [
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
  const frontendRoot = path.resolve(fileURLToPath(new URL("..", scriptUrl)));
  return Object.freeze({
    repoRoot: path.resolve(fileURLToPath(new URL("../../..", scriptUrl))),
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

async function resolveMacosBuildSource({ repoRoot, environment }) {
  const serialized = environment?.[SOURCE_IDENTITY_ENV];
  if (serialized === undefined) {
    return (
      await captureSourceIdentity({
        repoRoot,
        authority: "exact_main_commit",
      })
    ).identity;
  }
  if (
    typeof serialized !== "string" ||
    Buffer.byteLength(serialized, "utf8") > 1024
  ) {
    throw new Error("macOS build source provenance is invalid");
  }
  let source;
  try {
    source = validateSourceIdentity(JSON.parse(serialized));
  } catch {
    throw new Error("macOS build source provenance is invalid");
  }
  if (
    source.authority !== "worktree_snapshot" ||
    path.basename(repoRoot) !==
      `${source.base_commit}-${source.source_tree_sha256}` ||
    path.basename(path.dirname(repoRoot)) !== "sources" ||
    path.basename(path.dirname(path.dirname(repoRoot))) !==
      "macos-worktree-build"
  ) {
    throw new Error("macOS build source provenance is invalid");
  }
  return verifyImmutableSnapshotSource({ repoRoot, expected: source });
}

export function createMacosInternalTestPlan({
  frontendRoot,
  platform = process.platform,
  baseConfig,
  platformConfig,
  cargoTargetDir = path.join(frontendRoot, "src-tauri", "target"),
}) {
  const version = baseConfig?.version;
  const macOS = platformConfig?.bundle?.macOS;
  if (
    platform !== "darwin" ||
    !path.isAbsolute(frontendRoot) ||
    !path.isAbsolute(cargoTargetDir) ||
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
      cargoTargetDir,
      TARGET_TRIPLE,
      "release",
      "bundle",
      "dmg",
      `${PRODUCT_NAME}_${version}_aarch64.dmg`,
    ),
    desktopExecutable: path.join(
      cargoTargetDir,
      TARGET_TRIPLE,
      "release",
      DESKTOP_EXECUTABLE,
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
    throw releaseError(
      "release_artifact_cleanup_failed",
      "macOS internal-test artifact cleanup failed",
    );
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
  verifySource = resolveMacosBuildSource,
}) {
  if (!path.isAbsolute(repoRoot) || !path.isAbsolute(runTauri)) {
    throw releaseError(
      "release_contract_invalid",
      "macOS test release paths are invalid",
    );
  }
  let plan;
  try {
    plan = createMacosInternalTestPlan({
      frontendRoot,
      platform,
      baseConfig,
      platformConfig,
      cargoTargetDir: environment.CARGO_TARGET_DIR,
    });
  } catch {
    throw releaseError(
      "release_contract_invalid",
      "macOS test release config is invalid",
    );
  }
  const candidateDmg = internalTestCandidatePath(plan.dmg);
  let source;
  try {
    source = validateSourceIdentity(
      await verifySource({ repoRoot, environment }),
    );
  } catch {
    await removeReleaseArtifacts([candidateDmg]);
    throw releaseError(
      "release_source_provenance_failed",
      "macOS build source provenance is invalid",
    );
  }
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
    throw releaseError(
      "release_build_tool_failed",
      "macOS internal-test build failed",
    );
  }
  try {
    await rename(plan.dmg, candidateDmg);
  } catch {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw releaseError(
      "release_build_artifact_invalid",
      "macOS internal-test build artifact is invalid",
    );
  }
  let receipt;
  try {
    const entitlementReceipt = await applyEntitlements({
      dmg: candidateDmg,
      entitlements: plan.entitlements,
      source,
      platform,
    });
    if (
      entitlementReceipt?.library_validation_entitlement_scope !==
      "embedding_runtime_only"
    ) {
      throw new Error("macOS internal-test entitlement verification failed");
    }
  } catch {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw releaseError(
      "release_entitlement_failed",
      "macOS internal-test entitlement verification failed",
    );
  }
  try {
    receipt = await verifyDmg({
      repoRoot,
      targetTriple: plan.targetTriple,
      dmg: candidateDmg,
      platform,
      expectedSource: source,
      expectedDesktop: plan.desktopExecutable,
    });
    if (
      receipt?.schema_version !== "resume-ir.macos-dmg-composition.v3" ||
      JSON.stringify(receipt?.source) !== JSON.stringify(source) ||
      receipt?.distribution_signature !== "accepted" ||
      receipt?.distribution_profile !== "internal_test" ||
      receipt?.code_signature !== "ad_hoc_valid" ||
      receipt?.hardened_runtime !== true ||
      receipt?.library_validation_entitlement_scope !==
        "embedding_runtime_only" ||
      receipt?.notarization !== "not_requested" ||
      receipt?.tester_allow_list_required !== true ||
      !/^[a-f0-9]{64}$/.test(receipt?.dmg_sha256 ?? "") ||
      !/^[a-f0-9]{64}$/.test(receipt?.app_composition_digest ?? "") ||
      receipt?.digest_match !== true ||
      receipt?.release_claim !== "composition_only"
    ) {
      throw new Error("macOS internal-test verification failed");
    }
  } catch (error) {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw releaseError(
      "release_dmg_verification_failed",
      error instanceof Error
        ? error.message
        : "macOS internal-test verification failed",
    );
  }
  let finalSource;
  try {
    finalSource = validateSourceIdentity(
      await verifySource({ repoRoot, environment }),
    );
  } catch {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw releaseError(
      "release_postbuild_source_failed",
      "macOS build source provenance is invalid",
    );
  }
  if (JSON.stringify(finalSource) !== JSON.stringify(source)) {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw releaseError(
      "release_source_changed",
      "macOS build source provenance is invalid",
    );
  }
  try {
    await rename(candidateDmg, plan.dmg);
  } catch {
    await removeReleaseArtifacts([plan.dmg, candidateDmg]);
    throw releaseError(
      "release_artifact_promotion_failed",
      "macOS internal-test artifact promotion failed",
    );
  }
  return receipt;
}

async function runDefaultRelease() {
  if (process.argv.length !== 2) {
    throw releaseError(
      "release_contract_invalid",
      "macOS test release does not accept arguments",
    );
  }
  const paths = resolveMacosTestReleasePaths();
  let baseConfig;
  let platformConfig;
  try {
    [baseConfig, platformConfig] = await Promise.all([
      readBoundedJson(paths.baseConfig),
      readBoundedJson(paths.platformConfig),
    ]);
  } catch {
    throw releaseError(
      "release_contract_invalid",
      "macOS test release config is invalid",
    );
  }
  return buildMacosInternalTestRelease({
    ...paths,
    baseConfig,
    platformConfig,
  });
}

export async function runMacosTestReleaseCli({
  runRelease = runDefaultRelease,
  write = (value) => process.stdout.write(value),
} = {}) {
  try {
    const receipt = await runRelease();
    write(`${JSON.stringify(receipt)}\n`);
    return 0;
  } catch (error) {
    const errorCode =
      error instanceof MacosTestReleaseError &&
      MACOS_TEST_RELEASE_ERROR_CODES.includes(error.code)
        ? error.code
        : "release_internal_failure";
    write(
      `${JSON.stringify({
        schema_version: MACOS_TEST_RELEASE_FAILURE_SCHEMA,
        outcome: "failed",
        error_code: errorCode,
      })}\n`,
    );
    return 1;
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  process.exitCode = await runMacosTestReleaseCli();
}

import { spawnSync } from "node:child_process";
import {
  lstat,
  mkdir,
  mkdtemp,
  realpath,
  rmdir,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  validateMountedDmgLayout,
  verifyAdHocSignedApp,
  verifyMacosDmg,
} from "./verify-macos-dmg.mjs";
import { verifyBundleComposition } from "./macos-bundle-composition.mjs";
import {
  createInstallReceipt,
  defaultApplicationSupportRoot,
  persistInstallReceipt,
  readInstallReceipt,
  removeInstallReceipt,
  verifyInstallReceipt,
} from "./macos-install-receipt.mjs";
import {
  advanceLifecycleJournal,
  createLifecycleJournal,
  persistLifecycleJournal,
  readLifecycleJournal,
} from "./macos-lifecycle-journal.mjs";
import {
  recoverInstallTransaction,
  recoverUninstallTransaction,
  rollbackInstallTransaction,
  rollbackUninstallTransaction,
} from "./macos-lifecycle-transaction.mjs";
import {
  assertNoLifecycleArtifacts,
  lifecycleWorkspacePaths,
  makeStagedAppDurable,
  publishDurableStage,
} from "./macos-lifecycle-workspace.mjs";
import { runWithMacosLifecycleLock } from "./macos-lifecycle-execution.mjs";
import { requireLifecycleLockCapability } from "./macos-lifecycle-lock.mjs";
import { verifyBundledSidecar } from "./verify-bundled-sidecar.mjs";

const APP_NAME = "resume-ir.app";
const EXPECTED_BUNDLE_ID = "local.resume-ir.desktop";
const EXPECTED_DISPLAY_NAME = "resume-ir";
const EXPECTED_ICON_FILE = "icon.icns";
const MAX_TOOL_OUTPUT_BYTES = 64 * 1024;
const MAX_METADATA_BYTES = 256;
const MIN_EVIDENCE_VERSION = [0, 1, 1];
const DEFAULT_LSREGISTER =
  "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

function supportsUpgradeEvidence(version) {
  if (
    typeof version !== "string" ||
    !/^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(version)
  ) {
    return false;
  }
  const parts = version.split(".").map(Number);
  if (parts.some((part) => !Number.isSafeInteger(part))) return false;
  for (let index = 0; index < parts.length; index += 1) {
    if (parts[index] !== MIN_EVIDENCE_VERSION[index]) {
      return parts[index] > MIN_EVIDENCE_VERSION[index];
    }
  }
  return true;
}

async function defaultRunner(command, args) {
  return spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: MAX_TOOL_OUTPUT_BYTES,
    shell: false,
    windowsHide: true,
  });
}

function succeeded(result) {
  return !result?.error && result?.status === 0;
}

async function requireApplicationsRoot(applicationsDirectory) {
  if (!path.isAbsolute(applicationsDirectory)) {
    throw new Error("Applications root is invalid");
  }
  let metadata;
  let resolved;
  try {
    [metadata, resolved] = await Promise.all([
      lstat(applicationsDirectory),
      realpath(applicationsDirectory),
    ]);
  } catch {
    throw new Error("Applications root is invalid");
  }
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    path.resolve(applicationsDirectory) !== resolved ||
    path.basename(resolved) !== "Applications"
  ) {
    throw new Error("Applications root is invalid");
  }
  return resolved;
}

async function targetExists(target) {
  try {
    await lstat(target);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw new Error("install target is unavailable");
  }
}

function requireIdentity(metadata, expectedVersion) {
  if (
    metadata?.bundle_id !== EXPECTED_BUNDLE_ID ||
    metadata?.version !== expectedVersion ||
    metadata?.display_name !== EXPECTED_DISPLAY_NAME ||
    metadata?.icon_file !== EXPECTED_ICON_FILE
  ) {
    throw new Error("App identity is invalid");
  }
}

function requireDmgReceipt(receipt, targetTriple) {
  if (
    receipt?.schema_version !== "resume-ir.macos-dmg-composition.v1" ||
    receipt?.target_triple !== targetTriple ||
    receipt?.release_claim !== "composition_only" ||
    receipt?.dmg_count !== 1 ||
    receipt?.app_bundle_count !== 1 ||
    receipt?.digest_match !== true ||
    receipt?.architecture !== "arm64" ||
    receipt?.build_machine_identity_path_markers !== 0 ||
    receipt?.distribution_signature !== "accepted" ||
    receipt?.distribution_profile !== "internal_test" ||
    receipt?.code_signature !== "ad_hoc_valid" ||
    receipt?.hardened_runtime !== true ||
    receipt?.library_validation_entitlement_scope !==
      "embedding_runtime_only" ||
    receipt?.notarization !== "not_requested" ||
    receipt?.tester_allow_list_required !== true ||
    !/^[a-f0-9]{64}$/.test(receipt?.dmg_sha256 ?? "") ||
    !/^[a-f0-9]{64}$/.test(receipt?.app_composition_digest ?? "")
  ) {
    throw new Error("DMG composition receipt is invalid");
  }
  return receipt;
}

function requireSignature(receipt) {
  if (
    receipt?.code_signature !== "ad_hoc_valid" ||
    receipt?.hardened_runtime !== true
  ) {
    throw new Error("installed App signature is invalid");
  }
  return receipt;
}

async function readPlistField({ appBundle, field, runner }) {
  const result = await runner("plutil", [
    "-extract",
    field,
    "raw",
    path.join(appBundle, "Contents", "Info.plist"),
  ]);
  if (!succeeded(result) || typeof result.stdout !== "string") {
    throw new Error("App metadata is invalid");
  }
  const value = result.stdout.trim();
  if (!value || Buffer.byteLength(value, "utf8") > MAX_METADATA_BYTES) {
    throw new Error("App metadata is invalid");
  }
  return value;
}

export async function inspectMacosAppBundle({
  appBundle,
  platform = process.platform,
  runner = defaultRunner,
}) {
  if (platform !== "darwin" || !path.isAbsolute(appBundle)) {
    throw new Error("App metadata arguments are invalid");
  }
  let metadata;
  try {
    metadata = await lstat(appBundle);
  } catch {
    throw new Error("App bundle is invalid");
  }
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw new Error("App bundle is invalid");
  }
  const [bundleId, version, displayName, iconFile] = await Promise.all([
    readPlistField({ appBundle, field: "CFBundleIdentifier", runner }),
    readPlistField({ appBundle, field: "CFBundleShortVersionString", runner }),
    readPlistField({ appBundle, field: "CFBundleDisplayName", runner }),
    readPlistField({ appBundle, field: "CFBundleIconFile", runner }),
  ]);
  return {
    bundle_id: bundleId,
    version,
    display_name: displayName,
    icon_file: iconFile,
  };
}

async function detachMount({ attached, mountDirectory, runner }) {
  let failed = false;
  if (attached) {
    let result = await runner("hdiutil", ["detach", mountDirectory]);
    if (!succeeded(result)) {
      result = await runner("hdiutil", ["detach", mountDirectory, "-force"]);
    }
    failed = !succeeded(result);
  }
  try {
    await rmdir(mountDirectory);
  } catch {
    failed = true;
  }
  if (failed) throw new Error("DMG detach or cleanup failed");
}

function validateArguments({
  repoRoot,
  targetTriple,
  applicationsDirectory,
  expectedVersion,
  platform,
}) {
  if (
    platform !== "darwin" ||
    targetTriple !== "aarch64-apple-darwin" ||
    !path.isAbsolute(repoRoot) ||
    !path.isAbsolute(applicationsDirectory) ||
    !supportsUpgradeEvidence(expectedVersion)
  ) {
    throw new Error("macOS lifecycle arguments are invalid");
  }
}

export async function installMacosDmg(options) {
  validateArguments({
    repoRoot: options.repoRoot,
    targetTriple: options.targetTriple,
    applicationsDirectory: options.applicationsDirectory,
    expectedVersion: options.expectedVersion ?? "0.1.1",
    platform: options.platform ?? process.platform,
  });
  if (
    !path.isAbsolute(options.dmg) ||
    !path.isAbsolute(options.temporaryRoot ?? os.tmpdir())
  ) {
    throw new Error("macOS lifecycle arguments are invalid");
  }
  return runWithMacosLifecycleLock({
    applicationSupportRoot: options.applicationSupportRoot,
    resolveApplicationSupportRoot:
      options.resolveApplicationSupportRoot ?? defaultApplicationSupportRoot,
    lifecycleLockTestRuntime: options.lifecycleLockTestRuntime,
    execute: ({ applicationSupportRoot, lifecycleLockCapability }) =>
      installMacosDmgLocked(
        { ...options, applicationSupportRoot },
        lifecycleLockCapability,
      ),
  });
}

async function installMacosDmgLocked({
  repoRoot,
  targetTriple,
  dmg,
  applicationsDirectory,
  expectedVersion = "0.1.1",
  temporaryRoot = os.tmpdir(),
  platform = process.platform,
  runner = defaultRunner,
  verifyDmg = verifyMacosDmg,
  validateLayout = validateMountedDmgLayout,
  inspectApp = inspectMacosAppBundle,
  verifyApp = verifyBundledSidecar,
  verifyComposition = verifyBundleComposition,
  verifySignature = verifyAdHocSignedApp,
  applicationSupportRoot,
  resolveApplicationSupportRoot = defaultApplicationSupportRoot,
  createReceipt = createInstallReceipt,
  readReceipt = readInstallReceipt,
  persistReceipt = persistInstallReceipt,
  readJournal = readLifecycleJournal,
  persistJournal = persistLifecycleJournal,
  launchServicesCommand = DEFAULT_LSREGISTER,
  filesystem = {},
}, lifecycleLockCapability) {
  requireLifecycleLockCapability(lifecycleLockCapability);
  validateArguments({
    repoRoot,
    targetTriple,
    applicationsDirectory,
    expectedVersion,
    platform,
  });
  if (!path.isAbsolute(dmg) || !path.isAbsolute(temporaryRoot)) {
    throw new Error("macOS lifecycle arguments are invalid");
  }
  const resolvedApplications = await requireApplicationsRoot(applicationsDirectory);
  const target = path.join(resolvedApplications, APP_NAME);
  const resolvedApplicationSupport =
    applicationSupportRoot ?? (await resolveApplicationSupportRoot());

  let installedSignature;
  const verifyNew = async (appBundle, journal) => {
    if (
      journal.new_version !== expectedVersion ||
      journal.target_triple !== targetTriple
    ) {
      throw new Error("install journal does not match requested release");
    }
    requireIdentity(
      await inspectApp({ appBundle, platform, runner }),
      expectedVersion,
    );
    await verifyApp({ repoRoot, targetTriple, appBundle });
    const composition = await verifyComposition({
      appBundle,
      targetTriple,
      expectedVersion,
    });
    verifyInstallReceipt({ receipt: journal.new_receipt, composition });
    installedSignature = requireSignature(
      await verifySignature({ appBundle, platform, runner }),
    );
    return composition;
  };
  const register = async (appBundle) => {
    const result = await runner(launchServicesCommand, ["-f", appBundle]);
    if (!succeeded(result)) throw new Error("LaunchServices registration failed");
  };
  const unregister = async (appBundle) => {
    const result = await runner(launchServicesCommand, ["-u", appBundle]);
    if (!succeeded(result)) throw new Error("installed App cleanup failed");
  };
  const transactionOptions = (journal) => ({
    journal,
    target,
    applicationsRoot: resolvedApplications,
    applicationSupportRoot: resolvedApplicationSupport,
    readReceipt,
    persistReceipt,
    persistJournal,
    verifyNew,
    register,
    unregister,
    filesystem,
    lifecycleLockCapability,
  });
  const result = () => ({
    schema_version: "resume-ir.macos-installed-app.v1",
    target_triple: targetTriple,
    app_bundle_count: 1,
    bundle_id_match: true,
    version: expectedVersion,
    display_name: EXPECTED_DISPLAY_NAME,
    icon_metadata: EXPECTED_ICON_FILE,
    runtime_composition_verified: true,
    composition_digest_match: true,
    install_receipt: "owner_only",
    launch_services_registered: true,
    user_data_removed: false,
    code_signature: installedSignature.code_signature,
    hardened_runtime: installedSignature.hardened_runtime,
    notarization: "not_requested",
    tester_allow_list_required: true,
    release_claim: "internal_test_install_only",
  });

  const interrupted = await readJournal({
    applicationSupportRoot: resolvedApplicationSupport,
    allowMissing: true,
  });
  if (interrupted) {
    const recovery = await recoverInstallTransaction(
      transactionOptions(interrupted),
    );
    if (recovery.outcome === "committed") return result();
  }
  if (await targetExists(target)) throw new Error("install target already exists");
  await assertNoLifecycleArtifacts(resolvedApplications);
  if (
    (await readReceipt({
      applicationSupportRoot: resolvedApplicationSupport,
      allowMissing: true,
    })) !== undefined
  ) {
    throw new Error("install receipt exists without an installed App");
  }

  const dmgReceipt = requireDmgReceipt(
    await verifyDmg({
      repoRoot,
      targetTriple,
      dmg,
      temporaryRoot,
      platform,
      runner,
    }),
    targetTriple,
  );

  const mountDirectory = await mkdtemp(path.join(temporaryRoot, "resume-ir-install-"));
  let attached = false;
  let journal;
  try {
    const attach = await runner("hdiutil", [
      "attach",
      dmg,
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mountDirectory,
    ]);
    if (!succeeded(attach)) throw new Error("DMG attach failed");
    attached = true;
    const sourceApp = await validateLayout({ mountDirectory });
    requireIdentity(
      await inspectApp({ appBundle: sourceApp, platform, runner }),
      expectedVersion,
    );
    await verifyApp({ repoRoot, targetTriple, appBundle: sourceApp });
    const sourceComposition = await verifyComposition({
      appBundle: sourceApp,
      targetTriple,
      expectedVersion,
    });
    if (sourceComposition.composition_digest !== dmgReceipt.app_composition_digest) {
      throw new Error("DMG composition receipt is invalid");
    }
    requireSignature(
      await verifySignature({ appBundle: sourceApp, platform, runner }),
    );
    const nextReceipt = createReceipt({
      composition: sourceComposition,
      dmgSha256: dmgReceipt.dmg_sha256,
    });
    journal = createLifecycleJournal({
      operation: "install",
      phase: "install_prepared",
      newVersion: expectedVersion,
      newCompositionDigest: sourceComposition.composition_digest,
      newReceipt: nextReceipt,
    });
    await persistJournal({
      applicationSupportRoot: resolvedApplicationSupport,
      journal,
    });
    const paths = lifecycleWorkspacePaths({
      applicationsRoot: resolvedApplications,
      operation: "install",
      transactionId: journal.transaction_id,
    });
    try {
      await (filesystem.mkdir ?? mkdir)(paths.partial);
    } catch (error) {
      if (error?.code === "EEXIST") {
        throw new Error("install workspace already exists");
      }
      throw new Error("install workspace is unavailable");
    }
    const copiedResult = await runner("ditto", [sourceApp, paths.partial]);
    if (!succeeded(copiedResult)) throw new Error("App copy failed");
    await verifyNew(paths.partial, journal);
    await makeStagedAppDurable({
      appBundle: paths.partial,
      applicationsRoot: resolvedApplications,
    });
    await verifyNew(paths.partial, journal);
    journal = advanceLifecycleJournal({
      journal,
      phase: "install_before_stage_publish",
    });
    await persistJournal({
      applicationSupportRoot: resolvedApplicationSupport,
      journal,
    });
    await publishDurableStage({
      partial: paths.partial,
      ready: paths.ready,
      applicationsRoot: resolvedApplications,
      move: filesystem.rename,
    });
    journal = advanceLifecycleJournal({
      journal,
      phase: "install_stage_ready",
    });
    await persistJournal({
      applicationSupportRoot: resolvedApplicationSupport,
      journal,
    });
    await detachMount({ attached, mountDirectory, runner });
    attached = false;
    await recoverInstallTransaction(transactionOptions(journal));
    return result();
  } catch (error) {
    let cleanupError;
    if (attached) {
      try {
        await detachMount({ attached, mountDirectory, runner });
      } catch {
        cleanupError = new Error("DMG detach or cleanup failed");
      }
    }
    if (journal) {
      try {
        const current = await readJournal({
          applicationSupportRoot: resolvedApplicationSupport,
        });
        await rollbackInstallTransaction(transactionOptions(current));
      } catch {
        cleanupError = new Error("macOS install rollback failed");
      }
    }
    if (cleanupError) throw cleanupError;
    throw error;
  }
}

export async function uninstallMacosApp(options) {
  validateArguments({
    repoRoot: options.repoRoot,
    targetTriple: options.targetTriple,
    applicationsDirectory: options.applicationsDirectory,
    expectedVersion: options.expectedVersion ?? "0.1.1",
    platform: options.platform ?? process.platform,
  });
  return runWithMacosLifecycleLock({
    applicationSupportRoot: options.applicationSupportRoot,
    resolveApplicationSupportRoot:
      options.resolveApplicationSupportRoot ?? defaultApplicationSupportRoot,
    lifecycleLockTestRuntime: options.lifecycleLockTestRuntime,
    execute: ({ applicationSupportRoot, lifecycleLockCapability }) =>
      uninstallMacosAppLocked(
        { ...options, applicationSupportRoot },
        lifecycleLockCapability,
      ),
  });
}

async function uninstallMacosAppLocked({
  repoRoot,
  targetTriple,
  applicationsDirectory,
  expectedVersion = "0.1.1",
  platform = process.platform,
  runner = defaultRunner,
  inspectApp = inspectMacosAppBundle,
  verifyComposition = verifyBundleComposition,
  verifySignature = verifyAdHocSignedApp,
  applicationSupportRoot,
  resolveApplicationSupportRoot = defaultApplicationSupportRoot,
  readReceipt = readInstallReceipt,
  verifyReceipt = verifyInstallReceipt,
  removeReceipt = removeInstallReceipt,
  persistReceipt = persistInstallReceipt,
  readJournal = readLifecycleJournal,
  persistJournal = persistLifecycleJournal,
  launchServicesCommand = DEFAULT_LSREGISTER,
  filesystem = {},
}, lifecycleLockCapability) {
  requireLifecycleLockCapability(lifecycleLockCapability);
  validateArguments({
    repoRoot,
    targetTriple,
    applicationsDirectory,
    expectedVersion,
    platform,
  });
  const resolvedApplications = await requireApplicationsRoot(applicationsDirectory);
  const target = path.join(resolvedApplications, APP_NAME);
  const resolvedApplicationSupport =
    applicationSupportRoot ?? (await resolveApplicationSupportRoot());
  const verifyOld = async (appBundle, journal) => {
    if (
      journal.old_version !== expectedVersion ||
      journal.target_triple !== targetTriple
    ) {
      throw new Error("uninstall journal does not match requested release");
    }
    requireIdentity(
      await inspectApp({ appBundle, platform, runner }),
      expectedVersion,
    );
    const composition = await verifyComposition({
      appBundle,
      targetTriple,
      expectedVersion,
    });
    requireSignature(
      await verifySignature({ appBundle, platform, runner }),
    );
    verifyReceipt({ receipt: journal.old_receipt, composition });
    return composition;
  };
  const register = async (appBundle) => {
    const result = await runner(launchServicesCommand, ["-f", appBundle]);
    if (!succeeded(result)) throw new Error("LaunchServices registration failed");
  };
  const unregister = async (appBundle) => {
    const result = await runner(launchServicesCommand, ["-u", appBundle]);
    if (!succeeded(result)) {
      throw new Error("LaunchServices unregistration failed");
    }
  };
  const transactionOptions = (journal) => ({
    journal,
    target,
    applicationsRoot: resolvedApplications,
    applicationSupportRoot: resolvedApplicationSupport,
    readReceipt,
    persistReceipt,
    removeReceipt,
    persistJournal,
    verifyOld,
    register,
    unregister,
    filesystem,
    lifecycleLockCapability,
  });
  const result = {
    schema_version: "resume-ir.macos-uninstall.v1",
    app_bundle_removed: true,
    launch_services_unregistered: true,
    user_data_removed: false,
    release_claim: "local_uninstall_only",
  };

  const interrupted = await readJournal({
    applicationSupportRoot: resolvedApplicationSupport,
    allowMissing: true,
  });
  if (interrupted) {
    const recovery = await recoverUninstallTransaction(
      transactionOptions(interrupted),
    );
    if (recovery.outcome === "committed") return result;
  }
  if (!(await targetExists(target))) throw new Error("installed App is missing");
  await assertNoLifecycleArtifacts(resolvedApplications);
  const receipt = await readReceipt({
    applicationSupportRoot: resolvedApplicationSupport,
  });
  const initialJournal = createLifecycleJournal({
    operation: "uninstall",
    phase: "uninstall_prepared",
    oldVersion: expectedVersion,
    oldCompositionDigest: receipt.composition_digest,
    oldReceipt: receipt,
  });
  await verifyOld(target, initialJournal);
  await persistJournal({
    applicationSupportRoot: resolvedApplicationSupport,
    journal: initialJournal,
  });
  try {
    await recoverUninstallTransaction(transactionOptions(initialJournal));
  } catch (error) {
    try {
      const current = await readJournal({
        applicationSupportRoot: resolvedApplicationSupport,
      });
      await rollbackUninstallTransaction(transactionOptions(current));
    } catch {
      throw new Error("macOS uninstall rollback failed");
    }
    throw error;
  }
  return result;
}

function parseArguments(args) {
  const action = args[0];
  if (!["install", "uninstall"].includes(action)) {
    throw new Error("invalid macOS lifecycle arguments");
  }
  const values = new Map();
  for (let index = 1; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (
      !["--target", "--dmg", "--applications", "--version"].includes(key) ||
      !value ||
      values.has(key)
    ) {
      throw new Error("invalid macOS lifecycle arguments");
    }
    values.set(key, value);
  }
  if (
    !values.has("--target") ||
    !values.has("--applications") ||
    !values.has("--version") ||
    (action === "install" && !values.has("--dmg")) ||
    (action === "uninstall" && values.has("--dmg"))
  ) {
    throw new Error("invalid macOS lifecycle arguments");
  }
  return {
    action,
    targetTriple: values.get("--target"),
    dmg: values.get("--dmg"),
    applicationsDirectory: values.get("--applications"),
    expectedVersion: values.get("--version"),
  };
}

async function main() {
  const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
  const { action, ...args } = parseArguments(process.argv.slice(2));
  const receipt =
    action === "install"
      ? await installMacosDmg({ repoRoot, ...args })
      : await uninstallMacosApp({ repoRoot, ...args });
  console.log(JSON.stringify(receipt));
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    console.error(`macos-install-lifecycle: ${error.message}`);
    process.exitCode = 1;
  });
}

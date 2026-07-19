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

import { inspectMacosAppBundle } from "./macos-install-lifecycle.mjs";
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
  verifyInstallReceipt,
} from "./macos-install-receipt.mjs";
import {
  advanceLifecycleJournal,
  createLifecycleJournal,
  persistLifecycleJournal,
  readLifecycleJournal,
} from "./macos-lifecycle-journal.mjs";
import {
  recoverUpgradeTransaction,
  rollbackUpgradeTransaction,
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
const MIN_EVIDENCE_VERSION = "0.1.1";
const DEFAULT_LSREGISTER =
  "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

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

function parseVersion(version) {
  if (
    typeof version !== "string" ||
    version.length > 64 ||
    !/^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(version)
  ) {
    throw new Error("version is invalid");
  }
  const components = version.split(".").map(Number);
  if (components.some((value) => !Number.isSafeInteger(value))) {
    throw new Error("version is invalid");
  }
  return components;
}

export function compareThreePartVersions(left, right) {
  const leftParts = parseVersion(left);
  const rightParts = parseVersion(right);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] !== rightParts[index]) {
      return leftParts[index] > rightParts[index] ? 1 : -1;
    }
  }
  return 0;
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
    path.basename(resolved) !== "Applications" ||
    path.basename(path.normalize(applicationsDirectory)) !== "Applications"
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
    throw new Error("upgrade target is unavailable");
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
    receipt?.distribution_signature !== "accepted" ||
    receipt?.distribution_profile !== "internal_test" ||
    receipt?.code_signature !== "ad_hoc_valid" ||
    receipt?.hardened_runtime !== true ||
    receipt?.library_validation_entitlement_scope !==
      "embedding_runtime_only" ||
    receipt?.notarization !== "not_requested" ||
    receipt?.tester_allow_list_required !== true ||
    receipt?.dmg_count !== 1 ||
    receipt?.app_bundle_count !== 1 ||
    receipt?.digest_match !== true ||
    receipt?.architecture !== "arm64" ||
    receipt?.build_machine_identity_path_markers !== 0 ||
    !/^[a-f0-9]{64}$/.test(receipt?.dmg_sha256 ?? "") ||
    !/^[a-f0-9]{64}$/.test(receipt?.app_composition_digest ?? "")
  ) {
    throw new Error("DMG composition receipt is invalid");
  }
}

function requireSignature(receipt) {
  if (
    receipt?.code_signature !== "ad_hoc_valid" ||
    receipt?.hardened_runtime !== true
  ) {
    throw new Error("App signature is invalid");
  }
  return receipt;
}

function requireAppReceipt(receipt) {
  if (
    receipt?.digest_match !== true ||
    receipt?.architecture !== "arm64" ||
    receipt?.build_machine_identity_path_markers !== 0
  ) {
    throw new Error("App runtime composition is invalid");
  }
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
  dmg,
  applicationsDirectory,
  installedVersion,
  candidateVersion,
  temporaryRoot,
  platform,
}) {
  if (
    platform !== "darwin" ||
    targetTriple !== "aarch64-apple-darwin" ||
    !path.isAbsolute(repoRoot) ||
    !path.isAbsolute(dmg) ||
    !path.isAbsolute(applicationsDirectory) ||
    !path.isAbsolute(temporaryRoot)
  ) {
    throw new Error("macOS upgrade arguments are invalid");
  }
  if (compareThreePartVersions(candidateVersion, installedVersion) <= 0) {
    throw new Error("candidate version is not newer");
  }
  if (compareThreePartVersions(installedVersion, MIN_EVIDENCE_VERSION) < 0) {
    throw new Error("installed App predates upgrade evidence");
  }
}

async function createExclusiveDirectory(target, makeDirectory) {
  try {
    await makeDirectory(target);
  } catch {
    throw new Error("upgrade workspace is unavailable");
  }
}

async function verifyCandidateApp({
  appBundle,
  expectedVersion,
  expectedCompositionDigest,
  repoRoot,
  targetTriple,
  platform,
  runner,
  inspectApp,
  verifyApp,
  verifyComposition,
  verifySignature,
}) {
  requireIdentity(
    await inspectApp({ appBundle, platform, runner }),
    expectedVersion,
  );
  requireAppReceipt(await verifyApp({ repoRoot, targetTriple, appBundle }));
  const composition = await verifyComposition({
    appBundle,
    targetTriple,
    expectedVersion,
  });
  if (composition.composition_digest !== expectedCompositionDigest) {
    throw new Error("candidate App composition is invalid");
  }
  requireSignature(await verifySignature({ appBundle, platform, runner }));
  return composition;
}

async function verifyInstalledApp({
  appBundle,
  expectedVersion,
  expectedReceipt,
  targetTriple,
  platform,
  runner,
  inspectApp,
  verifyComposition,
  verifySignature,
  verifyReceipt,
}) {
  requireIdentity(
    await inspectApp({ appBundle, platform, runner }),
    expectedVersion,
  );
  const composition = await verifyComposition({
    appBundle,
    targetTriple,
    expectedVersion,
  });
  requireSignature(await verifySignature({ appBundle, platform, runner }));
  verifyReceipt({ receipt: expectedReceipt, composition });
  return composition;
}

export async function upgradeMacosDmg(options) {
  validateArguments({
    repoRoot: options.repoRoot,
    targetTriple: options.targetTriple,
    dmg: options.dmg,
    applicationsDirectory: options.applicationsDirectory,
    installedVersion: options.installedVersion,
    candidateVersion: options.candidateVersion,
    temporaryRoot: options.temporaryRoot ?? os.tmpdir(),
    platform: options.platform ?? process.platform,
  });
  return runWithMacosLifecycleLock({
    applicationSupportRoot: options.applicationSupportRoot,
    resolveApplicationSupportRoot:
      options.resolveApplicationSupportRoot ?? defaultApplicationSupportRoot,
    lifecycleLockTestRuntime: options.lifecycleLockTestRuntime,
    execute: ({ applicationSupportRoot, lifecycleLockCapability }) =>
      upgradeMacosDmgLocked(
        { ...options, applicationSupportRoot },
        lifecycleLockCapability,
      ),
  });
}

async function upgradeMacosDmgLocked({
  repoRoot,
  targetTriple,
  dmg,
  applicationsDirectory,
  installedVersion,
  candidateVersion,
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
  readReceipt = readInstallReceipt,
  verifyReceipt = verifyInstallReceipt,
  createReceipt = createInstallReceipt,
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
    dmg,
    applicationsDirectory,
    installedVersion,
    candidateVersion,
    temporaryRoot,
    platform,
  });
  const applicationsRoot = await requireApplicationsRoot(applicationsDirectory);
  const target = path.join(applicationsRoot, APP_NAME);
  const makeDirectory = filesystem.mkdir ?? mkdir;
  const resolvedApplicationSupport =
    applicationSupportRoot ?? (await resolveApplicationSupportRoot());
  const verifyOld = async (appBundle, journal) => {
    if (
      journal.old_version !== installedVersion ||
      journal.target_triple !== targetTriple
    ) {
      throw new Error("upgrade journal does not match installed release");
    }
    return verifyInstalledApp({
      appBundle,
      expectedVersion: installedVersion,
      expectedReceipt: journal.old_receipt,
      targetTriple,
      platform,
      runner,
      inspectApp,
      verifyComposition,
      verifySignature,
      verifyReceipt,
    });
  };
  const verifyNew = async (appBundle, journal) => {
    if (
      journal.new_version !== candidateVersion ||
      journal.target_triple !== targetTriple
    ) {
      throw new Error("upgrade journal does not match candidate release");
    }
    const composition = await verifyCandidateApp({
      appBundle,
      expectedVersion: candidateVersion,
      expectedCompositionDigest: journal.new_composition_digest,
      repoRoot,
      targetTriple,
      platform,
      runner,
      inspectApp,
      verifyApp,
      verifyComposition,
      verifySignature,
    });
    verifyReceipt({ receipt: journal.new_receipt, composition });
    return composition;
  };
  const classifyTarget = async (appBundle, journal) => {
    const metadata = await inspectApp({ appBundle, platform, runner });
    if (metadata.version === journal.old_version) {
      await verifyOld(appBundle, journal);
      return "old";
    }
    if (metadata.version === journal.new_version) {
      await verifyNew(appBundle, journal);
      return "new";
    }
    throw new Error("upgrade target does not match journal");
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
    applicationsRoot,
    applicationSupportRoot: resolvedApplicationSupport,
    readReceipt,
    persistReceipt,
    persistJournal,
    verifyOld,
    verifyNew,
    classifyTarget,
    register,
    unregister,
    filesystem,
    lifecycleLockCapability,
  });
  const result = {
    schema_version: "resume-ir.macos-app-upgrade.v1",
    target_triple: targetTriple,
    from_version: installedVersion,
    to_version: candidateVersion,
    app_bundle_count: 1,
    runtime_composition_verified: true,
    composition_digest_match: true,
    install_receipt: "owner_only",
    launch_services_registered: true,
    rollback_required: false,
    user_data_removed: false,
    distribution_signature: "accepted",
    release_claim: "local_upgrade_only",
  };

  const interrupted = await readJournal({
    applicationSupportRoot: resolvedApplicationSupport,
    allowMissing: true,
  });
  if (interrupted) {
    const recovery = await recoverUpgradeTransaction(
      transactionOptions(interrupted),
    );
    if (recovery.outcome === "committed") return result;
  }
  if (!(await targetExists(target))) throw new Error("installed App is missing");
  await assertNoLifecycleArtifacts(applicationsRoot);
  const oldReceipt = await readReceipt({
    applicationSupportRoot: resolvedApplicationSupport,
  });
  await verifyInstalledApp({
    appBundle: target,
    expectedVersion: installedVersion,
    expectedReceipt: oldReceipt,
    targetTriple,
    platform,
    runner,
    inspectApp,
    verifyComposition,
    verifySignature,
    verifyReceipt,
  });

  const composition = await verifyDmg({
    repoRoot,
    targetTriple,
    dmg,
    temporaryRoot,
    platform,
    runner,
  });
  requireDmgReceipt(composition, targetTriple);

  let mountDirectory;
  try {
    mountDirectory = await mkdtemp(path.join(temporaryRoot, "resume-ir-upgrade-"));
  } catch {
    throw new Error("DMG temporary mount is unavailable");
  }
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
    const sourceComposition = await verifyCandidateApp({
      appBundle: sourceApp,
      expectedVersion: candidateVersion,
      expectedCompositionDigest: composition.app_composition_digest,
      repoRoot,
      targetTriple,
      platform,
      runner,
      inspectApp,
      verifyApp,
      verifyComposition,
      verifySignature,
    });
    const nextReceipt = createReceipt({
      composition: sourceComposition,
      dmgSha256: composition.dmg_sha256,
    });
    journal = createLifecycleJournal({
      operation: "upgrade",
      phase: "upgrade_prepared",
      oldVersion: installedVersion,
      newVersion: candidateVersion,
      oldCompositionDigest: oldReceipt.composition_digest,
      newCompositionDigest: sourceComposition.composition_digest,
      oldReceipt,
      newReceipt: nextReceipt,
    });
    await persistJournal({
      applicationSupportRoot: resolvedApplicationSupport,
      journal,
    });
    const paths = lifecycleWorkspacePaths({
      applicationsRoot,
      operation: "upgrade",
      transactionId: journal.transaction_id,
    });
    await createExclusiveDirectory(paths.partial, makeDirectory);
    const copy = await runner("ditto", [sourceApp, paths.partial]);
    if (!succeeded(copy)) throw new Error("candidate App copy failed");
    await verifyNew(paths.partial, journal);
    await makeStagedAppDurable({
      appBundle: paths.partial,
      applicationsRoot,
    });
    await verifyNew(paths.partial, journal);
    journal = advanceLifecycleJournal({
      journal,
      phase: "upgrade_before_stage_publish",
    });
    await persistJournal({
      applicationSupportRoot: resolvedApplicationSupport,
      journal,
    });
    await publishDurableStage({
      partial: paths.partial,
      ready: paths.ready,
      applicationsRoot,
      move: filesystem.rename,
    });
    journal = advanceLifecycleJournal({
      journal,
      phase: "upgrade_stage_ready",
    });
    await persistJournal({
      applicationSupportRoot: resolvedApplicationSupport,
      journal,
    });
    await detachMount({ attached, mountDirectory, runner });
    attached = false;
    await recoverUpgradeTransaction(transactionOptions(journal));
    return result;
  } catch (error) {
    let cleanupError;
    if (attached) {
      try {
        await detachMount({ attached, mountDirectory, runner });
      } catch (detachError) {
        cleanupError = detachError;
      }
    }
    if (journal) {
      try {
        const current = await readJournal({
          applicationSupportRoot: resolvedApplicationSupport,
        });
        const recovery = await rollbackUpgradeTransaction(
          transactionOptions(current),
        );
        if (recovery.outcome === "committed") {
          throw new Error("macOS upgrade post-commit failure");
        }
      } catch (rollbackError) {
        if (rollbackError.message === "macOS upgrade post-commit failure") {
          throw rollbackError;
        }
        throw new Error("macOS upgrade rollback failed");
      }
    }
    if (cleanupError) throw cleanupError;
    throw error;
  }
}

function parseArguments(args) {
  const values = new Map();
  const allowed = new Set([
    "--target",
    "--dmg",
    "--applications",
    "--installed-version",
    "--candidate-version",
  ]);
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (!allowed.has(key) || !value || values.has(key)) {
      throw new Error("invalid macOS upgrade arguments");
    }
    values.set(key, value);
  }
  if (values.size !== allowed.size) {
    throw new Error("invalid macOS upgrade arguments");
  }
  return {
    targetTriple: values.get("--target"),
    dmg: values.get("--dmg"),
    applicationsDirectory: values.get("--applications"),
    installedVersion: values.get("--installed-version"),
    candidateVersion: values.get("--candidate-version"),
  };
}

async function main() {
  const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
  const receipt = await upgradeMacosDmg({
    repoRoot,
    ...parseArguments(process.argv.slice(2)),
  });
  console.log(JSON.stringify(receipt));
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    console.error(`macos-upgrade-lifecycle: ${error.message}`);
    process.exitCode = 1;
  });
}

// Current-schema reinstall transaction. Older installer receipts are rejected.
import {
  lstat,
  mkdir,
  realpath,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { inspectMacosAppBundle } from "./macos-install-lifecycle.mjs";
import {
  verifyMacosInternalTestSignaturePolicy,
  withVerifiedMacosDmg,
} from "./verify-macos-dmg.mjs";
import { verifyBundleComposition } from "./macos-bundle-composition.mjs";
import {
  createInstallReceipt,
  defaultApplicationSupportRoot,
  persistInstallReceipt,
  readInstallReceipt,
  verifyInstallReceipt,
} from "./macos-install-receipt.mjs";
import { validateSourceIdentity } from "./macos-source-identity.mjs";
import {
  advanceLifecycleJournal,
  createLifecycleJournal,
  persistLifecycleJournal,
  readLifecycleJournal,
} from "./macos-lifecycle-journal.mjs";
import {
  recoverReinstallTransaction,
  rollbackReinstallTransaction,
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
import {
  MACOS_SYSTEM_TOOLS,
  runClosedSystemTool,
} from "./macos-system-tools.mjs";
import { PRODUCT_VERSION } from "./product-version.mjs";

const APP_NAME = "resume-ir.app";
const EXPECTED_BUNDLE_ID = "local.resume-ir.desktop";
const EXPECTED_DISPLAY_NAME = "resume-ir";
const EXPECTED_ICON_FILE = "icon.icns";
const MAX_TOOL_OUTPUT_BYTES = 64 * 1024;
const CURRENT_VERSION = PRODUCT_VERSION;
const DEFAULT_LSREGISTER =
  "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

async function defaultSystemRunner(command, args) {
  return runClosedSystemTool(command, args, {
    encoding: "utf8",
    maxBuffer: MAX_TOOL_OUTPUT_BYTES,
    timeout: 120_000,
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
    throw new Error("reinstall target is unavailable");
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
  try {
    validateSourceIdentity(receipt?.source);
  } catch {
    throw new Error("DMG composition receipt is invalid");
  }
  if (
    receipt?.schema_version !== "resume-ir.macos-dmg-composition.v3" ||
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
  return receipt;
}

function requireSignature(receipt) {
  if (
    receipt?.code_signature !== "ad_hoc_valid" ||
    receipt?.hardened_runtime !== true ||
    receipt?.library_validation_entitlement_scope !==
      "embedding_runtime_only"
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
    throw new Error("macOS reinstall arguments are invalid");
  }
  const supportedReinstall =
    installedVersion === CURRENT_VERSION &&
    candidateVersion === CURRENT_VERSION;
  if (!supportedReinstall) {
    throw new Error("only the exact current-version reinstall is supported");
  }
}

async function createExclusiveDirectory(target, makeDirectory) {
  try {
    await makeDirectory(target);
  } catch {
    throw new Error("reinstall workspace is unavailable");
  }
}

async function verifySignedComposition({
  appBundle,
  expectedVersion,
  expectedCompositionDigest,
  expectedSource,
  targetTriple,
  platform,
  systemRunner,
  verifyComposition,
  verifySignaturePolicy,
}) {
  const signaturePolicy = requireSignature(
    await verifySignaturePolicy({
      appBundle,
      platform,
      runner: systemRunner,
    }),
  );
  const composition = await verifyComposition({
    appBundle,
    targetTriple,
    expectedVersion,
    expectedSource,
    verifySignaturePolicy: async () => signaturePolicy,
  });
  if (composition.composition_digest !== expectedCompositionDigest) {
    throw new Error("candidate App composition is invalid");
  }
  return composition;
}

async function verifyInstalledApp(options) {
  requireIdentity(
    await options.inspectApp({
      appBundle: options.appBundle,
      platform: options.platform,
      runner: options.systemRunner,
    }),
    options.expectedVersion,
  );
  return verifySignedComposition(options);
}

async function verifyCandidateApp(options) {
  requireIdentity(
    await options.inspectApp({
      appBundle: options.appBundle,
      platform: options.platform,
      runner: options.systemRunner,
    }),
    options.expectedVersion,
  );
  requireAppReceipt(
    await options.verifyApp({
      repoRoot: options.repoRoot,
      targetTriple: options.targetTriple,
      appBundle: options.appBundle,
    }),
  );
  return verifySignedComposition(options);
}

export async function reinstallMacosDmg(options) {
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
      reinstallMacosDmgLocked(
        { ...options, applicationSupportRoot },
        lifecycleLockCapability,
      ),
  });
}

async function reinstallMacosDmgLocked({
  repoRoot,
  targetTriple,
  dmg,
  applicationsDirectory,
  installedVersion,
  candidateVersion,
  temporaryRoot = os.tmpdir(),
  platform = process.platform,
  systemRunner = defaultSystemRunner,
  withVerifiedDmg = withVerifiedMacosDmg,
  inspectApp = inspectMacosAppBundle,
  verifyApp = verifyBundledSidecar,
  verifyComposition = verifyBundleComposition,
  verifySignaturePolicy = verifyMacosInternalTestSignaturePolicy,
  applicationSupportRoot,
  resolveApplicationSupportRoot = defaultApplicationSupportRoot,
  readCurrentReceipt = readInstallReceipt,
  verifyReceipt = verifyInstallReceipt,
  createReceipt = createInstallReceipt,
  replaceCurrentReceipt = persistInstallReceipt,
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
  const transactionPrefix = "reinstall";
  const readTransactionReceipt = ({ applicationSupportRoot }) =>
    readCurrentReceipt({ applicationSupportRoot, allowMissing: true });
  const verifyOld = async (appBundle, journal) => {
    if (
      journal.old_version !== installedVersion ||
      journal.target_triple !== targetTriple
    ) {
      throw new Error("reinstall journal does not match installed release");
    }
    const composition = await verifyInstalledApp({
      appBundle,
      expectedVersion: installedVersion,
      expectedCompositionDigest: journal.old_composition_digest,
      expectedSource: journal.old_receipt.source,
      targetTriple,
      platform,
      systemRunner,
      inspectApp,
      verifyComposition,
      verifySignaturePolicy,
    });
    verifyReceipt({ receipt: journal.old_receipt, composition });
    return composition;
  };
  const verifyNew = async (appBundle, journal) => {
    if (
      journal.new_version !== candidateVersion ||
      journal.target_triple !== targetTriple
    ) {
      throw new Error("reinstall journal does not match candidate release");
    }
    const composition = await verifyCandidateApp({
      appBundle,
      expectedVersion: candidateVersion,
      expectedCompositionDigest: journal.new_composition_digest,
      expectedSource: journal.new_receipt.source,
      repoRoot,
      targetTriple,
      platform,
      systemRunner,
      inspectApp,
      verifyApp,
      verifyComposition,
      verifySignaturePolicy,
    });
    verifyReceipt({ receipt: journal.new_receipt, composition });
    return composition;
  };
  const classifyTarget = async (appBundle, journal, workspaceState) => {
    const newTargetPhases = new Set([
      "reinstall_target_promoted",
      "reinstall_before_receipt_commit",
      "reinstall_receipt_committed",
      "reinstall_before_backup_cleanup",
      "reinstall_backup_tombstoned",
      "reinstall_complete",
    ]);
    const targetIsNew =
      workspaceState?.backupPresent === true ||
      newTargetPhases.has(journal.phase);
    if (targetIsNew) {
      await verifyNew(appBundle, journal);
      return "new";
    }
    await verifyOld(appBundle, journal);
    return "old";
  };
  const register = async (appBundle) => {
    const result = await systemRunner(launchServicesCommand, ["-f", appBundle]);
    if (!succeeded(result)) throw new Error("LaunchServices registration failed");
  };
  const unregister = async (appBundle) => {
    const result = await systemRunner(launchServicesCommand, ["-u", appBundle]);
    if (!succeeded(result)) {
      throw new Error("LaunchServices unregistration failed");
    }
  };
  const transactionOptions = (journal) => ({
    journal,
    target,
    applicationsRoot,
    applicationSupportRoot: resolvedApplicationSupport,
    readReceipt: readTransactionReceipt,
    persistReceipt: ({ applicationSupportRoot, receipt }) =>
      replaceCurrentReceipt({
        applicationSupportRoot,
        receipt,
        expectedReceipt: journal.old_receipt,
      }),
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
    schema_version: "resume-ir.macos-app-reinstall.v1",
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
    release_claim: "local_reinstall_only",
  };

  const interrupted = await readJournal({
    applicationSupportRoot: resolvedApplicationSupport,
    allowMissing: true,
  });
  if (interrupted) {
    const recovery = await recoverReinstallTransaction(
      transactionOptions(interrupted),
    );
    if (recovery.outcome === "committed") return result;
  }
  if (!(await targetExists(target))) throw new Error("installed App is missing");
  await assertNoLifecycleArtifacts(applicationsRoot);
  const oldReceipt = await readCurrentReceipt({
    applicationSupportRoot: resolvedApplicationSupport,
  });
  await verifyOld(target, {
    old_version: installedVersion,
    target_triple: targetTriple,
    old_composition_digest: oldReceipt.composition_digest,
    old_receipt: oldReceipt,
  });

  let journal;
  try {
    const paths = await withVerifiedDmg({
      repoRoot,
      targetTriple,
      dmg,
      temporaryRoot,
      platform,
      systemRunner,
      consumeVerifiedImage: async ({
        appBundle: sourceApp,
        appComposition: sourceComposition,
        receipt,
      }) => {
        const dmgReceipt = requireDmgReceipt(receipt, targetTriple);
        if (
          sourceComposition?.bundle_id !== EXPECTED_BUNDLE_ID ||
          sourceComposition?.version !== candidateVersion ||
          sourceComposition?.target_triple !== targetTriple ||
          JSON.stringify(sourceComposition?.source) !==
            JSON.stringify(dmgReceipt.source) ||
          sourceComposition?.composition_digest !==
            dmgReceipt.app_composition_digest
        ) {
          throw new Error("DMG composition receipt is invalid");
        }
        const nextReceipt = createReceipt({
          composition: sourceComposition,
          dmgSha256: dmgReceipt.dmg_sha256,
        });
        journal = createLifecycleJournal({
          operation: "reinstall",
          phase: `${transactionPrefix}_prepared`,
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
          operation: "reinstall",
          transactionId: journal.transaction_id,
        });
        await createExclusiveDirectory(paths.partial, makeDirectory);
        const copy = await systemRunner(MACOS_SYSTEM_TOOLS.ditto, [
          sourceApp,
          paths.partial,
        ]);
        if (!succeeded(copy)) throw new Error("candidate App copy failed");
        return paths;
      },
    });
    await verifyNew(paths.partial, journal);
    await makeStagedAppDurable({
      appBundle: paths.partial,
      applicationsRoot,
    });
    await verifyNew(paths.partial, journal);
    journal = advanceLifecycleJournal({
      journal,
      phase: `${transactionPrefix}_before_stage_publish`,
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
      phase: `${transactionPrefix}_stage_ready`,
    });
    await persistJournal({
      applicationSupportRoot: resolvedApplicationSupport,
      journal,
    });
    await recoverReinstallTransaction(transactionOptions(journal));
    return result;
  } catch (error) {
    if (journal) {
      try {
        const current = await readJournal({
          applicationSupportRoot: resolvedApplicationSupport,
        });
        const recovery = await rollbackReinstallTransaction(
          transactionOptions(current),
        );
        if (recovery.outcome === "committed") {
          throw new Error("macOS reinstall post-commit failure");
        }
      } catch (rollbackError) {
        if (
          rollbackError.message === "macOS reinstall post-commit failure"
        ) {
          throw rollbackError;
        }
        throw new Error("macOS reinstall rollback failed");
      }
    }
    throw error;
  }
}

export function parseReplacementArguments(args, operation) {
  if (operation !== "reinstall") {
    throw new Error("invalid macOS replacement arguments");
  }
  const invalid = `invalid macOS ${operation} arguments`;
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
      throw new Error(invalid);
    }
    values.set(key, value);
  }
  if (values.size !== allowed.size) {
    throw new Error(invalid);
  }
  return {
    targetTriple: values.get("--target"),
    dmg: values.get("--dmg"),
    applicationsDirectory: values.get("--applications"),
    installedVersion: values.get("--installed-version"),
    candidateVersion: values.get("--candidate-version"),
  };
}

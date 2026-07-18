import { spawnSync } from "node:child_process";
import {
  lstat,
  mkdir,
  mkdtemp,
  realpath,
  rename,
  rm,
  rmdir,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { inspectMacosAppBundle } from "./macos-install-lifecycle.mjs";
import {
  validateMountedDmgLayout,
  verifyMacosDmg,
} from "./verify-macos-dmg.mjs";
import { verifyBundledSidecar } from "./verify-bundled-sidecar.mjs";

const APP_NAME = "resume-ir.app";
const STAGE_NAME = ".resume-ir.app.upgrade-stage";
const BACKUP_NAME = ".resume-ir.app.upgrade-backup";
const EXPECTED_BUNDLE_ID = "local.resume-ir.desktop";
const EXPECTED_DISPLAY_NAME = "resume-ir";
const EXPECTED_ICON_FILE = "icon.icns";
const MAX_TOOL_OUTPUT_BYTES = 64 * 1024;
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
    !["accepted", "not_accepted"].includes(receipt?.distribution_signature) ||
    receipt?.dmg_count !== 1 ||
    receipt?.app_bundle_count !== 1 ||
    receipt?.digest_match !== true ||
    receipt?.architecture !== "arm64" ||
    receipt?.build_machine_identity_path_markers !== 0
  ) {
    throw new Error("DMG composition receipt is invalid");
  }
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
}

async function removeIfOwned(target, owned, remove) {
  if (owned && (await targetExists(target))) {
    await remove(target, { recursive: true, force: false });
  }
}

async function createExclusiveDirectory(target, makeDirectory) {
  try {
    await makeDirectory(target);
  } catch {
    throw new Error("upgrade workspace is unavailable");
  }
}

async function moveEntry(source, target, move, message) {
  try {
    await move(source, target);
  } catch {
    throw new Error(message);
  }
}

async function restoreOldApp({
  target,
  backup,
  installedVersion,
  platform,
  runner,
  inspectApp,
  verifyApp,
  launchServicesCommand,
  move,
  remove,
}) {
  if (await targetExists(target)) {
    await runner(launchServicesCommand, ["-u", target]);
    await remove(target, { recursive: true, force: false });
  }
  await moveEntry(backup, target, move, "old App restoration failed");
  requireIdentity(
    await inspectApp({ appBundle: target, platform, runner }),
    installedVersion,
  );
  requireAppReceipt(await verifyApp({ appBundle: target }));
  const registration = await runner(launchServicesCommand, ["-f", target]);
  if (!succeeded(registration)) throw new Error("old App registration failed");
}

export async function upgradeMacosDmg({
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
  launchServicesCommand = DEFAULT_LSREGISTER,
  filesystem = {},
}) {
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
  const stage = path.join(applicationsRoot, STAGE_NAME);
  const backup = path.join(applicationsRoot, BACKUP_NAME);
  const move = filesystem.rename ?? rename;
  const remove = filesystem.rm ?? rm;
  const makeDirectory = filesystem.mkdir ?? mkdir;

  if (!(await targetExists(target))) throw new Error("installed App is missing");
  if ((await targetExists(stage)) || (await targetExists(backup))) {
    throw new Error("upgrade workspace already exists");
  }
  requireIdentity(
    await inspectApp({ appBundle: target, platform, runner }),
    installedVersion,
  );
  requireAppReceipt(await verifyApp({ repoRoot, targetTriple, appBundle: target }));

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
  let stageOwned = false;
  let backupReserved = false;
  let backupOwned = false;
  let newTargetOwned = false;
  try {
    await createExclusiveDirectory(backup, makeDirectory);
    backupReserved = true;
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
      candidateVersion,
    );
    requireAppReceipt(
      await verifyApp({ repoRoot, targetTriple, appBundle: sourceApp }),
    );

    await createExclusiveDirectory(stage, makeDirectory);
    stageOwned = true;
    const copy = await runner("ditto", [sourceApp, stage]);
    if (!succeeded(copy)) throw new Error("candidate App copy failed");
    requireIdentity(
      await inspectApp({ appBundle: stage, platform, runner }),
      candidateVersion,
    );
    requireAppReceipt(await verifyApp({ repoRoot, targetTriple, appBundle: stage }));
    await detachMount({ attached, mountDirectory, runner });
    attached = false;

    await moveEntry(target, backup, move, "installed App backup failed");
    backupReserved = false;
    backupOwned = true;
    await moveEntry(stage, target, move, "upgrade promotion failed");
    stageOwned = false;
    newTargetOwned = true;
    requireIdentity(
      await inspectApp({ appBundle: target, platform, runner }),
      candidateVersion,
    );
    requireAppReceipt(await verifyApp({ repoRoot, targetTriple, appBundle: target }));
    const registration = await runner(launchServicesCommand, ["-f", target]);
    if (!succeeded(registration)) {
      throw new Error("LaunchServices registration failed");
    }
    try {
      await remove(backup, { recursive: true, force: false });
    } catch {
      throw new Error("old App backup cleanup failed");
    }
    backupOwned = false;
    return {
      schema_version: "resume-ir.macos-app-upgrade.v1",
      target_triple: targetTriple,
      from_version: installedVersion,
      to_version: candidateVersion,
      app_bundle_count: 1,
      runtime_composition_verified: true,
      launch_services_registered: true,
      rollback_required: false,
      user_data_removed: false,
      distribution_signature: composition.distribution_signature,
      release_claim: "local_upgrade_only",
    };
  } catch (error) {
    let cleanupError;
    if (attached) {
      try {
        await detachMount({ attached, mountDirectory, runner });
      } catch (detachError) {
        cleanupError = detachError;
      }
    }
    let rollbackError;
    if (backupOwned) {
      newTargetOwned = false;
      try {
        await restoreOldApp({
          target,
          backup,
          installedVersion,
          platform,
          runner,
          inspectApp,
          verifyApp: (args) => verifyApp({ repoRoot, targetTriple, ...args }),
          launchServicesCommand,
          move,
          remove,
        });
        backupOwned = false;
      } catch {
        rollbackError = new Error("macOS upgrade rollback failed");
      }
    }
    try {
      await removeIfOwned(stage, stageOwned, remove);
      await removeIfOwned(target, newTargetOwned, remove);
      await removeIfOwned(backup, backupReserved, remove);
    } catch {
      throw new Error("macOS upgrade cleanup failed");
    }
    if (rollbackError) throw rollbackError;
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

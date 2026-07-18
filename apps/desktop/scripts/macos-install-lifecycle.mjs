import { spawnSync } from "node:child_process";
import {
  lstat,
  mkdir,
  mkdtemp,
  realpath,
  rm,
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
import { verifyBundledSidecar } from "./verify-bundled-sidecar.mjs";

const APP_NAME = "resume-ir.app";
const EXPECTED_BUNDLE_ID = "local.resume-ir.desktop";
const EXPECTED_DISPLAY_NAME = "resume-ir";
const EXPECTED_ICON_FILE = "icon.icns";
const MAX_TOOL_OUTPUT_BYTES = 64 * 1024;
const MAX_METADATA_BYTES = 256;
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
    metadata.isSymbolicLink()
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
    !/^[0-9]+\.[0-9]+\.[0-9]+$/.test(expectedVersion)
  ) {
    throw new Error("macOS lifecycle arguments are invalid");
  }
}

export async function installMacosDmg({
  repoRoot,
  targetTriple,
  dmg,
  applicationsDirectory,
  expectedVersion = "0.1.0",
  temporaryRoot = os.tmpdir(),
  platform = process.platform,
  runner = defaultRunner,
  verifyDmg = verifyMacosDmg,
  validateLayout = validateMountedDmgLayout,
  inspectApp = inspectMacosAppBundle,
  verifyApp = verifyBundledSidecar,
  verifySignature = verifyAdHocSignedApp,
  launchServicesCommand = DEFAULT_LSREGISTER,
}) {
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
  if (await targetExists(target)) throw new Error("install target already exists");

  const composition = await verifyDmg({
    repoRoot,
    targetTriple,
    dmg,
    temporaryRoot,
    platform,
    runner,
  });
  if (
    composition?.schema_version !== "resume-ir.macos-dmg-composition.v1" ||
    composition?.release_claim !== "composition_only" ||
    composition?.distribution_signature !== "accepted" ||
    composition?.distribution_profile !== "internal_test" ||
    composition?.code_signature !== "ad_hoc_valid" ||
    composition?.hardened_runtime !== true ||
    composition?.notarization !== "not_requested" ||
    composition?.tester_allow_list_required !== true
  ) {
    throw new Error("DMG composition receipt is invalid");
  }

  const mountDirectory = await mkdtemp(path.join(temporaryRoot, "resume-ir-install-"));
  let attached = false;
  let targetOwned = false;
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
    try {
      await mkdir(target);
      targetOwned = true;
    } catch (error) {
      if (error?.code === "EEXIST") throw new Error("install target already exists");
      throw new Error("install target is unavailable");
    }
    const copiedResult = await runner("ditto", [sourceApp, target]);
    if (!succeeded(copiedResult)) throw new Error("App copy failed");
    requireIdentity(
      await inspectApp({ appBundle: target, platform, runner }),
      expectedVersion,
    );
    await verifyApp({ repoRoot, targetTriple, appBundle: target });
    await detachMount({ attached, mountDirectory, runner });
    attached = false;
    const registration = await runner(launchServicesCommand, ["-f", target]);
    if (!succeeded(registration)) {
      throw new Error("LaunchServices registration failed");
    }
    requireIdentity(
      await inspectApp({ appBundle: target, platform, runner }),
      expectedVersion,
    );
    await verifyApp({ repoRoot, targetTriple, appBundle: target });
    const signature = await verifySignature({ appBundle: target, platform, runner });
    if (
      signature?.code_signature !== "ad_hoc_valid" ||
      signature?.hardened_runtime !== true
    ) {
      throw new Error("installed App signature is invalid");
    }
    return {
      schema_version: "resume-ir.macos-installed-app.v1",
      target_triple: targetTriple,
      app_bundle_count: 1,
      bundle_id_match: true,
      version: expectedVersion,
      display_name: EXPECTED_DISPLAY_NAME,
      icon_metadata: EXPECTED_ICON_FILE,
      runtime_composition_verified: true,
      launch_services_registered: true,
      user_data_removed: false,
      code_signature: signature.code_signature,
      hardened_runtime: signature.hardened_runtime,
      notarization: "not_requested",
      tester_allow_list_required: true,
      release_claim: "internal_test_install_only",
    };
  } catch (error) {
    let cleanupError;
    if (attached) {
      try {
        await detachMount({ attached, mountDirectory, runner });
      } catch {
        cleanupError = new Error("DMG detach or cleanup failed");
      }
    }
    if (targetOwned && (await targetExists(target))) {
      await rm(target, { recursive: true, force: true });
    }
    if (cleanupError) throw cleanupError;
    throw error;
  }
}

export async function uninstallMacosApp({
  repoRoot,
  targetTriple,
  applicationsDirectory,
  expectedVersion = "0.1.0",
  platform = process.platform,
  runner = defaultRunner,
  inspectApp = inspectMacosAppBundle,
  verifyApp = verifyBundledSidecar,
  launchServicesCommand = DEFAULT_LSREGISTER,
}) {
  validateArguments({
    repoRoot,
    targetTriple,
    applicationsDirectory,
    expectedVersion,
    platform,
  });
  const resolvedApplications = await requireApplicationsRoot(applicationsDirectory);
  const target = path.join(resolvedApplications, APP_NAME);
  if (!(await targetExists(target))) throw new Error("installed App is missing");
  requireIdentity(
    await inspectApp({ appBundle: target, platform, runner }),
    expectedVersion,
  );
  await verifyApp({ repoRoot, targetTriple, appBundle: target });
  const unregister = await runner(launchServicesCommand, ["-u", target]);
  if (!succeeded(unregister)) {
    throw new Error("LaunchServices unregistration failed");
  }
  await rm(target, { recursive: true, force: false });
  return {
    schema_version: "resume-ir.macos-uninstall.v1",
    app_bundle_removed: true,
    launch_services_unregistered: true,
    user_data_removed: false,
    release_claim: "local_uninstall_only",
  };
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

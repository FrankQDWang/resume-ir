import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  installMacosDmg,
  inspectMacosAppBundle,
  uninstallMacosApp,
} from "./macos-install-lifecycle.mjs";

const EXPECTED_APP = {
  bundle_id: "local.resume-ir.desktop",
  version: "0.1.0",
  display_name: "resume-ir",
  icon_file: "icon.icns",
};

function dmgReceipt() {
  return {
    schema_version: "resume-ir.macos-dmg-composition.v1",
    release_claim: "composition_only",
    distribution_signature: "accepted",
    distribution_profile: "internal_test",
    code_signature: "ad_hoc_valid",
    hardened_runtime: true,
    notarization: "not_requested",
    tester_allow_list_required: true,
  };
}

const signatureReceipt = async () => ({
  code_signature: "ad_hoc_valid",
  hardened_runtime: true,
});

async function fixture(context) {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-install-test-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const applicationsDirectory = path.join(root, "Applications");
  const sourceApp = path.join(root, "source", "resume-ir.app");
  const dmg = path.join(root, "resume-ir.dmg");
  const temporaryRoot = path.join(root, "mounts");
  await mkdir(applicationsDirectory);
  await mkdir(sourceApp, { recursive: true });
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");
  return { root, applicationsDirectory, sourceApp, dmg, temporaryRoot };
}

function successfulRunner(sourceApp, calls, failureCommand) {
  return async (command, args) => {
    calls.push([command, ...args]);
    if (command === failureCommand) return { status: 1, stdout: "", stderr: "bounded" };
    if (
      failureCommand === "hdiutil-detach" &&
      command === "hdiutil" &&
      args[0] === "detach"
    ) {
      return { status: 1, stdout: "", stderr: "bounded" };
    }
    if (command === "hdiutil" && args[0] === "attach") {
      const mount = args.at(-1);
      await symlink(sourceApp, path.join(mount, "resume-ir.app"));
      await symlink("/Applications", path.join(mount, "Applications"));
    }
    if (command === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
    }
    if (command === "ditto") {
      await mkdir(args[1], { recursive: true });
      await writeFile(path.join(args[1], "copied"), "yes");
    }
    return { status: 0, stdout: "", stderr: "" };
  };
}

test("installs only after DMG verification and emits a bounded receipt", async (context) => {
  const values = await fixture(context);
  const calls = [];
  const inspected = [];
  const verifiedApps = [];
  const receipt = await installMacosDmg({
    repoRoot: values.root,
    targetTriple: "aarch64-apple-darwin",
    dmg: values.dmg,
    applicationsDirectory: values.applicationsDirectory,
    temporaryRoot: values.temporaryRoot,
    platform: "darwin",
    runner: successfulRunner(values.sourceApp, calls),
    verifyDmg: async () => dmgReceipt(),
    validateLayout: async ({ mountDirectory }) => path.join(mountDirectory, "resume-ir.app"),
    inspectApp: async ({ appBundle }) => {
      inspected.push(appBundle);
      return EXPECTED_APP;
    },
    verifyApp: async ({ appBundle }) => {
      verifiedApps.push(appBundle);
      return { digest_match: true, architecture: "arm64" };
    },
    verifySignature: signatureReceipt,
  });

  assert.equal(inspected.length, 3);
  assert.equal(verifiedApps.length, 2);
  assert.equal(calls.filter(([command]) => command === "ditto").length, 1);
  assert.equal(calls.filter(([command]) => command.endsWith("lsregister")).length, 1);
  assert.deepEqual(receipt, {
    schema_version: "resume-ir.macos-installed-app.v1",
    target_triple: "aarch64-apple-darwin",
    app_bundle_count: 1,
    bundle_id_match: true,
    version: "0.1.0",
    display_name: "resume-ir",
    icon_metadata: "icon.icns",
    runtime_composition_verified: true,
    launch_services_registered: true,
    user_data_removed: false,
    code_signature: "ad_hoc_valid",
    hardened_runtime: true,
    notarization: "not_requested",
    tester_allow_list_required: true,
    release_claim: "internal_test_install_only",
  });
});

test("rejects an existing target and symlinked Applications root", async (context) => {
  const values = await fixture(context);
  await mkdir(path.join(values.applicationsDirectory, "resume-ir.app"));
  await assert.rejects(
    installMacosDmg({
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      applicationsDirectory: values.applicationsDirectory,
      platform: "darwin",
    }),
    /install target already exists/,
  );

  const linkedRoot = path.join(values.root, "LinkedApplications");
  await symlink(values.applicationsDirectory, linkedRoot);
  await assert.rejects(
    installMacosDmg({
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      applicationsDirectory: linkedRoot,
      platform: "darwin",
    }),
    /Applications root is invalid/,
  );
});

test("removes a staged or installed App after copy, detach, or registration failure", async (context) => {
  for (const failureCommand of ["ditto", "hdiutil-detach", "lsregister"]) {
    const values = await fixture(context);
    const calls = [];
    await assert.rejects(
      installMacosDmg({
        repoRoot: values.root,
        targetTriple: "aarch64-apple-darwin",
        dmg: values.dmg,
        applicationsDirectory: values.applicationsDirectory,
        temporaryRoot: values.temporaryRoot,
        platform: "darwin",
        runner: successfulRunner(values.sourceApp, calls, failureCommand),
        verifyDmg: async () => dmgReceipt(),
        validateLayout: async ({ mountDirectory }) => path.join(mountDirectory, "resume-ir.app"),
        inspectApp: async () => EXPECTED_APP,
        verifyApp: async () => ({ digest_match: true, architecture: "arm64" }),
        verifySignature: signatureReceipt,
        launchServicesCommand: failureCommand === "lsregister" ? "lsregister" : undefined,
      }),
      failureCommand === "ditto"
        ? /App copy failed/
        : failureCommand === "hdiutil-detach"
          ? /DMG detach or cleanup failed/
          : /LaunchServices registration failed/,
    );
    await assert.rejects(readFile(path.join(values.applicationsDirectory, "resume-ir.app", "copied")));
  }
});

test("fails closed on bundle identity, version, or icon drift", async (context) => {
  const values = await fixture(context);
  const calls = [];
  await assert.rejects(
    installMacosDmg({
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      applicationsDirectory: values.applicationsDirectory,
      temporaryRoot: values.temporaryRoot,
      platform: "darwin",
      runner: successfulRunner(values.sourceApp, calls),
      verifyDmg: async () => dmgReceipt(),
      validateLayout: async ({ mountDirectory }) => path.join(mountDirectory, "resume-ir.app"),
      inspectApp: async () => ({ ...EXPECTED_APP, icon_file: "" }),
      verifyApp: async () => ({ digest_match: true, architecture: "arm64" }),
      verifySignature: signatureReceipt,
    }),
    /App identity is invalid/,
  );
});

test("uninstall removes only the verified App and preserves user data", async (context) => {
  const values = await fixture(context);
  const installed = path.join(values.applicationsDirectory, "resume-ir.app");
  const userData = path.join(values.root, "Library", "Application Support", "local.resume-ir.desktop");
  await mkdir(installed);
  await mkdir(userData, { recursive: true });
  await writeFile(path.join(userData, "sentinel"), "preserve");
  const receipt = await uninstallMacosApp({
    repoRoot: values.root,
    targetTriple: "aarch64-apple-darwin",
    applicationsDirectory: values.applicationsDirectory,
    platform: "darwin",
    runner: async () => ({ status: 0, stdout: "", stderr: "" }),
    inspectApp: async () => EXPECTED_APP,
    verifyApp: async () => ({ digest_match: true, architecture: "arm64" }),
  });
  await assert.rejects(readFile(path.join(installed, "copied")));
  assert.equal(await readFile(path.join(userData, "sentinel"), "utf8"), "preserve");
  assert.deepEqual(receipt, {
    schema_version: "resume-ir.macos-uninstall.v1",
    app_bundle_removed: true,
    launch_services_unregistered: true,
    user_data_removed: false,
    release_claim: "local_uninstall_only",
  });
});

test("reads only the expected bounded macOS bundle metadata", async (context) => {
  const values = await fixture(context);
  const calls = [];
  const fields = new Map([
    ["CFBundleIdentifier", "local.resume-ir.desktop"],
    ["CFBundleShortVersionString", "0.1.0"],
    ["CFBundleDisplayName", "resume-ir"],
    ["CFBundleIconFile", "icon.icns"],
  ]);
  const metadata = await inspectMacosAppBundle({
    appBundle: values.sourceApp,
    platform: "darwin",
    runner: async (command, args) => {
      calls.push([command, ...args]);
      return { status: 0, stdout: `${fields.get(args[1])}\n`, stderr: "" };
    },
  });
  assert.deepEqual(metadata, EXPECTED_APP);
  assert.equal(calls.length, 4);
  assert.ok(calls.every(([command]) => command === "plutil"));
});

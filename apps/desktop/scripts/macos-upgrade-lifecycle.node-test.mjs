import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rename,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  compareThreePartVersions,
  upgradeMacosDmg,
} from "./macos-upgrade-lifecycle.mjs";

const OLD_VERSION = "0.1.0";
const NEW_VERSION = "0.1.1";

function appReceipt() {
  return {
    digest_match: true,
    architecture: "arm64",
    build_machine_identity_path_markers: 0,
  };
}

function dmgReceipt() {
  return {
    schema_version: "resume-ir.macos-dmg-composition.v1",
    target_triple: "aarch64-apple-darwin",
    dmg_count: 1,
    app_bundle_count: 1,
    digest_match: true,
    architecture: "arm64",
    build_machine_identity_path_markers: 0,
    release_claim: "composition_only",
    distribution_signature: "not_accepted",
  };
}

async function fixture(context) {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-upgrade-test-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const applicationsDirectory = path.join(root, "Applications");
  const target = path.join(applicationsDirectory, "resume-ir.app");
  const sourceApp = path.join(root, "source", "resume-ir.app");
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  const userData = path.join(root, "user-data");
  await mkdir(target, { recursive: true });
  await mkdir(sourceApp, { recursive: true });
  await mkdir(temporaryRoot);
  await mkdir(userData);
  await writeFile(path.join(target, "version"), OLD_VERSION);
  await writeFile(path.join(sourceApp, "version"), NEW_VERSION);
  await writeFile(path.join(dmg), "synthetic-dmg");
  await writeFile(path.join(userData, "sentinel"), "preserve");
  return {
    root,
    applicationsDirectory,
    target,
    sourceApp,
    temporaryRoot,
    dmg,
    userData,
  };
}

async function inspectApp({ appBundle }) {
  return {
    bundle_id: "local.resume-ir.desktop",
    version: await readFile(path.join(appBundle, "version"), "utf8"),
    display_name: "resume-ir",
    icon_file: "icon.icns",
  };
}

function createRunner(sourceApp, calls, { failCopy = false, failDetach = false, failRegistrations = 0 } = {}) {
  let registrationCount = 0;
  return async (command, args) => {
    calls.push([command, ...args]);
    if (command === "ditto") {
      if (failCopy) return { status: 1, stdout: "", stderr: "bounded" };
      await writeFile(
        path.join(args[1], "version"),
        await readFile(path.join(sourceApp, "version"), "utf8"),
      );
    }
    if (command === "hdiutil" && args[0] === "detach" && failDetach) {
      return { status: 1, stdout: "", stderr: "bounded" };
    }
    if (command.endsWith("lsregister") && args[0] === "-f") {
      registrationCount += 1;
      if (registrationCount <= failRegistrations) {
        return { status: 1, stdout: "", stderr: "bounded" };
      }
    }
    return { status: 0, stdout: "", stderr: "" };
  };
}

function upgradeArguments(values, overrides = {}) {
  const calls = [];
  return {
    calls,
    args: {
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      applicationsDirectory: values.applicationsDirectory,
      installedVersion: OLD_VERSION,
      candidateVersion: NEW_VERSION,
      temporaryRoot: values.temporaryRoot,
      platform: "darwin",
      runner: createRunner(values.sourceApp, calls, overrides.runnerOptions),
      verifyDmg: async () => dmgReceipt(),
      validateLayout: async () => values.sourceApp,
      inspectApp,
      verifyApp: async () => appReceipt(),
      ...overrides,
    },
  };
}

async function assertOldState(values) {
  assert.equal(await readFile(path.join(values.target, "version"), "utf8"), OLD_VERSION);
  assert.equal(await readFile(path.join(values.userData, "sentinel"), "utf8"), "preserve");
  assert.deepEqual((await readdir(values.applicationsDirectory)).sort(), ["resume-ir.app"]);
}

test("upgrades a verified App through an exclusive sibling stage", async (context) => {
  const values = await fixture(context);
  const { args, calls } = upgradeArguments(values);
  const receipt = await upgradeMacosDmg(args);

  assert.equal(await readFile(path.join(values.target, "version"), "utf8"), NEW_VERSION);
  assert.equal(await readFile(path.join(values.userData, "sentinel"), "utf8"), "preserve");
  assert.deepEqual((await readdir(values.applicationsDirectory)).sort(), ["resume-ir.app"]);
  assert.equal(calls.filter(([command]) => command === "ditto").length, 1);
  assert.equal(calls.filter(([command]) => command.endsWith("lsregister")).length, 1);
  assert.deepEqual(receipt, {
    schema_version: "resume-ir.macos-app-upgrade.v1",
    target_triple: "aarch64-apple-darwin",
    from_version: OLD_VERSION,
    to_version: NEW_VERSION,
    app_bundle_count: 1,
    runtime_composition_verified: true,
    launch_services_registered: true,
    rollback_required: false,
    user_data_removed: false,
    distribution_signature: "not_accepted",
    release_claim: "local_upgrade_only",
  });
});

test("rejects same-version, downgrade, invalid versions, and reserved-name collisions", async (context) => {
  assert.equal(compareThreePartVersions("0.1.1", "0.1.0"), 1);
  assert.equal(compareThreePartVersions("0.1.0", "0.1.0"), 0);
  assert.equal(compareThreePartVersions("0.0.9", "0.1.0"), -1);
  assert.throws(() => compareThreePartVersions("0.1.0-beta", OLD_VERSION), /version is invalid/);
  assert.throws(() => compareThreePartVersions("00.1.0", OLD_VERSION), /version is invalid/);

  for (const candidateVersion of [OLD_VERSION, "0.0.9"]) {
    const values = await fixture(context);
    const { args } = upgradeArguments(values, { candidateVersion });
    await assert.rejects(upgradeMacosDmg(args), /candidate version is not newer/);
    await assertOldState(values);
  }

  const values = await fixture(context);
  await mkdir(path.join(values.applicationsDirectory, ".resume-ir.app.upgrade-stage"));
  const { args } = upgradeArguments(values);
  await assert.rejects(upgradeMacosDmg(args), /upgrade workspace already exists/);
  assert.equal(await readFile(path.join(values.userData, "sentinel"), "utf8"), "preserve");
});

test("rejects a symlinked Applications root without touching the App", async (context) => {
  const values = await fixture(context);
  const linkedRoot = path.join(values.root, "LinkedApplications");
  await symlink(values.applicationsDirectory, linkedRoot);
  const { args } = upgradeArguments(values, { applicationsDirectory: linkedRoot });
  await assert.rejects(upgradeMacosDmg(args), /Applications root is invalid/);
  await assertOldState(values);
});

test("leaves the old App untouched when copy or detach fails before the first rename", async (context) => {
  for (const runnerOptions of [{ failCopy: true }, { failDetach: true }]) {
    const values = await fixture(context);
    const { args } = upgradeArguments(values, { runnerOptions });
    await assert.rejects(upgradeMacosDmg(args));
    await assertOldState(values);
  }
});

test("restores the old App after promotion, post-verify, or registration failure", async (context) => {
  for (const failure of ["promotion", "post_verify", "registration"]) {
    const values = await fixture(context);
    let renameCount = 0;
    const filesystem = {
      rename: async (...args) => {
        renameCount += 1;
        if (failure === "promotion" && renameCount === 2) {
          throw new Error("synthetic promotion failure");
        }
        await rename(...args);
      },
    };
    const overrides = {
      filesystem,
      runnerOptions: failure === "registration" ? { failRegistrations: 1 } : undefined,
    };
    if (failure === "post_verify") {
      let verificationCount = 0;
      overrides.verifyApp = async () => {
        verificationCount += 1;
        if (verificationCount === 4) {
          throw new Error("synthetic post-verify failure");
        }
        return appReceipt();
      };
    }
    const { args } = upgradeArguments(values, overrides);
    await assert.rejects(
      upgradeMacosDmg(args),
      (error) => {
        assert.equal(error.message.includes(values.root), false);
        if (failure === "promotion") {
          assert.match(error.message, /upgrade promotion failed/);
        }
        return true;
      },
      failure,
    );
    await assertOldState(values);
  }
});

test("reports a rollback failure instead of claiming the upgrade succeeded", async (context) => {
  const values = await fixture(context);
  const { args } = upgradeArguments(values, {
    runnerOptions: { failRegistrations: 2 },
  });
  await assert.rejects(upgradeMacosDmg(args), /macOS upgrade rollback failed/);
  await assertOldState(values);
});

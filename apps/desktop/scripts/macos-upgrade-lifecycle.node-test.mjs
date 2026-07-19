import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readFile,
  realpath,
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

const OLD_VERSION = "0.1.1";
const NEW_VERSION = "0.1.2";
const OLD_COMPOSITION_DIGEST =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEW_COMPOSITION_DIGEST =
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DMG_SHA256 =
  "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

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
    distribution_signature: "accepted",
    distribution_profile: "internal_test",
    code_signature: "ad_hoc_valid",
    hardened_runtime: true,
    library_validation_entitlement_scope: "embedding_runtime_only",
    notarization: "not_requested",
    tester_allow_list_required: true,
    dmg_sha256: DMG_SHA256,
    app_composition_digest: NEW_COMPOSITION_DIGEST,
  };
}

function compositionReceipt(version) {
  return {
    bundle_id: "local.resume-ir.desktop",
    version,
    target_triple: "aarch64-apple-darwin",
    composition_digest:
      version === OLD_VERSION ? OLD_COMPOSITION_DIGEST : NEW_COMPOSITION_DIGEST,
  };
}

function installReceipt(version) {
  const composition = compositionReceipt(version);
  return {
    schema_version: "resume-ir.macos-install-receipt.v1",
    bundle_id: composition.bundle_id,
    version,
    target_triple: composition.target_triple,
    composition_digest: composition.composition_digest,
    dmg_sha256: version === OLD_VERSION ? OLD_COMPOSITION_DIGEST : DMG_SHA256,
  };
}

const signatureReceipt = async () => ({
  code_signature: "ad_hoc_valid",
  hardened_runtime: true,
});

async function fixture(context) {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-upgrade-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const applicationsDirectory = path.join(root, "Applications");
  const target = path.join(applicationsDirectory, "resume-ir.app");
  const sourceApp = path.join(root, "source", "resume-ir.app");
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  const userData = path.join(root, "user-data");
  const applicationSupportRoot = path.join(
    root,
    "Library",
    "Application Support",
  );
  await mkdir(target, { recursive: true });
  await mkdir(sourceApp, { recursive: true });
  await mkdir(temporaryRoot);
  await mkdir(userData);
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
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
    applicationSupportRoot,
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

async function verifyCompositionFixture({ appBundle, expectedVersion }) {
  assert.equal(
    await readFile(path.join(appBundle, "version"), "utf8"),
    expectedVersion,
  );
  return compositionReceipt(expectedVersion);
}

function verifyReceiptFixture({ receipt, composition }) {
  assert.equal(receipt.version, composition.version);
  assert.equal(receipt.composition_digest, composition.composition_digest);
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
  const verifiedCandidateApps = [];
  const persistedReceipts = [];
  let storedReceipt = installReceipt(OLD_VERSION);
  return {
    calls,
    verifiedCandidateApps,
    persistedReceipts,
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
      verifyApp: async ({ appBundle }) => {
        assert.notEqual(
          await readFile(path.join(appBundle, "version"), "utf8"),
          OLD_VERSION,
        );
        verifiedCandidateApps.push(appBundle);
        return appReceipt();
      },
      verifyComposition: verifyCompositionFixture,
      verifySignature: signatureReceipt,
      applicationSupportRoot: values.applicationSupportRoot,
      readReceipt: async () => storedReceipt,
      verifyReceipt: verifyReceiptFixture,
      persistReceipt: async ({ receipt }) => {
        persistedReceipts.push(receipt);
        storedReceipt = receipt;
        return receipt;
      },
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
  const { args, calls, persistedReceipts, verifiedCandidateApps } =
    upgradeArguments(values);
  const receipt = await upgradeMacosDmg(args);

  assert.equal(await readFile(path.join(values.target, "version"), "utf8"), NEW_VERSION);
  assert.equal(await readFile(path.join(values.userData, "sentinel"), "utf8"), "preserve");
  assert.deepEqual((await readdir(values.applicationsDirectory)).sort(), ["resume-ir.app"]);
  assert.equal(calls.filter(([command]) => command === "ditto").length, 1);
  assert.equal(calls.filter(([command]) => command.endsWith("lsregister")).length, 1);
  assert.equal(verifiedCandidateApps.length, 7);
  assert.deepEqual(persistedReceipts, [installReceipt(NEW_VERSION)]);
  assert.deepEqual(receipt, {
    schema_version: "resume-ir.macos-app-upgrade.v1",
    target_triple: "aarch64-apple-darwin",
    from_version: OLD_VERSION,
    to_version: NEW_VERSION,
    app_bundle_count: 1,
    runtime_composition_verified: true,
    composition_digest_match: true,
    install_receipt: "owner_only",
    launch_services_registered: true,
    rollback_required: false,
    user_data_removed: false,
    distribution_signature: "accepted",
    release_claim: "local_upgrade_only",
  });
});

test("rejects same-version, downgrade, invalid versions, and reserved-name collisions", async (context) => {
  assert.equal(compareThreePartVersions("0.1.2", "0.1.1"), 1);
  assert.equal(compareThreePartVersions("0.1.1", "0.1.1"), 0);
  assert.equal(compareThreePartVersions("0.1.0", "0.1.1"), -1);
  assert.throws(() => compareThreePartVersions("0.1.1-beta", OLD_VERSION), /version is invalid/);
  assert.throws(() => compareThreePartVersions("00.1.0", OLD_VERSION), /version is invalid/);

  for (const candidateVersion of [OLD_VERSION, "0.1.0"]) {
    const values = await fixture(context);
    const { args } = upgradeArguments(values, { candidateVersion });
    await assert.rejects(upgradeMacosDmg(args), /candidate version is not newer/);
    await assertOldState(values);
  }

  const values = await fixture(context);
  await mkdir(path.join(values.applicationsDirectory, ".resume-ir.app.upgrade-stage"));
  const { args } = upgradeArguments(values);
  await assert.rejects(upgradeMacosDmg(args), /orphan lifecycle workspace exists/);
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

test("fails closed for 0.1.0 and for missing installed evidence", async (context) => {
  const legacy = await fixture(context);
  const { args: legacyArgs } = upgradeArguments(legacy, {
    installedVersion: "0.1.0",
    candidateVersion: "0.1.1",
  });
  await assert.rejects(
    upgradeMacosDmg(legacyArgs),
    /installed App predates upgrade evidence/,
  );
  await assertOldState(legacy);

  for (const missing of ["receipt", "composition"]) {
    const values = await fixture(context);
    const overrides =
      missing === "receipt"
        ? {
            readReceipt: async () => {
              throw new Error("install receipt is unavailable");
            },
          }
        : {
            verifyComposition: async () => {
              throw new Error("bundle composition evidence is unavailable");
            },
          };
    const { args, verifiedCandidateApps } = upgradeArguments(values, overrides);
    await assert.rejects(upgradeMacosDmg(args), /evidence|receipt/);
    assert.equal(verifiedCandidateApps.length, 0);
    await assertOldState(values);
  }
});

test("removes transaction-owned partial copy and rolls back a verified staged App", async (context) => {
  for (const runnerOptions of [{ failCopy: true }, { failDetach: true }]) {
    const values = await fixture(context);
    const { args } = upgradeArguments(values, { runnerOptions });
    await assert.rejects(
      upgradeMacosDmg(args),
      runnerOptions.failCopy ? /candidate App copy failed/ : undefined,
    );
    assert.equal(
      await readFile(path.join(values.target, "version"), "utf8"),
      OLD_VERSION,
    );
    assert.equal(
      await readFile(path.join(values.userData, "sentinel"), "utf8"),
      "preserve",
    );
    assert.deepEqual(
      (await readdir(values.applicationsDirectory)).sort(),
      ["resume-ir.app"],
    );
  }
});

test("restores the old App after promotion, post-verify, or registration failure", async (context) => {
  for (const failure of ["promotion", "post_verify", "registration"]) {
    const values = await fixture(context);
    let renameCount = 0;
    const filesystem = {
      rename: async (...args) => {
        renameCount += 1;
        if (failure === "promotion" && renameCount === 3) {
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
      let candidateVerificationCount = 0;
      overrides.verifyComposition = async (request) => {
        if (request.expectedVersion === NEW_VERSION) {
          candidateVerificationCount += 1;
          if (candidateVerificationCount === 3) {
            throw new Error("synthetic post-verify failure");
          }
        }
        return verifyCompositionFixture(request);
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

test("retries partial tombstone GC without deleting the committed new App", async (context) => {
  const values = await fixture(context);
  let backupCleanupFailed = false;
  const filesystem = {
    rm: async (target, options) => {
      if (
        !backupCleanupFailed &&
        path.basename(target).startsWith(".resume-ir.app.lifecycle-trash.") &&
        path.basename(target).endsWith(".backup")
      ) {
        backupCleanupFailed = true;
        await rm(path.join(target, "version"));
        throw new Error("synthetic backup cleanup failure");
      }
      await rm(target, options);
    },
  };
  const { args, persistedReceipts } = upgradeArguments(values, { filesystem });
  await assert.rejects(
    upgradeMacosDmg(args),
    /macOS upgrade post-commit failure/,
  );
  assert.deepEqual(persistedReceipts, [installReceipt(NEW_VERSION)]);
  assert.equal(
    await readFile(path.join(values.target, "version"), "utf8"),
    NEW_VERSION,
  );
  assert.equal(await readFile(path.join(values.userData, "sentinel"), "utf8"), "preserve");
  assert.deepEqual(await readdir(values.applicationsDirectory), ["resume-ir.app"]);
});

test("preserves committed new App when receipt persistence reports after commit", async (context) => {
  const values = await fixture(context);
  const persistedReceipts = [];
  let storedReceipt = installReceipt(OLD_VERSION);
  let failedAfterCommit = false;
  const { args } = upgradeArguments(values, {
    readReceipt: async () => storedReceipt,
    persistReceipt: async ({ receipt }) => {
      persistedReceipts.push(receipt);
      storedReceipt = receipt;
      if (receipt.version === NEW_VERSION && !failedAfterCommit) {
        failedAfterCommit = true;
        throw new Error("synthetic post-rename fsync failure");
      }
      return receipt;
    },
  });

  await assert.rejects(upgradeMacosDmg(args), /macOS upgrade post-commit failure/);
  assert.deepEqual(persistedReceipts, [installReceipt(NEW_VERSION)]);
  assert.deepEqual(storedReceipt, installReceipt(NEW_VERSION));
  assert.equal(
    await readFile(path.join(values.target, "version"), "utf8"),
    NEW_VERSION,
  );
  assert.equal(await readFile(path.join(values.userData, "sentinel"), "utf8"), "preserve");
});

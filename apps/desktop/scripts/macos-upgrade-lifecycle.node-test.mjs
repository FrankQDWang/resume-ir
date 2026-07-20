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
import { withVerifiedMacosDmg } from "./verify-macos-dmg.mjs";

const OLD_VERSION = "0.1.1";
const NEW_VERSION = "0.1.2";
const OLD_COMPOSITION_DIGEST =
  "18a2d41769f6e2fcc6cc504085b40f25ec185a27109eac525e551513ec5801c6";
const NEW_COMPOSITION_DIGEST =
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const OLD_DMG_SHA256 =
  "363ce8d5db7c120a05fc7c282a9f9b6a8e1173f3175c308839dfb1440867c780";
const DMG_SHA256 =
  "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const SOURCE_COMMIT = "0123456789abcdef0123456789abcdef01234567";

function appReceipt() {
  return {
    digest_match: true,
    architecture: "arm64",
    build_machine_identity_path_markers: 0,
  };
}

function dmgReceipt() {
  return {
    schema_version: "resume-ir.macos-dmg-composition.v2",
    target_triple: "aarch64-apple-darwin",
    source_commit: SOURCE_COMMIT,
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
  const composition = {
    bundle_id: "local.resume-ir.desktop",
    version,
    target_triple: "aarch64-apple-darwin",
    composition_digest:
      version === OLD_VERSION ? OLD_COMPOSITION_DIGEST : NEW_COMPOSITION_DIGEST,
  };
  return version === OLD_VERSION
    ? composition
    : { ...composition, source_commit: SOURCE_COMMIT };
}

function installReceipt(version) {
  const composition = compositionReceipt(version);
  return version === OLD_VERSION ? {
    schema_version: "resume-ir.macos-install-receipt.v1",
    bundle_id: composition.bundle_id,
    version,
    target_triple: composition.target_triple,
    composition_digest: composition.composition_digest,
    dmg_sha256: OLD_DMG_SHA256,
  } : {
    schema_version: "resume-ir.macos-install-receipt.v2",
    bundle_id: composition.bundle_id,
    version,
    target_triple: composition.target_triple,
    source_commit: SOURCE_COMMIT,
    composition_digest: composition.composition_digest,
    dmg_sha256: DMG_SHA256,
  };
}

const signatureReceipt = async () => ({
  code_signature: "ad_hoc_valid",
  hardened_runtime: true,
  library_validation_entitlement_scope: "embedding_runtime_only",
});

function verifiedDmgLease(sourceApp, { cleanupFails = false } = {}) {
  return async ({ consumeVerifiedImage }) => {
    const result = await consumeVerifiedImage({
      appBundle: sourceApp,
      appComposition: compositionReceipt(NEW_VERSION),
      receipt: dmgReceipt(),
    });
    if (cleanupFails) throw new Error("DMG detach or cleanup failed");
    return result;
  };
}

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
  assert.equal(receipt.source_commit, composition.source_commit);
}

function createRunner(sourceApp, calls, { failCopy = false, failRegistrations = 0 } = {}) {
  let registrationCount = 0;
  return async (command, args) => {
    calls.push([command, ...args]);
    const tool = path.basename(command);
    if (tool === "ditto") {
      if (failCopy) return { status: 1, stdout: "", stderr: "bounded" };
      await writeFile(
        path.join(args[1], "version"),
        await readFile(path.join(sourceApp, "version"), "utf8"),
      );
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
  let legacyStoredReceipt = installReceipt(OLD_VERSION);
  let currentStoredReceipt;
  const inspectReceiptSet = async () => {
    if (legacyStoredReceipt && currentStoredReceipt) {
      return {
        state: "both_valid",
        legacy_receipt: legacyStoredReceipt,
        current_receipt: currentStoredReceipt,
      };
    }
    if (legacyStoredReceipt) {
      return {
        state: "legacy_only",
        legacy_receipt: legacyStoredReceipt,
        current_receipt: null,
      };
    }
    if (currentStoredReceipt) {
      return {
        state: "current_only",
        legacy_receipt: null,
        current_receipt: currentStoredReceipt,
      };
    }
    throw new Error("install receipt set is invalid");
  };
  return {
    calls,
    verifiedCandidateApps,
    persistedReceipts,
    inspectReceiptSet,
    args: {
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      applicationsDirectory: values.applicationsDirectory,
      installedVersion: OLD_VERSION,
      candidateVersion: NEW_VERSION,
      temporaryRoot: values.temporaryRoot,
      platform: "darwin",
      systemRunner: createRunner(values.sourceApp, calls, overrides.runnerOptions),
      withVerifiedDmg: verifiedDmgLease(values.sourceApp, {
        cleanupFails: overrides.leaseCleanupFails,
      }),
      inspectApp,
      verifyApp: async ({ appBundle }) => {
        if (
          (await readFile(path.join(appBundle, "version"), "utf8")) !==
          OLD_VERSION
        ) {
          verifiedCandidateApps.push(appBundle);
        }
        return appReceipt();
      },
      verifyComposition: verifyCompositionFixture,
      verifyLegacyComposition: verifyCompositionFixture,
      verifySignaturePolicy: signatureReceipt,
      applicationSupportRoot: values.applicationSupportRoot,
      inspectReceiptSet,
      readLegacyReceipt: async () => legacyStoredReceipt,
      readCurrentReceipt: async () => currentStoredReceipt,
      verifyReceipt: verifyReceiptFixture,
      createCurrentReceipt: async ({ receipt }) => {
        persistedReceipts.push(receipt);
        if (currentStoredReceipt) throw new Error("current receipt already exists");
        currentStoredReceipt = receipt;
        if (overrides.failCurrentCreateAfterCommit) {
          throw new Error("synthetic post-rename fsync failure");
        }
        return receipt;
      },
      removeLegacyReceipt: async ({ expectedReceipt }) => {
        assert.deepEqual(legacyStoredReceipt, expectedReceipt);
        legacyStoredReceipt = undefined;
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
  const {
    args,
    calls,
    persistedReceipts,
    verifiedCandidateApps,
    inspectReceiptSet,
  } =
    upgradeArguments(values);
  const receipt = await upgradeMacosDmg(args);

  assert.equal(await readFile(path.join(values.target, "version"), "utf8"), NEW_VERSION);
  assert.equal(await readFile(path.join(values.userData, "sentinel"), "utf8"), "preserve");
  assert.deepEqual((await readdir(values.applicationsDirectory)).sort(), ["resume-ir.app"]);
  assert.equal(
    calls.filter(([command]) => path.basename(command) === "ditto").length,
    1,
  );
  assert.ok(calls.every(([command]) => path.isAbsolute(command)));
  assert.equal(calls.filter(([command]) => command.endsWith("lsregister")).length, 1);
  assert.equal(verifiedCandidateApps.length, 6);
  assert.deepEqual(persistedReceipts, [installReceipt(NEW_VERSION)]);
  assert.equal((await inspectReceiptSet()).state, "current_only");
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

test("upgrade copies from the verified mounted image without a second mount", async (context) => {
  const values = await fixture(context);
  const swappedApp = path.join(values.root, "swapped", "resume-ir.app");
  await mkdir(swappedApp, { recursive: true });
  await writeFile(path.join(swappedApp, "version"), NEW_VERSION);
  await writeFile(path.join(values.sourceApp, "payload"), "verified");
  await writeFile(path.join(swappedApp, "payload"), "swapped");
  let verifiedConsumerInvoked = false;
  const calls = [];
  const runner = async (command, args) => {
    calls.push([command, ...args]);
    const tool = path.basename(command);
    if (tool === "hdiutil" && args[0] === "attach") {
      return { status: 0, stdout: "", stderr: "" };
    }
    if (tool === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (tool === "ditto") {
      await writeFile(
        path.join(args[1], "version"),
        await readFile(path.join(args[0], "version"), "utf8"),
      );
      await writeFile(
        path.join(args[1], "payload"),
        await readFile(path.join(args[0], "payload"), "utf8"),
      );
    }
    return { status: 0, stdout: "", stderr: "" };
  };
  const { args } = upgradeArguments(values, {
    systemRunner: runner,
    withVerifiedDmg: async (request) => {
      await writeFile(values.dmg, "replacement-dmg");
      if (typeof request.consumeVerifiedImage === "function") {
        verifiedConsumerInvoked = true;
        return request.consumeVerifiedImage({
          appBundle: values.sourceApp,
          appComposition: compositionReceipt(NEW_VERSION),
          receipt: dmgReceipt(),
        });
      }
      throw new Error("verified DMG lease is unavailable");
    },
  });
  await upgradeMacosDmg(args);

  assert.equal(verifiedConsumerInvoked, true);
  assert.equal(
    calls.filter(
      ([command, action]) =>
        path.basename(command) === "hdiutil" && action === "attach",
    ).length,
    0,
  );
  assert.equal(await readFile(path.join(values.target, "payload"), "utf8"), "verified");
});

test("upgrade recovers a partial verifier mount after attach failure", async (context) => {
  const values = await fixture(context);
  let detachCalls = 0;
  const detachArguments = [];
  const runner = async (command, args) => {
    if (path.basename(command) === "hdiutil" && args[0] === "attach") {
      await writeFile(path.join(args.at(-1), "partial-mount"), "mounted");
      return { status: 1, stdout: "", stderr: "bounded" };
    }
    if (path.basename(command) === "hdiutil" && args[0] === "detach") {
      detachCalls += 1;
      detachArguments.push(args);
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    return { status: 0, stdout: "", stderr: "" };
  };
  const { args } = upgradeArguments(values, {
    systemRunner: runner,
    withVerifiedDmg: (request) =>
      withVerifiedMacosDmg({
        ...request,
        verifySource: async () => SOURCE_COMMIT,
        mountProbe: async () => true,
      }),
  });

  await assert.rejects(upgradeMacosDmg(args), /DMG attach failed/);
  assert.equal(detachCalls, 1);
  assert.equal(detachArguments[0].includes("-force"), true);
  assert.deepEqual(await readdir(values.temporaryRoot), []);
  await assertOldState(values);
});

test("rejects a candidate without the exact installed signature policy", async (context) => {
  const values = await fixture(context);
  const { args } = upgradeArguments(values, {
    verifySignaturePolicy: async () => ({
      code_signature: "ad_hoc_valid",
      hardened_runtime: true,
    }),
  });
  await assert.rejects(upgradeMacosDmg(args), /App signature is invalid/);
  await assertOldState(values);
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
    await assert.rejects(
      upgradeMacosDmg(args),
      /only the exact 0\.1\.1 to 0\.1\.2 upgrade is supported/,
    );
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
    /only the exact 0\.1\.1 to 0\.1\.2 upgrade is supported/,
  );
  await assertOldState(legacy);

  for (const missing of ["receipt", "composition"]) {
    const values = await fixture(context);
    const overrides =
      missing === "receipt"
        ? {
            inspectReceiptSet: async () => {
              throw new Error("install receipt is unavailable");
            },
          }
        : {
            verifyLegacyComposition: async () => {
              throw new Error("bundle composition evidence is unavailable");
            },
          };
    const { args, verifiedCandidateApps } = upgradeArguments(values, overrides);
    await assert.rejects(upgradeMacosDmg(args), /evidence|receipt/);
    assert.equal(verifiedCandidateApps.length, 0);
    await assertOldState(values);
  }
});

test("rejects both receipt generations when no upgrade journal exists", async (context) => {
  const values = await fixture(context);
  const { args, verifiedCandidateApps } = upgradeArguments(values, {
    inspectReceiptSet: async () => ({
      state: "both_valid",
      legacy_receipt: installReceipt(OLD_VERSION),
      current_receipt: installReceipt(NEW_VERSION),
    }),
  });

  await assert.rejects(
    upgradeMacosDmg(args),
    /legacy upgrade receipt set is invalid/,
  );
  assert.equal(verifiedCandidateApps.length, 0);
  await assertOldState(values);
});

test("removes transaction-owned partial copy and rolls back a verified staged App", async (context) => {
  for (const failure of ["copy", "lease_cleanup"]) {
    const values = await fixture(context);
    const { args } = upgradeArguments(values, {
      runnerOptions: failure === "copy" ? { failCopy: true } : undefined,
      leaseCleanupFails: failure === "lease_cleanup",
    });
    await assert.rejects(
      upgradeMacosDmg(args),
      failure === "copy"
        ? /candidate App copy failed/
        : /DMG detach or cleanup failed/,
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
  const { args, persistedReceipts, inspectReceiptSet } = upgradeArguments(values, {
    failCurrentCreateAfterCommit: true,
  });

  await assert.rejects(upgradeMacosDmg(args), /macOS upgrade post-commit failure/);
  assert.deepEqual(persistedReceipts, [installReceipt(NEW_VERSION)]);
  assert.equal((await inspectReceiptSet()).state, "current_only");
  assert.equal(
    await readFile(path.join(values.target, "version"), "utf8"),
    NEW_VERSION,
  );
  assert.equal(await readFile(path.join(values.userData, "sentinel"), "utf8"), "preserve");
});

import assert from "node:assert/strict";
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  installMacosDmg,
  inspectMacosAppBundle,
  uninstallMacosApp,
} from "./macos-install-lifecycle.mjs";
import {
  persistInstallReceipt,
  readInstallReceipt,
  validateInstallReceipt,
} from "./macos-install-receipt.mjs";
import { readLifecycleJournal } from "./macos-lifecycle-journal.mjs";
import {
  acquireLifecycleLock,
  prepareLifecycleLockFile,
  releaseLifecycleLock,
} from "./macos-lifecycle-lock.mjs";
import { prepareOwnerEvidenceDirectory } from "./macos-owner-evidence-store.mjs";
import { withVerifiedMacosDmg } from "./verify-macos-dmg.mjs";

const SOURCE_COMMIT = "0123456789abcdef0123456789abcdef01234567";
const SOURCE = Object.freeze({
  authority: "worktree_snapshot",
  base_commit: SOURCE_COMMIT,
  source_tree_sha256: "a".repeat(64),
});

const EXPECTED_APP = {
  bundle_id: "local.resume-ir.desktop",
  version: "0.1.2",
  display_name: "resume-ir",
  icon_file: "icon.icns",
};

function dmgReceipt() {
  return {
    schema_version: "resume-ir.macos-dmg-composition.v3",
    target_triple: "aarch64-apple-darwin",
    source: SOURCE,
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
    dmg_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    app_composition_digest:
      "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  };
}

function compositionReceipt(version = EXPECTED_APP.version) {
  return {
    bundle_id: EXPECTED_APP.bundle_id,
    version,
    target_triple: "aarch64-apple-darwin",
    source: SOURCE,
    composition_digest: dmgReceipt().app_composition_digest,
  };
}

function installReceipt(version = EXPECTED_APP.version) {
  const composition = compositionReceipt(version);
  return {
    schema_version: "resume-ir.macos-install-receipt.v3",
    bundle_id: composition.bundle_id,
    version,
    target_triple: composition.target_triple,
    source: SOURCE,
    composition_digest: composition.composition_digest,
    dmg_sha256: dmgReceipt().dmg_sha256,
  };
}

const signatureReceipt = async () => ({
  code_signature: "ad_hoc_valid",
  hardened_runtime: true,
  library_validation_entitlement_scope: "embedding_runtime_only",
});

function verifiedDmgLease(sourceApp, options = {}) {
  return async ({ consumeVerifiedImage }) => {
    const result = await consumeVerifiedImage({
      appBundle: sourceApp,
      appComposition: options.composition ?? compositionReceipt(),
      receipt: options.receipt ?? dmgReceipt(),
    });
    if (options.cleanupFails) {
      throw new Error("DMG detach or cleanup failed");
    }
    return result;
  };
}

async function fixture(context) {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-install-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const applicationsDirectory = path.join(root, "Applications");
  const sourceApp = path.join(root, "source", "resume-ir.app");
  const dmg = path.join(root, "resume-ir.dmg");
  const temporaryRoot = path.join(root, "mounts");
  const applicationSupportRoot = path.join(
    await realpath(root),
    "Library",
    "Application Support",
  );
  await mkdir(applicationsDirectory);
  await mkdir(sourceApp, { recursive: true });
  await mkdir(temporaryRoot);
  await mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 });
  await writeFile(dmg, "synthetic-dmg");
  return {
    root,
    applicationsDirectory,
    sourceApp,
    dmg,
    temporaryRoot,
    applicationSupportRoot,
  };
}

function successfulRunner(sourceApp, calls, failureCommand) {
  return async (command, args) => {
    calls.push([command, ...args]);
    const tool = path.basename(command);
    if (
      tool === failureCommand &&
      (failureCommand !== "lsregister" || args[0] === "-f")
    ) {
      return { status: 1, stdout: "", stderr: "bounded" };
    }
    if (
      failureCommand === "hdiutil-detach" &&
      tool === "hdiutil" &&
      args[0] === "detach"
    ) {
      return { status: 1, stdout: "", stderr: "bounded" };
    }
    if (tool === "hdiutil" && args[0] === "attach") {
      const mount = args.at(-1);
      await symlink(sourceApp, path.join(mount, "resume-ir.app"));
      await symlink("/Applications", path.join(mount, "Applications"));
    }
    if (tool === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
    }
    if (tool === "ditto") {
      await mkdir(args[1], { recursive: true });
      await writeFile(path.join(args[1], "copied"), "yes");
    }
    return { status: 0, stdout: "", stderr: "" };
  };
}

test("public lifecycle entry fails closed when another process owner holds the lock", async (context) => {
  const values = await fixture(context);
  const lockFile = await prepareLifecycleLockFile({
    applicationSupportRoot: values.applicationSupportRoot,
    prepareEvidenceDirectory: prepareOwnerEvidenceDirectory,
  });
  const capability = await acquireLifecycleLock({ lockFile });
  try {
    await assert.rejects(
      installMacosDmg({
        repoRoot: values.root,
        targetTriple: "aarch64-apple-darwin",
        dmg: values.dmg,
        applicationsDirectory: values.applicationsDirectory,
        platform: "darwin",
        applicationSupportRoot: values.applicationSupportRoot,
      }),
      /lifecycle lock is unavailable/,
    );
  } finally {
    await releaseLifecycleLock(capability);
  }
  assert.deepEqual(await readdir(values.applicationsDirectory), []);
  assert.equal(
    await readLifecycleJournal({
      applicationSupportRoot: values.applicationSupportRoot,
      allowMissing: true,
    }),
    undefined,
  );
});

test("installs only after DMG verification and emits a bounded receipt", async (context) => {
  const values = await fixture(context);
  const calls = [];
  const inspected = [];
  const verifiedApps = [];
  const verifiedCompositions = [];
  const persistedReceipts = [];
  const receipt = await installMacosDmg({
    repoRoot: values.root,
    targetTriple: "aarch64-apple-darwin",
    dmg: values.dmg,
    applicationsDirectory: values.applicationsDirectory,
    temporaryRoot: values.temporaryRoot,
    platform: "darwin",
    systemRunner: successfulRunner(values.sourceApp, calls),
    withVerifiedDmg: verifiedDmgLease(values.sourceApp),
    inspectApp: async ({ appBundle }) => {
      inspected.push(appBundle);
      return EXPECTED_APP;
    },
    verifyApp: async ({ appBundle }) => {
      verifiedApps.push(appBundle);
      return { digest_match: true, architecture: "arm64" };
    },
    verifyComposition: async ({ appBundle }) => {
      verifiedCompositions.push(appBundle);
      return compositionReceipt();
    },
    verifySignaturePolicy: signatureReceipt,
    applicationSupportRoot: values.applicationSupportRoot,
    persistReceipt: async ({ applicationSupportRoot, receipt: installReceipt }) => {
      persistedReceipts.push(installReceipt);
      return persistInstallReceipt({
        applicationSupportRoot,
        receipt: installReceipt,
      });
    },
  });

  assert.equal(inspected.length, 5);
  assert.equal(verifiedApps.length, 5);
  assert.equal(verifiedCompositions.length, 5);
  assert.equal(persistedReceipts.length, 1);
  assert.deepEqual(persistedReceipts[0], {
    schema_version: "resume-ir.macos-install-receipt.v3",
    bundle_id: EXPECTED_APP.bundle_id,
    version: EXPECTED_APP.version,
    target_triple: "aarch64-apple-darwin",
    source: SOURCE,
    composition_digest: dmgReceipt().app_composition_digest,
    dmg_sha256: dmgReceipt().dmg_sha256,
  });
  assert.equal(
    calls.filter(([command]) => path.basename(command) === "ditto").length,
    1,
  );
  assert.ok(calls.every(([command]) => path.isAbsolute(command)));
  assert.equal(calls.filter(([command]) => command.endsWith("lsregister")).length, 1);
  assert.deepEqual(receipt, {
    schema_version: "resume-ir.macos-installed-app.v1",
    target_triple: "aarch64-apple-darwin",
    app_bundle_count: 1,
    bundle_id_match: true,
    version: "0.1.2",
    display_name: "resume-ir",
    icon_metadata: "icon.icns",
    runtime_composition_verified: true,
    composition_digest_match: true,
    install_receipt: "owner_only",
    launch_services_registered: true,
    user_data_removed: false,
    code_signature: "ad_hoc_valid",
    hardened_runtime: true,
    notarization: "not_requested",
    tester_allow_list_required: true,
    release_claim: "internal_test_install_only",
  });
});

test("copies from the verified mounted image without remounting a replaced DMG pathname", async (context) => {
  const values = await fixture(context);
  const swappedApp = path.join(values.root, "swapped", "resume-ir.app");
  await mkdir(swappedApp, { recursive: true });
  await writeFile(path.join(values.sourceApp, "payload"), "verified");
  await writeFile(path.join(swappedApp, "payload"), "swapped");
  const calls = [];
  let verifiedConsumerInvoked = false;
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
        path.join(args[1], "payload"),
        await readFile(path.join(args[0], "payload"), "utf8"),
      );
    }
    return { status: 0, stdout: "", stderr: "" };
  };
  const receipt = await installMacosDmg({
    repoRoot: values.root,
    targetTriple: "aarch64-apple-darwin",
    dmg: values.dmg,
    applicationsDirectory: values.applicationsDirectory,
    temporaryRoot: values.temporaryRoot,
    platform: "darwin",
    systemRunner: runner,
    withVerifiedDmg: async (request) => {
      await writeFile(values.dmg, "replacement-dmg");
      if (typeof request.consumeVerifiedImage === "function") {
        verifiedConsumerInvoked = true;
        return request.consumeVerifiedImage({
          appBundle: values.sourceApp,
          appComposition: compositionReceipt(),
          receipt: dmgReceipt(),
        });
      }
      throw new Error("verified DMG lease is unavailable");
    },
    inspectApp: async () => EXPECTED_APP,
    verifyApp: async () => ({ digest_match: true, architecture: "arm64" }),
    verifyComposition: async () => compositionReceipt(),
    verifySignaturePolicy: signatureReceipt,
    applicationSupportRoot: values.applicationSupportRoot,
  });

  assert.equal(verifiedConsumerInvoked, true);
  assert.equal(
    calls.filter(
      ([command, action]) =>
        path.basename(command) === "hdiutil" && action === "attach",
    ).length,
    0,
  );
  assert.equal(
    await readFile(
      path.join(values.applicationsDirectory, "resume-ir.app", "payload"),
      "utf8",
    ),
    "verified",
  );
  assert.equal(receipt.install_receipt, "owner_only");
});

test("install recovers a partial verifier mount after attach timeout", async (context) => {
  const values = await fixture(context);
  let detachCalls = 0;
  const detachArguments = [];
  const runner = async (command, args) => {
    if (path.basename(command) === "hdiutil" && args[0] === "attach") {
      await writeFile(path.join(args.at(-1), "partial-mount"), "mounted");
      return {
        status: null,
        error: { code: "ETIMEDOUT" },
        stdout: "",
        stderr: "bounded",
      };
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

  await assert.rejects(
    installMacosDmg({
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      applicationsDirectory: values.applicationsDirectory,
      temporaryRoot: values.temporaryRoot,
      platform: "darwin",
      systemRunner: runner,
      withVerifiedDmg: (request) =>
        withVerifiedMacosDmg({
          ...request,
          verifySource: async () => SOURCE,
          mountProbe: async () => true,
        }),
      applicationSupportRoot: values.applicationSupportRoot,
    }),
    /DMG attach failed/,
  );
  assert.equal(detachCalls, 1);
  assert.equal(detachArguments[0].includes("-force"), true);
  assert.deepEqual(await readdir(values.temporaryRoot), []);
});

test("rejects an install without the exact signature and entitlement policy", async (context) => {
  const values = await fixture(context);
  await assert.rejects(
    installMacosDmg({
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      applicationsDirectory: values.applicationsDirectory,
      temporaryRoot: values.temporaryRoot,
      platform: "darwin",
      systemRunner: successfulRunner(values.sourceApp, []),
      withVerifiedDmg: verifiedDmgLease(values.sourceApp),
      inspectApp: async () => EXPECTED_APP,
      verifyApp: async () => ({ digest_match: true, architecture: "arm64" }),
      verifyComposition: async () => compositionReceipt(),
      verifySignaturePolicy: async () => ({
        code_signature: "ad_hoc_valid",
        hardened_runtime: true,
      }),
      applicationSupportRoot: values.applicationSupportRoot,
    }),
    /installed App signature is invalid/,
  );
  assert.deepEqual(await readdir(values.applicationsDirectory), []);
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
      applicationSupportRoot: values.applicationSupportRoot,
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
      applicationSupportRoot: values.applicationSupportRoot,
    }),
    /Applications root is invalid/,
  );
});

test("hard-cut install receipt validation rejects v2 evidence", () => {
  const legacy = {
    ...installReceipt(),
    schema_version: "resume-ir.macos-install-receipt.v2",
  };
  assert.throws(() => validateInstallReceipt(legacy), /install receipt is invalid/);
});

test("removes a staged or installed App after copy, lease cleanup, or registration failure", async (context) => {
  for (const failureCommand of ["ditto", "lease-cleanup", "lsregister"]) {
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
        systemRunner: successfulRunner(values.sourceApp, calls, failureCommand),
        withVerifiedDmg: verifiedDmgLease(values.sourceApp, {
          cleanupFails: failureCommand === "lease-cleanup",
        }),
        inspectApp: async () => EXPECTED_APP,
        verifyApp: async () => ({ digest_match: true, architecture: "arm64" }),
        verifyComposition: async () => compositionReceipt(),
        verifySignaturePolicy: signatureReceipt,
        applicationSupportRoot: values.applicationSupportRoot,
        launchServicesCommand: failureCommand === "lsregister" ? "lsregister" : undefined,
      }),
      failureCommand === "ditto"
        ? /App copy failed/
        : failureCommand === "lease-cleanup"
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
      systemRunner: successfulRunner(values.sourceApp, calls),
      withVerifiedDmg: verifiedDmgLease(values.sourceApp),
      inspectApp: async () => ({ ...EXPECTED_APP, icon_file: "" }),
      verifyApp: async () => ({ digest_match: true, architecture: "arm64" }),
      verifyComposition: async () => compositionReceipt(),
      verifySignaturePolicy: signatureReceipt,
      applicationSupportRoot: values.applicationSupportRoot,
    }),
    /App identity is invalid/,
  );
});

test("removes the installed App when owner-only receipt persistence fails", async (context) => {
  const values = await fixture(context);
  const calls = [];
  let syncCalls = 0;
  await assert.rejects(
    installMacosDmg({
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      applicationsDirectory: values.applicationsDirectory,
      temporaryRoot: values.temporaryRoot,
      platform: "darwin",
      systemRunner: successfulRunner(values.sourceApp, calls),
      withVerifiedDmg: verifiedDmgLease(values.sourceApp),
      inspectApp: async () => EXPECTED_APP,
      verifyApp: async () => ({ digest_match: true, architecture: "arm64" }),
      verifyComposition: async () => compositionReceipt(),
      verifySignaturePolicy: signatureReceipt,
      applicationSupportRoot: values.applicationSupportRoot,
      persistReceipt: ({ applicationSupportRoot, receipt }) =>
        persistInstallReceipt({
          applicationSupportRoot,
          receipt,
          operations: {
            syncDirectory: async () => {
              syncCalls += 1;
              if (syncCalls === 1) throw new Error("synthetic fsync failure");
            },
          },
        }),
    }),
    /install receipt could not be persisted/,
  );
  await assert.rejects(
    readFile(path.join(values.applicationsDirectory, "resume-ir.app", "copied")),
  );
  await assert.rejects(
    readInstallReceipt({
      applicationSupportRoot: values.applicationSupportRoot,
    }),
    /install receipt is unavailable/,
  );
  assert.equal(
    calls.filter(
      ([command, action]) => command.endsWith("lsregister") && action === "-u",
    ).length,
    1,
  );
});

test("uninstall removes only the verified App and preserves user data", async (context) => {
  const values = await fixture(context);
  const installed = path.join(values.applicationsDirectory, "resume-ir.app");
  const userData = path.join(values.root, "Library", "Application Support", "local.resume-ir.desktop");
  await mkdir(installed);
  await mkdir(userData, { recursive: true });
  await writeFile(path.join(userData, "sentinel"), "preserve");
  await persistInstallReceipt({
    applicationSupportRoot: values.applicationSupportRoot,
    receipt: installReceipt(),
  });
  const receipt = await uninstallMacosApp({
    repoRoot: values.root,
    targetTriple: "aarch64-apple-darwin",
    applicationsDirectory: values.applicationsDirectory,
    platform: "darwin",
    systemRunner: async () => ({ status: 0, stdout: "", stderr: "" }),
    inspectApp: async () => EXPECTED_APP,
    verifyComposition: async () => compositionReceipt(),
    verifySignaturePolicy: signatureReceipt,
    applicationSupportRoot: path.join(values.root, "Library", "Application Support"),
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

test("uninstall rolls back before commit and converges tombstone GC after commit", async (context) => {
  for (const failure of ["receipt", "app"]) {
    const values = await fixture(context);
    const installed = path.join(values.applicationsDirectory, "resume-ir.app");
    const storedReceipt = installReceipt();
    let currentReceipt = storedReceipt;
    const persisted = [];
    const registrations = [];
    let appCleanupFailed = false;
    await mkdir(installed);
    const filesystem =
      failure === "app"
        ? {
            rm: async (target, options) => {
              if (!appCleanupFailed) {
                appCleanupFailed = true;
                throw new Error("synthetic App deletion failure");
              }
              await rm(target, options);
            },
          }
        : {};
    await assert.rejects(
      uninstallMacosApp({
        repoRoot: values.root,
        targetTriple: "aarch64-apple-darwin",
        applicationsDirectory: values.applicationsDirectory,
        platform: "darwin",
        systemRunner: async (command, args) => {
          if (command.endsWith("lsregister")) registrations.push(args[0]);
          return { status: 0, stdout: "", stderr: "" };
        },
        inspectApp: async () => EXPECTED_APP,
        verifyComposition: async () => compositionReceipt(),
        verifySignaturePolicy: signatureReceipt,
        applicationSupportRoot: values.applicationSupportRoot,
        readReceipt: async () => currentReceipt,
        verifyReceipt: ({ receipt }) => assert.deepEqual(receipt, storedReceipt),
        removeReceipt: async () => {
          if (failure === "receipt") {
            throw new Error("synthetic receipt removal failure");
          }
          currentReceipt = undefined;
          return storedReceipt;
        },
        persistReceipt: async ({ receipt }) => {
          persisted.push(receipt);
          currentReceipt = receipt;
          return receipt;
        },
        filesystem,
      }),
      failure === "receipt"
        ? /synthetic receipt removal failure/
        : /synthetic App deletion failure/,
    );
    assert.deepEqual(persisted, []);
    assert.deepEqual(
      registrations,
      failure === "receipt" ? ["-u", "-f"] : ["-u"],
    );
    assert.deepEqual(
      (await readdir(values.applicationsDirectory)).sort(),
      failure === "receipt" ? ["resume-ir.app"] : [],
    );
  }
});

test("reads only the expected bounded macOS bundle metadata", async (context) => {
  const values = await fixture(context);
  const calls = [];
  const fields = new Map([
    ["CFBundleIdentifier", "local.resume-ir.desktop"],
    ["CFBundleShortVersionString", "0.1.2"],
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
  assert.ok(calls.every(([command]) => command === "/usr/bin/plutil"));
});

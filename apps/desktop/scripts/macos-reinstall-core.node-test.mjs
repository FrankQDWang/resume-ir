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

import { reinstallMacosDmg } from "./macos-reinstall-core.mjs";

const VERSION = "0.1.2";
const OLD_SOURCE = Object.freeze({
  authority: "exact_main_commit",
  base_commit: "1".repeat(40),
  source_tree_sha256: "2".repeat(64),
});
const NEW_SOURCE = Object.freeze({
  authority: "worktree_snapshot",
  base_commit: "3".repeat(40),
  source_tree_sha256: "4".repeat(64),
});
const OLD_COMPOSITION_DIGEST = "5".repeat(64);
const NEW_COMPOSITION_DIGEST = "6".repeat(64);
const OLD_DMG_SHA256 = "7".repeat(64);
const NEW_DMG_SHA256 = "8".repeat(64);

function composition(generation) {
  return {
    bundle_id: "local.resume-ir.desktop",
    version: VERSION,
    target_triple: "aarch64-apple-darwin",
    source: generation === "old" ? OLD_SOURCE : NEW_SOURCE,
    composition_digest:
      generation === "old"
        ? OLD_COMPOSITION_DIGEST
        : NEW_COMPOSITION_DIGEST,
  };
}

function installReceipt(generation) {
  const value = composition(generation);
  return {
    schema_version: "resume-ir.macos-install-receipt.v3",
    bundle_id: value.bundle_id,
    version: value.version,
    target_triple: value.target_triple,
    source: value.source,
    composition_digest: value.composition_digest,
    dmg_sha256:
      generation === "old" ? OLD_DMG_SHA256 : NEW_DMG_SHA256,
  };
}

function dmgReceipt(overrides = {}) {
  return {
    schema_version: "resume-ir.macos-dmg-composition.v3",
    target_triple: "aarch64-apple-darwin",
    source: NEW_SOURCE,
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
    dmg_sha256: NEW_DMG_SHA256,
    app_composition_digest: NEW_COMPOSITION_DIGEST,
    ...overrides,
  };
}

async function fixture(context) {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-reinstall-test-")),
  );
  context.after(() => rm(root, { recursive: true, force: true }));
  const applicationsDirectory = path.join(root, "Applications");
  const target = path.join(applicationsDirectory, "resume-ir.app");
  const sourceApp = path.join(root, "source", "resume-ir.app");
  const temporaryRoot = path.join(root, "mounts");
  const applicationSupportRoot = path.join(
    root,
    "Library",
    "Application Support",
  );
  await Promise.all([
    mkdir(target, { recursive: true }),
    mkdir(sourceApp, { recursive: true }),
    mkdir(temporaryRoot),
    mkdir(applicationSupportRoot, { recursive: true, mode: 0o700 }),
  ]);
  await writeFile(path.join(target, "generation"), "old");
  await writeFile(path.join(sourceApp, "generation"), "new");
  return {
    applicationSupportRoot,
    applicationsDirectory,
    root,
    sourceApp,
    target,
    temporaryRoot,
  };
}

function runnerFor(sourceApp, { failCopy = false, failRegistration = false } = {}) {
  return async (command, args) => {
    if (path.basename(command) === "ditto") {
      if (failCopy) return { status: 1, stdout: "", stderr: "bounded" };
      await writeFile(
        path.join(args[1], "generation"),
        await readFile(path.join(sourceApp, "generation"), "utf8"),
      );
    }
    if (
      command.endsWith("lsregister") &&
      args[0] === "-f" &&
      failRegistration
    ) {
      return { status: 1, stdout: "", stderr: "bounded" };
    }
    return { status: 0, stdout: "", stderr: "" };
  };
}

function argumentsFor(values, overrides = {}) {
  let storedReceipt = installReceipt("old");
  const persisted = [];
  const verifyComposition = async ({
    appBundle,
    expectedSource,
    expectedVersion,
  }) => {
    assert.equal(expectedVersion, VERSION);
    const generation = await readFile(
      path.join(appBundle, "generation"),
      "utf8",
    );
    const value = composition(generation);
    assert.deepEqual(expectedSource, value.source);
    return value;
  };
  return {
    persisted,
    readStored: () => storedReceipt,
    args: {
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: path.join(values.root, "resume-ir.dmg"),
      applicationsDirectory: values.applicationsDirectory,
      installedVersion: VERSION,
      candidateVersion: VERSION,
      temporaryRoot: values.temporaryRoot,
      platform: "darwin",
      systemRunner: runnerFor(values.sourceApp, overrides),
      withVerifiedDmg: async ({ consumeVerifiedImage }) =>
        consumeVerifiedImage({
          appBundle: values.sourceApp,
          appComposition: composition("new"),
          receipt: dmgReceipt(overrides.dmgReceipt),
        }),
      inspectApp: async () => ({
        bundle_id: "local.resume-ir.desktop",
        version: VERSION,
        display_name: "resume-ir",
        icon_file: "icon.icns",
      }),
      verifyApp: async () => ({
        digest_match: true,
        architecture: "arm64",
        build_machine_identity_path_markers: 0,
      }),
      verifyComposition,
      verifySignaturePolicy: async () => ({
        code_signature: "ad_hoc_valid",
        hardened_runtime: true,
        library_validation_entitlement_scope: "embedding_runtime_only",
      }),
      applicationSupportRoot: values.applicationSupportRoot,
      readCurrentReceipt: async () => storedReceipt,
      verifyReceipt: ({ receipt, composition: value }) => {
        assert.equal(receipt.composition_digest, value.composition_digest);
        assert.deepEqual(receipt.source, value.source);
      },
      replaceCurrentReceipt: async ({ receipt, expectedReceipt }) => {
        assert.deepEqual(expectedReceipt, storedReceipt);
        persisted.push(receipt);
        storedReceipt = receipt;
        return receipt;
      },
      filesystem: overrides.filesystem,
    },
  };
}

test("reinstalls the current App through one receipt-bound atomic replacement", async (context) => {
  const values = await fixture(context);
  const state = argumentsFor(values);
  const receipt = await reinstallMacosDmg(state.args);

  assert.equal(receipt.schema_version, "resume-ir.macos-app-reinstall.v1");
  assert.equal(receipt.release_claim, "local_reinstall_only");
  assert.deepEqual(state.persisted, [installReceipt("new")]);
  assert.deepEqual(state.readStored(), installReceipt("new"));
  assert.equal(
    await readFile(path.join(values.target, "generation"), "utf8"),
    "new",
  );
  assert.deepEqual(await readdir(values.applicationsDirectory), [
    "resume-ir.app",
  ]);
});

test("validates the installed generation from its own source receipt", async (context) => {
  const values = await fixture(context);
  let candidateChecks = 0;
  const state = argumentsFor(values);
  state.args.verifyApp = async ({ appBundle }) => {
    if (candidateChecks === 0) assert.notEqual(appBundle, values.target);
    candidateChecks += 1;
    return {
      digest_match: true,
      architecture: "arm64",
      build_machine_identity_path_markers: 0,
    };
  };

  await reinstallMacosDmg(state.args);
  assert.equal(candidateChecks > 0, true);
});

test("copy, promotion, and registration failures retain the verified old App", async (context) => {
  for (const failure of ["copy", "promotion", "registration"]) {
    const values = await fixture(context);
    let renameCount = 0;
    const state = argumentsFor(values, {
      failCopy: failure === "copy",
      failRegistration: failure === "registration",
      filesystem: {
        rename: async (...args) => {
          renameCount += 1;
          if (failure === "promotion" && renameCount === 3) {
            throw new Error("synthetic promotion failure");
          }
          await rename(...args);
        },
      },
    });

    await assert.rejects(reinstallMacosDmg(state.args));
    assert.equal(
      await readFile(path.join(values.target, "generation"), "utf8"),
      "old",
    );
    assert.deepEqual(state.readStored(), installReceipt("old"));
    assert.deepEqual(await readdir(values.applicationsDirectory), [
      "resume-ir.app",
    ]);
  }
});

test("rejects old versions, source drift, and a symlinked Applications root", async (context) => {
  const values = await fixture(context);
  const state = argumentsFor(values);
  await assert.rejects(
    reinstallMacosDmg({ ...state.args, installedVersion: "0.1.1" }),
    /current-version reinstall/,
  );

  const mismatched = argumentsFor(values, {
    dmgReceipt: { source: OLD_SOURCE },
  });
  await assert.rejects(
    reinstallMacosDmg(mismatched.args),
    /DMG composition receipt is invalid/,
  );

  const linked = path.join(values.root, "LinkedApplications");
  await symlink(values.applicationsDirectory, linked);
  await assert.rejects(
    reinstallMacosDmg({
      ...state.args,
      applicationsDirectory: linked,
    }),
    /Applications root is invalid/,
  );
});

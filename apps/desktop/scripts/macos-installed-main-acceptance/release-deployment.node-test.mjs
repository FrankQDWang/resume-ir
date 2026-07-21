import assert from "node:assert/strict";
import test from "node:test";

import {
  deployExactInstalledRelease,
  parseReleaseBuildReceipt,
} from "./release-deployment.mjs";
import { MACOS_TEST_RELEASE_ERROR_CODES } from "../macos-test-release.mjs";
import { COMPOSITION, DMG, HEAD, ICON } from "./fixtures.mjs";

const baseOptions = Object.freeze({
  applicationSupportRoot: "/synthetic/support",
  repoRoot: "/synthetic/repo",
  temporaryParent: "/synthetic/tmp",
});
const BUILD_ENVIRONMENT = Object.freeze({
  CARGO_HOME: "/synthetic/tmp/cargo-home",
  HOME: "/synthetic/tmp/home",
  LANG: "C",
  LC_ALL: "C",
  NPM_CONFIG_CACHE: "/synthetic/tmp/npm-cache",
  NPM_CONFIG_GLOBALCONFIG: "/synthetic/tmp/npm-global-config",
  NPM_CONFIG_SCRIPT_SHELL: "/bin/sh",
  NPM_CONFIG_UPDATE_NOTIFIER: "false",
  NPM_CONFIG_USERCONFIG: "/synthetic/tmp/npm-user-config",
  PATH: "/synthetic/tmp/tool-bin:/usr/bin:/bin:/usr/sbin:/sbin",
  RUSTUP_HOME: "/synthetic/rustup",
  TMPDIR: "/synthetic/tmp/runtime",
});

test("preserves a typed release-stage failure from the isolated build entry", async () => {
  const runTool = async () => ({
    status: 1,
    timedOut: false,
    overflow: false,
    stderr: "",
    stdout:
      '{"schema_version":"resume-ir.macos-test-release-failure.v1","outcome":"failed","error_code":"release_dmg_verification_failed"}\n',
  });

  await assert.rejects(
    async () => parseReleaseBuildReceipt(await runTool()),
    /release_dmg_verification_failed/,
  );
});

test("rejects malformed or unknown release failure receipts", async () => {
  for (const stdout of [
    '{"schema_version":"resume-ir.macos-test-release-failure.v1","outcome":"failed","error_code":"release_unknown"}\n',
    '{"schema_version":"resume-ir.macos-test-release-failure.v1","outcome":"failed","error_code":"release_build_tool_failed","detail":"private"}\n',
    "not-json\n",
  ]) {
    await assert.rejects(
      async () =>
        parseReleaseBuildReceipt({
          status: 1,
          timedOut: false,
          overflow: false,
          stderr: "",
          stdout,
        }),
      /release_build_failed/,
    );
  }
});

test("forwards every closed release failure class without accepting extra fields", async () => {
  for (const errorCode of MACOS_TEST_RELEASE_ERROR_CODES) {
    assert.throws(
      () =>
        parseReleaseBuildReceipt({
          status: 1,
          timedOut: false,
          overflow: false,
          stderr: "",
          stdout: `${JSON.stringify({
            schema_version: "resume-ir.macos-test-release-failure.v1",
            outcome: "failed",
            error_code: errorCode,
          })}\n`,
        }),
      new RegExp(errorCode),
    );
  }
});

test("does not trust a typed failure receipt from an unhealthy child result", () => {
  const stdout = `${JSON.stringify({
    schema_version: "resume-ir.macos-test-release-failure.v1",
    outcome: "failed",
    error_code: "release_build_tool_failed",
  })}\n`;

  for (const unhealthy of [
    { timedOut: true, overflow: false, stderr: "" },
    { timedOut: false, overflow: true, stderr: "" },
    { timedOut: false, overflow: false, stderr: "unexpected" },
  ]) {
    assert.throws(
      () =>
        parseReleaseBuildReceipt({
          status: 1,
          stdout,
          ...unhealthy,
        }),
      /release_build_failed/,
    );
  }
});

function dependencies(installedVersion) {
  const calls = [];
  return {
    calls,
    values: {
      deriveCommitProductBinding: async () => {
        calls.push("source");
        return { iconSha256: ICON, version: "0.1.2" };
      },
      verifyGitMainBinding: async () => {
        calls.push("git");
        return { detached: false, gitHead: HEAD };
      },
      assertMutationAuthority: async () => {
        calls.push("guard");
        return { gitHead: HEAD, iconSha256: ICON, version: "0.1.2" };
      },
      createImmutableBuildSource: async () => {
        calls.push("immutable-clone");
        return {
          buildEnvironment: BUILD_ENVIRONMENT,
          repoRoot: "/synthetic/tmp/immutable-commit-root",
          async cleanup() {
            calls.push("immutable-cleanup");
          },
        };
      },
      buildVerifiedDmg: async () => {
        calls.push("build");
        return {
          appCompositionDigest: COMPOSITION,
          dmg: "/synthetic/release/resume-ir_0.1.2_aarch64.dmg",
          dmgSha256: DMG,
          sourceCommit: HEAD,
        };
      },
      inspectInstalledVersion: async () => {
        calls.push("inspect");
        return installedVersion;
      },
      installCurrent: async () => calls.push("install"),
      uninstallCurrent: async () => calls.push("uninstall"),
      upgradeLegacy: async () => calls.push("upgrade"),
      verifyInstalledSourceBindings: async () => {
        calls.push("verify-installed");
        return {
          composition: { composition_digest: COMPOSITION },
          dmgSha256: DMG,
          gitHead: HEAD,
          iconSha256: ICON,
          version: "0.1.2",
        };
      },
    },
  };
}

test("always builds and promotes the exact release instead of accepting a pre-existing App", async () => {
  for (const [installedVersion, expectedAction, expectedCalls] of [
    [undefined, "install", ["git", "source", "guard", "immutable-clone", "guard", "build", "guard", "inspect", "guard", "install", "verify-installed", "immutable-cleanup"]],
    ["0.1.1", "upgrade", ["git", "source", "guard", "immutable-clone", "guard", "build", "guard", "inspect", "guard", "upgrade", "verify-installed", "immutable-cleanup"]],
    [
      "0.1.2",
      "reinstall",
      ["git", "source", "guard", "immutable-clone", "guard", "build", "guard", "inspect", "guard", "uninstall", "guard", "install", "verify-installed", "immutable-cleanup"],
    ],
  ]) {
    const fixture = dependencies(installedVersion);
    const result = await deployExactInstalledRelease(
      baseOptions,
      fixture.values,
    );
    assert.equal(result.deploymentAction, expectedAction);
    assert.equal(result.version, "0.1.2");
    assert.equal(result.gitHead, HEAD);
    assert.equal(result.compositionDigest, COMPOSITION);
    assert.deepEqual(fixture.calls, expectedCalls);
  }
});

test("rejects future source, future installed versions, and build-to-install drift", async () => {
  const futureSource = dependencies(undefined);
  futureSource.values.deriveCommitProductBinding = async () => ({
    iconSha256: ICON,
    version: "0.1.3",
  });
  await assert.rejects(
    deployExactInstalledRelease(baseOptions, futureSource.values),
    /required_release_invalid/,
  );
  assert.deepEqual(futureSource.calls, ["git"]);

  const futureInstalled = dependencies("0.1.3");
  await assert.rejects(
    deployExactInstalledRelease(baseOptions, futureInstalled.values),
    /installed_version_invalid/,
  );
  assert.deepEqual(futureInstalled.calls, [
    "git",
    "source",
    "guard",
    "immutable-clone",
    "guard",
    "build",
    "guard",
    "inspect",
    "immutable-cleanup",
  ]);

  const drift = dependencies(undefined);
  drift.values.verifyInstalledSourceBindings = async () => ({
    composition: { composition_digest: "f".repeat(64) },
    dmgSha256: DMG,
    gitHead: HEAD,
    iconSha256: ICON,
    version: "0.1.2",
  });
  await assert.rejects(
    deployExactInstalledRelease(baseOptions, drift.values),
    /installed_deployment_binding_mismatch/,
  );

  const dmgDrift = dependencies(undefined);
  dmgDrift.values.verifyInstalledSourceBindings = async () => ({
    composition: { composition_digest: COMPOSITION },
    dmgSha256: "f".repeat(64),
    gitHead: HEAD,
    iconSha256: ICON,
    version: "0.1.2",
  });
  await assert.rejects(
    deployExactInstalledRelease(baseOptions, dmgDrift.values),
    /installed_deployment_binding_mismatch/,
  );
});

test("dirty, non-main, and ahead-or-behind provenance cannot reach build or system mutation", async () => {
  for (const provenanceFailure of ["dirty", "non-main", "ahead-or-behind"]) {
    const fixture = dependencies(undefined);
    fixture.values.verifyGitMainBinding = async () => {
      fixture.calls.push(`git-${provenanceFailure}`);
      throw new Error("git_main_binding_invalid");
    };
    await assert.rejects(
      deployExactInstalledRelease(baseOptions, fixture.values),
      /git_main_binding_invalid/,
    );
    assert.deepEqual(fixture.calls, [`git-${provenanceFailure}`]);
    assert.equal(
      fixture.calls.some((call) =>
        ["build", "inspect", "install", "uninstall", "upgrade"].includes(
          call,
        ),
      ),
      false,
    );
  }
});

test("builds only from an immutable commit-derived root and rechecks authority after the await", async () => {
  const fixture = dependencies(undefined);
  let live = true;
  const systemMutations = [];
  fixture.values.assertMutationAuthority = async () => {
    fixture.calls.push("guard");
    if (!live) throw new Error("source_authority_changed");
  };
  fixture.values.createImmutableBuildSource = async ({ source }) => {
    fixture.calls.push("immutable-clone");
    assert.equal(source.gitHead, HEAD);
    return {
      buildEnvironment: BUILD_ENVIRONMENT,
      repoRoot: "/synthetic/tmp/immutable-commit-root",
      async cleanup() {
        fixture.calls.push("immutable-cleanup");
      },
    };
  };
  fixture.values.buildVerifiedDmg = async ({ repoRoot }) => {
    fixture.calls.push("build");
    assert.equal(repoRoot, "/synthetic/tmp/immutable-commit-root");
    assert.notEqual(repoRoot, baseOptions.repoRoot);
    live = false;
    return {
      appCompositionDigest: COMPOSITION,
      dmg: "/synthetic/release/resume-ir_0.1.2_aarch64.dmg",
      dmgSha256: DMG,
      sourceCommit: HEAD,
    };
  };
  for (const operation of ["installCurrent", "uninstallCurrent", "upgradeLegacy"]) {
    fixture.values[operation] = async () => systemMutations.push(operation);
  }

  await assert.rejects(
    deployExactInstalledRelease(
      { ...baseOptions, preverifiedSource: { gitHead: HEAD, iconSha256: ICON, version: "0.1.2" } },
      fixture.values,
    ),
    /source_authority_changed/,
  );
  assert.deepEqual(systemMutations, []);
  assert.equal(fixture.calls.includes("immutable-cleanup"), true);
});

test("reinstall rechecks live authority before both uninstall and install", async () => {
  const fixture = dependencies("0.1.2");
  let guardCalls = 0;
  const mutations = [];
  fixture.values.assertMutationAuthority = async () => {
    guardCalls += 1;
    fixture.calls.push(`guard-${guardCalls}`);
    if (mutations.length === 1) throw new Error("lifecycle_lock_lost");
  };
  fixture.values.createImmutableBuildSource = async () => ({
    buildEnvironment: BUILD_ENVIRONMENT,
    repoRoot: "/synthetic/tmp/immutable-commit-root",
    async cleanup() {},
  });
  fixture.values.uninstallCurrent = async () => mutations.push("uninstall");
  fixture.values.installCurrent = async () => mutations.push("install");

  await assert.rejects(
    deployExactInstalledRelease(
      { ...baseOptions, preverifiedSource: { gitHead: HEAD, iconSha256: ICON, version: "0.1.2" } },
      fixture.values,
    ),
    /lifecycle_lock_lost/,
  );
  assert.deepEqual(mutations, ["uninstall"]);
});

test("a drift-and-restore during build cannot alter the commit-derived build root", async () => {
  const fixture = dependencies(undefined);
  let liveAuthority = HEAD;
  let builtFrom;
  fixture.values.assertMutationAuthority = async () => {
    fixture.calls.push("guard");
    if (liveAuthority !== HEAD) throw new Error("source_authority_changed");
    return { gitHead: HEAD, iconSha256: ICON, version: "0.1.2" };
  };
  fixture.values.buildVerifiedDmg = async ({ repoRoot }) => {
    builtFrom = repoRoot;
    fixture.calls.push("build");
    liveAuthority = "f".repeat(40);
    await new Promise((resolve) => setImmediate(resolve));
    liveAuthority = HEAD;
    return {
      appCompositionDigest: COMPOSITION,
      dmg: "/synthetic/release/resume-ir_0.1.2_aarch64.dmg",
      dmgSha256: DMG,
      sourceCommit: HEAD,
    };
  };
  const result = await deployExactInstalledRelease(
    {
      ...baseOptions,
      preverifiedSource: {
        gitHead: HEAD,
        iconSha256: ICON,
        version: "0.1.2",
      },
    },
    fixture.values,
  );
  assert.equal(result.deploymentAction, "install");
  assert.equal(builtFrom, "/synthetic/tmp/immutable-commit-root");
  assert.notEqual(builtFrom, baseOptions.repoRoot);
});

test("passes the immutable source closed environment through the Tauri build boundary", async () => {
  const fixture = dependencies(undefined);
  fixture.values.createImmutableBuildSource = async () => ({
    buildEnvironment: BUILD_ENVIRONMENT,
    repoRoot: "/synthetic/tmp/immutable-commit-root",
    async cleanup() {},
  });
  fixture.values.buildVerifiedDmg = async ({ environment, repoRoot }) => {
    assert.equal(repoRoot, "/synthetic/tmp/immutable-commit-root");
    assert.equal(environment, BUILD_ENVIRONMENT);
    assert.equal(environment.PATH.includes(baseOptions.repoRoot), false);
    return {
      appCompositionDigest: COMPOSITION,
      dmg: "/synthetic/release/resume-ir_0.1.2_aarch64.dmg",
      dmgSha256: DMG,
      sourceCommit: HEAD,
    };
  };

  await deployExactInstalledRelease(baseOptions, fixture.values);
});

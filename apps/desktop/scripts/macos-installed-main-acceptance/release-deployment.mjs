import { lstat, readFile, realpath } from "node:fs/promises";
import path from "node:path";

import { inspectMacosAppBundle } from "../macos-install-lifecycle.mjs";
import { defaultApplicationSupportRoot } from "../macos-install-receipt.mjs";
import {
  createMacosInternalTestPlan,
  MACOS_TEST_RELEASE_ERROR_CODES,
  MACOS_TEST_RELEASE_FAILURE_SCHEMA,
} from "../macos-test-release.mjs";
import { runBoundedTool, toolSucceeded } from "./bounded-process.mjs";
import {
  CLONE_TIMEOUT_MS,
  INSTALLED_APP_BUNDLE,
  TARGET_TRIPLE,
  exactKeys,
  fail,
} from "./core.mjs";
import {
  createImmutableBuildSource,
  validClosedBuildEnvironment,
} from "./immutable-build-source.mjs";
import {
  REQUIRED_INSTALLED_VERSION,
  deriveCommitProductBinding,
  verifyGitMainBinding,
  verifyInstalledSourceBindings,
} from "./source-bindings.mjs";

const LEGACY_VERSION = "0.1.1";
const APPLICATIONS_DIRECTORY = "/Applications";
const MAX_CONFIG_BYTES = 64 * 1024;

async function readBoundedConfig(file) {
  let metadata;
  let resolved;
  let source;
  try {
    [metadata, resolved] = await Promise.all([lstat(file), realpath(file)]);
    if (
      !metadata.isFile() ||
      metadata.isSymbolicLink() ||
      resolved !== path.resolve(file) ||
      metadata.size === 0 ||
      metadata.size > MAX_CONFIG_BYTES
    ) {
      fail("required_release_invalid");
    }
    source = await readFile(file, "utf8");
    return JSON.parse(source);
  } catch (error) {
    if (error?.code === "required_release_invalid") throw error;
    fail("required_release_invalid");
  }
}

function parseJsonLine(result, failureCode) {
  if (
    result.stderr !== "" ||
    typeof result.stdout !== "string" ||
    !result.stdout.endsWith("\n") ||
    result.stdout.slice(0, -1).includes("\n")
  ) {
    fail(failureCode);
  }
  try {
    return JSON.parse(result.stdout);
  } catch {
    fail(failureCode);
  }
}

function parseSingleJsonLine(result, failureCode) {
  if (!toolSucceeded(result)) fail(failureCode);
  return parseJsonLine(result, failureCode);
}

export function parseReleaseBuildReceipt(result) {
  const receipt = parseJsonLine(result, "release_build_failed");
  if (toolSucceeded(result)) return receipt;
  if (
    result.timedOut === false &&
    result.overflow === false &&
    Number.isSafeInteger(result.status) &&
    result.status !== 0 &&
    exactKeys(receipt, ["schema_version", "outcome", "error_code"]) &&
    receipt.schema_version === MACOS_TEST_RELEASE_FAILURE_SCHEMA &&
    receipt.outcome === "failed" &&
    MACOS_TEST_RELEASE_ERROR_CODES.includes(receipt.error_code)
  ) {
    fail(receipt.error_code);
  }
  fail("release_build_failed");
}

async function defaultBuildVerifiedDmg({
  environment,
  repoRoot,
  runTool,
  signal,
  source,
}) {
  if (!validClosedBuildEnvironment(environment)) {
    fail("immutable_build_toolchain_invalid");
  }
  const frontendRoot = path.join(repoRoot, "apps", "desktop");
  const baseConfigFile = path.join(
    frontendRoot,
    "src-tauri",
    "tauri.conf.json",
  );
  const platformConfigFile = path.join(
    frontendRoot,
    "src-tauri",
    "tauri.macos.conf.json",
  );
  const [baseConfig, platformConfig] = await Promise.all([
    readBoundedConfig(baseConfigFile),
    readBoundedConfig(platformConfigFile),
  ]);
  if (
    source.version !== REQUIRED_INSTALLED_VERSION ||
    baseConfig.version !== REQUIRED_INSTALLED_VERSION
  ) {
    fail("required_release_invalid");
  }
  let plan;
  try {
    plan = createMacosInternalTestPlan({
      frontendRoot,
      baseConfig,
      platformConfig,
    });
  } catch {
    fail("required_release_invalid");
  }
  const script = path.join(frontendRoot, "scripts", "macos-test-release.mjs");
  const receipt = parseReleaseBuildReceipt(
    await runTool(process.execPath, [script], {
      cwd: repoRoot,
      env: environment,
      signal,
      timeoutMs: CLONE_TIMEOUT_MS,
    }),
  );
  if (
    receipt?.schema_version !== "resume-ir.macos-dmg-composition.v2" ||
    !/^[a-f0-9]{40}$/.test(receipt.source_commit ?? "") ||
    !/^[a-f0-9]{64}$/.test(receipt.dmg_sha256 ?? "") ||
    !/^[a-f0-9]{64}$/.test(receipt.app_composition_digest ?? "")
  ) {
    fail("release_build_failed");
  }
  return Object.freeze({
    appCompositionDigest: receipt.app_composition_digest,
    dmg: plan.dmg,
    dmgSha256: receipt.dmg_sha256,
    sourceCommit: receipt.source_commit,
  });
}

async function defaultInspectInstalledVersion({ runTool }) {
  try {
    const metadata = await lstat(INSTALLED_APP_BUNDLE);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      fail("installed_version_invalid");
    }
  } catch (error) {
    if (error?.code === "ENOENT") return undefined;
    if (error?.code === "installed_version_invalid") throw error;
    fail("installed_version_invalid");
  }
  let identity;
  try {
    identity = await inspectMacosAppBundle({
      appBundle: INSTALLED_APP_BUNDLE,
      runner: runTool,
    });
  } catch {
    fail("installed_version_invalid");
  }
  if (
    identity.bundle_id !== "local.resume-ir.desktop" ||
    identity.display_name !== "resume-ir" ||
    identity.icon_file !== "icon.icns"
  ) {
    fail("installed_version_invalid");
  }
  return identity.version;
}

async function runLifecycleCommand({
  args,
  expected,
  repoRoot,
  runTool,
  script,
  signal,
}) {
  const receipt = parseSingleJsonLine(
    await runTool(process.execPath, [script, ...args], {
      cwd: repoRoot,
      env: process.env,
      signal,
      timeoutMs: CLONE_TIMEOUT_MS,
    }),
    "release_promotion_failed",
  );
  if (!expected(receipt)) fail("release_promotion_failed");
}

function defaultPromotionOperations({ repoRoot, runTool, signal }) {
  const scriptsRoot = path.join(repoRoot, "apps", "desktop", "scripts");
  const lifecycle = path.join(scriptsRoot, "macos-install-lifecycle.mjs");
  const upgrade = path.join(scriptsRoot, "macos-upgrade-lifecycle.mjs");
  const common = [
    "--target",
    TARGET_TRIPLE,
    "--applications",
    APPLICATIONS_DIRECTORY,
  ];
  const invoke = (script, args, expected) =>
    runLifecycleCommand({
      args,
      expected,
      repoRoot,
      runTool,
      script,
      signal,
    });
  return Object.freeze({
    installCurrent: ({ dmg }) =>
      invoke(
        lifecycle,
        [
          "install",
          ...common,
          "--dmg",
          dmg,
          "--version",
          REQUIRED_INSTALLED_VERSION,
        ],
        (receipt) =>
          receipt?.schema_version === "resume-ir.macos-installed-app.v1" &&
          receipt.version === REQUIRED_INSTALLED_VERSION,
      ),
    uninstallCurrent: () =>
      invoke(
        lifecycle,
        [
          "uninstall",
          ...common,
          "--version",
          REQUIRED_INSTALLED_VERSION,
        ],
        (receipt) =>
          receipt?.schema_version === "resume-ir.macos-uninstall.v1",
      ),
    upgradeLegacy: ({ dmg }) =>
      invoke(
        upgrade,
        [
          ...common,
          "--dmg",
          dmg,
          "--installed-version",
          LEGACY_VERSION,
          "--candidate-version",
          REQUIRED_INSTALLED_VERSION,
        ],
        (receipt) =>
          receipt?.schema_version === "resume-ir.macos-app-upgrade.v1" &&
          receipt.from_version === LEGACY_VERSION &&
          receipt.to_version === REQUIRED_INSTALLED_VERSION,
      ),
  });
}

function requireDeploymentBinding({ built, installed, source }) {
  if (
    source.version !== REQUIRED_INSTALLED_VERSION ||
    installed?.version !== REQUIRED_INSTALLED_VERSION ||
    installed.gitHead !== built.sourceCommit ||
    installed.dmgSha256 !== built.dmgSha256 ||
    installed.iconSha256 !== source.iconSha256 ||
    installed.composition?.composition_digest !==
      built.appCompositionDigest
  ) {
    fail("installed_deployment_binding_mismatch");
  }
}

function requireSourceBinding(source) {
  if (
    source?.version !== REQUIRED_INSTALLED_VERSION ||
    !/^[a-f0-9]{40}$/.test(source.gitHead ?? "") ||
    !/^[a-f0-9]{64}$/.test(source.iconSha256 ?? "")
  ) {
    fail("required_release_invalid");
  }
  return source;
}

export function sourceAuthorityMatches(expected, observed) {
  return (
    expected?.gitHead === observed?.gitHead &&
    expected?.version === observed?.version &&
    expected?.iconSha256 === observed?.iconSha256
  );
}

export async function verifyReleaseSourceBeforeDeployment(
  { repoRoot },
  overrides = {},
) {
  const runTool = overrides.runTool ?? runBoundedTool;
  const git = await (overrides.verifyGitMainBinding ?? verifyGitMainBinding)(
    repoRoot,
    runTool,
  );
  const product = await (
    overrides.deriveCommitProductBinding ?? deriveCommitProductBinding
  )(repoRoot, git?.gitHead, runTool);
  return Object.freeze(
    requireSourceBinding({
      gitHead: git?.gitHead,
      iconSha256: product?.iconSha256,
      version: product?.version,
    }),
  );
}

export async function deployExactInstalledRelease(options, overrides = {}) {
  const runTool = overrides.runTool ?? runBoundedTool;
  const expectedSource = requireSourceBinding(
    options.preverifiedSource ??
      (await verifyReleaseSourceBeforeDeployment(options, {
        ...overrides,
        runTool,
      })),
  );
  const assertMutationAuthority =
    overrides.assertMutationAuthority ??
    (async () => {
      const observed = await verifyReleaseSourceBeforeDeployment(options, {
        ...overrides,
        runTool,
      });
      if (!sourceAuthorityMatches(expectedSource, observed)) {
        fail("source_authority_changed");
      }
      return observed;
    });
  const guard = async (operation) => {
    const observed = await assertMutationAuthority(operation, expectedSource);
    if (observed && !sourceAuthorityMatches(expectedSource, observed)) {
      fail("source_authority_changed");
    }
  };
  let immutable;
  try {
    await guard("create_immutable_build_source");
    immutable = await (
      overrides.createImmutableBuildSource ?? createImmutableBuildSource
    )({
      repoRoot: options.repoRoot,
      runTool,
      signal: options.signal,
      source: expectedSource,
      temporaryParent: options.temporaryParent,
    });
    if (
      typeof immutable?.repoRoot !== "string" ||
      !path.isAbsolute(immutable.repoRoot) ||
      immutable.repoRoot === options.repoRoot ||
      !validClosedBuildEnvironment(immutable.buildEnvironment) ||
      typeof immutable.cleanup !== "function"
    ) {
      fail("immutable_build_source_invalid");
    }
    await guard("build_release");
    const built = await (
      overrides.buildVerifiedDmg ?? defaultBuildVerifiedDmg
    )({
      environment: immutable.buildEnvironment,
      repoRoot: immutable.repoRoot,
      runTool,
      signal: options.signal,
      source: expectedSource,
    });
    if (built?.sourceCommit !== expectedSource.gitHead) {
      fail("release_build_failed");
    }
    await guard("post_build");
    const installedVersion = await (
      overrides.inspectInstalledVersion ?? defaultInspectInstalledVersion
    )({ ...options, runTool });
    const defaults = defaultPromotionOperations({
      repoRoot: immutable.repoRoot,
      runTool,
      signal: options.signal,
    });
    const operations = {
      installCurrent: overrides.installCurrent ?? defaults.installCurrent,
      uninstallCurrent: overrides.uninstallCurrent ?? defaults.uninstallCurrent,
      upgradeLegacy: overrides.upgradeLegacy ?? defaults.upgradeLegacy,
    };
    let deploymentAction;
    if (installedVersion === undefined) {
      await guard("install_current");
      await operations.installCurrent({ dmg: built.dmg });
      deploymentAction = "install";
    } else if (installedVersion === LEGACY_VERSION) {
      await guard("upgrade_legacy");
      await operations.upgradeLegacy({ dmg: built.dmg });
      deploymentAction = "upgrade";
    } else if (installedVersion === REQUIRED_INSTALLED_VERSION) {
      await guard("uninstall_current");
      await operations.uninstallCurrent();
      await guard("install_current");
      await operations.installCurrent({ dmg: built.dmg });
      deploymentAction = "reinstall";
    } else {
      fail("installed_version_invalid");
    }
    const installed = await (
      overrides.verifyInstalledSourceBindings ?? verifyInstalledSourceBindings
    )({
      applicationSupportRoot: options.applicationSupportRoot,
      repoRoot: options.repoRoot,
      runTool,
    });
    requireDeploymentBinding({ built, installed, source: expectedSource });
    return Object.freeze({
      compositionDigest: built.appCompositionDigest,
      deploymentAction,
      dmgSha256: built.dmgSha256,
      gitHead: built.sourceCommit,
      version: REQUIRED_INSTALLED_VERSION,
    });
  } finally {
    await immutable?.cleanup();
  }
}

export async function requireDefaultApplicationSupportRoot(candidate) {
  let expected;
  let actual;
  try {
    [expected, actual] = await Promise.all([
      defaultApplicationSupportRoot().then(realpath),
      realpath(candidate),
    ]);
  } catch {
    fail("application_support_root_invalid");
  }
  if (expected !== actual) fail("application_support_root_invalid");
  return actual;
}

import { randomBytes } from "node:crypto";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  realpath,
  rename,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { MACOS_SYSTEM_TOOLS } from "../macos-system-tools.mjs";
import { toolSucceeded } from "./bounded-process.mjs";
import {
  CLONE_TIMEOUT_MS,
  GIT_HEAD,
  currentUid,
  exactKeys,
  fail,
} from "./core.mjs";
import { requireSecureDirectory } from "./filesystem-cow.mjs";
import { stageImmutableRuntimePacks } from "./immutable-runtime-packs.mjs";

const BUILD_PREFIX = ".resume-ir-installed-main-build-";
const EXPECTED_ORIGIN = "https://github.com/FrankQDWang/resume-ir.git";
const SYSTEM_BUILD_PATH = Object.freeze([
  "/usr/bin",
  "/bin",
  "/usr/sbin",
  "/sbin",
]);
const BUILD_ENVIRONMENT_KEYS = Object.freeze([
  "CARGO_HOME",
  "HOME",
  "LANG",
  "LC_ALL",
  "NPM_CONFIG_CACHE",
  "NPM_CONFIG_GLOBALCONFIG",
  "NPM_CONFIG_SCRIPT_SHELL",
  "NPM_CONFIG_UPDATE_NOTIFIER",
  "NPM_CONFIG_USERCONFIG",
  "PATH",
  "RUSTUP_HOME",
  "TMPDIR",
]);
const GIT_ENVIRONMENT = Object.freeze({
  GIT_CONFIG_GLOBAL: "/dev/null",
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_NO_REPLACE_OBJECTS: "1",
  GIT_OPTIONAL_LOCKS: "0",
  GIT_TERMINAL_PROMPT: "0",
  HOME: "/var/empty",
  LANG: "C",
  LC_ALL: "C",
  PATH: "/usr/bin:/bin",
});

function exactSuccess(result, stdout = "") {
  return (
    toolSucceeded(result) && result.stderr === "" && result.stdout === stdout
  );
}

async function requireCanonicalExecutable(candidate) {
  let metadata;
  let resolved;
  try {
    resolved = await realpath(candidate);
    metadata = await lstat(resolved);
  } catch {
    fail("immutable_build_toolchain_invalid");
  }
  if (
    !path.isAbsolute(candidate ?? "") ||
    !path.isAbsolute(resolved) ||
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    ![0, currentUid()].includes(metadata.uid) ||
    (metadata.mode & 0o022) !== 0 ||
    (metadata.mode & 0o111) === 0
  ) {
    fail("immutable_build_toolchain_invalid");
  }
  return resolved;
}

async function createPrivateDirectory(directory) {
  await mkdir(directory, { mode: 0o700 });
  await chmod(directory, 0o700);
  return directory;
}

async function createCanonicalBuildToolchain(root, runtime = {}) {
  let homeCandidate = runtime.homeDirectory;
  if (homeCandidate === undefined) {
    try {
      homeCandidate = os.userInfo().homedir;
    } catch {
      fail("immutable_build_toolchain_invalid");
    }
  }
  const nodeCandidate = runtime.nodeExecutable ?? process.execPath;
  const home = (
    await requireSecureDirectory(homeCandidate, {
      allowedUids: [currentUid()],
    })
  ).resolved;
  const nodeExecutable = await requireCanonicalExecutable(nodeCandidate);
  const npmCli = await requireCanonicalExecutable(
    path.resolve(
      path.dirname(nodeExecutable),
      "..",
      "lib",
      "node_modules",
      "npm",
      "bin",
      "npm-cli.js",
    ),
  );
  const cargoEntry = path.join(home, ".cargo", "bin", "cargo");
  const rustcEntry = path.join(home, ".cargo", "bin", "rustc");
  const [cargoExecutable, rustcExecutable] = await Promise.all([
    requireCanonicalExecutable(cargoEntry),
    requireCanonicalExecutable(rustcEntry),
  ]);
  const rustupHome = (
    await requireSecureDirectory(path.join(home, ".rustup"), {
      allowedUids: [currentUid()],
    })
  ).resolved;
  const toolBin = await createPrivateDirectory(path.join(root, "tool-bin"));
  const buildHome = await createPrivateDirectory(path.join(root, "home"));
  const cargoHome = await createPrivateDirectory(path.join(root, "cargo-home"));
  const npmCache = await createPrivateDirectory(path.join(root, "npm-cache"));
  const runtimeTemporary = await createPrivateDirectory(path.join(root, "tmp"));
  const npmGlobalConfig = path.join(root, "npm-global-config");
  const npmUserConfig = path.join(root, "npm-user-config");
  await Promise.all([
    writeFile(npmGlobalConfig, "", { flag: "wx", mode: 0o600 }),
    writeFile(npmUserConfig, "", { flag: "wx", mode: 0o600 }),
  ]);
  await Promise.all([
    symlink(nodeExecutable, path.join(toolBin, "node")),
    symlink(npmCli, path.join(toolBin, "npm")),
    symlink(cargoExecutable, path.join(toolBin, "cargo")),
    symlink(rustcExecutable, path.join(toolBin, "rustc")),
  ]);
  const buildEnvironment = Object.freeze({
    CARGO_HOME: cargoHome,
    HOME: buildHome,
    LANG: "C",
    LC_ALL: "C",
    NPM_CONFIG_CACHE: npmCache,
    NPM_CONFIG_GLOBALCONFIG: npmGlobalConfig,
    NPM_CONFIG_SCRIPT_SHELL: "/bin/sh",
    NPM_CONFIG_UPDATE_NOTIFIER: "false",
    NPM_CONFIG_USERCONFIG: npmUserConfig,
    PATH: [toolBin, ...SYSTEM_BUILD_PATH].join(":"),
    RUSTUP_HOME: rustupHome,
    TMPDIR: runtimeTemporary,
  });
  return Object.freeze({ buildEnvironment, nodeExecutable, npmCli });
}

export function validClosedBuildEnvironment(value) {
  const pathEntries = typeof value?.PATH === "string" ? value.PATH.split(":") : [];
  return (
    exactKeys(value, BUILD_ENVIRONMENT_KEYS) &&
    [
      value.CARGO_HOME,
      value.HOME,
      value.NPM_CONFIG_CACHE,
      value.NPM_CONFIG_GLOBALCONFIG,
      value.NPM_CONFIG_USERCONFIG,
      value.RUSTUP_HOME,
      value.TMPDIR,
    ].every((entry) => path.isAbsolute(entry ?? "")) &&
    value.LANG === "C" &&
    value.LC_ALL === "C" &&
    value.NPM_CONFIG_GLOBALCONFIG !== value.NPM_CONFIG_USERCONFIG &&
    value.NPM_CONFIG_SCRIPT_SHELL === "/bin/sh" &&
    value.NPM_CONFIG_UPDATE_NOTIFIER === "false" &&
    pathEntries.length === SYSTEM_BUILD_PATH.length + 1 &&
    path.isAbsolute(pathEntries[0] ?? "") &&
    path.basename(pathEntries[0]) === "tool-bin" &&
    JSON.stringify(pathEntries.slice(1)) === JSON.stringify(SYSTEM_BUILD_PATH)
  );
}

async function removeExactBuildRoot(root, parent, identity) {
  const quarantine = path.join(
    parent,
    `.resume-ir-installed-main-build-quarantine-${randomBytes(32).toString("hex")}`,
  );
  try {
    const [current, parentBefore] = await Promise.all([
      lstat(root),
      lstat(parent),
    ]);
    if (
      !current.isDirectory() ||
      current.isSymbolicLink() ||
      current.dev !== identity.root.dev ||
      current.ino !== identity.root.ino ||
      !parentBefore.isDirectory() ||
      parentBefore.isSymbolicLink() ||
      parentBefore.dev !== identity.parent.dev ||
      parentBefore.ino !== identity.parent.ino
    ) {
      fail("immutable_build_cleanup_failed");
    }
    await rename(root, quarantine);
    const moved = await lstat(quarantine);
    if (moved.dev !== identity.root.dev || moved.ino !== identity.root.ino) {
      fail("immutable_build_cleanup_failed");
    }
    await rm(quarantine, { recursive: true, force: false });
  } catch (error) {
    if (error?.code === "immutable_build_cleanup_failed") throw error;
    fail("immutable_build_cleanup_failed");
  }
}

export async function createImmutableBuildSource({
  repoRoot,
  runTool,
  runtime,
  signal,
  source,
  stageRuntimePacks = stageImmutableRuntimePacks,
  temporaryParent,
}) {
  if (
    !path.isAbsolute(repoRoot ?? "") ||
    !path.isAbsolute(temporaryParent ?? "") ||
    !GIT_HEAD.test(source?.gitHead ?? "") ||
    typeof runTool !== "function"
  ) {
    fail("immutable_build_source_invalid");
  }
  const parent = await requireSecureDirectory(temporaryParent, {
    privateMode: true,
  });
  let root;
  let identity;
  try {
    root = await mkdtemp(path.join(parent.resolved, BUILD_PREFIX));
    await chmod(root, 0o700);
    const [resolved, metadata] = await Promise.all([realpath(root), lstat(root)]);
    if (
      resolved !== root ||
      path.dirname(root) !== parent.resolved ||
      !path.basename(root).startsWith(BUILD_PREFIX) ||
      !metadata.isDirectory() ||
      metadata.isSymbolicLink() ||
      metadata.uid !== process.getuid() ||
      (metadata.mode & 0o777) !== 0o700
    ) {
      fail("immutable_build_source_invalid");
    }
    identity = { parent: parent.metadata, root: metadata };
    const toolchain = await createCanonicalBuildToolchain(root, runtime);
    const immutableRepoRoot = path.join(root, "source");
    const git = (args, cwd = "/") =>
      runTool(MACOS_SYSTEM_TOOLS.git, args, {
        cwd,
        env: GIT_ENVIRONMENT,
        signal,
        timeoutMs: CLONE_TIMEOUT_MS,
      });
    if (
      !exactSuccess(
        await git([
          "clone",
          "--quiet",
          "--no-local",
          "--no-checkout",
          "--",
          repoRoot,
          immutableRepoRoot,
        ]),
      ) ||
      !exactSuccess(
        await git([
          "-C",
          immutableRepoRoot,
          "checkout",
          "--quiet",
          "--detach",
          source.gitHead,
        ]),
      ) ||
      !exactSuccess(
        await git([
          "-C",
          immutableRepoRoot,
          "remote",
          "set-url",
          "origin",
          EXPECTED_ORIGIN,
        ]),
      )
    ) {
      fail("immutable_build_source_invalid");
    }
    const head = await git([
      "-C",
      immutableRepoRoot,
      "rev-parse",
      "--verify",
      "HEAD",
    ]);
    const status = await git([
      "-C",
      immutableRepoRoot,
      "status",
      "--porcelain=v1",
      "--untracked-files=all",
    ]);
    if (!exactSuccess(head, `${source.gitHead}\n`) || !exactSuccess(status)) {
      fail("immutable_build_source_invalid");
    }
    await stageRuntimePacks({
      immutableRepoRoot,
      sourceRepoRoot: repoRoot,
    });
    const stagedStatus = await git([
      "-C",
      immutableRepoRoot,
      "status",
      "--porcelain=v1",
      "--untracked-files=all",
    ]);
    if (!exactSuccess(stagedStatus)) fail("immutable_build_source_invalid");
    const frontendRoot = path.join(immutableRepoRoot, "apps", "desktop");
    const install = await runTool(
      toolchain.nodeExecutable,
      [
        toolchain.npmCli,
        "ci",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
      ],
      {
        cwd: frontendRoot,
        env: toolchain.buildEnvironment,
        signal,
        timeoutMs: CLONE_TIMEOUT_MS,
      },
    );
    if (!toolSucceeded(install)) fail("immutable_build_source_invalid");
    return Object.freeze({
      buildEnvironment: toolchain.buildEnvironment,
      repoRoot: immutableRepoRoot,
      cleanup: () => removeExactBuildRoot(root, parent.resolved, identity),
    });
  } catch (error) {
    if (root && identity) {
      await removeExactBuildRoot(root, parent.resolved, identity).catch(() => {});
    }
    if (error?.code?.startsWith("immutable_build_")) throw error;
    fail("immutable_build_source_invalid");
  }
}

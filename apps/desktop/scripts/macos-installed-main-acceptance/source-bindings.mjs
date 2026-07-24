import { createHash } from "node:crypto";
import path from "node:path";

import {
  BUNDLE_COMPOSITION_SCHEMA,
  verifyBundleComposition,
} from "../macos-bundle-composition.mjs";
import {
  captureSourceIdentity,
  validateSourceIdentity,
} from "../macos-source-identity.mjs";
import {
  CLOSED_SYSTEM_TOOL_ENV,
  MACOS_SYSTEM_TOOLS,
} from "../macos-system-tools.mjs";
import { verifyMacosInternalTestSignaturePolicy } from "../verify-macos-dmg.mjs";
import {
  INSTALL_RECEIPT_SCHEMA,
  readInstallReceipt,
  verifyInstallReceipt,
} from "../macos-install-receipt.mjs";
import { toolSucceeded } from "./bounded-process.mjs";
import {
  GIT_HEAD,
  INSTALLED_APP_BUNDLE,
  TARGET_TRIPLE,
  TOOL_TIMEOUT_MS,
  fail,
} from "./core.mjs";

const MAX_MANIFEST_BYTES = 64 * 1024;
const MAX_ICON_BYTES = 8 * 1024 * 1024;
export const REQUIRED_INSTALLED_VERSION = "0.1.2";
const EXPECTED_ORIGIN = "https://github.com/FrankQDWang/resume-ir.git";
const EXPECTED_BUNDLE_ID = "local.resume-ir.desktop";
const EXPECTED_PRODUCT_NAME = "resume-ir";
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
const EXPECTED_EXECUTABLES = Object.freeze([
  Object.freeze({ role: "desktop", file: "resume-desktop" }),
  Object.freeze({ role: "daemon", file: "resume-daemon" }),
  Object.freeze({ role: "embedding_runtime", file: "resume-embedding-runtime" }),
  Object.freeze({ role: "pdf_renderer", file: "resume-pdf-render-runtime" }),
]);
const SOURCE_BINDING_PATHS = Object.freeze([
  "apps/desktop/package.json",
  "apps/desktop/src-tauri/icons/icon.icns",
  "apps/desktop/src-tauri/tauri.conf.json",
]);

function parseRemoteMain(source) {
  if (typeof source !== "string") fail("git_main_binding_invalid");
  const lines = source.trimEnd().split("\n");
  if (lines.length !== 1) fail("git_main_binding_invalid");
  const match = lines[0].match(/^([a-f0-9]{40})\trefs\/heads\/main$/);
  if (!match) fail("git_main_binding_invalid");
  return match[1];
}

function parseGitHead(result) {
  const value = result?.stdout?.trim();
  if (
    !toolSucceeded(result) ||
    result.stderr !== "" ||
    !GIT_HEAD.test(value ?? "")
  ) {
    fail("git_main_binding_invalid");
  }
  return value;
}

function parseBranch(result) {
  if (toolSucceeded(result) && result.stderr === "") {
    const branch = result.stdout.trim();
    if (branch !== "main") fail("git_main_binding_invalid");
    return Object.freeze({ branch, detached: false });
  }
  if (
    result?.status === 1 &&
    result.timedOut === false &&
    result.overflow === false &&
    result.stdout === "" &&
    result.stderr === ""
  ) {
    return Object.freeze({ branch: null, detached: true });
  }
  fail("git_main_binding_invalid");
}

export async function verifyGitMainBinding(repoRoot, runTool) {
  const git = (args) =>
    runTool(MACOS_SYSTEM_TOOLS.git, ["-C", repoRoot, ...args], {
      env: GIT_ENVIRONMENT,
      timeoutMs: TOOL_TIMEOUT_MS,
    });
  const headBefore = parseGitHead(await git(["rev-parse", "--verify", "HEAD"]));
  const branchBefore = parseBranch(
    await git(["symbolic-ref", "--quiet", "--short", "HEAD"]),
  );
  const dirtyBefore = await git([
    "status",
    "--porcelain=v1",
    "--untracked-files=all",
  ]);
  const originBefore = await git(["remote", "get-url", "--all", "origin"]);
  const rawOriginBefore = await git([
    "config",
    "--local",
    "--get-all",
    "remote.origin.url",
  ]);
  const trackedInputs = await git([
    "ls-files",
    "--error-unmatch",
    "--",
    ...SOURCE_BINDING_PATHS,
  ]);
  const remoteBefore = await git([
    "ls-remote",
    "--exit-code",
    "origin",
    "refs/heads/main",
  ]);
  const remoteAfter = await git([
    "ls-remote",
    "--exit-code",
    "origin",
    "refs/heads/main",
  ]);
  const rawOriginAfter = await git([
    "config",
    "--local",
    "--get-all",
    "remote.origin.url",
  ]);
  const originAfter = await git(["remote", "get-url", "--all", "origin"]);
  const dirtyAfter = await git([
    "status",
    "--porcelain=v1",
    "--untracked-files=all",
  ]);
  const branchAfter = parseBranch(
    await git(["symbolic-ref", "--quiet", "--short", "HEAD"]),
  );
  const headAfter = parseGitHead(await git(["rev-parse", "--verify", "HEAD"]));
  if (
    !toolSucceeded(dirtyBefore) ||
    !toolSucceeded(dirtyAfter) ||
    !toolSucceeded(originBefore) ||
    !toolSucceeded(originAfter) ||
    !toolSucceeded(rawOriginBefore) ||
    !toolSucceeded(rawOriginAfter) ||
    !toolSucceeded(remoteBefore) ||
    !toolSucceeded(remoteAfter) ||
    !toolSucceeded(trackedInputs) ||
    originBefore.stderr !== "" ||
    originAfter.stderr !== "" ||
    rawOriginBefore.stderr !== "" ||
    rawOriginAfter.stderr !== "" ||
    remoteBefore.stderr !== "" ||
    remoteAfter.stderr !== "" ||
    trackedInputs.stderr !== "" ||
    typeof trackedInputs.stdout !== "string" ||
    dirtyBefore.stderr !== "" ||
    dirtyAfter.stderr !== "" ||
    dirtyBefore.stdout !== "" ||
    dirtyAfter.stdout !== "" ||
    originBefore.stdout !== `${EXPECTED_ORIGIN}\n` ||
    originAfter.stdout !== originBefore.stdout ||
    rawOriginBefore.stdout !== `${EXPECTED_ORIGIN}\n` ||
    rawOriginAfter.stdout !== rawOriginBefore.stdout ||
    branchAfter.branch !== branchBefore.branch ||
    branchAfter.detached !== branchBefore.detached ||
    headAfter !== headBefore ||
    JSON.stringify(trackedInputs.stdout.trimEnd().split("\n").sort()) !==
      JSON.stringify([...SOURCE_BINDING_PATHS].sort())
  ) {
    fail("git_main_binding_invalid");
  }
  const remoteMainBefore = parseRemoteMain(remoteBefore.stdout);
  const remoteMainAfter = parseRemoteMain(remoteAfter.stdout);
  if (headBefore !== remoteMainBefore || remoteMainAfter !== remoteMainBefore) {
    fail("git_main_binding_invalid");
  }
  return Object.freeze({ detached: branchBefore.detached, gitHead: headBefore });
}

function parseCommitJson(result) {
  if (
    !toolSucceeded(result) ||
    result.stderr !== "" ||
    typeof result.stdout !== "string" ||
    Buffer.byteLength(result.stdout, "utf8") > MAX_MANIFEST_BYTES
  ) {
    fail("source_manifest_invalid");
  }
  try {
    return JSON.parse(result.stdout);
  } catch {
    fail("source_manifest_invalid");
  }
}

export async function deriveCommitProductBinding(repoRoot, gitHead, runTool) {
  if (!GIT_HEAD.test(gitHead ?? "") || typeof runTool !== "function") {
    fail("source_manifest_invalid");
  }
  const show = (relative) =>
    runTool(
      MACOS_SYSTEM_TOOLS.git,
      ["-C", repoRoot, "show", `${gitHead}:${relative}`],
      {
        env: GIT_ENVIRONMENT,
        timeoutMs: TOOL_TIMEOUT_MS,
      },
    );
  const packageManifest = parseCommitJson(await show("apps/desktop/package.json"));
  const tauriManifest = parseCommitJson(
    await show("apps/desktop/src-tauri/tauri.conf.json"),
  );
  const icon = await runTool(
    MACOS_SYSTEM_TOOLS.git,
    [
      "-C",
      repoRoot,
      "show",
      `${gitHead}:apps/desktop/src-tauri/icons/icon.icns`,
    ],
    {
      env: GIT_ENVIRONMENT,
      maxStdoutBytes: MAX_ICON_BYTES,
      stdoutMode: "buffer",
      timeoutMs: TOOL_TIMEOUT_MS,
    },
  );
  if (
    !toolSucceeded(icon) ||
    icon.stderr !== "" ||
    !Buffer.isBuffer(icon.stdout) ||
    icon.stdout.length < 1 ||
    icon.stdout.length > MAX_ICON_BYTES ||
    packageManifest?.name !== "resume-ir-desktop" ||
    packageManifest.version !== REQUIRED_INSTALLED_VERSION ||
    tauriManifest?.productName !== EXPECTED_PRODUCT_NAME ||
    tauriManifest?.identifier !== EXPECTED_BUNDLE_ID ||
    tauriManifest?.version !== packageManifest.version
  ) {
    fail("source_manifest_invalid");
  }
  return Object.freeze({
    iconSha256: createHash("sha256").update(icon.stdout).digest("hex"),
    version: packageManifest.version,
  });
}

export async function deriveExactMainSourceIdentity(
  repoRoot,
  gitHead,
  capture = captureSourceIdentity,
) {
  if (
    !path.isAbsolute(repoRoot ?? "") ||
    !GIT_HEAD.test(gitHead ?? "") ||
    typeof capture !== "function"
  ) {
    fail("source_manifest_invalid");
  }
  let source;
  try {
    source = validateSourceIdentity(
      (
        await capture({
          repoRoot,
          authority: "exact_main_commit",
        })
      )?.identity,
    );
  } catch {
    fail("source_manifest_invalid");
  }
  if (
    source.authority !== "exact_main_commit" ||
    source.base_commit !== gitHead
  ) {
    fail("source_manifest_invalid");
  }
  return source;
}

function executablePaths(composition) {
  if (
    !Array.isArray(composition.executables) ||
    composition.executables.length !== EXPECTED_EXECUTABLES.length ||
    !composition.executables.every(
      (entry, index) =>
        entry.role === EXPECTED_EXECUTABLES[index].role &&
        entry.file === EXPECTED_EXECUTABLES[index].file,
    )
  ) {
    fail("installed_composition_invalid");
  }
  return Object.freeze(
    Object.fromEntries(
      composition.executables.map(({ role, file }) => [
        role,
        path.join(INSTALLED_APP_BUNDLE, "Contents", "MacOS", file),
      ]),
    ),
  );
}

export async function verifyInstalledSourceBindings({
  applicationSupportRoot,
  deriveSourceIdentity = deriveExactMainSourceIdentity,
  repoRoot,
  runTool,
  verifySignaturePolicy = verifyMacosInternalTestSignaturePolicy,
}) {
  const { gitHead } = await verifyGitMainBinding(repoRoot, runTool);
  const product = await deriveCommitProductBinding(repoRoot, gitHead, runTool);
  const source = await deriveSourceIdentity(repoRoot, gitHead);
  let composition;
  try {
    composition = await verifyBundleComposition({
      appBundle: INSTALLED_APP_BUNDLE,
      targetTriple: TARGET_TRIPLE,
      expectedVersion: product.version,
      expectedSource: source,
      verifySignaturePolicy: ({ appBundle }) =>
        verifySignaturePolicy({
          appBundle,
          platform: "darwin",
          runner: (command, args) =>
            runTool(command, args, {
              env: CLOSED_SYSTEM_TOOL_ENV,
              timeoutMs: TOOL_TIMEOUT_MS,
            }),
        }),
    });
  } catch {
    fail("installed_composition_invalid");
  }
  if (
    composition.schema_version !== BUNDLE_COMPOSITION_SCHEMA ||
    JSON.stringify(composition.source) !== JSON.stringify(source) ||
    composition.icon?.sha256 !== product.iconSha256
  ) {
    fail("installed_composition_binding_mismatch");
  }
  let receipt;
  try {
    receipt = await readInstallReceipt({ applicationSupportRoot });
    verifyInstallReceipt({ receipt, composition });
  } catch {
    fail("installed_receipt_invalid");
  }
  if (
    receipt?.schema_version !== INSTALL_RECEIPT_SCHEMA ||
    receipt.version !== REQUIRED_INSTALLED_VERSION ||
    composition.version !== REQUIRED_INSTALLED_VERSION ||
    JSON.stringify(receipt.source) !== JSON.stringify(source)
  ) {
    fail("installed_receipt_invalid");
  }
  return Object.freeze({
    composition,
    dmgSha256: receipt.dmg_sha256,
    executablePaths: executablePaths(composition),
    gitHead,
    iconSha256: product.iconSha256,
    source,
    version: product.version,
  });
}

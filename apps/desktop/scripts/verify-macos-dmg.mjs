import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  lstat,
  mkdtemp,
  readFile,
  readlink,
  readdir,
  realpath,
  rmdir,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { verifyBundledSidecar } from "./verify-bundled-sidecar.mjs";
import { verifyBundleComposition } from "./macos-bundle-composition.mjs";
import { verifyMainSourceProvenance } from "./macos-source-provenance.mjs";
import {
  MACOS_SYSTEM_TOOLS,
  runClosedSystemTool,
} from "./macos-system-tools.mjs";

export const MAX_DMG_BYTES = 1024 * 1024 * 1024;
export const APPLE_TOOL_TIMEOUT_MS = 120 * 1000;
const MAX_TOOL_OUTPUT_BYTES = 64 * 1024;
const APP_NAME = "resume-ir.app";
const OPTIONAL_METADATA = ".DS_Store";
const VOLUME_ICON = ".VolumeIcon.icns";
const MAX_VOLUME_ICON_BYTES = 8 * 1024 * 1024;
const LIBRARY_VALIDATION_ENTITLEMENT =
  "com.apple.security.cs.disable-library-validation";
const MAX_INFO_PLIST_BYTES = 64 * 1024;

async function defaultRunner(command, args) {
  return runClosedSystemTool(command, args, {
    encoding: "utf8",
    maxBuffer: MAX_TOOL_OUTPUT_BYTES,
    timeout: APPLE_TOOL_TIMEOUT_MS,
  });
}

function succeeded(result) {
  return !result?.error && result?.status === 0;
}

async function mountedFilesystemAt(mountDirectory) {
  try {
    const [mountMetadata, parentMetadata] = await Promise.all([
      lstat(mountDirectory),
      lstat(path.dirname(mountDirectory)),
    ]);
    return mountMetadata.dev !== parentMetadata.dev;
  } catch {
    return false;
  }
}

async function digestFile(file) {
  const digest = createHash("sha256");
  try {
    for await (const chunk of createReadStream(file)) digest.update(chunk);
  } catch {
    throw new Error("DMG file is invalid");
  }
  return digest.digest("hex");
}

function validDmgMetadata(metadata, maxDmgBytes) {
  return (
    metadata?.isFile() &&
    !metadata.isSymbolicLink() &&
    metadata.size > 0 &&
    metadata.size <= maxDmgBytes
  );
}

function sameDmgIdentity(left, right) {
  // hdiutil may update extended metadata during a read-only attach, which
  // changes ctime without changing the file identity or the verified bytes.
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.mode === right.mode &&
    left.size === right.size &&
    left.mtimeMs === right.mtimeMs
  );
}

async function metadataForDmg(dmg, maxDmgBytes, changed = false) {
  let metadata;
  try {
    metadata = await lstat(dmg);
  } catch {
    throw new Error(changed ? "DMG file changed during verification" : "DMG file is invalid");
  }
  if (!validDmgMetadata(metadata, maxDmgBytes)) {
    throw new Error(changed ? "DMG file changed during verification" : "DMG file is invalid");
  }
  return metadata;
}

function boundedToolOutput(result) {
  const output = `${result?.stdout ?? ""}\n${result?.stderr ?? ""}`;
  if (Buffer.byteLength(output, "utf8") > MAX_TOOL_OUTPUT_BYTES) {
    throw new Error("ad-hoc signature is invalid");
  }
  return output;
}

export async function verifyAdHocSignedApp({
  appBundle,
  platform = process.platform,
  runner = defaultRunner,
}) {
  if (platform !== "darwin" || !path.isAbsolute(appBundle)) {
    throw new Error("ad-hoc signature arguments are invalid");
  }
  try {
    const mainExecutable = await resolveMacosAppExecutable(appBundle);
    const macosDirectory = path.join(appBundle, "Contents", "MacOS");
    const targets = [
      { target: appBundle, bundle: true },
      { target: path.join(macosDirectory, mainExecutable), bundle: false },
      { target: path.join(macosDirectory, "resume-daemon"), bundle: false },
      {
        target: path.join(macosDirectory, "resume-embedding-runtime"),
        bundle: false,
      },
      {
        target: path.join(macosDirectory, "resume-pdf-render-runtime"),
        bundle: false,
      },
    ];
    for (const { target, bundle } of targets) {
      await validateSignedTarget(target);
      const verification = await runner(MACOS_SYSTEM_TOOLS.codesign, [
        "--verify",
        ...(bundle ? ["--deep"] : []),
        "--strict",
        "--verbose=2",
        target,
      ]);
      if (!succeeded(verification)) {
        throw new Error("ad-hoc signature is invalid");
      }
      const description = await runner(MACOS_SYSTEM_TOOLS.codesign, [
        "--display",
        "--verbose=4",
        target,
      ]);
      if (!succeeded(description)) {
        throw new Error("ad-hoc signature is invalid");
      }
      const metadata = boundedToolOutput(description);
      const flags = metadata.match(
        /^CodeDirectory .*flags=0x[0-9a-f]+\(([^)]+)\)/m,
      )?.[1].split(",");
      if (
        !/^Signature=adhoc$/m.test(metadata) ||
        !/^TeamIdentifier=not set$/m.test(metadata) ||
        (bundle &&
          !/^Sealed Resources version=\d+ rules=\d+ files=\d+$/m.test(
            metadata,
          )) ||
        JSON.stringify(flags) !== JSON.stringify(["adhoc", "runtime"]) ||
        /^Authority=/m.test(metadata)
      ) {
        throw new Error("ad-hoc signature is invalid");
      }
    }
  } catch {
    throw new Error("ad-hoc signature is invalid");
  }
  return Object.freeze({
    code_signature: "ad_hoc_valid",
    hardened_runtime: true,
  });
}

function isStrictDescendant(root, candidate) {
  const relative = path.relative(root, candidate);
  return (
    relative.length > 0 &&
    relative !== ".." &&
    !relative.startsWith(`..${path.sep}`) &&
    !path.isAbsolute(relative)
  );
}

export async function validateMountedDmgLayout({
  mountDirectory,
  allowFseventsd = false,
}) {
  if (!path.isAbsolute(mountDirectory)) {
    throw new Error("DMG mount path is invalid");
  }
  let entries;
  try {
    entries = (await readdir(mountDirectory)).sort();
  } catch {
    throw new Error("DMG root is unavailable");
  }
  const allowed = new Set([
    OPTIONAL_METADATA,
    VOLUME_ICON,
    "Applications",
    APP_NAME,
  ]);
  if (allowFseventsd) allowed.add(".fseventsd");
  if (
    entries.length < 2 ||
    entries.length > allowed.size ||
    entries.some((entry) => !allowed.has(entry))
  ) {
    throw new Error("unexpected DMG root entry");
  }
  if (
    !entries.includes("Applications") ||
    !entries.includes(APP_NAME) ||
    !entries.includes(VOLUME_ICON)
  ) {
    throw new Error("required DMG root entry is missing");
  }

  if (entries.includes(".fseventsd")) {
    const transient = path.join(mountDirectory, ".fseventsd");
    let metadata;
    let mountRealPath;
    let transientRealPath;
    try {
      [metadata, mountRealPath, transientRealPath] = await Promise.all([
        lstat(transient),
        realpath(mountDirectory),
        realpath(transient),
      ]);
    } catch {
      throw new Error("DMG transient metadata is invalid");
    }
    if (
      !allowFseventsd ||
      !metadata.isDirectory() ||
      metadata.isSymbolicLink() ||
      !isStrictDescendant(mountRealPath, transientRealPath)
    ) {
      throw new Error("DMG transient metadata is invalid");
    }
  }

  const appBundle = path.join(mountDirectory, APP_NAME);
  const applicationsLink = path.join(mountDirectory, "Applications");
  let appMetadata;
  let linkMetadata;
  try {
    [appMetadata, linkMetadata] = await Promise.all([
      lstat(appBundle),
      lstat(applicationsLink),
    ]);
  } catch {
    throw new Error("required DMG root entry is invalid");
  }
  if (!appMetadata.isDirectory() || appMetadata.isSymbolicLink()) {
    throw new Error("DMG App bundle is invalid");
  }
  let linkTarget;
  try {
    linkTarget = await readlink(applicationsLink);
  } catch {
    throw new Error("DMG Applications link is invalid");
  }
  if (!linkMetadata.isSymbolicLink() || linkTarget !== "/Applications") {
    throw new Error("DMG Applications link is invalid");
  }

  if (entries.includes(OPTIONAL_METADATA)) {
    let metadata;
    try {
      metadata = await lstat(path.join(mountDirectory, OPTIONAL_METADATA));
    } catch {
      throw new Error("DMG metadata entry is invalid");
    }
    if (
      !metadata.isFile() ||
      metadata.isSymbolicLink() ||
      metadata.size > MAX_TOOL_OUTPUT_BYTES
    ) {
      throw new Error("DMG metadata entry is invalid");
    }
  }
  let volumeIcon;
  try {
    volumeIcon = await lstat(path.join(mountDirectory, VOLUME_ICON));
  } catch {
    throw new Error("DMG volume icon is invalid");
  }
  if (
    !volumeIcon.isFile() ||
    volumeIcon.isSymbolicLink() ||
    volumeIcon.size === 0 ||
    volumeIcon.size > MAX_VOLUME_ICON_BYTES
  ) {
    throw new Error("DMG volume icon is invalid");
  }
  return appBundle;
}

async function resolveMacosAppExecutable(appBundle) {
  const infoPlist = path.join(appBundle, "Contents", "Info.plist");
  let metadata;
  try {
    metadata = await lstat(infoPlist);
  } catch {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > MAX_INFO_PLIST_BYTES
  ) {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
  let source;
  try {
    source = await readFile(infoPlist, "utf8");
  } catch {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
  if (Buffer.byteLength(source, "utf8") > MAX_INFO_PLIST_BYTES) {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
  const match = source.match(
    /<key>\s*CFBundleExecutable\s*<\/key>\s*<string>\s*([^<]+?)\s*<\/string>/,
  );
  const executable = match?.[1];
  if (
    typeof executable !== "string" ||
    executable.length === 0 ||
    executable.length > 128 ||
    path.basename(executable) !== executable ||
    !/^[A-Za-z0-9._-]+$/.test(executable)
  ) {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
  return executable;
}

async function validateSignedTarget(target) {
  let metadata;
  try {
    metadata = await lstat(target);
  } catch {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
  if (
    metadata.isSymbolicLink() ||
    (!metadata.isFile() && !metadata.isDirectory()) ||
    (metadata.isFile() && (metadata.size === 0 || (metadata.mode & 0o111) === 0))
  ) {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
}

function entitlementState(result) {
  if (!succeeded(result)) {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
  let output;
  try {
    output = boundedToolOutput(result);
  } catch {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
  const allKeys = [
    ...output.matchAll(/<key>\s*([^<]+?)\s*<\/key>/g),
  ].map((match) => match[1]);
  if ((output.match(/<key\b/g) ?? []).length !== allKeys.length) {
    throw new Error("macOS internal-test entitlement scope is invalid");
  }
  const escaped = LIBRARY_VALIDATION_ENTITLEMENT.replaceAll(".", "\\.");
  const key = new RegExp(`<key>\\s*${escaped}\\s*<\\/key>`, "g");
  const enabled = new RegExp(
    `<key>\\s*${escaped}\\s*<\\/key>\\s*<true\\s*\\/>`,
  );
  const keys = output.match(key) ?? [];
  return { allKeys, keyCount: keys.length, enabled: enabled.test(output) };
}

export async function verifyMacosInternalTestEntitlements({
  appBundle,
  platform = process.platform,
  runner = defaultRunner,
}) {
  if (platform !== "darwin" || !path.isAbsolute(appBundle)) {
    throw new Error("macOS internal-test entitlement arguments are invalid");
  }
  const mainExecutable = await resolveMacosAppExecutable(appBundle);
  const macosDirectory = path.join(appBundle, "Contents", "MacOS");
  const targets = [
    { target: appBundle, expected: false },
    { target: path.join(macosDirectory, mainExecutable), expected: false },
    { target: path.join(macosDirectory, "resume-daemon"), expected: false },
    {
      target: path.join(macosDirectory, "resume-embedding-runtime"),
      expected: true,
    },
    {
      target: path.join(macosDirectory, "resume-pdf-render-runtime"),
      expected: false,
    },
  ];
  for (const { target, expected } of targets) {
    await validateSignedTarget(target);
    let result;
    try {
      result = await runner(MACOS_SYSTEM_TOOLS.codesign, [
        "--display",
        "--entitlements",
        "-",
        "--xml",
        target,
      ]);
    } catch {
      throw new Error("macOS internal-test entitlement scope is invalid");
    }
    const state = entitlementState(result);
    if (
      (expected &&
        (state.keyCount !== 1 ||
          !state.enabled ||
          JSON.stringify(state.allKeys) !==
            JSON.stringify([LIBRARY_VALIDATION_ENTITLEMENT]))) ||
      (!expected && state.allKeys.length !== 0)
    ) {
      throw new Error("macOS internal-test entitlement scope is invalid");
    }
  }
  return Object.freeze({
    library_validation_entitlement_scope: "embedding_runtime_only",
  });
}

export async function verifyMacosInternalTestSignaturePolicy(options) {
  const signature = await verifyAdHocSignedApp(options);
  const entitlementScope = await verifyMacosInternalTestEntitlements(options);
  return Object.freeze({ ...signature, ...entitlementScope });
}

async function cleanupMount({
  attached,
  attachAttempted,
  mountDirectory,
  runner,
  mountProbe,
}) {
  let cleanupFailed = false;
  if (attached) {
    let result;
    try {
      result = await runner(MACOS_SYSTEM_TOOLS.hdiutil, [
        "detach",
        mountDirectory,
      ]);
    } catch {
      result = undefined;
    }
    if (!succeeded(result) && (await mountProbe(mountDirectory))) {
      try {
        result = await runner(MACOS_SYSTEM_TOOLS.hdiutil, [
          "detach",
          mountDirectory,
          "-force",
        ]);
      } catch {
        result = undefined;
      }
    }
    cleanupFailed =
      !succeeded(result) && (await mountProbe(mountDirectory));
  } else if (attachAttempted && (await mountProbe(mountDirectory))) {
    let result;
    try {
      result = await runner(MACOS_SYSTEM_TOOLS.hdiutil, [
        "detach",
        mountDirectory,
        "-force",
      ]);
    } catch {
      result = undefined;
    }
    cleanupFailed =
      !succeeded(result) && (await mountProbe(mountDirectory));
  }
  try {
    await rmdir(mountDirectory);
  } catch {
    cleanupFailed = true;
  }
  return cleanupFailed ? new Error("DMG detach or cleanup failed") : undefined;
}

async function consumeVerifiedMacosDmg({
  repoRoot,
  targetTriple,
  dmg,
  temporaryRoot = os.tmpdir(),
  maxDmgBytes = MAX_DMG_BYTES,
  platform = process.platform,
  runner = defaultRunner,
  verifyApp = verifyBundledSidecar,
  verifySource = verifyMainSourceProvenance,
  mountProbe = mountedFilesystemAt,
  consumeVerifiedImage,
}) {
  if (
    platform !== "darwin" ||
    targetTriple !== "aarch64-apple-darwin" ||
    !path.isAbsolute(repoRoot) ||
    !path.isAbsolute(dmg) ||
    !path.isAbsolute(temporaryRoot) ||
    typeof consumeVerifiedImage !== "function"
  ) {
    throw new Error("macOS DMG verification arguments are invalid");
  }
  const sourceCommit = await verifySource({ repoRoot });
  if (!/^[a-f0-9]{40}$/.test(sourceCommit ?? "")) {
    throw new Error("macOS build source provenance is invalid");
  }
  const initialDmgMetadata = await metadataForDmg(dmg, maxDmgBytes);
  const dmgSha256 = await digestFile(dmg);
  const hashedDmgMetadata = await metadataForDmg(dmg, maxDmgBytes, true);
  if (!sameDmgIdentity(initialDmgMetadata, hashedDmgMetadata)) {
    throw new Error("DMG file changed during verification");
  }

  let mountDirectory;
  try {
    mountDirectory = await mkdtemp(path.join(temporaryRoot, "resume-ir-dmg-"));
  } catch {
    throw new Error("DMG temporary mount is unavailable");
  }
  let attached = false;
  let attachAttempted = false;
  let consumerResult;
  let verificationError;
  try {
    attachAttempted = true;
    const attach = await runner(MACOS_SYSTEM_TOOLS.hdiutil, [
      "attach",
      dmg,
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mountDirectory,
    ]);
    if (!succeeded(attach)) throw new Error("DMG attach failed");
    attached = true;

    const attachedDmgMetadata = await metadataForDmg(dmg, maxDmgBytes, true);
    if (!sameDmgIdentity(hashedDmgMetadata, attachedDmgMetadata)) {
      throw new Error("DMG file changed during verification");
    }
    const attachedDmgSha256 = await digestFile(dmg);
    const finalDmgMetadata = await metadataForDmg(dmg, maxDmgBytes, true);
    if (
      !sameDmgIdentity(attachedDmgMetadata, finalDmgMetadata) ||
      attachedDmgSha256 !== dmgSha256
    ) {
      throw new Error("DMG file changed during verification");
    }

    const appBundle = await validateMountedDmgLayout({ mountDirectory });
    const volumeIcon = await lstat(path.join(mountDirectory, VOLUME_ICON));
    const appReceipt = await verifyApp({ repoRoot, targetTriple, appBundle });
    let signaturePolicy;
    const appComposition = await verifyBundleComposition({
      appBundle,
      targetTriple,
      expectedSourceCommit: sourceCommit,
      verifySignaturePolicy: async ({ appBundle: boundAppBundle }) => {
        signaturePolicy = await verifyMacosInternalTestSignaturePolicy({
          appBundle: boundAppBundle,
          platform,
          runner,
        });
        return signaturePolicy;
      },
    });
    const gatekeeper = await runner(MACOS_SYSTEM_TOOLS.spctl, [
      "-a",
      "-vv",
      "--type",
      "execute",
      appBundle,
    ]);
    const receipt = Object.freeze({
      schema_version: "resume-ir.macos-dmg-composition.v2",
      target_triple: targetTriple,
      source_commit: sourceCommit,
      dmg_count: 1,
      dmg_bytes: finalDmgMetadata.size,
      dmg_sha256: attachedDmgSha256,
      app_composition_digest: appComposition.composition_digest,
      mounted_read_only: true,
      app_bundle_count: 1,
      applications_link_count: 1,
      volume_icon_count: 1,
      volume_icon_bytes: volumeIcon.size,
      daemon_sidecar_count: appReceipt.daemon_sidecar_count,
      embedding_sidecar_count: appReceipt.embedding_sidecar_count,
      pdf_renderer_sidecar_count: appReceipt.pdf_renderer_sidecar_count,
      embedding_resource_file_count: appReceipt.embedding_resource_file_count,
      embedding_resource_bytes: appReceipt.embedding_resource_bytes,
      classifier_resource_file_count: appReceipt.classifier_resource_file_count,
      classifier_resource_bytes: appReceipt.classifier_resource_bytes,
      ocr_resource_file_count: appReceipt.ocr_resource_file_count,
      ocr_resource_bytes: appReceipt.ocr_resource_bytes,
      digest_match: appReceipt.digest_match,
      executable: appReceipt.executable,
      architecture: appReceipt.architecture,
      build_machine_identity_path_markers:
        appReceipt.build_machine_identity_path_markers,
      code_signature: signaturePolicy.code_signature,
      hardened_runtime: signaturePolicy.hardened_runtime,
      library_validation_entitlement_scope:
        signaturePolicy.library_validation_entitlement_scope,
      notarization: "not_requested",
      distribution_signature: "accepted",
      gatekeeper: succeeded(gatekeeper) ? "accepted" : "rejected",
      distribution_profile: "internal_test",
      tester_allow_list_required: true,
      release_claim: "composition_only",
    });
    consumerResult = await consumeVerifiedImage(
      Object.freeze({
        appBundle,
        appComposition: Object.freeze({ ...appComposition }),
        receipt,
      }),
    );
  } catch (error) {
    verificationError = error;
  }

  const cleanupError = await cleanupMount({
    attached,
    attachAttempted,
    mountDirectory,
    runner,
    mountProbe,
  });
  if (cleanupError) throw cleanupError;
  if (verificationError) throw verificationError;
  return consumerResult;
}

export async function withVerifiedMacosDmg({
  systemRunner = defaultRunner,
  ...options
}) {
  return consumeVerifiedMacosDmg({
    ...options,
    runner: systemRunner,
  });
}

export async function verifyMacosDmg({ runner = defaultRunner, ...options }) {
  return withVerifiedMacosDmg({
    ...options,
    systemRunner: runner,
    consumeVerifiedImage: ({ receipt }) => receipt,
  });
}

function parseArguments(args) {
  const values = new Map();
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (
      !["--target", "--dmg"].includes(key) ||
      !value ||
      values.has(key)
    ) {
      throw new Error("invalid DMG verification arguments");
    }
    values.set(key, value);
  }
  if (values.size !== 2) throw new Error("invalid DMG verification arguments");
  return {
    dmg: values.get("--dmg"),
    targetTriple: values.get("--target"),
  };
}

async function main() {
  const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
  const args = parseArguments(process.argv.slice(2));
  const receipt = await verifyMacosDmg({ repoRoot, ...args });
  console.log(JSON.stringify(receipt));
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    console.error(`verify-macos-dmg: ${error.message}`);
    process.exitCode = 1;
  });
}

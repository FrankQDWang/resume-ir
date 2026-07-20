import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import {
  chmod,
  lstat,
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
import { fileURLToPath } from "node:url";

import {
  APPLE_TOOL_TIMEOUT_MS,
  MAX_DMG_BYTES,
  validateMountedDmgLayout,
  verifyMacosInternalTestEntitlements,
  verifyMacosDmg,
  withVerifiedMacosDmg,
} from "./verify-macos-dmg.mjs";
import {
  applyMacosInternalTestEntitlements,
  buildMacosInternalTestRelease,
  createMacosInternalTestEnvironment,
  createMacosInternalTestPlan,
  resolveMacosTestReleasePaths,
  runSilentReleaseBuild,
  stageMountedDmg,
} from "./macos-test-release.mjs";
import { writeBundleComposition } from "./macos-bundle-composition.mjs";

const SOURCE_COMMIT = "0123456789abcdef0123456789abcdef01234567";
const verifySyntheticSource = async () => SOURCE_COMMIT;

function syntheticMachO(payload) {
  const suffix = Buffer.from(payload);
  const header = Buffer.alloc(32);
  header.writeUInt32LE(0xfeedfacf, 0);
  header.writeUInt32LE(0x0100000c, 4);
  return Buffer.concat([header, suffix]);
}

function sha256(body) {
  return createHash("sha256").update(body).digest("hex");
}

function isTool(command, name) {
  return path.basename(command) === name;
}

async function createMountedLayout(root, { withComposition = false } = {}) {
  const appBundle = path.join(root, "resume-ir.app");
  const macosDirectory = path.join(appBundle, "Contents", "MacOS");
  const resourcesDirectory = path.join(appBundle, "Contents", "Resources");
  await mkdir(macosDirectory, { recursive: true });
  for (const directory of [
    "classifier/runtime-pack",
    "embedding/runtime-pack",
    "ocr/runtime-pack",
  ]) {
    await mkdir(path.join(resourcesDirectory, directory), { recursive: true });
  }
  await writeFile(
    path.join(appBundle, "Contents", "Info.plist"),
    [
      '<?xml version="1.0" encoding="UTF-8"?>',
      '<plist version="1.0"><dict>',
      "<key>CFBundleIdentifier</key><string>local.resume-ir.desktop</string>",
      "<key>CFBundleShortVersionString</key><string>0.1.2</string>",
      "<key>CFBundleDisplayName</key><string>resume-ir</string>",
      "<key>CFBundleIconFile</key><string>icon.icns</string>",
      "<key>CFBundleExecutable</key><string>resume-desktop</string>",
      "</dict></plist>",
    ].join(""),
  );
  for (const executable of [
    "resume-desktop",
    "resume-daemon",
    "resume-embedding-runtime",
    "resume-pdf-render-runtime",
  ]) {
    const executablePath = path.join(macosDirectory, executable);
    await writeFile(executablePath, syntheticMachO(`synthetic-${executable}`));
    await chmod(executablePath, 0o755);
  }
  for (const pack of ["classifier", "embedding", "ocr"]) {
    const packDirectory = path.join(resourcesDirectory, pack, "runtime-pack");
    const payload = Buffer.from(`synthetic-${pack}-payload`);
    await writeFile(path.join(packDirectory, "payload.bin"), payload);
    await writeFile(
      path.join(packDirectory, "runtime-pack.json"),
      `${JSON.stringify({
        schema_version: `synthetic.${pack}.v1`,
        files: [
          {
            role: "payload",
            file: "payload.bin",
            bytes: payload.length,
            sha256: sha256(payload),
          },
        ],
      })}\n`,
    );
  }
  await writeFile(path.join(resourcesDirectory, "icon.icns"), "synthetic-icon");
  if (withComposition) {
    await writeBundleComposition({
      appBundle,
      targetTriple: "aarch64-apple-darwin",
      sourceCommit: SOURCE_COMMIT,
    });
  }
  await symlink("/Applications", path.join(root, "Applications"));
  await writeFile(path.join(root, ".DS_Store"), "synthetic");
  await writeFile(path.join(root, ".VolumeIcon.icns"), "synthetic-icon");
}

const LIBRARY_VALIDATION_ENTITLEMENT = [
  '<?xml version="1.0" encoding="UTF-8"?>',
  '<plist version="1.0"><dict>',
  "<key>com.apple.security.cs.disable-library-validation</key><true/>",
  "</dict></plist>",
].join("");
const SHIPPED_VOLUME_ICON_BYTES = 2_482_309;
const EXPECTED_MAX_VOLUME_ICON_BYTES = 8 * 1024 * 1024;

function entitlementRunner({
  leakTo = new Set(),
  omitFromEmbedding = false,
  extraEmbeddingEntitlement = false,
} = {}) {
  return async (command, args) => {
    if (path.basename(command) !== "codesign" || !args.includes("--entitlements")) {
      return { status: 0, stdout: "", stderr: "" };
    }
    const target = args.at(-1);
    const basename = path.basename(target);
    const hasEntitlement =
      (!omitFromEmbedding && basename === "resume-embedding-runtime") ||
      leakTo.has(basename) ||
      (target.endsWith(".app") && leakTo.has("resume-ir.app"));
    const entitlementSource = hasEntitlement
      ? extraEmbeddingEntitlement && basename === "resume-embedding-runtime"
        ? LIBRARY_VALIDATION_ENTITLEMENT.replace(
            "</dict>",
            "<key>com.apple.security.network.client</key><true/></dict>",
          )
        : LIBRARY_VALIDATION_ENTITLEMENT
      : "";
    return {
      status: 0,
      stdout: entitlementSource,
      stderr: `Executable=${target}`,
    };
  };
}

async function createTestReleaseFixture(context) {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-test-release-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const frontendRoot = path.join(root, "frontend");
  const repoRoot = path.join(root, "repo");
  const runTauri = path.join(root, "run-tauri.mjs");
  await Promise.all([
    mkdir(frontendRoot, { recursive: true }),
    mkdir(repoRoot, { recursive: true }),
    writeFile(runTauri, "synthetic"),
  ]);
  const baseConfig = { productName: "resume-ir", version: "0.1.0" };
  const platformConfig = {
    bundle: {
      targets: ["dmg"],
      macOS: { signingIdentity: "-", hardenedRuntime: true },
    },
  };
  const plan = createMacosInternalTestPlan({
    frontendRoot,
    platform: "darwin",
    baseConfig,
    platformConfig,
  });
  await mkdir(path.dirname(plan.dmg), { recursive: true });
  return {
    root,
    repoRoot,
    frontendRoot,
    runTauri,
    baseConfig,
    platformConfig,
    plan,
  };
}

function testReleaseCandidatePath(dmg) {
  const parsed = path.parse(dmg);
  return path.join(
    parsed.dir,
    `.${parsed.name}.internal-test-candidate${parsed.ext}`,
  );
}

function verifiedDmgReceipt() {
  return {
    schema_version: "resume-ir.macos-dmg-composition.v2",
    source_commit: SOURCE_COMMIT,
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
    digest_match: true,
    release_claim: "composition_only",
  };
}

test("accepts only the bounded Tauri drag-to-Applications layout", async (context) => {
  const mountDirectory = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-layout-"));
  context.after(() => rm(mountDirectory, { recursive: true, force: true }));
  await createMountedLayout(mountDirectory);

  assert.equal(
    await validateMountedDmgLayout({ mountDirectory }),
    path.join(mountDirectory, "resume-ir.app"),
  );
  await writeFile(path.join(mountDirectory, "unexpected-runtime"), "no");
  await assert.rejects(
    validateMountedDmgLayout({ mountDirectory }),
    /unexpected DMG root entry/,
  );
  await rm(path.join(mountDirectory, "unexpected-runtime"));
  await rm(path.join(mountDirectory, ".VolumeIcon.icns"));
  await symlink("/tmp", path.join(mountDirectory, ".VolumeIcon.icns"));
  await assert.rejects(
    validateMountedDmgLayout({ mountDirectory }),
    /DMG volume icon is invalid/,
  );
  await rm(path.join(mountDirectory, ".VolumeIcon.icns"));
  await writeFile(
    path.join(mountDirectory, ".VolumeIcon.icns"),
    Buffer.alloc(SHIPPED_VOLUME_ICON_BYTES),
  );
  assert.equal(
    await validateMountedDmgLayout({ mountDirectory }),
    path.join(mountDirectory, "resume-ir.app"),
  );
  await writeFile(
    path.join(mountDirectory, ".VolumeIcon.icns"),
    Buffer.alloc(EXPECTED_MAX_VOLUME_ICON_BYTES + 1),
  );
  await assert.rejects(
    validateMountedDmgLayout({ mountDirectory }),
    /DMG volume icon is invalid/,
  );
});

test("gives bounded Apple tools enough time for large signed bundles", () => {
  assert.ok(APPLE_TOOL_TIMEOUT_MS >= 60_000);
  assert.ok(APPLE_TOOL_TIMEOUT_MS <= 5 * 60_000);
});

test("rejects a wrong Applications link or symlinked App", async (context) => {
  const wrongLink = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-link-"));
  const symlinkedApp = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-app-"));
  context.after(() => rm(wrongLink, { recursive: true, force: true }));
  context.after(() => rm(symlinkedApp, { recursive: true, force: true }));

  await mkdir(path.join(wrongLink, "resume-ir.app"));
  await symlink("/tmp", path.join(wrongLink, "Applications"));
  await writeFile(path.join(wrongLink, ".VolumeIcon.icns"), "synthetic-icon");
  await assert.rejects(
    validateMountedDmgLayout({ mountDirectory: wrongLink }),
    /Applications link is invalid/,
  );

  await symlink("/Applications", path.join(symlinkedApp, "resume-ir.app"));
  await symlink("/Applications", path.join(symlinkedApp, "Applications"));
  await writeFile(path.join(symlinkedApp, ".VolumeIcon.icns"), "synthetic-icon");
  await assert.rejects(
    validateMountedDmgLayout({ mountDirectory: symlinkedApp }),
    /App bundle is invalid/,
  );
});

test("accepts library-validation bypass only on the embedding runtime", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-entitlements-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  await createMountedLayout(root);
  const appBundle = path.join(root, "resume-ir.app");

  assert.deepEqual(
    await verifyMacosInternalTestEntitlements({
      appBundle,
      platform: "darwin",
      runner: entitlementRunner(),
    }),
    { library_validation_entitlement_scope: "embedding_runtime_only" },
  );

  for (const runner of [
    entitlementRunner({ leakTo: new Set(["resume-daemon"]) }),
    entitlementRunner({ leakTo: new Set(["resume-desktop"]) }),
    entitlementRunner({ leakTo: new Set(["resume-ir.app"]) }),
    entitlementRunner({ leakTo: new Set(["resume-pdf-render-runtime"]) }),
    entitlementRunner({ omitFromEmbedding: true }),
    entitlementRunner({ extraEmbeddingEntitlement: true }),
  ]) {
    await assert.rejects(
      verifyMacosInternalTestEntitlements({
        appBundle,
        platform: "darwin",
        runner,
      }),
      /internal-test entitlement scope is invalid/,
    );
  }
});

test("invokes the entitlement trust tool by its absolute system path", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-entitlement-tool-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  await createMountedLayout(root);
  const commands = [];
  await verifyMacosInternalTestEntitlements({
    appBundle: path.join(root, "resume-ir.app"),
    platform: "darwin",
    runner: async (command, args) => {
      commands.push(command);
      return entitlementRunner()(command, args);
    },
  });
  assert.deepEqual(new Set(commands), new Set(["/usr/bin/codesign"]));
});

test("rebuilds the Tauri DMG from a clean staging root and signs only the embedding runtime with the entitlement", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-rewrite-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const dmg = path.join(root, "resume-ir.dmg");
  const entitlements = new URL(
    "../src-tauri/entitlements.internal-test.plist",
    import.meta.url,
  );
  await writeFile(dmg, "original-dmg");
  const calls = [];
  const runner = async (command, args) => {
    calls.push([command, ...args]);
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      await createMountedLayout(args[args.indexOf("-mountpoint") + 1]);
      await mkdir(
        path.join(args[args.indexOf("-mountpoint") + 1], ".fseventsd"),
      );
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "create") {
      const stagingDirectory = args[args.indexOf("-srcfolder") + 1];
      assert.equal((await readdir(stagingDirectory)).includes(".fseventsd"), false);
      await writeFile(args.at(-1), "rewritten-dmg");
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "codesign") && args.includes("--entitlements")) {
      const target = args.at(-1);
      return {
        status: 0,
        stdout: path.basename(target) === "resume-embedding-runtime"
          ? LIBRARY_VALIDATION_ENTITLEMENT
          : "",
        stderr: `Executable=${target}`,
      };
    }
    if (isTool(command, "codesign") && args.includes("--display")) {
      return {
        status: 0,
        stdout: "",
        stderr: [
          "CodeDirectory v=20400 size=120 flags=0x10002(adhoc,runtime)",
          "Signature=adhoc",
          "TeamIdentifier=not set",
          "Sealed Resources version=2 rules=13 files=42",
        ].join("\n"),
      };
    }
    return { status: 0, stdout: "", stderr: "" };
  };

  const entitlementReceipt = await applyMacosInternalTestEntitlements({
    dmg,
    entitlements: fileURLToPath(entitlements),
    sourceCommit: SOURCE_COMMIT,
    platform: "darwin",
    runner,
  });
  assert.equal(
    entitlementReceipt.library_validation_entitlement_scope,
    "embedding_runtime_only",
  );
  assert.match(entitlementReceipt.app_composition_digest, /^[a-f0-9]{64}$/);
  assert.equal(await readFile(dmg, "utf8"), "rewritten-dmg");

  const signingCalls = calls.filter(
    ([command, ...args]) =>
      isTool(command, "codesign") && args.includes("--force"),
  );
  assert.equal(signingCalls.length, 2);
  assert.equal(signingCalls[0].at(-1).endsWith("/resume-embedding-runtime"), true);
  assert.equal(signingCalls[0].includes("--entitlements"), true);
  assert.equal(signingCalls[1].at(-1).endsWith("/resume-ir.app"), true);
  assert.equal(signingCalls[1].includes("--entitlements"), false);
  assert.equal(signingCalls.some((call) => call.includes("--deep")), false);

  const attach = calls.find(
    ([command, action]) => isTool(command, "hdiutil") && action === "attach",
  );
  const create = calls.find(
    ([command, action]) => isTool(command, "hdiutil") && action === "create",
  );
  assert.equal(attach.includes("-readonly"), true);
  assert.equal(attach.includes("-shadow"), false);
  assert.equal(create.includes("-srcfolder"), true);
  assert.equal(create[create.indexOf("-fs") + 1], "HFS+");
});

test("staging stops before App copy when an earlier file copy fails", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-stage-order-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const mountDirectory = path.join(root, "mounted");
  const stagingDirectory = path.join(root, "staging");
  await mkdir(mountDirectory);
  await createMountedLayout(mountDirectory);
  const calls = [];

  await assert.rejects(
    stageMountedDmg({
      mountDirectory,
      appBundle: path.join(mountDirectory, "resume-ir.app"),
      stagingDirectory,
      operations: {
        copyFile: async (source) => {
          calls.push(`copy:${path.basename(source)}`);
          if (path.basename(source) === ".VolumeIcon.icns") {
            throw new Error("synthetic copy failure");
          }
        },
        cp: async () => {
          calls.push("app-copy");
        },
        symlink: async () => {
          calls.push("symlink");
        },
      },
    }),
    /DMG staging failed/,
  );
  assert.deepEqual(calls, ["copy:.DS_Store", "copy:.VolumeIcon.icns"]);
});

test("keeps the original DMG and cleans the staging workspace when re-signing fails", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-rewrite-fail-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const dmg = path.join(root, "resume-ir.dmg");
  const entitlements = fileURLToPath(
    new URL("../src-tauri/entitlements.internal-test.plist", import.meta.url),
  );
  await writeFile(dmg, "original-dmg");
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      await createMountedLayout(args[args.indexOf("-mountpoint") + 1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (
      isTool(command, "codesign") &&
      args.includes("--force") &&
      args.at(-1).endsWith("/resume-embedding-runtime")
    ) {
      return { status: 1, stdout: "", stderr: "/private/build/path" };
    }
    return { status: 0, stdout: "", stderr: "" };
  };

  await assert.rejects(
    applyMacosInternalTestEntitlements({
      dmg,
      entitlements,
      sourceCommit: SOURCE_COMMIT,
      platform: "darwin",
      runner,
    }),
    (error) => {
      assert.equal(error.message, "macOS internal-test entitlement signing failed");
      assert.equal(error.message.includes(root), false);
      return true;
    },
  );
  assert.equal(await readFile(dmg, "utf8"), "original-dmg");
  assert.deepEqual((await readdir(root)).sort(), ["resume-ir.dmg"]);
});

test("rejects a symlinked embedding signing path before codesign", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-rewrite-symlink-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const dmg = path.join(root, "resume-ir.dmg");
  const entitlements = fileURLToPath(
    new URL("../src-tauri/entitlements.internal-test.plist", import.meta.url),
  );
  const externalExecutable = path.join(root, "external-runtime");
  await writeFile(dmg, "original-dmg");
  await writeFile(externalExecutable, "external");
  await chmod(externalExecutable, 0o755);
  let signingCalls = 0;
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      const mountDirectory = args[args.indexOf("-mountpoint") + 1];
      await createMountedLayout(mountDirectory);
      const runtime = path.join(
        mountDirectory,
        "resume-ir.app",
        "Contents",
        "MacOS",
        "resume-embedding-runtime",
      );
      await rm(runtime);
      await symlink(externalExecutable, runtime);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "codesign")) signingCalls += 1;
    return { status: 0, stdout: "", stderr: "" };
  };

  await assert.rejects(
    applyMacosInternalTestEntitlements({
      dmg,
      entitlements,
      sourceCommit: SOURCE_COMMIT,
      platform: "darwin",
      runner,
    }),
    /native signing path is invalid/,
  );
  assert.equal(signingCalls, 0);
  assert.equal(await readFile(dmg, "utf8"), "original-dmg");
});

test("rejects a symlinked daemon before the outer App codesign", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-daemon-symlink-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const dmg = path.join(root, "resume-ir.dmg");
  const entitlements = fileURLToPath(
    new URL("../src-tauri/entitlements.internal-test.plist", import.meta.url),
  );
  const externalExecutable = path.join(root, "external-daemon");
  await writeFile(dmg, "original-dmg");
  await writeFile(externalExecutable, "external");
  await chmod(externalExecutable, 0o755);
  let signingCalls = 0;
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      const mountDirectory = args[args.indexOf("-mountpoint") + 1];
      await createMountedLayout(mountDirectory);
      const daemon = path.join(
        mountDirectory,
        "resume-ir.app",
        "Contents",
        "MacOS",
        "resume-daemon",
      );
      await rm(daemon);
      await symlink(externalExecutable, daemon);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "codesign")) signingCalls += 1;
    return { status: 0, stdout: "", stderr: "" };
  };

  await assert.rejects(
    applyMacosInternalTestEntitlements({
      dmg,
      entitlements,
      sourceCommit: SOURCE_COMMIT,
      platform: "darwin",
      runner,
    }),
    /native signing path is invalid/,
  );
  assert.equal(signingCalls, 0);
  assert.equal(await readFile(dmg, "utf8"), "original-dmg");
});

test("rejects a symlinked .fseventsd entry instead of deleting through it", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-fseventsd-link-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const dmg = path.join(root, "resume-ir.dmg");
  const entitlements = fileURLToPath(
    new URL("../src-tauri/entitlements.internal-test.plist", import.meta.url),
  );
  await writeFile(dmg, "original-dmg");
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      const mountDirectory = args[args.indexOf("-mountpoint") + 1];
      await createMountedLayout(mountDirectory);
      await symlink(root, path.join(mountDirectory, ".fseventsd"));
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    return { status: 0, stdout: "", stderr: "" };
  };

  await assert.rejects(
    applyMacosInternalTestEntitlements({
      dmg,
      entitlements,
      sourceCommit: SOURCE_COMMIT,
      platform: "darwin",
      runner,
    }),
    /transient metadata is invalid/,
  );
  assert.equal(await readFile(dmg, "utf8"), "original-dmg");
  assert.equal((await lstat(root)).isDirectory(), true);
});

test("fails closed after bounded detach recovery during DMG rewriting", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-rewrite-detach-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const dmg = path.join(root, "resume-ir.dmg");
  const entitlements = fileURLToPath(
    new URL("../src-tauri/entitlements.internal-test.plist", import.meta.url),
  );
  await writeFile(dmg, "original-dmg");
  const detachCalls = [];
  let mounted = true;
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      await createMountedLayout(args[args.indexOf("-mountpoint") + 1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      detachCalls.push(args);
      if (args.includes("-force")) {
        mounted = false;
        await rm(args[1], { recursive: true, force: true });
        await mkdir(args[1]);
      }
      return {
        status: args.includes("-force") ? 0 : 1,
        stdout: "",
        stderr: args.includes("-force") ? "" : "/private/mount/path",
      };
    }
    if (isTool(command, "codesign") && args.includes("--entitlements")) {
      const target = args.at(-1);
      return {
        status: 0,
        stdout: path.basename(target) === "resume-embedding-runtime"
          ? LIBRARY_VALIDATION_ENTITLEMENT
          : "",
        stderr: `Executable=${target}`,
      };
    }
    return { status: 0, stdout: "", stderr: "" };
  };

  await assert.rejects(
    applyMacosInternalTestEntitlements({
      dmg,
      entitlements,
      sourceCommit: SOURCE_COMMIT,
      platform: "darwin",
      runner,
      mountProbe: async () => mounted,
    }),
    /DMG detach or cleanup failed/,
  );
  assert.equal(detachCalls.length, 2);
  assert.equal(detachCalls[1].includes("-force"), true);
  assert.equal(await readFile(dmg, "utf8"), "original-dmg");
  assert.deepEqual((await readdir(root)).sort(), ["resume-ir.dmg"]);
});

test("detaches a partial mount after attach times out and preserves the attach error", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-attach-timeout-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const dmg = path.join(root, "resume-ir.dmg");
  const entitlements = fileURLToPath(
    new URL("../src-tauri/entitlements.internal-test.plist", import.meta.url),
  );
  await writeFile(dmg, "original-dmg");
  let detachCalls = 0;
  const detachArguments = [];
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      await createMountedLayout(args[args.indexOf("-mountpoint") + 1]);
      return {
        status: null,
        error: { code: "ETIMEDOUT" },
        stdout: "",
        stderr: "/private/mount/path",
      };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      detachCalls += 1;
      detachArguments.push(args);
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    return { status: 0, stdout: "", stderr: "" };
  };

  await assert.rejects(
    applyMacosInternalTestEntitlements({
      dmg,
      entitlements,
      sourceCommit: SOURCE_COMMIT,
      platform: "darwin",
      runner,
      mountProbe: async () => true,
    }),
    (error) => {
      assert.equal(error.message, "macOS internal-test DMG attach failed");
      assert.equal(error.message.includes(root), false);
      return true;
    },
  );
  assert.equal(detachCalls, 1);
  assert.equal(detachArguments[0].includes("-force"), true);
  assert.equal(await readFile(dmg, "utf8"), "original-dmg");
  assert.deepEqual((await readdir(root)).sort(), ["resume-ir.dmg"]);
});

test("verifies one DMG across a read-only attach ctime change and always detaches", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-verify-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");
  const initialCtimeMs = (await lstat(dmg)).ctimeMs;
  const calls = [];
  const runner = async (command, args) => {
    calls.push([command, ...args]);
    const tool = path.basename(command);
    if (tool === "hdiutil" && args[0] === "attach") {
      await chmod(dmg, 0o600);
      await chmod(dmg, 0o644);
      const mountDirectory = args.at(-1);
      await createMountedLayout(mountDirectory, { withComposition: true });
      return { status: 0, stdout: "", stderr: "" };
    }
    if (tool === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (tool === "codesign" && args.includes("--entitlements")) {
      const target = args.at(-1);
      return {
        status: 0,
        stdout: path.basename(target) === "resume-embedding-runtime"
          ? LIBRARY_VALIDATION_ENTITLEMENT
          : "",
        stderr: `Executable=${target}`,
      };
    }
    if (tool === "codesign" && args.includes("--display")) {
      return {
        status: 0,
        stdout: "",
        stderr: [
          "CodeDirectory v=20400 size=120 flags=0x10002(adhoc,runtime)",
          "Signature=adhoc",
          "TeamIdentifier=not set",
          "Sealed Resources version=2 rules=13 files=42",
        ].join("\n"),
      };
    }
    if (tool === "codesign") {
      return { status: 0, stdout: "", stderr: "" };
    }
    return { status: 1, stdout: "", stderr: "not accepted" };
  };
  const verifiedApps = [];
  let leaseConsumed = false;
  const receipt = await withVerifiedMacosDmg({
    repoRoot: root,
    targetTriple: "aarch64-apple-darwin",
    dmg,
    temporaryRoot,
    platform: "darwin",
    systemRunner: runner,
    verifySource: verifySyntheticSource,
    verifyApp: async ({ appBundle }) => {
      verifiedApps.push(appBundle);
      return {
        daemon_sidecar_count: 1,
        embedding_sidecar_count: 1,
        pdf_renderer_sidecar_count: 1,
        embedding_resource_file_count: 7,
        embedding_resource_bytes: 10,
        classifier_resource_file_count: 2,
        classifier_resource_bytes: 15,
        ocr_resource_file_count: 31,
        ocr_resource_bytes: 20,
        digest_match: true,
        executable: true,
        architecture: "arm64",
        build_machine_identity_path_markers: 0,
      };
    },
    consumeVerifiedImage: async ({
      appBundle,
      appComposition,
      receipt: verifiedReceipt,
    }) => {
      leaseConsumed = true;
      assert.equal(await lstat(appBundle).then((metadata) => metadata.isDirectory()), true);
      assert.equal(
        appComposition.composition_digest,
        verifiedReceipt.app_composition_digest,
      );
      return verifiedReceipt;
    },
  });

  assert.equal(verifiedApps.length, 1);
  assert.equal(leaseConsumed, true);
  assert.notEqual((await lstat(dmg)).ctimeMs, initialCtimeMs);
  assert.deepEqual(calls[0].slice(0, 6), [
    "/usr/bin/hdiutil",
    "attach",
    dmg,
    "-readonly",
    "-nobrowse",
    "-mountpoint",
  ]);
  assert.equal(calls.at(-1)[1], "detach");
  assert.deepEqual(await readdir(temporaryRoot), []);
  assert.match(receipt.app_composition_digest, /^[a-f0-9]{64}$/);
  assert.deepEqual(receipt, {
    schema_version: "resume-ir.macos-dmg-composition.v2",
    target_triple: "aarch64-apple-darwin",
    source_commit: SOURCE_COMMIT,
    dmg_count: 1,
    dmg_bytes: 13,
    dmg_sha256: "bf55618abcf4f76365c1784a970a08f5c650c6fc3ad8a613a042601c9a688b61",
    app_composition_digest: receipt.app_composition_digest,
    mounted_read_only: true,
    app_bundle_count: 1,
    applications_link_count: 1,
    volume_icon_count: 1,
    volume_icon_bytes: 14,
    daemon_sidecar_count: 1,
    embedding_sidecar_count: 1,
    pdf_renderer_sidecar_count: 1,
    embedding_resource_file_count: 7,
    embedding_resource_bytes: 10,
    classifier_resource_file_count: 2,
    classifier_resource_bytes: 15,
    ocr_resource_file_count: 31,
    ocr_resource_bytes: 20,
    digest_match: true,
    executable: true,
    architecture: "arm64",
    build_machine_identity_path_markers: 0,
    code_signature: "ad_hoc_valid",
    hardened_runtime: true,
    library_validation_entitlement_scope: "embedding_runtime_only",
    notarization: "not_requested",
    distribution_signature: "accepted",
    gatekeeper: "rejected",
    distribution_profile: "internal_test",
    tester_allow_list_required: true,
    release_claim: "composition_only",
  });
});

test("rejects a DMG pathname replacement between digest and attach", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-swap-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  const replacement = path.join(root, "replacement.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "verified-dmg-bytes");
  await writeFile(replacement, "replacement-dmg-bytes");
  let replaced = false;
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      await rename(replacement, dmg);
      replaced = true;
      await createMountedLayout(args.at(-1), { withComposition: true });
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "codesign") && args.includes("--entitlements")) {
      return entitlementRunner()(command, args);
    }
    if (isTool(command, "codesign") && args.includes("--display")) {
      return {
        status: 0,
        stdout: "",
        stderr: [
          "CodeDirectory v=20400 size=120 flags=0x10002(adhoc,runtime)",
          "Signature=adhoc",
          "TeamIdentifier=not set",
          "Sealed Resources version=2 rules=13 files=42",
        ].join("\n"),
      };
    }
    if (isTool(command, "codesign")) {
      return { status: 0, stdout: "", stderr: "" };
    }
    return { status: 1, stdout: "", stderr: "expected Gatekeeper rejection" };
  };

  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      targetTriple: "aarch64-apple-darwin",
      dmg,
      temporaryRoot,
      platform: "darwin",
      runner,
      verifySource: verifySyntheticSource,
      verifyApp: async () => ({
        daemon_sidecar_count: 1,
        embedding_sidecar_count: 1,
        pdf_renderer_sidecar_count: 1,
        embedding_resource_file_count: 1,
        embedding_resource_bytes: 1,
        classifier_resource_file_count: 1,
        classifier_resource_bytes: 1,
        ocr_resource_file_count: 1,
        ocr_resource_bytes: 1,
        digest_match: true,
        executable: true,
        architecture: "arm64",
        build_machine_identity_path_markers: 0,
      }),
    }),
    /DMG file changed during verification/,
  );
  assert.equal(replaced, true);
  assert.deepEqual(await readdir(temporaryRoot), []);
});

test("rejects an unsealed or non-ad-hoc App instead of issuing a test receipt", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-signature-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");

  for (const signatureCase of ["verify_failed", "developer_identity"]) {
    const runner = async (command, args) => {
      if (isTool(command, "hdiutil") && args[0] === "attach") {
        await createMountedLayout(args.at(-1), { withComposition: true });
        return { status: 0, stdout: "", stderr: "" };
      }
      if (isTool(command, "hdiutil") && args[0] === "detach") {
        await rm(args[1], { recursive: true, force: true });
        await mkdir(args[1]);
        return { status: 0, stdout: "", stderr: "" };
      }
      if (isTool(command, "codesign") && args.includes("--verify")) {
        return { status: signatureCase === "verify_failed" ? 1 : 0, stdout: "", stderr: "" };
      }
      if (isTool(command, "codesign") && args.includes("--display")) {
        return {
          status: 0,
          stdout: "",
          stderr: [
            "CodeDirectory v=20400 size=120 flags=0x10000(runtime)",
            "Authority=Developer ID Application: Synthetic Test",
            "TeamIdentifier=ABCDEFGHIJ",
            "Sealed Resources version=2 rules=13 files=42",
          ].join("\n"),
        };
      }
      return { status: 1, stdout: "", stderr: "" };
    };
    await assert.rejects(
      verifyMacosDmg({
        repoRoot: root,
        verifySource: verifySyntheticSource,
        targetTriple: "aarch64-apple-darwin",
        dmg,
        temporaryRoot,
        platform: "darwin",
        runner,
        verifyApp: async () => ({
          daemon_sidecar_count: 1,
          embedding_sidecar_count: 1,
          pdf_renderer_sidecar_count: 1,
          embedding_resource_file_count: 7,
          embedding_resource_bytes: 10,
          classifier_resource_file_count: 2,
          classifier_resource_bytes: 15,
          ocr_resource_file_count: 31,
          ocr_resource_bytes: 20,
          digest_match: true,
          executable: true,
          architecture: "arm64",
          build_machine_identity_path_markers: 0,
        }),
      }),
      /bundle signature policy does not match/,
    );
  }
});

test("rejects a nested executable whose signature policy differs from the App", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-nested-signature-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");
  const validMetadata = [
    "CodeDirectory v=20400 size=120 flags=0x10002(adhoc,runtime)",
    "Signature=adhoc",
    "TeamIdentifier=not set",
    "Sealed Resources version=2 rules=13 files=42",
  ].join("\n");
  const invalidNestedMetadata = [
    "CodeDirectory v=20400 size=120 flags=0x10000(runtime)",
    "Authority=Developer ID Application: Synthetic Test",
    "TeamIdentifier=ABCDEFGHIJ",
  ].join("\n");
  const runner = async (command, args) => {
    const tool = path.basename(command);
    if (tool === "hdiutil" && args[0] === "attach") {
      await createMountedLayout(args.at(-1), { withComposition: true });
      return { status: 0, stdout: "", stderr: "" };
    }
    if (tool === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (tool === "codesign" && args.includes("--entitlements")) {
      const target = args.at(-1);
      return {
        status: 0,
        stdout: path.basename(target) === "resume-embedding-runtime"
          ? LIBRARY_VALIDATION_ENTITLEMENT
          : "",
        stderr: `Executable=${target}`,
      };
    }
    if (tool === "codesign" && args.includes("--display")) {
      return {
        status: 0,
        stdout: "",
        stderr: path.basename(args.at(-1)) === "resume-daemon"
          ? invalidNestedMetadata
          : validMetadata,
      };
    }
    if (tool === "codesign") return { status: 0, stdout: "", stderr: "" };
    return { status: 1, stdout: "", stderr: "expected Gatekeeper rejection" };
  };
  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      verifySource: verifySyntheticSource,
      targetTriple: "aarch64-apple-darwin",
      dmg,
      temporaryRoot,
      platform: "darwin",
      runner,
      verifyApp: async () => ({
        daemon_sidecar_count: 1,
        embedding_sidecar_count: 1,
        pdf_renderer_sidecar_count: 1,
        embedding_resource_file_count: 7,
        embedding_resource_bytes: 10,
        classifier_resource_file_count: 2,
        classifier_resource_bytes: 15,
        ocr_resource_file_count: 31,
        ocr_resource_bytes: 20,
        digest_match: true,
        executable: true,
        architecture: "arm64",
        build_machine_identity_path_markers: 0,
      }),
    }),
    /bundle signature policy does not match/,
  );
});

test("rejects irregular, empty, or oversized images and cleans an attach failure", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-invalid-"));
  const temporaryRoot = path.join(root, "mounts");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  const directory = path.join(root, "directory.dmg");
  await mkdir(directory);
  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      verifySource: verifySyntheticSource,
      targetTriple: "aarch64-apple-darwin",
      dmg: directory,
      temporaryRoot,
      platform: "darwin",
    }),
    /DMG file is invalid/,
  );
  const empty = path.join(root, "empty.dmg");
  await writeFile(empty, "");
  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      verifySource: verifySyntheticSource,
      targetTriple: "aarch64-apple-darwin",
      dmg: empty,
      temporaryRoot,
      platform: "darwin",
    }),
    /DMG file is invalid/,
  );

  const oversized = path.join(root, "oversized.dmg");
  await writeFile(oversized, "x");
  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      verifySource: verifySyntheticSource,
      targetTriple: "aarch64-apple-darwin",
      dmg: oversized,
      temporaryRoot,
      platform: "darwin",
      maxDmgBytes: 0,
    }),
    /DMG file is invalid/,
  );
  assert.ok(MAX_DMG_BYTES > 0);

  const image = path.join(root, "attach-fails.dmg");
  await writeFile(image, "x");
  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      verifySource: verifySyntheticSource,
      targetTriple: "aarch64-apple-darwin",
      dmg: image,
      temporaryRoot,
      platform: "darwin",
      runner: async () => ({ status: 1, stdout: "", stderr: "private path" }),
    }),
    /DMG attach failed/,
  );
  assert.deepEqual(await readdir(temporaryRoot), []);
});

test("the verifier detaches a partial mount after attach times out", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-verify-timeout-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");
  let detachCalls = 0;
  const detachArguments = [];
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      await createMountedLayout(args.at(-1), { withComposition: true });
      return {
        status: null,
        error: { code: "ETIMEDOUT" },
        stdout: "",
        stderr: "/private/mount/path",
      };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      detachCalls += 1;
      detachArguments.push(args);
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    return { status: 1, stdout: "", stderr: "" };
  };

  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      verifySource: verifySyntheticSource,
      targetTriple: "aarch64-apple-darwin",
      dmg,
      temporaryRoot,
      platform: "darwin",
      runner,
      mountProbe: async () => true,
    }),
    /DMG attach failed/,
  );
  assert.equal(detachCalls, 1);
  assert.equal(detachArguments[0].includes("-force"), true);
  assert.deepEqual(await readdir(temporaryRoot), []);
});

test("a stuck partial verifier mount gets one forced detach attempt", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-partial-stuck-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");
  const detachArguments = [];
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      await writeFile(path.join(args.at(-1), "partial-mount"), "mounted");
      return {
        status: null,
        error: { code: "ETIMEDOUT" },
        stdout: "",
        stderr: "bounded",
      };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      detachArguments.push(args);
      return { status: 1, stdout: "", stderr: "bounded" };
    }
    return { status: 1, stdout: "", stderr: "" };
  };

  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      verifySource: verifySyntheticSource,
      targetTriple: "aarch64-apple-darwin",
      dmg,
      temporaryRoot,
      platform: "darwin",
      runner,
      mountProbe: async () => true,
    }),
    /DMG detach or cleanup failed/,
  );
  assert.equal(detachArguments.length, 1);
  assert.equal(detachArguments[0].includes("-force"), true);
});

test("preserves the verification error when detach reports failure after unmounting", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-detach-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      await createMountedLayout(args.at(-1), { withComposition: true });
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      if (args.includes("-force")) {
        await rm(args[1], { recursive: true, force: true });
        await mkdir(args[1]);
      } else {
        await rm(args[1], { recursive: true, force: true });
        await mkdir(args[1]);
      }
      return { status: 1, stdout: "", stderr: "" };
    }
    return { status: 1, stdout: "", stderr: "" };
  };
  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      verifySource: verifySyntheticSource,
      targetTriple: "aarch64-apple-darwin",
      dmg,
      temporaryRoot,
      platform: "darwin",
      runner,
      verifyApp: async () => ({}),
    }),
    /bundle signature policy does not match/,
  );
  assert.deepEqual(await readdir(temporaryRoot), []);
});

test("reports cleanup failure when a verifier mount remains after detach recovery", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-detach-stuck-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");
  const runner = async (command, args) => {
    if (isTool(command, "hdiutil") && args[0] === "attach") {
      await createMountedLayout(args.at(-1));
      return { status: 0, stdout: "", stderr: "" };
    }
    if (isTool(command, "hdiutil") && args[0] === "detach") {
      return { status: 1, stdout: "", stderr: "" };
    }
    return { status: 1, stdout: "", stderr: "" };
  };

  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
      verifySource: verifySyntheticSource,
      targetTriple: "aarch64-apple-darwin",
      dmg,
      temporaryRoot,
      platform: "darwin",
      runner,
      verifyApp: async () => ({}),
      mountProbe: async () => true,
    }),
    /DMG detach or cleanup failed/,
  );
});

test("locked macOS platform config selects only the DMG installer", async () => {
  const config = JSON.parse(
    await readFile(
      new URL("../src-tauri/tauri.macos.conf.json", import.meta.url),
      "utf8",
    ),
  );
  assert.equal(config.$schema, "https://schema.tauri.app/config/2");
  assert.deepEqual(config.bundle.targets, ["dmg"]);
  assert.deepEqual(config.bundle.icon, ["icons/icon.icns"]);
  assert.deepEqual(config.bundle.macOS, {
    signingIdentity: "-",
    hardenedRuntime: true,
  });
});

test("locks one credential-free arm64 internal-test build", async () => {
  const paths = resolveMacosTestReleasePaths();
  const baseConfig = JSON.parse(await readFile(paths.baseConfig, "utf8"));
  const platformConfig = JSON.parse(
    await readFile(paths.platformConfig, "utf8"),
  );
  const plan = createMacosInternalTestPlan({
    frontendRoot: paths.frontendRoot,
    platform: "darwin",
    baseConfig,
    platformConfig,
  });
  assert.deepEqual(plan.tauriArguments, [
    "build",
    "--target",
    "aarch64-apple-darwin",
    "--bundles",
    "dmg",
    "--ci",
  ]);
  assert.equal(path.basename(plan.dmg), "resume-ir_0.1.2_aarch64.dmg");
  assert.equal(
    plan.entitlements,
    path.join(paths.frontendRoot, "src-tauri", "entitlements.internal-test.plist"),
  );
  assert.deepEqual(
    createMacosInternalTestEnvironment({
      KEEP_ME: "yes",
      APPLE_ID: "removed",
      APPLE_API_KEY_PATH: "removed",
      APPLE_CERTIFICATE: "removed",
    }),
    { KEEP_ME: "yes", APPLE_SIGNING_IDENTITY: "-" },
  );

  for (const candidate of [
    { ...platformConfig, bundle: { ...platformConfig.bundle, macOS: {} } },
    {
      ...platformConfig,
      bundle: {
        ...platformConfig.bundle,
        macOS: { signingIdentity: "-", hardenedRuntime: false },
      },
    },
  ]) {
    assert.throws(
      () =>
        createMacosInternalTestPlan({
          frontendRoot: paths.frontendRoot,
          platform: "darwin",
          baseConfig,
          platformConfig: candidate,
        }),
      /config is invalid/,
    );
  }
});

test("isolates child build output from the machine-readable release receipt", () => {
  const result = runSilentReleaseBuild(
    process.execPath,
    [
      "-e",
      'process.stdout.write("build stdout\\n"); process.stderr.write("build stderr\\n")',
    ],
    { cwd: os.tmpdir(), env: process.env },
  );

  assert.equal(result.status, 0);
  assert.equal(result.stdout, null);
  assert.equal(result.stderr, null);
});

test("removes a stale canonical DMG when the Tauri build fails", async (context) => {
  const fixture = await createTestReleaseFixture(context);
  await writeFile(fixture.plan.dmg, "stale-unverified-dmg");

  await assert.rejects(
    buildMacosInternalTestRelease({
      ...fixture,
      verifySource: verifySyntheticSource,
      platform: "darwin",
      runner: () => ({ status: 1, stderr: "/private/build/path" }),
    }),
    /internal-test build failed/,
  );
  await assert.rejects(lstat(fixture.plan.dmg), { code: "ENOENT" });
});

test("removes a stale candidate when prebuild source provenance fails", async (context) => {
  const fixture = await createTestReleaseFixture(context);
  const candidate = testReleaseCandidatePath(fixture.plan.dmg);
  await writeFile(candidate, "stale-candidate");
  await writeFile(fixture.plan.dmg, "previous-verified-dmg");
  let buildCalled = false;

  await assert.rejects(
    buildMacosInternalTestRelease({
      ...fixture,
      verifySource: async () => {
        throw new Error("macOS build source provenance is invalid");
      },
      platform: "darwin",
      runner: () => {
        buildCalled = true;
        return { status: 0 };
      },
    }),
    /source provenance is invalid/,
  );
  assert.equal(buildCalled, false);
  await assert.rejects(lstat(candidate), { code: "ENOENT" });
  assert.equal(await readFile(fixture.plan.dmg, "utf8"), "previous-verified-dmg");
});

test("deletes the candidate when source provenance drifts before promotion", async (context) => {
  const fixture = await createTestReleaseFixture(context);
  let provenanceChecks = 0;

  await assert.rejects(
    buildMacosInternalTestRelease({
      ...fixture,
      verifySource: async () => {
        provenanceChecks += 1;
        return provenanceChecks === 1 ? SOURCE_COMMIT : "f".repeat(40);
      },
      platform: "darwin",
      runner: () => {
        writeFileSync(fixture.plan.dmg, "fresh-tauri-dmg");
        return { status: 0 };
      },
      applyEntitlements: async () => ({
        library_validation_entitlement_scope: "embedding_runtime_only",
      }),
      verifyDmg: async () => verifiedDmgReceipt(),
    }),
    /source provenance is invalid/,
  );
  assert.equal(provenanceChecks, 2);
  await assert.rejects(lstat(fixture.plan.dmg), { code: "ENOENT" });
  await assert.rejects(lstat(testReleaseCandidatePath(fixture.plan.dmg)), {
    code: "ENOENT",
  });
});

test("does not leave a canonical DMG when full verification fails", async (context) => {
  const fixture = await createTestReleaseFixture(context);

  await assert.rejects(
    buildMacosInternalTestRelease({
      ...fixture,
      verifySource: verifySyntheticSource,
      platform: "darwin",
      runner: () => {
        writeFileSync(fixture.plan.dmg, "fresh-tauri-dmg");
        return { status: 0 };
      },
      applyEntitlements: async () => ({
        library_validation_entitlement_scope: "embedding_runtime_only",
      }),
      verifyDmg: async () => {
        throw new Error("synthetic verification failure");
      },
    }),
    /synthetic verification failure/,
  );
  await assert.rejects(lstat(fixture.plan.dmg), { code: "ENOENT" });
});

test("promotes only the fully verified candidate to the canonical DMG path", async (context) => {
  const fixture = await createTestReleaseFixture(context);
  const receipt = verifiedDmgReceipt();
  let entitlementCandidate;
  let verifiedCandidate;

  assert.deepEqual(
    await buildMacosInternalTestRelease({
      ...fixture,
      verifySource: verifySyntheticSource,
      platform: "darwin",
      runner: () => {
        mkdirSync(path.dirname(fixture.plan.dmg), { recursive: true });
        writeFileSync(fixture.plan.dmg, "fresh-tauri-dmg");
        return { status: 0 };
      },
      applyEntitlements: async ({ dmg }) => {
        entitlementCandidate = dmg;
        return {
          library_validation_entitlement_scope: "embedding_runtime_only",
        };
      },
      verifyDmg: async ({ dmg }) => {
        verifiedCandidate = dmg;
        return receipt;
      },
    }),
    receipt,
  );
  assert.notEqual(entitlementCandidate, fixture.plan.dmg);
  assert.equal(verifiedCandidate, entitlementCandidate);
  assert.equal(await readFile(fixture.plan.dmg, "utf8"), "fresh-tauri-dmg");
  assert.deepEqual(await readdir(path.dirname(fixture.plan.dmg)), [
    path.basename(fixture.plan.dmg),
  ]);
});

test("test-release wrapper returns only a verified bounded receipt", async (context) => {
  const fixture = await createTestReleaseFixture(context);
  const receipt = verifiedDmgReceipt();
  const calls = [];
  const entitlementCalls = [];
  const verificationCalls = [];
  assert.deepEqual(
    await buildMacosInternalTestRelease({
      ...fixture,
      verifySource: verifySyntheticSource,
      platform: "darwin",
      environment: { APPLE_PASSWORD: "removed", KEEP_ME: "yes" },
      runner: (command, args, options) => {
        calls.push({ command, args, options });
        writeFileSync(fixture.plan.dmg, "fresh-tauri-dmg");
        return { status: 0 };
      },
      applyEntitlements: async (request) => {
        entitlementCalls.push(request);
        return {
          library_validation_entitlement_scope: "embedding_runtime_only",
        };
      },
      verifyDmg: async (request) => {
        verificationCalls.push(request);
        return receipt;
      },
    }),
    receipt,
  );
  assert.equal(calls.length, 1);
  assert.equal(calls[0].args[0], fixture.runTauri);
  assert.equal(calls[0].options.env.APPLE_PASSWORD, undefined);
  assert.equal(calls[0].options.env.APPLE_SIGNING_IDENTITY, "-");
  assert.equal(entitlementCalls.length, 1);
  assert.notEqual(entitlementCalls[0].dmg, fixture.plan.dmg);
  assert.equal(verificationCalls.length, 1);
  assert.equal(Object.hasOwn(verificationCalls[0], "runner"), false);
  assert.equal(Object.hasOwn(verificationCalls[0], "systemRunner"), false);

  await assert.rejects(
    buildMacosInternalTestRelease({
      ...fixture,
      verifySource: verifySyntheticSource,
      platform: "darwin",
      runner: () => ({ status: 1, stderr: "/private/build/path" }),
    }),
    (error) => {
      assert.equal(error.message, "macOS internal-test build failed");
      assert.equal(error.message.includes(fixture.repoRoot), false);
      return true;
    },
  );
});

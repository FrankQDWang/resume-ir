import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
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
} from "./verify-macos-dmg.mjs";
import {
  applyMacosInternalTestEntitlements,
  buildMacosInternalTestRelease,
  createMacosInternalTestEnvironment,
  createMacosInternalTestPlan,
  resolveMacosTestReleasePaths,
  stageMountedDmg,
} from "./macos-test-release.mjs";

async function createMountedLayout(root) {
  const appBundle = path.join(root, "resume-ir.app");
  const macosDirectory = path.join(appBundle, "Contents", "MacOS");
  await mkdir(macosDirectory, { recursive: true });
  await writeFile(
    path.join(appBundle, "Contents", "Info.plist"),
    [
      '<?xml version="1.0" encoding="UTF-8"?>',
      '<plist version="1.0"><dict>',
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
    await writeFile(executablePath, "synthetic-executable");
    await chmod(executablePath, 0o755);
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

function entitlementRunner({ leakTo = new Set(), omitFromEmbedding = false } = {}) {
  return async (command, args) => {
    if (command !== "codesign" || !args.includes("--entitlements")) {
      return { status: 0, stdout: "", stderr: "" };
    }
    const target = args.at(-1);
    const basename = path.basename(target);
    const hasEntitlement =
      (!omitFromEmbedding && basename === "resume-embedding-runtime") ||
      leakTo.has(basename) ||
      (target.endsWith(".app") && leakTo.has("resume-ir.app"));
    return {
      status: 0,
      stdout: hasEntitlement ? LIBRARY_VALIDATION_ENTITLEMENT : "",
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

function verifiedDmgReceipt() {
  return {
    schema_version: "resume-ir.macos-dmg-composition.v1",
    distribution_signature: "accepted",
    distribution_profile: "internal_test",
    code_signature: "ad_hoc_valid",
    hardened_runtime: true,
    library_validation_entitlement_scope: "embedding_runtime_only",
    notarization: "not_requested",
    tester_allow_list_required: true,
    dmg_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
    if (command === "hdiutil" && args[0] === "attach") {
      await createMountedLayout(args[args.indexOf("-mountpoint") + 1]);
      await mkdir(
        path.join(args[args.indexOf("-mountpoint") + 1], ".fseventsd"),
      );
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "hdiutil" && args[0] === "create") {
      const stagingDirectory = args[args.indexOf("-srcfolder") + 1];
      assert.equal((await readdir(stagingDirectory)).includes(".fseventsd"), false);
      await writeFile(args.at(-1), "rewritten-dmg");
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "codesign" && args.includes("--entitlements")) {
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

  assert.deepEqual(
    await applyMacosInternalTestEntitlements({
      dmg,
      entitlements: fileURLToPath(entitlements),
      platform: "darwin",
      runner,
    }),
    { library_validation_entitlement_scope: "embedding_runtime_only" },
  );
  assert.equal(await readFile(dmg, "utf8"), "rewritten-dmg");

  const signingCalls = calls.filter(
    ([command, ...args]) => command === "codesign" && args.includes("--force"),
  );
  assert.equal(signingCalls.length, 2);
  assert.equal(signingCalls[0].at(-1).endsWith("/resume-embedding-runtime"), true);
  assert.equal(signingCalls[0].includes("--entitlements"), true);
  assert.equal(signingCalls[1].at(-1).endsWith("/resume-ir.app"), true);
  assert.equal(signingCalls[1].includes("--entitlements"), false);
  assert.equal(signingCalls.some((call) => call.includes("--deep")), false);

  const attach = calls.find(
    ([command, action]) => command === "hdiutil" && action === "attach",
  );
  const create = calls.find(
    ([command, action]) => command === "hdiutil" && action === "create",
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
    if (command === "hdiutil" && args[0] === "attach") {
      await createMountedLayout(args[args.indexOf("-mountpoint") + 1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (
      command === "codesign" &&
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
    if (command === "hdiutil" && args[0] === "attach") {
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
    if (command === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "codesign") signingCalls += 1;
    return { status: 0, stdout: "", stderr: "" };
  };

  await assert.rejects(
    applyMacosInternalTestEntitlements({
      dmg,
      entitlements,
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
    if (command === "hdiutil" && args[0] === "attach") {
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
    if (command === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "codesign") signingCalls += 1;
    return { status: 0, stdout: "", stderr: "" };
  };

  await assert.rejects(
    applyMacosInternalTestEntitlements({
      dmg,
      entitlements,
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
    if (command === "hdiutil" && args[0] === "attach") {
      const mountDirectory = args[args.indexOf("-mountpoint") + 1];
      await createMountedLayout(mountDirectory);
      await symlink(root, path.join(mountDirectory, ".fseventsd"));
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "hdiutil" && args[0] === "detach") {
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
    if (command === "hdiutil" && args[0] === "attach") {
      await createMountedLayout(args[args.indexOf("-mountpoint") + 1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "hdiutil" && args[0] === "detach") {
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
    if (command === "codesign" && args.includes("--entitlements")) {
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
  const runner = async (command, args) => {
    if (command === "hdiutil" && args[0] === "attach") {
      await createMountedLayout(args[args.indexOf("-mountpoint") + 1]);
      return {
        status: null,
        error: { code: "ETIMEDOUT" },
        stdout: "",
        stderr: "/private/mount/path",
      };
    }
    if (command === "hdiutil" && args[0] === "detach") {
      detachCalls += 1;
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
  assert.equal(await readFile(dmg, "utf8"), "original-dmg");
  assert.deepEqual((await readdir(root)).sort(), ["resume-ir.dmg"]);
});

test("verifies one DMG, delegates exact App verification, and always detaches", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-verify-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");
  const calls = [];
  const runner = async (command, args) => {
    calls.push([command, ...args]);
    if (command === "hdiutil" && args[0] === "attach") {
      const mountDirectory = args.at(-1);
      await createMountedLayout(mountDirectory);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "hdiutil" && args[0] === "detach") {
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "codesign" && args.includes("--entitlements")) {
      const target = args.at(-1);
      return {
        status: 0,
        stdout: path.basename(target) === "resume-embedding-runtime"
          ? LIBRARY_VALIDATION_ENTITLEMENT
          : "",
        stderr: `Executable=${target}`,
      };
    }
    if (command === "codesign" && args.includes("--display")) {
      return {
        status: 0,
        stdout: "",
        stderr: [
          "CodeDirectory v=20400 size=120 flags=0x10000(runtime)",
          "Signature=adhoc",
          "TeamIdentifier=not set",
          "Sealed Resources version=2 rules=13 files=42",
        ].join("\n"),
      };
    }
    if (command === "codesign") {
      return { status: 0, stdout: "", stderr: "" };
    }
    return { status: 1, stdout: "", stderr: "not accepted" };
  };
  const verifiedApps = [];
  const receipt = await verifyMacosDmg({
    repoRoot: root,
    targetTriple: "aarch64-apple-darwin",
    dmg,
    temporaryRoot,
    platform: "darwin",
    runner,
    verifyApp: async ({ appBundle }) => {
      verifiedApps.push(appBundle);
      return {
        daemon_sidecar_count: 1,
        embedding_sidecar_count: 1,
        pdf_renderer_sidecar_count: 1,
        embedding_resource_file_count: 7,
        embedding_resource_bytes: 10,
        ocr_resource_file_count: 31,
        ocr_resource_bytes: 20,
        digest_match: true,
        executable: true,
        architecture: "arm64",
        build_machine_identity_path_markers: 0,
      };
    },
  });

  assert.equal(verifiedApps.length, 1);
  assert.deepEqual(calls[0].slice(0, 6), [
    "hdiutil",
    "attach",
    dmg,
    "-readonly",
    "-nobrowse",
    "-mountpoint",
  ]);
  assert.equal(calls.at(-1)[1], "detach");
  assert.deepEqual(await readdir(temporaryRoot), []);
  assert.deepEqual(receipt, {
    schema_version: "resume-ir.macos-dmg-composition.v1",
    target_triple: "aarch64-apple-darwin",
    dmg_count: 1,
    dmg_bytes: 13,
    dmg_sha256: "bf55618abcf4f76365c1784a970a08f5c650c6fc3ad8a613a042601c9a688b61",
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

test("rejects an unsealed or non-ad-hoc App instead of issuing a test receipt", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-signature-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");

  for (const signatureCase of ["verify_failed", "developer_identity"]) {
    const runner = async (command, args) => {
      if (command === "hdiutil" && args[0] === "attach") {
        await createMountedLayout(args.at(-1));
        return { status: 0, stdout: "", stderr: "" };
      }
      if (command === "hdiutil" && args[0] === "detach") {
        await rm(args[1], { recursive: true, force: true });
        await mkdir(args[1]);
        return { status: 0, stdout: "", stderr: "" };
      }
      if (command === "codesign" && args.includes("--verify")) {
        return { status: signatureCase === "verify_failed" ? 1 : 0, stdout: "", stderr: "" };
      }
      if (command === "codesign" && args.includes("--display")) {
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
          ocr_resource_file_count: 31,
          ocr_resource_bytes: 20,
          digest_match: true,
          executable: true,
          architecture: "arm64",
          build_machine_identity_path_markers: 0,
        }),
      }),
      /ad-hoc signature is invalid/,
    );
  }
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
  const runner = async (command, args) => {
    if (command === "hdiutil" && args[0] === "attach") {
      await createMountedLayout(args.at(-1));
      return {
        status: null,
        error: { code: "ETIMEDOUT" },
        stdout: "",
        stderr: "/private/mount/path",
      };
    }
    if (command === "hdiutil" && args[0] === "detach") {
      detachCalls += 1;
      await rm(args[1], { recursive: true, force: true });
      await mkdir(args[1]);
      return { status: 0, stdout: "", stderr: "" };
    }
    return { status: 1, stdout: "", stderr: "" };
  };

  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
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
  assert.deepEqual(await readdir(temporaryRoot), []);
});

test("preserves the verification error when detach reports failure after unmounting", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-dmg-detach-"));
  const temporaryRoot = path.join(root, "mounts");
  const dmg = path.join(root, "resume-ir.dmg");
  context.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(temporaryRoot);
  await writeFile(dmg, "synthetic-dmg");
  const runner = async (command, args) => {
    if (command === "hdiutil" && args[0] === "attach") {
      await createMountedLayout(args.at(-1));
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "hdiutil" && args[0] === "detach") {
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
      targetTriple: "aarch64-apple-darwin",
      dmg,
      temporaryRoot,
      platform: "darwin",
      runner,
      verifyApp: async () => ({}),
    }),
    /ad-hoc signature is invalid/,
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
    if (command === "hdiutil" && args[0] === "attach") {
      await createMountedLayout(args.at(-1));
      return { status: 0, stdout: "", stderr: "" };
    }
    if (command === "hdiutil" && args[0] === "detach") {
      return { status: 1, stdout: "", stderr: "" };
    }
    return { status: 1, stdout: "", stderr: "" };
  };

  await assert.rejects(
    verifyMacosDmg({
      repoRoot: root,
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
  assert.equal(path.basename(plan.dmg), "resume-ir_0.1.0_aarch64.dmg");
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

test("removes a stale canonical DMG when the Tauri build fails", async (context) => {
  const fixture = await createTestReleaseFixture(context);
  await writeFile(fixture.plan.dmg, "stale-unverified-dmg");

  await assert.rejects(
    buildMacosInternalTestRelease({
      ...fixture,
      platform: "darwin",
      runner: () => ({ status: 1, stderr: "/private/build/path" }),
    }),
    /internal-test build failed/,
  );
  await assert.rejects(lstat(fixture.plan.dmg), { code: "ENOENT" });
});

test("does not leave a canonical DMG when full verification fails", async (context) => {
  const fixture = await createTestReleaseFixture(context);

  await assert.rejects(
    buildMacosInternalTestRelease({
      ...fixture,
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
  assert.deepEqual(
    await buildMacosInternalTestRelease({
      ...fixture,
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
      verifyDmg: async () => receipt,
    }),
    receipt,
  );
  assert.equal(calls.length, 1);
  assert.equal(calls[0].args[0], fixture.runTauri);
  assert.equal(calls[0].options.env.APPLE_PASSWORD, undefined);
  assert.equal(calls[0].options.env.APPLE_SIGNING_IDENTITY, "-");
  assert.equal(entitlementCalls.length, 1);
  assert.notEqual(entitlementCalls[0].dmg, fixture.plan.dmg);

  await assert.rejects(
    buildMacosInternalTestRelease({
      ...fixture,
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

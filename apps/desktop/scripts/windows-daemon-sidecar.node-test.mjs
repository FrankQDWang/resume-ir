import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  buildWindowsDaemonSidecar,
  createWindowsDaemonSidecarBuildPlan,
  promoteWindowsDaemonSidecar,
} from "./windows-daemon-sidecar.mjs";

const RUNTIME_TARGET = "x86_64-pc-windows-gnu";
const BUNDLE_TARGET = "x86_64-pc-windows-msvc";

function sha256(body) {
  return createHash("sha256").update(body).digest("hex");
}

function syntheticExecutable(importName = "KERNEL32.dll", marker = "fixture") {
  const body = Buffer.alloc(4096);
  const pe = 0x80;
  const optional = pe + 24;
  const section = optional + 0xf0;
  const raw = (rva) => 0x400 + rva - 0x1000;
  body.write("MZ", 0, "ascii");
  body.writeUInt32LE(pe, 0x3c);
  body.write("PE\0\0", pe, "ascii");
  body.writeUInt16LE(0x8664, pe + 4);
  body.writeUInt16LE(1, pe + 6);
  body.writeUInt16LE(0xf0, pe + 20);
  body.writeUInt16LE(0x0002, pe + 22);
  body.writeUInt16LE(0x20b, optional);
  body.writeUInt32LE(16, optional + 108);
  body.writeUInt32LE(0x1200, optional + 120);
  body.writeUInt32LE(40, optional + 124);
  body.write(".rdata", section, "ascii");
  body.writeUInt32LE(0x1000, section + 8);
  body.writeUInt32LE(0x1000, section + 12);
  body.writeUInt32LE(0xc00, section + 16);
  body.writeUInt32LE(0x400, section + 20);
  body.writeUInt32LE(0x1300, raw(0x1200) + 12);
  body.write(importName, raw(0x1300), "ascii");
  const encodedMarker = Buffer.isBuffer(marker) ? marker : Buffer.from(marker);
  encodedMarker.copy(body, raw(0x1400));
  return body;
}

async function fixture(context, prefix) {
  const root = await mkdtemp(path.join(os.tmpdir(), prefix));
  const repoRoot = path.join(root, "repo");
  const homeDirectory = path.join(root, "home", "builder-user");
  const buildTargetDir = path.join(root, "build");
  await mkdir(repoRoot, { recursive: true });
  await mkdir(homeDirectory, { recursive: true });
  context.after(() => rm(root, { recursive: true, force: true }));
  return { root, repoRoot, homeDirectory, buildTargetDir };
}

async function writeArtifact(plan, body) {
  await mkdir(path.dirname(plan.artifact.source), { recursive: true });
  await writeFile(plan.artifact.source, body);
}

test("pins GNU daemon runtime and MSVC bundle targets separately", () => {
  const repoRoot = path.join(path.sep, "synthetic", "resume-ir");
  const homeDirectory = path.join(path.sep, "synthetic", "builder-home");
  const buildTargetDir = path.join(path.sep, "synthetic-build", "windows-daemon");
  const plan = createWindowsDaemonSidecarBuildPlan({
    repoRoot,
    homeDirectory,
    buildTargetDir,
  });
  assert.equal(plan.schemaVersion, "resume-ir.windows-daemon-sidecar-build-plan.v1");
  assert.equal(plan.runtimeTargetTriple, RUNTIME_TARGET);
  assert.equal(plan.bundleTargetTriple, BUNDLE_TARGET);
  assert.equal(plan.cargoZigbuildVersion, "0.23.0");
  assert.equal(plan.zigVersion, "0.16.0");
  assert.equal(plan.command, "cargo");
  assert.deepEqual(plan.args, [
    "zigbuild",
    "--quiet",
    "--locked",
    "--release",
    "--target",
    RUNTIME_TARGET,
    "-p",
    "resume-daemon",
  ]);
  assert.deepEqual(plan.environment.CARGO_ENCODED_RUSTFLAGS.split("\u001f"), [
    "-D",
    "warnings",
    "-C",
    "target-feature=+crt-static",
    `--remap-path-prefix=${repoRoot}=/source/resume-ir`,
    `--remap-path-prefix=${homeDirectory}=/build-home`,
  ]);
  assert.equal(plan.environment.CARGO_TARGET_DIR, buildTargetDir);
  assert.equal(plan.environment.RUSTFLAGS, undefined);
  assert.deepEqual(
    { role: plan.artifact.role, file: plan.artifact.file },
    { role: "daemon", file: `resume-daemon-${BUNDLE_TARGET}.exe` },
  );
});

test("builds, validates and stages the reviewed x64 daemon", async (context) => {
  const paths = await fixture(context, "resume-ir-win-daemon-");
  const plan = createWindowsDaemonSidecarBuildPlan(paths);
  const body = syntheticExecutable();
  const inspections = [];
  let observedBuild;
  const receipt = await buildWindowsDaemonSidecar({
    ...paths,
    inspect: async (request) => {
      inspections.push(request);
      return request.command === "cargo-zigbuild"
        ? "cargo-zigbuild 0.23.0"
        : "0.16.0";
    },
    runBuild: async (request) => {
      observedBuild = request;
      await writeArtifact(plan, body);
    },
  });
  assert.deepEqual(
    inspections.map(({ command, args }) => ({ command, args })),
    [
      { command: "cargo-zigbuild", args: ["--version"] },
      { command: "zig", args: ["version"] },
    ],
  );
  assert.deepEqual(observedBuild, {
    command: "cargo",
    args: plan.args,
    cwd: paths.repoRoot,
    environment: plan.environment,
  });
  assert.deepEqual(receipt, {
    schema_version: "resume-ir.windows-daemon-sidecar-build.v1",
    runtime_target_triple: RUNTIME_TARGET,
    bundle_target_triple: BUNDLE_TARGET,
    profile: "release",
    cargo_zigbuild_version: "0.23.0",
    zig_version: "0.16.0",
    artifact_count: 1,
    dependency_closure: "windows-10-system-dlls-only",
    sqlcipher_openssl_linkage: "static",
    build_machine_identity_path_markers: 0,
    artifacts: [
      {
        role: "daemon",
        file: `resume-daemon-${BUNDLE_TARGET}.exe`,
        bytes: body.length,
        sha256: sha256(body),
        import_count: 1,
      },
    ],
  });
  assert.ok(Buffer.byteLength(JSON.stringify(receipt)) < 4096);
  assert.ok(!JSON.stringify(receipt).includes(paths.root));
  assert.deepEqual(await readFile(plan.artifact.destination), body);
});

test("rejects non-system and dynamic runtime imports without replacing output", async (context) => {
  const paths = await fixture(context, "resume-ir-win-import-");
  const plan = createWindowsDaemonSidecarBuildPlan(paths);
  await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", "accepted"));
  await promoteWindowsDaemonSidecar(plan);
  const accepted = await readFile(plan.artifact.destination);
  for (const imported of "LIBCRYPTO-3-X64.DLL LIBGCC_S_SEH-1.DLL LIBWINPTHREAD-1.DLL VCRUNTIME140.DLL UNKNOWN.DLL".split(
    " ",
  )) {
    await writeArtifact(plan, syntheticExecutable(imported, "rejected"));
    await assert.rejects(
      promoteWindowsDaemonSidecar(plan),
      /dependency closure is not self-contained/,
    );
    assert.deepEqual(await readFile(plan.artifact.destination), accepted);
  }
});

test("accepts generic system path text but rejects exact builder identity", async (context) => {
  const paths = await fixture(context, "resume-ir-win-identity-");
  const plan = createWindowsDaemonSidecarBuildPlan(paths);
  await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", "/Users/"));
  await promoteWindowsDaemonSidecar(plan);
  const accepted = await readFile(plan.artifact.destination);
  for (const marker of [
    Buffer.from(paths.repoRoot, "utf8"),
    Buffer.from(paths.homeDirectory, "utf16le"),
  ]) {
    await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", marker));
    await assert.rejects(
      promoteWindowsDaemonSidecar(plan),
      /contains build-machine identity/,
    );
    assert.deepEqual(await readFile(plan.artifact.destination), accepted);
  }
});

test("refuses a symlink build target before invoking Cargo", async (context) => {
  const paths = await fixture(context, "resume-ir-win-build-target-");
  const realBuild = path.join(path.dirname(paths.buildTargetDir), "real-build");
  await mkdir(realBuild);
  await symlink(realBuild, paths.buildTargetDir);
  let invoked = false;
  await assert.rejects(
    buildWindowsDaemonSidecar({
      ...paths,
      inspect: async ({ command }) =>
        command === "cargo-zigbuild" ? "cargo-zigbuild 0.23.0" : "0.16.0",
      runBuild: async () => {
        invoked = true;
      },
    }),
    /build target is not secure/,
  );
  assert.equal(invoked, false);
});

test("rejects toolchain drift before preparing the build target", async (context) => {
  const paths = await fixture(context, "resume-ir-win-toolchain-");
  await assert.rejects(
    buildWindowsDaemonSidecar({
      ...paths,
      inspect: async ({ command }) =>
        command === "cargo-zigbuild" ? "cargo-zigbuild 0.24.0" : "0.16.0",
      runBuild: async () => assert.fail("build must not run"),
    }),
    /toolchain version is invalid/,
  );
});

test("restores the prior daemon after atomic promotion fails", async (context) => {
  const paths = await fixture(context, "resume-ir-win-daemon-rollback-");
  const plan = createWindowsDaemonSidecarBuildPlan(paths);
  await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", "accepted"));
  await promoteWindowsDaemonSidecar(plan);
  const accepted = await readFile(plan.artifact.destination);
  await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", "candidate"));
  await assert.rejects(
    promoteWindowsDaemonSidecar(plan, {
      beforePromote: () => {
        throw new Error("synthetic promotion failure");
      },
    }),
    /Windows daemon sidecar staging failed/,
  );
  assert.deepEqual(await readFile(plan.artifact.destination), accepted);
});

test("fails closed for missing or non-x64 daemon artifacts", async (context) => {
  const paths = await fixture(context, "resume-ir-win-daemon-missing-");
  const plan = createWindowsDaemonSidecarBuildPlan(paths);
  await assert.rejects(
    promoteWindowsDaemonSidecar(plan),
    /Windows daemon sidecar artifact is missing/,
  );
  const malformed = syntheticExecutable();
  malformed.writeUInt16LE(0x014c, 0x84);
  await writeArtifact(plan, malformed);
  await assert.rejects(promoteWindowsDaemonSidecar(plan), /is not x64/);
});

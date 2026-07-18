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
  buildWindowsOnnxRuntime,
  createWindowsOnnxRuntimeBuildPlan,
} from "./windows-onnxruntime-builder.mjs";

const COMMIT = "2d924974ef147392ced8409d36bd6d2e7fcc8a74";
const environment = {
  VSCMD_VER: "17.14.8",
  VCToolsVersion: "14.44.35207",
  WindowsSDKVersion: "10.0.26100.0\\",
};

function sha256(body) {
  return createHash("sha256").update(body).digest("hex");
}

function syntheticDll(importName = "KERNEL32.dll") {
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
  body.writeUInt16LE(0x2000, pe + 22);
  body.writeUInt16LE(0x20b, optional);
  body.writeUInt32LE(16, optional + 108);
  body.writeUInt32LE(0x1100, optional + 112);
  body.writeUInt32LE(0x80, optional + 116);
  body.writeUInt32LE(0x1200, optional + 120);
  body.writeUInt32LE(40, optional + 124);
  body.write(".rdata", section, "ascii");
  body.writeUInt32LE(0x1000, section + 8);
  body.writeUInt32LE(0x1000, section + 12);
  body.writeUInt32LE(0xc00, section + 16);
  body.writeUInt32LE(0x400, section + 20);
  body.writeUInt32LE(1, raw(0x1100) + 24);
  body.writeUInt32LE(0x1150, raw(0x1100) + 32);
  body.writeUInt32LE(0x1170, raw(0x1150));
  body.write("OrtGetApiBase\0", raw(0x1170), "ascii");
  body.writeUInt32LE(0x1300, raw(0x1200) + 12);
  body.write(importName, raw(0x1300), "ascii");
  return body;
}

async function fixture(context, prefix = "resume-ir-win-ort-builder-") {
  const root = await mkdtemp(path.join(os.tmpdir(), prefix));
  const sourceRoot = path.join(root, "onnxruntime");
  const destination = path.join(root, "published", "runtime");
  const contractFile = path.join(root, "source-contract.json");
  const buildScript = path.join(sourceRoot, "tools", "ci_build", "build.py");
  await mkdir(path.dirname(buildScript), { recursive: true });
  const license = Buffer.from("synthetic MIT license\n");
  const notices = Buffer.from("synthetic notices\n");
  await writeFile(buildScript, "# synthetic official build script\n");
  await writeFile(path.join(sourceRoot, "LICENSE"), license);
  await writeFile(path.join(sourceRoot, "ThirdPartyNotices.txt"), notices);
  const contract = JSON.parse(
    await readFile(
      new URL(
        "../resources/embedding/x86_64-pc-windows-msvc/source-contract.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  contract.onnxruntime.source_license_file = {
    file: "LICENSE",
    bytes: license.length,
    sha256: sha256(license),
  };
  contract.onnxruntime.source_notices_file = {
    file: "ThirdPartyNotices.txt",
    bytes: notices.length,
    sha256: sha256(notices),
  };
  await writeFile(contractFile, `${JSON.stringify(contract, null, 2)}\n`);
  context.after(() => rm(root, { recursive: true, force: true }));
  return { root, sourceRoot, destination, contractFile };
}

function inspectFixture(overrides = {}) {
  return async ({ command, args }) => {
    const key = `${command} ${args.join(" ")}`;
    const values = {
      "python --version": "Python 3.12.10\r\n",
      "cmake --version": "cmake version 3.31.8\r\n",
      "git rev-parse HEAD": `${COMMIT}\n`,
      "git status --porcelain --untracked-files=all": "",
      "git submodule status --recursive": ` ${"a".repeat(40)} cmake/external/onnx\n`,
      ...overrides,
    };
    if (!(key in values)) throw new Error("unexpected inspection request");
    return values[key];
  };
}

test("pins the official native Windows static-MSVCRT build owner", () => {
  const plan = createWindowsOnnxRuntimeBuildPlan({
    sourceRoot: path.join(path.sep, "synthetic", "onnxruntime"),
    destination: path.join(path.sep, "synthetic-output", "runtime"),
    platform: "win32",
    architecture: "x64",
    environment,
  });
  assert.equal(plan.schemaVersion, "resume-ir.windows-onnxruntime-build-plan.v1");
  assert.equal(plan.targetTriple, "x86_64-pc-windows-msvc");
  assert.equal(plan.command, "python");
  assert.deepEqual(plan.args.slice(-10), [
    "--config",
    "Release",
    "--build_shared_lib",
    "--enable_msvc_static_runtime",
    "--skip_submodule_sync",
    "--parallel",
    "1",
    "--update",
    "--build",
    "--test",
  ]);
  assert.equal(
    plan.artifactSource,
    path.join(plan.buildRoot, "Release", "Release", "onnxruntime.dll"),
  );
  assert.deepEqual(
    [plan.visualStudioVersion, plan.msvcToolsetVersion, plan.windowsSdkVersion],
    ["17.14.8", "14.44.35207", "10.0.26100.0"],
  );
});

test("publishes a validated runtime root with provenance v2", async (context) => {
  const inputs = await fixture(context);
  const plan = createWindowsOnnxRuntimeBuildPlan({
    ...inputs,
    platform: "win32",
    architecture: "x64",
    environment,
  });
  let buildRequest;
  const body = syntheticDll();
  const result = await buildWindowsOnnxRuntime({
    ...inputs,
    platform: "win32",
    architecture: "x64",
    environment,
    inspect: inspectFixture(),
    runBuild: async (request) => {
      buildRequest = request;
      await mkdir(path.dirname(plan.artifactSource), { recursive: true });
      await writeFile(plan.artifactSource, body);
    },
  });
  assert.deepEqual(buildRequest, {
    command: "python",
    args: plan.args,
    cwd: inputs.sourceRoot,
  });
  assert.deepEqual(result, {
    schema_version: "resume-ir.windows-onnxruntime-build.v1",
    target_triple: "x86_64-pc-windows-msvc",
    source_commit: COMMIT,
    profile: "Release",
    tests_passed: true,
    dependency_closure: "windows-system-dlls-only",
    artifact_count: 1,
    artifacts: [
      {
        role: "runtime_library",
        file: "onnxruntime.dll",
        bytes: body.length,
        sha256: sha256(body),
        import_count: 1,
      },
    ],
    toolchain: {
      visual_studio_version: "17.14.8",
      msvc_toolset_version: "14.44.35207",
      windows_sdk_version: "10.0.26100.0",
    },
  });
  assert.ok(Buffer.byteLength(JSON.stringify(result)) < 4096);
  assert.ok(!JSON.stringify(result).includes(inputs.root));
  const provenance = JSON.parse(
    await readFile(path.join(inputs.destination, "build-provenance.json"), "utf8"),
  );
  assert.equal(provenance.schema_version, "resume-ir.onnxruntime-windows-build-provenance.v2");
  assert.equal(provenance.python_version, "3.12.10");
  assert.equal(provenance.cmake_version, "3.31.8");
  assert.equal(provenance.tests_passed, true);
  assert.deepEqual(await readFile(path.join(inputs.destination, "onnxruntime.dll")), body);
});

test("rejects unsupported hosts, tool drift and source drift before build", async (context) => {
  const inputs = await fixture(context, "resume-ir-win-ort-admission-");
  assert.throws(
    () =>
      createWindowsOnnxRuntimeBuildPlan({
        ...inputs,
        platform: "darwin",
        architecture: "arm64",
        environment,
      }),
    /requires native Windows x64/,
  );
  for (const overrides of [
    { "python --version": "Python 3.9.19\n" },
    { "cmake --version": "cmake version 3.27.9\n" },
    { "git rev-parse HEAD": `${"b".repeat(40)}\n` },
    { "git status --porcelain --untracked-files=all": " M source.cc\n" },
    { "git status --porcelain --untracked-files=all": "?? local.cmake\n" },
    { "git submodule status --recursive": `-${"a".repeat(40)} cmake/external/onnx\n` },
  ]) {
    let built = false;
    await assert.rejects(
      buildWindowsOnnxRuntime({
        ...inputs,
        platform: "win32",
        architecture: "x64",
        environment,
        inspect: inspectFixture(overrides),
        runBuild: async () => {
          built = true;
        },
      }),
      /source or toolchain identity is invalid/,
    );
    assert.equal(built, false);
  }
});

test("rejects unsafe paths and a symlinked build root", async (context) => {
  const inputs = await fixture(context, "resume-ir-win-ort-path-");
  assert.throws(
    () =>
      createWindowsOnnxRuntimeBuildPlan({
        ...inputs,
        destination: path.join(inputs.sourceRoot, "published"),
        platform: "win32",
        architecture: "x64",
        environment,
      }),
    /paths overlap/,
  );
  const plan = createWindowsOnnxRuntimeBuildPlan({
    ...inputs,
    platform: "win32",
    architecture: "x64",
    environment,
  });
  await mkdir(path.dirname(plan.buildRoot), { recursive: true });
  const outside = path.join(inputs.root, "outside");
  await mkdir(outside);
  await symlink(outside, plan.buildRoot);
  await assert.rejects(
    buildWindowsOnnxRuntime({
      ...inputs,
      platform: "win32",
      architecture: "x64",
      environment,
      inspect: inspectFixture(),
      runBuild: async () => assert.fail("build must not run"),
    }),
    /build root is invalid/,
  );
});

test("rejects a forbidden runtime import and missing artifact", async (context) => {
  for (const [prefix, body, error] of [
    ["resume-ir-win-ort-crt-", syntheticDll("VCRUNTIME140.dll"), /publish failed/],
    ["resume-ir-win-ort-missing-", null, /build artifact is missing/],
  ]) {
    const inputs = await fixture(context, prefix);
    const plan = createWindowsOnnxRuntimeBuildPlan({
      ...inputs,
      platform: "win32",
      architecture: "x64",
      environment,
    });
    await assert.rejects(
      buildWindowsOnnxRuntime({
        ...inputs,
        platform: "win32",
        architecture: "x64",
        environment,
        inspect: inspectFixture(),
        runBuild: async () => {
          if (body) {
            await mkdir(path.dirname(plan.artifactSource), { recursive: true });
            await writeFile(plan.artifactSource, body);
          }
        },
      }),
      error,
    );
  }
});

test("restores a previously accepted runtime when promotion fails", async (context) => {
  const inputs = await fixture(context, "resume-ir-win-ort-rollback-");
  const plan = createWindowsOnnxRuntimeBuildPlan({
    ...inputs,
    platform: "win32",
    architecture: "x64",
    environment,
  });
  const run = (marker, beforePromote) =>
    buildWindowsOnnxRuntime({
      ...inputs,
      platform: "win32",
      architecture: "x64",
      environment,
      inspect: inspectFixture(),
      runBuild: async () => {
        const body = syntheticDll();
        body.write(marker, 0x900, "ascii");
        await mkdir(path.dirname(plan.artifactSource), { recursive: true });
        await writeFile(plan.artifactSource, body);
      },
      beforePromote,
    });
  await run("accepted");
  const accepted = await readFile(path.join(inputs.destination, "onnxruntime.dll"));
  await assert.rejects(
    run("candidate", () => {
      throw new Error("synthetic promotion failure");
    }),
    /publish failed/,
  );
  assert.deepEqual(
    await readFile(path.join(inputs.destination, "onnxruntime.dll")),
    accepted,
  );
});

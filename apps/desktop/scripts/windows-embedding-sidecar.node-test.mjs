import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  buildWindowsEmbeddingSidecar,
  createWindowsEmbeddingSidecarBuildPlan,
  promoteWindowsEmbeddingSidecar,
} from "./windows-embedding-sidecar.mjs";

const TARGET = "x86_64-pc-windows-msvc";

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
  body.write(marker, raw(0x1400), "ascii");
  return body;
}

async function writeArtifact(plan, body) {
  await mkdir(path.dirname(plan.artifact.source), { recursive: true });
  await writeFile(plan.artifact.source, body);
}

test("pins the official cargo-xwin release build to the resident embedding runtime", () => {
  const repoRoot = path.join(path.sep, "synthetic", "resume-ir");
  const homeDirectory = path.join(path.sep, "synthetic", "builder-home");
  const plan = createWindowsEmbeddingSidecarBuildPlan({ repoRoot, homeDirectory });
  assert.equal(plan.schema_version, "resume-ir.windows-embedding-sidecar-build-plan.v1");
  assert.equal(plan.target_triple, TARGET);
  assert.equal(plan.command, "cargo");
  assert.equal(plan.cargo_xwin_version, "0.22.0");
  assert.deepEqual(plan.environment.CARGO_ENCODED_RUSTFLAGS.split("\u001f"), [
    "-C",
    "target-feature=+crt-static",
    `--remap-path-prefix=${repoRoot}=/source/resume-ir`,
    `--remap-path-prefix=${homeDirectory}=/build-home`,
  ]);
  assert.equal(plan.environment.RUSTFLAGS, undefined);
  assert.deepEqual(plan.args, [
    "xwin",
    "build",
    "--quiet",
    "--locked",
    "--release",
    "--target",
    TARGET,
    "-p",
    "resume-embedding-runtime",
  ]);
  assert.deepEqual(
    { role: plan.artifact.role, file: plan.artifact.file },
    {
      role: "embedding_runtime",
      file: `resume-embedding-runtime-${TARGET}.exe`,
    },
  );
});

test("builds, validates and stages the reviewed x64 embedding sidecar", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-embedding-"));
  context.after(() => rm(repoRoot, { recursive: true, force: true }));
  const homeDirectory = path.join(os.tmpdir(), "resume-ir-builder-home");
  const plan = createWindowsEmbeddingSidecarBuildPlan({ repoRoot, homeDirectory });
  const body = syntheticExecutable();
  await writeArtifact(plan, body);
  let observed;
  const receipt = await buildWindowsEmbeddingSidecar({
    repoRoot,
    homeDirectory,
    inspectTool: async () => "cargo-xwin-xwin 0.22.0",
    runBuild: async (request) => {
      observed = request;
    },
  });
  assert.deepEqual(observed, {
    command: "cargo",
    args: plan.args,
    cwd: repoRoot,
    environment: plan.environment,
  });
  assert.deepEqual(receipt, {
    schema_version: "resume-ir.windows-embedding-sidecar-build.v1",
    target_triple: TARGET,
    profile: "release",
    cargo_xwin_version: "0.22.0",
    artifact_count: 1,
    dependency_closure: "windows-system-dlls-only",
    build_machine_identity_path_markers: 0,
    artifacts: [
      {
        role: "embedding_runtime",
        file: `resume-embedding-runtime-${TARGET}.exe`,
        bytes: body.length,
        sha256: sha256(body),
        import_count: 1,
      },
    ],
  });
  assert.ok(JSON.stringify(receipt).length < 4096);
  assert.ok(!JSON.stringify(receipt).includes(repoRoot));
});

test("rejects dynamic CRT input without changing accepted output", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-crt-"));
  context.after(() => rm(repoRoot, { recursive: true, force: true }));
  const plan = createWindowsEmbeddingSidecarBuildPlan({ repoRoot });
  await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", "accepted"));
  await promoteWindowsEmbeddingSidecar(plan);
  const accepted = await readFile(plan.artifact.destination);
  await writeArtifact(plan, syntheticExecutable("VCRUNTIME140.dll", "rejected"));
  await assert.rejects(
    promoteWindowsEmbeddingSidecar(plan),
    /dependency closure is not self-contained/,
  );
  assert.deepEqual(await readFile(plan.artifact.destination), accepted);
});

test("rejects build-machine identity without changing accepted output", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-path-"));
  context.after(() => rm(repoRoot, { recursive: true, force: true }));
  const plan = createWindowsEmbeddingSidecarBuildPlan({ repoRoot });
  await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", "accepted"));
  await promoteWindowsEmbeddingSidecar(plan);
  const accepted = await readFile(plan.artifact.destination);
  await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", repoRoot));
  await assert.rejects(
    promoteWindowsEmbeddingSidecar(plan),
    /contains build-machine identity/,
  );
  assert.deepEqual(await readFile(plan.artifact.destination), accepted);
});

test("restores the prior sidecar when atomic promotion fails", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-rollback-"));
  context.after(() => rm(repoRoot, { recursive: true, force: true }));
  const plan = createWindowsEmbeddingSidecarBuildPlan({ repoRoot });
  await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", "accepted"));
  await promoteWindowsEmbeddingSidecar(plan);
  const accepted = await readFile(plan.artifact.destination);
  await writeArtifact(plan, syntheticExecutable("KERNEL32.dll", "candidate"));
  await assert.rejects(
    promoteWindowsEmbeddingSidecar(plan, {
      beforePromote: () => {
        throw new Error("synthetic promotion failure");
      },
    }),
    /Windows embedding sidecar staging failed/,
  );
  assert.deepEqual(await readFile(plan.artifact.destination), accepted);
});

test("fails closed when the exact build artifact is missing", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-missing-"));
  context.after(() => rm(repoRoot, { recursive: true, force: true }));
  const plan = createWindowsEmbeddingSidecarBuildPlan({ repoRoot });
  await assert.rejects(
    promoteWindowsEmbeddingSidecar(plan),
    /Windows embedding sidecar artifact is missing/,
  );
});

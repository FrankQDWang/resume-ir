import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  assembleWindowsOcrPack,
  createWindowsOcrRuntimePackManifest,
  validateWindowsOcrBuildProvenance,
  validateWindowsOcrDependencyClosure,
} from "./windows-ocr-artifact.mjs";
import { inspectWindowsPeExecutable } from "./windows-pe.mjs";
import { readWindowsOcrSourceContract } from "./windows-ocr-pack.mjs";

const contractFile = fileURLToPath(
  new URL("../resources/ocr/x86_64-pc-windows-msvc/source-contract.json", import.meta.url),
);

function sha256(body) {
  return createHash("sha256").update(body).digest("hex");
}

function syntheticExecutable(importName = "KERNEL32.dll") {
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
  body.writeUInt16LE(0x0022, pe + 22);
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
  return body;
}

function buildProvenance(contract, artifact, imports = ["KERNEL32.DLL"]) {
  const source = (value) => ({
    version: value.version,
    source_repository: value.source_repository,
    source_tag: value.source_tag,
    source_commit: value.source_commit,
    cmake_generator: value.cmake_generator,
    cmake_arguments: value.cmake_arguments,
    source_tree_clean: true,
  });
  return {
    schema_version: contract.tesseract.build_provenance_schema,
    target_triple: contract.target_triple,
    tesseract: source(contract.tesseract),
    leptonica: source(contract.leptonica),
    msvc_runtime: "static",
    msvc_toolset_version: "14.44.35207",
    windows_sdk_version: "10.0.26100.0",
    cmake_version: "4.1.2",
    ninja_version: "1.13.1",
    tests_passed: true,
    artifact_file: "tesseract.exe",
    artifact_bytes: artifact.length,
    artifact_sha256: sha256(artifact),
    artifact_imports: imports,
  };
}

test("accepts one reviewed static x64 OCR artifact and creates a bounded pack manifest", () => {
  const contract = readWindowsOcrSourceContract(contractFile);
  const artifact = syntheticExecutable();
  const image = inspectWindowsPeExecutable(artifact);
  validateWindowsOcrDependencyClosure(image.imports, contract);
  const provenance = validateWindowsOcrBuildProvenance({
    provenance: buildProvenance(contract, artifact),
    contract,
    artifactBytes: artifact.length,
    artifactSha256: sha256(artifact),
    imports: image.imports,
  });
  const manifest = createWindowsOcrRuntimePackManifest({
    contract,
    artifactIdentity: {
      bytes: artifact.length,
      sha256: sha256(artifact),
    },
    noticeIdentity: {
      bytes: 512,
      sha256: "a".repeat(64),
    },
  });
  assert.equal(provenance.msvc_runtime, "static");
  assert.deepEqual(image.imports, ["KERNEL32.DLL"]);
  assert.equal(manifest.target_triple, "x86_64-pc-windows-msvc");
  assert.deepEqual(
    manifest.files.map(({ role, file }) => [role, file]),
    [
      ["engine_binary", "tesseract.exe"],
      ["language_eng", "tessdata/eng.traineddata"],
      ["language_chi_sim", "tessdata/chi_sim.traineddata"],
      ["engine_config", "tessdata/configs/tsv"],
      ["license_text", "LICENSES/Tesseract-Apache-2.0.txt"],
      ["license_text", "LICENSES/Leptonica-BSD-2-Clause.txt"],
      ["license_text", "LICENSES/tessdata-fast-Apache-2.0.txt"],
      ["third_party_notice", "THIRD-PARTY-NOTICES.json"],
    ],
  );
});

test("rejects dynamic CRT imports and provenance drift", () => {
  const contract = readWindowsOcrSourceContract(contractFile);
  const artifact = syntheticExecutable("VCRUNTIME140.dll");
  const image = inspectWindowsPeExecutable(artifact);
  assert.throws(
    () => validateWindowsOcrDependencyClosure(image.imports, contract),
    /dependency closure is not self-contained/,
  );
  const reviewedArtifact = syntheticExecutable();
  const provenance = buildProvenance(contract, reviewedArtifact);
  provenance.tesseract.source_commit = "0".repeat(40);
  assert.throws(
    () =>
      validateWindowsOcrBuildProvenance({
        provenance,
        contract,
        artifactBytes: reviewedArtifact.length,
        artifactSha256: sha256(reviewedArtifact),
        imports: ["KERNEL32.DLL"],
      }),
    /build provenance does not match artifact/,
  );
});

test("rejects malformed or DLL-shaped OCR executables", () => {
  assert.throws(() => inspectWindowsPeExecutable(Buffer.alloc(1024)), /PE image/);
  const dll = syntheticExecutable();
  dll.writeUInt16LE(0x2022, 0x80 + 22);
  assert.throws(() => inspectWindowsPeExecutable(dll), /executable shape/);
});

test("rejects overlapping or extra artifact inputs before candidate assembly", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-ocr-artifact-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const runtimeRoot = path.join(root, "runtime");
  const dataRoot = path.join(root, "data");
  await mkdir(runtimeRoot);
  await mkdir(dataRoot);
  await writeFile(path.join(runtimeRoot, "unexpected.dll"), "not-reviewed");
  await assert.rejects(
    assembleWindowsOcrPack({
      contractFile,
      runtimeRoot,
      dataRoot,
      destination: runtimeRoot,
    }),
    /must not overlap/,
  );
  await assert.rejects(
    assembleWindowsOcrPack({
      contractFile,
      runtimeRoot,
      dataRoot,
      destination: path.join(root, "candidate"),
    }),
    /must contain exactly the reviewed files/,
  );
});

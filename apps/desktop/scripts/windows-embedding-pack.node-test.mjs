import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  assembleWindowsEmbeddingPack,
  inspectWindowsPeImage,
  validateWindowsEmbeddingSourceContract,
} from "./windows-embedding-pack.mjs";

const productionContract = new URL(
  "../resources/embedding/x86_64-pc-windows-msvc/source-contract.json",
  import.meta.url,
);

function sha256(body) {
  return createHash("sha256").update(body).digest("hex");
}

function syntheticPe(importName = "KERNEL32.dll") {
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

async function createFixture(root, importName = "KERNEL32.dll") {
  const runtimeRoot = path.join(root, "runtime");
  const modelPackRoot = path.join(root, "model");
  const destination = path.join(root, "assembled");
  await mkdir(runtimeRoot, { recursive: true });
  await mkdir(modelPackRoot, { recursive: true });
  const contract = JSON.parse(await readFile(productionContract, "utf8"));
  const license = Buffer.from("synthetic MIT license\n");
  const notices = Buffer.from("synthetic third-party notices\n");
  contract.onnxruntime.source_license_file.bytes = license.length;
  contract.onnxruntime.source_license_file.sha256 = sha256(license);
  contract.onnxruntime.source_notices_file.bytes = notices.length;
  contract.onnxruntime.source_notices_file.sha256 = sha256(notices);
  const modelBodies = [
    ["model", "model.onnx", "model"],
    ["tokenizer", "tokenizer.json", "tokenizer"],
    ["model_config", "config.json", "config"],
    ["special_tokens_map", "special_tokens_map.json", "special"],
    ["tokenizer_config", "tokenizer_config.json", "tokenizer-config"],
  ];
  const modelEntries = [];
  for (const [role, file, value] of modelBodies) {
    const body = Buffer.from(value);
    await writeFile(path.join(modelPackRoot, file), body);
    modelEntries.push({ role, file, bytes: body.length, sha256: sha256(body) });
  }
  const modelManifest = {
    schema_version: "resume-ir.embedding-runtime-pack.v1",
    runtime_pack_id: "intfloat-multilingual-e5-small-qint8-r1",
    model_id: "intfloat-multilingual-e5-small-qint8-r1",
    upstream_model_id: "intfloat/multilingual-e5-small",
    upstream_revision: "614241f622f53c4eeff9890bdc4f31cfecc418b3",
    dimension: 384,
    provider: "cpu",
    network_access: "disabled",
    license_reviewed: true,
    model_license: "MIT",
    onnxruntime_license: "MIT",
    files: [
      { role: "runtime_library", file: "old-runtime", bytes: 1, sha256: "a".repeat(64) },
      ...modelEntries,
    ],
    upstream_model_file: "onnx/model_qint8_avx512_vnni.onnx",
    quantization: "dynamic_int8",
  };
  const modelManifestBody = Buffer.from(`${JSON.stringify(modelManifest, null, 2)}\n`);
  await writeFile(path.join(modelPackRoot, "runtime-pack.json"), modelManifestBody);
  contract.model_assets.source_manifest_sha256 = sha256(modelManifestBody);
  const contractFile = path.join(root, "source-contract.json");
  await writeFile(contractFile, `${JSON.stringify(contract, null, 2)}\n`);
  const dll = syntheticPe(importName);
  await writeFile(path.join(runtimeRoot, "onnxruntime.dll"), dll);
  await writeFile(path.join(runtimeRoot, "LICENSE"), license);
  await writeFile(path.join(runtimeRoot, "ThirdPartyNotices.txt"), notices);
  await writeFile(
    path.join(runtimeRoot, "build-provenance.json"),
    `${JSON.stringify(
      {
        schema_version: "resume-ir.onnxruntime-windows-build-provenance.v2",
        target_triple: "x86_64-pc-windows-msvc",
        source_repository: "https://github.com/microsoft/onnxruntime",
        source_tag: "v1.24.4",
        source_commit: "2d924974ef147392ced8409d36bd6d2e7fcc8a74",
        version: "1.24.4",
        api_version: 24,
        build_arguments: contract.onnxruntime.build_arguments,
        provider: "cpu",
        telemetry: false,
        source_tree_clean: true,
        builder_platform: "windows",
        builder_architecture: "x86_64",
        python_version: "3.12.10",
        visual_studio_version: "17.14.8",
        msvc_toolset_version: "14.44.35207",
        windows_sdk_version: "10.0.26100.0",
        cmake_version: "4.1.2",
        tests_passed: true,
        artifact_file: "onnxruntime.dll",
        artifact_bytes: dll.length,
        artifact_sha256: sha256(dll),
      },
      null,
      2,
    )}\n`,
  );
  return { contractFile, runtimeRoot, modelPackRoot, destination };
}

test("assembles only a reviewed static-CRT x64 embedding dependency closure", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-embedding-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const fixture = await createFixture(root);
  const result = await assembleWindowsEmbeddingPack(fixture);
  assert.deepEqual(result, {
    schema_version: "resume-ir.windows-embedding-pack-assembly.v1",
    target_triple: "x86_64-pc-windows-msvc",
    resource_file_count: 11,
    runtime_import_count: 1,
  });
  assert.deepEqual((await readdir(fixture.destination)).sort(), [
    "ONNXRUNTIME-LICENSE.txt",
    "ONNXRUNTIME-THIRD-PARTY-NOTICES.txt",
    "build-provenance.json",
    "config.json",
    "model.onnx",
    "onnxruntime.dll",
    "runtime-pack.json",
    "source-contract.json",
    "special_tokens_map.json",
    "tokenizer.json",
    "tokenizer_config.json",
  ]);
  const manifest = JSON.parse(
    await readFile(path.join(fixture.destination, "runtime-pack.json"), "utf8"),
  );
  assert.equal(manifest.files[0].file, "onnxruntime.dll");
});

test("rejects dynamic MSVC imports even when provenance claims static runtime", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-embedding-crt-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  await assert.rejects(
    assembleWindowsEmbeddingPack(await createFixture(root, "MSVCP140.dll")),
    /dependency closure is not self-contained/,
  );
});

test("rejects a contract that accepts the observed dynamic-CRT archive", async () => {
  const contract = JSON.parse(await readFile(productionContract, "utf8"));
  contract.onnxruntime.official_prebuilt_observation.accepted_as_self_contained = true;
  assert.throws(
    () => validateWindowsEmbeddingSourceContract(contract),
    /source contract is invalid/,
  );
  assert.deepEqual(inspectWindowsPeImage(syntheticPe()).imports, ["KERNEL32.DLL"]);
});

test("rejects an output that overlaps a reviewed input", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-embedding-overlap-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const fixture = await createFixture(root);
  fixture.destination = fixture.modelPackRoot;
  await assert.rejects(assembleWindowsEmbeddingPack(fixture), /must not overlap/);
});

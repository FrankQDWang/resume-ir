import assert from "node:assert/strict";
import { copyFile, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  readWindowsOcrBuilderContract,
  validateWindowsOcrBuilderContract,
} from "./windows-ocr-builder.mjs";
import { readWindowsOcrSourceContract } from "./windows-ocr-pack.mjs";

const builderRoot = fileURLToPath(
  new URL("../runtime-build/windows-ocr/", import.meta.url),
);
const builderContractFile = path.join(builderRoot, "builder-contract.json");
const sourceContractFile = fileURLToPath(
  new URL(
    "../resources/ocr/x86_64-pc-windows-msvc/source-contract.json",
    import.meta.url,
  ),
);

test("accepts the pinned independent Windows OCR builder", async () => {
  const contract = await readWindowsOcrBuilderContract(builderContractFile);
  const source = readWindowsOcrSourceContract(sourceContractFile);
  assert.equal(contract.container.platform, "linux/amd64");
  assert.equal(contract.xwin.version, "0.9.0");
  assert.equal(contract.xwin.sdk_version, "10.0.26100");
  assert.equal(contract.xwin.crt_version, "14.44.17.14");
  assert.equal(contract.stages.native_smoke_required, true);
  assert.equal(contract.stages.native_smoke_host, "linux/amd64-native");
  assert.ok(
    source.leptonica.cmake_arguments.includes(
      "-DCMAKE_POLICY_DEFAULT_CMP0091=NEW",
    ),
  );
  assert.ok(source.tesseract.cmake_arguments.includes("-DLEPT_TIFF_RESULT=1"));
  assert.ok(
    source.tesseract.cmake_arguments.includes(
      "-DLEPT_TIFF_COMPILE_SUCCESS=TRUE",
    ),
  );
});

test("rejects builder pin, smoke, recipe, and extra-field drift", async () => {
  const original = JSON.parse(await readFile(builderContractFile, "utf8"));
  for (const change of [
    (value) => {
      value.xwin.archive_sha256 = "0".repeat(64);
    },
    (value) => {
      value.xwin.http_retry = 0;
    },
    (value) => {
      value.stages.native_smoke_required = false;
    },
    (value) => {
      value.toolchain.msvc_runtime = "dynamic";
    },
    (value) => {
      value.extra = true;
    },
  ]) {
    const candidate = structuredClone(original);
    change(candidate);
    assert.throws(
      () => validateWindowsOcrBuilderContract(candidate),
      /Windows OCR builder contract is invalid/,
    );
  }

  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-win-ocr-builder-"));
  try {
    for (const entry of original.recipe_files) {
      await copyFile(path.join(builderRoot, entry.file), path.join(root, entry.file));
    }
    await writeFile(
      path.join(root, "builder-contract.json"),
      `${JSON.stringify(original, null, 2)}\n`,
    );
    await writeFile(path.join(root, "Dockerfile"), "FROM scratch\n");
    await assert.rejects(
      readWindowsOcrBuilderContract(path.join(root, "builder-contract.json")),
      /Windows OCR builder recipe identity is invalid/,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

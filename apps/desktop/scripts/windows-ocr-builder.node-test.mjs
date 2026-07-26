import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  readWindowsOcrBuilderContract,
  validateWindowsOcrBuilderContract,
} from "./windows-ocr-builder.mjs";

const builderContractFile = fileURLToPath(
  new URL("../runtime-build/windows-ocr/builder-contract.json", import.meta.url),
);

test("accepts the pinned native Windows OCR builder", async () => {
  const contract = await readWindowsOcrBuilderContract(builderContractFile);
  assert.equal(contract.host.platform, "windows");
  assert.equal(contract.host.architecture, "x86_64");
  assert.equal(contract.host.native_only, true);
  assert.equal(contract.toolchain.msvc_runtime, "static");
  assert.equal(contract.stages.native_smoke_required, true);
  assert.equal(contract.stages.native_smoke_host, "windows/x86_64");
  assert.equal(contract.stages.emulated_smoke_acceptable, false);
});

test("rejects native host, toolchain, smoke, and extra-field drift", async () => {
  const original = JSON.parse(await readFile(builderContractFile, "utf8"));
  for (const change of [
    (value) => {
      value.host.platform = "linux";
    },
    (value) => {
      value.host.native_only = false;
    },
    (value) => {
      value.stages.native_smoke_required = false;
    },
    (value) => {
      value.toolchain.msvc_runtime = "dynamic";
    },
    (value) => {
      value.stages.emulated_smoke_acceptable = true;
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
});

test("rejects non-absolute and missing native builder contracts", () => {
  assert.throws(
    () => readWindowsOcrBuilderContract("builder-contract.json"),
    /path is invalid/,
  );
  assert.throws(
    () =>
      readWindowsOcrBuilderContract(
        path.join(os.tmpdir(), "resume-ir-missing-windows-ocr-builder.json"),
      ),
    /contract is missing/,
  );
});

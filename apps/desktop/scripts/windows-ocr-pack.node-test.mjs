import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  readWindowsOcrSourceContract,
  validateWindowsOcrSourceContract,
} from "./windows-ocr-pack.mjs";

const contractFile = fileURLToPath(
  new URL(
    "../resources/ocr/x86_64-pc-windows-msvc/source-contract.json",
    import.meta.url,
  ),
);

test("accepts the pinned static Windows OCR source contract", () => {
  const contract = readWindowsOcrSourceContract(contractFile);
  assert.equal(
    contract.tesseract.source_commit,
    "6e1d56a847e697de07b38619356550e5cf4e8633",
  );
  assert.equal(
    contract.leptonica.source_commit,
    "13275a278eb55b5746e33f95fbf5a2c8f604b3ab",
  );
  assert.equal(
    contract.traineddata.source_commit,
    "65727574dfcd264acbb0c3e07860e4e9e9b22185",
  );
  assert.equal(contract.final_artifact.msvc_runtime, "static");
  assert.equal(
    contract.final_artifact.required_dependency_closure,
    "windows-system-dlls-only",
  );
  assert.deepEqual(contract.protocol.languages, ["eng", "chi_sim"]);
  assert.equal(contract.protocol.input_format, "ppm-p6-rgb8");
  assert.equal(contract.protocol.stdout_max_bytes, 4 * 1024 * 1024);
});

test("rejects source, build, protocol, dependency, and extra-field drift", async () => {
  const original = JSON.parse(await readFile(contractFile, "utf8"));
  const changes = [
    (value) => {
      value.tesseract.source_commit = "0".repeat(40);
    },
    (value) => {
      value.tesseract.cmake_arguments[1] = "-DBUILD_SHARED_LIBS=ON";
    },
    (value) => {
      value.leptonica.cmake_arguments[4] = "-DENABLE_PNG=ON";
    },
    (value) => {
      value.traineddata.files[0].sha256 = "0".repeat(64);
    },
    (value) => {
      value.tesseract.engine_config_file.sha256 = "0".repeat(64);
    },
    (value) => {
      value.protocol.input_format = "arbitrary-image";
    },
    (value) => {
      value.protocol.stdout_max_bytes += 1;
    },
    (value) => {
      value.final_artifact.forbidden_import_prefixes.pop();
    },
    (value) => {
      value.product_runtime_network_access = "enabled";
    },
    (value) => {
      value.extra = true;
    },
  ];
  for (const change of changes) {
    const candidate = structuredClone(original);
    change(candidate);
    assert.throws(
      () => validateWindowsOcrSourceContract(candidate),
      /Windows OCR source contract is invalid/,
    );
  }
});

test("rejects missing, malformed, oversized, and symlinked contract files", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-windows-ocr-contract-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  assert.throws(
    () => readWindowsOcrSourceContract(path.join(root, "missing.json")),
    /source contract is missing/,
  );
  const malformed = path.join(root, "malformed.json");
  await writeFile(malformed, "{");
  assert.throws(() => readWindowsOcrSourceContract(malformed), /not valid JSON/);
  const oversized = path.join(root, "oversized.json");
  await writeFile(oversized, "x".repeat(64 * 1024 + 1));
  assert.throws(() => readWindowsOcrSourceContract(oversized), /contract file is invalid/);
  const link = path.join(root, "contract-link.json");
  await symlink(contractFile, link);
  assert.throws(() => readWindowsOcrSourceContract(link), /contract file is invalid/);
});

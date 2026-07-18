import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  readWindowsPdfRendererSourceContract,
  validateWindowsPdfRendererSourceContract,
} from "./windows-pdf-renderer.mjs";

const contractFile = fileURLToPath(
  new URL(
    "../resources/pdf-renderer/x86_64-pc-windows-msvc/source-contract.json",
    import.meta.url,
  ),
);

test("accepts the pinned static PDFium renderer contract", () => {
  const contract = readWindowsPdfRendererSourceContract(contractFile);
  assert.equal(contract.rejected_platform_api.accepted, false);
  assert.equal(contract.rejected_platform_api.desktop_package_identity_required, true);
  assert.equal(contract.wrapper.cargo_feature, "windows-static-pdfium");
  assert.equal(contract.pdfium.source_commit, "91b9d569b34be4f38eed7b3c49b227356c3aadad");
  assert.ok(contract.pdfium.gn_arguments.includes("is_component_build=false"));
  assert.ok(contract.pdfium.gn_arguments.includes("pdf_is_complete_lib=true"));
});

test("rejects package-identity, dynamic-link, and resource-bound drift", async () => {
  const original = JSON.parse(await readFile(contractFile, "utf8"));
  const platformApi = structuredClone(original);
  platformApi.rejected_platform_api.accepted = true;
  assert.throws(
    () => validateWindowsPdfRendererSourceContract(platformApi),
    /source contract is invalid/,
  );
  const dynamic = structuredClone(original);
  dynamic.pdfium.gn_arguments[4] = "is_component_build=true";
  assert.throws(
    () => validateWindowsPdfRendererSourceContract(dynamic),
    /source contract is invalid/,
  );
  const unbounded = structuredClone(original);
  unbounded.protocol.page_max_pixels += 1;
  assert.throws(
    () => validateWindowsPdfRendererSourceContract(unbounded),
    /source contract is invalid/,
  );
});

test("rejects symlinked and malformed contract files", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-pdfium-contract-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const malformed = path.join(root, "malformed.json");
  await writeFile(malformed, "{");
  assert.throws(
    () => readWindowsPdfRendererSourceContract(malformed),
    /not valid JSON/,
  );
  const link = path.join(root, "contract-link.json");
  await symlink(contractFile, link);
  assert.throws(
    () => readWindowsPdfRendererSourceContract(link),
    /contract file is invalid/,
  );
});

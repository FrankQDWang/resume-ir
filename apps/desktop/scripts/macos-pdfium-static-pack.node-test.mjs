import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { readMacosPdfiumSourceContract } from "./macos-pdfium-static-pack.mjs";

const contractFile = fileURLToPath(
  new URL(
    "../resources/pdf-renderer/aarch64-apple-darwin/source-contract.json",
    import.meta.url,
  ),
);

test("standalone macOS PDFium does not depend on Chromium PGO profiles", () => {
  const contract = readMacosPdfiumSourceContract(contractFile);
  assert.ok(contract.pdfium.gn_arguments.includes("chrome_pgo_phase=0"));
  assert.ok(
    contract.pdfium.gn_arguments.includes("clang_use_unsafe_buffers_plugin=false"),
  );
  assert.ok(contract.pdfium.gn_arguments.includes("use_thin_lto=false"));
  assert.ok(!contract.pdfium.gn_arguments.includes("treat_warnings_as_errors=false"));
});

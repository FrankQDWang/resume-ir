import { fileURLToPath } from "node:url";
import path from "node:path";

import { verifyMacosPdfiumStaticPack } from "./macos-pdfium-static-pack.mjs";

const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));

verifyMacosPdfiumStaticPack({
  directory: path.join(repoRoot, ".cache", "resume-ir-macos-pdfium-static-pack"),
  sourceContract: path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "pdf-renderer",
    "aarch64-apple-darwin",
    "source-contract.json",
  ),
}).catch((error) => {
  console.error(`verify-macos-pdfium-static-pack: ${error.message}`);
  process.exitCode = 1;
});

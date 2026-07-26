import { constants } from "node:fs";
import {
  access,
  lstat,
  mkdtemp,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import { assemble } from "./assemble-macos-ocr-pack.mjs";

const TARGET = "aarch64-apple-darwin";
const ENGINE_VERSION = "5.5.2";

function fail(message) {
  throw new Error(`macOS OCR pack build blocked: ${message}`);
}

async function regularFile(file) {
  try {
    const canonical = await realpath(file);
    const metadata = await lstat(canonical);
    if (metadata.isFile() && !metadata.isSymbolicLink() && metadata.size > 0) {
      return canonical;
    }
  } catch {
    // Try the next bounded candidate.
  }
  return undefined;
}

async function executable(file) {
  const candidate = await regularFile(file);
  if (!candidate) return undefined;
  try {
    await access(candidate, constants.X_OK);
    return candidate;
  } catch {
    return undefined;
  }
}

async function commandFromPath(name) {
  for (const directory of (process.env.PATH ?? "").split(path.delimiter)) {
    if (!directory || !path.isAbsolute(directory)) continue;
    const candidate = await executable(path.join(directory, name));
    if (candidate) return candidate;
  }
  return undefined;
}

function commandOutput(command, args) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
    timeout: 10_000,
  });
  if (result.error || result.status !== 0) fail("native dependency probe failed");
  return `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
}

async function resolveTesseract() {
  const configured = process.env.RESUME_IR_TESSERACT_COMMAND;
  const command = configured
    ? await executable(configured)
    : await commandFromPath("tesseract");
  if (!command) fail("Tesseract executable is unavailable");
  const version = commandOutput(command, ["--version"]).match(/tesseract\s+(\d+\.\d+\.\d+)/i)?.[1];
  if (version !== ENGINE_VERSION) fail("Tesseract version does not match the reviewed contract");
  return command;
}

async function resolveTessdataRoot(tesseract) {
  const configured = process.env.RESUME_IR_TESSDATA_ROOT;
  const candidates = [
    configured,
    path.resolve(path.dirname(tesseract), "..", "share", "tessdata"),
    "/opt/homebrew/share/tessdata",
    "/usr/local/share/tessdata",
  ].filter(Boolean);
  for (const candidate of candidates) {
    if (!path.isAbsolute(candidate)) continue;
    const eng = await regularFile(path.join(candidate, "eng.traineddata"));
    const chiSim = await regularFile(path.join(candidate, "chi_sim.traineddata"));
    const tsv = await regularFile(path.join(candidate, "configs", "tsv"));
    if (eng && chiSim && tsv) return { root: await realpath(candidate), eng, chiSim };
  }
  fail("reviewed eng and chi_sim tessdata are unavailable");
}

async function main() {
  if (process.platform !== "darwin" || process.arch !== "arm64") {
    fail("native macOS arm64 host is required");
  }
  const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
  const tesseract = await resolveTesseract();
  const tessdata = await resolveTessdataRoot(tesseract);
  const temporary = await mkdtemp(path.join(os.tmpdir(), "resume-ir-macos-ocr-"));
  try {
    const sourceManifest = path.join(temporary, "source-manifest.json");
    const manifest = {
      schema_version: "resume-ir.ocr-runtime-manifest.v1",
      runtime_pack_id: "local-tesseract-5.5.2-eng-chi-sim",
      components: [
        {
          id: "tesseract",
          kind: "ocr-engine",
          version: ENGINE_VERSION,
          artifact: { path: tesseract },
          license: { id: "Apache-2.0", reviewed: true },
        },
      ],
      languages: [
        {
          id: "eng",
          artifact: { path: tessdata.eng },
          license: { id: "Apache-2.0", reviewed: true },
        },
        {
          id: "chi_sim",
          artifact: { path: tessdata.chiSim },
          license: { id: "Apache-2.0", reviewed: true },
        },
      ],
    };
    await writeFile(sourceManifest, `${JSON.stringify(manifest, null, 2)}\n`, {
      mode: 0o600,
    });
    const result = await assemble({
      expectedManifest: path.join(
        repoRoot,
        "apps",
        "desktop",
        "resources",
        "ocr",
        TARGET,
        "runtime-pack.json",
      ),
      manifest: sourceManifest,
      out: path.join(repoRoot, ".cache", "resume-ir-macos-ocr-runtime-pack"),
    });
    console.log("macOS OCR pack: built");
    console.log(`target: ${TARGET}`);
    console.log(`files: ${result.fileCount}`);
    console.log(`engine libraries: ${result.libraryCount}`);
    console.log("paths: <redacted>");
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

main().catch((error) => {
  console.error(
    error instanceof Error ? error.message : "macOS OCR pack build blocked",
  );
  process.exitCode = 1;
});

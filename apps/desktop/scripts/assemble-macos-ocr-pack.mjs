import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  readFile,
  realpath,
  readdir,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

import {
  MACOS_SYSTEM_TOOLS,
  runClosedSystemTool,
} from "./macos-system-tools.mjs";

const TARGET_TRIPLE = "aarch64-apple-darwin";
const LICENSE_IDS = [
  "0BSD",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "CC0-1.0",
  "IJG",
  "MIT",
  "Zlib",
  "libtiff",
];
const LICENSE_SOURCES = [
  ...LICENSE_IDS.map((id) => ({
    file: `${id}.txt`,
    url: `https://spdx.org/licenses/${encodeURIComponent(id)}.txt`,
  })),
  {
    file: "libpng-2.0.txt",
    url: "https://raw.githubusercontent.com/pnggroup/libpng/v1.6.58/LICENSE",
  },
];
const MAX_LICENSE_BYTES = 128 * 1024;
const SYSTEM_PREFIXES = ["/System/Library/", "/usr/lib/"];

function fail(message) {
  throw new Error(`OCR pack assembly blocked: ${message}`);
}

function parseArguments(args) {
  const values = new Map();
  let reviewed = false;
  for (let index = 0; index < args.length; index += 1) {
    const key = args[index];
    if (key === "--reviewed") {
      reviewed = true;
      continue;
    }
    if (!["--manifest", "--out", "--expected-manifest"].includes(key)) {
      fail("invalid argument");
    }
    const value = args[index + 1];
    if (!value || values.has(key)) fail("invalid argument");
    values.set(key, value);
    index += 1;
  }
  if (!reviewed) fail("license review is incomplete");
  const result = {
    manifest: values.get("--manifest"),
    out: values.get("--out"),
    expectedManifest: values.get("--expected-manifest"),
  };
  if (Object.values(result).some((value) => !value || !path.isAbsolute(value))) {
    fail("absolute input and output paths are required");
  }
  return result;
}

function defaultRunner(program, args) {
  return runClosedSystemTool(program, args, {
    encoding: "utf8",
    maxBuffer: 4 * 1024 * 1024,
    timeout: 30_000,
  });
}

function run(program, args, runner = defaultRunner) {
  const result = runner(program, args);
  if (result.error || result.status !== 0) fail("native tool failed");
  return result.stdout;
}

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

async function checkedSourceFile(file) {
  if (typeof file !== "string" || file.length === 0 || file.length > 4096) {
    fail("reviewed artifact path is invalid");
  }
  let canonical;
  try {
    canonical = await realpath(file);
  } catch {
    fail("reviewed artifact is unavailable");
  }
  const metadata = await stat(canonical);
  if (!metadata.isFile() || metadata.size === 0) fail("reviewed artifact is invalid");
  return canonical;
}

function validateReviewedManifest(manifest) {
  if (
    !manifest ||
    manifest.schema_version !== "resume-ir.ocr-runtime-manifest.v1" ||
    manifest.runtime_pack_id !== "local-tesseract-poppler-eng-chi-sim" ||
    !Array.isArray(manifest.components) ||
    !Array.isArray(manifest.languages)
  ) {
    fail("reviewed manifest contract is invalid");
  }
  const engine = manifest.components.find(
    (component) => component?.id === "tesseract" && component.kind === "ocr-engine",
  );
  if (
    !engine ||
    engine.version !== "5.5.2" ||
    engine.license?.id !== "Apache-2.0" ||
    engine.license?.reviewed !== true ||
    !engine.artifact?.path
  ) {
    fail("reviewed Tesseract contract is invalid");
  }
  const languageById = new Map(manifest.languages.map((entry) => [entry?.id, entry]));
  for (const id of ["eng", "chi_sim"]) {
    const language = languageById.get(id);
    if (
      language?.license?.id !== "Apache-2.0" ||
      language.license.reviewed !== true ||
      !language.artifact?.path
    ) {
      fail("reviewed language contract is invalid");
    }
  }
  if (languageById.size !== 2) fail("reviewed language set is not exact");
  return { engine, languageById };
}

export function machoDependencies(file, runner = defaultRunner) {
  return run(MACOS_SYSTEM_TOOLS.otool, ["-L", file], runner)
    .split("\n")
    .slice(1)
    .map((line) => line.trim().match(/^(.*?) \(compatibility/)?.[1])
    .filter(Boolean);
}

function machoId(file, runner = defaultRunner) {
  const lines = run(MACOS_SYSTEM_TOOLS.otool, ["-D", file], runner)
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  return lines.length > 1 ? lines[1] : undefined;
}

function isSystemDependency(dependency) {
  return SYSTEM_PREFIXES.some((prefix) => dependency.startsWith(prefix));
}

async function collectEngineClosure(engine, runner = defaultRunner) {
  const queue = [engine];
  const sources = new Set();
  const dependencies = new Map();
  while (queue.length > 0) {
    const source = await realpath(queue.pop());
    if (sources.has(source)) continue;
    sources.add(source);
    const id = machoId(source, runner);
    const edges = [];
    for (const edge of machoDependencies(source, runner)) {
      if (isSystemDependency(edge) || edge === id) {
        edges.push({ original: edge });
        continue;
      }
      let candidate;
      if (path.isAbsolute(edge)) candidate = edge;
      else if (edge.startsWith("@rpath/")) {
        candidate = path.join(path.dirname(source), edge.slice("@rpath/".length));
      } else if (edge.startsWith("@loader_path/")) {
        candidate = path.join(path.dirname(source), edge.slice("@loader_path/".length));
      } else {
        fail("engine has an unresolved loader dependency");
      }
      const target = await checkedSourceFile(candidate);
      edges.push({ original: edge, target });
      queue.push(target);
    }
    dependencies.set(source, { edges, id });
  }
  return { dependencies, sources: [...sources] };
}

async function fetchLicenseText(source) {
  const response = await fetch(source.url, {
    redirect: "follow",
    signal: AbortSignal.timeout(20_000),
  });
  if (!response.ok) fail("license text is unavailable");
  const bytes = Buffer.from(await response.arrayBuffer());
  if (
    bytes.length < 32 ||
    bytes.length > MAX_LICENSE_BYTES ||
    bytes.includes(Buffer.from("<html", "utf8"))
  ) {
    fail("license text response is invalid");
  }
  return bytes;
}

export async function stageEngine({
  dependencies,
  engine,
  root,
  runner = defaultRunner,
  sources,
}) {
  const libDirectory = path.join(root, "lib");
  await mkdir(libDirectory, { recursive: true });
  const destinations = new Map();
  const names = new Map();
  for (const source of sources) {
    const destination =
      source === engine
        ? path.join(root, "tesseract")
        : path.join(libDirectory, path.basename(source));
    const previous = names.get(path.basename(destination));
    if (previous && previous !== source) fail("engine library basename collision");
    names.set(path.basename(destination), source);
    destinations.set(source, destination);
    await copyFile(source, destination);
    await chmod(destination, 0o755);
  }
  for (const source of sources) {
    const destination = destinations.get(source);
    const { edges, id } = dependencies.get(source);
    for (const edge of edges) {
      if (!edge.target) continue;
      const dependencyDestination = destinations.get(edge.target);
      if (!dependencyDestination) fail("engine closure is incomplete");
      const replacement =
        source === engine
          ? `@loader_path/lib/${path.basename(dependencyDestination)}`
          : `@loader_path/${path.basename(dependencyDestination)}`;
      run(
        MACOS_SYSTEM_TOOLS.installNameTool,
        ["-change", edge.original, replacement, destination],
        runner,
      );
    }
    if (source !== engine) {
      run(
        MACOS_SYSTEM_TOOLS.installNameTool,
        ["-id", `@loader_path/${path.basename(destination)}`, destination],
        runner,
      );
    }
    run(
      MACOS_SYSTEM_TOOLS.codesign,
      ["--force", "--sign", "-", destination],
      runner,
    );
  }
}

async function listFiles(root, relative = "") {
  const directory = path.join(root, relative);
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const child = path.join(relative, entry.name);
    if (entry.isSymbolicLink()) fail("assembled pack contains a symlink");
    if (entry.isDirectory()) files.push(...(await listFiles(root, child)));
    else if (entry.isFile()) files.push(child);
    else fail("assembled pack contains an unsupported entry");
  }
  return files.sort();
}

function roleFor(file) {
  if (file === "tesseract") return "engine_binary";
  if (file.startsWith("lib/")) return "engine_library";
  if (file === "tessdata/eng.traineddata") return "language_eng";
  if (file === "tessdata/chi_sim.traineddata") return "language_chi_sim";
  if (file === "tessdata/configs/tsv") return "engine_config";
  if (file.startsWith("LICENSES/")) return "license_text";
  if (file === "THIRD-PARTY-NOTICES.json") return "third_party_notice";
  fail("assembled pack contains an undeclared file");
}

async function assemble({ expectedManifest, manifest, out, runner = defaultRunner }) {
  const sourceManifest = validateReviewedManifest(
    JSON.parse(await readFile(manifest, "utf8")),
  );
  const engine = await checkedSourceFile(sourceManifest.engine.artifact.path);
  const closure = await collectEngineClosure(engine, runner);
  const parent = path.dirname(out);
  const temporary = path.join(parent, `${path.basename(out)}.tmp-${process.pid}-${Date.now()}`);
  const backup = path.join(parent, `${path.basename(out)}.old-${process.pid}-${Date.now()}`);
  const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
  const notices = path.join(
    repoRoot,
    "apps/desktop/resources/ocr/aarch64-apple-darwin/third-party-components.json",
  );
  await mkdir(parent, { recursive: true });
  await rm(temporary, { recursive: true, force: true });
  await mkdir(temporary, { mode: 0o700 });
  try {
    await stageEngine({ ...closure, engine, root: temporary, runner });
    const tessdata = path.join(temporary, "tessdata");
    await mkdir(tessdata);
    const tessdataSources = [];
    for (const id of ["eng", "chi_sim"]) {
      const source = await checkedSourceFile(sourceManifest.languageById.get(id).artifact.path);
      const sourceRoot = path.dirname(source);
      if (!tessdataSources.includes(sourceRoot)) tessdataSources.push(sourceRoot);
      await copyFile(source, path.join(tessdata, `${id}.traineddata`));
    }
    const configs = path.join(tessdata, "configs");
    await mkdir(configs);
    let tsvConfig;
    for (const sourceRoot of tessdataSources) {
      try {
        tsvConfig = await checkedSourceFile(path.join(sourceRoot, "configs", "tsv"));
        break;
      } catch {
        // Try the next reviewed tessdata root without exposing local paths.
      }
    }
    if (!tsvConfig) fail("reviewed Tesseract TSV config is unavailable");
    await copyFile(
      tsvConfig,
      path.join(configs, "tsv"),
    );
    await copyFile(notices, path.join(temporary, "THIRD-PARTY-NOTICES.json"));
    const licenses = path.join(temporary, "LICENSES");
    await mkdir(licenses);
    const licenseBodies = await Promise.all(LICENSE_SOURCES.map(fetchLicenseText));
    for (let index = 0; index < LICENSE_SOURCES.length; index += 1) {
      await writeFile(path.join(licenses, LICENSE_SOURCES[index].file), licenseBodies[index]);
    }
    const files = [];
    for (const file of await listFiles(temporary)) {
      const metadata = await lstat(path.join(temporary, file));
      files.push({
        role: roleFor(file),
        file,
        bytes: metadata.size,
        sha256: await sha256(path.join(temporary, file)),
        executable: file === "tesseract",
      });
    }
    const runtimeManifest = {
      schema_version: "resume-ir.desktop-ocr-runtime-pack.v1",
      runtime_pack_id: "tesseract-5.5.2-tessdata-fast-4.1.0-macos-arm64-r1",
      target_triple: TARGET_TRIPLE,
      engine: "tesseract",
      engine_version: "5.5.2",
      renderer: "macos-pdfkit-coregraphics",
      languages: ["eng", "chi_sim"],
      network_access: "disabled",
      license_reviewed: true,
      third_party_notice: "THIRD-PARTY-NOTICES.json",
      files,
    };
    const manifestBody = `${JSON.stringify(runtimeManifest, null, 2)}\n`;
    await writeFile(path.join(temporary, "runtime-pack.json"), manifestBody);
    await mkdir(path.dirname(expectedManifest), { recursive: true });
    await writeFile(expectedManifest, manifestBody);
    let previous = false;
    try {
      await rename(out, backup);
      previous = true;
    } catch (error) {
      if (!error || error.code !== "ENOENT") throw error;
    }
    try {
      await rename(temporary, out);
    } catch (error) {
      if (previous) await rename(backup, out);
      throw error;
    }
    await rm(backup, { recursive: true, force: true });
    return { fileCount: files.length + 1, libraryCount: closure.sources.length - 1 };
  } finally {
    await rm(temporary, { recursive: true, force: true });
    await rm(backup, { recursive: true, force: true });
  }
}

const moduleFile = fileURLToPath(import.meta.url);
if (process.argv[1] && path.resolve(process.argv[1]) === moduleFile) {
  const options = parseArguments(process.argv.slice(2));
  assemble(options)
    .then(({ fileCount, libraryCount }) => {
      console.log("macOS OCR pack: assembled");
      console.log(`target: ${TARGET_TRIPLE}`);
      console.log(`files: ${fileCount}`);
      console.log(`engine libraries: ${libraryCount}`);
      console.log("paths: <redacted>");
    })
    .catch((error) => {
      console.error(
        error instanceof Error ? error.message : "OCR pack assembly blocked",
      );
      process.exitCode = 1;
    });
}

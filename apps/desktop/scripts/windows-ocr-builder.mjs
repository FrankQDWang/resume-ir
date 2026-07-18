import { createHash } from "node:crypto";
import { lstat, readFile, readdir } from "node:fs/promises";
import path from "node:path";

const SCHEMA = "resume-ir.windows-ocr-builder-contract.v1";
const TARGET = "x86_64-pc-windows-msvc";
const BASE_IMAGE =
  "rust:1.81.0-slim-bookworm@sha256:f9fb6bdb0483de4ade93b262a3f6cf8c2985fca1d34784914bbcabd5a34d3197";
const XWIN_SHA = "31e1033f30608ba6b821d17f1461042bd54c23424813c9b4e9ae15b6d32fa4cd";
const RECIPE_FILES = [
  {
    role: "container_recipe",
    file: "Dockerfile",
    bytes: 2978,
    sha256: "e7d7ae724752e3a3e488a9fb0c2f17f6600af0e2d8424997baf77849c9ca7436",
  },
  {
    role: "compile_script",
    file: "build.sh",
    bytes: 4444,
    sha256: "abb413e5734797f028125d7032721bc6a7b4add4b0d174117364d425f18f7bf7",
  },
  {
    role: "native_smoke_script",
    file: "smoke.sh",
    bytes: 1127,
    sha256: "19021c758efecf957298695588c8695d4a11d057825b408363d5b5f7f24d555e",
  },
  {
    role: "cmake_toolchain",
    file: "toolchain.cmake",
    bytes: 1317,
    sha256: "d055d8681df7940b4b3f068948063589da9c505bb13f7e206552991fcf18e064",
  },
];

function exactKeys(value, keys) {
  return (
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...keys].sort())
  );
}

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function validateWindowsOcrBuilderContract(contract) {
  const container = contract?.container;
  const xwin = contract?.xwin;
  const toolchain = contract?.toolchain;
  const stages = contract?.stages;
  const smoke = contract?.smoke;
  if (
    !exactKeys(contract, [
      "schema_version",
      "target_triple",
      "container",
      "xwin",
      "toolchain",
      "stages",
      "smoke",
      "recipe_files",
    ]) ||
    contract.schema_version !== SCHEMA ||
    contract.target_triple !== TARGET ||
    !exactKeys(container, ["platform", "base_image"]) ||
    container.platform !== "linux/amd64" ||
    container.base_image !== BASE_IMAGE ||
    !exactKeys(xwin, [
      "version",
      "archive_sha256",
      "sdk_version",
      "crt_version",
      "architecture",
      "license_acceptance",
      "http_retry",
    ]) ||
    xwin.version !== "0.9.0" ||
    xwin.archive_sha256 !== XWIN_SHA ||
    xwin.sdk_version !== "10.0.26100" ||
    xwin.crt_version !== "14.44.17.14" ||
    xwin.architecture !== "x86_64" ||
    xwin.license_acceptance !== "explicit" ||
    xwin.http_retry !== 3 ||
    !exactKeys(toolchain, [
      "clang_cl_version",
      "cmake_version",
      "ninja_version",
      "wine_version",
      "msvc_runtime",
      "lto",
      "cross_compile_probe_execution",
    ]) ||
    toolchain.clang_cl_version !== "19.1.7" ||
    toolchain.cmake_version !== "3.25.1" ||
    toolchain.ninja_version !== "1.11.1" ||
    toolchain.wine_version !== "11.10" ||
    toolchain.msvc_runtime !== "static" ||
    toolchain.lto !== true ||
    toolchain.cross_compile_probe_execution !== "disabled" ||
    !exactKeys(stages, [
      "compile_output",
      "validated_output",
      "native_smoke_required",
      "native_smoke_host",
      "emulated_smoke_acceptable",
    ]) ||
    stages.compile_output !== "compile-output" ||
    stages.validated_output !== "validated-output" ||
    stages.native_smoke_required !== true ||
    stages.native_smoke_host !== "linux/amd64-native" ||
    stages.emulated_smoke_acceptable !== false ||
    !exactKeys(smoke, [
      "input_format",
      "input_max_bytes",
      "languages",
      "output_format",
      "output_max_bytes",
      "timeout_seconds",
    ]) ||
    smoke.input_format !== "ppm-p6-rgb8" ||
    smoke.input_max_bytes !== 32 * 1024 * 1024 ||
    !same(smoke.languages, ["eng", "chi_sim"]) ||
    smoke.output_format !== "tesseract-tsv" ||
    smoke.output_max_bytes !== 4 * 1024 * 1024 ||
    smoke.timeout_seconds !== 30 ||
    !same(contract.recipe_files, RECIPE_FILES)
  ) {
    throw new Error("Windows OCR builder contract is invalid");
  }
  return contract;
}

async function boundedRegularFile(file, maxBytes) {
  let metadata;
  try {
    metadata = await lstat(file);
  } catch {
    throw new Error("Windows OCR builder recipe is missing");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > maxBytes
  ) {
    throw new Error("Windows OCR builder recipe file is invalid");
  }
  return metadata;
}

export async function readWindowsOcrBuilderContract(file) {
  if (typeof file !== "string" || !path.isAbsolute(file)) {
    throw new Error("Windows OCR builder contract path is invalid");
  }
  await boundedRegularFile(file, 64 * 1024);
  let contract;
  try {
    contract = validateWindowsOcrBuilderContract(
      JSON.parse(await readFile(file, "utf8")),
    );
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("Windows OCR builder contract is not valid JSON");
    }
    throw error;
  }
  const root = path.dirname(file);
  const entries = await readdir(root, { withFileTypes: true });
  if (
    entries.some((entry) => !entry.isFile()) ||
    !same(
      entries.map(({ name }) => name).sort(),
      ["builder-contract.json", ...RECIPE_FILES.map(({ file: name }) => name)].sort(),
    )
  ) {
    throw new Error("Windows OCR builder recipe root is invalid");
  }
  for (const identity of RECIPE_FILES) {
    const recipe = path.join(root, identity.file);
    const metadata = await boundedRegularFile(recipe, 64 * 1024);
    const body = await readFile(recipe);
    const sha256 = createHash("sha256").update(body).digest("hex");
    if (metadata.size !== identity.bytes || sha256 !== identity.sha256) {
      throw new Error("Windows OCR builder recipe identity is invalid");
    }
  }
  return contract;
}

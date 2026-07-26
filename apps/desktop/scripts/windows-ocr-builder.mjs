import { lstatSync, readFileSync } from "node:fs";
import path from "node:path";

const SCHEMA = "resume-ir.windows-ocr-builder-contract.v2";
const TARGET = "x86_64-pc-windows-msvc";

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
  const host = contract?.host;
  const toolchain = contract?.toolchain;
  const stages = contract?.stages;
  const network = contract?.network;
  const smoke = contract?.smoke;
  if (
    !exactKeys(contract, [
      "schema_version",
      "target_triple",
      "host",
      "toolchain",
      "stages",
      "network",
      "smoke",
    ]) ||
    contract.schema_version !== SCHEMA ||
    contract.target_triple !== TARGET ||
    !exactKeys(host, ["platform", "architecture", "native_only"]) ||
    host.platform !== "windows" ||
    host.architecture !== "x86_64" ||
    host.native_only !== true ||
    !exactKeys(toolchain, [
      "visual_studio_minimum",
      "cmake_minimum",
      "ninja_minimum",
      "msvc_runtime",
      "lto",
    ]) ||
    toolchain.visual_studio_minimum !== "17.13" ||
    toolchain.cmake_minimum !== "3.25" ||
    toolchain.ninja_minimum !== "1.11" ||
    toolchain.msvc_runtime !== "static" ||
    toolchain.lto !== true ||
    !exactKeys(stages, [
      "source_checkout",
      "compile_output",
      "validated_output",
      "native_smoke_required",
      "native_smoke_host",
      "emulated_smoke_acceptable",
    ]) ||
    stages.source_checkout !== "exact-clean-commit" ||
    stages.compile_output !== "compile-output" ||
    stages.validated_output !== "validated-output" ||
    stages.native_smoke_required !== true ||
    stages.native_smoke_host !== "windows/x86_64" ||
    stages.emulated_smoke_acceptable !== false ||
    !exactKeys(network, ["source_build_only", "packaged_runtime_access"]) ||
    network.source_build_only !== true ||
    network.packaged_runtime_access !== "disabled" ||
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
    smoke.timeout_seconds !== 30
  ) {
    throw new Error("Windows OCR builder contract is invalid");
  }
  return contract;
}

export function readWindowsOcrBuilderContract(file) {
  if (typeof file !== "string" || !path.isAbsolute(file)) {
    throw new Error("Windows OCR builder contract path is invalid");
  }
  let metadata;
  try {
    metadata = lstatSync(file);
  } catch {
    throw new Error("Windows OCR builder contract is missing");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > 16 * 1024
  ) {
    throw new Error("Windows OCR builder contract file is invalid");
  }
  try {
    return validateWindowsOcrBuilderContract(
      JSON.parse(readFileSync(file, "utf8")),
    );
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("Windows OCR builder contract is not valid JSON");
    }
    throw error;
  }
}

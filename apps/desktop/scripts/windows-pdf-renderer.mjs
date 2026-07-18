import { lstatSync, readFileSync } from "node:fs";
import path from "node:path";

const TARGET = "x86_64-pc-windows-msvc";
const CONTRACT_SCHEMA = "resume-ir.windows-pdf-renderer-source-contract.v1";
const PDFIUM_SOURCE = "https://pdfium.googlesource.com/pdfium.git";
const PDFIUM_COMMIT = "91b9d569b34be4f38eed7b3c49b227356c3aadad";
const BUILD_REVISION = "613f5c13bccbc15bd7ce8da9acb13ac06459f8cb";
const GN_ARGUMENTS = [
  'target_os="win"',
  'target_cpu="x64"',
  "is_debug=false",
  "is_official_build=true",
  "is_component_build=false",
  "pdf_is_standalone=true",
  "pdf_is_complete_lib=true",
  "pdf_enable_v8=false",
  "pdf_enable_xfa=false",
  "pdf_use_skia=false",
  "pdf_use_partition_alloc=false",
  "pdf_bundle_freetype=true",
  "use_custom_libcxx=false",
  "symbol_level=0",
];
const BUILD_TARGETS = ["pdfium", "pdfium_unittests", "pdfium_embeddertests"];
const ENVIRONMENT = [
  "RESUME_IR_PDF_RENDER_INPUT_PATH",
  "RESUME_IR_PDF_RENDER_PAGE_NO",
  "RESUME_IR_PDF_RENDER_DPI",
];
const FORBIDDEN_IMPORT_PREFIXES = [
  "MSVCP",
  "VCRUNTIME",
  "CONCRT",
  "UCRTBASE",
  "API-MS-WIN-CRT-",
];

function exactKeys(value, keys) {
  return (
    value &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value).sort()) === JSON.stringify([...keys].sort())
  );
}

function sameArray(left, right) {
  return Array.isArray(left) && JSON.stringify(left) === JSON.stringify(right);
}

export function validateWindowsPdfRendererSourceContract(contract) {
  const rejected = contract?.rejected_platform_api;
  const wrapper = contract?.wrapper;
  const protocol = contract?.protocol;
  const exitCodes = protocol?.exit_codes;
  const pdfium = contract?.pdfium;
  const license = pdfium?.source_license_file;
  if (
    !exactKeys(contract, [
      "schema_version",
      "target_triple",
      "product_runtime_network_access",
      "rejected_platform_api",
      "wrapper",
      "protocol",
      "pdfium",
    ]) ||
    contract.schema_version !== CONTRACT_SCHEMA ||
    contract.target_triple !== TARGET ||
    contract.product_runtime_network_access !== "disabled" ||
    !exactKeys(rejected, [
      "name",
      "official_support_document",
      "desktop_package_identity_required",
      "target_installer_identity",
      "accepted",
    ]) ||
    rejected.name !== "Windows.Data.Pdf" ||
    rejected.official_support_document !==
      "https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/winrt-api-desktop-app-support" ||
    rejected.desktop_package_identity_required !== true ||
    rejected.target_installer_identity !== "tauri-nsis-unpackaged-win32" ||
    rejected.accepted !== false ||
    !exactKeys(wrapper, [
      "crate",
      "dependency",
      "dependency_version",
      "dependency_feature",
      "cargo_feature",
      "link_mode",
      "workspace_unsafe_code_allowed",
    ]) ||
    wrapper.crate !== "resume-pdf-render-runtime" ||
    wrapper.dependency !== "pdfium-render" ||
    wrapper.dependency_version !== "0.9.3" ||
    wrapper.dependency_feature !== "pdfium_7881" ||
    wrapper.cargo_feature !== "windows-static-pdfium" ||
    wrapper.link_mode !== "complete-static-library" ||
    wrapper.workspace_unsafe_code_allowed !== false ||
    !exactKeys(protocol, [
      "arguments",
      "environment",
      "input_max_bytes",
      "path_max_utf16_units",
      "page_min",
      "page_max",
      "dpi_min",
      "dpi_max",
      "dimension_max_pixels",
      "page_max_pixels",
      "stdout_max_bytes",
      "stdout_format",
      "stderr",
      "exit_codes",
    ]) ||
    !sameArray(protocol.arguments, []) ||
    !sameArray(protocol.environment, ENVIRONMENT) ||
    protocol.input_max_bytes !== 64 * 1024 * 1024 ||
    protocol.path_max_utf16_units !== 32_767 ||
    protocol.page_min !== 1 ||
    protocol.page_max !== 512 ||
    protocol.dpi_min !== 72 ||
    protocol.dpi_max !== 600 ||
    protocol.dimension_max_pixels !== 10_000 ||
    protocol.page_max_pixels !== 10_000_000 ||
    protocol.stdout_max_bytes !== 32 * 1024 * 1024 ||
    protocol.stdout_format !== "ppm-p6-rgb8" ||
    protocol.stderr !== "bounded-generic-only" ||
    !exactKeys(exitCodes, ["success", "unavailable", "invalid_request", "resource_limit"]) ||
    exitCodes.success !== 0 ||
    exitCodes.unavailable !== 1 ||
    exitCodes.invalid_request !== 2 ||
    exitCodes.resource_limit !== 3 ||
    !exactKeys(pdfium, [
      "release",
      "source_repository",
      "source_commit",
      "source_build_dependency_revision",
      "license",
      "source_license_file",
      "build_provenance_schema",
      "gn_arguments",
      "build_targets",
      "static_library_file",
      "final_binary_file",
      "required_final_dependency_closure",
      "forbidden_final_import_prefixes",
    ]) ||
    pdfium.release !== "chromium/7881" ||
    pdfium.source_repository !== PDFIUM_SOURCE ||
    pdfium.source_commit !== PDFIUM_COMMIT ||
    pdfium.source_build_dependency_revision !== BUILD_REVISION ||
    pdfium.license !== "LicenseRef-PDFium-Root-LICENSE" ||
    !exactKeys(license, ["file", "bytes", "sha256"]) ||
    license.file !== "LICENSE" ||
    license.bytes !== 12_896 ||
    license.sha256 !== "1fe9dea718fbd75cf149adaf4d8a22a4335604d964ddb76d1b45383dec8668c9" ||
    pdfium.build_provenance_schema !== "resume-ir.pdfium-windows-build-provenance.v1" ||
    !sameArray(pdfium.gn_arguments, GN_ARGUMENTS) ||
    !sameArray(pdfium.build_targets, BUILD_TARGETS) ||
    pdfium.static_library_file !== "pdfium.lib" ||
    pdfium.final_binary_file !== "resume-pdf-render-runtime.exe" ||
    pdfium.required_final_dependency_closure !== "windows-system-dlls-only" ||
    !sameArray(pdfium.forbidden_final_import_prefixes, FORBIDDEN_IMPORT_PREFIXES)
  ) {
    throw new Error("Windows PDF renderer source contract is invalid");
  }
  return contract;
}

export function readWindowsPdfRendererSourceContract(file) {
  if (!path.isAbsolute(file)) {
    throw new Error("Windows PDF renderer source contract path is invalid");
  }
  let metadata;
  try {
    metadata = lstatSync(file);
  } catch {
    throw new Error("Windows PDF renderer source contract is missing");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > 64 * 1024
  ) {
    throw new Error("Windows PDF renderer source contract file is invalid");
  }
  try {
    return validateWindowsPdfRendererSourceContract(JSON.parse(readFileSync(file, "utf8")));
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("Windows PDF renderer source contract is not valid JSON");
    }
    throw error;
  }
}

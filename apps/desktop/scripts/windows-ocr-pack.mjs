import { lstatSync, readFileSync } from "node:fs";
import path from "node:path";

const TARGET = "x86_64-pc-windows-msvc";
const SCHEMA = "resume-ir.windows-ocr-source-contract.v1";
const PACK_SCHEMA = "resume-ir.desktop-ocr-runtime-pack.v1";
const PACK_ID = "tesseract-5.5.2-tessdata-fast-4.1.0-windows-x64-static-r1";
const TESSERACT_COMMIT = "6e1d56a847e697de07b38619356550e5cf4e8633";
const LEPTONICA_COMMIT = "13275a278eb55b5746e33f95fbf5a2c8f604b3ab";
const TESSDATA_COMMIT = "65727574dfcd264acbb0c3e07860e4e9e9b22185";
const SHA256 = /^[a-f0-9]{64}$/;
const TESSERACT_ARGUMENTS = [
  "-DCMAKE_POLICY_DEFAULT_CMP0091=NEW",
  "-DCMAKE_BUILD_TYPE=Release",
  "-DBUILD_SHARED_LIBS=OFF",
  "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded",
  "-DSW_BUILD=OFF",
  "-DBUILD_TRAINING_TOOLS=OFF",
  "-DGRAPHICS_DISABLED=ON",
  "-DOPENMP_BUILD=OFF",
  "-DDISABLE_TIFF=ON",
  "-DLEPT_TIFF_RESULT=1",
  "-DLEPT_TIFF_COMPILE_SUCCESS=TRUE",
  "-DDISABLE_ARCHIVE=ON",
  "-DDISABLE_CURL=ON",
  "-DENABLE_LTO=ON",
  "-DFAST_FLOAT=ON",
];
const LEPTONICA_ARGUMENTS = [
  "-DCMAKE_POLICY_DEFAULT_CMP0091=NEW",
  "-DCMAKE_BUILD_TYPE=Release",
  "-DBUILD_SHARED_LIBS=OFF",
  "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded",
  "-DSW_BUILD=OFF",
  "-DENABLE_PNG=OFF",
  "-DBUILD_PROG=OFF",
  "-DENABLE_ZLIB=OFF",
  "-DENABLE_GIF=OFF",
  "-DENABLE_JPEG=OFF",
  "-DENABLE_TIFF=OFF",
  "-DENABLE_WEBP=OFF",
  "-DENABLE_OPENJPEG=OFF",
];
const PROTOCOL_ARGUMENTS = [
  "<bounded-absolute-ppm-path>",
  "stdout",
  "--psm",
  "6",
  "-l",
  "eng+chi_sim",
  "tsv",
];
const FORBIDDEN_IMPORTS = [
  "MSVCP",
  "VCRUNTIME",
  "CONCRT",
  "UCRTBASE",
  "API-MS-WIN-CRT-",
  "LIBGOMP",
  "VCOMP",
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

function fileIdentity(value, { file, bytes, sha256, role }) {
  const keys =
    role === undefined
      ? ["file", "bytes", "sha256"]
      : ["role", "file", "bytes", "sha256"];
  return (
    exactKeys(value, keys) &&
    (role === undefined || value.role === role) &&
    value.file === file &&
    value.bytes === bytes &&
    value.sha256 === sha256 &&
    SHA256.test(value.sha256)
  );
}

export function validateWindowsOcrSourceContract(contract) {
  const protocol = contract?.protocol;
  const tesseract = contract?.tesseract;
  const leptonica = contract?.leptonica;
  const traineddata = contract?.traineddata;
  const artifact = contract?.final_artifact;
  const dataFiles = traineddata?.files;
  if (
    !exactKeys(contract, [
      "schema_version",
      "target_triple",
      "runtime_pack_schema",
      "runtime_pack_id",
      "product_runtime_network_access",
      "process_containment_owner",
      "protocol",
      "tesseract",
      "leptonica",
      "traineddata",
      "final_artifact",
    ]) ||
    contract.schema_version !== SCHEMA ||
    contract.target_triple !== TARGET ||
    contract.runtime_pack_schema !== PACK_SCHEMA ||
    contract.runtime_pack_id !== PACK_ID ||
    contract.product_runtime_network_access !== "disabled" ||
    contract.process_containment_owner !== "ocr_tesseract" ||
    !exactKeys(protocol, [
      "input_format",
      "input_max_bytes",
      "arguments",
      "languages",
      "page_segmentation_mode",
      "stdout_format",
      "stdout_max_bytes",
      "stderr",
      "poll_interval_ms",
    ]) ||
    protocol.input_format !== "ppm-p6-rgb8" ||
    protocol.input_max_bytes !== 32 * 1024 * 1024 ||
    !sameArray(protocol.arguments, PROTOCOL_ARGUMENTS) ||
    !sameArray(protocol.languages, ["eng", "chi_sim"]) ||
    protocol.page_segmentation_mode !== 6 ||
    protocol.stdout_format !== "tesseract-tsv" ||
    protocol.stdout_max_bytes !== 4 * 1024 * 1024 ||
    protocol.stderr !== "bounded-generic-only" ||
    protocol.poll_interval_ms !== 10 ||
    !exactKeys(tesseract, [
      "version",
      "source_repository",
      "source_tag",
      "source_commit",
      "license",
      "source_license_file",
      "build_provenance_schema",
      "cmake_generator",
      "cmake_arguments",
      "build_target",
      "final_binary_file",
      "engine_config_file",
    ]) ||
    tesseract.version !== "5.5.2" ||
    tesseract.source_repository !== "https://github.com/tesseract-ocr/tesseract" ||
    tesseract.source_tag !== "5.5.2" ||
    tesseract.source_commit !== TESSERACT_COMMIT ||
    tesseract.license !== "Apache-2.0" ||
    !fileIdentity(tesseract.source_license_file, {
      file: "LICENSE",
      bytes: 11_358,
      sha256: "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
    }) ||
    tesseract.build_provenance_schema !==
      "resume-ir.tesseract-windows-build-provenance.v1" ||
    tesseract.cmake_generator !== "Ninja" ||
    !sameArray(tesseract.cmake_arguments, TESSERACT_ARGUMENTS) ||
    tesseract.build_target !== "tesseract" ||
    tesseract.final_binary_file !== "tesseract.exe" ||
    !fileIdentity(tesseract.engine_config_file, {
      role: "engine_config",
      file: "tessdata/configs/tsv",
      bytes: 22,
      sha256: "59d079bb75d8b3d7c839a3564580cb559e362c93a9d70f234e421c0c3e767e04",
    }) ||
    !exactKeys(leptonica, [
      "version",
      "source_repository",
      "source_tag",
      "source_commit",
      "license",
      "source_license_file",
      "cmake_generator",
      "cmake_arguments",
      "required_input_decoder",
    ]) ||
    leptonica.version !== "1.87.0" ||
    leptonica.source_repository !== "https://github.com/DanBloomberg/leptonica" ||
    leptonica.source_tag !== "1.87.0" ||
    leptonica.source_commit !== LEPTONICA_COMMIT ||
    leptonica.license !== "BSD-2-Clause" ||
    !fileIdentity(leptonica.source_license_file, {
      file: "leptonica-license.txt",
      bytes: 1_521,
      sha256: "87829abb5bbb00b55a107365da89e9a33f86c4250169e5a1e5588505be7d5806",
    }) ||
    leptonica.cmake_generator !== "Ninja" ||
    !sameArray(leptonica.cmake_arguments, LEPTONICA_ARGUMENTS) ||
    leptonica.required_input_decoder !== "pnm" ||
    !exactKeys(traineddata, [
      "version",
      "source_repository",
      "source_tag",
      "source_commit",
      "license",
      "source_license_file",
      "files",
    ]) ||
    traineddata.version !== "4.1.0" ||
    traineddata.source_repository !== "https://github.com/tesseract-ocr/tessdata_fast" ||
    traineddata.source_tag !== "4.1.0" ||
    traineddata.source_commit !== TESSDATA_COMMIT ||
    traineddata.license !== "Apache-2.0" ||
    !fileIdentity(traineddata.source_license_file, {
      file: "LICENSE",
      bytes: 11_358,
      sha256: "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30",
    }) ||
    !Array.isArray(dataFiles) ||
    dataFiles.length !== 2 ||
    !fileIdentity(dataFiles[0], {
      role: "language_eng",
      file: "eng.traineddata",
      bytes: 4_113_088,
      sha256: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2",
    }) ||
    !fileIdentity(dataFiles[1], {
      role: "language_chi_sim",
      file: "chi_sim.traineddata",
      bytes: 2_469_156,
      sha256: "a5fcb6f0db1e1d6d8522f39db4e848f05984669172e584e8d76b6b3141e1f730",
    }) ||
    !exactKeys(artifact, [
      "file",
      "machine",
      "max_bytes",
      "msvc_runtime",
      "required_dependency_closure",
      "forbidden_import_prefixes",
      "runtime_download_allowed",
    ]) ||
    artifact.file !== "tesseract.exe" ||
    artifact.machine !== "x86_64" ||
    artifact.max_bytes !== 64 * 1024 * 1024 ||
    artifact.msvc_runtime !== "static" ||
    artifact.required_dependency_closure !== "windows-system-dlls-only" ||
    !sameArray(artifact.forbidden_import_prefixes, FORBIDDEN_IMPORTS) ||
    artifact.runtime_download_allowed !== false
  ) {
    throw new Error("Windows OCR source contract is invalid");
  }
  return contract;
}

export function readWindowsOcrSourceContract(file) {
  if (!path.isAbsolute(file)) throw new Error("Windows OCR source contract path is invalid");
  let metadata;
  try {
    metadata = lstatSync(file);
  } catch {
    throw new Error("Windows OCR source contract is missing");
  }
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > 64 * 1024
  ) {
    throw new Error("Windows OCR source contract file is invalid");
  }
  try {
    return validateWindowsOcrSourceContract(JSON.parse(readFileSync(file, "utf8")));
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("Windows OCR source contract is not valid JSON");
    }
    throw error;
  }
}

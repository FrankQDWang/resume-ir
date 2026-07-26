import { createHash } from "node:crypto";
import {
  copyFileSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

import { assembleWindowsOcrPack } from "./windows-ocr-artifact.mjs";
import { readWindowsOcrBuilderContract } from "./windows-ocr-builder.mjs";
import { inspectWindowsPeExecutable } from "./windows-pe.mjs";
import { readWindowsOcrSourceContract } from "./windows-ocr-pack.mjs";

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env ?? process.env,
    encoding: options.encoding ?? "utf8",
    shell: false,
    stdio: options.stdio ?? "inherit",
    maxBuffer: 1024 * 1024,
  });
  if (result.error || result.status !== 0) {
    throw new Error(`Windows OCR build command failed: ${command}`);
  }
  return result;
}

function output(command, args, cwd) {
  return run(command, args, { cwd, stdio: "pipe" }).stdout.trim();
}

function directDirectory(directory, label) {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const metadata = lstatSync(directory);
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw new Error(`${label} must be a regular directory`);
  }
}

function exactCheckout({ root, repository, revision }) {
  if (!path.isAbsolute(root)) throw new Error("Windows OCR checkout path is invalid");
  if (!exists(root)) {
    run("git", ["clone", "--no-checkout", repository, root]);
  } else {
    const metadata = lstatSync(root);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      throw new Error("Windows OCR checkout path is unsafe");
    }
  }
  run("git", ["-C", root, "fetch", "--depth", "1", "origin", revision]);
  run("git", ["-C", root, "checkout", "--detach", "FETCH_HEAD"]);
  if (
    output("git", ["rev-parse", "HEAD"], root) !== revision ||
    output("git", ["status", "--porcelain", "--untracked-files=no"], root) !== ""
  ) {
    throw new Error("Windows OCR checkout did not resolve to the exact clean revision");
  }
}

function exists(file) {
  try {
    lstatSync(file);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

function firstExisting(candidates, label) {
  const matches = candidates.filter(exists);
  if (matches.length !== 1) {
    throw new Error(`${label} did not produce one exact artifact`);
  }
  const metadata = lstatSync(matches[0]);
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size === 0) {
    throw new Error(`${label} artifact is invalid`);
  }
  return matches[0];
}

function numericVersion(text, label) {
  const match = text.match(/\d+(?:\.\d+){1,4}/u);
  if (!match) throw new Error(`${label} version is unavailable`);
  return match[0];
}

function sourceProvenance(source) {
  return {
    version: source.version,
    source_repository: source.source_repository,
    source_tag: source.source_tag,
    source_commit: source.source_commit,
    cmake_generator: source.cmake_generator,
    cmake_arguments: source.cmake_arguments,
    source_tree_clean: true,
  };
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function parseArguments(args, defaults) {
  const values = { ...defaults };
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (
      !value ||
      !["--workspace", "--destination", "--source-contract", "--builder-contract"].includes(
        key,
      )
    ) {
      throw new Error("invalid Windows OCR build arguments");
    }
    values[key.slice(2).replaceAll("-", "_")] = path.resolve(value);
  }
  return values;
}

export async function buildWindowsOcrPack({
  workspace,
  destination,
  source_contract: sourceContractFile,
  builder_contract: builderContractFile,
}) {
  if (process.platform !== "win32" || process.arch !== "x64") {
    throw new Error("Windows OCR pack must be built natively on x86_64 Windows");
  }
  for (const value of [
    workspace,
    destination,
    sourceContractFile,
    builderContractFile,
  ]) {
    if (!path.isAbsolute(value)) throw new Error("Windows OCR build paths must be absolute");
  }
  const contract = readWindowsOcrSourceContract(sourceContractFile);
  readWindowsOcrBuilderContract(builderContractFile);
  directDirectory(workspace, "Windows OCR build workspace");

  const sources = path.join(workspace, "sources");
  const builds = path.join(workspace, "build");
  const install = path.join(workspace, "install");
  const inputs = path.join(workspace, "validated-inputs");
  for (const directory of [sources, builds]) directDirectory(directory, "Windows OCR build root");
  rmSync(install, { recursive: true, force: true });
  rmSync(inputs, { recursive: true, force: true });
  directDirectory(install, "Windows OCR install root");
  directDirectory(inputs, "Windows OCR validated input root");

  const tesseractSource = path.join(sources, "tesseract");
  const leptonicaSource = path.join(sources, "leptonica");
  const tessdataSource = path.join(sources, "tessdata_fast");
  exactCheckout({
    root: tesseractSource,
    repository: contract.tesseract.source_repository,
    revision: contract.tesseract.source_commit,
  });
  exactCheckout({
    root: leptonicaSource,
    repository: contract.leptonica.source_repository,
    revision: contract.leptonica.source_commit,
  });
  exactCheckout({
    root: tessdataSource,
    repository: contract.traineddata.source_repository,
    revision: contract.traineddata.source_commit,
  });

  const leptonicaBuild = path.join(builds, "leptonica");
  const tesseractBuild = path.join(builds, "tesseract");
  rmSync(leptonicaBuild, { recursive: true, force: true });
  rmSync(tesseractBuild, { recursive: true, force: true });
  run("cmake", [
    "-S",
    leptonicaSource,
    "-B",
    leptonicaBuild,
    "-G",
    contract.leptonica.cmake_generator,
    `-DCMAKE_INSTALL_PREFIX=${install}`,
    ...contract.leptonica.cmake_arguments,
  ]);
  run("cmake", ["--build", leptonicaBuild, "--target", "install", "--config", "Release"]);
  run("cmake", [
    "-S",
    tesseractSource,
    "-B",
    tesseractBuild,
    "-G",
    contract.tesseract.cmake_generator,
    `-DCMAKE_PREFIX_PATH=${install}`,
    ...contract.tesseract.cmake_arguments,
  ]);
  run("cmake", [
    "--build",
    tesseractBuild,
    "--target",
    contract.tesseract.build_target,
    "--config",
    "Release",
  ]);

  const executable = firstExisting(
    [
      path.join(tesseractBuild, "bin", contract.tesseract.final_binary_file),
      path.join(tesseractBuild, "bin", "Release", contract.tesseract.final_binary_file),
      path.join(tesseractBuild, contract.tesseract.final_binary_file),
      path.join(tesseractBuild, "Release", contract.tesseract.final_binary_file),
    ],
    "Windows OCR native build",
  );
  const runtimeRoot = path.join(inputs, "runtime");
  const dataRoot = path.join(inputs, "tessdata");
  directDirectory(runtimeRoot, "Windows OCR runtime input");
  directDirectory(path.join(dataRoot, "configs"), "Windows OCR data input");
  copyFileSync(executable, path.join(runtimeRoot, "tesseract.exe"));
  copyFileSync(path.join(tesseractSource, "LICENSE"), path.join(runtimeRoot, "LICENSE"));
  copyFileSync(
    path.join(leptonicaSource, contract.leptonica.source_license_file.file),
    path.join(runtimeRoot, "leptonica-license.txt"),
  );
  copyFileSync(path.join(tessdataSource, "LICENSE"), path.join(dataRoot, "LICENSE"));
  for (const entry of contract.traineddata.files) {
    copyFileSync(path.join(tessdataSource, entry.file), path.join(dataRoot, entry.file));
  }
  copyFileSync(
    path.join(tesseractSource, contract.tesseract.engine_config_file.file),
    path.join(dataRoot, "configs", "tsv"),
  );

  const executableFile = path.join(runtimeRoot, "tesseract.exe");
  const executableBytes = readFileSync(executableFile);
  const image = inspectWindowsPeExecutable(executableBytes);
  const smokeInput = path.join(inputs, "smoke.ppm");
  writeFileSync(smokeInput, "P6\n1 1\n255\n\u0000\u0000\u0000", "binary");
  const smoke = run(
    executableFile,
    [smokeInput, "stdout", "--psm", "6", "-l", "eng+chi_sim", "tsv"],
    {
      env: { ...process.env, TESSDATA_PREFIX: dataRoot },
      stdio: "pipe",
    },
  );
  if (Buffer.byteLength(smoke.stdout) > 4 * 1024 * 1024) {
    throw new Error("Windows OCR native smoke output exceeded its bound");
  }
  const cl = spawnSync("cl.exe", [], { encoding: "utf8", shell: false });
  const provenance = {
    schema_version: contract.tesseract.build_provenance_schema,
    target_triple: contract.target_triple,
    tesseract: sourceProvenance(contract.tesseract),
    leptonica: sourceProvenance(contract.leptonica),
    msvc_runtime: "static",
    msvc_toolset_version: numericVersion(
      `${cl.stdout ?? ""}\n${cl.stderr ?? ""}`,
      "MSVC",
    ),
    windows_sdk_version: numericVersion(
      process.env.WindowsSDKVersion ?? "",
      "Windows SDK",
    ),
    cmake_version: numericVersion(output("cmake", ["--version"]), "CMake"),
    ninja_version: numericVersion(output("ninja", ["--version"]), "Ninja"),
    tests_passed: true,
    artifact_file: "tesseract.exe",
    artifact_bytes: executableBytes.length,
    artifact_sha256: sha256(executableFile),
    artifact_imports: image.imports,
  };
  writeFileSync(
    path.join(runtimeRoot, "build-provenance.json"),
    `${JSON.stringify(provenance, null, 2)}\n`,
  );
  return assembleWindowsOcrPack({
    contractFile: sourceContractFile,
    runtimeRoot,
    dataRoot,
    destination,
  });
}

const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
const defaults = {
  workspace: path.join(repoRoot, ".cache", "resume-ir-windows-ocr-source-build"),
  destination: path.join(repoRoot, ".cache", "resume-ir-windows-ocr-runtime-pack"),
  source_contract: path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "ocr",
    "x86_64-pc-windows-msvc",
    "source-contract.json",
  ),
  builder_contract: path.join(
    repoRoot,
    "apps",
    "desktop",
    "runtime-build",
    "windows-ocr",
    "builder-contract.json",
  ),
};

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  buildWindowsOcrPack(parseArguments(process.argv.slice(2), defaults))
    .then((receipt) => {
      console.log(
        `built reviewed Windows OCR pack (${receipt.resource_file_count} files)`,
      );
    })
    .catch((error) => {
      console.error(`build-windows-ocr-pack: ${error.message}`);
      process.exitCode = 1;
    });
}

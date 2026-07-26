import { lstatSync, mkdirSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

import {
  assembleWindowsPdfiumStaticPack,
  readWindowsPdfRendererSourceContract,
} from "./windows-pdf-renderer.mjs";

const DEPOT_TOOLS_REPOSITORY =
  "https://chromium.googlesource.com/chromium/tools/depot_tools.git";

function run(command, args, { cwd, env = process.env } = {}) {
  const result = spawnSync(command, args, {
    cwd,
    env,
    shell: false,
    stdio: "inherit",
  });
  if (result.error || result.status !== 0) {
    throw new Error(`${path.basename(command)} failed`);
  }
}

function output(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    shell: false,
  });
  if (result.error || result.status !== 0) {
    throw new Error(`${path.basename(command)} verification failed`);
  }
  return result.stdout.trim();
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

function ensureOwnedDirectory(directory) {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const metadata = lstatSync(directory);
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw new Error("Windows PDFium build workspace is not a secure directory");
  }
}

function ensureCheckout({ directory, repository, revision }) {
  if (!path.isAbsolute(directory)) throw new Error("checkout path must be absolute");
  if (!exists(directory)) {
    run("git", ["clone", "--no-checkout", repository, directory]);
  } else {
    const metadata = lstatSync(directory);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      throw new Error("existing checkout path is unsafe");
    }
  }
  run("git", ["-C", directory, "fetch", "--depth", "1", "origin", revision]);
  run("git", ["-C", directory, "checkout", "--detach", "FETCH_HEAD"]);
  if (
    output("git", ["rev-parse", "HEAD"], directory) !== revision ||
    output("git", ["status", "--porcelain", "--untracked-files=no"], directory) !== ""
  ) {
    throw new Error("checkout did not resolve to the reviewed revision");
  }
}

function parseArguments(args, defaults) {
  const parsed = { ...defaults };
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (!value || !["--workspace", "--destination", "--contract"].includes(key)) {
      throw new Error("invalid Windows PDFium build arguments");
    }
    parsed[key.slice(2)] = path.resolve(value);
  }
  return parsed;
}

export async function buildWindowsPdfiumStaticPack({
  workspace,
  destination,
  contract: contractPath,
}) {
  if (process.platform !== "win32") {
    throw new Error("Windows PDFium static pack must be built on Windows");
  }
  for (const value of [workspace, destination, contractPath]) {
    if (!path.isAbsolute(value)) throw new Error("PDFium build paths must be absolute");
  }
  const contract = readWindowsPdfRendererSourceContract(contractPath);
  ensureOwnedDirectory(workspace);
  const depotTools = path.join(workspace, "depot_tools");
  ensureCheckout({
    directory: depotTools,
    repository: DEPOT_TOOLS_REPOSITORY,
    revision: contract.pdfium.source_build_dependency_revision,
  });
  const environment = {
    ...process.env,
    DEPOT_TOOLS_UPDATE: "0",
    DEPOT_TOOLS_WIN_TOOLCHAIN: "0",
    PATH: `${depotTools}${path.delimiter}${process.env.PATH ?? ""}`,
  };
  const clientRoot = path.join(workspace, "client");
  ensureOwnedDirectory(clientRoot);
  run(
    path.join(depotTools, "gclient.bat"),
    [
      "config",
      "--unmanaged",
      "--custom-var",
      'checkout_configuration="minimal"',
      contract.pdfium.source_repository,
    ],
    { cwd: clientRoot, env: environment },
  );
  run(
    path.join(depotTools, "gclient.bat"),
    [
      "sync",
      "--delete_unversioned_trees",
      "--force",
      "--revision",
      `pdfium@${contract.pdfium.source_commit}`,
    ],
    { cwd: clientRoot, env: environment },
  );
  const checkout = path.join(clientRoot, "pdfium");
  if (
    output("git", ["rev-parse", "HEAD"], checkout) !==
      contract.pdfium.source_commit ||
    output("git", ["status", "--porcelain", "--untracked-files=no"], checkout) !== ""
  ) {
    throw new Error("gclient did not resolve the exact clean PDFium commit");
  }
  const buildOutput = path.join(checkout, "out", "resume-ir-release-x64");
  ensureOwnedDirectory(buildOutput);
  const argsFile = path.join(buildOutput, contract.pack.args_file);
  writeFileSync(
    argsFile,
    `${contract.pdfium.gn_arguments.join("\n")}\n`,
    { encoding: "utf8", mode: 0o600 },
  );
  run(path.join(depotTools, "gn.exe"), ["gen", buildOutput], {
    cwd: checkout,
    env: environment,
  });
  run(
    path.join(depotTools, "autoninja.bat"),
    ["-C", buildOutput, ...contract.pdfium.build_targets],
    { cwd: checkout, env: environment },
  );
  return assembleWindowsPdfiumStaticPack({
    library: path.join(buildOutput, "obj", contract.pdfium.static_library_file),
    license: path.join(checkout, contract.pdfium.source_license_file.file),
    args: argsFile,
    destination,
    sourceContract: contractPath,
  });
}

const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
const defaults = {
  workspace: path.join(repoRoot, ".cache", "resume-ir-windows-pdfium-source-build"),
  destination: path.join(repoRoot, ".cache", "resume-ir-windows-pdfium-static-pack"),
  contract: path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "pdf-renderer",
    "x86_64-pc-windows-msvc",
    "source-contract.json",
  ),
};

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  buildWindowsPdfiumStaticPack(
    parseArguments(process.argv.slice(2), defaults),
  ).catch((error) => {
    console.error(`build-windows-pdfium-static-pack: ${error.message}`);
    process.exitCode = 1;
  });
}

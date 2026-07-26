import { lstatSync, mkdirSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

import {
  assembleMacosPdfiumStaticPack,
  readMacosPdfiumSourceContract,
} from "./macos-pdfium-static-pack.mjs";
import { resolveMacosXcodeToolchain } from "./macos-xcode-toolchain.mjs";

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

function ensureOwnedDirectory(directory) {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const metadata = lstatSync(directory);
  if (
    !metadata.isDirectory() ||
    metadata.isSymbolicLink() ||
    (typeof process.getuid === "function" && metadata.uid !== process.getuid())
  ) {
    throw new Error("PDFium build workspace is not a secure owned directory");
  }
}

function ensureCheckout({ directory, repository, revision }) {
  if (!path.isAbsolute(directory)) throw new Error("checkout path must be absolute");
  try {
    const metadata = lstatSync(directory);
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      throw new Error("existing checkout path is unsafe");
    }
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
    run("git", ["clone", "--no-checkout", repository, directory]);
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
      throw new Error("invalid macOS PDFium build arguments");
    }
    parsed[key.slice(2)] = path.resolve(value);
  }
  return parsed;
}

export async function buildMacosPdfiumStaticPack({
  workspace,
  destination,
  contract: contractPath,
}) {
  for (const value of [workspace, destination, contractPath]) {
    if (!path.isAbsolute(value)) throw new Error("PDFium build paths must be absolute");
  }
  const contract = readMacosPdfiumSourceContract(contractPath);
  const xcode = resolveMacosXcodeToolchain();
  ensureOwnedDirectory(workspace);
  const depotTools = path.join(workspace, "depot_tools");
  ensureCheckout({
    directory: depotTools,
    repository: DEPOT_TOOLS_REPOSITORY,
    revision: contract.pdfium.source_build_dependency_revision,
  });
  const environment = {
    ...xcode.environment,
    DEPOT_TOOLS_UPDATE: "0",
    PATH: `${depotTools}${path.delimiter}${process.env.PATH ?? ""}`,
  };
  run(path.join(depotTools, "ensure_bootstrap"), [], {
    cwd: depotTools,
    env: environment,
  });
  const clientRoot = path.join(workspace, "client");
  ensureOwnedDirectory(clientRoot);
  if (!lstatSync(clientRoot).isDirectory()) {
    throw new Error("PDFium client workspace is invalid");
  }
  run(
    path.join(depotTools, "gclient"),
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
    path.join(depotTools, "gclient"),
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
  if (output("git", ["rev-parse", "HEAD"], checkout) !== contract.pdfium.source_commit) {
    throw new Error("gclient did not resolve the reviewed PDFium commit");
  }
  const buildOutput = path.join(checkout, "out", "resume-ir-release-arm64");
  ensureOwnedDirectory(buildOutput);
  writeFileSync(
    path.join(buildOutput, contract.pack.args_file),
    `${contract.pdfium.gn_arguments.join("\n")}\n`,
    { encoding: "utf8", mode: 0o600 },
  );
  run(path.join(depotTools, "gn"), ["gen", buildOutput], {
    cwd: checkout,
    env: environment,
  });
  run(
    path.join(depotTools, "autoninja"),
    ["-C", buildOutput, contract.pdfium.build_target],
    { cwd: checkout, env: environment },
  );
  return assembleMacosPdfiumStaticPack({
    checkout,
    buildOutput,
    destination,
    sourceContract: contractPath,
  });
}

const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
const defaults = {
  workspace: path.join(repoRoot, ".cache", "resume-ir-pdfium-source-build"),
  destination: path.join(repoRoot, ".cache", "resume-ir-macos-pdfium-static-pack"),
  contract: path.join(
    repoRoot,
    "apps",
    "desktop",
    "resources",
    "pdf-renderer",
    "aarch64-apple-darwin",
    "source-contract.json",
  ),
};

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  buildMacosPdfiumStaticPack(parseArguments(process.argv.slice(2), defaults)).catch(
    (error) => {
      console.error(`build-macos-pdfium-static-pack: ${error.message}`);
      process.exitCode = 1;
    },
  );
}

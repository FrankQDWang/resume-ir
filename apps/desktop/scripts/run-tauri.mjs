import { spawnSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const RUSTFLAG_SEPARATOR = "\u001f";

export function resolveTauriPaths(scriptUrl = import.meta.url) {
  return Object.freeze({
    repoRoot: fileURLToPath(new URL("../../..", scriptUrl)),
    frontendRoot: fileURLToPath(new URL("..", scriptUrl)),
    cli: fileURLToPath(
      new URL("../node_modules/@tauri-apps/cli/tauri.js", scriptUrl),
    ),
  });
}

export function createTauriBuildEnvironment({
  environment,
  repoRoot,
  homeDirectory,
}) {
  if (!path.isAbsolute(repoRoot) || !path.isAbsolute(homeDirectory)) {
    throw new Error("build path remapping requires absolute paths");
  }
  if (environment.RUSTFLAGS) {
    throw new Error(
      "RUSTFLAGS must be unset; use CARGO_ENCODED_RUSTFLAGS for explicit build flags",
    );
  }

  const remapRoots = [
    [repoRoot, "/source/resume-ir"],
    [environment.CARGO_HOME, "/cargo-home"],
    [environment.RUSTUP_HOME, "/rustup-home"],
    [environment.TMPDIR, "/build-tmp"],
    [homeDirectory, "/build-home"],
  ];
  const seen = new Set();
  const remapFlags = remapRoots.flatMap(([source, destination]) => {
    if (!path.isAbsolute(source ?? "") || seen.has(source)) return [];
    seen.add(source);
    return [`--remap-path-prefix=${source}=${destination}`];
  });
  const inherited = environment.CARGO_ENCODED_RUSTFLAGS;
  return {
    ...environment,
    CARGO_ENCODED_RUSTFLAGS: [inherited, ...remapFlags]
      .filter(Boolean)
      .join(RUSTFLAG_SEPARATOR),
  };
}

export function selectTauriEnvironment({
  arguments: cliArguments,
  environment,
  repoRoot,
  homeDirectory,
}) {
  const releaseBuild =
    cliArguments[0] === "build" && !cliArguments.includes("--debug");
  return releaseBuild
    ? createTauriBuildEnvironment({ environment, repoRoot, homeDirectory })
    : environment;
}

export function withDesktopComposition(cliArguments, bundleConfig) {
  if (!["build", "dev"].includes(cliArguments[0])) return cliArguments;
  const delimiter = cliArguments.indexOf("--");
  const insertion = delimiter === -1 ? cliArguments.length : delimiter;
  return [
    ...cliArguments.slice(0, insertion),
    "--config",
    bundleConfig,
    ...cliArguments.slice(insertion),
  ];
}

function main() {
  const { repoRoot, frontendRoot, cli } = resolveTauriPaths();
  const cliArguments = process.argv.slice(2);
  const effectiveArguments = withDesktopComposition(
    cliArguments,
    path.join(frontendRoot, "src-tauri", "tauri.bundle.conf.json"),
  );
  const environment = selectTauriEnvironment({
    arguments: cliArguments,
    environment: process.env,
    repoRoot,
    homeDirectory: os.homedir(),
  });
  const result = spawnSync(process.execPath, [cli, ...effectiveArguments], {
    cwd: frontendRoot,
    env: environment,
    shell: false,
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  try {
    main();
  } catch (error) {
    console.error(`run-tauri: ${error.message}`);
    process.exitCode = 1;
  }
}

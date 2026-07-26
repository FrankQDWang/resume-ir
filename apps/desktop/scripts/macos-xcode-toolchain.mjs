import { spawnSync } from "node:child_process";
import { lstatSync } from "node:fs";
import path from "node:path";

function commandOutput(command, args, env) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    env,
    shell: false,
  });
  if (result.error || result.status !== 0) {
    throw new Error("macOS PDFium source build requires a complete Xcode installation");
  }
  return result.stdout.trim();
}

export function validateMacosXcodeToolchain({
  developerDirectory,
  xcodeVersion,
}) {
  if (
    !path.isAbsolute(developerDirectory) ||
    path.basename(developerDirectory) === "CommandLineTools"
  ) {
    throw new Error("macOS PDFium source build requires a complete Xcode installation");
  }
  const metadata = lstatSync(developerDirectory);
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw new Error("selected Xcode developer directory is unsafe");
  }
  if (!/^Xcode \d+(?:\.\d+){1,2}\nBuild version [A-Za-z0-9]+$/u.test(xcodeVersion)) {
    throw new Error("selected Xcode installation returned an invalid version");
  }
  return developerDirectory;
}

export function chooseMacosDeveloperDirectory({ configured, selected }) {
  if (configured !== undefined) {
    const value = configured.trim();
    if (value.length === 0 || !path.isAbsolute(value)) {
      throw new Error("configured Xcode developer directory is invalid");
    }
    return value;
  }
  return selected;
}

export function resolveMacosXcodeToolchain(env = process.env) {
  const developerDirectory = chooseMacosDeveloperDirectory({
    configured: env.DEVELOPER_DIR,
    selected:
      env.DEVELOPER_DIR === undefined
        ? commandOutput("xcode-select", ["-p"], env)
        : undefined,
  });
  const selectedEnvironment = {
    ...env,
    DEVELOPER_DIR: developerDirectory,
  };
  const xcodeVersion = commandOutput(
    "/usr/bin/xcodebuild",
    ["-version"],
    selectedEnvironment,
  );
  return {
    developerDirectory: validateMacosXcodeToolchain({
      developerDirectory,
      xcodeVersion,
    }),
    environment: selectedEnvironment,
  };
}

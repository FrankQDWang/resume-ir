import { spawnSync } from "node:child_process";
import path from "node:path";

export const MACOS_SYSTEM_TOOLS = Object.freeze({
  codesign: "/usr/bin/codesign",
  ditto: "/usr/bin/ditto",
  git: "/usr/bin/git",
  hdiutil: "/usr/bin/hdiutil",
  installNameTool: "/usr/bin/install_name_tool",
  otool: "/usr/bin/otool",
  plutil: "/usr/bin/plutil",
  spctl: "/usr/sbin/spctl",
});

export const CLOSED_SYSTEM_TOOL_ENV = Object.freeze({
  HOME: "/var/empty",
  LANG: "C",
  LC_ALL: "C",
  PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
  TMPDIR: "/tmp",
});

export function runClosedSystemTool(command, args, options = {}) {
  if (
    typeof command !== "string" ||
    !path.isAbsolute(command) ||
    !Array.isArray(args) ||
    !args.every(
      (argument) =>
        typeof argument === "string" && !argument.includes("\0"),
    )
  ) {
    throw new Error("macOS system tool invocation is invalid");
  }
  return spawnSync(command, args, {
    ...options,
    env: CLOSED_SYSTEM_TOOL_ENV,
    shell: false,
    windowsHide: true,
  });
}

import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

import {
  CLOSED_SYSTEM_TOOL_ENV,
  MACOS_SYSTEM_TOOLS,
  runClosedSystemTool,
} from "./macos-system-tools.mjs";

test("pins every native trust tool to an absolute system path", () => {
  assert.deepEqual(MACOS_SYSTEM_TOOLS, {
    codesign: "/usr/bin/codesign",
    ditto: "/usr/bin/ditto",
    git: "/usr/bin/git",
    hdiutil: "/usr/bin/hdiutil",
    installNameTool: "/usr/bin/install_name_tool",
    otool: "/usr/bin/otool",
    plutil: "/usr/bin/plutil",
    spctl: "/usr/sbin/spctl",
  });
  assert.ok(
    Object.values(MACOS_SYSTEM_TOOLS).every((command) =>
      path.isAbsolute(command)
    ),
  );
});

test("executes native trust tools with a closed environment", () => {
  const result = runClosedSystemTool("/usr/bin/env", [], {
    encoding: "utf8",
  });
  assert.equal(result.status, 0);
  assert.equal(result.stderr, "");
  assert.deepEqual(
    Object.fromEntries(
      result.stdout
        .trimEnd()
        .split("\n")
        .map((entry) => entry.split(/=(.*)/s).slice(0, 2)),
    ),
    CLOSED_SYSTEM_TOOL_ENV,
  );
});

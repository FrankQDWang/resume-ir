import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

import {
  chooseMacosDeveloperDirectory,
  validateMacosXcodeToolchain,
} from "./macos-xcode-toolchain.mjs";

test("prefers an explicit Xcode directory without changing global selection", () => {
  assert.equal(
    chooseMacosDeveloperDirectory({
      configured: " /Applications/Xcode.app/Contents/Developer ",
      selected: "/Library/Developer/CommandLineTools",
    }),
    "/Applications/Xcode.app/Contents/Developer",
  );
});

test("rejects a relative explicit Xcode directory", () => {
  assert.throws(
    () =>
      chooseMacosDeveloperDirectory({
        configured: "Xcode.app/Contents/Developer",
        selected: "/Library/Developer/CommandLineTools",
      }),
    /configured Xcode developer directory is invalid/u,
  );
});

test("accepts a complete Xcode developer directory", () => {
  const root = mkdtempSync(path.join(tmpdir(), "resume-ir-xcode-"));
  const developerDirectory = path.join(root, "Xcode.app", "Contents", "Developer");
  mkdirSync(developerDirectory, { recursive: true });
  try {
    assert.equal(
      validateMacosXcodeToolchain({
        developerDirectory,
        xcodeVersion: "Xcode 16.4\nBuild version 16F6",
      }),
      developerDirectory,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rejects Command Line Tools without treating it as Xcode", () => {
  assert.throws(
    () =>
      validateMacosXcodeToolchain({
        developerDirectory: "/Library/Developer/CommandLineTools",
        xcodeVersion: "Xcode 16.4\nBuild version 16F6",
      }),
    /requires a complete Xcode installation/u,
  );
});

test("rejects malformed Xcode version output", () => {
  const root = mkdtempSync(path.join(tmpdir(), "resume-ir-xcode-"));
  const developerDirectory = path.join(root, "Xcode.app", "Contents", "Developer");
  mkdirSync(developerDirectory, { recursive: true });
  try {
    assert.throws(
      () =>
        validateMacosXcodeToolchain({
          developerDirectory,
          xcodeVersion: "not xcode",
        }),
      /invalid version/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

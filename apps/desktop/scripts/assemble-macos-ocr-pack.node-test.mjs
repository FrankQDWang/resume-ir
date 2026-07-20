import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  machoDependencies,
  stageEngine,
} from "./assemble-macos-ocr-pack.mjs";

test("pins OCR Mach-O inspection to the absolute system tool", () => {
  const commands = [];
  const dependencies = machoDependencies("/synthetic/tesseract", (command) => {
    commands.push(command);
    return {
      status: 0,
      stdout:
        "/synthetic/tesseract:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)\n",
      stderr: "",
    };
  });

  assert.deepEqual(dependencies, ["/usr/lib/libSystem.B.dylib"]);
  assert.deepEqual(commands, ["/usr/bin/otool"]);
});

test("pins OCR Mach-O rewrite and signing to absolute system tools", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "resume-ir-ocr-tools-"));
  context.after(() => rm(root, { recursive: true, force: true }));
  const sourceRoot = path.join(root, "source");
  const engine = path.join(sourceRoot, "tesseract");
  const library = path.join(sourceRoot, "libsynthetic.dylib");
  await mkdir(sourceRoot);
  await writeFile(engine, "synthetic-engine");
  await writeFile(library, "synthetic-library");
  const commands = [];
  const runner = (command) => {
    commands.push(command);
    return { status: 0, stdout: "", stderr: "" };
  };

  await stageEngine({
    dependencies: new Map([
      [
        engine,
        {
          edges: [{ original: library, target: library }],
          id: undefined,
        },
      ],
      [library, { edges: [], id: library }],
    ]),
    engine,
    root: path.join(root, "staged"),
    runner,
    sources: [engine, library],
  });

  assert.deepEqual(
    new Set(commands),
    new Set(["/usr/bin/install_name_tool", "/usr/bin/codesign"]),
  );
});

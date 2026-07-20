#!/usr/bin/env node

import path from "node:path";
import { pathToFileURL } from "node:url";

import {
  ACCEPTANCE_SCHEMA,
  COW_CLONE_COMPLETE,
  AcceptanceError,
  asAcceptanceError,
  validAbsolutePath,
} from "./macos-installed-main-acceptance/core.mjs";
import { cloneDirectoryTreeCow } from "./macos-installed-main-acceptance/filesystem-cow.mjs";
import {
  normalizeAcceptanceOptions,
  parseAcceptanceArgs,
} from "./macos-installed-main-acceptance/options.mjs";
import { runInstalledMainAcceptance } from "./macos-installed-main-acceptance/orchestrator-receipt.mjs";

async function runInternalCowClone(argv) {
  if (
    argv.length !== 2 ||
    !validAbsolutePath(argv[0]) ||
    !validAbsolutePath(argv[1])
  ) {
    process.exitCode = 64;
    return;
  }
  try {
    await cloneDirectoryTreeCow(argv[0], argv[1]);
    process.stdout.write(COW_CLONE_COMPLETE);
  } catch {
    process.exitCode = 70;
  }
}

async function main() {
  if (process.argv[2] === "--internal-cow-clone") {
    await runInternalCowClone(process.argv.slice(3));
    return;
  }
  process.exitCode = await runAcceptanceCli();
}

export async function runAcceptanceCli({
  argv = process.argv.slice(2),
  runAcceptance = runInstalledMainAcceptance,
  signalSource = process,
  write = (value) => process.stdout.write(value),
} = {}) {
  const controller = new AbortController();
  const interrupt = () => {
    if (!controller.signal.aborted) {
      controller.abort(new AcceptanceError("acceptance_interrupted"));
    }
  };
  signalSource.on("SIGINT", interrupt);
  signalSource.on("SIGTERM", interrupt);
  try {
    const report = await runAcceptance(parseAcceptanceArgs(argv), {
      signal: controller.signal,
    });
    write(`${JSON.stringify(report)}\n`);
    return 0;
  } catch (error) {
    const failure = asAcceptanceError(error);
    write(
      `${JSON.stringify({
        schema_version: ACCEPTANCE_SCHEMA,
        outcome: "failed",
        error_code: failure.code,
      })}\n`,
    );
    return 1;
  } finally {
    signalSource.off("SIGINT", interrupt);
    signalSource.off("SIGTERM", interrupt);
  }
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  await main();
}

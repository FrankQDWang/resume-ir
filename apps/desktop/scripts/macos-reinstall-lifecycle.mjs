#!/usr/bin/env node

import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  parseReplacementArguments,
  reinstallMacosDmg,
} from "./macos-upgrade-lifecycle.mjs";

async function main() {
  const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
  const receipt = await reinstallMacosDmg({
    repoRoot,
    ...parseReplacementArguments(process.argv.slice(2), "reinstall"),
  });
  console.log(JSON.stringify(receipt));
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    console.error(`macos-reinstall-lifecycle: ${error.message}`);
    process.exitCode = 1;
  });
}

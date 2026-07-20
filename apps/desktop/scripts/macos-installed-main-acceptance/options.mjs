import os from "node:os";
import { realpathSync } from "node:fs";

import {
  REPO_ROOT,
  fail,
  validAbsolutePath,
} from "./core.mjs";

export function normalizeAcceptanceOptions(options) {
  let defaultTemporaryParent;
  try {
    defaultTemporaryParent = realpathSync(os.tmpdir());
  } catch {
    fail("arguments_invalid");
  }
  const normalized = {
    authorizedSourceDataDir: options?.authorizedSourceDataDir,
    repoRoot: options?.repoRoot ?? REPO_ROOT,
    temporaryParent: options?.temporaryParent ?? defaultTemporaryParent,
  };
  if (
    !validAbsolutePath(normalized.authorizedSourceDataDir) ||
    !validAbsolutePath(normalized.repoRoot) ||
    !validAbsolutePath(normalized.temporaryParent)
  ) {
    fail("arguments_invalid");
  }
  return Object.freeze(normalized);
}

export function parseAcceptanceArgs(argv) {
  const values = {};
  const names = new Map([
    ["--authorized-source-data-dir", "authorizedSourceDataDir"],
    ["--repo-root", "repoRoot"],
    ["--temporary-parent", "temporaryParent"],
  ]);
  for (let index = 0; index < argv.length; index += 2) {
    const key = names.get(argv[index]);
    const value = argv[index + 1];
    if (!key || value === undefined || values[key] !== undefined) {
      fail("arguments_invalid");
    }
    values[key] = value;
  }
  return normalizeAcceptanceOptions(values);
}

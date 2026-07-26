import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";

import {
  deriveCommitProductBinding,
  deriveExactMainSourceIdentity,
  verifyGitMainBinding,
} from "./source-bindings.mjs";

const HEAD = "a".repeat(40);
const EXPECTED_ORIGIN = "https://github.com/FrankQDWang/resume-ir.git";
const SOURCE = Object.freeze({
  authority: "exact_main_commit",
  base_commit: HEAD,
  source_tree_sha256: "e".repeat(64),
});

function toolResult(status, stdout = "") {
  return {
    status,
    stdout,
    stderr: "",
    timedOut: false,
    overflow: false,
  };
}

function gitRunner({
  branch = "main",
  dirty = "",
  origin = EXPECTED_ORIGIN,
  rawOrigin = origin,
  remote = HEAD,
  calls = [],
} = {}) {
  return async (command, args, options) => {
    calls.push({ command, args, options });
    const operation = args.slice(2).join(" ");
    if (operation === "rev-parse --verify HEAD") {
      return toolResult(0, `${HEAD}\n`);
    }
    if (operation === "symbolic-ref --quiet --short HEAD") {
      return branch === null ? toolResult(1) : toolResult(0, `${branch}\n`);
    }
    if (operation === "status --porcelain=v1 --untracked-files=all") {
      return toolResult(0, dirty);
    }
    if (operation === "remote get-url --all origin") {
      return toolResult(0, `${origin}\n`);
    }
    if (operation === "config --local --get-all remote.origin.url") {
      return toolResult(0, `${rawOrigin}\n`);
    }
    if (operation === "ls-remote --exit-code origin refs/heads/main") {
      return toolResult(0, `${remote}\trefs/heads/main\n`);
    }
    if (
      operation ===
      "ls-files --error-unmatch -- apps/desktop/package.json apps/desktop/src-tauri/icons/icon.icns apps/desktop/src-tauri/tauri.conf.json"
    ) {
      return toolResult(
        0,
        [
          "apps/desktop/package.json",
          "apps/desktop/src-tauri/icons/icon.icns",
          "apps/desktop/src-tauri/tauri.conf.json",
          "",
        ].join("\n"),
      );
    }
    throw new Error(`unexpected git operation: ${operation}`);
  };
}

test("binds clean main or detached exact-main to a fresh remote observation", async () => {
  const calls = [];
  assert.deepEqual(
    await verifyGitMainBinding("/synthetic/repo", gitRunner({ calls })),
    { detached: false, gitHead: HEAD },
  );
  assert.ok(
    calls.every(
      ({ command, options }) =>
        command === "/usr/bin/git" &&
        options.env.GIT_CONFIG_GLOBAL === "/dev/null" &&
        options.env.GIT_CONFIG_NOSYSTEM === "1" &&
        options.env.GIT_NO_REPLACE_OBJECTS === "1" &&
        options.env.GIT_TERMINAL_PROMPT === "0",
    ),
  );
  assert.deepEqual(
    await verifyGitMainBinding(
      "/synthetic/repo",
      gitRunner({ branch: null }),
    ),
    { detached: true, gitHead: HEAD },
  );
  await assert.rejects(
    verifyGitMainBinding(
      "/synthetic/repo",
      gitRunner({ dirty: "?? untracked-build-input\n" }),
    ),
    /git_main_binding_invalid/,
  );
  await assert.rejects(
    verifyGitMainBinding(
      "/synthetic/repo",
      gitRunner({ remote: "b".repeat(40) }),
    ),
    /git_main_binding_invalid/,
  );
  await assert.rejects(
    verifyGitMainBinding(
      "/synthetic/repo",
      gitRunner({ branch: "feature" }),
    ),
    /git_main_binding_invalid/,
  );
  await assert.rejects(
    verifyGitMainBinding(
      "/synthetic/repo",
      gitRunner({ origin: "https://example.invalid/resume-ir.git" }),
    ),
    /git_main_binding_invalid/,
  );
  await assert.rejects(
    verifyGitMainBinding(
      "/synthetic/repo",
      gitRunner({ origin: `${EXPECTED_ORIGIN}\n${EXPECTED_ORIGIN}` }),
    ),
    /git_main_binding_invalid/,
  );
  await assert.rejects(
    verifyGitMainBinding(
      "/synthetic/repo",
      gitRunner({ rawOrigin: "resume-ir-origin-alias" }),
    ),
    /git_main_binding_invalid/,
  );
});

test("samples provenance serially and rejects a torn HEAD or remote bracket", async () => {
  let active = 0;
  let maximumActive = 0;
  const base = gitRunner();
  await verifyGitMainBinding("/synthetic/repo", async (...args) => {
    active += 1;
    maximumActive = Math.max(maximumActive, active);
    await new Promise((resolve) => setImmediate(resolve));
    try {
      return await base(...args);
    } finally {
      active -= 1;
    }
  });
  assert.equal(maximumActive, 1);

  let heads = 0;
  await assert.rejects(
    verifyGitMainBinding("/synthetic/repo", async (command, args, options) => {
      const operation = args.slice(2).join(" ");
      if (operation === "rev-parse --verify HEAD") {
        heads += 1;
        return toolResult(0, `${heads === 1 ? HEAD : "b".repeat(40)}\n`);
      }
      return base(command, args, options);
    }),
    /git_main_binding_invalid/,
  );

  let remotes = 0;
  await assert.rejects(
    verifyGitMainBinding("/synthetic/repo", async (command, args, options) => {
      const operation = args.slice(2).join(" ");
      if (operation === "ls-remote --exit-code origin refs/heads/main") {
        remotes += 1;
        const head = remotes === 1 ? HEAD : "b".repeat(40);
        return toolResult(0, `${head}\trefs/heads/main\n`);
      }
      return base(command, args, options);
    }),
    /git_main_binding_invalid/,
  );
});

test("derives version and binary icon digest from exact commit blobs", async () => {
  const icon = Buffer.from([0, 255, 1, 254, 2, 253]);
  const calls = [];
  const result = await deriveCommitProductBinding(
    "/synthetic/repo",
    HEAD,
    async (command, args, options) => {
      calls.push({ command, args, options });
      const object = args.at(-1);
      if (object.endsWith(":apps/desktop/package.json")) {
        return toolResult(
          0,
          JSON.stringify({ name: "resume-ir-desktop", version: "0.1.2" }),
        );
      }
      if (object.endsWith(":apps/desktop/src-tauri/tauri.conf.json")) {
        return toolResult(
          0,
          JSON.stringify({
            productName: "resume-ir",
            version: "../package.json",
            identifier: "local.resume-ir.desktop",
          }),
        );
      }
      if (object.endsWith(":apps/desktop/src-tauri/icons/icon.icns")) {
        return { ...toolResult(0), stdout: icon };
      }
      throw new Error(`unexpected object ${object}`);
    },
  );
  assert.deepEqual(result, {
    iconSha256: createHash("sha256").update(icon).digest("hex"),
    version: "0.1.2",
  });
  assert.equal(calls.at(-1).options.stdoutMode, "buffer");
  assert.equal(calls.at(-1).options.maxStdoutBytes, 8 * 1024 * 1024);
  assert.ok(
    calls.every(({ args }) => args.at(-1).startsWith(`${HEAD}:apps/desktop/`)),
  );
});

test("binds the closed source identity to the exact verified main commit", async () => {
  assert.deepEqual(
    await deriveExactMainSourceIdentity(
      "/synthetic/repo",
      HEAD,
      async ({ repoRoot, authority }) => {
        assert.equal(repoRoot, "/synthetic/repo");
        assert.equal(authority, "exact_main_commit");
        return { identity: SOURCE };
      },
    ),
    SOURCE,
  );

  await assert.rejects(
    deriveExactMainSourceIdentity(
      "/synthetic/repo",
      HEAD,
      async () => ({
        identity: { ...SOURCE, base_commit: "b".repeat(40) },
      }),
    ),
    /source_manifest_invalid/,
  );
});

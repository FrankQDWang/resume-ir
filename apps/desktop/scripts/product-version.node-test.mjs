import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  PRODUCT_VERSION,
  PRODUCT_VERSION_SOURCE,
  parseProductVersionManifest,
  productVersionFromManifest,
} from "./product-version.mjs";

test("package manifest is the single product version authority", () => {
  assert.equal(PRODUCT_VERSION, "0.1.2");
  assert.equal(PRODUCT_VERSION_SOURCE, "../package.json");
  const tauriConfig = JSON.parse(
    readFileSync(
      new URL("../src-tauri/tauri.conf.json", import.meta.url),
      "utf8",
    ),
  );
  assert.equal(tauriConfig.version, PRODUCT_VERSION_SOURCE);

  const cargoManifest = readFileSync(
    new URL("../src-tauri/Cargo.toml", import.meta.url),
    "utf8",
  );
  assert.match(cargoManifest, /^version = "0\.0\.0"$/m);
  assert.doesNotMatch(cargoManifest, new RegExp(`version = "${PRODUCT_VERSION}"`));

  for (const relative of [
    "macos-install-lifecycle.mjs",
    "macos-reinstall-core.mjs",
    "macos-lifecycle-journal.mjs",
    "macos-test-release.mjs",
    "macos-worktree-release.mjs",
    "macos-installed-main-acceptance/source-bindings.mjs",
    "macos-installed-main-acceptance/release-deployment.mjs",
  ]) {
    const source = readFileSync(new URL(relative, import.meta.url), "utf8");
    assert.equal(
      source.includes(JSON.stringify(PRODUCT_VERSION)),
      false,
      `${relative} duplicates the product version`,
    );
  }
});

test("product version authority rejects malformed or unrelated manifests", () => {
  for (const source of [
    "{}",
    '{"name":"other","version":"0.1.2"}',
    '{"name":"resume-ir-desktop","version":"v0.1.2"}',
    '{"name":"resume-ir-desktop","version":"01.2.3"}',
    "not-json",
  ]) {
    assert.throws(
      () => parseProductVersionManifest(source),
      /product version manifest is invalid/,
    );
  }
  assert.equal(
    productVersionFromManifest({
      name: "resume-ir-desktop",
      version: "12.34.56",
    }),
    "12.34.56",
  );
});

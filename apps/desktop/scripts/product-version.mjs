import { readFileSync } from "node:fs";

export const PRODUCT_VERSION_SOURCE = "../package.json";
const PRODUCT_MANIFEST_URL = new URL("../package.json", import.meta.url);
const PRODUCT_VERSION_PATTERN =
  /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;

export function parseProductVersionManifest(source) {
  let manifest;
  try {
    manifest = JSON.parse(source);
  } catch {
    throw new Error("product version manifest is invalid");
  }
  return productVersionFromManifest(manifest);
}

export function productVersionFromManifest(manifest) {
  if (
    manifest?.name !== "resume-ir-desktop" ||
    typeof manifest.version !== "string" ||
    !PRODUCT_VERSION_PATTERN.test(manifest.version)
  ) {
    throw new Error("product version manifest is invalid");
  }
  return manifest.version;
}

export function readProductVersion() {
  return parseProductVersionManifest(
    readFileSync(PRODUCT_MANIFEST_URL, "utf8"),
  );
}

export const PRODUCT_VERSION = readProductVersion();

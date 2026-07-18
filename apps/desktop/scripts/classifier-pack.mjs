import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  readFile,
  rename,
  rm,
} from "node:fs/promises";
import path from "node:path";

const SCHEMA_VERSION = "resume-ir.desktop-classifier-model-pack.v1";
const CLASSIFIER_EPOCH = "precision_first_v4";
const FEATURE_CONTRACT = "bounded_normalized_text_plus_structure_v1";
const DISTRIBUTION_SCOPE = "user_authorized_internal_test";
const MODEL_ROLE = "linear_promotion_model";
const MODEL_FILE = "linear-promotion-model.json";
const MAX_MODEL_BYTES = 32 * 1024 * 1024;
const SHA256_PATTERN = /^[a-f0-9]{64}$/;

function hasExactKeys(value, expected) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const actual = Object.keys(value).sort();
  return actual.length === expected.length && actual.every((key, index) => key === expected[index]);
}

export function validateClassifierPackManifest(manifest) {
  if (
    !hasExactKeys(manifest, [
      "classifier_epoch",
      "distribution_scope",
      "feature_contract",
      "files",
      "network_access",
      "schema_version",
    ].sort()) ||
    manifest.schema_version !== SCHEMA_VERSION ||
    manifest.classifier_epoch !== CLASSIFIER_EPOCH ||
    manifest.feature_contract !== FEATURE_CONTRACT ||
    manifest.distribution_scope !== DISTRIBUTION_SCOPE ||
    manifest.network_access !== "disabled" ||
    !Array.isArray(manifest.files) ||
    manifest.files.length !== 1
  ) {
    throw new Error("classifier model manifest contract is invalid");
  }
  const [model] = manifest.files;
  if (
    !hasExactKeys(model, ["bytes", "file", "role", "sha256"]) ||
    model.role !== MODEL_ROLE ||
    model.file !== MODEL_FILE ||
    path.basename(model.file) !== model.file ||
    !Number.isSafeInteger(model.bytes) ||
    model.bytes <= 0 ||
    model.bytes > MAX_MODEL_BYTES ||
    typeof model.sha256 !== "string" ||
    !SHA256_PATTERN.test(model.sha256)
  ) {
    throw new Error("classifier model manifest file contract is invalid");
  }
  return manifest;
}

async function readDirectRegularFile(file, label) {
  let metadata;
  try {
    metadata = await lstat(file);
  } catch {
    throw new Error(`${label} is missing`);
  }
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    throw new Error(`${label} must be a regular non-symlink file`);
  }
  return metadata;
}

async function sha256(file) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest("hex");
}

async function readManifest(file, label) {
  await readDirectRegularFile(file, label);
  try {
    return validateClassifierPackManifest(JSON.parse(await readFile(file, "utf8")));
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("classifier model manifest is not valid JSON");
    }
    throw error;
  }
}

export async function stageClassifierResourcePack(plan) {
  let sourceRoot;
  try {
    sourceRoot = await lstat(plan.sourcePackRoot);
  } catch {
    throw new Error("classifier model source is missing");
  }
  if (!sourceRoot.isDirectory() || sourceRoot.isSymbolicLink()) {
    throw new Error("classifier model source must be a regular directory");
  }
  const sourceManifestPath = path.join(plan.sourcePackRoot, "runtime-pack.json");
  const expected = await readManifest(plan.expectedManifest, "expected classifier manifest");
  const source = await readManifest(sourceManifestPath, "source classifier manifest");
  if (JSON.stringify(source) !== JSON.stringify(expected)) {
    throw new Error("classifier model source does not match reviewed manifest");
  }

  const [model] = expected.files;
  const sourceModel = path.join(plan.sourcePackRoot, model.file);
  const sourceMetadata = await readDirectRegularFile(sourceModel, "classifier model artifact");
  if (sourceMetadata.size !== model.bytes || (await sha256(sourceModel)) !== model.sha256) {
    throw new Error("classifier model artifact does not match manifest");
  }

  const parent = path.dirname(plan.destination);
  const temporary = path.join(
    parent,
    `${path.basename(plan.destination)}.tmp-${process.pid}-${Date.now()}`,
  );
  const backup = path.join(
    parent,
    `${path.basename(plan.destination)}.old-${process.pid}-${Date.now()}`,
  );
  await mkdir(parent, { recursive: true });
  await rm(temporary, { recursive: true, force: true });
  await mkdir(temporary, { mode: 0o700 });
  try {
    await copyFile(plan.expectedManifest, path.join(temporary, "runtime-pack.json"));
    await copyFile(sourceModel, path.join(temporary, model.file));
    await chmod(path.join(temporary, "runtime-pack.json"), 0o644);
    await chmod(path.join(temporary, model.file), 0o644);

    const staged = await readManifest(
      path.join(temporary, "runtime-pack.json"),
      "staged classifier manifest",
    );
    const stagedModel = path.join(temporary, staged.files[0].file);
    const stagedMetadata = await readDirectRegularFile(stagedModel, "staged classifier model");
    if (
      JSON.stringify(staged) !== JSON.stringify(expected) ||
      stagedMetadata.size !== model.bytes ||
      (await sha256(stagedModel)) !== model.sha256
    ) {
      throw new Error("staged classifier model does not match reviewed composition");
    }

    let previous = false;
    try {
      await rename(plan.destination, backup);
      previous = true;
    } catch (error) {
      if (!error || error.code !== "ENOENT") throw error;
    }
    try {
      await rename(temporary, plan.destination);
    } catch (error) {
      if (previous) await rename(backup, plan.destination);
      throw error;
    }
    await rm(backup, { recursive: true, force: true });
  } finally {
    await rm(temporary, { recursive: true, force: true });
    await rm(backup, { recursive: true, force: true });
  }
  return Object.freeze({
    schema_version: "resume-ir.classifier-resource-stage.v1",
    target_triple: plan.targetTriple,
    resource_file_count: 2,
  });
}

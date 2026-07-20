import { chmod, mkdir } from "node:fs/promises";
import path from "node:path";

import { stageClassifierResourcePack } from "../classifier-pack.mjs";
import { stageOcrResourcePack } from "../ocr-pack.mjs";
import {
  createDesktopCompositionPlan,
  stageEmbeddingResourcePack,
} from "../prepare-sidecar.mjs";
import { fail } from "./core.mjs";

const TARGET_TRIPLE = "aarch64-apple-darwin";
const PACKS = Object.freeze({
  classifier: "resume-ir-classifier-model-pack",
  embedding: "resume-ir-native-e5-qint8-pack",
  ocr: "resume-ir-macos-ocr-runtime-pack",
});

export async function stageImmutableRuntimePacks(
  { immutableRepoRoot, sourceRepoRoot },
  dependencies = {},
) {
  if (
    !path.isAbsolute(immutableRepoRoot ?? "") ||
    !path.isAbsolute(sourceRepoRoot ?? "") ||
    immutableRepoRoot === sourceRepoRoot
  ) {
    fail("immutable_build_source_invalid");
  }
  const cacheRoot = path.join(immutableRepoRoot, ".cache");
  await mkdir(cacheRoot, { mode: 0o700 });
  await chmod(cacheRoot, 0o700);
  const plan = (
    dependencies.createPlan ?? createDesktopCompositionPlan
  )({
    repoRoot: immutableRepoRoot,
    sourceClassifierPackRoot: path.join(
      sourceRepoRoot,
      ".cache",
      PACKS.classifier,
    ),
    sourceOcrPackRoot: path.join(sourceRepoRoot, ".cache", PACKS.ocr),
    sourcePackRoot: path.join(sourceRepoRoot, ".cache", PACKS.embedding),
    targetTriple: TARGET_TRIPLE,
  });
  await (dependencies.stageEmbedding ?? stageEmbeddingResourcePack)({
    ...plan.resourcePack,
    destination: path.join(cacheRoot, PACKS.embedding),
  });
  await (dependencies.stageOcr ?? stageOcrResourcePack)({
    ...plan.ocrResourcePack,
    destination: path.join(cacheRoot, PACKS.ocr),
  });
  await (dependencies.stageClassifier ?? stageClassifierResourcePack)({
    ...plan.classifierResourcePack,
    destination: path.join(cacheRoot, PACKS.classifier),
  });
}

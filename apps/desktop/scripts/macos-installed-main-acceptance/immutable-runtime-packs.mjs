import { chmod, mkdir } from "node:fs/promises";
import path from "node:path";

import { stageClassifierResourcePack } from "../classifier-pack.mjs";
import { stageOcrResourcePack } from "../ocr-pack.mjs";
import { stageMacosPdfiumStaticBuildPack } from "../macos-pdfium-build-pack-stage.mjs";
import {
  createDesktopCompositionPlan,
  stageCoreMlResourcePack,
  stageEmbeddingResourcePack,
} from "../prepare-sidecar.mjs";
import { fail } from "./core.mjs";

const TARGET_TRIPLE = "aarch64-apple-darwin";
const PACKS = Object.freeze({
  classifier: "resume-ir-classifier-model-pack",
  coreml: "resume-ir-coreml-runtime-pack",
  embedding: "resume-ir-native-e5-qint8-pack",
  ocr: "resume-ir-macos-ocr-runtime-pack",
  pdfium: "resume-ir-macos-pdfium-static-pack",
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
  await mkdir(cacheRoot, { recursive: true, mode: 0o700 });
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
    sourceCoreMlPackRoot: path.join(sourceRepoRoot, ".cache", PACKS.coreml),
    sourceOcrPackRoot: path.join(sourceRepoRoot, ".cache", PACKS.ocr),
    sourcePackRoot: path.join(sourceRepoRoot, ".cache", PACKS.embedding),
    sourcePdfiumPackRoot: path.join(sourceRepoRoot, ".cache", PACKS.pdfium),
    targetTriple: TARGET_TRIPLE,
  });
  await (dependencies.stageEmbedding ?? stageEmbeddingResourcePack)({
    ...plan.resourcePack,
    destination: path.join(cacheRoot, PACKS.embedding),
  });
  await (dependencies.stageCoreMl ?? stageCoreMlResourcePack)({
    ...plan.coreMlResourcePack,
    destination: path.join(cacheRoot, PACKS.coreml),
  });
  await (dependencies.stageOcr ?? stageOcrResourcePack)({
    ...plan.ocrResourcePack,
    destination: path.join(cacheRoot, PACKS.ocr),
  });
  await (dependencies.stageClassifier ?? stageClassifierResourcePack)({
    ...plan.classifierResourcePack,
    destination: path.join(cacheRoot, PACKS.classifier),
  });
  await (dependencies.stagePdfium ?? stageMacosPdfiumStaticBuildPack)({
    ...plan.pdfiumResourcePack,
    destination: path.join(cacheRoot, PACKS.pdfium),
  });
}

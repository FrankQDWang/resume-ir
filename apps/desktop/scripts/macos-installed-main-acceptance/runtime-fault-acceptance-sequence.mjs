import { exactKeys, fail } from "./core.mjs";
import {
  OPTIONAL_RUNTIME_NAMES,
  RUNTIME_FAULT_CASES,
} from "./native-runtime-fault-plan.mjs";
import { syntheticCanaryImportRequest } from "./synthetic-canary.mjs";

const CAPABILITY_NAMES = Object.freeze([
  "keyword_search",
  "detail",
  "semantic_search",
  "hybrid_search",
  "text_import",
  "ocr_import",
  "index_publication",
]);

const BEHAVIOR_RUNTIME_BY_CELL = Object.freeze({
  embedding_missing: "embedding",
  ocr_missing: "ocr",
  classifier_missing: "classifier",
});

function expectedCapabilities(expectedReasons) {
  const embedding = Object.hasOwn(expectedReasons, "embedding");
  const ocr = Object.hasOwn(expectedReasons, "ocr");
  const classifier = Object.hasOwn(expectedReasons, "classifier");
  const available = () => ({ state: "available", reason: null });
  const unavailable = (reason) => ({ state: "unavailable", reason });
  return {
    keyword_search: available(),
    detail: available(),
    semantic_search: embedding
      ? unavailable("embedding_unavailable")
      : available(),
    hybrid_search: embedding
      ? { state: "degraded", reason: "embedding_unavailable" }
      : available(),
    text_import: classifier
      ? unavailable("classifier_unavailable")
      : embedding
        ? unavailable("embedding_unavailable")
        : available(),
    ocr_import: classifier
      ? unavailable("classifier_unavailable")
      : embedding
        ? unavailable("embedding_unavailable")
        : ocr
          ? unavailable("ocr_unavailable")
          : available(),
    index_publication: classifier
      ? unavailable("classifier_unavailable")
      : embedding
        ? unavailable("embedding_unavailable")
        : available(),
  };
}

function normalizedBehaviorEvidence(runtimeName, value) {
  const contracts = {
    embedding: [
      "detailAvailable",
      "hybridLexicalPartial",
      "keywordAvailable",
      "selectionPreserved",
    ],
    ocr: ["backlogRetained", "claimGateStable", "visibleEpochPreserved"],
    classifier: [
      "classifierEpochPreserved",
      "importRejectedBeforeClaim",
      "visibleEpochPreserved",
    ],
  };
  const keys = contracts[runtimeName];
  if (!keys || !exactKeys(value, keys) || keys.some((key) => value[key] !== true)) {
    fail("optional_runtime_behavior_evidence_invalid");
  }
  return Object.fromEntries(
    keys.map((key) => [key.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`), true]),
  );
}

function publicInstalledEvidence(definition, observed, behaviors) {
  const expected = definition.expectedReasons;
  const expectedCapabilityMatrix = expectedCapabilities(expected);
  const behaviorRuntime = BEHAVIOR_RUNTIME_BY_CELL[definition.cell] ?? null;
  const behaviorRuntimes = behaviorRuntime ? [behaviorRuntime] : [];
  if (
    !exactKeys(observed, [
      "cell",
      "capabilities",
      "coreState",
      "evidenceSource",
      "optionalRuntimes",
      "v29AuthorityPreserved",
    ]) ||
    observed.cell !== definition.cell ||
    observed.evidenceSource !== "installed_app" ||
    observed.coreState !== "ready" ||
    observed.v29AuthorityPreserved !== true ||
    !exactKeys(observed.optionalRuntimes, OPTIONAL_RUNTIME_NAMES) ||
    !OPTIONAL_RUNTIME_NAMES.every((name) => {
      const expectedReason = expected[name] ?? null;
      const runtime = observed.optionalRuntimes[name];
      return expectedReason === null
        ? exactKeys(runtime, ["state", "reason"]) &&
            runtime.state === "available" &&
            runtime.reason === null
        : exactKeys(runtime, ["state", "reason"]) &&
            runtime.state === "unavailable" &&
            runtime.reason === expectedReason;
    }) ||
    !exactKeys(observed.capabilities, CAPABILITY_NAMES) ||
    !CAPABILITY_NAMES.every(
      (name) =>
        JSON.stringify(observed.capabilities[name]) ===
        JSON.stringify(expectedCapabilityMatrix[name]),
    ) ||
    !exactKeys(behaviors, behaviorRuntimes)
  ) {
    fail("optional_runtime_fault_evidence_invalid");
  }
  const behaviorEvidence = Object.fromEntries(
    behaviorRuntimes.map((name) => [
      name,
      normalizedBehaviorEvidence(name, behaviors[name]),
    ]),
  );
  return Object.freeze({
    evidence_source: "installed_app",
    expected_runtime_reasons: { ...expected },
    native_mutation_applied: true,
    native_observation: {
      core_state: "ready",
      optional_runtimes: observed.optionalRuntimes,
      capabilities: expectedCapabilityMatrix,
      v29_authority_preserved: true,
    },
    behavior_evidence: behaviorEvidence,
    validation_scope: behaviorRuntime
      ? "status_capability_and_behavior"
      : "status_capability_matrix_only",
  });
}

function publicProjectedEvidence(definition, observed) {
  if (
    !exactKeys(observed, [
      "cell",
      "evidenceSource",
      "expectedReasons",
      "nativeMutationApplied",
      "projectionReason",
    ]) ||
    observed.cell !== definition.cell ||
    observed.evidenceSource !== "deterministic_contract_projection" ||
    JSON.stringify(observed.expectedReasons) !==
      JSON.stringify(definition.expectedReasons) ||
    observed.nativeMutationApplied !== false ||
    observed.projectionReason !== "post_attestation_failure_surface_absent"
  ) {
    fail("optional_runtime_fault_evidence_invalid");
  }
  return Object.freeze({
    evidence_source: "deterministic_contract_projection",
    expected_runtime_reasons: { ...definition.expectedReasons },
    native_mutation_applied: false,
    native_observation: null,
    behavior_evidence: null,
    projection_reason: observed.projectionReason,
  });
}

async function prepareBehaviorInputs(runtime, launchVerified, workspace, definition) {
  const behaviorRuntime = BEHAVIOR_RUNTIME_BY_CELL[definition.cell] ?? null;
  let canary;
  let embeddingSelection;
  if (behaviorRuntime === "embedding") {
    const healthySession = await launchVerified(workspace);
    const ready = await runtime.waitForReady(healthySession);
    canary = await runtime.createSyntheticCanary(workspace);
    await runtime.importSyntheticCanary(healthySession, canary, ready);
    embeddingSelection = await runtime.captureFaultWitness(healthySession);
    await runtime.quitApp(healthySession);
    await runtime.assertZeroResidue(healthySession);
  } else if (behaviorRuntime === "classifier") {
    canary = await runtime.createSyntheticCanary(workspace);
  }
  const ocrFixture = behaviorRuntime === "ocr"
    ? await runtime.prepareOcrFaultFixture(workspace)
    : null;
  return { canary, embeddingSelection, ocrFixture };
}

async function validateBehavior(
  runtime,
  session,
  definition,
  { canary, embeddingSelection, ocrFixture },
) {
  const evidence = {};
  const behaviorRuntime = BEHAVIOR_RUNTIME_BY_CELL[definition.cell] ?? null;
  if (behaviorRuntime === "embedding") {
    evidence.embedding = await runtime.validateEmbeddingFaultBehavior(
      session,
      embeddingSelection,
      definition.cell,
    );
  }
  if (behaviorRuntime === "ocr") {
    evidence.ocr = await runtime.validateOcrFaultBehavior(
      session,
      ocrFixture,
      definition.cell,
    );
  }
  if (behaviorRuntime === "classifier") {
    evidence.classifier = await runtime.validateClassifierFaultBehavior(
      session,
      syntheticCanaryImportRequest(canary),
      definition.cell,
    );
  }
  return evidence;
}

export async function executeRuntimeFaultAcceptance(runtime, launchVerified) {
  const evidence = {};
  for (const definition of RUNTIME_FAULT_CASES) {
    const workspace = await runtime.createClone(definition.cell);
    if (definition.evidenceSource === "deterministic_contract_projection") {
      const fault = await runtime.prepareFaultCell(workspace, definition.cell);
      await runtime.activateFaultCell(workspace);
      evidence[definition.cell] = publicProjectedEvidence(
        definition,
        await runtime.validateProjectedRuntimeFault(definition.cell),
      );
      await runtime.releaseFaultCell(fault);
      continue;
    }

    const behaviorInputs = await prepareBehaviorInputs(
      runtime,
      launchVerified,
      workspace,
      definition,
    );
    const fault = await runtime.prepareFaultCell(workspace, definition.cell);
    const session = await launchVerified(workspace);
    const observed = await runtime.validateRuntimeFaultCase(
      session,
      definition.cell,
    );
    evidence[definition.cell] = publicInstalledEvidence(
      definition,
      observed,
      await validateBehavior(runtime, session, definition, behaviorInputs),
    );
    await runtime.quitApp(session);
    await runtime.assertZeroResidue(session);
    await runtime.releaseFaultCell(fault);
  }
  runtime.validateFaultCoverage();
  return Object.freeze(evidence);
}

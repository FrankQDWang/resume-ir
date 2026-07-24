import { exactKeys, fail } from "./core.mjs";
import { validDaemonStatus } from "./ipc-contracts.mjs";

export const OPTIONAL_RUNTIME_NAMES = Object.freeze([
  "embedding",
  "ocr",
  "classifier",
]);

const nativeMutation = (target, activation) =>
  Object.freeze({ activation, target });

const nativeCase = (cell, expectedReasons, mutations) =>
  Object.freeze({
    cell,
    evidenceSource: "installed_app",
    expectedReasons: Object.freeze({ ...expectedReasons }),
    mutations: Object.freeze(mutations),
  });

const projectedCase = (cell, expectedReasons, explanation) =>
  Object.freeze({
    cell,
    evidenceSource: "deterministic_contract_projection",
    expectedReasons: Object.freeze({ ...expectedReasons }),
    explanation,
    mutations: Object.freeze([]),
  });

export const RUNTIME_FAULT_CASES = Object.freeze([
  nativeCase("embedding_missing", { embedding: "missing" }, [
    nativeMutation("embedding", "missing"),
  ]),
  nativeCase("embedding_invalid", { embedding: "invalid" }, [
    nativeMutation("embedding", "invalid"),
  ]),
  nativeCase("embedding_start_failed", { embedding: "start_failed" }, [
    nativeMutation("embedding", "deny_execution_after_attestation"),
  ]),
  nativeCase("ocr_missing", { ocr: "missing" }, [
    nativeMutation("pdfRenderer", "missing"),
  ]),
  nativeCase("ocr_invalid", { ocr: "invalid" }, [
    nativeMutation("pdfRenderer", "invalid"),
  ]),
  nativeCase("ocr_start_failed", { ocr: "start_failed" }, [
    nativeMutation("ocrEngine", "deny_execution_after_attestation"),
  ]),
  nativeCase("classifier_missing", { classifier: "missing" }, [
    nativeMutation("classifierModel", "missing"),
  ]),
  nativeCase("classifier_invalid", { classifier: "invalid" }, [
    nativeMutation("classifierModel", "invalid"),
  ]),
  projectedCase(
    "classifier_start_failed",
    { classifier: "start_failed" },
    "classifier startup has no independently mutable post-attestation process boundary",
  ),
  nativeCase(
    "embedding_ocr_missing",
    { embedding: "missing", ocr: "missing" },
    [
      nativeMutation("embedding", "missing"),
      nativeMutation("pdfRenderer", "missing"),
    ],
  ),
  nativeCase(
    "embedding_classifier_invalid",
    { embedding: "invalid", classifier: "invalid" },
    [
      nativeMutation("embedding", "invalid"),
      nativeMutation("classifierModel", "invalid"),
    ],
  ),
  nativeCase(
    "all_runtimes_missing",
    { embedding: "missing", ocr: "missing", classifier: "missing" },
    [
      nativeMutation("embedding", "missing"),
      nativeMutation("pdfRenderer", "missing"),
      nativeMutation("classifierModel", "missing"),
    ],
  ),
]);

const CASES_BY_NAME = new Map(RUNTIME_FAULT_CASES.map((entry) => [entry.cell, entry]));

export const REQUIRED_FAULT_CELLS = Object.freeze([
  "slow_initialization",
  ...RUNTIME_FAULT_CASES.map(({ cell }) => cell),
]);

export function runtimeFaultCase(cell) {
  const definition = CASES_BY_NAME.get(cell);
  if (!definition) fail("installed_fault_cell_invalid");
  return definition;
}

export function expectedRuntimeReason(definition, runtimeName) {
  if (!OPTIONAL_RUNTIME_NAMES.includes(runtimeName)) {
    fail("installed_fault_cell_invalid");
  }
  return definition.expectedReasons[runtimeName] ?? null;
}

export function runtimeFaultStatusMatches(status, definition) {
  if (
    !validDaemonStatus(status) ||
    status?.process_state !== "ready" ||
    status?.core?.state !== "ready" ||
    status?.core?.reason !== null ||
    status?.error !== null ||
    status?.repair_progress !== null ||
    status?.index_health !== "ready" ||
    status?.snapshot_present !== true ||
    !exactKeys(status?.optional_runtimes, OPTIONAL_RUNTIME_NAMES)
  ) {
    return false;
  }
  return OPTIONAL_RUNTIME_NAMES.every((runtimeName) => {
    const expected = expectedRuntimeReason(definition, runtimeName);
    const runtime = status.optional_runtimes[runtimeName];
    return expected === null
      ? runtime.state === "available" && runtime.reason === null
      : runtime.state === "unavailable" && runtime.reason === expected;
  });
}

export function projectedRuntimeFaultEvidence(cell) {
  const definition = runtimeFaultCase(cell);
  if (definition.evidenceSource !== "deterministic_contract_projection") {
    fail("installed_fault_cell_invalid");
  }
  return Object.freeze({
    cell: definition.cell,
    evidenceSource: definition.evidenceSource,
    expectedReasons: definition.expectedReasons,
    nativeMutationApplied: false,
    projectionReason: "post_attestation_failure_surface_absent",
  });
}

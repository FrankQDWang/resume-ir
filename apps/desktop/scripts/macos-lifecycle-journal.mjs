import { createHash, randomBytes } from "node:crypto";

import { validateInstallReceipt } from "./macos-install-receipt.mjs";
import {
  createOwnerEvidence,
  ownerEvidencePath,
  persistOwnerEvidence,
  readOwnerEvidence,
  removeOwnerEvidence,
} from "./macos-owner-evidence-store.mjs";

export const LIFECYCLE_JOURNAL_FILE =
  "resume-ir.desktop-lifecycle-journal.v1.json";
export const LIFECYCLE_JOURNAL_SCHEMA =
  "resume-ir.macos-lifecycle-journal.v1";

const EXPECTED_BUNDLE_ID = "local.resume-ir.desktop";
const SUPPORTED_TARGET = "aarch64-apple-darwin";
const MAX_JOURNAL_BYTES = 16 * 1024;
const DIGEST = /^[a-f0-9]{64}$/;
const TRANSACTION_ID = /^[a-f0-9]{32}$/;
const PHASES = Object.freeze({
  install: new Set([
    "install_prepared",
    "install_before_stage_publish",
    "install_stage_ready",
    "install_before_stage_cleanup",
    "install_stage_tombstoned",
    "install_before_target_cleanup",
    "install_target_tombstoned",
    "install_before_promotion",
    "install_target_promoted",
    "install_before_receipt_commit",
    "install_receipt_committed",
    "install_complete",
  ]),
  reinstall: new Set([
    "reinstall_prepared",
    "reinstall_before_stage_publish",
    "reinstall_stage_ready",
    "reinstall_before_stage_cleanup",
    "reinstall_stage_tombstoned",
    "reinstall_before_backup",
    "reinstall_backup_ready",
    "reinstall_before_promotion",
    "reinstall_target_promoted",
    "reinstall_before_receipt_commit",
    "reinstall_receipt_committed",
    "reinstall_before_backup_cleanup",
    "reinstall_backup_tombstoned",
    "reinstall_before_recovery_target_cleanup",
    "reinstall_target_tombstoned",
    "reinstall_before_restore",
    "reinstall_complete",
  ]),
  uninstall: new Set([
    "uninstall_prepared",
    "uninstall_before_quarantine",
    "uninstall_quarantined",
    "uninstall_before_receipt_removal",
    "uninstall_receipt_removed",
    "uninstall_before_quarantine_cleanup",
    "uninstall_quarantine_tombstoned",
    "uninstall_before_receipt_restore",
    "uninstall_before_restore",
    "uninstall_complete",
  ]),
});

const JOURNAL_KEYS = Object.freeze([
  "schema_version",
  "transaction_id",
  "operation",
  "phase",
  "bundle_id",
  "target_triple",
  "old_version",
  "new_version",
  "old_composition_digest",
  "new_composition_digest",
  "old_receipt",
  "old_receipt_digest",
  "new_receipt",
  "new_receipt_digest",
]);

function journalError(message = "lifecycle journal is invalid") {
  return new Error(message);
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value)) === JSON.stringify(expected)
  );
}

function digestReceipt(receipt) {
  return createHash("sha256").update(JSON.stringify(receipt)).digest("hex");
}

function validateEvidenceSide({
  version,
  compositionDigest,
  receipt,
  receiptDigest,
  required,
}) {
  if (!required) {
    if (
      version !== null ||
      compositionDigest !== null ||
      receipt !== null ||
      receiptDigest !== null
    ) {
      throw journalError();
    }
    return;
  }
  let validatedReceipt;
  try {
    validatedReceipt = validateInstallReceipt(receipt);
  } catch {
    throw journalError();
  }
  if (
    version !== validatedReceipt.version ||
    compositionDigest !== validatedReceipt.composition_digest ||
    !DIGEST.test(compositionDigest ?? "") ||
    !DIGEST.test(receiptDigest ?? "") ||
    receiptDigest !== digestReceipt(validatedReceipt)
  ) {
    throw journalError();
  }
}

export function validateLifecycleJournal(journal) {
  if (
    !exactKeys(journal, JOURNAL_KEYS) ||
    journal.schema_version !== LIFECYCLE_JOURNAL_SCHEMA ||
    !TRANSACTION_ID.test(journal.transaction_id ?? "") ||
    !Object.hasOwn(PHASES, journal.operation) ||
    !PHASES[journal.operation].has(journal.phase) ||
    journal.bundle_id !== EXPECTED_BUNDLE_ID ||
    journal.target_triple !== SUPPORTED_TARGET
  ) {
    throw journalError();
  }

  const hasOld = journal.operation !== "install";
  const hasNew = journal.operation !== "uninstall";
  validateEvidenceSide({
    version: journal.old_version,
    compositionDigest: journal.old_composition_digest,
    receipt: journal.old_receipt,
    receiptDigest: journal.old_receipt_digest,
    required: hasOld,
  });
  validateEvidenceSide({
    version: journal.new_version,
    compositionDigest: journal.new_composition_digest,
    receipt: journal.new_receipt,
    receiptDigest: journal.new_receipt_digest,
    required: hasNew,
  });
  if (
    journal.operation === "reinstall" &&
    journal.old_version !== journal.new_version
  ) {
    throw journalError();
  }
  return journal;
}

function canonicalJournal({
  transactionId,
  operation,
  phase,
  oldVersion,
  newVersion,
  oldCompositionDigest,
  newCompositionDigest,
  oldReceipt,
  newReceipt,
}) {
  return {
    schema_version: LIFECYCLE_JOURNAL_SCHEMA,
    transaction_id: transactionId,
    operation,
    phase,
    bundle_id: EXPECTED_BUNDLE_ID,
    target_triple: SUPPORTED_TARGET,
    old_version: oldVersion ?? null,
    new_version: newVersion ?? null,
    old_composition_digest: oldCompositionDigest ?? null,
    new_composition_digest: newCompositionDigest ?? null,
    old_receipt: oldReceipt ?? null,
    old_receipt_digest: oldReceipt ? digestReceipt(oldReceipt) : null,
    new_receipt: newReceipt ?? null,
    new_receipt_digest: newReceipt ? digestReceipt(newReceipt) : null,
  };
}

export function createLifecycleJournal({
  transactionId = randomBytes(16).toString("hex"),
  operation,
  phase,
  oldVersion,
  newVersion,
  oldCompositionDigest,
  newCompositionDigest,
  oldReceipt,
  newReceipt,
}) {
  return validateLifecycleJournal(
    canonicalJournal({
      transactionId,
      operation,
      phase,
      oldVersion,
      newVersion,
      oldCompositionDigest,
      newCompositionDigest,
      oldReceipt,
      newReceipt,
    }),
  );
}

export function advanceLifecycleJournal({ journal, phase }) {
  validateLifecycleJournal(journal);
  return validateLifecycleJournal({ ...journal, phase });
}

function transactionProjection(journal) {
  return {
    schema_version: journal.schema_version,
    transaction_id: journal.transaction_id,
    operation: journal.operation,
    bundle_id: journal.bundle_id,
    target_triple: journal.target_triple,
    old_version: journal.old_version,
    new_version: journal.new_version,
    old_composition_digest: journal.old_composition_digest,
    new_composition_digest: journal.new_composition_digest,
    old_receipt: journal.old_receipt,
    old_receipt_digest: journal.old_receipt_digest,
    new_receipt: journal.new_receipt,
    new_receipt_digest: journal.new_receipt_digest,
  };
}

export function lifecycleJournalPath(applicationSupportRoot) {
  return ownerEvidencePath(applicationSupportRoot, LIFECYCLE_JOURNAL_FILE);
}

export async function readLifecycleJournal({
  applicationSupportRoot,
  allowMissing = false,
}) {
  const evidence = await readOwnerEvidence({
    applicationSupportRoot,
    fileName: LIFECYCLE_JOURNAL_FILE,
    maxBytes: MAX_JOURNAL_BYTES,
    validate: validateLifecycleJournal,
    label: "lifecycle journal",
    allowMissing,
  });
  return evidence?.value;
}

export async function persistLifecycleJournal({
  applicationSupportRoot,
  journal,
  operations = {},
}) {
  const existing = await readLifecycleJournal({
    applicationSupportRoot,
    allowMissing: true,
  });
  if (
    existing &&
    exactKeys(journal, JOURNAL_KEYS) &&
    JSON.stringify(transactionProjection(existing)) !==
      JSON.stringify(transactionProjection(journal))
  ) {
    throw journalError("lifecycle journal transaction does not match");
  }
  const validated = validateLifecycleJournal(journal);
  if (!existing) {
    return createOwnerEvidence({
      applicationSupportRoot,
      fileName: LIFECYCLE_JOURNAL_FILE,
      value: validated,
      maxBytes: MAX_JOURNAL_BYTES,
      validate: validateLifecycleJournal,
      label: "lifecycle journal",
      operations,
    });
  }
  return persistOwnerEvidence({
    applicationSupportRoot,
    fileName: LIFECYCLE_JOURNAL_FILE,
    value: validated,
    maxBytes: MAX_JOURNAL_BYTES,
    validate: validateLifecycleJournal,
    label: "lifecycle journal",
    operations,
  });
}

export async function removeLifecycleJournal({
  applicationSupportRoot,
  expectedJournal,
  operations = {},
}) {
  validateLifecycleJournal(expectedJournal);
  return removeOwnerEvidence({
    applicationSupportRoot,
    fileName: LIFECYCLE_JOURNAL_FILE,
    expectedValue: expectedJournal,
    maxBytes: MAX_JOURNAL_BYTES,
    validate: validateLifecycleJournal,
    label: "lifecycle journal",
    operations,
  });
}

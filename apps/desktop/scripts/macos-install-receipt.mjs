import {
  createOwnerEvidence,
  defaultApplicationSupportRoot,
  ownerEvidencePath,
  persistOwnerEvidence,
  readOwnerEvidence,
  removeOwnerEvidence,
} from "./macos-owner-evidence-store.mjs";
import { validateSourceIdentity } from "./macos-source-identity.mjs";

export const INSTALL_RECEIPT_FILE = "resume-ir.install-receipt.v3.json";
export const INSTALL_RECEIPT_SCHEMA = "resume-ir.macos-install-receipt.v3";
export { defaultApplicationSupportRoot };

const EXPECTED_BUNDLE_ID = "local.resume-ir.desktop";
const SUPPORTED_TARGET = "aarch64-apple-darwin";
const MAX_RECEIPT_BYTES = 4 * 1024;
const DIGEST = /^[a-f0-9]{64}$/;
const VERSION = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;
const MIN_VERSION = [0, 1, 2];

function receiptError(message = "install receipt is invalid") {
  return new Error(message);
}

function supportedVersion(version) {
  if (!VERSION.test(version ?? "")) return false;
  const parts = version.split(".").map(Number);
  if (parts.some((part) => !Number.isSafeInteger(part))) return false;
  for (let index = 0; index < parts.length; index += 1) {
    if (parts[index] !== MIN_VERSION[index]) {
      return parts[index] > MIN_VERSION[index];
    }
  }
  return true;
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value)) === JSON.stringify(expected)
  );
}

export function validateInstallReceipt(receipt) {
  try {
    validateSourceIdentity(receipt?.source);
  } catch {
    throw receiptError();
  }
  if (
    !exactKeys(receipt, [
      "schema_version",
      "bundle_id",
      "version",
      "target_triple",
      "source",
      "composition_digest",
      "dmg_sha256",
    ]) ||
    receipt.schema_version !== INSTALL_RECEIPT_SCHEMA ||
    receipt.bundle_id !== EXPECTED_BUNDLE_ID ||
    !supportedVersion(receipt.version) ||
    receipt.target_triple !== SUPPORTED_TARGET ||
    !DIGEST.test(receipt.composition_digest) ||
    !DIGEST.test(receipt.dmg_sha256)
  ) {
    throw receiptError();
  }
  return receipt;
}

export function installReceiptPath(applicationSupportRoot) {
  return ownerEvidencePath(applicationSupportRoot, INSTALL_RECEIPT_FILE);
}

export function createInstallReceipt({ composition, dmgSha256 }) {
  let source;
  try {
    source = validateSourceIdentity(composition?.source);
  } catch {
    throw receiptError();
  }
  if (
    composition === null ||
    typeof composition !== "object" ||
    composition.bundle_id !== EXPECTED_BUNDLE_ID ||
    !supportedVersion(composition.version) ||
    composition.target_triple !== SUPPORTED_TARGET ||
    !DIGEST.test(composition.composition_digest ?? "") ||
    !DIGEST.test(dmgSha256 ?? "")
  ) {
    throw receiptError();
  }
  return validateInstallReceipt({
    schema_version: INSTALL_RECEIPT_SCHEMA,
    bundle_id: composition.bundle_id,
    version: composition.version,
    target_triple: composition.target_triple,
    source,
    composition_digest: composition.composition_digest,
    dmg_sha256: dmgSha256,
  });
}

export function verifyInstallReceipt({ receipt, composition }) {
  validateInstallReceipt(receipt);
  if (
    composition?.bundle_id !== receipt.bundle_id ||
    composition?.version !== receipt.version ||
    composition?.target_triple !== receipt.target_triple ||
    JSON.stringify(composition?.source) !== JSON.stringify(receipt.source) ||
    composition?.composition_digest !== receipt.composition_digest
  ) {
    throw receiptError("install receipt does not match bundle composition");
  }
  return receipt;
}

export async function persistInstallReceipt({
  applicationSupportRoot,
  receipt,
  expectedReceipt,
  operations = {},
}) {
  return persistOwnerEvidence({
    applicationSupportRoot,
    fileName: INSTALL_RECEIPT_FILE,
    value: receipt,
    expectedValue: expectedReceipt,
    maxBytes: MAX_RECEIPT_BYTES,
    validate: validateInstallReceipt,
    label: "install receipt",
    operations,
  });
}

export async function createInstallReceiptEvidence({
  applicationSupportRoot,
  receipt,
  operations = {},
}) {
  return createOwnerEvidence({
    applicationSupportRoot,
    fileName: INSTALL_RECEIPT_FILE,
    value: receipt,
    maxBytes: MAX_RECEIPT_BYTES,
    validate: validateInstallReceipt,
    label: "install receipt",
    operations,
  });
}

export async function readInstallReceipt({
  applicationSupportRoot,
  allowMissing = false,
}) {
  const evidence = await readOwnerEvidence({
    applicationSupportRoot,
    fileName: INSTALL_RECEIPT_FILE,
    maxBytes: MAX_RECEIPT_BYTES,
    validate: validateInstallReceipt,
    label: "install receipt",
    allowMissing,
  });
  return evidence?.value;
}

export async function removeInstallReceipt({
  applicationSupportRoot,
  expectedReceipt,
  operations = {},
}) {
  return removeOwnerEvidence({
    applicationSupportRoot,
    fileName: INSTALL_RECEIPT_FILE,
    expectedValue: expectedReceipt,
    maxBytes: MAX_RECEIPT_BYTES,
    validate: validateInstallReceipt,
    label: "install receipt",
    operations,
  });
}

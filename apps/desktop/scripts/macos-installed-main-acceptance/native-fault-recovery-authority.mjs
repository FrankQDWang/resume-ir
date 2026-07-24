import path from "node:path";

import {
  readBundleCompositionEvidence,
  validateBundleCompositionEvidence,
} from "../macos-bundle-composition.mjs";
import {
  readInstallReceipt,
  verifyInstallReceipt,
} from "../macos-install-receipt.mjs";
import { executablePayloadSha256 } from "../verify-bundled-sidecar.mjs";
import { fileSha256 } from "./native-fault-file-ops.mjs";

export const INSTALLED_FAULT_RECOVERY_AUTHORITY_SCHEMA =
  "resume-ir.installed-fault-recovery-authority.v1";

const DIGEST = /^[a-f0-9]{64}$/;
const TARGETS = Object.freeze([
  Object.freeze({
    digest: "sha256_without_code_signature_v1",
    file: "Contents/MacOS/resume-embedding-runtime",
  }),
  Object.freeze({
    digest: "sha256_without_code_signature_v1",
    file: "Contents/MacOS/resume-pdf-render-runtime",
  }),
  Object.freeze({
    digest: "sha256",
    file: "Contents/Resources/ocr/runtime-pack/tesseract",
  }),
  Object.freeze({
    digest: "sha256",
    file:
      "Contents/Resources/classifier/runtime-pack/linear-promotion-model.json",
  }),
]);

function authorityError() {
  return new Error("installed fault recovery authority is invalid");
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value)) === JSON.stringify(expected)
  );
}

function trustedEntries(composition) {
  const entries = new Map(composition.app_files.map((entry) => [entry.file, entry]));
  return TARGETS.map((target) => {
    const entry = entries.get(target.file);
    if (entry?.digest !== target.digest || !DIGEST.test(entry.sha256 ?? "")) {
      throw authorityError();
    }
    return Object.freeze({
      file: target.file,
      digest: target.digest,
      sha256: entry.sha256,
    });
  });
}

export function createInstalledFaultRecoveryAuthority(composition) {
  let validated;
  try {
    validated = validateBundleCompositionEvidence(composition);
  } catch {
    throw authorityError();
  }
  return Object.freeze({
    schema: INSTALLED_FAULT_RECOVERY_AUTHORITY_SCHEMA,
    compositionDigest: validated.composition_digest,
    targets: Object.freeze(trustedEntries(validated)),
  });
}

export function validateInstalledFaultRecoveryAuthority(authority) {
  if (
    !exactKeys(authority, ["schema", "compositionDigest", "targets"]) ||
    authority.schema !== INSTALLED_FAULT_RECOVERY_AUTHORITY_SCHEMA ||
    !DIGEST.test(authority.compositionDigest) ||
    !Array.isArray(authority.targets) ||
    authority.targets.length !== TARGETS.length ||
    !authority.targets.every((entry, index) => {
      const expected = TARGETS[index];
      return (
        exactKeys(entry, ["file", "digest", "sha256"]) &&
        entry.file === expected.file &&
        entry.digest === expected.digest &&
        DIGEST.test(entry.sha256)
      );
    })
  ) {
    throw authorityError();
  }
  return authority;
}

export async function readInstalledFaultRecoveryAuthority({
  appBundle,
  applicationSupportRoot,
}) {
  try {
    const [composition, receipt] = await Promise.all([
      readBundleCompositionEvidence({ appBundle }),
      readInstallReceipt({ applicationSupportRoot }),
    ]);
    verifyInstallReceipt({ receipt, composition });
    return createInstalledFaultRecoveryAuthority(composition);
  } catch {
    throw authorityError();
  }
}

export function trustedRecoveryTarget(authority, appBundle, target) {
  const validated = validateInstalledFaultRecoveryAuthority(authority);
  const relative = path.relative(appBundle, target).replaceAll(path.sep, "/");
  const entry = validated.targets.find((candidate) => candidate.file === relative);
  if (!entry) throw authorityError();
  return entry;
}

export async function trustedRecoveryDigest(file, digest) {
  if (digest === "sha256_without_code_signature_v1") {
    return executablePayloadSha256(file);
  }
  if (digest === "sha256") return fileSha256(file);
  throw authorityError();
}

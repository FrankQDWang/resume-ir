import { lstat, readFile, realpath } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { verifyBundleComposition } from "./macos-bundle-composition.mjs";
import { installMacosDmg } from "./macos-install-lifecycle.mjs";
import { validateSourceIdentity } from "./macos-source-identity.mjs";
import {
  verifyMacosInternalTestSignaturePolicy,
  withVerifiedMacosDmg,
} from "./verify-macos-dmg.mjs";
import { sha256 } from "./verify-bundled-sidecar.mjs";

const MAX_MANIFEST_BYTES = 64 * 1024;
const SHA256 = /^[a-f0-9]{64}$/;
const TARGET_TRIPLE = "aarch64-apple-darwin";

function installError(message = "macOS worktree artifact install failed") {
  return new Error(message);
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value).sort()) ===
      JSON.stringify([...expected].sort())
  );
}

function validCount(value) {
  return Number.isSafeInteger(value) && value >= 0;
}

function validateCompositionReceipt(receipt, source, dmgSha256) {
  const countFields = [
    "dmg_count",
    "dmg_bytes",
    "app_bundle_count",
    "applications_link_count",
    "volume_icon_count",
    "volume_icon_bytes",
    "daemon_sidecar_count",
    "embedding_sidecar_count",
    "pdf_renderer_sidecar_count",
    "embedding_resource_file_count",
    "embedding_resource_bytes",
    "classifier_resource_file_count",
    "classifier_resource_bytes",
    "ocr_resource_file_count",
    "ocr_resource_bytes",
    "build_machine_identity_path_markers",
  ];
  if (
    !exactKeys(receipt, [
      "schema_version",
      "target_triple",
      "source",
      ...countFields,
      "dmg_sha256",
      "app_composition_digest",
      "mounted_read_only",
      "digest_match",
      "executable",
      "architecture",
      "code_signature",
      "hardened_runtime",
      "library_validation_entitlement_scope",
      "notarization",
      "distribution_signature",
      "gatekeeper",
      "distribution_profile",
      "tester_allow_list_required",
      "release_claim",
    ]) ||
    countFields.some((field) => !validCount(receipt[field])) ||
    receipt.schema_version !== "resume-ir.macos-dmg-composition.v3" ||
    receipt.target_triple !== TARGET_TRIPLE ||
    JSON.stringify(receipt.source) !== JSON.stringify(source) ||
    receipt.dmg_count !== 1 ||
    receipt.dmg_sha256 !== dmgSha256 ||
    !SHA256.test(receipt.app_composition_digest ?? "") ||
    receipt.mounted_read_only !== true ||
    receipt.digest_match !== true ||
    receipt.executable !== true ||
    receipt.architecture !== "arm64" ||
    receipt.build_machine_identity_path_markers !== 0 ||
    receipt.code_signature !== "ad_hoc_valid" ||
    receipt.hardened_runtime !== true ||
    receipt.library_validation_entitlement_scope !== "embedding_runtime_only" ||
    receipt.notarization !== "not_requested" ||
    receipt.distribution_signature !== "accepted" ||
    !["accepted", "rejected"].includes(receipt.gatekeeper) ||
    receipt.distribution_profile !== "internal_test" ||
    receipt.tester_allow_list_required !== true ||
    receipt.release_claim !== "composition_only"
  ) {
    throw installError("macOS worktree artifact receipt is invalid");
  }
  return Object.freeze(receipt);
}

async function readArtifactManifest({ dmg, artifactManifest }) {
  if (
    !path.isAbsolute(dmg ?? "") ||
    !path.isAbsolute(artifactManifest ?? "") ||
    artifactManifest !== `${dmg}.json`
  ) {
    throw installError("macOS worktree artifact paths are invalid");
  }
  let dmgMetadata;
  let manifestMetadata;
  let resolvedDmg;
  let resolvedManifest;
  let source;
  try {
    [dmgMetadata, manifestMetadata, resolvedDmg, resolvedManifest, source] =
      await Promise.all([
        lstat(dmg),
        lstat(artifactManifest),
        realpath(dmg),
        realpath(artifactManifest),
        readFile(artifactManifest, "utf8"),
      ]);
  } catch {
    throw installError("macOS worktree artifact is unavailable");
  }
  if (
    !dmgMetadata.isFile() ||
    dmgMetadata.isSymbolicLink() ||
    !manifestMetadata.isFile() ||
    manifestMetadata.isSymbolicLink() ||
    resolvedDmg !== dmg ||
    resolvedManifest !== artifactManifest ||
    manifestMetadata.size === 0 ||
    manifestMetadata.size > MAX_MANIFEST_BYTES ||
    Buffer.byteLength(source, "utf8") !== manifestMetadata.size
  ) {
    throw installError("macOS worktree artifact is invalid");
  }
  let manifest;
  try {
    manifest = JSON.parse(source);
  } catch {
    throw installError("macOS worktree artifact manifest is invalid");
  }
  if (
    `${JSON.stringify(manifest)}\n` !== source ||
    !exactKeys(manifest, [
      "schema_version",
      "source",
      "artifact_file",
      "dmg_sha256",
      "composition_receipt",
    ]) ||
    manifest.schema_version !== "resume-ir.macos-worktree-artifact.v1" ||
    manifest.artifact_file !== path.basename(dmg) ||
    !SHA256.test(manifest.dmg_sha256 ?? "")
  ) {
    throw installError("macOS worktree artifact manifest is invalid");
  }
  const sourceIdentity = validateSourceIdentity(manifest.source);
  if (sourceIdentity.authority !== "worktree_snapshot") {
    throw installError("macOS worktree artifact source is invalid");
  }
  const actualDmgSha256 = await sha256(dmg);
  if (actualDmgSha256 !== manifest.dmg_sha256) {
    throw installError("macOS worktree artifact digest does not match");
  }
  return Object.freeze({
    source: sourceIdentity,
    receipt: validateCompositionReceipt(
      manifest.composition_receipt,
      sourceIdentity,
      actualDmgSha256,
    ),
  });
}

function receiptProjection(receipt) {
  return Object.freeze({
    schema_version: "resume-ir.desktop-bundle-composition.v1",
    target_triple: receipt.target_triple,
    desktop_executable_count: 1,
    icon_file_count: 1,
    daemon_sidecar_count: receipt.daemon_sidecar_count,
    embedding_sidecar_count: receipt.embedding_sidecar_count,
    pdf_renderer_sidecar_count: receipt.pdf_renderer_sidecar_count,
    embedding_resource_file_count: receipt.embedding_resource_file_count,
    embedding_resource_bytes: receipt.embedding_resource_bytes,
    classifier_resource_file_count: receipt.classifier_resource_file_count,
    classifier_resource_bytes: receipt.classifier_resource_bytes,
    ocr_resource_file_count: receipt.ocr_resource_file_count,
    ocr_resource_bytes: receipt.ocr_resource_bytes,
    digest_match: true,
    executable: true,
    architecture: receipt.architecture,
    path_scan_scope: "repo_root_and_builder_home",
    build_machine_identity_path_markers: 0,
  });
}

function sameCompositionReceipt(actual, expected) {
  return JSON.stringify(actual) === JSON.stringify(expected);
}

export async function installMacosWorktreeArtifact({
  repoRoot,
  targetTriple,
  dmg,
  artifactManifest = `${dmg}.json`,
  applicationsDirectory,
  expectedVersion,
  platform = process.platform,
  systemRunner,
  installDmg = installMacosDmg,
  verifyDmg = withVerifiedMacosDmg,
  verifyComposition = verifyBundleComposition,
  verifySignature = verifyMacosInternalTestSignaturePolicy,
}) {
  if (
    platform !== "darwin" ||
    targetTriple !== TARGET_TRIPLE ||
    !path.isAbsolute(repoRoot ?? "") ||
    !path.isAbsolute(applicationsDirectory ?? "") ||
    typeof expectedVersion !== "string" ||
    typeof installDmg !== "function" ||
    typeof verifyDmg !== "function" ||
    typeof verifyComposition !== "function" ||
    typeof verifySignature !== "function"
  ) {
    throw installError("macOS worktree artifact install arguments are invalid");
  }
  const artifact = await readArtifactManifest({ dmg, artifactManifest });
  const verifySnapshotApp = async ({
    appBundle,
    signatureRunner = systemRunner,
  }) => {
    const composition = await verifyComposition({
      appBundle,
      targetTriple,
      expectedVersion,
      expectedSource: artifact.source,
      verifySignaturePolicy: ({ appBundle: boundAppBundle }) =>
        verifySignature({
          appBundle: boundAppBundle,
          platform,
          ...(signatureRunner === undefined ? {} : { runner: signatureRunner }),
        }),
    });
    if (
      composition.composition_digest !== artifact.receipt.app_composition_digest
    ) {
      throw installError("macOS worktree App composition does not match");
    }
    return receiptProjection(artifact.receipt);
  };
  const withVerifiedSnapshotDmg = (options) =>
    verifyDmg({
      ...options,
      expectedSource: artifact.source,
      verifyApp: ({ appBundle }) =>
        verifySnapshotApp({
          appBundle,
          signatureRunner: options.systemRunner,
        }),
      consumeVerifiedImage: async (payload) => {
        if (!sameCompositionReceipt(payload.receipt, artifact.receipt)) {
          throw installError("macOS worktree DMG receipt does not match");
        }
        return options.consumeVerifiedImage(payload);
      },
    });
  return installDmg({
    repoRoot,
    targetTriple,
    dmg,
    applicationsDirectory,
    expectedVersion,
    platform,
    ...(systemRunner === undefined ? {} : { systemRunner }),
    withVerifiedDmg: withVerifiedSnapshotDmg,
    verifyApp: ({ appBundle }) => verifySnapshotApp({ appBundle }),
  });
}

function parseArguments(args) {
  const values = new Map();
  const allowed = new Set([
    "--target",
    "--dmg",
    "--artifact-manifest",
    "--applications",
    "--version",
  ]);
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index];
    const value = args[index + 1];
    if (!allowed.has(key) || !value || values.has(key)) {
      throw installError("invalid macOS worktree install arguments");
    }
    values.set(key, value);
  }
  if (values.size !== allowed.size) {
    throw installError("invalid macOS worktree install arguments");
  }
  return {
    targetTriple: values.get("--target"),
    dmg: values.get("--dmg"),
    artifactManifest: values.get("--artifact-manifest"),
    applicationsDirectory: values.get("--applications"),
    expectedVersion: values.get("--version"),
  };
}

async function main() {
  const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
  const result = await installMacosWorktreeArtifact({
    repoRoot,
    ...parseArguments(process.argv.slice(2)),
  });
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    console.error(`macos-worktree-install: ${error.message}`);
    process.exitCode = 1;
  });
}

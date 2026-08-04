import assert from "node:assert/strict";
import { chmod, mkdtemp, realpath, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { installMacosWorktreeArtifact } from "./macos-worktree-install.mjs";
import { sha256 } from "./verify-bundled-sidecar.mjs";

const SOURCE = Object.freeze({
  authority: "worktree_snapshot",
  base_commit: "a".repeat(40),
  source_tree_sha256: "b".repeat(64),
});

function compositionReceipt(dmgSha256) {
  return {
    schema_version: "resume-ir.macos-dmg-composition.v4",
    target_triple: "aarch64-apple-darwin",
    source: SOURCE,
    dmg_count: 1,
    dmg_bytes: 9,
    dmg_sha256: dmgSha256,
    app_composition_digest: "c".repeat(64),
    mounted_read_only: true,
    app_bundle_count: 1,
    applications_link_count: 1,
    volume_icon_count: 1,
    volume_icon_bytes: 1,
    daemon_sidecar_count: 1,
    embedding_sidecar_count: 1,
    pdf_renderer_sidecar_count: 1,
    embedding_resource_file_count: 7,
    embedding_resource_bytes: 7,
    classifier_resource_file_count: 2,
    classifier_resource_bytes: 2,
    ocr_resource_file_count: 31,
    ocr_resource_bytes: 31,
    pdfium_resource_file_count: 4,
    pdfium_resource_bytes: 4,
    digest_match: true,
    executable: true,
    architecture: "arm64",
    build_machine_identity_path_markers: 0,
    code_signature: "ad_hoc_valid",
    hardened_runtime: true,
    library_validation_entitlement_scope: "embedding_runtime_only",
    notarization: "not_requested",
    distribution_signature: "accepted",
    gatekeeper: "rejected",
    distribution_profile: "internal_test",
    tester_allow_list_required: true,
    release_claim: "composition_only",
  };
}

async function fixture(edition = "coreml") {
  const root = await realpath(
    await mkdtemp(path.join(os.tmpdir(), "resume-ir-worktree-install-")),
  );
  const editionName =
    edition === "coreml" ? "macos15_coreml" : "macos14_onnx";
  const dmg = path.join(
    root,
    `resume-ir_0.1.3_${editionName}_aarch64_bbbbbbbbbbbb.dmg`,
  );
  await writeFile(dmg, "dmg-bytes", { mode: 0o444 });
  await chmod(dmg, 0o444);
  const digest = await sha256(dmg);
  const receipt = compositionReceipt(digest);
  const manifest = {
    schema_version: "resume-ir.macos-worktree-artifact.v1",
    edition,
    source: SOURCE,
    artifact_file: path.basename(dmg),
    dmg_sha256: digest,
    composition_receipt: receipt,
  };
  const artifactManifest = `${dmg}.json`;
  await writeFile(artifactManifest, `${JSON.stringify(manifest)}\n`, {
    mode: 0o444,
  });
  await chmod(artifactManifest, 0o444);
  return { root, dmg, artifactManifest, receipt };
}

test("installs a receipt-bound worktree snapshot without main provenance", async () => {
  const values = await fixture();
  let verifiedDmg = false;
  let verifiedInstalledApp = false;
  const verifyComposition = async ({ expectedSource }) => {
    assert.deepEqual(expectedSource, SOURCE);
    return {
      composition_digest: "c".repeat(64),
      runtime_manifests: [{ role: "coreml_embedding" }],
    };
  };
  const result = await installMacosWorktreeArtifact({
    repoRoot: values.root,
    targetTriple: "aarch64-apple-darwin",
    dmg: values.dmg,
    artifactManifest: values.artifactManifest,
    applicationsDirectory: "/Applications",
    expectedVersion: "0.1.3",
    platform: "darwin",
    verifyComposition,
    verifySignature: async () => ({
      code_signature: "ad_hoc_valid",
      hardened_runtime: true,
      library_validation_entitlement_scope: "embedding_runtime_only",
    }),
    verifyDmg: async (options) => {
      verifiedDmg = true;
      const appReceipt = await options.verifyApp({
        appBundle: path.join(values.root, "mounted.app"),
      });
      assert.equal(appReceipt.daemon_sidecar_count, 1);
      return options.consumeVerifiedImage({
        appBundle: path.join(values.root, "mounted.app"),
        appComposition: {
          bundle_id: "local.resume-ir.desktop",
          version: "0.1.3",
          target_triple: "aarch64-apple-darwin",
          source: SOURCE,
          composition_digest: "c".repeat(64),
        },
        receipt: values.receipt,
      });
    },
    installDmg: async (options) => {
      const mounted = await options.withVerifiedDmg({
        systemRunner: async () => ({ status: 0 }),
        consumeVerifiedImage: ({ receipt }) => receipt,
      });
      assert.deepEqual(mounted, values.receipt);
      await options.verifyApp({
        appBundle: path.join(values.root, "installed.app"),
      });
      verifiedInstalledApp = true;
      return { version: options.expectedVersion };
    },
  });
  assert.deepEqual(result, { version: "0.1.3" });
  assert.equal(verifiedDmg, true);
  assert.equal(verifiedInstalledApp, true);
});

test("rejects a worktree artifact whose DMG bytes drift", async () => {
  const values = await fixture();
  await chmod(values.dmg, 0o644);
  await writeFile(values.dmg, "changed");
  await assert.rejects(
    installMacosWorktreeArtifact({
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      artifactManifest: values.artifactManifest,
      applicationsDirectory: "/Applications",
      expectedVersion: "0.1.3",
      platform: "darwin",
      installDmg: async () => {
        throw new Error("must not install");
      },
    }),
    /artifact digest does not match/,
  );
});

test("rejects a copied App whose composition differs from the artifact", async () => {
  const values = await fixture();
  await assert.rejects(
    installMacosWorktreeArtifact({
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      artifactManifest: values.artifactManifest,
      applicationsDirectory: "/Applications",
      expectedVersion: "0.1.3",
      platform: "darwin",
      verifyComposition: async () => ({
        composition_digest: "d".repeat(64),
        runtime_manifests: [{ role: "coreml_embedding" }],
      }),
      installDmg: async (options) =>
        options.verifyApp({
          appBundle: path.join(values.root, "installed.app"),
        }),
    }),
    /App composition does not match/,
  );
});

test("rejects a provider edition whose App composition has the other provider", async () => {
  const values = await fixture("onnx");
  await assert.rejects(
    installMacosWorktreeArtifact({
      repoRoot: values.root,
      targetTriple: "aarch64-apple-darwin",
      dmg: values.dmg,
      artifactManifest: values.artifactManifest,
      applicationsDirectory: "/Applications",
      expectedVersion: "0.1.3",
      platform: "darwin",
      verifyComposition: async () => ({
        composition_digest: "c".repeat(64),
        runtime_manifests: [{ role: "coreml_embedding" }],
      }),
      installDmg: async (options) =>
        options.verifyApp({
          appBundle: path.join(values.root, "installed.app"),
        }),
    }),
    /App composition does not match/,
  );
});

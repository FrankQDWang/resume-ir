import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  createDesktopCompositionPlan,
  createPdfRendererPlan,
  createSidecarPlan,
  defaultSidecarBuildTargetDir,
  runSidecarBuild,
  stageEmbeddingResourcePack,
  stageBuiltSidecar,
  validateWindowsProcessContainmentContract,
} from "./prepare-sidecar.mjs";
import { stageOcrResourcePack } from "./ocr-pack.mjs";
import {
  createTauriBuildEnvironment,
  resolveTauriPaths,
  selectTauriEnvironment,
  withDesktopComposition,
} from "./run-tauri.mjs";
import { verifyBundledSidecar } from "./verify-bundled-sidecar.mjs";

function syntheticMachO(payload) {
  const suffix = Buffer.from(payload);
  const header = Buffer.alloc(32);
  header.writeUInt32LE(0xfeedfacf, 0);
  header.writeUInt32LE(0x0100000c, 4);
  return Buffer.concat([header, suffix]);
}

function syntheticSignedMachO(payload, signature) {
  const body = Buffer.from(payload);
  const signatureBytes = Buffer.from(signature);
  const header = Buffer.alloc(32);
  header.writeUInt32LE(0xfeedfacf, 0);
  header.writeUInt32LE(0x0100000c, 4);
  header.writeUInt32LE(1, 16);
  header.writeUInt32LE(16, 20);
  const command = Buffer.alloc(16);
  command.writeUInt32LE(0x1d, 0);
  command.writeUInt32LE(16, 4);
  command.writeUInt32LE(header.length + command.length + body.length, 8);
  command.writeUInt32LE(signatureBytes.length, 12);
  return Buffer.concat([header, command, body, signatureBytes]);
}

async function writeExecutable(file, body) {
  await writeFile(file, body);
  if (process.platform !== "win32") await chmod(file, 0o755);
}

function sha256(body) {
  return createHash("sha256").update(body).digest("hex");
}

function syntheticWindowsProcessContainmentContract() {
  return {
    schema_version: "resume-ir.windows-process-containment.v1",
    target_triple: "x86_64-pc-windows-msvc",
    minimum_windows_build: 10240,
    wrapper_crate: "process-containment",
    job_limit: "kill_on_job_close",
    breakaway_allowed: false,
    spawn_failure_mode: "fail_closed_and_reaped",
    workspace_unsafe_code_allowed: false,
    covered_spawn_owners: [
      "desktop_daemon",
      "embedding_one_shot",
      "embedding_resident",
      "ocr_custom_engine",
      "ocr_tesseract",
      "pdf_custom_renderer",
      "pdf_pdftoppm",
    ],
  };
}

async function createSyntheticPack(root, { sourceSymlink = false } = {}) {
  const source = path.join(root, "source-pack");
  const expectedManifest = path.join(root, "expected-runtime-pack.json");
  await mkdir(source, { recursive: true });
  const files = [
    ["runtime_library", "libonnxruntime.dylib", syntheticMachO("ort")],
    ["model", "model.onnx", Buffer.from("model")],
    ["tokenizer", "tokenizer.json", Buffer.from("tokenizer")],
    ["model_config", "config.json", Buffer.from("config")],
    ["special_tokens_map", "special_tokens_map.json", Buffer.from("special")],
    ["tokenizer_config", "tokenizer_config.json", Buffer.from("tokenizer-config")],
  ];
  for (const [, file, body] of files) await writeFile(path.join(source, file), body);
  if (sourceSymlink) {
    await writeFile(path.join(root, "outside-model"), "outside");
    await rm(path.join(source, "model.onnx"));
    await symlink(path.join(root, "outside-model"), path.join(source, "model.onnx"));
  }
  const manifest = {
    schema_version: "resume-ir.embedding-runtime-pack.v1",
    runtime_pack_id: "intfloat-multilingual-e5-small-qint8-r1",
    model_id: "intfloat-multilingual-e5-small-qint8-r1",
    upstream_model_id: "intfloat/multilingual-e5-small",
    upstream_revision: "614241f622f53c4eeff9890bdc4f31cfecc418b3",
    dimension: 384,
    provider: "cpu",
    network_access: "disabled",
    license_reviewed: true,
    model_license: "MIT",
    onnxruntime_license: "MIT",
    files: files.map(([role, file, body]) => ({
      role,
      file,
      bytes: body.length,
      sha256: sha256(body),
    })),
    upstream_model_file: "onnx/model_qint8_avx512_vnni.onnx",
    quantization: "dynamic_int8",
  };
  const manifestBody = `${JSON.stringify(manifest, null, 2)}\n`;
  await writeFile(path.join(source, "runtime-pack.json"), manifestBody);
  await writeFile(expectedManifest, manifestBody);
  return { expectedManifest, manifest, source };
}

async function createSyntheticOcrPack(root) {
  const source = path.join(root, "source-ocr-pack");
  const expectedManifest = path.join(root, "expected-ocr-runtime-pack.json");
  await mkdir(source, { recursive: true });
  const files = [
    ["engine_binary", "tesseract", syntheticMachO("engine"), true],
    ...Array.from({ length: 15 }, (_, index) => [
      "engine_library",
      `lib/libsynthetic-${String(index).padStart(2, "0")}.dylib`,
      syntheticMachO(`library-${index}`),
      false,
    ]),
    ["language_eng", "tessdata/eng.traineddata", Buffer.from("eng"), false],
    ["language_chi_sim", "tessdata/chi_sim.traineddata", Buffer.from("chi"), false],
    ["engine_config", "tessdata/configs/tsv", Buffer.from("tsv"), false],
    ...Array.from({ length: 10 }, (_, index) => [
      "license_text",
      `LICENSES/license-${String(index).padStart(2, "0")}.txt`,
      Buffer.from(`license-${index}`),
      false,
    ]),
    [
      "third_party_notice",
      "THIRD-PARTY-NOTICES.json",
      Buffer.from('{"synthetic":true}\n'),
      false,
    ],
  ];
  for (const [, file, body, executable] of files) {
    const destination = path.join(source, file);
    await mkdir(path.dirname(destination), { recursive: true });
    await writeFile(destination, body);
    if (executable) await chmod(destination, 0o755);
  }
  const manifest = {
    schema_version: "resume-ir.desktop-ocr-runtime-pack.v1",
    runtime_pack_id: "tesseract-5.5.2-tessdata-fast-4.1.0-macos-arm64-r1",
    target_triple: "aarch64-apple-darwin",
    engine: "tesseract",
    engine_version: "5.5.2",
    renderer: "macos-pdfkit-coregraphics",
    languages: ["eng", "chi_sim"],
    network_access: "disabled",
    license_reviewed: true,
    third_party_notice: "THIRD-PARTY-NOTICES.json",
    files: files.map(([role, file, body, executable]) => ({
      role,
      file,
      bytes: body.length,
      sha256: sha256(body),
      executable,
    })),
  };
  const body = `${JSON.stringify(manifest, null, 2)}\n`;
  await writeFile(path.join(source, "runtime-pack.json"), body);
  await writeFile(expectedManifest, body);
  return { expectedManifest, source };
}

async function copyTree(source, destination) {
  await mkdir(destination, { recursive: true });
  for (const entry of await readdir(source, { withFileTypes: true })) {
    const from = path.join(source, entry.name);
    const to = path.join(destination, entry.name);
    if (entry.isDirectory()) await copyTree(from, to);
    else await copyFile(from, to);
  }
}

async function prepareSyntheticBundleComposition(
  repoRoot,
  appBundle,
  { daemonPayload = "same-daemon" } = {},
) {
  const targetTriple = "aarch64-apple-darwin";
  const pack = await createSyntheticPack(repoRoot);
  const ocrPack = await createSyntheticOcrPack(repoRoot);
  const plan = createDesktopCompositionPlan({
    repoRoot,
    targetTriple,
    debug: false,
    sourcePackRoot: pack.source,
    expectedManifest: pack.expectedManifest,
    sourceOcrPackRoot: ocrPack.source,
    expectedOcrManifest: ocrPack.expectedManifest,
  });
  const macosDirectory = path.join(appBundle, "Contents", "MacOS");
  await mkdir(macosDirectory, { recursive: true });
  const sidecarBodies = new Map([
    ["resume-daemon", syntheticMachO(daemonPayload)],
    ["resume-embedding-runtime", syntheticMachO("same-runtime")],
    ["resume-pdf-render-runtime", syntheticMachO("same-renderer")],
  ]);
  for (const sidecar of plan.sidecars) {
    const body = sidecarBodies.get(sidecar.binaryName);
    await mkdir(path.dirname(sidecar.destination), { recursive: true });
    await writeExecutable(sidecar.destination, body);
    await writeExecutable(path.join(macosDirectory, sidecar.binaryName), body);
  }
  await stageEmbeddingResourcePack(plan.resourcePack);
  await stageOcrResourcePack(plan.ocrResourcePack);
  const bundledPack = path.join(
    appBundle,
    "Contents",
    "Resources",
    "embedding",
    "runtime-pack",
  );
  await mkdir(bundledPack, { recursive: true });
  for (const entry of await readdir(plan.resourcePack.destination)) {
    await copyFile(
      path.join(plan.resourcePack.destination, entry),
      path.join(bundledPack, entry),
    );
  }
  const bundledOcrPack = path.join(
    appBundle,
    "Contents",
    "Resources",
    "ocr",
    "runtime-pack",
  );
  await copyTree(plan.ocrResourcePack.destination, bundledOcrPack);
  return {
    expectedManifest: pack.expectedManifest,
    expectedOcrManifest: ocrPack.expectedManifest,
    plan,
    targetTriple,
  };
}

test("release build paths are remapped without inheriting shell Rust flags", () => {
  const repoRoot = path.join(path.sep, "synthetic", "resume-ir");
  const homeDirectory = path.join(path.sep, "synthetic", "builder-home");
  const environment = createTauriBuildEnvironment({
    environment: { CARGO_ENCODED_RUSTFLAGS: "-C\u001fopt-level=2" },
    repoRoot,
    homeDirectory,
  });
  assert.deepEqual(environment.CARGO_ENCODED_RUSTFLAGS.split("\u001f"), [
    "-C",
    "opt-level=2",
    `--remap-path-prefix=${repoRoot}=/source/resume-ir`,
    `--remap-path-prefix=${homeDirectory}=/build-home`,
  ]);
  assert.throws(
    () =>
      createTauriBuildEnvironment({
        environment: { RUSTFLAGS: "-C target-cpu=native" },
        repoRoot,
        homeDirectory,
      }),
    /RUSTFLAGS must be unset/,
  );
  assert.ok(path.isAbsolute(defaultSidecarBuildTargetDir()));
  if (process.platform !== "win32") {
    assert.ok(!defaultSidecarBuildTargetDir().startsWith(homeDirectory));
  }
  const paths = resolveTauriPaths();
  const runTauriUrl = new URL("./run-tauri.mjs", import.meta.url);
  assert.equal(paths.frontendRoot, fileURLToPath(new URL("..", runTauriUrl)));
  assert.equal(paths.repoRoot, fileURLToPath(new URL("../../..", runTauriUrl)));
  assert.equal(path.basename(paths.cli), "tauri.js");
  const debugEnvironment = { RUSTFLAGS: "-C target-cpu=native" };
  assert.equal(
    selectTauriEnvironment({
      arguments: ["build", "--debug"],
      environment: debugEnvironment,
      repoRoot,
      homeDirectory,
    }),
    debugEnvironment,
  );
  assert.equal(
    selectTauriEnvironment({
      arguments: ["dev"],
      environment: debugEnvironment,
      repoRoot,
      homeDirectory,
    }),
    debugEnvironment,
  );
  const bundleConfig = path.join(paths.frontendRoot, "src-tauri", "tauri.bundle.conf.json");
  assert.deepEqual(withDesktopComposition(["build", "--ci"], bundleConfig), [
    "build",
    "--ci",
    "--config",
    bundleConfig,
  ]);
  assert.deepEqual(withDesktopComposition(["dev"], bundleConfig), [
    "dev",
    "--config",
    bundleConfig,
  ]);
  assert.deepEqual(
    withDesktopComposition(
      ["build", "--", "--synthetic-runner-arg"],
      bundleConfig,
    ),
    [
      "build",
      "--config",
      bundleConfig,
      "--",
      "--synthetic-runner-arg",
    ],
  );
});

test("plans target-triple sidecars without depending on the working directory", () => {
  const repoRoot = path.join(path.sep, "synthetic", "resume-ir");
  const mac = createSidecarPlan({
    repoRoot,
    targetTriple: "aarch64-apple-darwin",
    debug: false,
  });
  assert.deepEqual(mac.cargoArgs, [
    "build",
    "--manifest-path",
    path.join(repoRoot, "Cargo.toml"),
    "-p",
    "resume-daemon",
    "--bin",
    "resume-daemon",
    "--locked",
    "--target",
    "aarch64-apple-darwin",
    "--target-dir",
    path.join(repoRoot, "target"),
    "--release",
  ]);
  assert.equal(
    mac.source,
    path.join(repoRoot, "target", "aarch64-apple-darwin", "release", "resume-daemon"),
  );
  assert.equal(
    mac.destination,
    path.join(repoRoot, "target", "tauri-sidecars", "resume-daemon-aarch64-apple-darwin"),
  );

  const windows = createSidecarPlan({
    repoRoot,
    targetTriple: "x86_64-pc-windows-msvc",
    debug: true,
  });
  assert.equal(
    windows.source,
    path.join(repoRoot, "target", "x86_64-pc-windows-msvc", "debug", "resume-daemon.exe"),
  );
  assert.equal(
    windows.destination,
    path.join(
      repoRoot,
      "target",
      "tauri-sidecars",
      "resume-daemon-x86_64-pc-windows-msvc.exe",
    ),
  );
  assert.ok(!windows.cargoArgs.includes("--release"));
});

test("accepts reviewed Windows containment but refuses partial runtime composition", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-windows-plan-"));
  context.after(() => rm(repoRoot, { recursive: true, force: true }));
  const processContainmentContract = path.join(repoRoot, "containment.json");
  await writeFile(
    processContainmentContract,
    JSON.stringify(syntheticWindowsProcessContainmentContract()),
  );
  const windowsEmbeddingSourceContract = fileURLToPath(
    new URL(
      "../resources/embedding/x86_64-pc-windows-msvc/source-contract.json",
      import.meta.url,
    ),
  );
  const windowsPdfRendererSourceContract = fileURLToPath(
    new URL(
      "../resources/pdf-renderer/x86_64-pc-windows-msvc/source-contract.json",
      import.meta.url,
    ),
  );
  const windowsOcrSourceContract = fileURLToPath(
    new URL(
      "../resources/ocr/x86_64-pc-windows-msvc/source-contract.json",
      import.meta.url,
    ),
  );
  assert.throws(
    () =>
      createDesktopCompositionPlan({
        repoRoot,
        targetTriple: "x86_64-pc-windows-msvc",
        debug: false,
        processContainmentContract,
        windowsEmbeddingSourceContract,
        windowsPdfRendererSourceContract,
        windowsOcrSourceContract,
      }),
    /reviewed static-CRT x64 embedding, static Tesseract OCR, static PDFium renderer, and process-containment contracts are present; real reviewed embedding\/Tesseract\/PDFium artifacts, expected pack manifests, final PE dependency closure, and native evidence are required; refusing a partial NSIS build/,
  );
});

test("rejects breakaway or incomplete Windows process containment", () => {
  const breakaway = syntheticWindowsProcessContainmentContract();
  breakaway.breakaway_allowed = true;
  assert.throws(
    () => validateWindowsProcessContainmentContract(breakaway),
    /process containment contract is invalid/,
  );

  const missingOwner = syntheticWindowsProcessContainmentContract();
  missingOwner.covered_spawn_owners.pop();
  assert.throws(
    () => validateWindowsProcessContainmentContract(missingOwner),
    /process containment contract is invalid/,
  );
});

test("fails closed for an unsupported or missing target triple", () => {
  assert.throws(
    () => createSidecarPlan({ repoRoot: path.sep, targetTriple: "", debug: false }),
    /target triple is required/,
  );
  assert.throws(
    () =>
      createSidecarPlan({
        repoRoot: path.sep,
        targetTriple: "x86_64-unknown-linux-gnu",
        debug: false,
      }),
    /target triple is not supported/,
  );
});

test("plans three sidecars and immutable arm64 embedding and OCR packs", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-composition-plan-"));
  context.after(async () => {
    const { rm } = await import("node:fs/promises");
    await rm(repoRoot, { recursive: true, force: true });
  });
  const pack = await createSyntheticPack(repoRoot);
  const plan = createDesktopCompositionPlan({
    repoRoot,
    targetTriple: "aarch64-apple-darwin",
    debug: false,
    sourcePackRoot: pack.source,
    expectedManifest: pack.expectedManifest,
  });

  assert.deepEqual(
    plan.sidecars.map((sidecar) => path.basename(sidecar.destination)),
    [
      "resume-daemon-aarch64-apple-darwin",
      "resume-embedding-runtime-aarch64-apple-darwin",
      "resume-pdf-render-runtime-aarch64-apple-darwin",
    ],
  );
  assert.equal(
    plan.resourcePack.destination,
    path.join(repoRoot, "target", "tauri-resources", "embedding-runtime-pack"),
  );
  const renderer = createPdfRendererPlan({
    repoRoot,
    buildTargetDir: path.join(repoRoot, "target"),
    targetTriple: "aarch64-apple-darwin",
    debug: false,
  });
  assert.equal(renderer.buildKind, "clang");
  assert.ok(renderer.clangArgs.includes("CoreGraphics"));
  assert.throws(
    () =>
      createDesktopCompositionPlan({
        repoRoot,
        targetTriple: "x86_64-apple-darwin",
        debug: false,
      }),
    /embedding resource target is not supported/,
  );
});

test("stages only the exact reviewed embedding pack and rejects symlinks", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-pack-stage-"));
  context.after(async () => {
    const { rm } = await import("node:fs/promises");
    await rm(repoRoot, { recursive: true, force: true });
  });
  const pack = await createSyntheticPack(repoRoot);
  await writeFile(path.join(pack.source, "private-evidence.json"), "must-not-copy");
  const plan = createDesktopCompositionPlan({
    repoRoot,
    targetTriple: "aarch64-apple-darwin",
    debug: false,
    sourcePackRoot: pack.source,
    expectedManifest: pack.expectedManifest,
  });

  await stageEmbeddingResourcePack(plan.resourcePack);
  assert.deepEqual((await readdir(plan.resourcePack.destination)).sort(), [
    "config.json",
    "libonnxruntime.dylib",
    "model.onnx",
    "runtime-pack.json",
    "special_tokens_map.json",
    "tokenizer.json",
    "tokenizer_config.json",
  ]);
  assert.notEqual(
    (await stat(path.join(plan.resourcePack.destination, "libonnxruntime.dylib"))).mode &
      0o200,
    0,
  );

  const badRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-pack-symlink-"));
  const badPack = await createSyntheticPack(badRoot, { sourceSymlink: true });
  const badPlan = createDesktopCompositionPlan({
    repoRoot,
    targetTriple: "aarch64-apple-darwin",
    debug: false,
    sourcePackRoot: badPack.source,
    expectedManifest: badPack.expectedManifest,
  });
  await assert.rejects(
    stageEmbeddingResourcePack(badPlan.resourcePack),
    /regular non-symlink file/,
  );
});

test(
  "stages the built sidecar and makes Unix targets executable",
  { skip: process.platform === "win32" },
  async (context) => {
    const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-sidecar-test-"));
    context.after(async () => {
      const { rm } = await import("node:fs/promises");
      await rm(repoRoot, { recursive: true, force: true });
    });
    const plan = createSidecarPlan({
      repoRoot,
      targetTriple: "aarch64-apple-darwin",
      debug: false,
    });
    await mkdir(path.dirname(plan.source), { recursive: true });
    await writeFile(plan.source, "synthetic-daemon");
    await chmod(plan.source, 0o644);

    await stageBuiltSidecar(plan);

    assert.equal(await readFile(plan.destination, "utf8"), "synthetic-daemon");
    assert.notEqual((await stat(plan.destination)).mode & 0o111, 0);
  },
);

test("rejects an empty build without replacing a prior staged sidecar", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-sidecar-empty-"));
  context.after(async () => {
    const { rm } = await import("node:fs/promises");
    await rm(repoRoot, { recursive: true, force: true });
  });
  const plan = createSidecarPlan({
    repoRoot,
    targetTriple: "x86_64-pc-windows-msvc",
    debug: false,
  });
  await mkdir(path.dirname(plan.source), { recursive: true });
  await mkdir(path.dirname(plan.destination), { recursive: true });
  await writeFile(plan.source, "");
  await writeFile(plan.destination, "previous-daemon");

  await assert.rejects(stageBuiltSidecar(plan), /built daemon sidecar is empty/);
  assert.equal(await readFile(plan.destination, "utf8"), "previous-daemon");
});

test("stages a Windows executable without Unix mode assumptions", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-sidecar-windows-"));
  context.after(async () => {
    const { rm } = await import("node:fs/promises");
    await rm(repoRoot, { recursive: true, force: true });
  });
  const plan = createSidecarPlan({
    repoRoot,
    targetTriple: "x86_64-pc-windows-msvc",
    debug: true,
  });
  await mkdir(path.dirname(plan.source), { recursive: true });
  await writeFile(plan.source, "synthetic-windows-daemon");

  await stageBuiltSidecar(plan);

  assert.equal(await readFile(plan.destination, "utf8"), "synthetic-windows-daemon");
});

test("a failed Cargo build cannot replace a previously staged daemon", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-sidecar-build-fail-"));
  context.after(async () => {
    const { rm } = await import("node:fs/promises");
    await rm(repoRoot, { recursive: true, force: true });
  });
  const plan = createSidecarPlan({
    repoRoot,
    targetTriple: "aarch64-apple-darwin",
    debug: false,
  });
  await mkdir(path.dirname(plan.destination), { recursive: true });
  await writeFile(plan.destination, "previous-daemon");

  assert.throws(
    () => runSidecarBuild(plan, () => ({ error: undefined, status: 1 })),
    /daemon sidecar build failed/,
  );
  assert.equal(await readFile(plan.destination, "utf8"), "previous-daemon");
});

test("desktop config prepares three sidecars and two resource packs", async () => {
  const configPath = new URL("../src-tauri/tauri.conf.json", import.meta.url);
  const bundleConfigPath = new URL(
    "../src-tauri/tauri.bundle.conf.json",
    import.meta.url,
  );
  const windowsConfigPath = new URL(
    "../src-tauri/tauri.windows.conf.json",
    import.meta.url,
  );
  const tauriSchemaPath = new URL(
    "../node_modules/@tauri-apps/cli/config.schema.json",
    import.meta.url,
  );
  const packagePath = new URL("../package.json", import.meta.url);
  const config = JSON.parse(await readFile(configPath, "utf8"));
  const bundleConfig = JSON.parse(await readFile(bundleConfigPath, "utf8"));
  const windowsConfig = JSON.parse(await readFile(windowsConfigPath, "utf8"));
  const tauriSchema = JSON.parse(await readFile(tauriSchemaPath, "utf8"));
  const packageJson = JSON.parse(await readFile(packagePath, "utf8"));

  assert.equal(config.build.beforeBuildCommand, "npm run build");
  assert.equal(config.bundle.active, false);
  assert.equal(config.bundle.externalBin, undefined);
  assert.deepEqual(bundleConfig.build.beforeDevCommand, {
    script: "npm run dev:bundle",
    cwd: "..",
  });
  assert.deepEqual(bundleConfig.build.beforeBuildCommand, {
    script: "npm run build:bundle",
    cwd: "..",
  });
  assert.equal(bundleConfig.bundle.active, true);
  assert.deepEqual(bundleConfig.bundle.externalBin, [
    "../../../target/tauri-sidecars/resume-daemon",
    "../../../target/tauri-sidecars/resume-embedding-runtime",
    "../../../target/tauri-sidecars/resume-pdf-render-runtime",
  ]);
  assert.deepEqual(bundleConfig.bundle.resources, {
    "../../../target/tauri-resources/embedding-runtime-pack/":
      "embedding/runtime-pack/",
    "../../../target/tauri-resources/ocr-runtime-pack/": "ocr/runtime-pack/",
  });
  assert.equal(windowsConfig.$schema, "https://schema.tauri.app/config/2");
  assert.deepEqual(windowsConfig.bundle.targets, ["nsis"]);
  assert.deepEqual(windowsConfig.bundle.windows, {
    allowDowngrades: false,
    webviewInstallMode: { type: "offlineInstaller", silent: true },
    nsis: { installMode: "currentUser" },
  });
  const installerModes = tauriSchema.definitions.NSISInstallerMode.oneOf.flatMap(
    (variant) => variant.enum ?? [],
  );
  const webviewModes = tauriSchema.definitions.WebviewInstallMode.oneOf.flatMap(
    (variant) => variant.properties?.type?.enum ?? [],
  );
  assert.ok(installerModes.includes("currentUser"));
  assert.ok(webviewModes.includes("offlineInstaller"));
  assert.equal(
    packageJson.scripts["dev:bundle"],
    "node scripts/prepare-sidecar.mjs --debug && npm run dev",
  );
  assert.equal(
    packageJson.scripts["build:bundle"],
    "node scripts/prepare-sidecar.mjs && npm run build",
  );
});

test("desktop frontend needs no inline style CSP exception", async () => {
  const config = JSON.parse(
    await readFile(
      new URL("../src-tauri/tauri.conf.json", import.meta.url),
      "utf8",
    ),
  );
  const appSource = await readFile(
    new URL("../src/App.tsx", import.meta.url),
    "utf8",
  );
  const stylesheet = await readFile(
    new URL("../src/styles.css", import.meta.url),
    "utf8",
  );

  assert.equal(
    config.app.security.csp,
    "default-src 'self'; style-src 'self'; img-src 'self' data:",
  );
  assert.doesNotMatch(config.app.security.csp, /unsafe-inline|unsafe-eval|https?:/);
  assert.doesNotMatch(appSource, /<[^>]+\sstyle\s*=/);
  assert.doesNotMatch(appSource, /dangerouslySetInnerHTML/);
  assert.match(appSource, /<progress[^>]+className="progress-track/);
  assert.match(appSource, /aria-label="可搜索简历比例"/);
  assert.match(stylesheet, /progress\.progress-track\s*\{/);
  assert.match(stylesheet, /::-webkit-progress-value/);
  assert.match(stylesheet, /::-moz-progress-bar/);
});

test("verifies exact native sidecars and embedding resources in a macOS app", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-bundle-verify-"));
  context.after(async () => {
    const { rm } = await import("node:fs/promises");
    await rm(repoRoot, { recursive: true, force: true });
  });
  const appBundle = path.join(repoRoot, "synthetic.app");
  const composition = await prepareSyntheticBundleComposition(repoRoot, appBundle);

  const receipt = await verifyBundledSidecar({
    repoRoot,
    targetTriple: composition.targetTriple,
    appBundle,
    expectedManifest: composition.expectedManifest,
    expectedOcrManifest: composition.expectedOcrManifest,
  });

  assert.equal(receipt.daemon_sidecar_count, 1);
  assert.equal(receipt.embedding_sidecar_count, 1);
  assert.equal(receipt.pdf_renderer_sidecar_count, 1);
  assert.equal(receipt.embedding_resource_file_count, 7);
  assert.equal(receipt.ocr_resource_file_count, 31);
  assert.equal(receipt.digest_match, true);
  assert.equal(receipt.architecture, "arm64");
  assert.equal(receipt.path_scan_scope, "repo_root_and_builder_home");
  assert.equal(receipt.build_machine_identity_path_markers, 0);
});

test("rejects mismatched or duplicate bundled daemon sidecars", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-bundle-reject-"));
  context.after(async () => {
    const { rm } = await import("node:fs/promises");
    await rm(repoRoot, { recursive: true, force: true });
  });
  const targetTriple = "aarch64-apple-darwin";
  const staged = path.join(
    repoRoot,
    "target",
    "tauri-sidecars",
    `resume-daemon-${targetTriple}`,
  );
  const appBundle = path.join(repoRoot, "synthetic.app");
  const macosDirectory = path.join(appBundle, "Contents", "MacOS");
  const composition = await prepareSyntheticBundleComposition(repoRoot, appBundle, {
    daemonPayload: "expected-daemon",
  });
  await writeExecutable(
    path.join(macosDirectory, "resume-daemon"),
    syntheticMachO("different-daemon"),
  );

  await assert.rejects(
    verifyBundledSidecar({
      repoRoot,
      targetTriple,
      appBundle,
      expectedManifest: composition.expectedManifest,
      expectedOcrManifest: composition.expectedOcrManifest,
    }),
    /does not match/,
  );
  await writeExecutable(
    path.join(macosDirectory, "resume-daemon"),
    syntheticMachO("expected-daemon"),
  );
  await writeExecutable(
    path.join(macosDirectory, "resume-daemon-copy"),
    syntheticMachO("expected-daemon"),
  );
  await assert.rejects(
    verifyBundledSidecar({
      repoRoot,
      targetTriple,
      appBundle,
      expectedManifest: composition.expectedManifest,
      expectedOcrManifest: composition.expectedOcrManifest,
    }),
    /exactly one/,
  );
});

test("matches executable payload while allowing only the Mach-O signature blob to change", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-bundle-signature-"));
  context.after(() => rm(repoRoot, { recursive: true, force: true }));
  const targetTriple = "aarch64-apple-darwin";
  const appBundle = path.join(repoRoot, "synthetic.app");
  const composition = await prepareSyntheticBundleComposition(repoRoot, appBundle);
  const staged = path.join(
    repoRoot,
    "target",
    "tauri-sidecars",
    `resume-daemon-${targetTriple}`,
  );
  const bundled = path.join(appBundle, "Contents", "MacOS", "resume-daemon");
  await writeExecutable(
    staged,
    syntheticSignedMachO("same-executable-payload", "linker-signature"),
  );
  await writeExecutable(
    bundled,
    syntheticSignedMachO("same-executable-payload", "tauri-ad-hoc-signature"),
  );

  const receipt = await verifyBundledSidecar({
    repoRoot,
    targetTriple,
    appBundle,
    expectedManifest: composition.expectedManifest,
    expectedOcrManifest: composition.expectedOcrManifest,
  });
  assert.equal(receipt.digest_match, true);

  await writeExecutable(
    bundled,
    syntheticSignedMachO("changed-executable-payload", "tauri-ad-hoc-signature"),
  );
  await assert.rejects(
    verifyBundledSidecar({
      repoRoot,
      targetTriple,
      appBundle,
      expectedManifest: composition.expectedManifest,
      expectedOcrManifest: composition.expectedOcrManifest,
    }),
    /does not match the staged binary/,
  );
});

test("rejects a bundled daemon containing a build-machine identity path", async (context) => {
  const repoRoot = await mkdtemp(path.join(os.tmpdir(), "resume-ir-bundle-path-"));
  context.after(async () => {
    const { rm } = await import("node:fs/promises");
    await rm(repoRoot, { recursive: true, force: true });
  });
  const targetTriple = "aarch64-apple-darwin";
  const appBundle = path.join(repoRoot, "synthetic.app");
  const composition = await prepareSyntheticBundleComposition(repoRoot, appBundle);
  const staged = path.join(
    repoRoot,
    "target",
    "tauri-sidecars",
    `resume-daemon-${targetTriple}`,
  );
  const bundled = path.join(
    appBundle,
    "Contents",
    "MacOS",
    "resume-daemon",
  );
  const body = syntheticMachO(`synthetic-prefix:${repoRoot}`);
  await writeExecutable(staged, body);
  await writeExecutable(bundled, body);

  await assert.rejects(
    verifyBundledSidecar({
      repoRoot,
      targetTriple,
      appBundle,
      expectedManifest: composition.expectedManifest,
      expectedOcrManifest: composition.expectedOcrManifest,
    }),
    /build-machine identity path marker/,
  );
});

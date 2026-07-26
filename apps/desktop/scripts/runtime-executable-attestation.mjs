import { createHash } from "node:crypto";
import { chmod, lstat, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import path from "node:path";

export const RUNTIME_EXECUTABLE_ATTESTATION_SCHEMA =
  "resume-ir.runtime-executable-attestation.v1";

const SUPPORTED_TARGET = "aarch64-apple-darwin";
const DIGEST_ALGORITHM = "sha256_without_code_signature_v1";
const MAX_EXECUTABLE_BYTES = 256 * 1024 * 1024;
const MAX_LOAD_COMMANDS = 4096;
const LC_CODE_SIGNATURE = 0x1d;
const LC_SEGMENT_64 = 0x19;
const EXPECTED = Object.freeze([
  Object.freeze({
    role: "embedding_runtime",
    binaryName: "resume-embedding-runtime",
    buildFile: "resume-embedding-runtime-aarch64-apple-darwin",
    runtimeFile: "resume-embedding-runtime",
  }),
  Object.freeze({
    role: "pdf_renderer",
    binaryName: "resume-pdf-render-runtime",
    buildFile: "resume-pdf-render-runtime-aarch64-apple-darwin",
    runtimeFile: "resume-pdf-render-runtime",
  }),
]);
const SHA256 = /^[a-f0-9]{64}$/;

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value)) === JSON.stringify(expected)
  );
}

export function validateRuntimeExecutableAttestation(value, plan) {
  if (
    !exactKeys(value, ["schema_version", "target_triple", "profile", "executables"]) ||
    value.schema_version !== RUNTIME_EXECUTABLE_ATTESTATION_SCHEMA ||
    value.target_triple !== SUPPORTED_TARGET ||
    value.target_triple !== plan.targetTriple ||
    !["debug", "release"].includes(value.profile) ||
    value.profile !== plan.profile ||
    !Array.isArray(value.executables) ||
    value.executables.length !== EXPECTED.length
  ) {
    throw new Error("runtime executable attestation contract is invalid");
  }
  for (let index = 0; index < EXPECTED.length; index += 1) {
    const entry = value.executables[index];
    const expected = EXPECTED[index];
    if (
      !exactKeys(entry, [
        "role",
        "build_file",
        "runtime_file",
        "architecture",
        "digest",
        "payload_bytes",
        "payload_sha256",
      ]) ||
      entry.role !== expected.role ||
      entry.build_file !== expected.buildFile ||
      entry.runtime_file !== expected.runtimeFile ||
      entry.architecture !== "arm64" ||
      entry.digest !== DIGEST_ALGORITHM ||
      !Number.isSafeInteger(entry.payload_bytes) ||
      entry.payload_bytes <= 0 ||
      entry.payload_bytes > MAX_EXECUTABLE_BYTES ||
      !SHA256.test(entry.payload_sha256)
    ) {
      throw new Error("runtime executable attestation entry is invalid");
    }
  }
  return value;
}

export async function stageRuntimeExecutableAttestation(plan, runtimeSidecars) {
  if (
    !path.isAbsolute(plan.destination) ||
    plan.targetTriple !== SUPPORTED_TARGET ||
    !["debug", "release"].includes(plan.profile) ||
    !Array.isArray(runtimeSidecars) ||
    runtimeSidecars.length !== EXPECTED.length
  ) {
    throw new Error("runtime executable attestation plan is invalid");
  }
  const executables = [];
  for (let index = 0; index < EXPECTED.length; index += 1) {
    const sidecar = runtimeSidecars[index];
    const expected = EXPECTED[index];
    if (
      sidecar.binaryName !== expected.binaryName ||
      path.basename(sidecar.destination) !== expected.buildFile ||
      path.dirname(sidecar.destination) !== path.dirname(plan.destination)
    ) {
      throw new Error("runtime executable attestation build role is invalid");
    }
    await requireExecutable(sidecar.destination);
    const identity = await runtimeExecutablePayloadIdentity(sidecar.destination);
    executables.push({
      role: expected.role,
      build_file: expected.buildFile,
      runtime_file: expected.runtimeFile,
      architecture: identity.architecture,
      digest: DIGEST_ALGORITHM,
      payload_bytes: identity.payloadBytes,
      payload_sha256: identity.payloadSha256,
    });
  }
  const attestation = validateRuntimeExecutableAttestation(
    {
      schema_version: RUNTIME_EXECUTABLE_ATTESTATION_SCHEMA,
      target_triple: plan.targetTriple,
      profile: plan.profile,
      executables,
    },
    plan,
  );
  const parent = path.dirname(plan.destination);
  const temporary = path.join(
    parent,
    `${path.basename(plan.destination)}.tmp-${process.pid}-${Date.now()}`,
  );
  await mkdir(parent, { recursive: true });
  try {
    await writeFile(temporary, `${JSON.stringify(attestation)}\n`, { mode: 0o600 });
    await chmod(temporary, 0o600);
    await rename(temporary, plan.destination);
  } finally {
    await rm(temporary, { force: true });
  }
  return plan.destination;
}

export async function runtimeExecutablePayloadIdentity(file) {
  const bytes = await readFile(file);
  if (
    bytes.length < 32 ||
    bytes.length > MAX_EXECUTABLE_BYTES ||
    bytes.readUInt32LE(0) !== 0xfeedfacf ||
    bytes.readUInt32LE(4) !== 0x0100000c
  ) {
    throw new Error("runtime executable is not a bounded arm64 Mach-O");
  }
  const commandCount = bytes.readUInt32LE(16);
  const commandBytes = bytes.readUInt32LE(20);
  const commandEnd = 32 + commandBytes;
  if (
    commandCount > MAX_LOAD_COMMANDS ||
    commandEnd > bytes.length ||
    commandCount * 8 > commandBytes
  ) {
    throw new Error("runtime executable Mach-O load commands are invalid");
  }
  let offset = 32;
  let signature;
  const linkeditCommands = [];
  for (let index = 0; index < commandCount; index += 1) {
    if (offset + 8 > commandEnd) {
      throw new Error("runtime executable Mach-O load command is truncated");
    }
    const command = bytes.readUInt32LE(offset);
    const size = bytes.readUInt32LE(offset + 4);
    if (size < 8 || offset + size > commandEnd) {
      throw new Error("runtime executable Mach-O load command size is invalid");
    }
    if (command === LC_CODE_SIGNATURE) {
      if (signature || size !== 16) {
        throw new Error("runtime executable Mach-O code signature command is invalid");
      }
      signature = {
        commandOffset: offset,
        dataOffset: bytes.readUInt32LE(offset + 8),
        dataSize: bytes.readUInt32LE(offset + 12),
      };
    }
    if (
      command === LC_SEGMENT_64 &&
      size >= 72 &&
      bytes.subarray(offset + 8, offset + 24).toString("utf8").replaceAll("\0", "") ===
        "__LINKEDIT"
    ) {
      linkeditCommands.push(offset);
    }
    offset += size;
  }
  if (offset !== commandEnd) {
    throw new Error("runtime executable Mach-O load command region is invalid");
  }
  let payload;
  if (!signature) {
    payload = bytes;
  } else {
    if (
      signature.dataSize === 0 ||
      signature.dataOffset < commandEnd ||
      signature.dataOffset + signature.dataSize !== bytes.length
    ) {
      throw new Error("runtime executable Mach-O signature payload is invalid");
    }
    payload = Buffer.from(bytes.subarray(0, signature.dataOffset));
    payload.writeUInt32LE(0, signature.commandOffset + 8);
    payload.writeUInt32LE(0, signature.commandOffset + 12);
    for (const commandOffset of linkeditCommands) {
      payload.writeBigUInt64LE(0n, commandOffset + 32);
      payload.writeBigUInt64LE(0n, commandOffset + 48);
    }
  }
  return Object.freeze({
    architecture: "arm64",
    payloadBytes: payload.length,
    payloadSha256: createHash("sha256").update(payload).digest("hex"),
  });
}

async function requireExecutable(file) {
  let metadata;
  try {
    metadata = await lstat(file);
  } catch {
    throw new Error("attested runtime executable is missing");
  }
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size === 0) {
    throw new Error("attested runtime executable must be a regular non-symlink file");
  }
  if (process.platform !== "win32" && (metadata.mode & 0o111) === 0) {
    throw new Error("attested runtime executable must be executable");
  }
  if (process.platform !== "win32" && (metadata.mode & 0o022) !== 0) {
    throw new Error("attested runtime executable permissions are unsafe");
  }
}

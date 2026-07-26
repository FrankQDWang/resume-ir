import { createHash } from "node:crypto";
import { chmod, lstat, mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import path from "node:path";

export const RUNTIME_EXECUTABLE_ATTESTATION_SCHEMA =
  "resume-ir.runtime-executable-attestation.v1";

const DIGEST_ALGORITHM = "sha256_without_code_signature_v1";
const MAX_EXECUTABLE_BYTES = 256 * 1024 * 1024;
const MAX_LOAD_COMMANDS = 4096;
const LC_CODE_SIGNATURE = 0x1d;
const LC_SEGMENT_64 = 0x19;
const TARGETS = Object.freeze({
  "aarch64-apple-darwin": Object.freeze({
    architecture: "arm64",
    suffix: "",
  }),
  "x86_64-pc-windows-msvc": Object.freeze({
    architecture: "x86_64",
    suffix: ".exe",
  }),
});
const SHA256 = /^[a-f0-9]{64}$/;

function expectedExecutables(targetTriple) {
  const target = TARGETS[targetTriple];
  if (!target) throw new Error("runtime executable attestation target is unsupported");
  return Object.freeze([
    Object.freeze({
      role: "embedding_runtime",
      binaryName: "resume-embedding-runtime",
      buildFile: `resume-embedding-runtime-${targetTriple}${target.suffix}`,
      runtimeFile: `resume-embedding-runtime${target.suffix}`,
    }),
    Object.freeze({
      role: "pdf_renderer",
      binaryName: "resume-pdf-render-runtime",
      buildFile: `resume-pdf-render-runtime-${targetTriple}${target.suffix}`,
      runtimeFile: `resume-pdf-render-runtime${target.suffix}`,
    }),
  ]);
}

function exactKeys(value, expected) {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    JSON.stringify(Object.keys(value)) === JSON.stringify(expected)
  );
}

export function validateRuntimeExecutableAttestation(value, plan) {
  const target = TARGETS[value?.target_triple];
  const expectedExecutablesForTarget = target
    ? expectedExecutables(value.target_triple)
    : [];
  if (
    !exactKeys(value, ["schema_version", "target_triple", "profile", "executables"]) ||
    value.schema_version !== RUNTIME_EXECUTABLE_ATTESTATION_SCHEMA ||
    !target ||
    value.target_triple !== plan.targetTriple ||
    !["debug", "release"].includes(value.profile) ||
    value.profile !== plan.profile ||
    !Array.isArray(value.executables) ||
    value.executables.length !== expectedExecutablesForTarget.length
  ) {
    throw new Error("runtime executable attestation contract is invalid");
  }
  for (let index = 0; index < expectedExecutablesForTarget.length; index += 1) {
    const entry = value.executables[index];
    const expected = expectedExecutablesForTarget[index];
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
      entry.architecture !== target.architecture ||
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
  const expectedExecutablesForTarget = expectedExecutables(plan.targetTriple);
  if (
    !path.isAbsolute(plan.destination) ||
    !["debug", "release"].includes(plan.profile) ||
    !Array.isArray(runtimeSidecars) ||
    runtimeSidecars.length !== expectedExecutablesForTarget.length
  ) {
    throw new Error("runtime executable attestation plan is invalid");
  }
  const executables = [];
  for (let index = 0; index < expectedExecutablesForTarget.length; index += 1) {
    const sidecar = runtimeSidecars[index];
    const expected = expectedExecutablesForTarget[index];
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
  if (bytes.subarray(0, 2).toString("ascii") === "MZ") {
    return pePayloadIdentity(bytes);
  }
  return machoPayloadIdentity(bytes);
}

function machoPayloadIdentity(bytes) {
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

function pePayloadIdentity(original) {
  if (original.length < 0x40 || original.length > MAX_EXECUTABLE_BYTES) {
    throw new Error("runtime executable is not a bounded x86_64 PE");
  }
  const bytes = Buffer.from(original);
  const peOffset = bytes.readUInt32LE(0x3c);
  const coffOffset = peOffset + 4;
  const optionalOffset = coffOffset + 20;
  if (
    peOffset > bytes.length - 24 ||
    bytes.subarray(peOffset, peOffset + 4).toString("binary") !== "PE\u0000\u0000" ||
    bytes.readUInt16LE(coffOffset) !== 0x8664 ||
    optionalOffset > bytes.length - 2 ||
    bytes.readUInt16LE(optionalOffset) !== 0x20b
  ) {
    throw new Error("runtime executable is not a bounded x86_64 PE");
  }
  const optionalBytes = bytes.readUInt16LE(coffOffset + 16);
  const optionalEnd = optionalOffset + optionalBytes;
  const checksumOffset = optionalOffset + 64;
  const directoryCountOffset = optionalOffset + 108;
  const securityEntryOffset = optionalOffset + 112 + 4 * 8;
  if (
    optionalEnd > bytes.length ||
    checksumOffset + 4 > optionalEnd ||
    directoryCountOffset + 4 > optionalEnd ||
    bytes.readUInt32LE(directoryCountOffset) <= 4 ||
    securityEntryOffset + 8 > optionalEnd
  ) {
    throw new Error("runtime executable PE optional header is invalid");
  }
  bytes.fill(0, checksumOffset, checksumOffset + 4);
  const certificateOffset = bytes.readUInt32LE(securityEntryOffset);
  const certificateBytes = bytes.readUInt32LE(securityEntryOffset + 4);
  bytes.fill(0, securityEntryOffset, securityEntryOffset + 8);
  let payload = bytes;
  if (certificateOffset !== 0 || certificateBytes !== 0) {
    if (
      certificateOffset < optionalEnd ||
      certificateBytes === 0 ||
      certificateOffset + certificateBytes !== bytes.length
    ) {
      throw new Error("runtime executable PE signature payload is invalid");
    }
    payload = bytes.subarray(0, certificateOffset);
  }
  return Object.freeze({
    architecture: "x86_64",
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

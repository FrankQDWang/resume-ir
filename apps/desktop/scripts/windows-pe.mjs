const MAX_PE_BYTES = 256 * 1024 * 1024;

function readCString(buffer, offset, label) {
  let end = offset;
  while (end < buffer.length && end - offset <= 128 && buffer[end] !== 0) end += 1;
  if (end === buffer.length || end - offset > 128) {
    throw new Error(`${label} is invalid`);
  }
  const value = buffer.toString("ascii", offset, end);
  if (!/^[A-Za-z0-9_.-]+$/.test(value)) throw new Error(`${label} is invalid`);
  return value;
}

function inspectWindowsPe(buffer, imageKind) {
  if (!Buffer.isBuffer(buffer) || buffer.length < 1024 || buffer.length > MAX_PE_BYTES) {
    throw new Error("Windows PE image is invalid");
  }
  const pe = buffer.readUInt32LE(0x3c);
  if (
    buffer.toString("ascii", 0, 2) !== "MZ" ||
    pe < 64 ||
    pe + 24 > buffer.length ||
    buffer.toString("ascii", pe, pe + 4) !== "PE\0\0" ||
    buffer.readUInt16LE(pe + 4) !== 0x8664
  ) {
    throw new Error("Windows PE image is not x64");
  }
  const sectionCount = buffer.readUInt16LE(pe + 6);
  const optionalSize = buffer.readUInt16LE(pe + 20);
  const characteristics = buffer.readUInt16LE(pe + 22);
  const optional = pe + 24;
  const sectionTable = optional + optionalSize;
  const isDll = (characteristics & 0x2000) !== 0;
  if (
    sectionCount === 0 ||
    sectionCount > 96 ||
    optionalSize < 128 ||
    sectionTable + sectionCount * 40 > buffer.length ||
    buffer.readUInt16LE(optional) !== 0x20b ||
    buffer.readUInt32LE(optional + 108) < 2
  ) {
    throw new Error("Windows PE image shape is invalid");
  }
  if (imageKind === "executable" && (isDll || (characteristics & 0x0002) === 0)) {
    throw new Error("Windows PE executable shape is invalid");
  }
  if (imageKind === "dynamic_library" && !isDll) {
    throw new Error("Windows PE dynamic library shape is invalid");
  }
  const sections = Array.from({ length: sectionCount }, (_, index) => {
    const start = sectionTable + index * 40;
    return {
      virtualAddress: buffer.readUInt32LE(start + 12),
      rawSize: buffer.readUInt32LE(start + 16),
      rawOffset: buffer.readUInt32LE(start + 20),
    };
  });
  const rvaOffset = (rva) => {
    const section = sections.find((entry) => {
      const delta = rva - entry.virtualAddress;
      return delta >= 0 && delta < entry.rawSize;
    });
    if (!section) throw new Error("Windows PE directory is invalid");
    const offset = section.rawOffset + rva - section.virtualAddress;
    if (offset >= buffer.length) throw new Error("Windows PE directory is invalid");
    return offset;
  };
  const directory = (index) => ({
    rva: buffer.readUInt32LE(optional + 112 + index * 8),
    size: buffer.readUInt32LE(optional + 116 + index * 8),
  });
  const exportDirectory = directory(0);
  const importDirectory = directory(1);
  if (!importDirectory.rva || !importDirectory.size) {
    throw new Error("Windows PE import directory is incomplete");
  }
  const exports = [];
  if (exportDirectory.rva || exportDirectory.size) {
    if (!exportDirectory.rva || !exportDirectory.size) {
      throw new Error("Windows PE export directory is invalid");
    }
    const exportOffset = rvaOffset(exportDirectory.rva);
    if (exportOffset + 40 > buffer.length) {
      throw new Error("Windows PE exports are invalid");
    }
    const exportCount = buffer.readUInt32LE(exportOffset + 24);
    const exportNamesRva = buffer.readUInt32LE(exportOffset + 32);
    if (exportCount > 4096 || (exportCount > 0 && !exportNamesRva)) {
      throw new Error("Windows PE exports are invalid");
    }
    if (exportCount > 0) {
      const exportNamesOffset = rvaOffset(exportNamesRva);
      if (exportNamesOffset + exportCount * 4 > buffer.length) {
        throw new Error("Windows PE exports are invalid");
      }
      for (let index = 0; index < exportCount; index += 1) {
        exports.push(
          readCString(
            buffer,
            rvaOffset(buffer.readUInt32LE(exportNamesOffset + index * 4)),
            "Windows PE export",
          ),
        );
      }
    }
  }
  if (imageKind === "dynamic_library" && exports.length === 0) {
    throw new Error("Windows PE dynamic library exports are incomplete");
  }
  const imports = [];
  let descriptor = rvaOffset(importDirectory.rva);
  let terminated = false;
  const descriptorLimit = Math.min(512, Math.floor(importDirectory.size / 20));
  for (
    let index = 0;
    index < descriptorLimit && descriptor + 20 <= buffer.length;
    index += 1
  ) {
    const fields = Array.from({ length: 5 }, (_, field) =>
      buffer.readUInt32LE(descriptor + field * 4),
    );
    if (fields.every((value) => value === 0)) {
      terminated = true;
      break;
    }
    imports.push(readCString(buffer, rvaOffset(fields[3]), "Windows PE import"));
    descriptor += 20;
  }
  if (!terminated || imports.length === 0) {
    throw new Error("Windows PE imports are invalid");
  }
  return Object.freeze({
    exports: Object.freeze([...new Set(exports)].sort()),
    imports: Object.freeze(
      [...new Set(imports.map((value) => value.toUpperCase()))].sort(),
    ),
  });
}

export function inspectWindowsPeDynamicLibrary(buffer) {
  return inspectWindowsPe(buffer, "dynamic_library");
}

export function inspectWindowsPeExecutable(buffer) {
  return inspectWindowsPe(buffer, "executable");
}

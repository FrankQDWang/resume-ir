import {
  chmod,
  copyFile,
  mkdir,
  rename,
  rm,
} from "node:fs/promises";
import path from "node:path";

import { verifyMacosPdfiumStaticPack } from "./macos-pdfium-static-pack.mjs";

/**
 * Copies the reviewed compile-time PDFium archive into an immutable build
 * snapshot without converting it into the smaller installed resource pack.
 */
export async function stageMacosPdfiumStaticBuildPack({
  destination,
  directory,
  sourceContract,
}) {
  for (const value of [destination, directory, sourceContract]) {
    if (!path.isAbsolute(value ?? "")) {
      throw new Error("PDFium static build pack paths must be absolute");
    }
  }
  if (destination === directory) {
    throw new Error("PDFium static build pack destination must be isolated");
  }
  const verified = await verifyMacosPdfiumStaticPack({
    directory,
    sourceContract,
  });
  const entries = [
    verified.contract.pack.library_file,
    verified.contract.pack.license_file,
    verified.contract.pack.args_file,
    "runtime-pack.json",
  ];
  const parent = path.dirname(destination);
  const temporary = path.join(
    parent,
    `${path.basename(destination)}.tmp-${process.pid}-${Date.now()}`,
  );
  const backup = path.join(
    parent,
    `${path.basename(destination)}.old-${process.pid}-${Date.now()}`,
  );
  await mkdir(parent, { recursive: true });
  await rm(temporary, { recursive: true, force: true });
  await rm(backup, { recursive: true, force: true });
  await mkdir(temporary, { mode: 0o700 });
  try {
    for (const entry of entries) {
      const target = path.join(temporary, entry);
      await copyFile(path.join(directory, entry), target);
      await chmod(target, 0o644);
    }
    await verifyMacosPdfiumStaticPack({
      directory: temporary,
      sourceContract,
    });
    let previous = false;
    try {
      await rename(destination, backup);
      previous = true;
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
    }
    try {
      await rename(temporary, destination);
    } catch (error) {
      if (previous) await rename(backup, destination);
      throw error;
    }
    await verifyMacosPdfiumStaticPack({ directory: destination, sourceContract });
    await rm(backup, { recursive: true, force: true });
  } finally {
    await rm(temporary, { recursive: true, force: true });
    await rm(backup, { recursive: true, force: true });
  }
}

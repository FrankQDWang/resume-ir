import { readFile, readdir } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

const DEFAULT_PDFJS_ROOT = fileURLToPath(
  new URL("../node_modules/pdfjs-dist/", import.meta.url),
)
const RESOURCE_DIRECTORIES = Object.freeze(["cmaps", "standard_fonts"])

export async function pdfJsAssetEntries(pdfJsRoot = DEFAULT_PDFJS_ROOT) {
  const entries = []
  for (const directory of RESOURCE_DIRECTORIES) {
    const sourceDirectory = path.join(pdfJsRoot, directory)
    for (const name of await readdir(sourceDirectory)) {
      entries.push({
        source: path.join(sourceDirectory, name),
        output: `pdfjs/${directory}/${name}`,
      })
    }
  }
  return entries.sort((left, right) => left.output.localeCompare(right.output))
}

export async function verifyPdfJsBuildAssets(
  distRoot,
  pdfJsRoot = DEFAULT_PDFJS_ROOT,
) {
  const expected = await pdfJsAssetEntries(pdfJsRoot)
  const actual = []
  for (const directory of RESOURCE_DIRECTORIES) {
    const outputDirectory = path.join(distRoot, "pdfjs", directory)
    for (const name of await readdir(outputDirectory)) {
      actual.push(`pdfjs/${directory}/${name}`)
    }
  }
  actual.sort()
  const expectedOutputs = expected.map(({ output }) => output)
  const expectedOutputSet = new Set(expectedOutputs)
  if (
    actual.length !== expectedOutputs.length
    || actual.some((output) => !expectedOutputSet.has(output))
  ) {
    throw new Error("PDF.js build resource manifest does not match the locked dependency")
  }
  let byteLength = 0
  for (const entry of expected) {
    if (entry.output.includes("://")) {
      throw new Error("PDF.js build resource URL must remain local")
    }
    const [source, output] = await Promise.all([
      readFile(entry.source),
      readFile(path.join(distRoot, entry.output)),
    ])
    if (!source.equals(output)) {
      throw new Error(`PDF.js build resource differs from dependency: ${entry.output}`)
    }
    byteLength += output.byteLength
  }
  for (const required of [
    "pdfjs/cmaps/Adobe-GB1-UCS2.bcmap",
    "pdfjs/cmaps/LICENSE",
    "pdfjs/standard_fonts/LICENSE_FOXIT",
    "pdfjs/standard_fonts/LICENSE_LIBERATION",
  ]) {
    if (!actual.includes(required)) {
      throw new Error(`PDF.js build resource is missing: ${required}`)
    }
  }
  return { fileCount: actual.length, byteLength }
}

export function pdfJsAssets(pdfJsRoot = DEFAULT_PDFJS_ROOT) {
  let entries
  const loadEntries = () => entries ??= pdfJsAssetEntries(pdfJsRoot)
  return [{
    name: "resume-ir-pdfjs-assets-build",
    apply: "build",
    async generateBundle() {
      for (const entry of await loadEntries()) {
        this.emitFile({
          type: "asset",
          fileName: entry.output,
          source: await readFile(entry.source),
        })
      }
    },
  }, {
    name: "resume-ir-pdfjs-assets-serve",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use(async (request, response, next) => {
        const pathname = new URL(request.url ?? "/", "http://localhost").pathname
        const entry = (await loadEntries()).find(
          ({ output }) => pathname === `/${output}`,
        )
        if (!entry) {
          next()
          return
        }
        try {
          response.statusCode = 200
          response.setHeader("Content-Type", "application/octet-stream")
          response.setHeader("Cache-Control", "no-store")
          response.end(await readFile(entry.source))
        } catch {
          response.statusCode = 404
          response.end()
        }
      })
    },
  }]
}

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

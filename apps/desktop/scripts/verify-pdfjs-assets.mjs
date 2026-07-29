import path from "node:path"

import { verifyPdfJsBuildAssets } from "./pdfjs-assets.mjs"

const summary = await verifyPdfJsBuildAssets(path.join(process.cwd(), "dist"))
console.log(
  `PDF.js build assets verified: ${summary.fileCount} files, ${summary.byteLength} bytes`,
)

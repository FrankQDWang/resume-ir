import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import path from "node:path"
import test from "node:test"

import { createCanvas } from "@napi-rs/canvas"
import { getDocument } from "pdfjs-dist/legacy/build/pdf.mjs"

import { pdfJsAssetEntries } from "./pdfjs-assets.mjs"

const encoder = new TextEncoder()

function syntheticCjkCidPdf(fontBytes) {
  const chunks = []
  const offsets = [0]
  let byteLength = 0
  const append = (value) => {
    const bytes = typeof value === "string" ? encoder.encode(value) : value
    chunks.push(bytes)
    byteLength += bytes.byteLength
  }
  const appendObject = (number, parts) => {
    offsets[number] = byteLength
    append(`${number} 0 obj\n`)
    for (const part of parts) append(part)
    append("\nendobj\n")
  }

  append("%PDF-1.7\n%âãÏÓ\n")
  appendObject(1, ["<< /Type /Catalog /Pages 2 0 R >>"])
  appendObject(2, ["<< /Type /Pages /Kids [3 0 R] /Count 1 >>"])
  appendObject(3, [
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] ",
    "/Resources << /Font << /F1 4 0 R >> >> /Contents 8 0 R >>",
  ])
  appendObject(4, [
    "<< /Type /Font /Subtype /Type0 /BaseFont /SyntheticCJK ",
    "/Encoding /Identity-H /DescendantFonts [5 0 R] >>",
  ])
  appendObject(5, [
    "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SyntheticCJK ",
    "/CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 5 >> ",
    "/FontDescriptor 6 0 R /DW 1000 /CIDToGIDMap 9 0 R >>",
  ])
  appendObject(6, [
    "<< /Type /FontDescriptor /FontName /SyntheticCJK /Flags 4 ",
    "/FontBBox [-600 -400 2100 1100] /ItalicAngle 0 /Ascent 905 ",
    "/Descent -212 /CapHeight 687 /StemV 80 /FontFile2 7 0 R >>",
  ])
  appendObject(7, [
    `<< /Length ${fontBytes.byteLength} /Length1 ${fontBytes.byteLength} >>\nstream\n`,
    fontBytes,
    "\nendstream",
  ])
  const content = "BT /F1 48 Tf 30 100 Td <1b581b581b581b58> Tj ET"
  appendObject(8, [
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
  ])
  const cidToGidMap = new Uint8Array((7_000 + 1) * 2)
  cidToGidMap[7_000 * 2 + 1] = 36
  appendObject(9, [
    `<< /Length ${cidToGidMap.byteLength} >>\nstream\n`,
    cidToGidMap,
    "\nendstream",
  ])
  const xrefOffset = byteLength
  append("xref\n0 10\n0000000000 65535 f \n")
  for (let object = 1; object <= 9; object += 1) {
    append(`${String(offsets[object]).padStart(10, "0")} 00000 n \n`)
  }
  append(`trailer\n<< /Size 10 /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`)

  const pdf = new Uint8Array(byteLength)
  let offset = 0
  for (const chunk of chunks) {
    pdf.set(chunk, offset)
    offset += chunk.byteLength
  }
  return pdf
}

test("ships the complete local PDF.js CMap and standard-font resource sets", async () => {
  const entries = await pdfJsAssetEntries()
  const outputs = entries.map(({ output }) => output)

  assert.equal(entries.length, 185)
  assert.ok(outputs.includes("pdfjs/cmaps/Adobe-GB1-UCS2.bcmap"))
  assert.ok(outputs.includes("pdfjs/cmaps/LICENSE"))
  assert.ok(outputs.includes("pdfjs/standard_fonts/LiberationSans-Regular.ttf"))
  assert.ok(outputs.includes("pdfjs/standard_fonts/LICENSE_FOXIT"))
  assert.ok(outputs.includes("pdfjs/standard_fonts/LICENSE_LIBERATION"))
  assert.equal(outputs.some((output) => output.includes("://")), false)
  for (const entry of entries) {
    assert.ok((await readFile(entry.source)).byteLength > 0)
  }
})

test("renders a no-ToUnicode Adobe-GB1 CID font instead of silently dropping glyphs", async () => {
  const pdfJsRoot = path.join(process.cwd(), "node_modules/pdfjs-dist")
  const fontBytes = new Uint8Array(await readFile(path.join(
    pdfJsRoot,
    "standard_fonts/LiberationSans-Regular.ttf",
  )))
  const document = await getDocument({
    data: syntheticCjkCidPdf(fontBytes),
    useSystemFonts: false,
    cMapUrl: path.join(pdfJsRoot, "cmaps/"),
    cMapPacked: true,
    standardFontDataUrl: path.join(pdfJsRoot, "standard_fonts/"),
  }).promise
  const page = await document.getPage(1)
  const text = (await page.getTextContent()).items.map(({ str }) => str).join("")
  assert.equal(text, "鹨鹨鹨鹨")
  const viewport = page.getViewport({ scale: 1 })
  const canvas = createCanvas(viewport.width, viewport.height)
  const context = canvas.getContext("2d")
  context.fillStyle = "white"
  context.fillRect(0, 0, viewport.width, viewport.height)

  await page.render({ canvas, canvasContext: context, viewport }).promise
  const pixels = context.getImageData(0, 0, viewport.width, viewport.height).data
  let darkPixels = 0
  for (let offset = 0; offset < pixels.length; offset += 4) {
    if (pixels[offset] < 240 || pixels[offset + 1] < 240 || pixels[offset + 2] < 240) {
      darkPixels += 1
    }
  }

  assert.ok(darkPixels > 1_000, `expected rendered glyphs, got ${darkPixels} dark pixels`)
  await document.destroy()
})

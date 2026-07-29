import { clearMocks, mockIPC } from "@tauri-apps/api/mocks"
import { beforeEach, describe, expect, it } from "vitest"

import { deliverDaemonPdfRange } from "./pdf-preview-range"
import { pdfJsResourceOptions } from "./pdf-preview-resources"

interface RangeRequest {
  operation: "preview_range"
  body: {
    request_id: string
    lease_id: string
    offset: number
    length: number
  }
}

if (typeof window === "undefined") {
  Object.defineProperty(globalThis, "window", { configurable: true, value: globalThis })
}

beforeEach(() => {
  clearMocks()
})

describe("daemon PDF range transport", () => {
  it("delivers one PDF.js range after reading multiple bounded daemon chunks", async () => {
    const requests: RangeRequest[] = []
    mockIPC((_command, payload) => {
      const request = (payload as { request: RangeRequest }).request
      requests.push(request)
      const bytes = Uint8Array.from(
        { length: request.body.length },
        (_, index) => request.body.offset + index,
      )
      return {
        http_status: 200,
        body: {
          schema_version: "resume-ir.source-preview-range.v1",
          request_id: request.body.request_id,
          status: "ok",
          offset: request.body.offset,
          bytes_read: bytes.byteLength,
          total_bytes: 12,
          data_base64: btoa(String.fromCharCode(...bytes)),
        },
      }
    })
    const delivered: Array<{ begin: number; bytes: Uint8Array }> = []
    await deliverDaemonPdfRange({
      totalBytes: 12,
      leaseId: "a".repeat(64),
      rangeBytes: 4,
      begin: 4,
      end: 12,
      onDataRange: (begin, bytes) => delivered.push({ begin, bytes }),
    })

    expect(requests).toHaveLength(2)
    expect(requests.map(({ body }) => [body.offset, body.length])).toEqual([
      [4, 4],
      [8, 4],
    ])
    expect(delivered).toEqual([
      { begin: 4, bytes: Uint8Array.from([4, 5, 6, 7, 8, 9, 10, 11]) },
    ])
  })
})

describe("PDF.js CJK CID rendering", () => {
  it("resolves local same-origin resource directories", () => {
    expect(pdfJsResourceOptions("tauri://localhost/pdfjs")).toEqual({
      cMapUrl: "tauri://localhost/pdfjs/cmaps/",
      cMapPacked: true,
      standardFontDataUrl: "tauri://localhost/pdfjs/standard_fonts/",
    })
  })
})

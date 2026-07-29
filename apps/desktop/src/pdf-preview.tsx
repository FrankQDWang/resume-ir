import { useEffect, useRef, useState } from "react"
import {
  GlobalWorkerOptions,
  PDFDataRangeTransport,
  getDocument,
  type PDFDocumentProxy,
  type PDFPageProxy,
} from "pdfjs-dist"
import workerUrl from "pdfjs-dist/build/pdf.worker.mjs?url"
import { ChevronLeft, ChevronRight, LoaderCircle } from "lucide-react"

import {
  closeSourcePreview,
  createSourcePreview,
  readSourcePreviewRange,
  type SearchSelection,
} from "./daemon"
import { deliverDaemonPdfRange } from "./pdf-preview-range"

GlobalWorkerOptions.workerSrc = workerUrl

function decodeBase64(value: string): Uint8Array {
  const decoded = atob(value)
  const bytes = new Uint8Array(decoded.length)
  for (let index = 0; index < decoded.length; index += 1) {
    bytes[index] = decoded.charCodeAt(index)
  }
  return bytes
}

class DaemonPdfRangeTransport extends PDFDataRangeTransport {
  constructor(
    private readonly totalBytes: number,
    initialData: Uint8Array,
    private readonly leaseId: string,
    private readonly rangeBytes: number,
    private readonly onFailure: (message: string) => void,
  ) {
    super(totalBytes, initialData, false)
  }

  requestDataRange(begin: number, end: number): void {
    if (
      !Number.isSafeInteger(begin)
      || !Number.isSafeInteger(end)
      || begin < 0
      || end <= begin
      || end > this.totalBytes
    ) {
      this.onFailure("原始 PDF 读取范围无效；可关闭详情后重新打开")
      return
    }
    void this.deliverRange(begin, end).catch(() => {
      this.onFailure("原始 PDF 读取已中断；可关闭详情后重新打开")
    })
  }

  private async deliverRange(begin: number, end: number): Promise<void> {
    await deliverDaemonPdfRange({
      totalBytes: this.totalBytes,
      leaseId: this.leaseId,
      rangeBytes: this.rangeBytes,
      begin,
      end,
      onDataRange: (offset, bytes) => this.onDataRange(offset, bytes),
    })
  }
}

export function PdfPreview({ selection, fileName }: {
  selection: SearchSelection
  fileName: string
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const renderRef = useRef<{ cancel: () => void } | null>(null)
  const [document, setDocument] = useState<PDFDocumentProxy | null>(null)
  const [pageNumber, setPageNumber] = useState(1)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState("")

  useEffect(() => {
    let disposed = false
    let leaseId: string | null = null
    let loaded: PDFDocumentProxy | null = null
    setLoading(true)
    setError("")
    setPageNumber(1)
    const open = async () => {
      const requestId = `gui-preview-create-${crypto.randomUUID()}`
      const created = await createSourcePreview(requestId, selection)
      if (
        created.http_status !== 200
        || created.body.schema_version !== "resume-ir.source-preview.v1"
        || created.body.request_id !== requestId
      ) {
        throw new Error("source preview unavailable")
      }
      leaseId = created.body.lease_id
      const initialLength = Math.min(created.body.range_bytes, created.body.byte_size)
      const initialRequestId = `gui-preview-range-${crypto.randomUUID()}`
      const initial = await readSourcePreviewRange(
        initialRequestId,
        leaseId,
        0,
        initialLength,
      )
      if (
        initial.http_status !== 200
        || initial.body.schema_version !== "resume-ir.source-preview-range.v1"
        || initial.body.request_id !== initialRequestId
        || initial.body.offset !== 0
        || initial.body.total_bytes !== created.body.byte_size
        || initial.body.bytes_read !== initialLength
      ) {
        throw new Error("source preview range unavailable")
      }
      const initialBytes = decodeBase64(initial.body.data_base64)
      if (initialBytes.byteLength !== initial.body.bytes_read) {
        throw new Error("source preview range length mismatch")
      }
      const transport = new DaemonPdfRangeTransport(
        created.body.byte_size,
        initialBytes,
        leaseId,
        created.body.range_bytes,
        setError,
      )
      loaded = await getDocument({
        range: transport,
        rangeChunkSize: created.body.range_bytes,
        disableAutoFetch: true,
        disableStream: true,
        disableRange: false,
      }).promise
      if (disposed) {
        await loaded.destroy()
        if (leaseId) {
          await closeSourcePreview(`gui-preview-close-${crypto.randomUUID()}`, leaseId)
          leaseId = null
        }
        return
      }
      setDocument(loaded)
      setLoading(false)
    }
    void open().catch(() => {
      if (leaseId) {
        void closeSourcePreview(`gui-preview-close-${crypto.randomUUID()}`, leaseId)
        leaseId = null
      }
      if (!disposed) {
        setError("原始 PDF 当前不可用；结构化字段和提取文本仍可查看")
        setLoading(false)
      }
    })
    return () => {
      disposed = true
      renderRef.current?.cancel()
      if (loaded) void loaded.destroy()
      if (leaseId) {
        void closeSourcePreview(`gui-preview-close-${crypto.randomUUID()}`, leaseId)
      }
    }
  }, [selection.doc_id, selection.version_id, selection.visible_epoch])

  useEffect(() => {
    if (!document || !canvasRef.current) return
    let disposed = false
    const render = async () => {
      const page: PDFPageProxy = await document.getPage(pageNumber)
      if (disposed || !canvasRef.current) return
      const viewport = page.getViewport({ scale: 1.35 })
      const canvas = canvasRef.current
      const context = canvas.getContext("2d", { alpha: false })
      if (!context) throw new Error("canvas unavailable")
      const pixelRatio = Math.min(window.devicePixelRatio || 1, 2)
      canvas.width = Math.floor(viewport.width * pixelRatio)
      canvas.height = Math.floor(viewport.height * pixelRatio)
      canvas.style.width = `${viewport.width}px`
      canvas.style.height = `${viewport.height}px`
      context.setTransform(pixelRatio, 0, 0, pixelRatio, 0, 0)
      const task = page.render({ canvas, canvasContext: context, viewport })
      renderRef.current = task
      await task.promise
      renderRef.current = null
    }
    void render().catch(() => {
      if (!disposed) setError("当前 PDF 页面渲染失败")
    })
    return () => {
      disposed = true
      renderRef.current?.cancel()
    }
  }, [document, pageNumber])

  if (loading) {
    return <div className="pdf-preview-state"><LoaderCircle className="spin" size={18} />正在打开原始 PDF</div>
  }
  if (!document) {
    return <div className="pdf-preview-state pdf-preview-error">{error}</div>
  }
  return <section className="pdf-preview" aria-label={`${fileName} 原始 PDF`}>
    <header>
      <strong>原始简历</strong>
      <div>
        <button type="button" className="icon-button" aria-label="上一页" disabled={pageNumber <= 1} onClick={() => setPageNumber((page) => Math.max(1, page - 1))}><ChevronLeft size={16} /></button>
        <span>{pageNumber} / {document.numPages}</span>
        <button type="button" className="icon-button" aria-label="下一页" disabled={pageNumber >= document.numPages} onClick={() => setPageNumber((page) => Math.min(document.numPages, page + 1))}><ChevronRight size={16} /></button>
      </div>
    </header>
    {error && <p className="pdf-preview-warning">{error}</p>}
    <div className="pdf-page"><canvas ref={canvasRef} /></div>
  </section>
}

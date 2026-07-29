import { readSourcePreviewRange } from "./daemon"

function decodeBase64(value: string): Uint8Array {
  const decoded = atob(value)
  const bytes = new Uint8Array(decoded.length)
  for (let index = 0; index < decoded.length; index += 1) {
    bytes[index] = decoded.charCodeAt(index)
  }
  return bytes
}

export async function deliverDaemonPdfRange({
  totalBytes,
  leaseId,
  rangeBytes,
  begin,
  end,
  onDataRange,
}: {
  totalBytes: number
  leaseId: string
  rangeBytes: number
  begin: number
  end: number
  onDataRange: (offset: number, bytes: Uint8Array) => void
}): Promise<void> {
  const requestedLength = end - begin
  const assembled = new Uint8Array(requestedLength)
  let offset = begin
  while (offset < end) {
    const length = Math.min(rangeBytes, end - offset)
    const requestId = `gui-preview-range-${crypto.randomUUID()}`
    const reply = await readSourcePreviewRange(
      requestId,
      leaseId,
      offset,
      length,
    )
    if (
      reply.http_status !== 200
      || reply.body.schema_version !== "resume-ir.source-preview-range.v1"
      || reply.body.request_id !== requestId
      || reply.body.offset !== offset
      || reply.body.total_bytes !== totalBytes
      || reply.body.bytes_read <= 0
      || reply.body.bytes_read > length
    ) {
      throw new Error("range contract mismatch")
    }
    const bytes = decodeBase64(reply.body.data_base64)
    if (bytes.byteLength !== reply.body.bytes_read) {
      throw new Error("range length mismatch")
    }
    assembled.set(bytes, offset - begin)
    offset += bytes.byteLength
    if (bytes.byteLength < length && offset < end) {
      throw new Error("premature end of file")
    }
  }
  onDataRange(begin, assembled)
}

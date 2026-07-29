import type {
  DocumentInitParameters,
  PDFDocumentLoadingTask,
} from "pdfjs-dist/types/src/display/api"

type PdfDocumentLoader = (
  options: DocumentInitParameters,
) => PDFDocumentLoadingTask

export function pdfJsResourceOptions(resourceRoot: string) {
  const root = resourceRoot.endsWith("/") ? resourceRoot : `${resourceRoot}/`
  return {
    cMapUrl: `${root}cmaps/`,
    cMapPacked: true,
    standardFontDataUrl: `${root}standard_fonts/`,
  } as const
}

export function createPdfDocumentLoadingTask(
  options: DocumentInitParameters,
  resourceRoot: string,
  loadDocument: PdfDocumentLoader,
) {
  return loadDocument({
    ...options,
    ...pdfJsResourceOptions(resourceRoot),
  })
}

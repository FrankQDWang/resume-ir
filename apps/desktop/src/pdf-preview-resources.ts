export function pdfJsResourceOptions(resourceRoot: string) {
  const root = resourceRoot.endsWith("/") ? resourceRoot : `${resourceRoot}/`
  return {
    cMapUrl: `${root}cmaps/`,
    cMapPacked: true,
    standardFontDataUrl: `${root}standard_fonts/`,
  } as const
}

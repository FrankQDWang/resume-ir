# License Governance

The current source license is GPL-3.0-or-later to match the bundled-first
Poppler/pdftoppm release path. Release licensing is selected by the reviewed
runtime distribution. Bundled runtime releases use GPL-3.0-or-later unless a
later legal review records a different compatible choice for the selected OCR,
PDF renderer, model, or tool chain. Third-party dependency, model, OCR,
dictionary, and tool licenses must be reviewed before assets are shipped or
committed.

The bundled-first runtime policy still keeps an external override path for
operators who must pin reviewed local binaries outside the product bundle.
PDFium remains the preferred bundled renderer candidate when it satisfies
quality and platform requirements; Poppler/pdftoppm is acceptable under the
GPL-compatible evidence boundary above.

Current rules:

- Rust dependency versions are locked by `Cargo.lock`.
- Real model weights, OCR language data, and local model caches are not tracked.
- GPL-family components are allowed only when the release package records the
  exact license, checksum, source-offer obligations, notices, and product
  rationale. AGPL, SSPL, unclear model licenses, and restrictive data licenses
  remain blocked unless a separate approval records isolation and product
  rationale.
- Release artifacts must include checksums and a reviewed SBOM before stable
  distribution.

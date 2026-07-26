# License Governance

The repository source license remains GPL-3.0-or-later. Product runtime
licensing is recorded per exact reviewed component and does not determine the
repository license. Third-party dependency, model, OCR, dictionary, and tool
licenses must be reviewed before assets are shipped or committed.

The desktop product uses a pinned, statically linked PDFium renderer on macOS
and Windows. Its root license, source commit, dependency revision, build
arguments, runtime-pack digest and final executable identity are all part of
the release contract. Poppler/pdftoppm is not a production desktop runtime or
fallback. The OCR engine remains Tesseract with reviewed tessdata.
The bundled-first product policy requires a reviewed source-offer for each
distributed runtime; an external override remains a local diagnostic lane and
never satisfies installer composition.

Current rules:

- Rust dependency versions are locked by `Cargo.lock`.
- Real model weights, OCR language data, and local model caches are not tracked.
- Every bundled component must record its exact license, checksum, source
  identity, notices and product rationale. AGPL, SSPL, unclear model licenses,
  and restrictive data licenses remain blocked unless a separate approval
  records isolation and product rationale.
- Release artifacts must include checksums and a reviewed SBOM before stable
  distribution.

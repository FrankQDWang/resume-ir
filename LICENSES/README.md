# License Governance

The current source license is MIT, but MIT is not a product packaging constraint.
This repository may change the source or release distribution license before
stable release when the reviewed bundled runtime distribution requires it.
Release licensing is selected by the reviewed runtime distribution.
Bundled runtime releases may move the release distribution to GPL-3.0-or-later or
another compatible license when the selected OCR, PDF renderer, model, or tool
chain requires it. Third-party dependency, model, OCR, dictionary, and tool
licenses must be reviewed before assets are shipped or committed.

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

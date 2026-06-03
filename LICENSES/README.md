# License Governance

The project license is MIT. Third-party dependency, model, OCR, dictionary, and
tool licenses must be reviewed before assets are shipped or committed.

Current rules:

- Rust dependency versions are locked by `Cargo.lock`.
- Real model weights, OCR language data, and local model caches are not tracked.
- GPL, AGPL, SSPL, unclear model licenses, and restrictive data licenses are
  blocked unless a separate approval records isolation and product rationale.
- Release artifacts must include checksums and a reviewed SBOM before stable
  distribution.

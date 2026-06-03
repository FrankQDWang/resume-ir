# Repository Instructions

This is a Rust workspace for a local-first resume search engine. Preserve the
privacy boundary: do not commit or upload real resumes, local data directories,
tokens, diagnostic bundles, model caches, or raw personal data.

Prefer small, test-backed changes. Query paths must remain read-only. Import,
OCR, embedding, and index maintenance work must be cancellable or bounded where
applicable. Public fixtures must be synthetic.

Before proposing a public push or PR, run:

```bash
./scripts/ci/verify-local.sh
```

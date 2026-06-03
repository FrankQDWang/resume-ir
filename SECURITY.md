# Security Policy

## Supported Stage

This project has not shipped a stable release yet. Security work is handled on
the main development line and through pull requests.

## Data Boundary

Do not upload or attach real resumes, local data directories, daemon auth
tokens, diagnostic bundles, model caches, or raw personally identifiable
information. Use synthetic fixtures or redacted reproductions.

Ignored local paths include:

- `local-data/`, `data/`, `resume-data/`, `resumes/`
- `diagnostics/`, `logs/`, `bench-output/`, `bench-results/`
- SQLite databases, log files, diagnostic archives, and model weight files

## Reporting

For now, report security issues privately to the repository owner
`@FrankQDWang`. Public issues should avoid secrets, raw resumes, local paths,
or screenshots containing personal data.

## Security Expectations

- No raw resume text or contact data in logs, debug output, CI artifacts, or
  diagnostics.
- Query paths must remain read-only.
- Local command workers must respect timeouts and avoid leaking payloads.
- New third-party dependencies require license and security review.

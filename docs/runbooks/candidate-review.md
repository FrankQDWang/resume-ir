# Candidate Review Runbook

## Scope

Local-only workflow for reviewing soft-dedupe suggestions and explicitly
folding or splitting candidate records. Do not upload command output, local
data directories, diagnostics, indexes, or resumes.

This workflow is operational tooling only. It is not dedupe-quality evidence
and cannot clear the private business dedupe-quality release blocker.

## Review Suggestions

List redacted same-name soft-dedupe suggestions:

```bash
resume-cli --data-dir <local-data-dir> candidate-review list --limit 20
```

Expected output includes only suggestion counts, opaque local version IDs,
confidence, `folded: false`, and `paths: <redacted>`. It must not print names,
schools, companies, skills, filenames, local paths, or resume text.

## Confirm Merge

Only merge versions after local human review:

```bash
resume-cli --data-dir <local-data-dir> candidate-review merge \
  --version <version-id-a> \
  --version <version-id-b> \
  --confidence 0.91
```

The merge creates a manual local candidate and assigns the selected versions.
Default search then folds those versions by candidate. Low-confidence soft
dedupe suggestions are never folded automatically.

## Undo Merge

Split a manual candidate back into independent versions:

```bash
resume-cli --data-dir <local-data-dir> candidate-review split \
  --candidate <candidate-id>
```

The split clears version assignments and refreshes candidate version counts.
It does not delete resumes, documents, full-text snapshots, vector records, or
diagnostic evidence.

## Release Boundary

Stable release still requires representative private business dedupe-quality
evidence through:

```bash
resume-benchmark dedupe-gate --report private-dedupe-quality.json \
  --require-private-business-labeled
```

Synthetic candidate-review tests only prove workflow behavior and redaction.
They do not prove production dedupe precision, recall, or reviewer quality.

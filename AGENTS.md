# Agent Instructions

## Workflow Routing

Use the curated `fw-*` workflow as the default interface for product work:

- `fw-office-hours`: idea intake, demand reality, and product direction.
- `fw-ceo-review`: CEO-level scope, ambition, and premise review.
- `fw-plan`: write or update the spec plus linked implementation plan.
- `fw-plan-review`: engineering review, with design review when UI is affected.
- `fw-build`: approved implementation with TDD and verification discipline.
- `fw-debug`: root-cause debugging.
- `fw-review`: implementation review gate.
- `fw-ship-lite`: branch finish and release-readiness report only.

One task has one execution owner. Repo files are source of truth; memory is
supporting context only.

## Privacy Boundary

Never commit or upload real resumes, local runtime data, tokens, diagnostic
packages, model caches, or raw personal data. Synthetic fixtures under
`tests/fixtures/` are allowed.

Before any public push, run:

```bash
./scripts/ci/guard-public-repo.sh
```

## Coding Discipline

- For non-trivial behavior changes, state the working assumption and success
  criteria before editing.
- Prefer focused tests before production code.
- Touch only files needed for the current slice.
- Keep interfaces clean; this product has not shipped, so do not add
  compatibility shims unless a current repo document requires them.
- Update `PROGRESS.md` for completed production slices.
- Commit each completed slice separately after verification passes.

## Verification

Use focused tests first, then the relevant broad checks. The default local
pre-PR command is:

```bash
./scripts/ci/verify-local.sh
```

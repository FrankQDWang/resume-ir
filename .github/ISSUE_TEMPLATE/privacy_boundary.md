---
name: Privacy boundary
about: Track privacy boundary evidence, leaks, or guardrail changes.
title: "[Privacy Boundary] "
labels: privacy
assignees: ""
---

<!-- contract:scope -->
## Scope

- [ ] Boundary surface:
- [ ] One execution owner:
- [ ] Data class involved:
- [ ] Public artifact affected:
- [ ] Linked PR or gate:

<!-- contract:evidence -->
## Evidence

- [ ] Finding summary:
- [ ] Evidence artifact id or hash:
- [ ] Redaction status:
- [ ] Public/private separation:
- [ ] Follow-up owner:

<!-- contract:verification -->
## Verification

- [ ] Guard command:
- [ ] Guard result:
- [ ] Manual inspection target:
- [ ] Manual inspection result:
- [ ] Regression guard:

<!-- contract:privacy_boundary -->
## Privacy Boundary

- [ ] No real resumes or raw personal data.
- [ ] No raw query text.
- [ ] No raw trace content.
- [ ] No candidate results.
- [ ] No local filesystem paths.
- [ ] No tokens, credentials, diagnostic packages, OCR text, model caches, or installer secrets.
- [ ] Use only symbolic env/source names such as `$RESUME_IR_QUERY_ARTIFACT_ROOT`.

<!-- contract:rollback_plan -->
## Rollback Plan

- [ ] Revert path:
- [ ] Public artifact cleanup:
- [ ] Notification or follow-up issue:

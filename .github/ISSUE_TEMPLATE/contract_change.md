---
name: Contract change
about: Track one authorized contract, schema, gate, or template change.
title: "[Contract] "
labels: contract
assignees: ""
---

<!-- contract:scope -->
## Scope

- [ ] Contract surface:
- [ ] One execution owner:
- [ ] Authorized change:
- [ ] Affected gates:
- [ ] Why isolated:
- [ ] Compatibility or versioning plan:

<!-- contract:evidence -->
## Evidence

- [ ] Current contract reference:
- [ ] Proposed contract reference:
- [ ] Fixture or template impact:
- [ ] Public evidence artifact id or hash:
- [ ] Reviewer-visible diff summary:

<!-- contract:verification -->
## Verification

- [ ] Contract check command:
- [ ] Contract check result:
- [ ] Public boundary command:
- [ ] Public boundary result:
- [ ] Downstream call sites checked:

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
- [ ] Contract version rollback:
- [ ] Gate rollback impact:

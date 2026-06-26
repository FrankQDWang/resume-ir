---
name: Benchmark infrastructure
about: Track benchmark runner, fixture, schema, or CI infrastructure work.
title: "[Benchmark Infra] "
labels: benchmark, infrastructure
assignees: ""
---

<!-- contract:scope -->
## Scope

- [ ] Infrastructure surface:
- [ ] Benchmark lane affected:
- [ ] One execution owner:
- [ ] Contract/schema affected:
- [ ] Runner version or fixture set:

<!-- contract:evidence -->
## Evidence

- [ ] Evidence artifact id or hash:
- [ ] `query_set_sha256` if query evidence is involved:
- [ ] Public fixture is synthetic or redacted:
- [ ] Private witness stays local:
- [ ] CI/runtime impact:

<!-- contract:verification -->
## Verification

- [ ] Focused command:
- [ ] Focused result:
- [ ] Contract command:
- [ ] Contract result:
- [ ] Public boundary command:
- [ ] Public boundary result:

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
- [ ] CI gate rollback impact:
- [ ] Benchmark artifact cleanup plan:

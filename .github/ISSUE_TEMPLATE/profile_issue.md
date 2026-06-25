---
name: Profile issue
about: Track one performance profile lane with baseline, hypothesis, evidence, and closure.
title: "[Profile] "
labels: performance, profile
assignees: ""
---

<!-- contract:profile_lane -->
## Profile Lane

- [ ] Primary lane: <!-- first_searchable | full_import_ocr_backlog | query_hot_path | agent_query_replay | repeat_amplification_control -->
- [ ] One execution owner:
- [ ] Dataset/corpus profile:
- [ ] `query_set_sha256`:
- [ ] `git_base_sha`:
- [ ] `git_head_sha`:
- [ ] Base drift: <!-- none | detected | reconciled -->
- [ ] Reconciliation status:
- [ ] Linked PRs:

<!-- contract:optimization_taxonomy -->
## Optimization Taxonomy

- [ ] `optimization_layer`: <!-- L1 | L2 | L3 | L4 -->
- [ ] `affected_layers`: <!-- optional: [] | [L1] | [L2] | [L3] | [L4] -->
- [ ] Bottleneck statement:
- [ ] Expected delta:
- [ ] Rollback condition:
- [ ] Negative controls:
- [ ] Acceptance gate:
- [ ] Workload manifest:
- [ ] Query set source:
- [ ] Corpus scale:
- [ ] Hardware class:
- [ ] Warm/cold definition:
- [ ] Cache state:
- [ ] Platform lane: <!-- macos_m4_discovery | windows_weak_host_validation | cross_os_ci_smoke -->
- [ ] Lower-layer closure rule checked:

<!-- contract:baseline -->
## Baseline

- [ ] Baseline command:
- [ ] Baseline artifact id:
- [ ] Benchmark report hash:
- [ ] Profiler evidence:
- [ ] Target metric:
- [ ] Current value:
- [ ] Success threshold:
- [ ] Regression guard:

<!-- contract:hypothesis -->
## Hypothesis

- [ ] Single measurable hypothesis:
- [ ] Expected bottleneck:
- [ ] Expected improvement:
- [ ] Verification command:
- [ ] Stop condition if hypothesis is false:

<!-- contract:evidence -->
## Evidence

- [ ] Benchmark command:
- [ ] Benchmark artifact id or benchmark report hash:
- [ ] Profiler evidence:
- [ ] Baseline-to-current comparison:
- [ ] Regression guard result:
- [ ] Public evidence is bounded and redacted:
- [ ] Private witness stays local:

<!-- contract:privacy_boundary -->
## Privacy Boundary

- [ ] No raw private data.
- [ ] No raw query text.
- [ ] No raw trace content.
- [ ] No local filesystem paths.
- [ ] No tokens, credentials, diagnostic packages, OCR text, model caches, or installer secrets.
- [ ] Use only symbolic env/source names such as `$RESUME_IR_QUERY_ARTIFACT_ROOT`.
- [ ] Commit only redacted aggregate summaries or schemas.

<!-- contract:closure -->
## Closure

- [ ] Outcome: <!-- won | lost | inconclusive | blocked -->
- [ ] Closing evidence:
- [ ] Linked PRs merged or follow-up issue linked:
- [ ] Success threshold met:
- [ ] Regression guard passed:
- [ ] Remaining risk:

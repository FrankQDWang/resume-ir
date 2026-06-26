<!-- contract:scope -->
## Scope

- [ ] Primary lane: <!-- first_searchable | full_import_ocr_backlog | query_hot_path | agent_query_replay | repeat_amplification_control | docs_contract | gui_manual_loop | benchmark_infra | contract_change | privacy_boundary -->
- [ ] This PR has one execution owner.
- [ ] This PR is limited to the linked issue and does not mix benchmark lanes.
- [ ] Production code changed: <!-- yes/no -->
- [ ] Contract surface changed: <!-- yes/no; list contract below if yes -->
- [ ] Public artifact only contains redacted aggregate evidence.

Contract surfaces touched:

- <!-- e.g. CLI args, diagnostics JSON, release-readiness JSON, workflow gate, GitHub template, none -->

<!-- contract:gui_visual_reference -->
## GUI Visual Contract

- [ ] GUI PR declares `visual_reference_version` or marks this section not applicable.
- [ ] Default stack remains `Tauri + React + Vite + Tailwind + TypeScript`.
- [ ] `UI-reference/` is treated as visual baseline, not functional clone.
- [ ] No production Next.js server is introduced.
- [ ] Visual token inventory and representative screenshots are linked when GUI visuals change.
- [ ] Toolkit bakeoff is not reopened without a linked blocker issue.

<!-- contract:linked_issue -->
## Linked Issue

- Closes: <!-- #issue -->
- Issue type: <!-- profile_issue | gui_manual_loop | benchmark_infra | contract_change | privacy_boundary | bug | security/privacy -->
- Issue contains required machine anchors: <!-- yes/no -->

<!-- contract:hypothesis_baseline -->
## Hypothesis / Baseline

- Hypothesis:
  - [ ] A single measurable claim is stated in the linked issue.
- Baseline:
  - [ ] Baseline command is recorded in the linked issue.
  - [ ] `git_base_sha`:
  - [ ] `git_head_sha`:
  - [ ] `query_set_sha256`:
  - [ ] Baseline drift checked:
  - [ ] Reconciliation status:

<!-- contract:changes -->
## Changes

- [ ] Implementation changes are listed by area.
- [ ] Contract/schema/docs changes are listed by file.
- [ ] New or changed CI gates are listed.

Change summary:

- <!-- itemize user-visible or contract-visible changes -->

<!-- contract:out_of_scope -->
## Out of Scope

- [ ] No raw private data, raw queries, raw traces, local paths, tokens, diagnostic packages, OCR text, model caches, or private benchmark reports are committed.
- [ ] No unrelated dirty or untracked files are included.
- [ ] No threshold lowering, gate bypass, lane mixing, or direct main push is included.

Explicit non-goals:

- <!-- list anything reviewers might otherwise expect -->

<!-- contract:verification -->
## Verification

- [ ] Focused tests or checks:
  - Command:
  - Result:
- [ ] Broad checks, if applicable:
  - Command:
  - Result:
- [ ] Public boundary check:
  - Command: `./scripts/ci/guard-public-repo.sh`
  - Result:
- [ ] Contract check, if applicable:
  - Command: `python3 scripts/ci/check-performance-contracts.py`
  - Result:

<!-- contract:performance_evidence -->
## Performance Evidence

- [ ] Performance-sensitive change: <!-- yes/no -->
- [ ] Benchmark lane:
- [ ] Dataset/corpus profile:
- [ ] `query_set_sha256`:
- [ ] Benchmark artifact id or report hash:
- [ ] Target metric:
- [ ] Success threshold:
- [ ] Regression guard:
- [ ] Hot path remains read-only:
- [ ] Private witness evidence stays local:

<!-- contract:optimization_taxonomy -->
## Performance Optimization Taxonomy

- [ ] Primary `optimization_layer`: <!-- L1 | L2 | L3 | L4 | n/a -->
- [ ] Optional `affected_layers`:
- [ ] `baseline_artifact`:
- [ ] `profiler_summary`:
- [ ] `stage_histogram`:
- [ ] `bottleneck_statement`:
- [ ] `hypothesis`:
- [ ] `expected_delta`:
- [ ] `rollback_condition`:
- [ ] `negative_controls`:
- [ ] `acceptance_gate`:
- [ ] `workload_manifest`:
- [ ] `query_set_source`:
- [ ] `corpus_scale`:
- [ ] `hardware_class`:
- [ ] `warm_or_cold_definition`:
- [ ] `cache_state`:
- [ ] `platform_lane`: <!-- macos_m4_discovery | windows_weak_host_validation | cross_os_ci_smoke | n/a -->
- [ ] `primary_blocker_layer`:
- [ ] `closing_layer`:
- [ ] `linked_blocker`:
- [ ] `closure_evidence`:
- [ ] `lower_layer_closes_higher_layer_blocker`: false
- [ ] Lower-layer closure check reviewed; this PR does not use a lower-layer optimization to close a higher-layer blocker.
- [ ] Hand-written SIMD/cache/prefetch work is absent, or a Scope Exception issue is linked.

<!-- contract:privacy_boundary -->
## Privacy Boundary

- [ ] No real resumes or raw personal data.
- [ ] No raw query text.
- [ ] No raw trace content.
- [ ] No candidate results.
- [ ] No local filesystem paths.
- [ ] No tokens, credentials, diagnostic packages, model caches, or installer secrets.
- [ ] Only env/source names such as `$RESUME_IR_QUERY_ARTIFACT_ROOT` are used for private evidence locations.
- [ ] Any evidence in this PR is a bounded redacted aggregate.

<!-- contract:rollback_plan -->
## Rollback Plan

- [ ] Rollback command or revert path is documented.
- [ ] Contract rollback impact is documented.
- [ ] Data migration rollback is not required, or the migration rollback is documented.

Rollback notes:

- <!-- how to undo this PR without private data exposure -->

<!-- contract:merge_readiness -->
## Merge Readiness

- [ ] Base branch is current or drift is explicitly reconciled.
- [ ] Required checks are passing.
- [ ] Review threads and requested changes are resolved.
- [ ] Merge method is selected.
- [ ] Admin bypass is not used.
- [ ] Direct main push is not used.
- [ ] The linked issue has closure evidence or the remaining gap is documented.

<!-- contract:scope_exception -->
## Scope Exception

Default: do not auto-merge scope exceptions. A scope exception may be considered
only when `ACTIVE_GOAL.toml` explicitly allows it for the active goal.

Auto-merge is disabled when any of these are true:

- [ ] `ACTIVE_GOAL.toml` does not explicitly allow this scope exception.
- [ ] Admin bypass would be required.
- [ ] Direct main push would be required.
- [ ] Requested changes are unresolved.
- [ ] Raw private data, raw query text, raw trace content, local paths, tokens,
      diagnostic packages, OCR text, model caches, or private benchmark reports
      would enter the public repo.
- [ ] Performance thresholds are lowered.
- [ ] Benchmark lanes are mixed.
- [ ] A required gate is bypassed.

Exception request:

- [ ] No scope exception requested.
- [ ] Scope exception requested and authorized by:
- [ ] Why isolated:

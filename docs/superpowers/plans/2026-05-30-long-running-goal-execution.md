# Long-Running Goal Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a repository-native protocol for unattended long-running Codex Goal execution.

**Architecture:** Keep the existing system design as the product source of truth and add one execution checklist that controls autonomous work. Link this plan to `docs/superpowers/specs/2026-05-30-long-running-goal-execution.md` so the checklist can go through `fw-plan-review` before a long build.

**Tech Stack:** Markdown documentation, existing repository design docs, Git-based progress tracking.

---

## Linked Spec

Spec: `docs/superpowers/specs/2026-05-30-long-running-goal-execution.md`

## File Structure

Create:

- `02_execution_plan_执行方案/10_长时间Goal执行清单.md` - primary Chinese runbook for long-running Goal execution.
- `docs/superpowers/specs/2026-05-30-long-running-goal-execution.md` - spec for this execution protocol.
- `docs/superpowers/plans/2026-05-30-long-running-goal-execution.md` - this implementation plan.

Modify:

- `02_execution_plan_执行方案/00_阅读顺序.md` - add the new checklist to the execution reading order.
- `MANIFEST.md` - list the new docs.

## Task 1: Add Long-Running Goal Checklist

**Files:**

- Create: `02_execution_plan_执行方案/10_长时间Goal执行清单.md`

- [ ] **Step 1: Write the checklist document**

Create `02_execution_plan_执行方案/10_长时间Goal执行清单.md` with these sections:

- Core principles
- Confirmed default boundaries
- Boundaries requiring human confirmation
- Hard prohibitions for unattended execution
- Slice execution overview
- Per-slice acceptance standards from S0 through S13
- Ready-to-copy Goal prompt
- Morning review checklist

- [ ] **Step 2: Verify required sections exist**

Run:

```bash
rg -n "已确认默认边界|必须人工确认的边界|无人值守硬性禁止|Slice 执行总览|Slice 验收标准|长时间 Goal 推荐提示词|早上复盘 checklist" 02_execution_plan_执行方案/10_长时间Goal执行清单.md
```

Expected: one match for every listed heading.

## Task 2: Add Superpowers Spec

**Files:**

- Create: `docs/superpowers/specs/2026-05-30-long-running-goal-execution.md`

- [ ] **Step 1: Write the spec**

Create the spec with these sections:

- Problem
- Goal
- Users
- Confirmed Decisions
- Requirements
- Non-Goals
- Acceptance

- [ ] **Step 2: Verify acceptance links the checklist and plan**

Run:

```bash
rg -n "10_长时间Goal执行清单|2026-05-30-long-running-goal-execution.md|Acceptance" docs/superpowers/specs/2026-05-30-long-running-goal-execution.md
```

Expected: matches for the checklist, linked plan, and acceptance section.

## Task 3: Add Linked Implementation Plan

**Files:**

- Create: `docs/superpowers/plans/2026-05-30-long-running-goal-execution.md`

- [ ] **Step 1: Write the plan header and linked spec**

Ensure this file starts with:

```markdown
# Long-Running Goal Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
```

- [ ] **Step 2: Verify the plan references the spec**

Run:

```bash
rg -n "Linked Spec|docs/superpowers/specs/2026-05-30-long-running-goal-execution.md" docs/superpowers/plans/2026-05-30-long-running-goal-execution.md
```

Expected: both references are present.

## Task 4: Update Reading Order

**Files:**

- Modify: `02_execution_plan_执行方案/00_阅读顺序.md`

- [ ] **Step 1: Add the new checklist to the reading order**

Append this item after `08_依赖许可与参考资料.md`:

```markdown
9. `09_一页版执行清单.md`
10. `10_长时间Goal执行清单.md`
```

- [ ] **Step 2: Verify reading order includes the new file**

Run:

```bash
rg -n "10_长时间Goal执行清单" 02_execution_plan_执行方案/00_阅读顺序.md
```

Expected: one match.

## Task 5: Update Manifest

**Files:**

- Modify: `MANIFEST.md`

- [ ] **Step 1: Add the new files to the manifest**

Add these entries:

```markdown
- `02_execution_plan_执行方案/10_长时间Goal执行清单.md`
- `docs/superpowers/specs/2026-05-30-long-running-goal-execution.md`
- `docs/superpowers/plans/2026-05-30-long-running-goal-execution.md`
```

- [ ] **Step 2: Verify manifest entries**

Run:

```bash
rg -n "10_长时间Goal执行清单|docs/superpowers/specs/2026-05-30-long-running-goal-execution|docs/superpowers/plans/2026-05-30-long-running-goal-execution" MANIFEST.md
```

Expected: three matches.

## Task 6: Self-Review

**Files:**

- Read: `02_execution_plan_执行方案/10_长时间Goal执行清单.md`
- Read: `docs/superpowers/specs/2026-05-30-long-running-goal-execution.md`
- Read: `docs/superpowers/plans/2026-05-30-long-running-goal-execution.md`

- [ ] **Step 1: Check for forbidden placeholders**

Run:

```bash
rg -n "T""BD|TO""DO|待""定|稍后""补|implement ""later|fill ""in" 02_execution_plan_执行方案/10_长时间Goal执行清单.md docs/superpowers/specs/2026-05-30-long-running-goal-execution.md docs/superpowers/plans/2026-05-30-long-running-goal-execution.md
```

Expected: no matches.

- [ ] **Step 2: Check that the future Goal has enough to run unattended**

Run:

```bash
rg -n "git init|PROGRESS.md|cargo test|cargo fmt --check|禁止|必须人工确认|验收" 02_execution_plan_执行方案/10_长时间Goal执行清单.md
```

Expected: matches for Git bootstrap, progress tracking, test commands, prohibitions, human-confirmation boundaries, and acceptance language.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-30-long-running-goal-execution.md`.

Recommended next stage: run `fw-plan-review` on the new long-running Goal protocol before starting the unattended build.

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `fw-ceo-review` | Scope & strategy | 0 | not run | Not required for this documentation-only execution protocol |
| Codex Review | independent plan check | Independent 2nd opinion | 0 | not run | Not requested |
| Eng Review | `fw-plan-review` | Architecture & tests | 1 | CLEAR | 0 blocking issues; 2 non-blocking concerns; 1 workflow-alignment improvement applied |
| Design Review | `fw-plan-design-review` | UI/UX gaps | 0 | not applicable | No UI/UX scope |
| DX Review | `plan-devex-review` | Developer experience gaps | 0 | not run | Not required before unattended build |

- **UNRESOLVED:** 0 blocking decisions.
- **VERDICT:** ENG CLEARED. The long-running Goal protocol is safe to use for S0-S13 execution, with the expectation that actual implementation remains gated by each slice's acceptance commands.
- **CONCERNS:** S0-S9 may exceed a single overnight run; that is acceptable because each slice has independent commit and test gates. Native dependency setup may vary by platform; the protocol correctly treats signing, real data, release, and commercial/license decisions as human-confirmation boundaries. P3/P4 are intentionally skeleton-only unless prior slices pass.
- **APPLIED DURING REVIEW:** The clean-thread handoff prompt now explicitly tells the future Goal to use `fw-build` discipline after `fw-plan-review`, and the morning review command block now notes that Cargo commands only apply once S1 has created `Cargo.toml`.

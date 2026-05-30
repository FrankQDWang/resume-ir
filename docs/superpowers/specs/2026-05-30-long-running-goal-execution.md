# Long-Running Goal Execution Spec

## Problem

The repository contains strong system design and execution documents, but a long-running Codex Goal needs stricter execution boundaries. Without an explicit slice plan, an unattended run may overreach into OCR, models, release signing, real user data, or broad implementation without passing intermediate gates.

## Goal

Create a repository-native execution protocol that lets a future Codex Goal run for 10+ hours, complete as much safe work as possible, and leave reviewable artifacts the next morning.

## Users

1. The project owner, who wants to sleep while the Goal keeps working.
2. The future Goal executor, which needs autonomous defaults and explicit stop conditions.
3. The morning reviewer, who needs commits, commands, progress logs, and checklist status.

## Confirmed Decisions

1. The macro architecture remains the one in `GOAL.md`, `01_system_design_系统设计`, and `02_execution_plan_执行方案`.
2. The default build path is Rust workspace, local daemon, CLI, SQLite metadata, and Tantivy full-text index.
3. The first autonomous target is P0/P1 through an import-to-search smoke loop.
4. If P0/P1 pass and time remains, the Goal may continue into P2/P3/P4 skeleton slices that do not require real data, unreviewed models, release signing, or heavy OCR.
5. Each slice must have its own acceptance command and commit.
6. Real resumes and personally identifiable information are out of scope for unattended execution.

## Requirements

### R1: Autonomous Defaults

The execution protocol must define decisions the Goal can make without user input, including Git initialization, slice order, testing expectations, and blocker handling.

### R2: Hard Boundaries

The protocol must list actions that require human confirmation, including remote pushes, PRs, releases, signing, real data import, destructive deletion, commercial licenses, and product direction changes.

### R3: Slice-Based Execution

The protocol must split implementation into independently reviewable slices. Each slice must define deliverables and acceptance commands.

### R4: Progress Artifacts

The future Goal must update `PROGRESS.md` while executing and commit completed slices separately.

### R5: Morning Review

The protocol must include the commands and checks the user should run after waking up.

### R6: Handoff Prompt

The protocol must include a ready-to-copy Goal prompt for the next clean conversation.

## Non-Goals

1. This spec does not implement the resume search engine.
2. This spec does not approve remote publishing, release, notarization, or signing.
3. This spec does not approve importing real resumes.
4. This spec does not replace `fw-plan-review`; it prepares a plan that can be reviewed next.

## Acceptance

1. `02_execution_plan_执行方案/10_长时间Goal执行清单.md` exists and includes confirmed defaults, human-confirmation boundaries, prohibited actions, slice list, acceptance commands, handoff prompt, and morning review checklist.
2. `02_execution_plan_执行方案/00_阅读顺序.md` includes the long-running Goal checklist in the reading order.
3. `MANIFEST.md` lists the new documents.
4. A linked implementation plan exists at `docs/superpowers/plans/2026-05-30-long-running-goal-execution.md`.

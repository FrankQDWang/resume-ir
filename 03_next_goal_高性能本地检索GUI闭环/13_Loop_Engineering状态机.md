# Loop Engineering 状态机

本状态机用于长程 Codex 目标任务。它的目标是防止目标漂移、证据混用和重复 blocked loop。

无人值守执行阶段的事实优先级：

1. Policy truth lives in `ACTIVE_GOAL.toml`, `perf/acceptance-matrix.toml`, schemas, and the autonomous entrypoint document.
2. Execution truth lives in GitHub PR/issue state, git branch/base sha, benchmark artifact hashes, and only then `perf/current-loop-state.json`.
3. 当前对话上下文只能解释执行意图，不能覆盖 policy truth 或 execution truth。

Autonomous delivery 主路径：

```text
goal_authorized
-> baseline_captured
-> discovery_profile_issue_opened
-> hypothesis_recorded
-> slice_selected
-> branch_active
-> implementation_active
-> verification_active
-> pr_opened
-> base_synced
-> pr_review_ready
-> ci_green
-> local_gate_green
-> privacy_gate_green
-> merge_method_selected
-> pr_merged
-> issue_closed_with_evidence
-> next_issue_or_goal_complete
```

Schema caveat: until Task 4 updates `perf/loop-state.schema.json`, these autonomous names are target delivery stages only. They are not valid `workflow_state` values in `perf/current-loop-state.json`; runners must keep writing the current schema fields and use GitHub issue/PR state, git branch/base sha, and benchmark artifact hashes as execution truth.

Terminology map:

| Concept | Current machine field | Current allowed values | Autonomous target meaning |
|---|---|---|---|
| review-gated workflow state | `workflow_state` | `intake`, `ceo_reviewed`, `plan_ready`, `plan_reviewed`, `slice_active`, `red_check_written`, `implementation_active`, `verification_active`, `evidence_review`, `slice_complete`, `blocked`, `goal_complete` | current loop report state machine |
| performance experiment state | `experiment_state` | `not_started`, `contract_locked`, `baseline_validated`, `profile_captured`, `bottleneck_selected`, `hypothesis_registered`, `optimization_slice_active`, `correctness_passed`, `perf_measured`, `reprofiled`, `cross_os_passed`, `accepted`, `reverted`, `blocked`, `complete` | profiling and hypothesis lifecycle |
| evidence lane | `evidence_lane` | `smoke`, `w0_docs`, `w1_private`, `soak_fault`, `gui_manual` | evidence cell class; display aliases may be W0, W1, soak/fault, GUI/manual |
| benchmark lane | not a current schema field until Task 4 | `first_searchable`, `full_import_ocr_backlog`, `query_hot_path`, `agent_query_replay`, `repeat_amplification_control` | workload and measurement category |
| autonomous delivery stage | no current JSON field | n/a | target issue-led slice-train stage, not a competing state machine |

## 1. Workflow State

| State | 进入条件 | 允许转移 | 必需证据 | 禁止事项 |
|---|---|---|---|---|
| `intake` | 用户提出目标或 reviewer 反馈 | `ceo_reviewed` | 原始需求、范围限制、隐私边界 | 直接开始代码实现 |
| `ceo_reviewed` | 完成方向、范围、风险判断 | `plan_ready` | CEO review 结论、推荐路线、用户确认 | 未确认路线就写执行计划 |
| `plan_ready` | spec 和 linked plan 已保存 | `plan_reviewed` | `docs/superpowers/specs/*` 与 `docs/superpowers/plans/*` | 跳过 plan review |
| `plan_reviewed` | 工程计划审查通过 | `slice_active` | review 结论、批准范围 | 扩大到未批准代码范围 |
| `slice_active` | 单个切片被选中 | `red_check_written` 或 `implementation_active` | 切片目标、验收命令、允许文件 | 同时执行多个互相影响的切片 |
| `red_check_written` | 行为切片已有失败验证 | `implementation_active` | 失败输出、测试名或检查名 | 用无关失败作为 red evidence |
| `implementation_active` | 正在修改批准范围内文件 | `verification_active` | diff、实现说明 | 修改未批准文件 |
| `verification_active` | 正在运行验收 | `evidence_review` 或 `blocked` | 命令、退出码、摘要 | 只看部分输出就宣布完成 |
| `evidence_review` | 验证输出已收集 | `slice_complete`, `goal_complete`, `blocked`, 或 `slice_active` | 证据分类、风险说明 | 把 smoke 当 W1 benchmark |
| `slice_complete` | 当前切片所有验收通过 | `slice_active` 或 `goal_complete` | 切片 diff、命令、证据 lane、隐私检查 | 把单切片完成说成整个目标完成 |
| `blocked` | 同一阻塞条件连续出现至少 3 次且无新证据路径 | `intake` 或 `ceo_reviewed` | 阻塞条件、连续次数、下一步所需外部输入 | 因任务困难、预算紧或验证慢而提前标 blocked |
| `goal_complete` | W0、W1、soak/fault、GUI/manual evidence cells 和五个 benchmark lanes 均通过且无开放 blocker | none | 完整验收矩阵、benchmark lane coverage、review closure、隐私检查 | 留下未说明的失败检查 |

## 2. Active Goal Record

每次长程执行都必须能回答：

```text
active_goal_id: resume-ir.performance-gui-loop.2026-06
spec_path: docs/superpowers/specs/2026-06-22-performance-goal-doc-contract.md
plan_path: docs/superpowers/plans/2026-06-22-performance-goal-doc-contract.md
goal_docs_root: 03_next_goal_高性能本地检索GUI闭环
allowed_paths_for_current_pr: GOAL.md, MANIFEST.md, ACTIVE_GOAL.toml, .github/workflows/pr.yml, 03_next_goal_高性能本地检索GUI闭环, docs/superpowers, perf, scripts/ci/check-performance-contracts.py
privacy_boundary: no raw resume text, raw query, candidate result, path, token, trace, diagnostics package, or model cache in git
```

当前 PR 的目标锁以 `ACTIVE_GOAL.toml` 为准。当前允许公开 CI 合同校验脚本和 workflow gate，但不允许生产 Rust、GUI、daemon、benchmark runner 或私有数据执行实现。未来生产实现必须通过新的 linked plan 扩展允许路径，不能复用当前 contract-only 允许范围。

## 3. Performance Experiment State

Workflow state 控制长程任务不漂移；experiment state 控制性能工作不跳过观测和对照。

| Experiment state | 进入条件 | 允许转移 | 必需证据 |
|---|---|---|---|
| `not_started` | 尚未选择性能切片 | `contract_locked` | active goal 和 acceptance matrix |
| `contract_locked` | P0 contract 通过 W0 | `baseline_validated` | query semantics、IPC、matrix、loop schema |
| `baseline_validated` | baseline 已运行 | `profile_captured` 或 `blocked` | resident daemon baseline、histogram、resource aggregate |
| `profile_captured` | profiler summary 已记录 | `bottleneck_selected` | flamegraph/sample summary、stage hotspot |
| `bottleneck_selected` | 单个 bottleneck 被选择 | `hypothesis_registered` | slice scope、expected red/green check |
| `hypothesis_registered` | 已记录可证伪假设 | `optimization_slice_active` | hypothesis、expected metric delta、rollback trigger |
| `optimization_slice_active` | 正在优化单一 hotspot | `correctness_passed` 或 `blocked` | diff、focused tests、stage metrics |
| `correctness_passed` | 语义、隐私和回归检查完成 | `perf_measured` 或 `reverted` | metamorphic checks、privacy flags、focused tests |
| `perf_measured` | 优化后性能已测 | `reprofiled`、`reverted` 或 `optimization_slice_active` | P95/P99、stage latency、resource aggregate |
| `reprofiled` | profiler 已确认瓶颈变化 | `accepted`、`reverted` 或 `bottleneck_selected` | before/after profiler refs、overhead <= 3% |
| `accepted` | 当前 scale cell redlines 通过 | `baseline_validated`、`cross_os_passed` 或 `complete` | accepted_cells、contract pins、redacted report |
| `reverted` | 假设失败或副作用超限 | `bottleneck_selected` 或 `blocked` | revert evidence、failed hypothesis |
| `cross_os_passed` | macOS/Windows 必需切片通过 | `complete` 或 `baseline_validated` | platform aggregate |
| `blocked` | 同一 blocker 连续 3 次且无新证据路径 | `contract_locked` 或 `bottleneck_selected` | blocked report |
| `complete` | 所有 evidence lanes 通过 | none | final redacted aggregate |

性能实现不得从 `contract_locked` 直接跳到 `optimization_slice_active`。没有 baseline、profiler evidence 和可证伪 hypothesis，只能做合同修复或观测面实现，不能声明优化成功。任何 `goal_complete` 状态必须在 `perf/loop-state.schema.json` 中包含 W0、D10K、D100K、D1M、soak/fault、GUI/manual accepted cells。完整 autonomous completion 还必须有五个 benchmark lanes 的 redacted evidence：`first_searchable`、`full_import_ocr_backlog`、`query_hot_path`、`agent_query_replay`、`repeat_amplification_control`。

## 4. Drift Checks

每个 `slice_active` 进入 `implementation_active` 前必须检查：

1. 当前 diff 是否只包含该切片允许路径。
2. 当前验收命令是否对应该切片。
3. 当前 `evidence_lane` 是否为 `smoke`、`w0_docs`、`w1_private`、`soak_fault` 或 `gui_manual` 中的一个。
4. 当前 `benchmark_lane` 是否为 `first_searchable`、`full_import_ocr_backlog`、`query_hot_path`、`agent_query_replay` 或 `repeat_amplification_control` 中的一个；Task 4 前该值只能出现在 issue/report evidence 中，不能写成 `perf/current-loop-state.json` 的机器字段。
5. 当前 query 语义是否仍遵守 simple text AND 合同。
6. 当前 daemon contract 是否仍通过版本化 IPC/diagnostics 暴露。
7. 当前 Loop state report 是否能通过 `perf/loop-state.schema.json`。
8. 当前实验报告是否能通过 `perf/experiment-report.schema.json`，除非该切片不是实验切片。
9. `perf/current-loop-state.json` 是否仍反映当前 PR 的 evidence lane、允许路径和 claim 状态。

## 5. Blocked Stop Rule

当同一阻塞条件在同一目标上下文中连续出现至少 3 次，并且没有新的输入、代码变化、环境变化或新证据路径可以改变结果时，状态必须进入 `blocked`。进入 `blocked` 后，报告必须包含：

1. 阻塞命令或证据。
2. 阻塞条件。
3. 连续出现次数。
4. 已尝试路径。
5. 继续前需要的人类输入或外部状态变化。

若用户输入、代码 diff、环境状态或证据路径发生变化，blocked 连续计数重置。不得因为任务困难、预算紧、验证慢、实现范围大或结果暂时不确定而进入 `blocked`。

Hard retry contract:

1. 每一次同条件 effective retry 都必须记录新的 `evidence_path`，且该路径必须指向新的命令输出、证据源、环境变化、代码变化或配置变化证据。
2. 没有新的 `evidence_path` 时，不得重复执行同一 retry；runner 必须直接进入 `blocked`，或在 `base_drift` 情况下先执行 reconciliation action，或回到 contract review 修正合同。
3. 只有同一 blocker 已经过 3 次各自带 distinct `evidence_path` 的 effective retry 后仍复现时，才可进入 `blocked`。

`base_drift` 是 reconciliation action，不消耗普通 retry。runner 先同步或 rebase 最新 `main` 并重跑 affected gates；只有相同失败仍复现时才开始计入 retry。

## 6. Completion Rule

只有当目标文档、验收矩阵、隐私边界、query 语义、IPC contract 和 reviewer ledger 均有对应证据时，docs-hardening 切片才可进入 `slice_complete`。

完整 performance + GUI 目标只有在 W0、D10K、D100K、D1M、soak/fault、GUI/manual 均有 redacted evidence，且 `first_searchable`、`full_import_ocr_backlog`、`query_hot_path`、`agent_query_replay`、`repeat_amplification_control` 五个 benchmark lanes 均有 redacted evidence，且 review ledger 无 `open` blocker 后，才可进入 `goal_complete`。

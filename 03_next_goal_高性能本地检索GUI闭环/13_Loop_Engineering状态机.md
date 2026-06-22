# Loop Engineering 状态机

本状态机用于长程 Codex 目标任务。它的目标是防止目标漂移、证据混用和重复 blocked loop。

## 1. State

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
| `evidence_review` | 验证输出已收集 | `complete`, `blocked`, 或 `slice_active` | 证据分类、风险说明 | 把 smoke 当 W1 benchmark |
| `blocked` | 同一阻塞条件连续出现至少 3 次且无新证据路径 | `intake` 或 `ceo_reviewed` | 阻塞条件、连续次数、下一步所需外部输入 | 因任务困难、预算紧或验证慢而提前标 blocked |
| `complete` | 该切片或目标全部验收通过 | none | 通过命令、diff scope、隐私检查 | 留下未说明的失败检查 |

## 2. Active Goal Record

每次长程执行都必须能回答：

```text
active_goal_id: resume-ir.performance-gui-loop.2026-06
spec_path: docs/superpowers/specs/2026-06-22-performance-goal-doc-contract.md
plan_path: docs/superpowers/plans/2026-06-22-performance-goal-doc-contract.md
goal_docs_root: 03_next_goal_高性能本地检索GUI闭环
allowed_paths: GOAL.md, MANIFEST.md, 03_next_goal_高性能本地检索GUI闭环, docs/superpowers
privacy_boundary: no raw resume text, raw query, candidate result, path, token, trace, diagnostics package, or model cache in git
```

## 3. Drift Checks

每个 `slice_active` 进入 `implementation_active` 前必须检查：

1. 当前 diff 是否只包含该切片允许路径。
2. 当前验收命令是否对应该切片。
3. 当前 benchmark lane 是否为 smoke、W0、W1、soak/fault 或 GUI/manual 中的一个。
4. 当前 query 语义是否仍遵守 simple text AND 合同。
5. 当前 daemon contract 是否仍通过版本化 IPC/diagnostics 暴露。

## 4. Blocked Stop Rule

当同一阻塞条件在同一目标上下文中连续出现至少 3 次，并且没有新的输入、代码变化、环境变化或新证据路径可以改变结果时，状态必须进入 `blocked`。进入 `blocked` 后，报告必须包含：

1. 阻塞命令或证据。
2. 阻塞条件。
3. 连续出现次数。
4. 已尝试路径。
5. 继续前需要的人类输入或外部状态变化。

若用户输入、代码 diff、环境状态或证据路径发生变化，blocked 连续计数重置。不得因为任务困难、预算紧、验证慢、实现范围大或结果暂时不确定而进入 `blocked`。

## 5. Completion Rule

只有当目标文档、验收矩阵、隐私边界、query 语义、IPC contract 和 reviewer ledger 均有对应证据时，docs-hardening 切片才可进入 `complete`。

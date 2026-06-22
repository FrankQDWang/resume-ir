# Performance Goal Documentation Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden PR #10's documentation package into a reliable execution contract for the upcoming high-performance local search, GUI, and manual/Codex closed-loop phase.

**Architecture:** Keep this as a docs-only contract repair. Root `GOAL.md` states the active goal authority, `03_next_goal_高性能本地检索GUI闭环/` owns the detailed execution contract, and `docs/superpowers/` preserves this spec and implementation plan as the workflow artifacts that must be reviewed before any build work.

**Tech Stack:** Markdown documentation, existing repository shell scripts, existing public-repo guard, git diff checks.

---

## Linked Spec

Spec: `docs/superpowers/specs/2026-06-22-performance-goal-doc-contract.md`

## Scope Guard

This plan is documentation-only. The implementation worker must not modify:

- `crates/`
- `scripts/`
- `Cargo.toml`
- `Cargo.lock`
- test source files
- real resumes
- raw query files
- local artifacts
- diagnostics packages
- model caches

Allowed paths:

- `GOAL.md`
- `MANIFEST.md`
- `03_next_goal_高性能本地检索GUI闭环/`
- `docs/superpowers/specs/`
- `docs/superpowers/plans/`

Before each commit, run:

```bash
{
  git diff --name-only origin/main...HEAD
  git diff --name-only --cached
  git diff --name-only
  git ls-files --others --exclude-standard
} | sort -u
```

Expected: every changed, staged, committed, or untracked path is in the allowed list above.

## File Structure

Create:

- `03_next_goal_高性能本地检索GUI闭环/12_Review问题映射与修复责任.md`
  Tracks every verified reviewer issue and assigns it to a documentation owner and acceptance condition.

- `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md`
  Defines long-running Codex state transitions, evidence gates, drift checks, and blocked-stop rules.

- `03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md`
  Separates smoke, W0, W1 private benchmark, soak/fault, and GUI/manual evidence lanes.

Modify:

- `GOAL.md`
  Clarifies that the previous current-stage closure is distinct from the next active performance + GUI + closed-loop goal.

- `MANIFEST.md`
  Lists the new goal-contract documents and this `docs/superpowers` spec/plan pair.

- `03_next_goal_高性能本地检索GUI闭环/00_阅读顺序.md`
  Adds the new contract documents to the reading order.

- `03_next_goal_高性能本地检索GUI闭环/01_目标边界与成功标准.md`
  Adds active goal authority, non-negotiable redlines, and completion criteria.

- `03_next_goal_高性能本地检索GUI闭环/02_系统架构与模块边界.md`
  Adds true-incremental, resident daemon, encryption mode, and platform FFI boundary notes.

- `03_next_goal_高性能本地检索GUI闭环/03_数据模型与存储协议.md`
  Adds active goal schema, acceptance matrix pointer, query semantics pointer, and visible epoch invariants.

- `03_next_goal_高性能本地检索GUI闭环/04_数据流与状态机.md`
  Links product dataflow states to the Loop Engineering state machine.

- `03_next_goal_高性能本地检索GUI闭环/05_Query_Benchmark与真实Query种子.md`
  Freezes query semantics, anti-overfit rules, private query lanes, and metamorphic checks.

- `03_next_goal_高性能本地检索GUI闭环/06_Daemon_IPC与Diagnostics契约.md`
  Adds request deadlines, cancellation, search batch, overload, backpressure, fairness, and redacted diagnostics.

- `03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md`
  Adds GUI dependency boundary, information architecture, interaction states, journey, responsive/a11y, design tokens, and toolkit bakeoff criteria.

- `03_next_goal_高性能本地检索GUI闭环/08_失败模式与恢复策略.md`
  Adds contract-level failure modes for loop drift, overload, benchmark contamination, and platform journal gaps.

- `03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md`
  Sharpens raw query, artifact, resume, diagnostics, and committed evidence rules.

- `03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md`
  Reorders implementation around P0 contract-first work before performance code.

- `03_next_goal_高性能本地检索GUI闭环/11_一页版目标图.md`
  Adds the one-page contract view: active goal, loop state, IPC boundary, query semantics, and evidence lanes.

## Task 1: Root Authority and Reading Order

**Files:**
- Modify: `GOAL.md`
- Modify: `MANIFEST.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/00_阅读顺序.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/01_目标边界与成功标准.md`

- [ ] **Step 1: Confirm the current authority conflict**

Run:

```bash
nl -ba GOAL.md | sed -n '13,45p'
```

Expected: the output shows that current-stage performance and UI are moved to follow-up goals, which conflicts with PR #10 being the active next-goal package.

- [ ] **Step 2: Append active next-goal authority without deleting current-stage history**

Keep the existing `## 当前阶段边界` section intact. It is historical/current-stage source-of-truth for runtime, packaging, signing, embedding, and blocked-evidence boundaries. Append this new section after the existing current-stage boundary:

```markdown
## 当前活跃后续目标

当前活跃后续目标是 `03_next_goal_高性能本地检索GUI闭环/`：在本地隐私边界内完成高性能检索、GUI、手工/Codex 结对闭环验证的执行合同和后续实现。

该目标的硬边界：

1. 查询热路径只允许检索、过滤、融合、bulk hydrate 和 snippet 返回；不得触发 OCR、全文解析或重模型推理。
2. 简单空格 query 的业务语义在性能优化前冻结，不得为了降低延迟改变召回语义。
3. daemon IPC/diagnostics contract 必须先版本化，GUI 只能依赖版本化 contract。
4. benchmark 证据必须区分 smoke、W0、W1、本机私有、soak/fault 和 GUI/manual，不得混用。
5. 真实简历、raw query、候选结果、路径、token、trace、diagnostics package 和模型缓存不得提交。
```

Do not remove the existing current-stage bullets unless a later review explicitly approves an archival rewrite.

- [ ] **Step 3: Update next-goal target boundary**

Append this section to `03_next_goal_高性能本地检索GUI闭环/01_目标边界与成功标准.md` after `## 4. 硬约束`:

```markdown
## 5. Active Contract Authority

本目录是当前活跃后续目标的执行合同。执行顺序必须先完成 P0 contract，再进入性能、GUI 或平台实现。

P0 contract 包含：

1. `12_Review问题映射与修复责任.md`
2. `13_Loop_Engineering状态机.md`
3. `14_W0_W1验收矩阵与证据协议.md`
4. `05_Query_Benchmark与真实Query种子.md` 中的查询语义冻结
5. `06_Daemon_IPC与Diagnostics契约.md` 中的 versioned IPC/diagnostics contract

若 `GOAL.md`、本文件、review ledger、Loop state、W0/W1 矩阵之间出现冲突，后续实现必须停止并回到 `fw-ceo-review` 或 `fw-plan`，不得自行选择更容易通过的目标。
```

- [ ] **Step 4: Update `MANIFEST.md`**

Add these entries after the current next-goal files:

```markdown
- `03_next_goal_高性能本地检索GUI闭环/12_Review问题映射与修复责任.md`
- `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md`
- `03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md`
- `docs/superpowers/specs/2026-06-22-performance-goal-doc-contract.md`
- `docs/superpowers/plans/2026-06-22-performance-goal-doc-contract.md`
```

- [ ] **Step 5: Update the next-goal reading order**

Add the new documents to `03_next_goal_高性能本地检索GUI闭环/00_阅读顺序.md` after the existing `11_一页版目标图.md` entry:

```markdown
12. `12_Review问题映射与修复责任.md`
13. `13_Loop_Engineering状态机.md`
14. `14_W0_W1验收矩阵与证据协议.md`
```

Then add this paragraph:

```markdown
执行任何后续实现前，先读 `12_Review问题映射与修复责任.md`、`13_Loop_Engineering状态机.md` 和 `14_W0_W1验收矩阵与证据协议.md`。这三份文档是防止目标漂移、benchmark 过拟合和证据混用的合同层。
```

- [ ] **Step 6: Verify docs-only scope**

Run:

```bash
{
  git diff --name-only origin/main...HEAD
  git diff --name-only --cached
  git diff --name-only
  git ls-files --others --exclude-standard
} | sort -u
```

Expected: changed, staged, committed, and untracked paths are limited to `GOAL.md`, `MANIFEST.md`, `03_next_goal_高性能本地检索GUI闭环/`, and `docs/superpowers/`.

- [ ] **Step 7: Commit**

Run:

```bash
git add GOAL.md MANIFEST.md '03_next_goal_高性能本地检索GUI闭环/00_阅读顺序.md' '03_next_goal_高性能本地检索GUI闭环/01_目标边界与成功标准.md'
git commit -m "docs: clarify active performance goal authority"
```

Expected: commit succeeds.

## Task 2: Reviewer Issue Ledger

**Files:**
- Create: `03_next_goal_高性能本地检索GUI闭环/12_Review问题映射与修复责任.md`

- [ ] **Step 1: Create the reviewer ledger document**

Create `03_next_goal_高性能本地检索GUI闭环/12_Review问题映射与修复责任.md` with this content:

```markdown
# Review 问题映射与修复责任

本文件把外部 review 的问题转成目标文档合同。它不记录 raw query、真实简历、候选结果、trace、token 或本机路径。

| ID | Reviewer claim | 核验结论 | 证据 | 文档责任 | 验收条件 |
|---|---|---|---|---|---|
| R01 | Root goal authority conflict | 真实 | `GOAL.md` 同时保留上一阶段边界和后续目标迁移语句 | `GOAL.md`, `01_目标边界与成功标准.md` | active next-goal authority 明确，旧阶段与新目标不冲突 |
| R02 | Query whitespace defaults to OR | 真实 | Tantivy 0.26 默认 `conjunction_by_default=false`; 当前代码使用 lenient parser | `05_Query_Benchmark与真实Query种子.md` | simple text query 语义冻结为 AND，OR 只能显式选择 |
| R03 | Incremental index is not true incremental | 真实 | 当前增量路径读取 active stored docs 并重新 publish snapshot | `02_系统架构与模块边界.md`, `03_数据模型与存储协议.md`, `10_实施切片与验收门槛.md` | P0 后的实现顺序先定义 manifest/micro-epoch/dirty subtree，再改索引热路径 |
| R04 | Snapshot publish loads whole Tantivy dir into memory | 真实 | 当前 archive builder 返回整体 `Vec<u8>` | `02_系统架构与模块边界.md`, `08_失败模式与恢复策略.md` | 文档要求 streaming snapshot 或 mmap-mode ADR，禁止百万级整包内存 publish |
| R05 | First searchable is too late | 真实 | 当前 import 在 crawl 和 pending docs 后 publish visibility | `04_数据流与状态机.md`, `10_实施切片与验收门槛.md` | Level1/Level2 visible epoch 不等待 OCR/vector |
| R06 | Private query benchmark spawns per query | 真实 | 当前 benchmark 写 query file 并为每条 query spawn command | `05_Query_Benchmark与真实Query种子.md`, `14_W0_W1验收矩阵与证据协议.md` | resident daemon benchmark lane 是 W1 gate，process-spawn lane 只能作为 smoke |
| R07 | IPC lacks deadline/cancel/batch/backpressure/fairness/overload | 真实 | search IPC args 仅有 query/mode/top_k/filters | `06_Daemon_IPC与Diagnostics契约.md` | contract 增加 deadline、cancel、batch、client_class、overload、retry_after_ms |
| R08 | Low-end adaptive governor is not algorithmic | 真实 | 目标 docs 只有 budget state，没有算法输入、阈值和恢复规则 | `04_数据流与状态机.md`, `08_失败模式与恢复策略.md` | 文档定义 CPU/RSS/page fault/battery/thermal/disk signals 和动作 |
| R09 | Business query semantics not frozen | 真实 | 当前 docs 没有 simple query AND/OR/phrase/filter 合同 | `05_Query_Benchmark与真实Query种子.md` | 语义冻结与 metamorphic checks 写入目标文档 |
| R10 | Platform journal strategy incomplete | 部分真实 | next-goal docs 提到 watcher first、FSEvents/USN gap，但未定义平台方案和 fallback | `02_系统架构与模块边界.md`, `04_数据流与状态机.md` | macOS FSEvents、Windows USN、fallback reconciliation 的 contract 明确 |
| R11 | Stable identity can drift | 真实 | 当前 crawler identity 包含 path/size/mtime/fingerprint | `03_数据模型与存储协议.md` | stable_file_id、content_fingerprint、path_alias、rename handling 分层 |
| R12 | Tantivy stores large body/snippet fields | 真实 | current schema stores `clean_text`, `section_text`, `all_sections` | `02_系统架构与模块边界.md`, `03_数据模型与存储协议.md` | docs 要求 sidecar body/snippet 和 hot-path fast fields |
| R13 | Large OR doc-id filter risk | 真实 | current allowed doc ids filter builds Should terms | `02_系统架构与模块边界.md`, `06_Daemon_IPC与Diagnostics契约.md` | docs 要求 bitmap/fast-field prefilter strategy |
| R14 | Encryption ADR missing | 真实 | current docs 没有 mmap performance mode vs strict encrypted app mode 决策 | `02_系统架构与模块边界.md`, `09_安全隐私与本地证据边界.md` | ADR section names the supported modes and tradeoff gates |
| R15 | Resident daemon harness/profiling stack missing | 真实 | current docs do not define harness, traces, histograms, profiler captures | `14_W0_W1验收矩阵与证据协议.md` | W1 includes resident daemon, histograms, resource samples, profiler references |
| R16 | GUI toolkit bakeoff missing | 真实 | docs mention GUI but no egui/Slint comparison gate | `07_GUI与手工Codex闭环.md`, `14_W0_W1验收矩阵与证据协议.md` | representative page bakeoff criteria documented before toolkit freeze |
| R17 | Acceptance matrix not machine-tight | 真实 | current gates list commands but not W0/W1/evidence classes | `14_W0_W1验收矩阵与证据协议.md`, `10_实施切片与验收门槛.md` | W0/W1/soak/fault/GUI evidence lanes separated |
| R18 | Loop engineering state machine missing | 真实 | no goal-loop state document exists | `13_Loop_Engineering状态机.md` | state machine defines states, transitions, evidence and drift stops |
| R19 | Anti-overfit policy incomplete | 真实 | privacy rules exist but do not define holdout/metamorphic/drift policies | `05_Query_Benchmark与真实Query种子.md`, `09_安全隐私与本地证据边界.md` | anti-overfit and committed-evidence rules explicit |
| R20 | Implementation order needs P0 reset | 真实 | current order starts docs, query extractor, benchmark before semantic contract is fully frozen | `10_实施切片与验收门槛.md` | P0 = goal authority, query semantics, IPC contract, acceptance matrix, loop state |
| R21 | `unsafe_code = "forbid"` may conflict with native journals | 真实 as future risk | workspace forbids unsafe; native journal FFI may require isolated carve-out | `02_系统架构与模块边界.md`, `10_实施切片与验收门槛.md` | docs require tiny isolated platform crate, safe public API, and explicit review before any carve-out |
```

- [ ] **Step 2: Check the ledger has every issue id**

Run:

```bash
rg -n "^\\| R[0-9][0-9] \\|" '03_next_goal_高性能本地检索GUI闭环/12_Review问题映射与修复责任.md'
```

Expected: 21 table rows are printed, from `R01` through `R21`.

- [ ] **Step 3: Commit**

Run:

```bash
git add '03_next_goal_高性能本地检索GUI闭环/12_Review问题映射与修复责任.md'
git commit -m "docs: map review findings to goal contract"
```

Expected: commit succeeds.

## Task 3: Loop Engineering State Machine

**Files:**
- Create: `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/04_数据流与状态机.md`

- [ ] **Step 1: Create the loop state machine document**

Create `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md` with this content:

````markdown
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
````

- [ ] **Step 2: Link product state to loop state**

Append this section to `03_next_goal_高性能本地检索GUI闭环/04_数据流与状态机.md`:

```markdown
## 6. Loop Engineering 绑定

产品状态机只描述导入、索引、查询和后台预算状态。长程 Codex 执行状态由 `13_Loop_Engineering状态机.md` 约束。

任何后续切片在进入实现前必须声明：

1. 当前 Loop state。
2. active goal id。
3. 本切片允许文件。
4. 本切片验收命令。
5. 本切片证据 lane。

若上述信息缺失，禁止开始实现。
```

- [ ] **Step 3: Verify links**

Run:

```bash
rg -n "13_Loop_Engineering状态机|Loop Engineering" '03_next_goal_高性能本地检索GUI闭环/04_数据流与状态机.md' '03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md'
```

Expected: output includes references in both files.

- [ ] **Step 4: Commit**

Run:

```bash
git add '03_next_goal_高性能本地检索GUI闭环/04_数据流与状态机.md' '03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md'
git commit -m "docs: define loop engineering state machine"
```

Expected: commit succeeds.

## Task 4: Query Semantics and Benchmark Anti-Overfit Contract

**Files:**
- Modify: `03_next_goal_高性能本地检索GUI闭环/05_Query_Benchmark与真实Query种子.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md`

- [ ] **Step 1: Add frozen query semantics**

Insert this section after `## 4. Buckets` in `05_Query_Benchmark与真实Query种子.md`:

```markdown
## 5. 查询语义冻结

性能优化前冻结以下业务语义：

1. simple text query 使用空格分隔词项时，默认语义是 required-all，即非停用词全部必须参与匹配。
2. OR 只能由显式布尔语法、显式 mode 或 GUI 明确选项触发。
3.  quoted phrase 是短语约束，不等同于普通 token 拆分。
4. 字段过滤是 hard filter，必须先于 ranking、fusion、rerank 和 snippet 执行。
5. 空 query、超长 query、互斥 query、极冷词 query 必须有有界响应和可解释 partial/zero-result 状态。
6. benchmark 调优不得改变 simple text、phrase、field filter、explicit OR 的语义。

文档层验收：

| Check | 期望 |
|---|---|
| term reorder | simple terms 重排后 result set 在 ranking tolerance 内稳定 |
| add required term | 加 required term 后候选集合不得变大 |
| explicit OR | 只有显式 OR 可以扩大 simple term 匹配 |
| field filter | 增加 hard filter 后候选集合不得变大 |
| smoke vs W1 | smoke 结果不得声称完整 500-query baseline |
```

- [ ] **Step 2: Renumber following sections**

After inserting the new section, renumber the former benchmark output and acceptance sections so the document headings remain strictly increasing.

Expected heading order:

```text
## 1. 来源边界
## 2. 本地私有输入
## 3. Query Set Schema
## 4. Buckets
## 5. 查询语义冻结
## 6. 生成策略
## 7. Benchmark 输出
## 8. 验收红线
```

- [ ] **Step 3: Add anti-overfit evidence rules**

Append this section to `09_安全隐私与本地证据边界.md`:

```markdown
## 7. Anti-overfit 证据规则

1. local private query-set 可以留在本机，但 committed evidence 只能包含 hash、count、bucket、latency percentile 和 redacted aggregate。
2. 任何 raw query、候选结果、简历正文、姓名、联系方式、路径、token、trace 或截图进入 git，当前目标失败。
3. benchmark 至少区分 smoke、W0、W1、soak/fault、GUI/manual。不同 lane 的结果不得互相替代。
4. 查询语义是固定合同，不允许为了降低 latency 把 simple text AND 改成 OR、降低 hard filter 强度或跳过 partial explanation。
5. 性能报告必须记录 query set hash、sample count、bucket count、dataset count、searchable count、machine profile 和 percentile confidence。
```

- [ ] **Step 4: Verify the query contract text**

Run:

```bash
rg -n "查询语义冻结|required-all|explicit OR|Anti-overfit|query set hash" '03_next_goal_高性能本地检索GUI闭环/05_Query_Benchmark与真实Query种子.md' '03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md'
```

Expected: output shows the inserted semantics and evidence rules.

- [ ] **Step 5: Commit**

Run:

```bash
git add '03_next_goal_高性能本地检索GUI闭环/05_Query_Benchmark与真实Query种子.md' '03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md'
git commit -m "docs: freeze query semantics and benchmark evidence"
```

Expected: commit succeeds.

## Task 5: Daemon IPC and Diagnostics Contract

**Files:**
- Modify: `03_next_goal_高性能本地检索GUI闭环/06_Daemon_IPC与Diagnostics契约.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md`

- [ ] **Step 1: Add request envelope fields**

Replace the current IPC envelope example with this version:

```json
{
  "schema_version": "resume-ir.ipc-request.v2",
  "request_id": "uuid",
  "client_class": "interactive_gui|codex_validation|benchmark|background",
  "deadline_ms": 200,
  "idempotency_key": "optional-stable-key",
  "cancel_token": "optional-cancel-token",
  "batch_id": "optional-batch-id",
  "payload": {}
}
```

Then add this paragraph:

```markdown
`client_class` controls fairness and overload behavior. `deadline_ms` is a contract, not a hint: daemon may return partial or overload instead of blocking beyond the deadline. `cancel_token` must be accepted by queued and active long-running work. `batch_id` groups GUI or benchmark requests without exposing raw query text in diagnostics.
```

- [ ] **Step 2: Add search batch and overload response contracts**

Insert this section after the search response contract:

````markdown
## 5. Search Batch and Overload Contract

Search batch request:

```json
{
  "schema_version": "resume-ir.search-batch-request.v1",
  "request_id": "uuid",
  "client_class": "benchmark",
  "deadline_ms": 30000,
  "batch_id": "bench-local-redacted-001",
  "queries": [
    {
      "query_id": "qhash_001",
      "query_shape": "hybrid",
      "query_text_ref": "local-only"
    }
  ],
  "top_k": 20
}
```

Overload response:

```json
{
  "schema_version": "resume-ir.ipc-response.v1",
  "request_id": "uuid",
  "status": "error",
  "error": {
    "code": "OVERLOADED",
    "retry_after_ms": 250,
    "degraded_mode": "interactive_only",
    "reason": "background_budget_exhausted"
  },
  "warnings": []
}
```

Fairness order:

1. `interactive_gui`
2. `codex_validation`
3. `benchmark`
4. `background`

The daemon may degrade OCR/vector/import/compaction before degrading interactive keyword and field-filter search.
````

- [ ] **Step 3: Renumber diagnostics and GUI sections**

After adding the new section, renumber later headings so the document remains ordered:

```text
## 5. Search Batch and Overload Contract
## 6. Diagnostics Contract
## 7. GUI 依赖面
```

- [ ] **Step 4: Update GUI dependency boundary**

Append this paragraph to `07_GUI与手工Codex闭环.md`:

```markdown
GUI 只能依赖 `06_Daemon_IPC与Diagnostics契约.md` 中的 versioned IPC/diagnostics contract。GUI 不读取 SQLite table、Tantivy field、sidecar path、ANN implementation、raw benchmark file 或 local artifact path。GUI 需要展示 overload、partial、cancelled、degraded、repairing 和 benchmark lane，而不是把这些状态折叠成通用错误。
```

- [ ] **Step 5: Add GUI information architecture contract**

Append this section to `07_GUI与手工Codex闭环.md`:

```markdown
## 5. GUI Information Architecture

GUI 是 app UI，不是 landing page。第一屏必须按同一信息层级设计，后续 egui/eframe 与 Slint bakeoff 不得各自发明不同页面结构。

第一屏扫描顺序：

1. 可搜索状态、root health 和 daemon 状态。
2. 查询输入、mode segmented control 和 filter 摘要。
3. 结果列表、partial/degraded/overload 解释。
4. 选中结果 detail/snippet/hydrate 状态。
5. diagnostics、evidence lane 和 redacted export 状态。

参考布局：

| 区域 | 职责 | 禁止 |
|---|---|---|
| top command bar | root selector/status、search input、mode、filter summary、pause/cancel | 把主要搜索动作藏进菜单 |
| left rail | import queue、Level1/2/3 counts、OCR/semantic queue、budget/degraded state | 用装饰 card 堆叠代替状态分组 |
| center workspace | 100 条结果列表、stable row height、partial markers、selection state | 让 hover、loading 或长文本改变列表布局 |
| right detail panel | selected result detail、snippet、hydrate status、explanation | 默认暴露 raw local path |
| bottom status strip | benchmark lane、latency、diagnostics/export、redaction flags | 把 W0/W1/smoke 证据混成一个状态 |
```

- [ ] **Step 6: Add GUI interaction state matrix**

Append this section to `07_GUI与手工Codex闭环.md`:

```markdown
## 6. GUI Interaction State Matrix

GUI 不得把所有非成功状态折叠成通用 error。每个区域必须有可见状态、可复核状态码和用户下一步动作。

| Feature | Loading | Empty | Error | Success | Partial / Degraded / Overload / Cancelled |
|---|---|---|---|---|---|
| root/import | root 正在扫描，显示 Level1/2/3 目标和队列入口 | 未选择 root，主动作是选择 root | root 不可读或 watcher unavailable，显示 retry 和 fallback 状态 | Level 计数增长，搜索可用性明确 | partial download、journal gap、budget degraded 必须显示原因和恢复动作 |
| search | deadline 内显示 pending，不阻塞已有结果 | zero-result 显示 query shape、filter 摘要和清除 filter 动作 | parse error 或 daemon error 显示 request_id 和 redacted reason | 结果列表、latency、参与 layer 明确 | partial/degraded/overload/cancelled 显示 layer、retry_after_ms 或 cancel 状态 |
| results | 结果列表保留 stable row height 和 skeleton | 结果为空时显示可执行下一步，不显示 raw query | hydrate/snippet 单项失败不清空整页 | 选中、排序、snippet 状态稳定 | 单项 partial marker 不改变列表布局 |
| detail | detail hydrate pending 显示占位 | 未选中时显示选择提示 | detail unavailable 显示 redacted reason | snippet、field、explanation 可见 | sidecar checksum 或 hydrate timeout 显示 partial detail |
| diagnostics/export | export pending 显示 redaction checklist | 无可导出证据时显示 lane 原因 | export blocked 显示 prohibited material class | 只导出 redacted aggregate | smoke/W0/W1/GUI/manual lane 必须清晰区分 |
```

- [ ] **Step 7: Add GUI user journey storyboard**

Append this section to `07_GUI与手工Codex闭环.md`:

```markdown
## 7. GUI User Journey Storyboard

GUI bakeoff 必须证明用户能完成从首次导入到 redacted evidence export 的完整闭环，而不是只展示静态 dashboard。

| Step | User does | First 5 seconds | First 5 minutes | Long-term trust evidence |
|---|---|---|---|---|
| first run | 选择 resume root | 立即知道 root 是否可读、是否开始扫描 | Level1/2/3 计数开始变化，等待原因可见 | root health、watcher/fallback 状态可复核 |
| first searchable | 等 Level2 可搜索 | 清楚知道现在能搜什么、还缺什么 | OCR/semantic 继续后台增强，不阻塞 keyword/field search | visible epoch 和 partial reason 可追踪 |
| query | 输入 keyword/filter/hybrid query | search input、mode、filter 摘要最显眼 | 结果、latency、参与 layer、partial/degraded 状态可解释 | simple text semantics 和 benchmark lane 没被 GUI 隐藏 |
| inspect | 选择结果看详情 | detail panel 不暴露 raw path | snippet、hydrate、sidecar 状态清楚 | detail 错误不污染整页结果 |
| manual validation | 人工确认 workflow | checklist 对应 import/search/detail/export | 可记录 pass/fail、partial 和 blocked 原因 | Codex 可用 redacted aggregate 复核 |
| evidence export | 导出 diagnostics | redaction checklist 明确 | 只产生 lane-aware aggregate，不含 raw query/resume/path/token | exported evidence 能支撑 W0/W1/GUI/manual 判定 |
```

- [ ] **Step 8: Add responsive and accessibility contract**

Append this section to `07_GUI与手工Codex闭环.md`:

```markdown
## 8. Responsive and Accessibility Contract

GUI bakeoff 必须覆盖 desktop、tablet 和 narrow viewport。响应式不是简单堆叠，而是保留搜索、结果、状态和证据 lane 的可操作性。

| Viewport | Layout contract | Must remain visible |
|---|---|---|
| desktop >= 1200px | top command bar + left rail + center workspace + right detail panel + bottom status strip | root health、search input、100-row result list、detail、diagnostics lane |
| tablet 768-1199px | left rail 可折叠，detail panel 可作为 side sheet，center workspace 保持主焦点 | search input、result list、partial/degraded marker、selected detail entry |
| narrow < 768px | single-column task flow：root/status、search、results、detail、diagnostics 以 tabs 或 segmented navigation 切换 | search action、result count、current lane、overload/partial reason、export status |

Accessibility requirements:

1. Keyboard order follows top command bar -> left rail -> center workspace -> right detail panel -> bottom status strip.
2. Focus visible state must be obvious without relying only on color.
3. Body text contrast ratio must be at least 4.5:1.
4. Interactive targets must be at least 44px on touch viewports.
5. Search status, overload, cancelled, degraded, and export completion must have screen-reader-visible status text.
6. Placeholder text cannot be the only label for search, filter, root selector, or export controls.
```

- [ ] **Step 9: Add GUI design system boundary**

Append this section to `07_GUI与手工Codex闭环.md`:

```markdown
## 9. GUI Design System Boundary

当前目标不创建完整 `DESIGN.md`，但 P8 GUI bakeoff 必须使用同一组临时 design tokens，避免 egui/eframe 与 Slint 因视觉风格不同而不可比较。正式 `DESIGN.md` 可以在 GUI 实现前通过单独计划建立。

Temporary tokens:

| Token | Contract |
|---|---|
| type scale | 12px metadata、14px dense table/list、16px body/control、20px section heading |
| spacing density | 4px inner gap、8px control gap、12px group gap、16px panel gap |
| row height | result row stable 44-56px，expanded row 不改变相邻 row height |
| status colors | success、partial、degraded、overload、error、blocked 必须语义区分，不只靠颜色 |
| focus style | keyboard focus 使用 outline 或 ring，不能只用 hover |
| panel surfaces | app UI 使用平静工作台面，不使用 marketing hero、装饰 card grid、渐变背景或图标装饰 |
```

- [ ] **Step 10: Add GUI toolkit bakeoff contract**

Append this section to `07_GUI与手工Codex闭环.md`:

```markdown
## 10. GUI Toolkit Bakeoff

技术栈冻结前必须用同一个 representative page 比较 egui/eframe 与 Slint。该 bakeoff 不实现完整 GUI，只验证能否承载目标工作台。两个代表页面必须遵守 `## 5. GUI Information Architecture`、`## 6. GUI Interaction State Matrix`、`## 7. GUI User Journey Storyboard`、`## 8. Responsive and Accessibility Contract` 和 `## 9. GUI Design System Boundary`，否则比较无效。

代表页面必须包含：

1. root 选择和 root 状态。
2. Level1/2/3 计数。
3. OCR/semantic/import 队列状态。
4. 搜索框、mode segmented control、filter 摘要。
5. 100 条结果列表，包含 partial/degraded/overload 状态。
6. 选中结果 detail panel。
7. diagnostics/export 入口。

评分项：

| 项 | 要求 |
|---|---|
| daemon contract fit | 只依赖 versioned IPC/diagnostics |
| information architecture fit | 同一 top command bar、left rail、center workspace、right detail panel、bottom status strip |
| state coverage | loading、empty、error、success、partial/degraded/overload/cancelled 均有可见表现 |
| journey completeness | first run、first searchable、query、inspect、manual validation、evidence export 可闭环 |
| dense data UI | 100 条结果和队列状态不卡顿、不跳布局 |
| cross-platform packaging | macOS/Windows 打包路径清晰 |
| responsive/a11y | desktop/tablet/narrow viewport、keyboard order、focus visible、contrast >= 4.5:1、44px touch target、screen-reader status 可验收 |
| design system fit | 同一 type scale、spacing density、row height、status colors、focus style、panel surfaces |
| manual/Codex verification | 能用脚本或稳定状态输出复核 |
| performance headroom | 低配机器下不持续占满 CPU/GPU |
| maintenance | 依赖数量、license、构建复杂度可接受 |

冻结条件：两个代表页面的截图、状态矩阵截图、journey checklist、responsive/a11y checklist、temporary token checklist、资源摘要、交互 checklist、打包 notes 和 tradeoff 结论进入 redacted docs 后，才允许选择 GUI toolkit。
```

- [ ] **Step 11: Verify IPC and GUI contract additions**

Run:

```bash
rg -n "deadline_ms|client_class|cancel_token|search-batch|OVERLOADED|retry_after_ms|interactive_gui|GUI 只能依赖|GUI Information Architecture|top command bar|left rail|center workspace|right detail panel|bottom status strip|GUI Interaction State Matrix|Loading|Empty|Error|Success|Partial|GUI User Journey Storyboard|first searchable|manual validation|evidence export|Responsive and Accessibility Contract|desktop|tablet|narrow|keyboard order|focus visible|contrast >= 4.5:1|44px|screen-reader|GUI Design System Boundary|type scale|spacing density|row height|status colors|focus style|panel surfaces|GUI Toolkit Bakeoff|egui|Slint|representative page|information architecture fit|state coverage|journey completeness|responsive/a11y|design system fit" '03_next_goal_高性能本地检索GUI闭环/06_Daemon_IPC与Diagnostics契约.md' '03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md'
```

Expected: output shows every contract field, GUI dependency boundary, information architecture area, interaction state matrix, user journey storyboard, responsive/a11y contract, design system boundary, and toolkit bakeoff criteria.

- [ ] **Step 12: Commit**

Run:

```bash
git add '03_next_goal_高性能本地检索GUI闭环/06_Daemon_IPC与Diagnostics契约.md' '03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md'
git commit -m "docs: freeze daemon ipc and gui contract"
```

Expected: commit succeeds.

## Task 6: Architecture Contract Gaps

**Files:**
- Modify: `03_next_goal_高性能本地检索GUI闭环/02_系统架构与模块边界.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/03_数据模型与存储协议.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/08_失败模式与恢复策略.md`

- [ ] **Step 1: Add architecture redlines**

Append this section to `02_系统架构与模块边界.md`:

```markdown
## 7. Review-driven 架构红线

1. Incremental index 必须基于 manifest diff、dirty subtree、micro-epoch 和 derived-state repair，不得把“读取旧 stored docs 后整包 publish”称为 true incremental。
2. Snapshot publish 必须有 streaming 或 mmap-compatible 路径；百万级目标下禁止把整个 Tantivy snapshot archive 读入单个内存 buffer。
3. First searchable 必须允许 Level1/Level2 先可见，OCR、semantic、rerank 属于后台增强。
4. Tantivy hot path 不保存大正文作为 required STORED 字段；正文和长 snippet 通过 sidecar/body cache 按 topN 读取。
5. Field prefilter 使用 bitmap、fast field 或等价结构，避免为大 doc-id 集合构建巨型 OR / giant OR filter。
6. Encryption 必须拆分为 performance mmap mode 与 strict app encrypted mode 的 ADR；默认选择必须说明安全、性能、恢复和平台限制。
7. Native platform journal 如需 FFI，必须隔离为 tiny platform crate，公开 safe API，并通过 plan review 批准 workspace `unsafe_code` 策略变更。
```

- [ ] **Step 2: Add active goal data contract**

Append this section to `03_数据模型与存储协议.md`:

````markdown
## 7. Active Goal Contract Record

目标文档必须能映射到如下记录，供后续实现生成机器可读合同：

```toml
goal_id = "resume-ir.performance-gui-loop.2026-06"
goal_docs_root = "03_next_goal_高性能本地检索GUI闭环"
spec_path = "docs/superpowers/specs/2026-06-22-performance-goal-doc-contract.md"
plan_path = "docs/superpowers/plans/2026-06-22-performance-goal-doc-contract.md"
query_semantics = "simple_text_required_all_v1"
ipc_contract = "resume-ir.ipc.v2"
evidence_matrix = "w0_w1_redacted_local_v1"
privacy_boundary = "no_raw_resume_or_query_in_git"
```

该记录的真实机器可读落地必须经过 plan review；本目标文档先冻结字段含义。
````

- [ ] **Step 3: Add platform journal failure rows**

Append these rows to the filesystem failure table in `08_失败模式与恢复策略.md`:

```markdown
| macOS FSEvents gap | root 标记 dirty subtree，触发 bounded reconciliation | journal gap test + dirty subtree evidence |
| Windows USN gap | volume 标记 dirty subtree，触发 bounded reconciliation | journal gap test + dirty subtree evidence |
| watcher unavailable | fallback periodic manifest diff | degraded status + user-visible warning |
```

- [ ] **Step 4: Add overload and loop-drift failure rows**

Append this section to `08_失败模式与恢复策略.md`:

```markdown
## 6. Contract Failure Modes

| 失败 | 恢复 | 验收 |
|---|---|---|
| interactive search overload | 返回 `OVERLOADED` 或 partial，不阻塞超过 deadline | IPC overload test |
| benchmark lane contamination | 标记 run invalid，不产生完整 baseline 声明 | benchmark summary rejects mixed lane |
| goal drift detected | 停止当前切片，回到 `ceo_reviewed` 或 `plan_ready` | Loop state evidence |
| repeated blocked evidence | 进入 `blocked`，报告外部输入需求 | blocked-stop report |
| raw private data in evidence | 当前目标失败，移除证据并重新生成 redacted aggregate | public repo guard + manual review |
```

- [ ] **Step 5: Verify architecture contract additions**

Run:

```bash
rg -n "true incremental|streaming|mmap|Level1/Level2|sidecar|巨型 OR|giant OR filter|Active Goal Contract|FSEvents gap|USN gap|goal drift" '03_next_goal_高性能本地检索GUI闭环/02_系统架构与模块边界.md' '03_next_goal_高性能本地检索GUI闭环/03_数据模型与存储协议.md' '03_next_goal_高性能本地检索GUI闭环/08_失败模式与恢复策略.md'
```

Expected: output shows architecture redlines including `巨型 OR / giant OR filter`, active goal fields, and failure rows.

- [ ] **Step 6: Commit**

Run:

```bash
git add '03_next_goal_高性能本地检索GUI闭环/02_系统架构与模块边界.md' '03_next_goal_高性能本地检索GUI闭环/03_数据模型与存储协议.md' '03_next_goal_高性能本地检索GUI闭环/08_失败模式与恢复策略.md'
git commit -m "docs: add performance architecture contract redlines"
```

Expected: commit succeeds.

## Task 7: W0/W1 Acceptance Matrix

**Files:**
- Create: `03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/11_一页版目标图.md`

- [ ] **Step 1: Create the acceptance matrix document**

Create `03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md` with this content:

````markdown
# W0/W1 验收矩阵与证据协议

## 1. Evidence Lanes

| Lane | 数据 | 可提交内容 | 用途 | 不可替代 |
|---|---|---|---|---|
| smoke | synthetic/public tiny set | pass/fail, counts, redacted summary | 快速防回归 | 不代表性能基线 |
| W0 | public/synthetic + docs checks | commands, diff scope, guard output | PR 可审查证据 | 不代表私有真实语料 |
| W1 | local private resumes + local private query set | hash, counts, percentiles, resource aggregates | 真实本机性能证据 | 不提交 raw data |
| soak/fault | local controlled fault run | redacted aggregate, failure class, recovery status | 长跑和恢复证据 | 不代表 query quality |
| GUI/manual | local GUI + daemon IPC | screen-state checklist, redacted diagnostics flags | 人机闭环体验 | 不代表 index scalability |

## 2. W0 Docs-hardening Gate

当前 docs-hardening PR 只需要 W0：

```bash
git diff --check -- GOAL.md MANIFEST.md docs/superpowers 03_next_goal_高性能本地检索GUI闭环
git diff --check --cached -- GOAL.md MANIFEST.md docs/superpowers 03_next_goal_高性能本地检索GUI闭环
while IFS= read -r path; do
  tmp="$(mktemp)"
  git diff --check --no-index /dev/null "$path" >"$tmp" 2>&1
  rc=$?
  if [ -s "$tmp" ]; then
    cat "$tmp"
    rm "$tmp"
    exit 1
  fi
  rm "$tmp"
  if [ "$rc" -ne 0 ] && [ "$rc" -ne 1 ]; then
    exit "$rc"
  fi
done < <(git ls-files --others --exclude-standard '*.md')
./scripts/ci/guard-public-repo.sh
{
  git diff --name-only origin/main...HEAD
  git diff --name-only --cached
  git diff --name-only
  git ls-files --others --exclude-standard
} | sort -u
```

Expected:

1. diff check passes.
2. public repo guard passes.
3. changed paths stay inside the approved docs-only scope.

## 3. W1 Private Benchmark Gate

后续性能实现完成后，W1 redacted evidence 必须包含：

1. query_set_sha256。
2. query_count。
3. bucket_counts。
4. document_count。
5. searchable_document_count。
6. vector_indexed_document_count。
7. machine_profile。
8. resident_daemon_batch_id。
9. spawn_per_query=false。
10. p50/p95/p99 per bucket。
11. histogram_bins per stage and bucket。
12. stage latency per bucket。
13. rss_peak_mb、cpu、disk read/write aggregate。
14. profiler_capture_refs。
15. trace_summary_redacted=true。
16. hot_path_ocr=false。
17. hot_path_parsing=false。
18. hot_path_heavy_model_inference=false。
19. percentile_confidence。

## 4. Soak and Fault Gate

Long-run evidence must cover:

1. resident daemon search batch。
2. daemon restart during import。
3. cancel active request。
4. overload response。
5. disk-space-low。
6. file-lock。
7. snapshot corrupt。
8. journal gap reconciliation。
9. degraded low-resource mode。

## 5. GUI/manual Gate

GUI acceptance must cover:

1. import submit。
2. status levels。
3. keyword search。
4. field filter search。
5. hybrid search partial/degraded display。
6. detail view without raw internal path dependency。
7. diagnostics export redaction flags。
8. pause/resume/cancel。
9. overload display。
10. benchmark lane display。

## 6. Completion Rule

完整 performance + GUI goal 只有在 W0、W1、soak/fault、GUI/manual 均有对应 redacted evidence 后才能标为 complete。
````

- [ ] **Step 2: Reorder implementation slices**

Replace the slice table in `10_实施切片与验收门槛.md` with this table:

```markdown
| 切片 | 目标 | 主要文件 |
|---|---|---|
| P0 | goal authority、query semantics、IPC contract、acceptance matrix、loop state | `GOAL.md`, `03_next_goal_高性能本地检索GUI闭环/`, `docs/superpowers/` |
| P1 | observation baseline and resident daemon benchmark harness | benchmark runner, daemon IPC tests, diagnostics docs |
| P2 | query semantics implementation and metamorphic checks | search planner, full-text parser, daemon search tests |
| P3 | fulltext hot path: resident reader/writer, fast fields, bitsets, bulk hydrate | fulltext index, meta-store, daemon search |
| P4 | import first-searchable path, manifest diff, dirty subtree, stable identity | fs crawler, import pipeline, meta-store |
| P5 | platform journal strategy and reconciliation | platform-specific watcher crates, safe API boundary |
| P6 | adaptive governor and overload behavior | daemon scheduler, OCR/vector/import budgets |
| P7 | snapshot/encryption ADR implementation path | fulltext snapshot, storage docs, diagnostics |
| P8 | GUI toolkit bakeoff and representative page | GUI app, daemon contract tests, manual checklist |
| P9 | final W1 benchmark, soak/fault, GUI/manual evidence | local validation, redacted reports |
| P10 | PGO/LTO/package-level tuning after semantics are locked | release/profile configuration |
```

- [ ] **Step 3: Update the one-page target graph**

Append these bullets to `11_一页版目标图.md`:

```markdown
6. P0 contract 先于性能实现。
7. Loop Engineering state machine 防止目标漂移。
8. Query semantics 固定后才允许优化。
9. Daemon IPC/diagnostics versioned contract 先于 GUI。
10. W0/W1/soak/fault/GUI evidence lane 分离。
```

- [ ] **Step 4: Verify acceptance matrix**

Run:

```bash
rg -n "W0|W1|smoke|soak/fault|GUI/manual|P0|Loop Engineering|Query semantics|resident_daemon_batch_id|spawn_per_query=false|histogram_bins|profiler_capture_refs|trace_summary_redacted=true" '03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md' '03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md' '03_next_goal_高性能本地检索GUI闭环/11_一页版目标图.md'
```

Expected: output shows all evidence lanes, resident daemon/profiling evidence fields, and P0-first ordering.

- [ ] **Step 5: Commit**

Run:

```bash
git add '03_next_goal_高性能本地检索GUI闭环/10_实施切片与验收门槛.md' '03_next_goal_高性能本地检索GUI闭环/11_一页版目标图.md' '03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md'
git commit -m "docs: define W0 W1 evidence matrix"
```

Expected: commit succeeds.

## Task 8: Final Docs-only Verification

**Files:**
- Verify only.

- [ ] **Step 1: Check changed paths**

Run:

```bash
{
  git diff --name-only origin/main...HEAD
  git diff --name-only --cached
  git diff --name-only
  git ls-files --others --exclude-standard
} | sort -u
```

Expected: output contains only:

```text
GOAL.md
MANIFEST.md
03_next_goal_高性能本地检索GUI闭环/...
docs/superpowers/...
```

- [ ] **Step 2: Run markdown diff whitespace check**

Run:

```bash
git diff --check -- GOAL.md MANIFEST.md docs/superpowers 03_next_goal_高性能本地检索GUI闭环
git diff --check --cached -- GOAL.md MANIFEST.md docs/superpowers 03_next_goal_高性能本地检索GUI闭环
while IFS= read -r path; do
  tmp="$(mktemp)"
  git diff --check --no-index /dev/null "$path" >"$tmp" 2>&1
  rc=$?
  if [ -s "$tmp" ]; then
    cat "$tmp"
    rm "$tmp"
    exit 1
  fi
  rm "$tmp"
  if [ "$rc" -ne 0 ] && [ "$rc" -ne 1 ]; then
    exit "$rc"
  fi
done < <(git ls-files --others --exclude-standard '*.md')
```

Expected: no output and exit code 0 for tracked and staged files; untracked Markdown files may produce `git diff --no-index` exit code 1 when no whitespace errors are present.

- [ ] **Step 3: Run privacy guard**

Run:

```bash
./scripts/ci/guard-public-repo.sh
```

Expected: exit code 0.

- [ ] **Step 4: Confirm reviewer coverage**

Run:

```bash
rg -n "^\\| R[0-9][0-9] \\|" '03_next_goal_高性能本地检索GUI闭环/12_Review问题映射与修复责任.md'
```

Expected: 21 rows, covering `R01` through `R21`.

- [ ] **Step 5: Confirm loop and evidence docs are linked**

Run:

```bash
rg -n "12_Review问题映射与修复责任|13_Loop_Engineering状态机|14_W0_W1验收矩阵与证据协议" MANIFEST.md '03_next_goal_高性能本地检索GUI闭环/00_阅读顺序.md'
```

Expected: all three target docs appear in both files.

- [ ] **Step 6: Report docs-hardening implementation handoff**

Prepare a short Chinese handoff containing:

```markdown
docs-hardening implementation complete.

Changed scope:
- GOAL.md
- MANIFEST.md
- 03_next_goal_高性能本地检索GUI闭环/
- docs/superpowers/

Verification:
- git diff --check: pass
- guard-public-repo: pass
- reviewer issue rows: 21
- docs-only scope: pass

Next workflow stage: fw-review / PR review.
```

Expected: no production performance implementation starts during this docs-hardening task.

## Self-Review Checklist

- Spec coverage: Tasks 1-8 cover every requirement in `docs/superpowers/specs/2026-06-22-performance-goal-doc-contract.md`.
- Scope: all planned edits are documentation files or fw-plan artifacts.
- Reviewer coverage: Task 2 covers R01-R21.
- Loop engineering: Task 3 creates the state machine and links it from the product state doc.
- Query semantics: Task 4 freezes simple text required-all semantics and benchmark evidence rules.
- IPC/GUI: Task 5 freezes request envelope, batch, overload, fairness, GUI dependency boundaries, information architecture, state coverage, journey, responsive/a11y, design tokens, and toolkit bakeoff.
- Architecture gaps: Task 6 records true incremental, streaming snapshot, first-searchable, sidecar, bitmap, encryption ADR, platform FFI policy.
- Acceptance matrix: Task 7 separates W0, W1, soak/fault, and GUI/manual lanes.
- Verification: Task 8 confirms docs-only scope, whitespace, privacy guard, reviewer coverage, and handoff.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-22-performance-goal-doc-contract.md`.

Two execution options after fw-plan-review approves this plan:

1. Subagent-Driven (recommended) - dispatch a fresh subagent per task, review between tasks, fast iteration.
2. Inline Execution - execute tasks in this session using executing-plans, batch execution with checkpoints.

The next workflow stage after approval is `fw-build` for this docs-only hardening plan. It is not approval to start production performance, daemon, indexing, or GUI implementation.

## Plan Review Required Outputs

### NOT in scope

- Production performance implementation: deferred because this plan is the docs-only contract hardening gate.
- Daemon/index/query/GUI code changes: deferred until `fw-build` executes an approved docs-only plan and a later implementation plan authorizes production code.
- Private 10k resume import benchmark and private query benchmark: deferred because current W0 evidence must stay public/synthetic and docs-only.
- Mockup selection and final GUI visual direction: deferred to P8 GUI toolkit bakeoff so egui/eframe and Slint compare against the same contract.
- Full `DESIGN.md`: deferred to a GUI implementation/design-system plan; this review adds temporary tokens only for fair bakeoff comparison.

### What already exists

- Root `GOAL.md` and `MANIFEST.md` already provide project-level orientation; Task 1 updates them instead of adding a competing root authority file.
- `03_next_goal_高性能本地检索GUI闭环/` already contains the system, data model, dataflow, query benchmark, IPC, GUI, failure, privacy, implementation, and one-page docs; this plan hardens that package rather than replacing it.
- `./scripts/ci/guard-public-repo.sh` already enforces public-repo privacy boundaries; Task 8 reuses it as the W0 guard.
- Current Rust test and CI infrastructure already exists for future implementation slices; this docs-hardening plan deliberately does not add new test code.
- Existing daemon IPC/search/indexing code provides live evidence for reviewer findings; this plan records contracts first and does not rebuild those paths.

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `fw-ceo-review` | Scope & strategy | 1 | CLEAR | Direction accepted earlier for docs-only contract hardening before broad performance/GUI work |
| Codex Review | `codex review` | Independent 2nd opinion | 0 | NOT RUN | Not run; external reviewer report was manually verified against repo evidence |
| Eng Review | `fw-plan-review` | Architecture & tests (required) | 1 | CLEAR | 12 issues found and folded into plan: authority, scope guard, task/file mismatch, Loop blocked rule, W1 profiling, verification checks |
| Design Review | `fw-plan-review` | UI/UX gaps | 1 | CLEAR | score: 5/10 -> 9/10, 6 decisions: text-only review, IA, states, journey, responsive/a11y, design tokens |
| DX Review | `plan-devex-review` | Developer experience gaps | 0 | NOT RUN | Not applicable to this docs-hardening plan |

- **VERDICT:** CEO + ENG + DESIGN CLEARED for `fw-build` of this docs-only hardening plan. Production performance, daemon, indexing, query parser, GUI, and private benchmark implementation remain out of scope.
NO UNRESOLVED DECISIONS

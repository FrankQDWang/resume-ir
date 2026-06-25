# GUI Performance Loop Contract Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the approved GUI / performance / loop design spec into target documents, machine-readable contracts, GitHub ledger templates, and public CI guards.

**Architecture:** Keep human-readable policy in the goal documents, machine policy in `ACTIVE_GOAL.toml` and `perf/acceptance-matrix.toml`, state shape in JSON schemas, ledger fields in GitHub templates, and enforcement in focused standard-library Python guards. This is a contract-only update: it must not implement GUI, daemon, benchmark runners, or private-data execution.

**Tech Stack:** Markdown, TOML parsed with Python `tomllib`, JSON Schema draft 2020-12 structure, standard-library Python guard scripts, GitHub issue templates, existing public privacy guard.

---

## Scope Check

This plan implements one cohesive contract slice. It does not start production search optimization, GUI implementation, daemon IPC changes, Tauri scaffolding, benchmark runner work, query extraction implementation, Windows SSH automation, or private benchmark execution.

The approved spec spans GUI, profiling taxonomy, platform lanes, and loop engineering. These are not independent production systems in this slice; they are one policy update because the autonomous runner needs the same fields across docs, TOML, schemas, templates, and guards before production work begins.

## File Structure

Modify:

- `03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md` - replace toolkit bakeoff with the Tauri/React/Vite/Tailwind default lane, `UI-reference/` visual baseline, and fallback-bakeoff rule.
- `03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md` - add the private corpus transfer boundary for macOS and Windows weak-host validation using symbolic env names only.
- `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md` - add GUI visual states, optimization-layer selection, platform-lane selection, and lower-layer closure rules.
- `03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md` - add platform-lane and goal-complete requirements for GUI/manual, D100K weak-host, and private corpus handling.
- `03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md` - add the L0/L1/L2/L3/L4 optimization taxonomy and required profile issue fields.
- `03_next_goal_高性能本地检索GUI闭环/17_机器可读Goal与Experiment协议.md` - document the new machine fields and their relationship to schemas and guards.
- `ACTIVE_GOAL.toml` - add GUI stack, visual reference, optimization layers, platform lanes, and private corpus handling policy.
- `perf/acceptance-matrix.toml` - add optimization-layer, platform-lane, GUI visual, and weak-host acceptance contracts.
- `perf/loop-state.schema.json` - add fields for `optimization_layer`, `affected_layers`, `platform_lane`, and GUI visual state metadata.
- `perf/experiment-report.schema.json` - add L0 workload/profile fields, optimization-layer fields, platform-lane evidence fields, and GUI visual evidence fields.
- `perf/current-loop-state.json` - keep the current state contract-only and record that this slice updates contract policy only.
- `perf/fixtures/valid/*.json` - update valid examples to include the new fields.
- `perf/fixtures/invalid/*.json` - add negative examples for missing optimization layer, invalid platform lane, and lower-layer closure misuse.
- `.github/ISSUE_TEMPLATE/profile_issue.md` - add required L0, optimization taxonomy, platform lane, and rollback/negative-control anchors.
- `.github/ISSUE_TEMPLATE/gui_manual_loop.md` - add visual-reference version, token inventory, visual diff, manual flow, and packaging evidence anchors.
- `.github/PULL_REQUEST_TEMPLATE.md` - add GUI visual reference, optimization layer, affected layers, platform lane, and lower-layer closure checklist.
- `scripts/ci/check-performance-contracts.py` - validate the new matrix/schema fields and fixture coverage.
- `scripts/ci/check-loop-state.py` - validate current loop-state invariants for layer/platform/visual metadata.
- `scripts/ci/check-experiment-report.py` - validate report invariants for profile issue fields and platform lane claims.
- `scripts/ci/check-autonomous-goal.py` - validate active goal GUI stack, visual reference, platform lane, and private corpus transfer policy.
- `scripts/ci/check-benchmark-lanes.py` - reject lower-layer closure claims where a lower optimization layer closes a higher-layer blocker.
- `scripts/ci/check-private-evidence-redaction.py` - ensure private roots and Windows private paths cannot appear in public docs or evidence.

Do not modify:

- `UI-reference/` tracking state. It remains a local visual reference unless a separate plan explicitly adds sanitized reference assets.
- `crates/`.
- GUI scaffold files.
- Benchmark runner scripts.
- Private corpus files.
- Raw SeekTalent artifacts.
- Raw profiler outputs or diagnostics packages.

## Task 1: Update GUI Visual And Stack Contract

**Files:**
- Modify: `03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md`
- Modify: `ACTIVE_GOAL.toml`
- Modify: `perf/acceptance-matrix.toml`
- Modify: `.github/ISSUE_TEMPLATE/gui_manual_loop.md`
- Modify: `.github/PULL_REQUEST_TEMPLATE.md`

- [ ] **Step 1: Replace the GUI toolkit bakeoff section**

In `03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md`, replace section `## 10. GUI Toolkit Bakeoff` with:

```markdown
## 10. GUI 技术栈与 UI-reference 视觉合同

默认 GUI 技术栈冻结为 `Tauri + React + Vite + Tailwind + TypeScript`。

Tauri 负责桌面壳、系统权限、打包、native bridge 和 daemon IPC 边界。React 负责界面状态和组件组合。Vite 负责把前端构建成 Tauri 可内嵌的静态资产。Tailwind 负责承载 `UI-reference/` 已有视觉语言。生产 GUI 不运行 Next.js server；当前 `UI-reference/` 的 Next.js 原型只作为视觉参考，不作为发布架构。

`UI-reference/` 是视觉基准，不是功能逐项复刻要求。功能、页面数量、字段和流程可以按 daemon IPC、diagnostics、benchmark/manual 验证和产品需求调整；视觉语言不得漂移。

必须保留的视觉不变量：

1. GUI 是安静、克制、高密度的本地工作台，不是 landing page。
2. 使用浅色背景、薄边框、紧凑 spacing、低装饰密度和清晰信息层级。
3. 保留 left rail、top command bar、center workspace、detail side sheet/panel、status/diagnostics affordance 的工作台结构。
4. 保留稳定 row/card 尺寸，hover、loading、长文本和 partial marker 不得造成列表布局跳动。
5. 使用 Lucide 风格图标、紧凑按钮、pill、tag、segmented control、redacted diagnostics/export affordance。
6. 主强调色应接近 reference primary，黑/灰承担主要文本和操作层级。
7. 圆角默认接近 8px，除非组件有明确本地理由。

验收目标是 pixel-level visual similarity，不是 identical functional clone。视觉相似性由 design token inventory、reference screenshot inventory、representative page screenshots、manual/Codex review 和 GUI/manual issue 证据共同判断。

Fallback bakeoff rule:

1. `Tauri + React + Vite + Tailwind + TypeScript` 是默认 lane。
2. 只有 GitHub issue 记录明确 blocker 后，才允许重新打开 egui/eframe、Slint 或其他 toolkit bakeoff。
3. 合法 blocker 包括 WebView2/Windows packaging 失败、weak-host runtime footprint 无法接受、100000 logical rows 交互性能不可达、native integration 无法满足 daemon IPC 或 diagnostics 边界。
4. fallback bakeoff 必须复用同一 `UI-reference/` inventory 和 representative pages，不能发明新的视觉风格。
```

- [ ] **Step 2: Add active-goal GUI stack fields**

In `ACTIVE_GOAL.toml`, add:

```toml
[gui]
default_stack = "tauri_react_vite_tailwind_typescript"
desktop_shell = "tauri"
frontend_runtime = "react"
frontend_build = "vite_static_assets"
style_system = "tailwind"
production_next_server_allowed = false
visual_reference = "UI-reference/"
visual_reference_role = "visual_baseline_not_functional_clone"
pixel_level_visual_similarity_required = true
toolkit_bakeoff_default_required = false
toolkit_bakeoff_requires_blocker_issue = true
raw_reference_assets_publication_allowed = false
```

- [ ] **Step 3: Add GUI visual matrix fields**

In `perf/acceptance-matrix.toml`, add:

```toml
[gui_stack]
default_stack = "tauri_react_vite_tailwind_typescript"
production_next_server_allowed = false
visual_reference_role = "visual_baseline_not_functional_clone"
pixel_level_visual_similarity_required = true
toolkit_bakeoff_requires_blocker_issue = true

[gui_visual_redlines]
left_rail_required = true
top_command_bar_required = true
center_workspace_required = true
detail_panel_or_side_sheet_required = true
dense_result_list_required = true
stable_row_or_card_dimensions_required = true
lucide_style_icon_vocabulary_required = true
tailwind_token_inventory_required = true
reference_screenshot_inventory_required = true
functional_clone_required = false
```

- [ ] **Step 4: Update GUI manual issue template**

In `.github/ISSUE_TEMPLATE/gui_manual_loop.md`, add these anchors under the existing evidence section:

```markdown
<!-- contract:gui_visual_reference -->
## GUI Visual Reference

- [ ] Visual reference version or local inventory id:
- [ ] Default stack: `Tauri + React + Vite + Tailwind + TypeScript`
- [ ] Production Next.js server is not used:
- [ ] Token inventory evidence:
- [ ] Reference screenshot inventory:
- [ ] Representative page screenshots:
- [ ] Pixel-level similarity reviewed:
- [ ] Functional divergence is product-required:
- [ ] Toolkit fallback blocker issue, if any:
```

- [ ] **Step 5: Update PR template**

In `.github/PULL_REQUEST_TEMPLATE.md`, add this checklist under GUI/manual or Scope:

```markdown
## GUI Visual Contract

- [ ] GUI PR declares `visual_reference_version` or marks this section not applicable.
- [ ] Default stack remains `Tauri + React + Vite + Tailwind + TypeScript`.
- [ ] `UI-reference/` is treated as visual baseline, not functional clone.
- [ ] No production Next.js server is introduced.
- [ ] Visual token inventory and representative screenshots are linked when GUI visuals change.
- [ ] Toolkit bakeoff is not reopened without a linked blocker issue.
```

- [ ] **Step 6: Verify Task 1**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
rg -n "egui/eframe 与 Slint bakeoff|技术栈冻结前必须" 03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md
rg -n "production_next_server_allowed = false|default_stack = \"tauri_react_vite_tailwind_typescript\"" ACTIVE_GOAL.toml perf/acceptance-matrix.toml
```

Expected:

```text
check-performance-contracts.py passed
```

The second command should return no matches. The third command should find the new stack fields.

- [ ] **Step 7: Commit Task 1**

```bash
git add 03_next_goal_高性能本地检索GUI闭环/07_GUI与手工Codex闭环.md ACTIVE_GOAL.toml perf/acceptance-matrix.toml .github/ISSUE_TEMPLATE/gui_manual_loop.md .github/PULL_REQUEST_TEMPLATE.md
git commit -m "docs: lock gui visual stack contract"
```

## Task 2: Add Performance Optimization Taxonomy Contract

**Files:**
- Modify: `03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md`
- Modify: `.github/ISSUE_TEMPLATE/profile_issue.md`
- Modify: `.github/PULL_REQUEST_TEMPLATE.md`
- Modify: `perf/acceptance-matrix.toml`

- [ ] **Step 1: Add taxonomy section to profiling document**

In `03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md`, add a new section after `## 1. 观测原则`:

```markdown
## 1.1 Performance Optimization Taxonomy

每个 profile issue 和 performance PR 必须声明唯一主层 `optimization_layer`。可选 `affected_layers` 只说明影响，不拥有验收目标。

Required fields:

```text
optimization_layer
affected_layers
baseline_artifact
profiler_summary
stage_histogram
bottleneck_statement
hypothesis
expected_delta
rollback_condition
negative_controls
acceptance_gate
workload_manifest
query_set_source
corpus_scale
hardware_class
warm_or_cold_definition
cache_state
platform_lane
```

### L0 Observation Precondition

L0 不是优化层。没有 baseline、profiler summary、stage histogram、workload manifest、可证伪 hypothesis、expected_delta、rollback_condition 和 negative_controls，不允许进入任何优化实现。

workload representativeness 属于 L0。profile 必须记录 query set source、corpus scale、hardware class、warm/cold definition 和 cache state。

### L1 Architecture-Level Optimization

预期收益：系统不崩、可恢复、10x 规模扩展。

L1 包含 daemon lifecycle、IPC、storage/index topology、BM25/Tantivy schema and parameter choices、ANN index choice、first-searchable、crash recovery、search while importing、OCR/semantic backgrounding 和 product latency contract。

L1 必须报告 first-searchable latency、time to first result、time to full index ready、resume after crash、search while importing 和 incremental searchable lag。

Algorithm / index choice 归入 L1，不新增 L5。

### L2 Parallelism-Level Optimization

预期收益：语料规模 x10、吞吐提升，同时不牺牲交互延迟。

L2 包含 pipeline concurrency、queueing、backpressure、OCR/vector scheduling、content-read concurrency、parser concurrency、writer behavior、batch hydrate/snippet、fairness、cancel、overload 和 admission control。

L2 必须同时报告 open-loop throughput、closed-loop user latency、queue wait histogram、scheduler fairness、cancel latency、peak RSS、IO saturation 和 GUI main-thread blocked time。

### L3 Compile-Level Optimization

预期收益：0-15% runtime improvement，或按真实瓶颈取得 binary/startup/resource improvement。

L3 包含 release profile、LTO、codegen units、dependency feature pruning、build metadata、binary size、startup/cold-path behavior、symbol/debug split 和 reproducible build settings。

L3 必须用同一代码、不同 build config 做 A/B。L3 不能关闭 L1 或 L2 blocker。

### L4 Microarchitecture-Level Optimization

预期收益：0.5-3% 单函数或局部 hotspot 改善。

L4 包含 allocation reduction、clone removal、hot-loop simplification、local data-structure changes 和 symbol/function-level optimization。

L4 必须绑定真实 symbol/function-level hotspot，并说明该函数占所属 stage 的比例。Criterion microbenchmark 只能补充，不能替代真实 profile 证据。

L4 默认不得改变 external behavior、ranking semantics、error semantics、data contract、IPC shape、diagnostics shape 或 persistence format。

### Lower-Layer Closure Rule

低层优化不能关闭高层 blocker：

1. L4 不能关闭 L1 blocker。
2. L3 不能关闭 L2 starvation、fairness 或 queue-pressure blocker。
3. L2 不能关闭 L1 crash recovery、first-searchable、daemon lifecycle、IPC 或 index topology blocker。

### Not Planned By Default

默认不手写 SIMD、branch prediction、cache-line alignment 或 prefetching。这些由 Tantivy、FAISS、ONNX Runtime、Rust 标准库或平台 runtime 处理。

Scope Exception 必须同时满足：

1. profile 证明热点在项目自有代码，不在库内部；
2. 现有库参数、index type 和 build feature 已调优；
3. 有 A/B benchmark 和 correctness oracle；
4. 有 cross-platform fallback；
5. 有 maintenance-cost assessment。
```

- [ ] **Step 2: Add loop-state drift rule text**

In `03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md`, add to `## 4. Drift Checks`:

```markdown
10. 性能 PR 是否声明唯一 `optimization_layer`，且可选 `affected_layers` 没有被当成验收目标。
11. `optimization_layer` 是否遵守 lower-layer closure rule：L4 不关闭 L1 blocker，L3 不关闭 L2 blocker，L2 不关闭 L1 blocker。
12. profile issue 是否包含 expected_delta、rollback_condition、negative_controls、workload_manifest、query_set_source、corpus_scale、hardware_class、warm_or_cold_definition、cache_state 和 platform_lane。
```

- [ ] **Step 3: Add matrix fields**

In `perf/acceptance-matrix.toml`, add:

```toml
[optimization_layers]
allowed = ["L1", "L2", "L3", "L4"]
require_single_primary_layer = true
allow_affected_layers = true
l0_is_precondition_not_layer = true
algorithm_index_choice_layer = "L1"
data_quality_workload_representativeness_layer = "L0"

[optimization_layer_redlines]
missing_baseline_blocks_optimization = true
missing_profile_blocks_optimization = true
missing_hypothesis_blocks_optimization = true
missing_expected_delta_blocks_optimization = true
missing_rollback_condition_blocks_optimization = true
missing_negative_controls_blocks_optimization = true
lower_layer_cannot_close_higher_layer_blocker = true
hand_written_simd_requires_scope_exception = true
```

- [ ] **Step 4: Update profile issue template**

In `.github/ISSUE_TEMPLATE/profile_issue.md`, add under `## Profile Lane`:

```markdown
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
```

- [ ] **Step 5: Update PR template**

In `.github/PULL_REQUEST_TEMPLATE.md`, add:

```markdown
## Performance Optimization Taxonomy

- [ ] Primary `optimization_layer`: <!-- L1 | L2 | L3 | L4 | n/a -->
- [ ] Optional `affected_layers`:
- [ ] L0 evidence linked: baseline, profiler summary, stage histogram, workload manifest.
- [ ] Hypothesis, expected delta, rollback condition, and negative controls are linked.
- [ ] This PR does not use a lower-layer optimization to close a higher-layer blocker.
- [ ] Hand-written SIMD/cache/prefetch work is absent, or a Scope Exception issue is linked.
```

- [ ] **Step 6: Verify Task 2**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
rg -n "optimization_layer|expected_delta|rollback_condition|negative_controls|Lower-Layer Closure Rule" 03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md 03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md .github/ISSUE_TEMPLATE/profile_issue.md .github/PULL_REQUEST_TEMPLATE.md perf/acceptance-matrix.toml
```

Expected: contract check passes and `rg` finds all required anchors.

- [ ] **Step 7: Commit Task 2**

```bash
git add 03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md 03_next_goal_高性能本地检索GUI闭环/13_Loop_Engineering状态机.md .github/ISSUE_TEMPLATE/profile_issue.md .github/PULL_REQUEST_TEMPLATE.md perf/acceptance-matrix.toml
git commit -m "docs: add performance optimization taxonomy"
```

## Task 3: Add Platform Lanes And Private Corpus Boundary

**Files:**
- Modify: `03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md`
- Modify: `03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md`
- Modify: `ACTIVE_GOAL.toml`
- Modify: `perf/acceptance-matrix.toml`
- Modify: `.github/ISSUE_TEMPLATE/profile_issue.md`

- [ ] **Step 1: Add platform-lane section to privacy boundary**

In `09_安全隐私与本地证据边界.md`, add:

```markdown
## macOS / Windows Private Corpus Boundary

私有简历 corpus 由 `$RESUME_IR_PRIVATE_RESUME_ROOT` 表示。公开文档、GitHub issue、PR、redacted report 和 committed evidence 只能使用该符号名，不能写入真实本机路径、Windows 私有路径、文件名、简历正文、OCR 文本、raw trace、raw query 或 diagnostics package。

平台 lane：

1. `macos_m4_discovery`：主要 discovery/profile lane，用于快速定位热点和建立 hypothesis。
2. `windows_weak_host_validation`：低配 Windows weak-host 验证 lane，用于代表实际猎头办公电脑。
3. `cross_os_ci_smoke`：GitHub Actions 或等价 CI smoke，只证明基本跨平台构建和测试，不替代 weak-host 性能证据。

Autonomous runner 可以自行决定私有 corpus 留在 macOS 本机执行，或通过私有网络传输到 Windows weak-host 的私有目录执行必需 gate。复制只允许发生在 private execution 中；公开证据只允许记录 hardware class、OS build class、power mode、runner version、benchmark runner version、redacted resource aggregate、redacted stage histogram、public-safe symbol summary、report hash、approved opaque manifest ref 或 HMAC-SHA256 opaque manifest ref。

Windows 或私有网络不可达时，runner 必须进入 reconciliation，不得跳过 gate 宣布通过。reconciliation 至少记录 SSH reachability、private root permission、runner version、PowerShell/WPR capability 和 changed environment state 的 evidence path。
```

- [ ] **Step 2: Add platform requirements to acceptance document**

In `14_W0_W1验收矩阵与证据协议.md`, add:

```markdown
## Platform Lane Evidence

`macos_m4_discovery` 可用于 hotspot 排序、profile hypothesis 和快速回归，但不能单独关闭 Windows weak-host、cross_os 或 goal_complete gate。

`windows_weak_host_validation` 在以下节点是必需证据：

1. L1 architecture acceptance，若 claim 涉及 lifecycle、filesystem watching、recovery 或 first-searchable。
2. L2 parallelism acceptance。
3. GUI toolkit 或 packaging acceptance。
4. D100K weak-host acceptance。
5. `cross_os_passed`。
6. `goal_complete`。

`cross_os_ci_smoke` 不能替代 `windows_weak_host_validation`。
```

- [ ] **Step 3: Add platform lane tool guidance**

In `15_性能观测与Profiling工具链.md`, add:

```markdown
## Platform Profiling Lanes

`macos_m4_discovery` 使用 Samply、Instruments、`tracing` span、hdrhistogram、release benchmark 和 synthetic pressure。它能指导 hotspot 排序，不能代表 Windows weak-host 结论。

`windows_weak_host_validation` 使用 WPR/WPA/ETW 或等价 Windows performance consumer、PowerShell runner、USN Journal / filesystem watcher verification、WebView2/Tauri packaging smoke 和 resource aggregate。

`cross_os_ci_smoke` 只证明 CI 层面的构建和测试 smoke。
```

- [ ] **Step 4: Add active-goal platform policy**

In `ACTIVE_GOAL.toml`, add:

```toml
[platform_lanes]
primary_discovery = "macos_m4_discovery"
weak_host_validation = "windows_weak_host_validation"
ci_smoke = "cross_os_ci_smoke"
macos_m4_can_rank_hotspots = true
macos_m4_can_close_windows_gate = false
cross_os_ci_smoke_can_replace_weak_host_perf = false

[platform_lanes.private_corpus_transfer]
runner_may_choose_transfer_to_windows = true
transfer_public_evidence_allowed = false
raw_private_paths_public_allowed = false
public_source_name = "$RESUME_IR_PRIVATE_RESUME_ROOT"
windows_unavailable_starts_reconciliation = true
```

- [ ] **Step 5: Add matrix platform policy**

In `perf/acceptance-matrix.toml`, add:

```toml
[platform_lanes]
allowed = ["macos_m4_discovery", "windows_weak_host_validation", "cross_os_ci_smoke"]
primary_discovery = "macos_m4_discovery"
weak_host_validation = "windows_weak_host_validation"
ci_smoke = "cross_os_ci_smoke"
macos_m4_can_close_windows_gate = false
cross_os_ci_smoke_can_replace_weak_host_perf = false

[platform_gate_requirements]
l1_platform_lifecycle_requires_windows_weak_host = true
l2_parallelism_requires_windows_weak_host = true
gui_packaging_requires_windows_weak_host = true
d100k_requires_windows_weak_host = true
cross_os_passed_requires_windows_weak_host = true
goal_complete_requires_windows_weak_host = true
```

- [ ] **Step 6: Verify Task 3**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
rg -n "macos_m4_discovery|windows_weak_host_validation|cross_os_ci_smoke|RESUME_IR_PRIVATE_RESUME_ROOT" 03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md 03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md 03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md ACTIVE_GOAL.toml perf/acceptance-matrix.toml .github/ISSUE_TEMPLATE/profile_issue.md
python3 scripts/ci/check-private-evidence-redaction.py
```

Expected: contract check passes. First `rg` finds symbolic platform fields. Second `rg` returns no real private path matches.

- [ ] **Step 7: Commit Task 3**

```bash
git add 03_next_goal_高性能本地检索GUI闭环/09_安全隐私与本地证据边界.md 03_next_goal_高性能本地检索GUI闭环/14_W0_W1验收矩阵与证据协议.md 03_next_goal_高性能本地检索GUI闭环/15_性能观测与Profiling工具链.md ACTIVE_GOAL.toml perf/acceptance-matrix.toml .github/ISSUE_TEMPLATE/profile_issue.md
git commit -m "docs: add platform profiling lane contract"
```

## Task 4: Extend Schemas And Fixtures

**Files:**
- Modify: `perf/loop-state.schema.json`
- Modify: `perf/experiment-report.schema.json`
- Modify: `perf/current-loop-state.json`
- Modify: `perf/fixtures/valid/agent-query-replay-report.json`
- Modify: `perf/fixtures/valid/autonomous-loop-slice-complete.json`
- Modify: `perf/fixtures/valid/loop-evidence-review.json`
- Modify: `perf/fixtures/valid/w1-d1m-goal-report.json`
- Create: `perf/fixtures/invalid/profile-missing-optimization-layer.json`
- Create: `perf/fixtures/invalid/profile-invalid-platform-lane.json`
- Create: `perf/fixtures/invalid/lower-layer-closes-higher-layer-blocker.json`

- [ ] **Step 1: Add loop-state schema fields**

In `perf/loop-state.schema.json`, add these top-level properties:

```json
"optimization_layer": { "$ref": "#/$defs/optimization_layer" },
"affected_layers": {
  "type": "array",
  "items": { "$ref": "#/$defs/optimization_layer" },
  "uniqueItems": true
},
"platform_lane": { "$ref": "#/$defs/platform_lane" },
"visual_reference": { "$ref": "#/$defs/visual_reference" },
"layer_closure": { "$ref": "#/$defs/layer_closure" }
```

Add these definitions under `$defs`:

```json
"optimization_layer": {
  "enum": ["L1", "L2", "L3", "L4"]
},
"platform_lane": {
  "enum": ["macos_m4_discovery", "windows_weak_host_validation", "cross_os_ci_smoke"]
},
"visual_reference": {
  "type": "object",
  "additionalProperties": false,
  "required": ["reference_role", "default_stack", "production_next_server_allowed"],
  "properties": {
    "reference_role": { "const": "visual_baseline_not_functional_clone" },
    "default_stack": { "const": "tauri_react_vite_tailwind_typescript" },
    "visual_reference_path": { "const": "UI-reference/" },
    "production_next_server_allowed": { "const": false },
    "pixel_level_visual_similarity_required": { "const": true }
  }
},
"layer_closure": {
  "type": "object",
  "additionalProperties": false,
  "required": ["primary_blocker_layer", "closing_layer", "lower_layer_closes_higher_layer_blocker"],
  "properties": {
    "primary_blocker_layer": { "$ref": "#/$defs/optimization_layer" },
    "closing_layer": { "$ref": "#/$defs/optimization_layer" },
    "lower_layer_closes_higher_layer_blocker": { "const": false }
  }
}
```

- [ ] **Step 2: Add experiment report schema fields**

In `perf/experiment-report.schema.json`, add top-level properties:

```json
"optimization": { "$ref": "#/$defs/optimization" },
"workload_manifest": { "$ref": "#/$defs/workload_manifest" },
"platform_evidence": { "$ref": "#/$defs/platform_evidence" },
"gui_visual": { "$ref": "#/$defs/gui_visual" }
```

Add definitions:

```json
"optimization": {
  "type": "object",
  "additionalProperties": false,
  "required": [
    "optimization_layer",
    "baseline_artifact",
    "profiler_summary",
    "stage_histogram",
    "bottleneck_statement",
    "hypothesis",
    "expected_delta",
    "rollback_condition",
    "negative_controls",
    "acceptance_gate"
  ],
  "properties": {
    "optimization_layer": { "enum": ["L1", "L2", "L3", "L4"] },
    "affected_layers": {
      "type": "array",
      "items": { "enum": ["L1", "L2", "L3", "L4"] },
      "uniqueItems": true
    },
    "baseline_artifact": { "type": "string", "minLength": 1 },
    "profiler_summary": { "type": "string", "minLength": 1 },
    "stage_histogram": { "type": "string", "minLength": 1 },
    "bottleneck_statement": { "type": "string", "minLength": 1 },
    "hypothesis": { "type": "string", "minLength": 1 },
    "expected_delta": { "type": "string", "minLength": 1 },
    "rollback_condition": { "type": "string", "minLength": 1 },
    "negative_controls": {
      "type": "array",
      "items": { "type": "string", "minLength": 1 },
      "minItems": 1
    },
    "acceptance_gate": { "type": "string", "minLength": 1 },
    "lower_layer_closes_higher_layer_blocker": { "const": false }
  }
},
"workload_manifest": {
  "type": "object",
  "additionalProperties": false,
  "required": ["query_set_source", "corpus_scale", "hardware_class", "warm_or_cold_definition", "cache_state"],
  "properties": {
    "query_set_source": { "type": "string", "minLength": 1 },
    "corpus_scale": { "type": "string", "minLength": 1 },
    "hardware_class": { "type": "string", "minLength": 1 },
    "warm_or_cold_definition": { "type": "string", "minLength": 1 },
    "cache_state": { "type": "string", "minLength": 1 }
  }
},
"platform_evidence": {
  "type": "object",
  "additionalProperties": false,
  "required": ["platform_lane", "hardware_class", "os_build_class", "power_mode", "runner_version"],
  "properties": {
    "platform_lane": { "enum": ["macos_m4_discovery", "windows_weak_host_validation", "cross_os_ci_smoke"] },
    "hardware_class": { "type": "string", "minLength": 1 },
    "os_build_class": { "type": "string", "minLength": 1 },
    "power_mode": { "type": "string", "minLength": 1 },
    "runner_version": { "type": "string", "minLength": 1 },
    "benchmark_runner_version": { "type": "string", "minLength": 1 },
    "redacted_resource_aggregate_ref": { "type": "string", "minLength": 1 },
    "redacted_stage_histogram_ref": { "type": "string", "minLength": 1 }
  }
},
"gui_visual": {
  "type": "object",
  "additionalProperties": false,
  "required": ["visual_reference_role", "default_stack", "production_next_server_allowed"],
  "properties": {
    "visual_reference_role": { "const": "visual_baseline_not_functional_clone" },
    "default_stack": { "const": "tauri_react_vite_tailwind_typescript" },
    "production_next_server_allowed": { "const": false },
    "token_inventory_ref": { "type": "string", "minLength": 1 },
    "screenshot_inventory_ref": { "type": "string", "minLength": 1 },
    "pixel_level_similarity_reviewed": { "type": "boolean" }
  }
}
```

- [ ] **Step 3: Require optimization/workload/platform for W1 private reports**

In `perf/experiment-report.schema.json`, extend the `w1_private` conditional `required` list to include:

```json
"optimization",
"workload_manifest",
"platform_evidence"
```

Extend the `gui_manual` conditional required list to include:

```json
"gui_visual"
```

- [ ] **Step 4: Update current loop state**

In `perf/current-loop-state.json`, add:

```json
"platform_lane": "cross_os_ci_smoke",
"visual_reference": {
  "reference_role": "visual_baseline_not_functional_clone",
  "default_stack": "tauri_react_vite_tailwind_typescript",
  "visual_reference_path": "UI-reference/",
  "production_next_server_allowed": false,
  "pixel_level_visual_similarity_required": true
}
```

Keep `workflow_state` as a contract-review state and do not claim production implementation.

- [ ] **Step 5: Update valid fixtures**

For W1 fixtures, add:

```json
"optimization": {
  "optimization_layer": "L1",
  "affected_layers": ["L2"],
  "baseline_artifact": "redacted://baseline/d10k",
  "profiler_summary": "redacted://profile/symbol-summary",
  "stage_histogram": "redacted://histogram/stages",
  "bottleneck_statement": "first-searchable architecture dominates import-to-query readiness",
  "hypothesis": "resident daemon first-searchable indexing reduces first-result latency",
  "expected_delta": "first-searchable p95 improves by at least 40%",
  "rollback_condition": "recovery failure rate increases or peak RSS exceeds gate",
  "negative_controls": ["query parser microbenchmark unchanged"],
  "acceptance_gate": "D10K_private_calibration",
  "lower_layer_closes_higher_layer_blocker": false
},
"workload_manifest": {
  "query_set_source": "$RESUME_IR_QUERY_ARTIFACT_ROOT source_search static set",
  "corpus_scale": "D10K_private_calibration",
  "hardware_class": "macos_m4_discovery",
  "warm_or_cold_definition": "warm daemon after 30s warmup",
  "cache_state": "documented per run"
},
"platform_evidence": {
  "platform_lane": "macos_m4_discovery",
  "hardware_class": "apple_silicon_m4_class",
  "os_build_class": "macos_current",
  "power_mode": "plugged",
  "runner_version": "resume-ir-runner-contract-v1",
  "benchmark_runner_version": "resume-ir-benchmark-contract-v1",
  "redacted_resource_aggregate_ref": "redacted://resources/aggregate",
  "redacted_stage_histogram_ref": "redacted://histogram/stages"
}
```

For GUI/manual fixtures, add:

```json
"gui_visual": {
  "visual_reference_role": "visual_baseline_not_functional_clone",
  "default_stack": "tauri_react_vite_tailwind_typescript",
  "production_next_server_allowed": false,
  "token_inventory_ref": "redacted://gui/token-inventory",
  "screenshot_inventory_ref": "redacted://gui/screenshots",
  "pixel_level_similarity_reviewed": true
}
```

- [ ] **Step 6: Add invalid fixtures**

Create `perf/fixtures/invalid/profile-missing-optimization-layer.json` by copying a valid W1 report and removing `optimization.optimization_layer`.

Create `perf/fixtures/invalid/profile-invalid-platform-lane.json` by copying a valid W1 report and setting:

```json
"platform_lane": "macos_only_claims_windows"
```

Create `perf/fixtures/invalid/lower-layer-closes-higher-layer-blocker.json` by copying a valid W1 report and setting:

```json
"lower_layer_closes_higher_layer_blocker": true
```

- [ ] **Step 7: Verify Task 4**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
python3 scripts/ci/check-loop-state.py
python3 scripts/ci/check-experiment-report.py
python3 -m json.tool perf/loop-state.schema.json >/dev/null
python3 -m json.tool perf/experiment-report.schema.json >/dev/null
python3 -m json.tool perf/current-loop-state.json >/dev/null
```

Expected:

```text
check-performance-contracts.py passed
check-loop-state.py passed
check-experiment-report.py passed
```

- [ ] **Step 8: Commit Task 4**

```bash
git add perf/loop-state.schema.json perf/experiment-report.schema.json perf/current-loop-state.json perf/fixtures/valid perf/fixtures/invalid
git commit -m "contracts: add gui performance platform schema fields"
```

## Task 5: Add Guard Enforcement

**Files:**
- Modify: `scripts/ci/check-performance-contracts.py`
- Modify: `scripts/ci/check-loop-state.py`
- Modify: `scripts/ci/check-experiment-report.py`
- Modify: `scripts/ci/check-autonomous-goal.py`
- Modify: `scripts/ci/check-benchmark-lanes.py`
- Modify: `scripts/ci/check-private-evidence-redaction.py`

- [ ] **Step 1: Add constants to aggregate guard**

In `scripts/ci/check-performance-contracts.py`, add:

```python
OPTIMIZATION_LAYERS = ["L1", "L2", "L3", "L4"]
PLATFORM_LANES = ["macos_m4_discovery", "windows_weak_host_validation", "cross_os_ci_smoke"]
GUI_DEFAULT_STACK = "tauri_react_vite_tailwind_typescript"
GUI_REFERENCE_ROLE = "visual_baseline_not_functional_clone"
```

- [ ] **Step 2: Validate matrix additions**

In `validate_matrix`, after current query and GUI checks, add:

```python
    optimization_layers = require_mapping(matrix.get("optimization_layers"), "matrix.optimization_layers")
    if optimization_layers.get("allowed") != OPTIMIZATION_LAYERS:
        fail("matrix.optimization_layers.allowed mismatch")
    require_bool(
        optimization_layers.get("require_single_primary_layer"),
        True,
        "matrix.optimization_layers.require_single_primary_layer",
    )
    require_bool(
        optimization_layers.get("allow_affected_layers"),
        True,
        "matrix.optimization_layers.allow_affected_layers",
    )

    platform_lanes = require_mapping(matrix.get("platform_lanes"), "matrix.platform_lanes")
    if platform_lanes.get("allowed") != PLATFORM_LANES:
        fail("matrix.platform_lanes.allowed mismatch")
    require_bool(
        platform_lanes.get("macos_m4_can_close_windows_gate"),
        False,
        "matrix.platform_lanes.macos_m4_can_close_windows_gate",
    )
    require_bool(
        platform_lanes.get("cross_os_ci_smoke_can_replace_weak_host_perf"),
        False,
        "matrix.platform_lanes.cross_os_ci_smoke_can_replace_weak_host_perf",
    )

    gui_stack = require_mapping(matrix.get("gui_stack"), "matrix.gui_stack")
    if gui_stack.get("default_stack") != GUI_DEFAULT_STACK:
        fail("matrix.gui_stack.default_stack mismatch")
    require_bool(gui_stack.get("production_next_server_allowed"), False, "matrix.gui_stack.production_next_server_allowed")
```

- [ ] **Step 3: Validate active goal additions**

In `scripts/ci/check-autonomous-goal.py`, after activation checks, add:

```python
    gui = active_goal.get("gui")
    if not isinstance(gui, dict):
        fail("ACTIVE_GOAL.toml: missing [gui]")
    if gui.get("default_stack") != "tauri_react_vite_tailwind_typescript":
        fail("gui.default_stack mismatch")
    require_bool(gui.get("production_next_server_allowed"), False, "gui.production_next_server_allowed")
    require_bool(gui.get("toolkit_bakeoff_default_required"), False, "gui.toolkit_bakeoff_default_required")
    require_bool(gui.get("toolkit_bakeoff_requires_blocker_issue"), True, "gui.toolkit_bakeoff_requires_blocker_issue")

    platform_lanes = active_goal.get("platform_lanes")
    if not isinstance(platform_lanes, dict):
        fail("ACTIVE_GOAL.toml: missing [platform_lanes]")
    if platform_lanes.get("primary_discovery") != "macos_m4_discovery":
        fail("platform_lanes.primary_discovery mismatch")
    if platform_lanes.get("weak_host_validation") != "windows_weak_host_validation":
        fail("platform_lanes.weak_host_validation mismatch")
    require_bool(platform_lanes.get("macos_m4_can_close_windows_gate"), False, "platform_lanes.macos_m4_can_close_windows_gate")
```

- [ ] **Step 4: Validate experiment report additions**

In `scripts/ci/check-experiment-report.py`, add checks that every W1 fixture has `optimization`, `workload_manifest`, and `platform_evidence`. The function body should require:

```python
optimization = report.get("optimization")
if report.get("evidence_lane") == "w1_private":
    if not isinstance(optimization, dict):
        fail(f"{path}.optimization: expected object")
    if optimization.get("optimization_layer") not in {"L1", "L2", "L3", "L4"}:
        fail(f"{path}.optimization.optimization_layer invalid")
    for key in [
        "baseline_artifact",
        "profiler_summary",
        "stage_histogram",
        "bottleneck_statement",
        "hypothesis",
        "expected_delta",
        "rollback_condition",
        "acceptance_gate",
    ]:
        if not optimization.get(key):
            fail(f"{path}.optimization.{key}: required")
    if not optimization.get("negative_controls"):
        fail(f"{path}.optimization.negative_controls: required")
    if optimization.get("lower_layer_closes_higher_layer_blocker") is not False:
        fail(f"{path}.optimization.lower_layer_closes_higher_layer_blocker: expected false")

    workload = report.get("workload_manifest")
    if not isinstance(workload, dict):
        fail(f"{path}.workload_manifest: expected object")
    for key in ["query_set_source", "corpus_scale", "hardware_class", "warm_or_cold_definition", "cache_state"]:
        if not workload.get(key):
            fail(f"{path}.workload_manifest.{key}: required")

    platform = report.get("platform_evidence")
    if not isinstance(platform, dict):
        fail(f"{path}.platform_evidence: expected object")
    if platform.get("platform_lane") not in {"macos_m4_discovery", "windows_weak_host_validation", "cross_os_ci_smoke"}:
        fail(f"{path}.platform_evidence.platform_lane invalid")
```

- [ ] **Step 5: Strengthen private evidence redaction**

In `scripts/ci/check-private-evidence-redaction.py`, add prohibited patterns:

```python
PROHIBITED_PATTERNS = [
    "/" + "Users/",
    "\\\\" + "Users" + "\\\\",
    "~" + "/",
    "$" + "HOME/",
    "C:" + "\\\\" + "Users" + "\\\\",
]
```

Allow `$RESUME_IR_PRIVATE_RESUME_ROOT`, `$RESUME_IR_QUERY_ARTIFACT_ROOT`, and `$RESUME_IR_LOCAL_EVIDENCE_DIR` as symbolic names.

- [ ] **Step 6: Verify Task 5**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
python3 scripts/ci/check-autonomous-goal.py
python3 scripts/ci/check-loop-state.py
python3 scripts/ci/check-experiment-report.py
python3 scripts/ci/check-benchmark-lanes.py
python3 scripts/ci/check-private-evidence-redaction.py
python3 -m py_compile scripts/ci/check-*.py
```

Expected: every check prints a `passed` line or exits 0. `py_compile` exits 0.

- [ ] **Step 7: Commit Task 5**

```bash
git add scripts/ci/check-performance-contracts.py scripts/ci/check-loop-state.py scripts/ci/check-experiment-report.py scripts/ci/check-autonomous-goal.py scripts/ci/check-benchmark-lanes.py scripts/ci/check-private-evidence-redaction.py
git commit -m "ci: enforce gui performance loop contract fields"
```

## Task 6: Final Contract Verification And PR Update

**Files:**
- Modify: PR body through GitHub CLI or GitHub app
- Verify: all files changed by Tasks 1-5

- [ ] **Step 1: Run focused public gates**

Run:

```bash
python3 scripts/ci/check-performance-contracts.py
python3 -m py_compile scripts/ci/check-*.py
./scripts/ci/guard-public-repo.sh
git diff --check -- ACTIVE_GOAL.toml perf .github scripts/ci 03_next_goal_高性能本地检索GUI闭环 docs/superpowers
```

Expected:

```text
check-performance-contracts.py passed
```

`py_compile`, privacy guard, and diff check exit 0.

- [ ] **Step 2: Run contract-private-string scan**

Run:

```bash
python3 scripts/ci/check-private-evidence-redaction.py
```

Expected: privacy redaction guard exits 0 and prints a passed line.

- [ ] **Step 3: Inspect branch status**

Run:

```bash
git status --short --branch
git log --oneline --max-count=8
```

Expected: branch is ahead by the new commits. `UI-reference/` may remain untracked and must not be staged.

- [ ] **Step 4: Update PR body**

Update PR #10 body with this summary:

```markdown
## Scope

Contract-only update for GUI stack/visual baseline, performance optimization taxonomy, platform profiling lanes, and autonomous loop drift controls.

## Changes

- Locks GUI default route to `Tauri + React + Vite + Tailwind + TypeScript`.
- Treats `UI-reference/` as visual baseline, not functional clone.
- Replaces default egui/Slint bakeoff with blocker-driven fallback.
- Adds L0/L1/L2/L3/L4 performance optimization taxonomy.
- Adds macOS M4 discovery, Windows weak-host validation, and cross-OS CI smoke lanes.
- Adds schema, fixture, issue template, PR template, and guard enforcement.

## Verification

- `python3 scripts/ci/check-performance-contracts.py`
- `python3 -m py_compile scripts/ci/check-*.py`
- `./scripts/ci/guard-public-repo.sh`
- `git diff --check -- ACTIVE_GOAL.toml perf .github scripts/ci 03_next_goal_高性能本地检索GUI闭环 docs/superpowers`

## Privacy Boundary

No raw resumes, raw queries, raw trace content, local private paths, tokens, diagnostics packages, OCR text, or model caches are committed. Private corpus roots are represented only through symbolic env names.
```

- [ ] **Step 5: Push branch**

Run:

```bash
git push
```

Expected: branch push succeeds and PR #10 updates.

- [ ] **Step 6: Watch PR checks**

Run:

```bash
gh pr checks 10 --watch
```

Expected: required public checks pass. If a check fails, use the check log as the next evidence path and fix only the contract slice that failed.

## Final Review Checklist

- [ ] `07_GUI与手工Codex闭环.md` no longer requires egui/Slint bakeoff before stack freeze.
- [ ] `ACTIVE_GOAL.toml` contains GUI, platform lane, and private corpus transfer policy.
- [ ] `15_性能观测与Profiling工具链.md` contains L0/L1/L2/L3/L4 taxonomy and lower-layer closure rule.
- [ ] `perf/acceptance-matrix.toml` contains optimization layer and platform lane redlines.
- [ ] `perf/experiment-report.schema.json` requires L0/workload/platform fields for W1 reports.
- [ ] `perf/loop-state.schema.json` can represent platform lane, visual reference, and layer closure metadata.
- [ ] Profile issue and PR templates include all required anchors.
- [ ] Guard scripts reject missing optimization layer, invalid platform lane, lower-layer closure misuse, and public private-path leakage.
- [ ] No production GUI/daemon/benchmark code changed in this plan.
- [ ] `UI-reference/` remains untracked unless a separate approved plan changes that.

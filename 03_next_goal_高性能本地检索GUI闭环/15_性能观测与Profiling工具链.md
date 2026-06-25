# 性能观测与 Profiling 工具链

本文件冻结后续性能优化的观测面。没有 baseline、trace、histogram 和 profiler evidence，不允许声明性能优化完成。

## 1. 观测原则

1. 先观测，再改热路径。
2. 每个优化切片必须先记录 baseline，再记录优化后结果。
3. 所有性能声明必须绑定当前机器 schema 的 evidence lane：`smoke`、`w0_docs`、`w1_private`、`soak_fault` 或 `gui_manual`。
4. W0/W1/soak/fault/GUI/manual 只是 display alias；机器字段和 issue/PR evidence anchors 必须使用 schema 值。
5. 私有 `w1_private` 只提交 redacted aggregate，不提交 raw query、真实简历、路径、trace 原文或 diagnostics package。
6. profiler 结果只能提交符号化摘要和本地文件 hash，不提交本机路径。
7. 性能报告必须说明 release build、warmup、重复次数、open-loop/closed-loop 方法、容量点和 coordinated omission 处理方式。

Profile issue ledger rules:

1. profile issue is the profiling ledger; `.github/ISSUE_TEMPLATE/profile_issue.md` is the target template path for that ledger, and until the template exists the linked GitHub issue text must carry the same anchors.
2. negative experiment is valid evidence
3. benchmark regression is recorded as a profile issue outcome/comment label, not a machine state; map it to `experiment_state=reverted` unless it also violates a release gate, in which case use `experiment_state=blocked`
4. no profile, no optimization

Required profile issue anchors/fields:

```text
Evidence Lane
Benchmark Lane
Dataset
Corpus Profile Hash
query_set_sha256
Baseline Command
Baseline Evidence
Profiler Evidence
Hypothesis
Target Metric
Success Threshold
Failure/Regression Guard
Outcome/Decision
Experiment Updates/Comment Log
Privacy Boundary
Linked PRs
Closing Evidence
```

Closure must include before/after metric, percentage change, `query_set_sha256`, `corpus_profile_hash`, command id or script name, and privacy redaction confirmation. `corpus_profile_hash` must follow the public-safe hash boundary in `09_安全隐私与本地证据边界.md`.

## 2. Instrumentation Contract

Rust 实现必须在热路径保留结构化 span 和 stage metrics：

| Stage | 必需字段 | 禁止 |
|---|---|---|
| query_parse | `request_id`, `query_shape`, `term_count`, `mode` | raw query text |
| prefilter | `candidate_count_before`, `candidate_count_after`, `filter_count` | field value 原文 |
| bm25 | `segment_count`, `candidate_count`, `elapsed_ms` | stored body |
| ann | `enabled`, `candidate_count`, `elapsed_ms`, `partial_reason` | embedding payload |
| fusion | `input_counts`, `output_count`, `elapsed_ms` | candidate identity 明文 |
| bulk_hydrate | `top_k`, `hydrated_count`, `elapsed_ms` | local path |
| snippet | `snippet_count`, `elapsed_ms`, `partial_reason` | raw snippet text in committed evidence |

每个 stage 必须能汇总到 histogram。`w1_private` 报告至少包含 P50/P95/P99、stage P95、RSS peak、CPU aggregate、disk read/write aggregate。

Methodology hard rules:

1. 使用 release build；debug/dev build 只能作为 smoke。
2. warmup 至少 30 秒，正式测量至少 5 次重复，报告 median run 和 worst valid run。
3. 同时记录 closed-loop latency 和 open-loop arrival latency；只用 closed-loop 不能证明 overload 行为。
4. 必须覆盖 30%、70%、100%、120% 四个 capacity points；benchmark/codex/background queue class 还要记录 admission/rejection。
5. 需要明确 coordinated omission 修正方式；没有修正时该报告不能用于完成声明。
6. profiler overhead 必须 <= 3%，否则 profiler run 只能定位热点，不能作为最终 latency 证据。

## 3. Toolchain

| 层 | 工具 | 用途 | 证据 |
|---|---|---|---|
| micro benchmark | Criterion 或等价 Rust benchmark | parser、prefilter、fusion、hydrate 的小范围回归 | public/synthetic summary |
| latency histogram | hdrhistogram 或等价结构 | resident daemon batch 的 percentile | redacted bucket histogram |
| structured tracing | `tracing` spans | stage latency、queue delay、partial reason | redacted span summary |
| macOS sampling | Samply 或 Instruments | CPU 火焰图、allocator、IO hotspot | symbol summary + local capture hash |
| Windows sampling | WPR/WPA 或 ETW consumer | CPU、disk、queue hotspot | symbol summary + local capture hash |
| long run | resident daemon soak harness | restart、cancel、overload、journal gap | `soak_fault` aggregate |

工具选择可以在实现期替换，但输出字段不能弱化。替换工具必须继续满足 `perf/experiment-report.schema.json`。

## 4. Baseline Sequence

每个性能切片进入实现前必须执行：

1. 锁定 `ACTIVE_GOAL.toml` 和 `perf/acceptance-matrix.toml` 的版本。
2. 运行 `smoke` 或 `w0_docs` baseline，确认公开验证仍可复现。
3. 对 `w1_private` 私有目标，运行 resident daemon baseline，禁止 per-query process spawn。
4. 捕获 stage histogram、resource aggregate 和 profiler summary。
5. 在 Loop 状态中进入 `baseline_validated`，再进入 `profile_captured`。
6. 记录可证伪 hypothesis：预期改善的 stage、预期幅度、正确性风险和 rollback trigger。
7. 按 hotspot 优先级选择单个优化切片。

## 5. Completion Redlines

任何性能切片满足以下任一条件时不得标为 complete：

1. 没有 baseline。
2. 只报告均值，没有 P95/P99。
3. 只报告端到端 latency，没有 stage latency。
4. 只使用 per-query process spawn benchmark 声称 daemon 性能。
5. 没有证明 hot path 中 OCR、全文解析、重模型推理为 false。
6. 没有说明 query semantics 版本。
7. `smoke` 或 synthetic 结果被当成 `w1_private` 私有基线。
8. 没有 release/warmup/repetition/capacity/coordinated-omission 方法说明。
9. D10K 或 D100K 结果被当成完整 `goal_complete` 证据。

## 6. Evidence Shape

后续实验报告必须能通过 `perf/experiment-report.schema.json` 校验。报告中 `thresholds.matrix` 必须指向 `perf/acceptance-matrix.toml`，并列出所有 failed redlines。未达标可以进入 `blocked` 或下一优化切片，但不能改写验收门槛后宣布通过。

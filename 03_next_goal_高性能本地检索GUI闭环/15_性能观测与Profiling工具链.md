# 性能观测与 Profiling 工具链

本文件冻结后续性能优化的观测面。没有 baseline、trace、histogram 和 profiler evidence，不允许声明性能优化完成。

## 1. 观测原则

1. 先观测，再改热路径。
2. 每个优化切片必须先记录 baseline，再记录优化后结果。
3. 所有性能声明必须绑定当前机器 schema 的 evidence lane：`smoke`、`w0_docs`、`w1_private`、`soak_fault` 或 `gui_manual`。
4. W0/W1/soak/fault/GUI/manual 只是 display alias；机器字段和 issue/PR evidence anchors 必须使用 schema 值。
5. 私有 `w1_private` 只提交 redacted aggregate，不提交 raw query、真实简历、路径、trace 原文或 diagnostics package。
6. profiler 结果只能提交 public-safe redacted symbol-summary/report hash、approved opaque manifest ref 或 HMAC-SHA256 opaque manifest ref，不提交本机路径。
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
git_base_sha
git_head_sha
base_drift/reconciliation_status
benchmark_report_hash or benchmark_artifact_id
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

`benchmark_report_hash` or `benchmark_artifact_id` must identify a public-safe redacted report, approved opaque manifest, or HMAC-SHA256 opaque manifest. Raw profiler capture/file hash is local-only private evidence and must not enter git, GitHub issue text, PR prose, or public summaries.

Closure must include before/after metric, percentage change, `query_set_sha256`, `corpus_profile_hash`, `git_base_sha`, `git_head_sha`, `base_drift/reconciliation_status`, public-safe `benchmark_report_hash` or `benchmark_artifact_id`, command id or script name, and privacy redaction confirmation. `query_set_sha256`, `corpus_profile_hash`, and benchmark report/artifact identifiers must follow the public-safe hash boundary in `09_安全隐私与本地证据边界.md`.

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

## Platform Profiling Lanes

`macos_m4_discovery` 使用 Samply、Instruments、`tracing` span、hdrhistogram、release benchmark 和 synthetic pressure。它能指导 hotspot 排序，不能代表 Windows weak-host 结论。

`windows_weak_host_validation` 使用 WPR/WPA/ETW 或等价 Windows performance consumer、PowerShell runner、USN Journal / filesystem watcher verification、WebView2/Tauri packaging smoke 和 resource aggregate。

`cross_os_ci_smoke` 只证明 CI 层面的构建和测试 smoke。

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
| macOS sampling | Samply 或 Instruments | CPU 火焰图、allocator、IO hotspot | public-safe symbol-summary/report hash or opaque/HMAC manifest ref |
| Windows sampling | WPR/WPA 或 ETW consumer | CPU、disk、queue hotspot | public-safe symbol-summary/report hash or opaque/HMAC manifest ref |
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

Synthetic smoke baseline 是第 2 步的公开 harness baseline。它必须输出
`resume-ir.synthetic-smoke-baseline.v1` redacted report 和
`resume-ir.synthetic-smoke-artifact-manifest.v1` manifest，并记录 query/OCR/vector
和 private-query runner 四个 component artifact 的 hash、size、schema_version 和
`target_claim = not_evaluated`。它还必须在 synthetic fixture 上实际观察 product
batch query protocol 和 stage timing 字段，并把 redacted `batch_protocol_stage`
timing 写入 smoke report。公开 smoke report 的 `harness_observations`
还必须记录 private-query runner 观察到的 `resume-ir-query-v2`、
`request_sample_count` 和 query embedding invocation count，证明 smoke
实际经过 batch runner 形状。公开 smoke report 还必须把 product query protocol
的 `rss_delta_mb` 和 private-query runner 的 redacted `rss_delta_mb` summary
写入 `resource_observations`，作为后续 D10K private baseline 的资源观测对齐点。
它不捕获 profiler，不打开 profile optimization issue，
不声明 resident daemon、D10K、W1、scale gate 或 goal completion。

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

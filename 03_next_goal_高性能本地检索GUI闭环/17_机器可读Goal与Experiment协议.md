# 机器可读 Goal 与 Experiment 协议

本文件说明根目录机器可读合同的用途。它们是文档 PR 的 review gate，不是私有 benchmark 输出。

## 1. Files

| 文件 | 责任 |
|---|---|
| `ACTIVE_GOAL.toml` | 当前活跃目标、允许路径、隐私边界、Loop 初始状态、PR 状态 |
| `perf/acceptance-matrix.toml` | W0/W1/soak/fault/GUI 红线和性能阈值 |
| `perf/loop-state.schema.json` | 长程目标状态报告 schema |
| `perf/experiment-report.schema.json` | redacted 实验报告 schema |

这些文件可以提交，因为只含 schema、阈值、状态和布尔隐私标记，不含真实 query、简历、路径或诊断包。

## 2. Goal Lock Rules

1. 每次长程 Codex 执行开始时读取 `ACTIVE_GOAL.toml`。
2. 若执行目标、允许路径、隐私边界或 PR 状态与用户请求冲突，停止并回到 `fw-ceo-review` 或 `fw-plan`。
3. 当前 PR 的 `production_code_allowed=false`；任何 Rust、GUI、脚本实现都必须等新的 implementation plan 批准后再改。
4. 目标锁不能被实现者临时放宽。需要放宽时必须先改 spec/plan 并重新 review。

## 3. Experiment Report Rules

W1、soak/fault 和 GUI/manual 证据必须生成本地私有完整报告，并只把 redacted aggregate summary 带入 git。公开 summary 必须满足：

1. schema version 固定。
2. dataset 和 query set 使用 hash，不出现路径或原文。
3. latency 至少包含 P50/P95/P99 和 stage P95。
4. resources 至少包含 RSS、CPU、disk aggregate。
5. hot path flags 明确为 false。
6. profiler capture 只提交 ref/hash，不提交本地 capture 文件。
7. thresholds 必须引用 `perf/acceptance-matrix.toml`，并列出 failed redlines。

## 4. Review Closure

review ledger 的每条问题都必须有：

1. `status`：`open`、`closed_by_contract`、`deferred_to_implementation` 或 `false_positive`。
2. `closure_evidence`：具体文件或机器合同。
3. `closed_by`：提交、PR 或后续切片。

没有 closure evidence 的问题不能从 review 中消失。实现阶段发现合同不够时，新增问题行，不覆盖旧行。

## 5. Parse Gate

当前 docs PR 的机器 gate：

```bash
python3 - <<'PY'
import json
import pathlib
import tomllib

tomllib.loads(pathlib.Path("ACTIVE_GOAL.toml").read_text())
tomllib.loads(pathlib.Path("perf/acceptance-matrix.toml").read_text())
json.loads(pathlib.Path("perf/loop-state.schema.json").read_text())
json.loads(pathlib.Path("perf/experiment-report.schema.json").read_text())
PY
```

通过 parse gate 只说明机器合同格式有效，不代表 W1 私有 benchmark 已经执行。

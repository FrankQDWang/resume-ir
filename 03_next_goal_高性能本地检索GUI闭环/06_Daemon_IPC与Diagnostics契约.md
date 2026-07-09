# Daemon IPC 与 Diagnostics 契约

## 1. 冻结含义

冻结 daemon IPC/diagnostics contract 是阶段级、版本化冻结，不是永久不变。

允许：

1. 内部存储 breaking change。
2. 内部索引 schema breaking change。
3. 新增 additive diagnostics 字段。
4. 新增 `performance.v1`、`benchmark.v1` 输出。

禁止：

1. 无版本改变现有 response envelope。
2. GUI 依赖未版本化内部字段。
3. diagnostics 输出 raw query、resume path、raw OCR text。
4. benchmark 证据混用 smoke 和 full baseline。

## 2. IPC Envelope

所有 daemon IPC request 使用稳定 envelope：

```json
{
  "schema_version": "resume-ir.ipc-request.v2",
  "request_id": "uuid",
  "client_capability": "interactive_gui|codex_validation|benchmark|background",
  "deadline_ms": 200,
  "idempotency_key": "optional-stable-key",
  "cancel_token": "optional-cancel-token",
  "batch_id": "optional-batch-id",
  "payload": {}
}
```

`client_capability` 是客户端声明的能力，不直接决定公平性。daemon 必须根据本地 IPC endpoint、session registration、benchmark registration token 和请求来源计算内部 `client_class`，并把结果写入 redacted diagnostics。未注册客户端默认降级为 `background` 或拒绝，不能通过自报字段抢占 interactive 配额。

`deadline_ms` 是合同，不是 hint：daemon may return partial or overload instead of blocking beyond the deadline. `cancel_token` must be accepted by queued and active long-running work. `batch_id` groups GUI or benchmark requests without exposing raw query text in diagnostics.

## 2.1 Transport and Framing

1. 默认 transport 是本机 Unix domain socket 或 Windows named pipe；loopback TCP 只能作为显式开发模式。
2. 每条消息使用 length-prefixed UTF-8 JSON envelope，最大 request body 是 65536 bytes；超过限制返回 `REQUEST_TOO_LARGE`。
3. IPC payload 可以在内存中携带 interactive raw query，但 daemon 禁止把 raw query 写入日志、diagnostics、trace、benchmark summary 或 git。
4. benchmark batch 可在本机私有 IPC/pipe 中携带 query，但不得把 raw query 写入 diagnostics、trace、benchmark summary 或 git；公开/诊断证据只引用 `query_set_sha256`、request/sample counts、bucket aggregate 和 redaction flags。
5. cancel request 必须能取消 queued、running 和 batch child request；已完成请求返回 idempotent cancelled/complete 状态。
6. 单个 batch 最多 64 个 query references；interactive search in-flight 上限 8，Codex validation 上限 2，benchmark 上限 1，background 上限 4。
7. queued cancel acknowledgement P95 必须 <= 200ms；running cancel 必须在下一安全 checkpoint 之前进入 cancelled 或 complete。

## 3. Status Contract

```json
{
  "schema_version": "resume-ir.status.v2",
  "visible_epoch": 42,
  "level_counts": {
    "level1_discovered": 10000,
    "level2_text_searchable": 8000,
    "level3_enhanced": 4500
  },
  "queues": {
    "parse": 0,
    "ocr": 1200,
    "embedding": 800,
    "compaction": 1
  },
  "budget_profile": "balanced",
  "health": "ok|degraded|repairing",
  "partial_reasons": []
}
```

## 4. Search Contract

Search response 必须说明每个查询是否 partial，以及哪些 layer 参与：

```json
{
  "schema_version": "resume-ir.search-response.v2",
  "request_id": "uuid",
  "visible_epoch": 42,
  "query_mode": "keyword|field_filter|hybrid|semantic",
  "partial": false,
  "partial_reasons": [],
  "latency_ms": 87,
  "stage_latency_ms": {
    "parse": 1,
    "prefilter": 6,
    "bm25": 24,
    "ann": 12,
    "fusion": 3,
    "bulk_hydrate": 18,
    "snippet": 15
  },
  "results": []
}
```

## 5. Search Batch and Overload Contract

Benchmark registration:

```json
{
  "schema_version": "resume-ir.query-set-register.v1",
  "request_id": "uuid",
  "client_capability": "benchmark",
  "query_set_sha256": "hex",
  "query_count": 500,
  "request_sample_count": 25000,
  "bucket_counts": {
    "single_term": 50,
    "and_2": 75,
    "and_3_5": 150,
    "and_6_16": 50,
    "field_filter": 75,
    "hybrid": 75,
    "semantic": 25
  },
  "contains_raw_query_in_diagnostics": false
}
```

Resident batch request stream:

```jsonl
{"schema_version":"resume-ir.query-batch-request.v2","request_id":"private-query-request-1","query":"<local-private-query>"}
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

Fairness and admission:

| Class | Admission | Weight | Hard rule |
|---|---|---:|---|
| `interactive_gui` | always admitted until overload deadline would be violated | 70 | keyword/field search protected first |
| `codex_validation` | admitted when interactive queue is healthy | 15 | cannot starve GUI |
| `benchmark` | max one active batch unless explicitly raised by plan | 10 | never runs during interactive overload |
| `background` | opportunistic | 5 | first class to pause |

The daemon uses weighted fair queueing with deadline-aware admission. OCR/vector/import/compaction degrade before interactive keyword and field-filter search. If interactive P95 or queue delay crosses `perf/acceptance-matrix.toml` governor redlines, benchmark and background work must pause or return `OVERLOADED`.

## 6. Diagnostics Contract

Diagnostics 必须默认脱敏：

```json
{
  "schema_version": "resume-ir.diagnostics.v2",
  "privacy_boundary": "redacted_local_aggregate",
  "contains_raw_resume_text": false,
  "contains_queries": false,
  "contains_resume_paths": false,
  "contains_candidate_results": false,
  "contains_snippet_text": false,
  "visible_epoch": 42,
  "metrics": {},
  "error_counts": {},
  "benchmark_refs": []
}
```

Diagnostics 只允许输出 aggregate metrics、bucket counts、hash/ref、redaction flags、error class、queue depth 和 redacted partial reason。不得输出 raw query、raw snippet、resume path、candidate identity、local capture path、token、cookie 或 diagnostics package 原文。

## 7. GUI 依赖面

GUI 只能依赖：

1. status contract。
2. search contract。
3. detail contract。
4. task control contract。
5. diagnostics export contract。
6. benchmark summary contract。

GUI 不依赖内部 SQLite table、Tantivy field name、sidecar path、ANN implementation。

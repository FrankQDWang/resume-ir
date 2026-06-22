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

所有 daemon IPC response 使用稳定 envelope：

```json
{
  "schema_version": "resume-ir.ipc-response.v1",
  "request_id": "uuid",
  "status": "ok|partial|error",
  "visible_epoch": 42,
  "data": {},
  "warnings": [],
  "diagnostics_ref": null
}
```

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

## 5. Diagnostics Contract

Diagnostics 必须默认脱敏：

```json
{
  "schema_version": "resume-ir.diagnostics.v2",
  "privacy_boundary": "redacted_local_aggregate",
  "contains_raw_resume_text": false,
  "contains_queries": false,
  "contains_resume_paths": false,
  "contains_candidate_results": false,
  "visible_epoch": 42,
  "metrics": {},
  "error_counts": {},
  "benchmark_refs": []
}
```

## 6. GUI 依赖面

GUI 只能依赖：

1. status contract。
2. search contract。
3. detail contract。
4. task control contract。
5. diagnostics export contract。
6. benchmark summary contract。

GUI 不依赖内部 SQLite table、Tantivy field name、sidecar path、ANN implementation。

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

## 1.1 2026-07-13 实现审计（历史 checkpoint）

本节保留 S759–S763 当时的审计证据；当前可执行合同以本文第 2、4、5、12 节的
S807 v3 硬切为准。v1/v2 字样不是兼容许可，也不得用于新客户端。

当前 daemon `/search` 是经过 bearer authentication 的 loopback HTTP/1.x
单请求接口；keyword、field、semantic 和 hybrid 的 focused tests 可执行通过，
S759 已硬切到有界 `resume-ir.ipc-request.v2` request envelope 和
`resume-ir.search-response.v2` success response；S760 又增加 resident query
worker 和独立 monotonic deadline scheduler，但它还不是本文件定义的完整
resident query IPC：

1. listener 可继续接收并调度 search connection，但 query execution 仍由单个
   resident worker 顺序执行；deadline response 不再被前序慢请求阻塞。
2. 单请求 client/server 仍返回 `Connection: close`，没有通用 persistent
   connection 或 length-prefixed framing。S763 的 batch endpoint 在一条
   close-delimited HTTP response 内流式返回 NDJSON child response；这减少 batch
   的连接次数，但不等价于完整 persistent framing。
3. S758 后 metadata store、Ready generation full-text reader 和 generation-bound
   vector search owner 已在 daemon 内复用，不再逐 request reopen/rebuild。
4. S761 为 authenticated request 增加总 in-flight 16 的硬上限，并按声明且校验
   的 capability 分区：interactive 8、Codex validation 2、benchmark request 8、
   background 4。超限立即返回 request-correlated HTTP 503
   `OVERLOADED`，固定 `retry_after_ms=250`，不保留 query task/stream clone。
   已完成请求会唤醒 deadline scheduler 并释放 permit；已超时但仍未退出的 work
   继续占用 permit。S762 增加有界 opaque `cancel_token` 和 authenticated
   `POST /search/cancel`：queued work 从队列移除，running embedding 在 10ms
   checkpoint 终止隔离进程组，atomic claim 防重复 response。最近 128 个
   terminal token 支持幂等返回；unknown/expired 都返回 `complete`。S763 增加
   authenticated `POST /search/batch`：一次只允许一个 active batch，每批 1..=64
   个同 capability v2 child，逐 child 经过现有 admission、deadline、cancel 和
   response path。尚无 server-derived session/benchmark registration 或 weighted
   fair scheduling；benchmark child 8 是 in-flight 上限，不等于允许 8 个 batch。
5. S759 后 request 必须携带有界 `request_id`、`client_capability`、
   `deadline_ms` 和 object payload；success response 回显 request identity、
   `visible_epoch`、partial metadata、总延迟和 7-stage latency。S760 后
   `deadline_ms` 使用 monotonic clock 覆盖 queue、stage 和 query embedding：
   排队或运行中的请求到期会立即返回 request-correlated
   `partial_reasons=["deadline_exceeded"]`，而不是等待队首慢请求结束；worker
   不再执行已由 deadline monitor 完成的 queued task。显式开发模式下的
   loopback HTTP `/search` success response
   仍附带固定、最多 7 项的标准 `Server-Timing` header：`query_parse`、
   `prefilter`、`bm25`、`ann`、`fusion`、`bulk_hydrate`、`snippet`。该 header
   不包含 query、候选身份、路径或正文。S763 的 NDJSON child body 继续携带 v2
   `stage_latency_ms`，但没有复制 `Server-Timing` header。该证据不能作为
   server-derived class、weighted fairness 或 persistent framing 已实现的证据。
6. direct CLI 和 daemon 都在搜索结束后向 SQLite 写 query observation；这不符合
   query hot path read-only 边界，必须改为有界异步 telemetry 或移出请求路径。
7. semantic/hybrid 每次 query 都通过 local command 生成 query embedding；当前
   benchmark 的 `spawn_per_query=false` 只证明外层 query runner 没有逐 query
   启动，并不证明 embedding runtime 或 daemon resources resident。

这些缺口不能因为 v2 identity/envelope 已落地而被当作完成；现有
command-resident synthetic harness、resident load baseline 或 daemon capability
tests 都不是 server-derived client class、weighted fairness 或完整 persistent
framing 已实现的证据。

## 2. IPC Envelope

所有 daemon IPC request 使用稳定 envelope：

```json
{
  "schema_version": "resume-ir.ipc-request.v3",
  "request_id": "uuid",
  "client_capability": "interactive_gui|codex_validation|benchmark|background",
  "deadline_ms": 200,
  "cancel_token": "optional-cancel-token",
  "payload": {}
}
```

Envelope 只允许上述字段；未知字段、旧别名和旧 schema 一律返回 400，不做兼容解析。

`client_capability` 是客户端声明的能力，不直接决定公平性。daemon 必须根据本地 IPC endpoint、session registration、benchmark registration token 和请求来源计算内部 `client_class`，并把结果写入 redacted diagnostics。未注册客户端默认降级为 `background` 或拒绝，不能通过自报字段抢占 interactive 配额。

`deadline_ms` 是合同，不是 hint：daemon may return partial or overload instead of blocking beyond the deadline. `cancel_token` must be accepted by queued and active long-running work. Batch 通过独立 `resume-ir.search-batch-request.v1` wrapper 携带 `batch_id`，不得混入单请求 envelope。

## 2.1 Transport and Framing

1. 默认 transport 是本机 Unix domain socket 或 Windows named pipe；loopback TCP 只能作为显式开发模式。
2. 每条消息使用 length-prefixed UTF-8 JSON envelope，最大 request body 是 65536 bytes；超过限制返回 `REQUEST_TOO_LARGE`。
3. IPC payload 可以在内存中携带 interactive raw query，但 daemon 禁止把 raw query 写入日志、diagnostics、trace、benchmark summary 或 git。
4. benchmark batch 可在本机私有 IPC/pipe 中携带 query，但不得把 raw query 写入 diagnostics、trace、benchmark summary 或 git；公开/诊断证据只引用 `query_set_sha256`、request/sample counts、bucket aggregate 和 redaction flags。
5. cancel request 必须能取消 queued、running 和 batch child request；已完成请求返回 idempotent cancelled/complete 状态。
6. 单个 batch 最多 64 个 query references；interactive search child in-flight 上限
   8，Codex validation 上限 2，benchmark child 上限 8，background 上限 4；active
   benchmark batch 上限 1。
7. queued cancel acknowledgement P95 必须 <= 200ms；running cancel 必须在下一安全 checkpoint 之前进入 cancelled 或 complete。

## 2.2 Search Cancellation

`cancel_token` 是 1..=128 bytes opaque ASCII，只允许字母、数字、`-`、`_`、`.`。
active/terminal-history token 不得重复注册，否则返回 correlated `CONFLICT`：

```json
{
  "schema_version": "resume-ir.search-cancel-request.v1",
  "request_id": "cancel-command-id",
  "cancel_token": "opaque-cancel-token"
}
```

Response：

```json
{
  "schema_version": "resume-ir.search-cancel-response.v1",
  "request_id": "cancel-command-id",
  "status": "cancelled|cancel_requested|complete"
}
```

`cancelled` 表示 queued work 已移除或 terminal history 已记录取消；
`cancel_requested` 表示 running work 将在下一 checkpoint 返回 v3 cancelled
且零结果；`complete` 表示已完成或 token 不在有界 registry 内。unknown/expired
故意共用 `complete`，禁止探测 request 存在性。

## 2.3 Search Batch

Authenticated `POST /search/batch` 使用批次级结构合同：

```json
{
  "schema_version": "resume-ir.search-batch-request.v1",
  "batch_id": "bounded-opaque-id",
  "requests": [
    {
      "schema_version": "resume-ir.ipc-request.v3",
      "request_id": "child-request-1",
      "client_capability": "benchmark",
      "deadline_ms": 200,
      "cancel_token": "optional-child-cancel-token",
      "payload": {}
    }
  ]
}
```

冻结边界：

1. `batch_id`、child `request_id` 和 `cancel_token` 使用相同的 1..=128 bytes
   opaque ASCII 字符集；每批 1..=64 个 child。
2. 同一批次内 child `request_id` 必须唯一，非空 `cancel_token` 必须唯一，所有
   child 的 `client_capability` 必须一致。结构或 payload 校验失败时，在任何 child
   admission 之前整批 HTTP 400 fail closed。
3. 一次只允许一个 active batch；第二个并发 batch 立即返回 correlated HTTP 503
   `batch_admission_exhausted`，不能进入 child queue。
4. batch admission 不是 child admission bypass。每个 child 仍独立使用总 in-flight
   16 和 class 8/2/8/4 上限；超限 child 得到自己的 correlated `OVERLOADED`，不会
   保留 query task 或 cancel registration。
5. 所有 child deadline 从 batch request 到达的 monotonic instant 开始，覆盖整批
   parse、child queue、execute 和 response serialization。每个 admitted child 的
   cancel token 继续支持 queued/active/idempotent cancellation。
6. HTTP 200 batch response 是 completion-order NDJSON；`sequence` 恢复输入顺序，
   每个 child 恰好一行：

```json
{
  "schema_version": "resume-ir.search-batch-child-response.v1",
  "batch_id": "bounded-opaque-id",
  "sequence": 0,
  "http_status": 200,
  "response": {"schema_version": "resume-ir.search-response.v3"}
}
```

`response` 是该 child 原本会得到的 v3 success/cancel/deadline body 或
`resume-ir.error.v1` 有界错误 body；batch wrapper 不记录 raw query、候选结果、路径或 token。最后一个
child 完成后 server 关闭 response write half。这个 close-delimited NDJSON transport
是当前 loopback 开发接口，不是 `resume-ir.ipc-framing.v1` 完成证据。

## 3. Status Contract

```json
{
  "schema_version": "daemon.status.v2",
  "status": "ok",
  "process_state": "ready",
  "service_state": "ready",
  "services": {
    "metadata": "ready",
    "query": "ready"
  },
  "repair_reason": null,
  "error": null,
  "visible_epoch": 42,
  "indexed_documents": 10000,
  "searchable_documents": 8000,
  "partial_documents": 1200,
  "failed_retryable": 0,
  "failed_permanent": 0,
  "recovery_queue_depth": 0,
  "ocr_queue_depth": 1200,
  "embedding_queue_depth": 800,
  "entity_mentions": 4500,
  "import_tasks_queued": 0,
  "index_health": "ready",
  "latest_import_scan": null,
  "ipc": {
    "accepted": 8,
    "completed": 8,
    "client_disconnect": 0,
    "request_failure": 0,
    "response_failure": 0
  }
}
```

`repair_reason` 是 `daemon.status.v2` 的 **required-nullable** 字段：每个 response
都必须包含该 key；无修复原因时值为 `null`，不得通过省略字段表达 `null`。允许的
非空枚举只有 `migration_rebuild`、`artifact_unavailable`、
`source_unavailable`、`runtime_invariant`。服务状态与原因的合法组合固定为：

1. `metadata=ready, query=ready`：`repair_reason=null`，aggregate
   `service_state=ready`，`status=ok`，`error=null`。
2. `metadata=ready, query=repairing`：原因只能是 `migration_rebuild` 或
   `artifact_unavailable`，aggregate `service_state=repairing`，`status=repairing`，
   error 固定为 `REPAIRING/wait_for_repair`。
3. `metadata=ready, query=unavailable`：原因只能是 `source_unavailable` 或
   `runtime_invariant`，aggregate `service_state=degraded`，`status=degraded`，
   error 固定为 `QUERY_SERVICE_UNAVAILABLE/retry`。
4. `metadata=unavailable, query=unavailable`：`repair_reason=null`，aggregate
   `service_state=degraded`，`status=degraded`，error 固定为
   `METADATA_UNAVAILABLE/retry`；corpus counts、epoch 和 index health 可以为
   `null`，但 process health 与 IPC counters 仍必须可读。

其他 service/reason 组合必须被 daemon/Tauri bridge fail closed，不得映射成通用
“修复中”。

## 4. Search Contract

Search response 必须说明每个查询是否 partial，以及哪些 layer 参与：

```json
{
  "schema_version": "resume-ir.search-response.v3",
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
  "result_count": 1,
  "results": [
    {
      "rank": 1,
      "selection": {
        "doc_id": "doc_<32-hex>",
        "version_id": "ver_<32-hex>",
        "visible_epoch": 42
      },
      "file_name": "redacted-name.pdf",
      "snippet": "bounded redacted snippet"
    }
  ]
}
```

`result_count` 必须与 `results` 长度完全一致；每个 hit 必须携带不可拆分的
`SearchSelection`，其 `visible_epoch` 必须与 response 顶层 epoch 一致。禁止平铺
`doc_id`/`version_id` 或通过 `doc_id -> latest` 补全版本。

每个 search result 的 `file_name` 必须先做 contact redaction，再按 UTF-8
边界截断到最多 160 bytes（省略号计入上限）。daemon、direct CLI 和 IPC
client 必须执行同一上限；该展示边界不得改变索引字段、排序或结果数量。

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

Local private benchmark runner request stream（不是 daemon `/search/batch` envelope）：

```jsonl
{"schema_version":"resume-ir.query-batch-request.v2","request_id":"private-query-request-1","query":"<local-private-query>"}
```

Overload response:

```json
{
  "schema_version": "resume-ir.error.v1",
  "request_id": "uuid",
  "status": "error",
  "error": {
    "code": "OVERLOADED",
    "action": "retry",
    "retry_after_ms": 250
  }
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

## 5.1 Local Detail/Hydrate Contract

默认 `POST /details` 的 `resume-ir.detail-response.v3` 只返回脱敏字段和短 snippet，
不得输出完整正文或本地路径。GUI 需要完整详情时使用独立、认证的
`POST /details/hydrate`。该 endpoint 只允许 loopback/本地 transport 和有效
daemon bearer token，不得用于 diagnostics、benchmark/public evidence 或默认
CLI stdout。

`resume-ir.detail-response.v3` 每个 immutable version 最多返回 256 个按持久化 mention 顺序排列的
脱敏结构化字段，并必须返回 `field_limit`、`field_count_total`、
`field_count_returned` 和 `fields_truncated`。本地 CLI 和 IPC client 使用同一
上限；IPC 成功 payload 超限或计数/截断元数据不一致时必须 fail closed，
不得继续解析或打印超限字段。

Request：

```json
{
  "schema_version": "resume-ir.detail-hydrate-request.v3",
  "request_id": "bounded-opaque-request-id",
  "selection": {
    "doc_id": "doc_<32-hex>",
    "version_id": "ver_<32-hex>",
    "visible_epoch": 42
  },
  "body_offset_bytes": 0,
  "body_limit_bytes": 32768
}
```

Response：

```json
{
  "schema_version": "resume-ir.detail-hydrate-response.v3",
  "request_id": "bounded-opaque-request-id",
  "selection": {
    "doc_id": "doc_<32-hex>",
    "version_id": "ver_<32-hex>",
    "visible_epoch": 42
  },
  "status": "ok",
  "document": {
    "body_page": {
      "encoding": "utf-8",
      "offset_bytes": 0,
      "next_offset_bytes": 32768,
      "total_bytes": 65536,
      "complete": false,
      "text": "bounded local-only page"
    }
  },
  "privacy": {
    "local_authenticated_only": true,
    "public_output_allowed": false
  }
}
```

Hard limits：

1. `body_limit_bytes` 范围是 4..=32768；cursor 必须位于 UTF-8 boundary。
2. 完整 response 最大 1048576 bytes，不得携带本地路径。
3. hydrate 只读取 selection 精确绑定的 immutable version `clean_text`，
   不读取 raw parser text，不触发 OCR、
   parse、classification、index merge 或其他写操作。
4. 无效配对、从未发布或已删除返回不泄露存在性的 404；目标 document
   已切换 active version 返回 409 `STALE_SELECTION`；
   错误不得回显 doc id、path、body、token 或本地数据目录。
5. GUI 通过 `next_offset_bytes` 读取至 `complete=true`；不得请求无上限正文并把
   corpus-size payload 注入 GUI/model-visible context。

所有带合法 request context 的 detail/hydrate success 和有界错误都必须回显精确
`request_id`；success 还必须回显完整 selection。错误不得回显 selection，避免泄露存在性。

## 6. Diagnostics Contract

Diagnostics 必须默认脱敏：

```json
{
  "schema_version": "resume-ir.diagnostics.v3",
  "privacy_boundary": "redacted_local_aggregate",
  "evidence_lane": "gui_manual",
  "evidence_status": "unaccepted",
  "contains_raw_resume_text": false,
  "contains_queries": false,
  "contains_resume_paths": false,
  "contains_candidate_results": false,
  "contains_snippet_text": false,
  "visible_epoch": 42,
  "process_state": "ready",
  "service_state": "ready",
  "services": {
    "metadata": "ready",
    "query": "ready"
  },
  "repair_reason": null,
  "error": null,
  "metrics": {
    "ipc": {
      "accepted": 8,
      "completed": 8,
      "client_disconnect": 0,
      "request_failure": 0,
      "response_failure": 0
    },
    "indexed_documents": 10000,
    "searchable_documents": 8000,
    "partial_documents": 1200
  },
  "error_counts": {
    "scan_error_buckets": []
  },
  "benchmark_refs": []
}
```

Diagnostics 只允许输出 aggregate metrics、bucket counts、hash/ref、redaction flags、error class、queue depth 和 redacted partial reason。不得输出 raw query、raw snippet、resume path、candidate identity、local capture path、token、cookie 或 diagnostics package 原文。

`repair_reason` 在 `resume-ir.diagnostics.v3` 中同样是 required-nullable，使用与
`daemon.status.v2` 完全相同的枚举和 service/reason 状态约束。metadata 不可用时
仍必须输出该 key 且值为 `null`；缺失 key、未知枚举或非法组合都必须由 Tauri
bridge 拒绝，而不是猜测状态。

## 7. GUI 依赖面

GUI 只能依赖：

1. status contract。
2. search contract。
3. detail contract。
4. task control contract。
5. diagnostics export contract。
6. benchmark summary contract。

GUI 不依赖内部 SQLite table、Tantivy field name、sidecar path、ANN implementation。

## 12. S807 v3 request, generation, and selection contract

`resume-ir.daemon-ipc.v2` discovery contains a random `instance_id` and explicit
`owner_mode`; authentication rotates per generation and is bound to that
instance. Desktop holds a native `ConnectionLease { generation, instance_id }`.
Generation drift returns a typed interruption and never silently replays a
business request.

Search/detail/hydrate use IPC v3. Every hit carries one inseparable
`SearchSelection { doc_id, version_id, visible_epoch }`; detail requests and
responses echo `request_id` plus the exact selection. Status remains available
when metadata is repairing and reports process/service health separately from
optional corpus counts. Bounded errors are fixed enums: malformed/range errors
are 400, invalid or unpublished pairs are non-disclosing 404, a replaced active
version is 409 `STALE_SELECTION` with action `refresh_search`, oversized output
is 413, and repairing/unavailable metadata or query service is 503.

Connection read/write, peer reset, timeout, auth rejection, route failure, and
single storage failure end only that request. Only listener failure, supervised
worker death, or a runtime invariant can produce `DaemonFatalError`. Responses
are bounded bytes written under timeout; sink failure is a counted
`ConnectionOutcome`, never a daemon exit. Diagnostics v3 reports saturating
accepted/completed/disconnect/request-failure/response-failure counters and
service state without raw errors, paths, queries, tokens, or bodies.

## 13. 2026-07-21 bootstrap and capability contract hard cut

This section supersedes the v2 discovery/auth/status and diagnostics v3 examples
above. There is no dual reader or negotiated downgrade.

- Discovery is `resume-ir.daemon-ipc.v3`; auth is
  `resume-ir.daemon-auth.v3`. Both carry the same 256-bit `launch_id` and
  independent daemon `instance_id`. A client connection is valid only for the
  supervisor generation, expected launch, instance, and rotating auth token.
- Status is `daemon.status.v3`; diagnostics is
  `resume-ir.diagnostics.v4`; aggregate IPC is `resume-ir.ipc.v4`; bounded
  business errors are `resume-ir.error.v2`. Search/detail/hydrate success
  responses remain v3 because their wire shape and selection semantics do not
  change.
- `process_state=ready` means only that the authenticated control plane is
  readable. `core.state` is one of
  `initializing|ready|repairing|degraded|blocked`. `optional_runtimes` contains
  exactly embedding, OCR, and classifier. `capabilities` contains exactly
  keyword search, detail, semantic search, hybrid search, text import, OCR
  import, and index publication.
- Runtime entries use `initializing|available|unavailable` and a closed reason
  set `missing|invalid|start_failed|not_configured`. Capability entries use
  `initializing|available|degraded|unavailable|blocked`. Store-backed aggregates
  are `null` until the store is ready; paths, tokens, manifests, and raw errors
  are never status or diagnostics fields.
- During bootstrap, authenticated status/diagnostics return 200 and every
  business route returns `SERVICE_INITIALIZING`. A permanent core blocker uses
  `SERVICE_BLOCKED`; an unavailable operation uses `CAPABILITY_UNAVAILABLE`.
  Each carries a closed action/capability/reason tuple. Hybrid may return lexical
  results with a bounded partial reason; `SEMANTIC_DISABLED` is reserved for a
  deliberate product-contract disable, not a missing or broken runtime.
- Status and heartbeat are generated from a bounded in-memory typed snapshot.
  They never open SQLite or initialize a runtime. Listener, token, launch ID,
  and instance ID remain stable across bootstrap-to-ready.

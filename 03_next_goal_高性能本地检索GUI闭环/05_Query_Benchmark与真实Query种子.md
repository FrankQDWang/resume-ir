# Query Benchmark 与真实 Query 种子

## 0. 2026-07-13 实现审计

当前仓库已经具备 query benchmark 的 schema、静态 query-set freezer、
tune/holdout 分层、7 个 bucket、redacted aggregate、stage histogram 和
`resume-ir-query-v2` resident-batch command harness；这些是可执行框架，不是已
冻结的生产 baseline。公开 synthetic smoke 只有 2 个 batch request，明确
`resident_daemon_observed=false`、`percentile_confidence=smoke` 和
`target_claim=not_evaluated`。本地 500-query D10K query set、同语义 resident
daemon baseline 和负载曲线尚未形成，不能用于 before/after 优化声明。

冻结前必须先消除下列实现漂移：

| Surface | 当前代码事实 | 结论 |
|---|---|---|
| query-set normalization | `core-domain` 对 freeze 输入执行 NFKC、逻辑 term 去重、4096-byte/16-term cap | implemented for freezer only |
| runtime keyword semantics | `search-planner` 仅压缩空白并删除内置 stopword；full-text 使用 Tantivy lenient parser，未启用 default conjunction | contract mismatch: runtime 不是冻结的 required-all 语义 |
| field filter | CLI/daemon 在 BM25/ANN 前通过 searchable entity lookup 求交，并在 hydrate 后复核 | implemented；仍要求非空文本 query |
| semantic | direct CLI/daemon 可用持久化 vector snapshot 和本地 embedding command 查询 | partial；每次 query 建立 command embedder并打开 vector index |
| hybrid | direct CLI/daemon 对 full-text 与 vector 结果做 RRF，再做 candidate fold | partial；继承 keyword 语义和 semantic runtime 问题 |
| benchmark resident batch | 一个 CLI 子进程顺序处理整批 JSONL，并报告每条 record 的 stage timing | command-resident only；不是 daemon IPC resident baseline |

因此不得先冻结 query set 再修 runtime semantics。下一步必须让 direct CLI、
daemon 和 benchmark execution 共用同一 canonical query semantics，并以
metamorphic tests 证明 required-all、explicit OR、phrase、field-filter subset、
NFKC、dedupe 和 size/term caps；完成后才能执行静态 freeze。

## 0.1 2026-07-14 Public synthetic query hot-path freeze

S751 已完成上述 runtime semantics 对齐。S752 的 fresh preflight 随后证明，当前
私有 `agent_query_replay` 仍不能严格冻结：没有 retained D10K-ready corpus，且
2,167 个允许的 source-search events 去重后只有 21 条唯一候选，7-bucket 覆盖
严重不足。该 500-query private gate 保持不变且继续阻塞；不得用 synthetic
query 替代它，也不得据此声明 W1、D10K、agent replay 或生产 workload 已验收。

为了在无真实用户历史的个人项目阶段建立可重复的工程起点，另行冻结公开
`query_hot_path` workload `resume-ir.public-synthetic-query-hot-path.v1`：

| Contract | Frozen value |
|---|---:|
| synthetic documents | 10,000 |
| unique query cycle | 500 |
| single_term | 50 |
| and_2 | 75 |
| and_3_5 | 150 |
| and_6_16 | 50 |
| field_filter | 75 |
| hybrid | 75 |
| semantic | 25 |
| allowed zero-result queries | 0 |

文档与 query 都由版本化确定性生成器产生；benchmark 的 redacted JSON 只公开
版本、规范规模、唯一数和 bucket aggregate，不输出逐条 query 或结果。任何
query 文本、分布或生成规则变化都必须升级 workload version，旧版 before/after
不得混用。规范验证命令是：

```bash
resume-benchmark synthetic-query --index-dir <redacted-temp-index> \
  --documents 10000 --queries 500 --top-k 10 --json
```

该命令仍是 in-process synthetic harness，不是 resident daemon/load baseline。
单次运行产生的 P95/QPS 只能证明 harness 可执行，不能冻结性能目标。下一 slice
必须在相同 workload/version、语义、top-k、H-tier 和资源预算下建立 resident
query baseline 与负载曲线；用户要求的 P95 降至初始可用 baseline 的 50% 以下、
稳定负载至少 2 倍，只能相对该 accepted baseline 判定，不能相对本节的一次性
smoke 数字判定。

## 0.2 2026-07-14 Resident public-synthetic before baseline

S756 新增 `resume-benchmark resident-query-load`，它先在空数据目录生成并发布
10,000 份确定性 synthetic 文档、完整 full-text snapshot、10,000 份 vector records
和 field-filter entities，再启动真实 release `resume-daemon` 并通过 authenticated
loopback `/search` 执行冻结 workload。正式方法固定为 30 秒 warmup、每个
closed-loop 并发点 5 次、每个 open-loop 容量点 5 次、top-k 10、30/70/100/120%
容量点，并以 scheduled-start-to-completion 计算 arrival latency。closed-loop QPS
使用包含尾部请求排空的真实 elapsed；跨重复连续推进与 500 互质的 query-cycle
permutation，不能反复只测 cycle 前缀。

报告 schema 固定为 `resume-ir.resident-query-load.v1`。输出只包含固定上限的
全局和 per-bucket P50/P95/P99、七阶段 latency/histogram、bucket/mode counts、
RSS/CPU aggregate 和隐私布尔值；不包含 query、候选结果、token 或路径。七个
代表 query 还必须在 warmup 前全部通过，保证 fulltext、field-filter、hybrid 和
semantic 产品路径真实可用。当前 IPC 明确是每请求新建并关闭连接，semantic/
hybrid 明确每 query 启动本地 embedding command；benchmark 不隐藏这些现状。

稳定容量定义在 baseline 前冻结为：实际吞吐至少达到 target QPS 的 95%，
arrival P95 不超过 1500ms，且所有 response/result contract 均一致。三次校准运行
分别因 closed-loop 分母/重复合同、per-bucket/semantic 覆盖不足和只建立 500 份
vector records 而被拒绝，均不能作为 before。最终 accepted public-synthetic
aggregate 是：

| Metric | Accepted before |
|---|---:|
| calibrated capacity | 2.831 QPS |
| stable capacity | 1.982 QPS |
| 30% arrival/service P95 | 1380.280 / 1374.130ms |
| 70% arrival/service P95 | 1387.538 / 1379.628ms |
| 100% arrival/service P95 | 1755.418 / 1747.631ms |
| 120% arrival/service P95 | 1902.207 / 1892.170ms |
| daemon RSS peak / H2 budget | 192.906 / 1536 MiB |
| host CPU mean / peak during measured phase | 44.990% / 82.979% |

70% 稳定点的 service P95 按 bucket 为：single-term 938.350ms、and-2
1003.099ms、and-3-5 615.417ms、and-6-16 161.743ms、field-filter 256.485ms、
hybrid 1392.958ms、semantic 1199.138ms。后续性能验收必须使用同一 workload、
语义、top-k、H2 预算和方法；每个有样本的主 bucket P95 目标不高于上述 before
的 50%，稳定容量目标至少 3.963 QPS。不得靠减少结果、改变 required-all/
field-filter 语义、跳过 semantic/hybrid 或放宽稳定容量条件达标。

该 baseline 的 `evidence_lane=smoke` 仅表示公开 synthetic 工程证据，不是
W1、D10K、agent replay、真实用户 workload、IPC readiness 或 GUI readiness。
本地报告保持 owner-only；公开仓库只记录本节的 redacted aggregate。
该 before 刻意保留开发机并行负载，不把其他进程消耗从 latency 中扣除；后续
after 必须继续报告 host/daemon CPU，并以 50% latency / 2x capacity 的大幅门槛
覆盖当前约数个百分点的跨运行波动，不能用单次微小变化作成功声明。

S757 将 vector snapshot 升级为 generation-bound v2，并在共享发布锁下
复用 resident HNSW。正式 after 的 calibrated/stable capacity 为 7.319/7.319
QPS，70% arrival/service/ANN P95 为 239.811/233.376/90.585ms。七个
bucket 中六个已低于 before 的 50%，但 single-term 572.567ms 仍高于
469.175ms 上限，因此只接受 ANN 瓶颈优化，不接受整体目标完成声明。

S758 复用 daemon metadata store 并按数据库 Ready generation 缓存 immutable
full-text reader 后，正式 after calibrated/stable capacity 达到
74.612/52.228 QPS，70% arrival/service P95 为 73.922/67.198ms。七个
bucket P95 为 60.317/56.138/49.608/29.750/28.560/78.916/71.793ms，
全部低于 S756 before 的 50%。该结果接受 initial resident query speed/load
目标，但不代表 IPC batch/cancel/deadline/overload 或 GUI readiness。

## 1. 来源边界

当前阶段只定义 `agent_query_replay` 静态基准。它只有一个允许来源：
`$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log` 中 `tool_called` /
`tool=source_search` 后紧跟的 source_search keyword query。

禁止读取或迁移：

1. 候选人结果卡。
2. 简历正文、公司、学校、姓名、电话、邮箱、履历片段。
3. 页面 trace、截图、浏览器状态、cookie、token。
4. 原始 artifact 文件路径。
5. 原始 query 文本进入 git。
6. JD、prompt、候选人资料、query history、search attempts、term pool、
   role、fingerprint、bucket 或本地字段派生 query。

## Agent Query Replay 静态基准

`agent_query_replay` 只使用 SeekTalent 真实运行中已经产生的 source search 查询，不从 JD、prompt、候选人资料或 trace 上下文构造 query。

`agent_query_replay` has exactly one allowed source: `$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log` + `tool_called` + `tool=source_search` 后紧跟的 source_search keyword query only。

Allowed source:

```text
source_root = $RESUME_IR_QUERY_ARTIFACT_ROOT
source_glob = **/runtime/trace.log
event_filter = tool_called
tool_filter = source_search
query_source = source_search keyword query only
query_extraction_version = trace_source_search_v1
```

`RESUME_IR_QUERY_ARTIFACT_ROOT` 必须指向本机 SeekTalent artifacts runs root；extractor 仍只能读取该 root 下的 `runtime/trace.log`，且只抽取 `tool_called` / `tool=source_search` 后紧跟的 source_search keyword query。

Forbidden sources:

```text
artifacts/benchmarks/*.jsonl job_description or hiring_notes
raw transcript
full prompt
candidate profile
resume text
file path
URL
provider payload
token
raw log line outside the source_search keyword query segment
debug blob
screenshot OCR
```

Query set 必须先从真实 `source_search` 调用中抽取候选，再筛选一组在 D10K 私有库上可用于稳定比较的固定集合。冻结时必须用当前本地 searchable corpus 校验候选并丢弃 zero-hit query；baseline gate 对冻结后的 benchmark 执行 `--max-zero-result-queries 0`。冻结后以 `query_set_sha256` 锁定。修改 extraction/redaction 规则必须生成新的 `query_set_sha256`，旧结果不得直接做 before/after 对比。

所有机器合同、公开 summary、issue/PR evidence 字段统一使用 `query_set_sha256`；不得使用 `query_set_hash` 作为字段名或别名。

公开/GitHub/git 中的 `query_set_sha256` 是 public-safe identifier，必须来自 redacted aggregate manifest、approved opaque manifest 或 HMAC-SHA256 opaque manifest。它不是 raw query text/list 的直接 SHA256。raw local query set hash 或 raw query set digest 只能作为本机私有证据，不能提交，不能写入 GitHub issue/PR。

当前 `resume-ir.query-set-summary.v2` 生成的 all/tune/holdout
`query_set_sha256` identifiers 必须使用 `resume-ir:query-set-summary:v2:*`
HMAC domain；旧 v1 domain 不能继续用于新的冻结 query set。

`RESUME_IR_LOCAL_EVIDENCE_DIR=<local-evidence-dir> RESUME_IR_QUERY_ARTIFACT_ROOT=<local-query-artifacts> resume-cli --data-dir <local-data-dir> benchmark-query-set preflight-agent-replay` 只能作为 freeze 前的本地可行性观测：它读取同一个 trace-root 后写入 `resume-ir.query-set-trace-preflight.v1` redacted aggregate artifact，包括 trace/source_search 计数、`query_index_available`、本地 corpus 的 `document_count` / `searchable_document_count` / `vector_indexed_document_count`、D10K corpus minimum/readiness/deficits、去重候选 query bucket counts/deficits，以及在当前本地 searchable corpus 可用时的 corpus-valid query bucket counts 和 D10K corpus-valid bucket deficits。缺少本地 search index 时，preflight 仍可输出 trace-only 候选覆盖，但 `freeze-agent-replay` 仍必须 fail closed，不能绕过 corpus-valid 过滤。它不得写 raw query set，不得在 stdout/stderr 或 artifact 输出 raw query、trace path、candidate result 或本机路径，不得生成或替代 `query_set_sha256`，也不得作为 D10K private baseline evidence。

`query_set_corpus_or_trace_coverage_insufficient` 必须同时看 corpus
observability 和 preflight bucket deficits：若 indexed corpus 本身不是
D10K-shaped，不能把 zero-hit 全部归因于 trace query 不足；下一步应先准备
D10K-shaped indexed local corpus，并补足 deficient buckets 的 trace-derived
`source_search` workload，再重新 freeze。

## 2. 本地私有输入

运行时通过环境变量提供私有输入，不把本机路径写入 repo 文档或证据：

```bash
export RESUME_IR_PRIVATE_RESUME_ROOT="<local private resume root>"
export RESUME_IR_DATA_DIR="<local private data dir>"
export RESUME_IR_QUERY_ARTIFACT_ROOT="<local SeekTalent artifacts root>"
export RESUME_IR_LOCAL_EVIDENCE_DIR="<local private evidence output>"
```

## 3. Query Set Schema

私有 query set 留在本机：

```json
{
  "schema_version": "resume-ir.query-set.jsonl.v2",
  "sample_id": "local-query-000001",
  "bucket": "single_term|and_2|and_3_5|and_6_16|field_filter|hybrid|semantic",
  "query": "<private local query text>",
  "source_kind": "trace_source_search_v1",
  "query_shape": {
    "term_count": 3,
    "has_boolean": false,
    "has_location": true,
    "has_years": false,
    "has_degree": true,
    "has_skill": true,
    "has_phrase": false
  }
}
```

This private query-set schema covers only the current `agent_query_replay`
static baseline. Every row must use `trace_source_search_v1`; other artifact
sources and local corpus-field query construction are outside this contract.

公开或提交的证据只允许包含：

```json
{
  "schema_version": "resume-ir.query-set-summary.v2",
  "privacy_boundary": "redacted_local_aggregate",
  "query_source": "trace_source_search_v1",
  "query_set_sha256": "<public-safe-sha256>",
  "tune_sha256": "<public-safe-sha256>",
  "holdout_sha256": "<public-safe-sha256>",
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
  "hmac_split": true,
  "contains_raw_query_text": false,
  "contains_raw_resume_text": false,
  "contains_candidate_results": false,
  "contains_local_paths": false
}
```

`benchmark-query-set freeze-agent-replay` 的默认 redacted summary 是
freeze/bootstrap artifact，
不是最终 benchmark evidence report。它默认写在 local-only JSONL 旁边，命名为
`<basename>.summary.json`；例如
`private-query-set.local.jsonl -> private-query-set.summary.json`。这个 summary
至少包含：

1. `schema_version`
2. `privacy_boundary`
3. `query_source`
4. `query_count`
5. `tune_query_count`
6. `holdout_query_count`
7. `bucket_counts`
8. `tune_bucket_counts`
9. `holdout_bucket_counts`
10. `candidate_queries_sampled`
11. `zero_hit_queries_dropped`
12. `query_set_sha256`
13. `tune_sha256`
14. `holdout_sha256`
15. `hmac_split=true`
16. all `contains_* = false` privacy booleans

后续真正的 benchmark evidence/report 仍必须补齐 `request_sample_count`、
`bucket_counts`、latency/resource aggregates，以及同一语义版本下的 holdout
aggregate；freeze summary 不能替代完整 benchmark report。

`resume-benchmark private-query` 必须读取 `resume-ir.query-set.jsonl.v2`
metadata，拒绝旧 query-set 行，并在 redacted private benchmark report 中输出与
冻结 query-set 相加一致的 `bucket_counts`、与实际执行请求数量相加一致的
`request_sample_count`/`samples_per_bucket`。每个 `query` 字段必须已经是
freeze 输出的 canonical query；runner 必须用共享 query-set canonicalizer 校验
JSONL 字符串等于 canonical 输出，拒绝未规范化或重复逻辑词的手写行。Full
private baseline 必须通过
runner 的 per-bucket request floor 执行，不能用事后手写
`samples_per_bucket` 冒充 bucket 覆盖；D10K private calibration 还必须让冻结
static query set 自身满足 `perf/acceptance-matrix.toml` 的 per-bucket query
minimums，重复执行少量 query 不能替代静态集合覆盖。Report 还必须输出每个 query 子命令返回的
`query_parse`、`prefilter`、`bm25`、`ann`、`fusion`、`bulk_hydrate`、`snippet`
阶段耗时聚合 `stage_latency_ms`。每个 query record 还必须返回该 query 的
`elapsed_ms`，report 的 `query_latency_ms` 必须来自这些逐条记录，不能用 batch
总耗时除以 query 数量伪造 per-query percentile。Report 还必须按
`samples_per_bucket` 输出 `query_latency_by_bucket`、`stage_latency_by_bucket_ms`
、`stage_histogram_ms`、`stage_histogram_by_bucket_ms` 和
`rss_delta_mb_by_bucket` redacted aggregate；stage histogram 必须使用固定
upper-bound bins 和 `overflow_count`，不得按 query 数量扩张。有执行样本的
bucket 必须有对应 latency/stage/RSS summary，零样本 bucket 不得伪造 summary。
该 report 仍不得输出 raw query、sample ID、query-set path 或本机路径。Full
current-stage evidence manifest 只能复制该 report 的公开安全聚合子集：
`private_query_observability` 记录 counts、bucket samples、P50/P95/P99、全局
stage P95、per-bucket stage P95、bounded `stage_histogram_ms` /
`stage_histogram_by_bucket_ms`、`rss_delta_mb`、`rss_delta_mb_by_bucket` 和
`zero_result_queries`，不得嵌入 report body、raw query、候选结果或路径。

Sibling `private-query-set.summary.json` 必须精确描述本次加载并测量的 JSONL：
`query_count` 和每个 `bucket_counts` entry 必须与 runner 解析出的 query-set
一致；多 query baseline 还必须有非空 tune/holdout split，且
`tune_query_count + holdout_query_count == query_count`。Summary 还必须输出
`tune_bucket_counts` 和 `holdout_bucket_counts`，每个 bucket 的 tune+holdout
数量必须等于 `bucket_counts`；任一数量大于 1 的 bucket 必须在 tune 和 holdout
两侧都有样本，避免后续调参只覆盖某些 query shape。`tune_sha256` 与
`holdout_sha256` 必须是 public-safe SHA-256 identifiers。Summary 不允许描述
超集、子集、无 holdout 的可调参 workload、不同 bucket distribution 或未分层
tune/holdout workload，否则 baseline 必须 fail closed，避免 `query_set_sha256`
指向的 redacted summary 与实际测量 workload 脱钩。
`resume-benchmark private-query --max-queries` 是静态 query set 的上限，不是
前缀截断开关；JSONL 非空行数超过该上限时必须 fail closed，不能忽略尾部 query。
静态 JSONL 中的 query 字符串必须唯一；重复 query 不能计入 `query_count` 或
bucket 覆盖，也不能通过重复执行冒充静态集合覆盖。`sample_id` 是静态集合内的
本地记录身份，也必须唯一；重复 `sample_id` 不能进入 benchmark 执行。

#53 baseline groundwork uses a single resident batch command, not per-query
process spawn: `resume-benchmark private-query` writes local-only batch JSONL
scratch, starts the batch-only `resume-cli benchmark-query-protocol --batch-jsonl`
surface once, reads one `resume-ir-query-v2 ... resume-ir-query-end` record per
query, and emits `query_runner=resident-batch-command` plus
`spawn_per_query=false`. Each batch JSONL request carries a local opaque
`request_id`, and request IDs must be unique within the batch before regular
file batches run. Each protocol record must echo the matching `request_id`; the
runner accepts out-of-order records from a concurrent resident batch command,
then reorders them by the original execution plan before attributing latency or
hits to the frozen static query set. Missing, duplicated, unknown, or
mismatched record bindings fail closed before aggregation. The protocol no
longer accepts the old `RESUME_IR_QUERY_INPUT_PATH` single-query API and does
not create per-query query temp files. This is the benchmark harness execution
surface for the frozen query set; it still does not claim W1/D10K resident
daemon acceptance until the daemon baseline, profiler, histogram, and private
evidence gates are run.

## 4. Buckets

| Bucket | 目标 | 示例形态 |
|---|---|---|
| single_term | 验证最短全文主路径和冷/热 term | 1 个技术、岗位、行业或地点词 |
| and_2 | 验证 simple text required-all 双 term | 2 个技术/岗位/地点词 |
| and_3_5 | 验证常见多 term AND query | 3-5 个技能、岗位、行业、地点、学历词 |
| and_6_16 | 验证长 AND query 和候选收缩 | 6-16 个组合词 |
| field_filter | 验证字段前置过滤 | 技能 + 地点 + 学历/年限 |
| hybrid | 验证 BM25 + field + semantic/fusion | 多 term + 条件 + 语义表达 |
| semantic | 验证 semantic recall 和 partial | 自然语言能力描述 |

## 5. 查询语义冻结

性能优化前冻结以下业务语义：

1. simple text query 使用空格分隔词项时，默认语义是 required-all，即所有规范化词项全部必须参与匹配。
2. OR 只能由显式布尔语法、显式 mode 或 GUI 明确选项触发。
3. quoted phrase 是短语约束，不等同于普通 token 拆分。
4. 字段过滤是 hard filter，必须先于 ranking、fusion、rerank 和 snippet 执行。
5. 空 query、超长 query、互斥 query、极冷词 query 必须有有界响应和可解释 partial/zero-result 状态。
6. benchmark 调优不得改变 simple text、phrase、field filter、explicit OR 的语义。
7. query input 先做 Unicode NFKC normalization，再 dedupe normalized duplicate terms；最大输入为 4096 bytes、16 terms。
8. stopword/synonym/stemming/typo/semantic expansion 默认关闭，除非进入新的显式语义版本。

Stopword、synonym、stemming、typo expansion 和 semantic expansion 都不是 simple text 默认语义的一部分。若后续产品要支持，必须作为显式 mode 或独立语义版本进入 spec/plan/review，不能在性能优化中静默启用。

文档层验收：

| Check | 候选集合公式 | 期望 |
|---|---|---|
| term reorder | `C(a b c) == C(c b a)` | simple terms 重排后候选集合完全一致；ranking 可不同但必须解释 |
| add required term | `C(a b c) subset C(a b)` | 加 required term 后候选集合不得变大 |
| explicit OR | `C(a OR b)` may superset `C(a b)` | 只有显式 OR 可以扩大 simple term 匹配 |
| phrase | `C("a b") subset C(a b)` | phrase 是更强约束，不得扩大候选集合 |
| field filter | `C(a b + filter:x) subset C(a b)` | 增加 hard filter 后候选集合不得变大 |
| zero/partial | `C(q)` bounded | 空、超长、互斥和极冷词必须返回有界、可解释结果 |
| smoke vs W1 | lane invariant | smoke 结果不得声称完整 500-query baseline |

这些 checks 必须在 P2 query semantics implementation 中成为 metamorphic tests；没有通过前，不允许进入热路径优化。

## 6. 生成策略

`agent_query_replay` static freeze strategy:

1. Read exactly `$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log`.
2. Filter exactly `event_filter = tool_called` and `tool_filter = source_search`.
3. Extract exactly the source_search keyword query immediately after `tool=source_search`.
4. Do not read query fields, terms arrays, query history, search attempts, term pools, roles, fingerprints, buckets, local fields, or synthetic edge queries.
5. Output local-only frozen query set plus redacted summary locked by `query_set_sha256`.
6. `resume-benchmark private-query` must read `query_set_sha256` from that sibling redacted summary; callers must not pass a separate query-set hash.
7. D10K private calibration requires every query row and summary to use `trace_source_search_v1`.

Current local static freeze path:

```bash
RESUME_IR_LOCAL_EVIDENCE_DIR=<local-evidence-dir> RESUME_IR_QUERY_ARTIFACT_ROOT="$RESUME_IR_QUERY_ARTIFACT_ROOT" resume-cli --data-dir <local-data-dir> benchmark-query-set freeze-agent-replay \
  --max-queries 500 \
  --min-queries 500
```

For `freeze-agent-replay`, omitting `--min-queries` defaults the minimum to
`--max-queries`; agent replay freezes therefore fail closed instead of freezing
an undersized query set by accident.

For the full D10K `agent_query_replay` freeze (`--max-queries 500
--min-queries 500`), this is a one-shot static selection from the existing
allowed trace artifacts, not a continuing SeekTalent query ingestion pipeline.
Passing `--max-queries 500` with a lower explicit `--min-queries` is rejected;
D10K agent replay freeze cannot be downgraded into a partial query set.
It also requires the selected local indexed corpus to be D10K-shaped before it
scans or writes the static query set: `document_count >= 10000`,
`searchable_document_count >= 8000`, and
`vector_indexed_document_count >= 8000`. When that corpus readiness check fails,
`freeze-agent-replay --max-queries 500` fails closed before writing local JSONL,
summary, or `query_set_sha256`; the trace preflight artifact remains the
redacted place to inspect corpus deficits. Smaller non-500 smoke freezes may
exercise static query-set wiring, but they cannot be used as D10K private
baseline evidence.
It continues scanning corpus-valid `source_search` candidates until the frozen
set satisfies the D10K bucket minimums from `perf/acceptance-matrix.toml`:
50 `single_term`, 75 `and_2`, 150 `and_3_5`, 50 `and_6_16`, 75
`field_filter`, 75 `hybrid`, and 25 `semantic`. Zero-result trace queries are
dropped from the static set and only counted in the redacted dropped aggregate.
If the static freeze cannot meet the
bucket minimums, stderr may report only redacted bucket deficit counts such as
`and_3_5=150`; it must not print raw query text, trace paths, or resume paths.
Freeze failures may also report redacted trace selection counters such as
`trace_logs`, `source_search_lines`, `extracted_queries`,
`normalization_rejected`, `duplicate_queries_dropped`,
`candidate_queries_sampled`, `zero_hit_queries_dropped`, and
`selected_queries` so operators can classify whether the static set is blocked
by missing trace data, privacy/shape filtering, duplicates, corpus misses, or
bucket coverage, without exposing the queries or paths.

This static freeze path must:

1. keep only queries extracted from `tool_called` + `tool=source_search` trace lines,
2. canonicalize each extracted query through the shared query-set domain
   semantics: NFKC, whitespace normalization, duplicate logical-term dedupe,
   and the frozen 4096-byte / 16-term cap,
3. drop zero-hit queries after validating them against the current local searchable corpus,
4. replace dropped samples only with other valid trace-derived queries,
5. write a sibling redacted summary named `<basename>.summary.json` with public-safe HMAC identifiers only,
6. keep raw query text inside the local JSONL only, never in stdout/stderr, git, or GitHub.

Full current-stage validation must be wired to an existing static local query
set, to explicit `--query-set-trace-root "$RESUME_IR_QUERY_ARTIFACT_ROOT"`, or
to `RESUME_IR_QUERY_ARTIFACT_ROOT` as the trace-root default. If all are absent,
it fails before query-set generation instead of falling back to
corpus-field query generation.

## 7. Benchmark 输出

每个 bucket 必须输出：

1. query_count。
2. request_sample_count。
3. zero_result_queries。
4. partial_queries。
5. p50/p95/p99。
6. per-record elapsed_ms source for latency percentile。
7. query_parse_ms。
8. prefilter_ms。
9. bm25_ms。
10. ann_ms。
11. fusion_ms。
12. bulk_hydrate_ms。
13. snippet_ms。
14. rss_delta_mb。
15. hot_path_ocr=false。
16. hot_path_parsing=false。
17. hot_path_heavy_model_inference=false。
18. spawn_per_query=false。

## 8. 验收红线

1. 任何 stdout/stderr、summary、diagnostics 中出现 raw query 内容，失败。
2. 任何提交文件中出现 private artifact path、resume path、candidate text，失败。
3. 500-query benchmark 不足 500 条时，不得声称任何 scale gate accepted；D1M 完成声明还必须有至少 25000 request samples。
4. smoke profile 可以小样本，但必须标明 `percentile_confidence=smoke`。
5. W1 私有 query set 必须分为 tune 与 holdout，公开证据只提交 public-safe tune/holdout identifiers、count 和 bucket count。
6. 调参只能读取 tune aggregate；最终完成声明必须包含 holdout aggregate，并通过同一语义版本。
7. 任一 query extractor version、public-safe artifact_input_id/manifest_id、query semantics version 或 public-safe dataset_manifest_sha256 变化，都必须生成新的 baseline；raw local artifact/dataset digest/hash 只能留本机，不得作为公开 drift evidence；新旧 baseline 不得混报。

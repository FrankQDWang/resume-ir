# Query Benchmark 与真实 Query 种子

## 1. 来源边界

本节的广义 query seed / local field / synthetic edge 规则只适用于非 `agent_query_replay` 的本地 query 种子构造。它不得用于解释、扩展或覆盖 `agent_query_replay` 静态基准。

`agent_query_replay` has exactly one allowed source: `$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log` 中的 `tool_called` / `source_search` invocation argument only。

除 `agent_query_replay` 外，SeekTalent artifacts 只作为 query 和 query 组合形态的原始材料来源。

允许读取：

1. query 字符串字段。
2. query terms 数组。
3. keyword query 字段。
4. sent query history 中的 query 形态。
5. search attempts 中的 keyword/query 形态。
6. 组合 query 的 term pool、role、fingerprint 和 bucket 信息。

禁止读取或迁移：

1. 候选人结果卡。
2. 简历正文、公司、学校、姓名、电话、邮箱、履历片段。
3. 页面 trace、截图、浏览器状态、cookie、token。
4. 原始 artifact 文件路径。
5. 原始 query 文本进入 git。

## Agent Query Replay 静态基准

`agent_query_replay` 只使用 SeekTalent 真实运行中已经产生的 source search 查询，不从 JD、prompt、候选人资料或 trace 上下文构造 query。

`agent_query_replay` has exactly one allowed source: `$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log` + `tool_called` + `source_search` invocation argument only。

Allowed source:

```text
source_root = $RESUME_IR_QUERY_ARTIFACT_ROOT
source_glob = **/runtime/trace.log
event_filter = tool_called
tool_filter = source_search
query_source = source_search invocation argument only
query_extraction_version = trace_source_search_v1
```

`RESUME_IR_QUERY_ARTIFACT_ROOT` 必须指向本机 SeekTalent artifacts runs root；extractor 仍只能读取该 root 下的 `runtime/trace.log`，且只抽取 `tool_called` / `source_search` invocation argument。

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
raw log line outside the source_search invocation
debug blob
screenshot OCR
```

Query set 必须先从真实 `source_search` 调用中抽取候选，再筛选一组在 D10K 私有库上可用于稳定比较的固定集合。少量 zero-result query 可以保留为单独 bucket；benchmark 不能被大量搜不到人的 query 主导。冻结后以 `query_set_sha256` 锁定。修改 extraction/redaction 规则必须生成新的 `query_set_sha256`，旧结果不得直接做 before/after 对比。

所有机器合同、公开 summary、issue/PR evidence 字段统一使用 `query_set_sha256`；不得使用 `query_set_hash` 作为字段名或别名。

公开/GitHub/git 中的 `query_set_sha256` 是 public-safe identifier，必须来自 redacted aggregate manifest、approved opaque manifest 或 HMAC-SHA256 opaque manifest。它不是 raw query text/list 的直接 SHA256。raw local query set hash 或 raw query set digest 只能作为本机私有证据，不能提交，不能写入 GitHub issue/PR。

## 2. 本地私有输入

运行时通过环境变量提供私有输入，不把本机路径写入 repo 文档或证据：

```bash
export RESUME_IR_PRIVATE_RESUME_ROOT="<local private resume root>"
export RESUME_IR_QUERY_ARTIFACT_ROOT="<local SeekTalent artifacts root>"
export RESUME_IR_LOCAL_EVIDENCE_DIR="<local private evidence output>"
```

## 3. Query Set Schema

私有 query set 留在本机：

```json
{
  "schema_version": "resume-ir.query-set.jsonl.v2",
  "sample_id": "local-query-000001",
  "bucket": "single_term|and_2|and_3_5|and_6_16|field_filter|hybrid|semantic|extreme",
  "query": "<private local query text>",
  "source_kind": "artifact_query|artifact_terms|local_field|synthetic_edge",
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

This private query-set schema covers broad local seed sets. `local_field` and `synthetic_edge` are not valid for `agent_query_replay`; `agent_query_replay` samples must come only from `trace_source_search_v1` extraction of `source_search` invocation arguments. Generic artifact sources such as query history, search attempts, term pools, roles, fingerprints, or buckets are not valid `agent_query_replay` sources.

公开或提交的证据只允许包含：

```json
{
  "schema_version": "resume-ir.query-set-summary.v1",
  "privacy_boundary": "redacted_local_aggregate",
  "query_set_sha256": "<public-safe-sha256>",
  "query_count": 500,
  "request_sample_count": 25000,
  "bucket_counts": {
    "single_term": 50,
    "and_2": 75,
    "and_3_5": 125,
    "and_6_16": 50,
    "field_filter": 75,
    "hybrid": 75,
    "semantic": 25,
    "extreme": 25
  },
  "contains_raw_query_text": false,
  "contains_raw_resume_text": false,
  "contains_candidate_results": false,
  "contains_local_paths": false
}
```

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
| extreme | 验证超长、互斥、冷词、错别字、空结果 | 不要求高召回，但要求不拖垮 |

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

This generic generation strategy is for broad local query seed sets only. It does not apply to `agent_query_replay`.

1. 从 artifacts 读取允许字段。
2. 提取 query 形态和 term 组合。
3. 删除包含邮箱、手机号、路径、URL、身份证、长数字串的样本。
4. 对每条 query 计算仅用于本地去重的 hash 和 shape；该本地 hash 不得作为公开 `query_set_sha256`。
5. 去重、分桶、采样。
6. 与本地字段派生 query 合并。
7. 输出本地私有 query-set 和可提交 redacted summary。

`agent_query_replay` generation strategy:

1. Read exactly `$RESUME_IR_QUERY_ARTIFACT_ROOT/**/runtime/trace.log`.
2. Filter exactly `event_filter = tool_called` and `tool_filter = source_search`.
3. Extract exactly the `source_search` invocation argument.
4. Do not read generic query fields, terms arrays, query history, search attempts, term pools, roles, fingerprints, buckets, local fields, or synthetic edge queries.
5. Output local-only frozen query set plus redacted summary locked by `query_set_sha256`.

## 7. Benchmark 输出

每个 bucket 必须输出：

1. query_count。
2. request_sample_count。
3. zero_result_queries。
4. partial_queries。
5. p50/p95/p99。
6. query_parse_ms。
7. prefilter_ms。
8. bm25_ms。
9. ann_ms。
10. fusion_ms。
11. bulk_hydrate_ms。
12. snippet_ms。
13. rss_delta_mb。
14. hot_path_ocr=false。
15. hot_path_parsing=false。
16. hot_path_heavy_model_inference=false。
17. spawn_per_query=false。

## 8. 验收红线

1. 任何 stdout/stderr、summary、diagnostics 中出现 raw query 内容，失败。
2. 任何提交文件中出现 private artifact path、resume path、candidate text，失败。
3. 500-query benchmark 不足 500 条时，不得声称任何 scale gate accepted；D1M 完成声明还必须有至少 25000 request samples。
4. smoke profile 可以小样本，但必须标明 `percentile_confidence=smoke`。
5. W1 私有 query set 必须分为 tune 与 holdout，公开证据只提交 public-safe tune/holdout identifiers、count 和 bucket count。
6. 调参只能读取 tune aggregate；最终完成声明必须包含 holdout aggregate，并通过同一语义版本。
7. 任一 query extractor version、public-safe artifact_input_id/manifest_id、query semantics version 或 public-safe dataset_sha256 变化，都必须生成新的 baseline；raw local artifact/dataset digest/hash 只能留本机，不得作为公开 drift evidence；新旧 baseline 不得混报。

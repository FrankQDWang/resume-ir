# Query Benchmark 与真实 Query 种子

## 1. 来源边界

SeekTalent artifacts 只作为 query 和 query 组合形态的原始材料来源。

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
  "bucket": "keyword|field_filter|hybrid|semantic|extreme",
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

公开或提交的证据只允许包含：

```json
{
  "schema_version": "resume-ir.query-set-summary.v1",
  "privacy_boundary": "redacted_local_aggregate",
  "query_set_sha256": "<sha256>",
  "sample_count": 500,
  "bucket_counts": {
    "keyword": 150,
    "field_filter": 100,
    "hybrid": 150,
    "semantic": 75,
    "extreme": 25
  },
  "contains_queries": false,
  "contains_resume_text": false,
  "contains_candidate_results": false,
  "contains_paths": false
}
```

## 4. Buckets

| Bucket | 目标 | 示例形态 |
|---|---|---|
| keyword | 验证全文主路径 | 2-5 个技术/岗位/行业词 |
| field_filter | 验证字段前置过滤 | 技能 + 地点 + 学历/年限 |
| hybrid | 验证 BM25 + field + semantic/fusion | 多 term + 条件 + 语义表达 |
| semantic | 验证 semantic recall 和 partial | 自然语言能力描述 |
| extreme | 验证超长、互斥、冷词、错别字、空结果 | 不要求高召回，但要求不拖垮 |

## 5. 查询语义冻结

性能优化前冻结以下业务语义：

1. simple text query 使用空格分隔词项时，默认语义是 required-all，即非停用词全部必须参与匹配。
2. OR 只能由显式布尔语法、显式 mode 或 GUI 明确选项触发。
3. quoted phrase 是短语约束，不等同于普通 token 拆分。
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

## 6. 生成策略

1. 从 artifacts 读取允许字段。
2. 提取 query 形态和 term 组合。
3. 删除包含邮箱、手机号、路径、URL、身份证、长数字串的样本。
4. 对每条 query 计算本地 hash 和 shape。
5. 去重、分桶、采样。
6. 与本地字段派生 query 合并。
7. 输出本地私有 query-set 和可提交 redacted summary。

## 7. Benchmark 输出

每个 bucket 必须输出：

1. query_count。
2. zero_result_queries。
3. partial_queries。
4. p50/p95/p99。
5. query_parse_ms。
6. prefilter_ms。
7. bm25_ms。
8. ann_ms。
9. fusion_ms。
10. bulk_hydrate_ms。
11. snippet_ms。
12. rss_delta_mb。
13. hot_path_ocr=false。
14. hot_path_parsing=false。
15. hot_path_heavy_model_inference=false。

## 8. 验收红线

1. 任何 stdout/stderr、summary、diagnostics 中出现 raw query 内容，失败。
2. 任何提交文件中出现 private artifact path、resume path、candidate text，失败。
3. 500-query benchmark 不足 500 条时，不得声称完整性能基线完成。
4. smoke profile 可以小样本，但必须标明 `percentile_confidence=smoke`。

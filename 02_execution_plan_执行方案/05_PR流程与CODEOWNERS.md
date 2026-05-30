# PR 流程与 CODEOWNERS

## 1. 分支保护

`main` 分支规则：

1. 禁止直接 push。
2. 必须通过 PR。
3. 必须通过 required checks。
4. 必须至少 1 名 reviewer。
5. 命中 CODEOWNERS 的路径必须 owner approval。
6. 必须线性历史或 squash merge。
7. 过期 review 在新 commit 后失效。
8. 管理员也不绕过规则，除非紧急修复并补审计。

## 2. PR 类型

| 类型 | 前缀 | 要求 |
|---|---|---|
| 功能 | `feat:` | 关联 issue、测试、文档 |
| 修复 | `fix:` | 复现用例、回归测试 |
| 性能 | `perf:` | benchmark 对比 |
| 重构 | `refactor:` | 行为不变说明 |
| 文档 | `docs:` | 不要求性能测试 |
| 构建 | `build:` | CI 和发布影响说明 |
| 安全 | `sec:` | 安全 owner 审核 |
| 实验 | `exp:` | 不直接进 main，除非转正 |

## 3. PR 模板

```md
## 变更摘要

## 关联 issue / ADR

## 影响范围
- [ ] 查询路径
- [ ] 导入路径
- [ ] OCR
- [ ] 模型
- [ ] 索引 schema
- [ ] 元数据 schema
- [ ] 安装包
- [ ] 安全/隐私

## 测试
- [ ] unit
- [ ] integration
- [ ] golden parser
- [ ] benchmark smoke
- [ ] Windows
- [ ] macOS

## 性能影响

## 数据迁移

## 回滚方案

## 截图/日志/诊断包（已脱敏）
```

## 4. Merge 策略

默认使用 squash merge。

规则：

1. PR 标题即 squash commit 标题。
2. 一个 PR 做一件事。
3. 大功能拆成多个可独立合并的 PR。
4. 不允许把大规模格式化和业务变更混在一起。
5. 性能变更必须带前后 benchmark。
6. schema 变更必须带迁移和回滚。

## 5. CODEOWNERS 示例

```text
# 全局默认
* @resume/core-maintainers

# 架构和 ADR
/docs/adr/ @resume/architecture
/docs/architecture/ @resume/architecture

# 核心领域模型
/crates/core-domain/ @resume/core-maintainers
/crates/config/ @resume/core-maintainers

# 摄取和解析
/crates/fs-crawler/ @resume/ingest
/crates/ingest-scheduler/ @resume/ingest
/crates/parser-common/ @resume/parsing
/crates/parser-docx/ @resume/parsing
/crates/parser-pdf/ @resume/parsing
/crates/parser-fallback/ @resume/parsing
/workers/doc-convert-worker/ @resume/parsing @resume/security

# OCR
/crates/ocr-client/ @resume/ocr
/workers/ocr-worker/ @resume/ocr @resume/platform

# 抽取和模型
/crates/extractor-rules/ @resume/extraction
/crates/extractor-model/ @resume/ml
/crates/embedder/ @resume/ml
/models/ @resume/ml @resume/security
/dictionaries/ @resume/extraction

# 存储与索引
/crates/meta-store/ @resume/storage
/crates/index-fulltext/ @resume/search
/crates/index-vector/ @resume/search @resume/ml
/crates/search-planner/ @resume/search
/crates/rank-fusion/ @resume/search
/crates/snippet/ @resume/search

# 安全与隐私
/crates/privacy/ @resume/security
/crates/diagnostics/ @resume/security @resume/platform
/SECURITY.md @resume/security

# API 与进程
/crates/ipc-api/ @resume/api
/proto/ @resume/api
/crates/daemon/ @resume/core-maintainers @resume/platform
/crates/cli/ @resume/api

# UI
/apps/desktop-ui/ @resume/ui

# 打包发布
/packaging/windows/ @resume/platform
/packaging/macos/ @resume/platform
/.github/workflows/ @resume/devops @resume/security

# 测试与基准
/tests/ @resume/qa
/benches/ @resume/performance
/tools/benchmark-runner/ @resume/performance
```

## 6. Review checklist

### 6.1 通用 checklist

1. 是否破坏跨平台路径？
2. 是否在查询路径加入慢操作？
3. 是否引入敏感日志？
4. 是否支持取消和超时？
5. 是否有错误码和可恢复状态？
6. 是否有测试？
7. 是否需要文档或 ADR？

### 6.2 查询路径 checklist

1. 是否只读？
2. 是否有 query budget？
3. 是否可能阻塞索引写入锁？
4. 是否只对 topN 做 snippet？
5. 是否影响 P95/P99？

### 6.3 导入路径 checklist

1. 是否可断点续跑？
2. 是否可取消？
3. 是否支持失败重试？
4. 是否有背压？
5. 是否避免一次性读入大文件？

### 6.4 OCR checklist

1. 是否只在需要 OCR 时触发？
2. 是否有页级超时？
3. 是否缓存？
4. 是否清理临时文件？
5. 是否不会阻塞查询？

### 6.5 安全 checklist

1. 是否写了 PII 到日志？
2. 是否导出了原文？
3. 是否新增第三方依赖？
4. 是否需要 license 审查？
5. 是否影响密钥或加密？

## 7. 例外流程

紧急修复可走 hotfix：

1. 从 main 拉 `fix/urgent-xxx`。
2. 最小变更。
3. 至少一名 owner 同步审核。
4. CI 必跑。
5. 合并后补完整 RCA。
6. 发布 patch。

不允许以“紧急”为由跳过安全和隐私检查。

## 8. ADR 触发条件

以下变更必须写 ADR：

1. 更换全文索引引擎。
2. 更换向量索引引擎。
3. 更换主语言或进程模型。
4. 查询路径增加模型推理。
5. 元数据 schema 大改。
6. 加密策略变化。
7. 跨平台支持范围变化。
8. 引入重型常驻依赖。

# GOAL

建设一个**本地优先、百万级简历、跨 Windows/macOS、低配机器可降级运行**的高性能简历检索内核。

核心目标：

1. 支持 Word、PDF、扫描 PDF、图片型 PDF 的本地导入、解析、OCR、结构化抽取和检索。
2. 查询热路径只做检索、过滤、融合、片段返回；不做 OCR、全文解析、重模型推理。
3. 采用倒排索引、结构化字段索引、向量索引分层设计，支持关键词、字段、语义、混合检索。
4. 对百万级简历库，热索引状态下以 P95 混合查询 `<200ms` 作为工程目标；低配机器允许关闭 OCR/向量/重排序以保稳定。
5. 全量数据、索引、日志、缓存默认本地保存；敏感信息默认脱敏、加密、可删除、可审计。

## 当前阶段边界

当前阶段不要求极致性能优化，也不因为真实 1 万简历 benchmark
延迟高而无限循环。当前完成线是：

1. 完成本地导入、增量管理、全文/字段/语义/混合检索的可用闭环，并
   生成可复现 current-stage dry-run、smoke、blocked handoff、观测指标和
   真实本机 1 万份本地验证流程。若 full 10k/8000 hot-index/500-query
   baseline 因 OCR backlog 或性能预算阻塞，记录 redacted aggregate
   blocked summary，不把它作为当前阶段收口 gate。
2. 保留指定目录扫描；全盘扫描视为用户把根目录或磁盘作为扫描根的同一能力。
3. OCR/PDF/model runtime 采用 bundled-first 方向：默认产品体验应尽量随
   安装包提供可审查 runtime，并保留 external override。Tesseract/tessdata
   可作为 Apache-2.0 OCR 方案；PDF renderer 优先评估可宽松分发的
   bundled 方案，Poppler/pdftoppm 可在 GPL-compatible license、
   source-offer、notice、checksum、SBOM 和 installer composition 审查完成后
   进入打包方案。
4. 完成 runtime manifest、checksum/license 记录、依赖检测、失败提示和
   runbook；不要把“未选择 OCR runtime”作为本阶段未知 blocker。
5. 完成 macOS/Windows install、upgrade、uninstall、rollback 的脚本、
   dry-run、CI evidence 和 runbook。
6. Signing 和 notarization 只完成自动化脚本、CI secret 接口、fail-closed
   gate 和文档；真实证书、开发者账号、私钥和 notarization credentials
   由人类后续提供。
7. Embedding runtime 必须走真实本地方案、manifest、checksum、license
   记录和失败闭环；若模型权重 license 未确认，标为 external/legal
   blocked，不伪造完成。

以上是上一 current-stage 收口边界，不是下一阶段性能目标的降级口径。full hot-index baseline 压实、500-query 私有 benchmark、百万级真实语料验证、P95/P99 压低、查询热路径极限调优和可视化 UI 已迁入下面的活跃后续目标。

## 当前活跃后续目标

当前活跃后续目标是 `03_next_goal_高性能本地检索GUI闭环/`：在本地隐私边界内完成高性能检索、GUI、手工/Codex 结对闭环验证的执行合同和后续实现。

机器可读目标锁是 `ACTIVE_GOAL.toml`；性能验收红线是 `perf/acceptance-matrix.toml`。两者只定义公开可提交的合同与 redacted aggregate 证据形状，不授权提交私有 benchmark 数据。

权威顺序：

1. 本文件定义产品级目标和阶段边界。
2. `ACTIVE_GOAL.toml` 锁定当前活跃目标、允许路径、隐私边界和 PR 状态。
3. `03_next_goal_高性能本地检索GUI闭环/` 定义该目标的系统设计、数据模型、状态机、失败模式和验收门槛。
4. `perf/` 定义可机器读取的验收矩阵、Loop 状态报告 schema 和实验报告 schema。
5. `docs/superpowers/` 记录本次目标文档修复的 spec 与 linked implementation plan。

该目标的硬边界：

1. 查询热路径只允许检索、过滤、融合、bulk hydrate 和 snippet 返回；不得触发 OCR、全文解析或重模型推理。
2. 简单空格 query 的业务语义在性能优化前冻结，不得为了降低延迟改变召回语义。
3. daemon IPC/diagnostics contract 必须先版本化，GUI 只能依赖版本化 contract。
4. benchmark 证据必须区分 smoke、W0、W1、本机私有、soak/fault 和 GUI/manual，不得混用。
5. 真实简历、raw query、候选结果、路径、token、trace、diagnostics package 和模型缓存不得提交。

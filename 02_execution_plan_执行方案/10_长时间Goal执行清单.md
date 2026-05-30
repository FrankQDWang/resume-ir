# 长时间 Goal 执行清单

本文用于把当前仓库的系统设计和执行方案交给一个长时间运行的 Codex Goal。目标是让执行者在无人值守时尽可能多完成工作，同时不越过安全、架构和发布边界。

核心原则：

1. 宏观方向由 `GOAL.md` 和两组设计文档决定。
2. 具体实现由 Goal 自主推进，但必须按 slice 顺序执行。
3. 每个 slice 都有独立验收标准，验收不通过不能宣称完成。
4. 遇到外部阻塞时记录阻塞、提交已通过的独立工作，然后继续下一个不依赖阻塞项的任务。
5. 早上复盘时，以提交记录、测试命令、运行日志和 checklist 状态为准。

## 1. 已确认默认边界

长时间 Goal 可以直接采用以下默认决策，不需要再向用户确认：

| 类别 | 默认决策 |
|---|---|
| 仓库 | 当前目录就是仓库根目录；若不存在 `.git`，先 `git init` 并提交现有文档基线 |
| 主路线 | Rust workspace + 本地 daemon + CLI + SQLite 元数据 + Tantivy 全文索引 |
| 执行顺序 | 先完成 P0/P1，再在验收通过后进入 P2/P3 的可独立切片 |
| 数据 | 只使用合成 fixture 和脱敏测试数据，不使用真实简历 |
| 网络 | 允许下载公开开源依赖；禁止上传用户数据、索引、日志、诊断包 |
| 测试 | 每个 slice 至少包含可运行测试或 smoke command |
| Git | 每个通过验收的 slice 单独提交；禁止 `git add -A`，只 stage 本 slice 文件 |
| 文档 | 如果实现偏离现有设计，必须更新 ADR 或执行文档说明原因 |
| 失败处理 | 最多连续 3 次尝试同一阻塞；仍失败则记录 blocker，转向不依赖该 blocker 的任务 |

## 2. 必须人工确认的边界

以下事项不能在无人值守 Goal 中自动决定：

1. 远程副作用：push、创建 PR、发布 release、上传 artifact、开启自动更新。
2. 证书和账号：Windows 签名证书、macOS notarization、GitHub secret、商业 license。
3. 商业或限制性模型：任何许可不清晰、需注册、需付费、禁止商用的模型或词典。
4. 真实数据接入：读取用户真实简历目录、导入第三方招聘系统数据、导出诊断包。
5. 数据删除：删除用户源文件、清空真实数据目录、不可逆迁移。
6. 目标变更：从本地优先改为云服务、从 Rust 主内核改为 Python/Java 常驻主进程。
7. 性能承诺变更：降低百万级或 P95 目标，只能记录未达标原因，不能改目标。

## 3. 无人值守硬性禁止

长时间 Goal 不能做这些事：

1. 不把查询、简历原文、日志或索引发到公网模型或公网搜索服务。
2. 不在查询热路径加入 OCR、全文解析、长文本大模型推理或全量索引合并。
3. 不为了赶进度跳过测试、跳过错误码、跳过取消/超时。
4. 不伪造 benchmark、质量评估或跨平台结果。
5. 不把明文手机号、邮箱、身份证号、简历原文写入日志、fixture 或提交信息。
6. 不把大规模格式化和功能变更混在同一个提交。
7. 不因为某个重依赖难接入就改成服务化 Elasticsearch/Milvus/Weaviate 主内核。

## 4. Slice 执行总览

Goal 应按下表从上到下推进。只有当前 slice 的验收通过后，才允许把它标为完成并提交。

| Slice | 目标 | 可并行性 | 完成后是否可继续 |
|---|---|---|---|
| S0 | 仓库和执行护栏 | 必须最先做 | 是 |
| S1 | Rust workspace 与基础工程骨架 | 顺序 | 是 |
| S2 | 核心领域模型、配置和错误模型 | 顺序 | 是 |
| S3 | SQLite 元数据和任务队列 | 依赖 S2 | 是 |
| S4 | daemon、CLI、状态查询和导入任务 skeleton | 依赖 S3 | 是 |
| S5 | 文件扫描、路径规范化、变更发现 | 依赖 S3/S4 | 是 |
| S6 | parser-common、docx 文本抽取、PDF 文本层判定 skeleton | 依赖 S2/S5 | 是 |
| S7 | 文本清洗、分段、基础字段规则 | 依赖 S6 | 是 |
| S8 | Tantivy 文件名/全文索引和 search CLI | 依赖 S6/S7 | 是 |
| S9 | 导入到查询闭环和崩溃恢复 smoke | 依赖 S3-S8 | 是 |
| S10 | P2 字段抽取和字段过滤 MVP | S9 通过后可做 | 是 |
| S11 | P3 语义检索 skeleton | S9 通过后可做，若模型许可不明只做接口和假实现 | 是 |
| S12 | P4 OCR 支路 skeleton | S9 通过后可做，默认只标记 OCR_REQUIRED，不跑重 OCR |
| S13 | 性能 smoke、故障注入、诊断包脱敏 | S9 通过后持续补 | 是 |

## 5. Slice 验收标准

### S0：仓库和执行护栏

交付：

1. 初始化 Git 仓库。
2. 提交当前文档基线。
3. 新增或更新 `README.md`，说明目标、当前阶段和常用命令。
4. 新增 `PROGRESS.md`，记录每个 slice 的状态、命令和 blocker。
5. 新增 `.gitignore`，覆盖 Rust、系统文件、测试产物和本地数据目录。

验收：

```bash
git status --short
git log --oneline -3
```

必须能看到文档基线提交，且工作区只包含当前 slice 有意变更。

### S1：Rust workspace 与基础工程骨架

交付：

1. 根 `Cargo.toml` workspace。
2. 最小 crate：`core-domain`、`config`、`meta-store`、`daemon`、`cli`。
3. 统一 Rust edition、lint 和测试命令。
4. CI skeleton 可以先落本地命令说明，GitHub workflow 可后续补。

验收：

```bash
cargo metadata --no-deps
cargo fmt --check
cargo test
```

### S2：核心领域模型、配置和错误模型

交付：

1. `Document`、`ResumeVersion`、`Candidate`、`Section`、`EntityMention`、`VectorRecord` 的 Rust 类型。
2. ID 类型或 newtype，避免字符串混用。
3. `ErrorKind` 和统一错误结构，包含 retryable、user_message、diagnostic_message、redaction_level、source_component。
4. Profile：Economy、Balanced、Turbo。

验收：

```bash
cargo test -p core-domain
cargo test -p config
```

测试必须覆盖 ID 生成、错误脱敏字段、profile 默认值。

### S3：SQLite 元数据和任务队列

交付：

1. SQLite schema v1。
2. migration runner。
3. document、resume_version、ingest_job、index_state 基础表。
4. job 状态更新、retryable job 查询、崩溃后恢复查询。

验收：

```bash
cargo test -p meta-store
```

测试必须覆盖 migration 幂等、任务状态恢复、删除标记默认查询不可见。

### S4：daemon、CLI、状态查询和导入任务 skeleton

交付：

1. `resume-cli status`。
2. `resume-cli import --root <path>` 提交导入任务。
3. `resume-cli search <query>` 可以在无索引时返回明确错误或空结果。
4. daemon lifecycle skeleton，至少支持前台运行。

验收：

```bash
cargo run -p resume-cli -- status
cargo run -p resume-cli -- import --root tests/fixtures/empty
cargo run -p resume-cli -- search "Java"
```

命令不能 panic；错误必须有用户可读信息。

### S5：文件扫描、路径规范化、变更发现

交付：

1. 扫描目录，过滤临时文件和不支持扩展名。
2. 规范化路径，支持中文路径和 macOS/Windows 分隔符差异。
3. 快速指纹：路径、大小、mtime、头尾采样或可替代实现。
4. 文件锁定、权限不足、外接盘不可达的错误状态。

验收：

```bash
cargo test -p fs-crawler
```

测试必须覆盖中文路径、同名文件、临时文件过滤、权限/不可访问模拟。

### S6：解析 skeleton、docx 文本抽取、PDF 文本层判定

交付：

1. `parser-common` 的 `Parser` trait、`ParseInput`、`ParseOutput`、`SupportLevel`。
2. `parser-docx` 支持基础 `.docx` 文本抽取。
3. `parser-pdf` 支持文本层检测，扫描 PDF 进入 `OCR_REQUIRED`。
4. 解析超时和 parser 错误映射。

验收：

```bash
cargo test -p parser-common
cargo test -p parser-docx
cargo test -p parser-pdf
```

测试必须包含 docx happy path、损坏文件、文本层 PDF、扫描 PDF 判定。

### S7：文本清洗、分段、基础字段规则

交付：

1. 空白、换行、页眉页脚基础清洗。
2. offset 映射基础能力。
3. 简历语义分段 fallback：无法识别时按长度和段落 chunk。
4. 邮箱、手机号、日期范围的强规则抽取。

验收：

```bash
cargo test -p text-normalizer
cargo test -p sectionizer
cargo test -p extractor-rules
```

测试必须覆盖中英混排、表格线性化文本、offset 映射、低置信字段不进强过滤。

### S8：Tantivy 文件名/全文索引和 search CLI

交付：

1. Tantivy schema：doc_id、version_id、file_name、clean_text、section_type、字段 fast fields。
2. index writer 和 reader 分离。
3. commit 后 reader reload。
4. `resume-cli search` 返回 rank、doc_id、file_name、snippet。

验收：

```bash
cargo test -p index-fulltext
cargo test -p search-planner
cargo run -p resume-cli -- search "Java 支付"
```

测试必须覆盖 commit 后可搜索、删除标记默认不可见、topN snippet 只对结果生成。

### S9：导入到查询闭环和崩溃恢复 smoke

交付：

1. 从 fixture 目录导入 docx/PDF 文本层。
2. 后台任务推进到 `SEARCHABLE` 或 `OCR_REQUIRED`。
3. search 能查到导入文档。
4. kill/restart 后已提交快照仍可读，未完成任务可重试。

验收：

```bash
cargo test --workspace
cargo run -p resume-cli -- import --root tests/fixtures/resumes
cargo run -p resume-cli -- status
cargo run -p resume-cli -- search "Java"
```

必须在 `PROGRESS.md` 记录实际输出摘要。

### S10：P2 字段抽取和字段过滤 MVP

进入条件：S9 通过。

交付：

1. email、phone、school、degree、skills、date range 的 MVP 抽取。
2. 字段置信度和原文证据。
3. 字段过滤查询：degree_min、skills_any、years_experience_min。
4. 候选人软去重 skeleton。

验收：

```bash
cargo test -p extractor-rules
cargo test -p rank-fusion
cargo run -p resume-cli -- search "Java" --degree bachelor --top-k 20
```

### S11：P3 语义检索 skeleton

进入条件：S9 通过。

交付：

1. `Embedder` trait。
2. 可替换 fake embedder，用于测试不依赖模型下载。
3. `VectorIndex` trait。
4. 混合检索接口和 RRF 融合测试。

验收：

```bash
cargo test -p embedder
cargo test -p index-vector
cargo test -p rank-fusion
```

如果模型 license 未确认，只能做接口、fake implementation 和测试，不下载或捆绑真实模型。

### S12：P4 OCR 支路 skeleton

进入条件：S9 通过。

交付：

1. OCR worker client 接口。
2. OCR_REQUIRED 状态和队列。
3. 页级超时、取消、缓存键类型。
4. 默认不运行重 OCR，只保证扫描件不拖垮查询。

验收：

```bash
cargo test -p ocr-client
cargo test -p ingest-scheduler
```

### S13：性能 smoke、故障注入、诊断包脱敏

进入条件：S9 通过。

交付：

1. 小数据 query benchmark smoke。
2. kill daemon、索引快照损坏、磁盘空间不足的可模拟测试。
3. `resume-cli doctor`。
4. `resume-cli export-diagnostics --redact` skeleton。

验收：

```bash
cargo test --workspace
cargo run -p resume-cli -- doctor
cargo run -p resume-cli -- export-diagnostics --redact
```

诊断包不得包含明文手机号、邮箱、完整路径或简历原文。

## 6. 长时间 Goal 推荐提示词

可以把下面这段作为另一个干净对话的 Goal：

```text
请在 /Users/frankqdwang/MLE/resume-ir 中长期执行。

先阅读 GOAL.md、MANIFEST.md、01_system_design_系统设计/00_阅读顺序.md、02_execution_plan_执行方案/00_阅读顺序.md、02_execution_plan_执行方案/10_长时间Goal执行清单.md。

目标：按 10_长时间Goal执行清单.md 的 slice 顺序尽可能多完成可验收工作。先做 S0-S9；如果全部通过且还有时间，再继续 S10-S13。

执行方式：把 docs/superpowers/specs/2026-05-30-long-running-goal-execution.md 和 docs/superpowers/plans/2026-05-30-long-running-goal-execution.md 当作已通过 fw-plan-review 的规格和计划，使用 fw-build 的执行纪律推进；不要进入 push、PR、release、签名或真实数据导入。

规则：
- 当前目录若还不是 Git 仓库，先 git init 并提交现有文档基线。
- 每个 slice 必须有测试或 smoke command，验收通过才提交。
- 每个 slice 更新 PROGRESS.md，记录完成项、命令、输出摘要、blocker。
- 不使用真实简历，不上传数据，不 push，不发布 release，不做签名/notarization。
- 若连续 3 次被同一问题阻塞，记录 blocker，提交已通过的独立工作，继续下一个不依赖它的任务。
- 第二天早上我会按 git log、PROGRESS.md、测试命令和 checklist 状态验收。
```

## 7. 早上复盘 checklist

醒来后优先看这些：

```bash
git log --oneline --decorate -20
git status --short
# 如果 Cargo.toml 尚不存在，说明执行还停在 S0；先看 PROGRESS.md 和 git log。
cargo fmt --check
cargo clippy --all-targets --all-features -D warnings
cargo test --workspace
```

然后检查：

1. `PROGRESS.md` 是否逐 slice 更新。
2. 每个完成 slice 是否有对应 commit。
3. 是否存在未说明的失败测试。
4. 是否有未确认事项被擅自越界处理。
5. 是否有明文 PII、真实路径、真实简历内容进入日志或 fixture。
6. 如果 S9 已完成，再决定是否继续推进 P2/P3/P4，或先做 `fw-plan-review`。

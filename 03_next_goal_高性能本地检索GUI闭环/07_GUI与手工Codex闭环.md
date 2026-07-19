# GUI 与手工 Codex 闭环

## 1. GUI 第一屏

GUI 第一屏是可操作工作台，不是 landing page。

必须包含：

1. Root 选择和当前 root 状态。
2. Level1/2/3 可见性计数。
3. 后台队列和暂停原因。
4. 搜索框和模式切换。
5. 结果列表。
6. 选中结果详情。
7. diagnostics/export 入口。

## 2. GUI 工作流

### 2.1 导入

1. 用户选择 root。
2. GUI 调 daemon `import`。
3. GUI 订阅或轮询 `status`。
4. Level1 可见后显示“已发现”。
5. Level2 可见后允许搜索。
6. Level3 增强时显示 OCR/semantic 进度。

### 2.2 查询

1. 用户输入 query。
2. GUI 发送 daemon search。
3. GUI 展示 latency、partial、bucket/layer 状态。
4. GUI 展示结果和 snippet。
5. 详情通过 daemon detail 单独读取。

### 2.3 手工验证

手工验证必须能记录：

1. 导入是否启动。
2. Level 计数是否增长。
3. keyword 查询是否返回。
4. filter/hybrid 查询是否 partial 或完整。
5. OCR/semantic 关闭时是否明确显示降级。
6. diagnostics 是否只含 redacted aggregate。

## 3. Codex 闭环脚本

Codex 验证不依赖截图内容作为唯一证据。每个 GUI/manual flow 都要有对应 CLI/daemon aggregate check：

```bash
./scripts/ci/verify-local.sh
RESUME_IR_LOCAL_EVIDENCE_DIR=<local-evidence-dir> RESUME_IR_QUERY_ARTIFACT_ROOT="$RESUME_IR_QUERY_ARTIFACT_ROOT" resume-cli --data-dir <local-data-dir> benchmark-query-set freeze-agent-replay --max-queries 500 --min-queries 500
scripts/local/run-current-stage-validation.sh --dry-run ... [--query-set <local-evidence-dir>/private-query-set.local.jsonl]
scripts/local/run-current-stage-validation.sh --execute ... [--query-set <local-evidence-dir>/private-query-set.local.jsonl]
```

query-set preparation 和 `--execute` 都只在本机跑；提交物只能是 redacted aggregate evidence。

## 4. GUI 验收

1. GUI 不直接打开私有 query set。
2. GUI 只在用户选中结果、进入本地 detail 时显示该文档的正文和路径；选中本身就是明确的 detail 动作，不增加第二次确认。
3. GUI 可在 OCR/semantic off 时搜索。
4. GUI 可在 daemon 重启后恢复状态。
5. GUI 可显示 partial 原因。
6. GUI 可导出 redacted diagnostics。

GUI 只能依赖 `06_Daemon_IPC与Diagnostics契约.md` 中的 versioned IPC/diagnostics contract。GUI 不读取 SQLite table、Tantivy field、sidecar path、ANN implementation、raw benchmark file 或 local artifact path。GUI 需要展示 overload、partial、cancelled、degraded、repairing 和 benchmark lane，而不是把这些状态折叠成通用错误。

## 5. GUI Information Architecture

GUI 是 app UI，不是 landing page。第一屏必须按同一信息层级设计，默认 `Tauri + React + Vite + Tailwind + TypeScript` lane 和任何合法 fallback 都必须复用同一 `UI-reference/` 视觉基准与页面结构。

第一屏扫描顺序：

1. 可搜索状态、root health 和 daemon 状态。
2. 查询输入、mode segmented control 和 filter 摘要。
3. 结果列表、partial/degraded/overload 解释。
4. 选中结果 detail/snippet/hydrate 状态。
5. diagnostics、evidence lane 和 redacted export 状态。

参考布局：

| 区域 | 职责 | 禁止 |
|---|---|---|
| top command bar | root selector/status、search input、mode、filter summary、pause/cancel | 把主要搜索动作藏进菜单 |
| left rail | import queue、Level1/2/3 counts、OCR/semantic queue、budget/degraded state | 用装饰 card 堆叠代替状态分组 |
| center workspace | 100000 logical results 的有界结果面；1440x900 参考态稳定显示 4 张大卡片，使用滚动或分页，buffer 最多 8 张，并保留 partial markers 和 selection state | 为了追求可见行数压缩 `UI-reference/` 的卡片密度，或让 hover、loading、长文本改变卡片尺寸 |
| right detail panel | selected result detail、snippet、自动本地 hydrate、完整正文和 display path | 把正文或路径写入 diagnostics、benchmark、日志或公开证据 |
| bottom status strip | benchmark lane、latency、diagnostics/export、redaction flags | 把 W0/W1/smoke 证据混成一个状态 |

## 6. GUI Interaction State Matrix

GUI 不得把所有非成功状态折叠成通用 error。每个区域必须有可见状态、可复核状态码和用户下一步动作。

| Feature | Loading | Empty | Error | Success | Partial / Degraded / Overload / Cancelled |
|---|---|---|---|---|---|
| root/import | root 正在扫描，显示 Level1/2/3 目标和队列入口 | 未选择 root，主动作是选择 root | root 不可读或 watcher unavailable，显示 retry 和 fallback 状态 | Level 计数增长，搜索可用性明确 | partial download、journal gap、budget degraded 必须显示原因和恢复动作 |
| search | deadline 内显示 pending，不阻塞已有结果 | zero-result 显示 query shape、filter 摘要和清除 filter 动作 | parse error 或 daemon error 显示 request_id 和 redacted reason | 结果列表、latency、参与 layer 明确 | partial/degraded/overload/cancelled 显示 layer、retry_after_ms 或 cancel 状态 |
| results | 结果列表保留 stable row height 和 skeleton | 结果为空时显示可执行下一步，不显示 raw query | hydrate/snippet 单项失败不清空整页 | 选中、排序、snippet 状态稳定 | 单项 partial marker 不改变列表布局 |
| detail | detail hydrate pending 显示占位 | 未选中时显示选择提示 | detail unavailable 显示 redacted reason | snippet、field、explanation 可见 | sidecar checksum 或 hydrate timeout 显示 partial detail |
| diagnostics/export | export pending 显示 redaction checklist | 无可导出证据时显示 lane 原因 | export blocked 显示 prohibited material class | 只导出 redacted aggregate | smoke/W0/W1/GUI/manual lane 必须清晰区分 |

## 7. GUI User Journey Storyboard

默认 GUI implementation acceptance 必须证明用户能完成从首次导入到 redacted evidence export 的完整闭环，而不是只展示静态 dashboard。

| Step | User does | First 5 seconds | First 5 minutes | Long-term trust evidence |
|---|---|---|---|---|
| first run | 选择 resume root | 立即知道 root 是否可读、是否开始扫描 | Level1/2/3 计数开始变化，等待原因可见 | root health、watcher/fallback 状态可复核 |
| first searchable | 等 Level2 可搜索 | 清楚知道现在能搜什么、还缺什么 | OCR/semantic 继续后台增强，不阻塞 keyword/field search | visible epoch 和 partial reason 可追踪 |
| query | 输入 keyword/filter/hybrid query | search input、mode、filter 摘要最显眼 | 结果、latency、参与 layer、partial/degraded 状态可解释 | simple text semantics 和 benchmark lane 没被 GUI 隐藏 |
| inspect | 选择结果看详情 | detail panel 不暴露 raw path | snippet、hydrate、sidecar 状态清楚 | detail 错误不污染整页结果 |
| manual validation | 人工确认 workflow | checklist 对应 import/search/detail/export | 可记录 pass/fail、partial 和 blocked 原因 | Codex 可用 redacted aggregate 复核 |
| evidence export | 导出 diagnostics | redaction checklist 明确 | 只产生 lane-aware aggregate，不含 raw query/resume/path/token | exported evidence 能支撑 W0/W1/GUI/manual 判定 |

## 8. Responsive and Accessibility Contract

默认 GUI visual contract 必须覆盖 desktop、tablet 和 narrow viewport。响应式不是简单堆叠，而是保留搜索、结果、状态和证据 lane 的可操作性。

| Viewport | Layout contract | Must remain visible |
|---|---|---|
| desktop >= 1200px | top command bar + left rail + center workspace + right detail panel + bottom status strip | root health、search input、4-card reference viewport、detail、diagnostics lane |
| tablet 768-1199px | left rail 可折叠，detail panel 可作为 side sheet，center workspace 保持主焦点 | search input、result list、partial/degraded marker、selected detail entry |
| narrow < 768px | single-column task flow：root/status、search、results、detail、diagnostics 以 tabs 或 segmented navigation 切换 | search action、result count、current lane、overload/partial reason、export status |

Accessibility requirements:

1. Keyboard order follows top command bar -> left rail -> center workspace -> right detail panel -> bottom status strip.
2. Focus visible state must be obvious without relying only on color.
3. Body text contrast ratio must be at least 4.5:1.
4. Interactive targets must be at least 44px on touch viewports.
5. Search status, overload, cancelled, degraded, and export completion must have screen-reader-visible status text.
6. Placeholder text cannot be the only label for search, filter, root selector, or export controls.

## 9. GUI Design System Boundary

当前目标不创建完整 `DESIGN.md`，但默认 GUI implementation acceptance 必须使用同一组临时 design tokens，确保实现复用 `UI-reference/` 视觉基准和页面结构。正式 `DESIGN.md` 可以在 GUI 实现前通过单独计划建立。

Temporary tokens:

| Token | Contract |
|---|---|
| type scale | 12px metadata、14px dense table/list、16px body/control、20px section heading |
| spacing density | 4px inner gap、8px control gap、12px group gap、16px panel gap |
| result card | 1440x900 参考态一屏稳定 4 张大卡；高度从 `UI-reference/search.png` 测量，hover、loading、partial 和 selection 不得引起布局跳动 |
| status colors | success、partial、degraded、overload、error、blocked 必须语义区分，不只靠颜色 |
| focus style | keyboard focus 使用 outline 或 ring，不能只用 hover |
| panel surfaces | app UI 使用平静工作台面，不使用 marketing hero、装饰 card grid、渐变背景或图标装饰 |

## 10. GUI 技术栈与 UI-reference 视觉合同

默认 GUI 技术栈冻结为 `Tauri + React + Vite + Tailwind + TypeScript`。

Tauri 负责桌面壳、系统权限、打包、native bridge 和 daemon IPC 边界。React 负责界面状态和组件组合。Vite 负责把前端构建成 Tauri 可内嵌的静态资产。Tailwind 负责承载 `UI-reference/` 已有视觉语言。生产 GUI 不运行 Next.js server；当前 `UI-reference/` 的 Next.js 原型只作为视觉参考，不作为发布架构。

`UI-reference/` 是视觉基准，不是功能逐项复刻要求。功能、页面数量、字段和流程可以按 daemon IPC、diagnostics、benchmark/manual 验证和产品需求调整；视觉语言不得漂移。

必须保留的视觉不变量：

1. GUI 是安静、克制、高密度的本地工作台，不是 landing page。
2. 使用浅色背景、薄边框、紧凑 spacing、低装饰密度和清晰信息层级。
3. 保留 left rail、top command bar、center workspace、detail side sheet/panel、status/diagnostics affordance 的工作台结构。
4. 保留稳定 row/card 尺寸，hover、loading、长文本和 partial marker 不得造成列表布局跳动。
5. 使用 Lucide 风格图标、紧凑按钮、pill、tag、segmented control、redacted diagnostics/export affordance。
6. 主强调色应接近 reference primary，黑/灰承担主要文本和操作层级。
7. 圆角默认接近 8px，除非组件有明确本地理由。

验收目标是 pixel-level visual similarity，不是 identical functional clone。视觉相似性由 design token inventory、reference screenshot inventory、representative page screenshots、manual/Codex review 和 GUI/manual issue 证据共同判断。

Fallback bakeoff rule:

1. `Tauri + React + Vite + Tailwind + TypeScript` 是默认 lane。
2. 只有 GitHub issue 记录明确 blocker 后，才允许重新打开 egui/eframe、Slint 或其他 toolkit bakeoff。
3. 合法 blocker 包括 WebView2/Windows packaging 失败、weak-host runtime footprint 无法接受、100000 logical rows 交互性能不可达、native integration 无法满足 daemon IPC 或 diagnostics 边界。
4. fallback bakeoff 必须复用同一 `UI-reference/` inventory 和 representative pages，不能发明新的视觉风格。

## 11. 当前桌面垂直边界

S764 固化了第一条真实桌面查询路径：

1. `apps/desktop/` 是独立的 `Tauri + React + Vite + Tailwind + TypeScript`
   应用，视觉密度、左侧栏、顶部查询、中央结果、右侧详情和底部状态延续
   `UI-reference/` 合同。
2. WebView 不读取 `ipc.auth`、`ipc.endpoints.json`、SQLite、索引目录或本地路径。
   Rust bridge 从 owner-only data dir 读取 versioned endpoint/token，仅允许
   `status`/`search`/`detail`/`cancel`，并限制 request <= 64 KiB、response <= 2 MiB。
3. 查询使用 `resume-ir.ipc-request.v3` 的 `interactive_gui` class、有界 deadline
   和 cancel token；结果与详情只消费 daemon 已脱敏合同，默认不提供 hydrate
   或 raw path 操作。
4. 当前已实现 loading、empty、error、partial、overload、cancelled 和 daemon
   degraded/unavailable 的独立界面状态；这些状态不得在后续切片中合并成通用错误。
5. 下一产品缺口是 native root chooser 到 daemon import/status 的垂直闭环。完整
   root path 必须留在 Rust/daemon 侧，WebView 只能获得有界显示标签、root handle
   和聚合状态。

S764 不声称 root/import、diagnostics/export、100000-row virtualization、打包/签名、
Windows 或 GUI/manual 完整验收已完成。

## 12. Native root/import/status 边界

S765 固化了 root 到 import/status 的第二条桌面垂直路径。本节保留的
`daemon.status.v1` 是历史 checkpoint；S807 已硬切为 `daemon.status.v2`，当前实现不得
回退或双读 v1。

1. 目录选择器由 Rust 原生进程持有，不给 WebView 浏览器文件系统权限；完整路径
   不进入 JavaScript、DOM、日志或返回 payload。
2. WebView 只接收随机 `root_handle` 和最多 80 字符的 basename 显示标签；原生
   registry 最多保留 16 个 root handle，过期 handle 必须重新选择。
3. 提交时 WebView 只回传 handle，Rust 解析完整路径并使用 owner-only token 调用
   loopback `POST /imports`。不设置隐式 `max_files`，成功响应必须匹配
   `daemon.import.v1`。
4. 选择取消、目录不可读、daemon unavailable/degraded、提交失败和 retry 是不同
   状态；提交后只轮询 `daemon.status.v1` 的有界聚合计数，不读取 SQLite 或任务路径。
5. 当前 root 标签、发现/可搜索/OCR/failed 计数和队列状态均有明确上限或 daemon
   typed contract；前端仍不持有 endpoint manifest、token、完整正文或原始路径。

S765 的编译、typed bridge、public-synthetic daemon import/status 证据已通过，但原生
选择器视觉截图仍不属于 accepted GUI/manual evidence。下一产品缺口是 lane-aware
redacted diagnostics/export；100000-row virtualization、打包/签名、Windows 和
GUI/manual 完整验收仍未完成。

## 13. Redacted diagnostics/export 边界

S766 固化了从 daemon 到 GUI 原生导出的第三条桌面垂直路径。本节保留的
`resume-ir.diagnostics.v2` 是历史 checkpoint；S807 已硬切为
`resume-ir.diagnostics.v3`，当前实现不得回退或双读 v2。

1. daemon endpoint manifest additive 提供 authenticated `GET /diagnostics`，成功合同
   是 `resume-ir.diagnostics.v2`；未认证请求返回 401。
2. payload 只包含固定聚合 metrics、最多 16 个 scan-error buckets、最多 64 个
   benchmark refs、`visible_epoch` 和五个 exact-false privacy flags。不得包含 root、
   query、resume text、snippet、candidate result、token 或 diagnostics package。
3. runtime export 固定声明 `gui_manual / unaccepted`，导出动作不能把自己升级成
   accepted GUI/manual、W0 或 W1 evidence。
4. WebView 可显示 typed bounded aggregate 和 5/5 redaction checklist；Rust bridge 在
   展示与导出前再次验证 schema、privacy boundary、lane/status 和数组上限。
5. 保存路径由 Rust 原生选择器持有。WebView 不传或接收完整保存路径；写出 JSON
   最大 256 KiB，receipt 只返回最多 80 字符的文件名。

S766 不把现有 CLI `diagnostics.v1` release-readiness evidence surface 静默解释为
daemon v2，也不声称原生保存选择器截图已形成 accepted GUI/manual evidence。下一步
必须先冻结 public-synthetic GUI virtualization/manual benchmark，再调 100000 logical
rows 的 viewport/overscan；打包/签名、Windows 和 goal completion 仍未完成。

## 14. Four-card reference benchmark 边界

S767 按用户确认的 `UI-reference/` 视觉密度硬切 GUI benchmark：

1. representative model 仍至少是 100000 logical results，不通过减少逻辑结果规模逃避渲染压力。
2. 1440x900 的参考搜索态必须稳定显示 4 张大结果卡；导航只能使用滚动或分页，滚动 buffer 最多 8 张。
3. 卡片密度、颜色、字号、边框、圆角、hover/focus、遮罩和 detail side sheet 以 `UI-reference/search.png` 与 `detail.png` 为视觉真值，不再为 20-60 行目标压缩设计。
4. 选中结果后 GUI 可自动通过 authenticated local detail/hydrate 读取正文和 display path；这些敏感内容可在本机产品 UI 显示，但仍禁止进入日志、diagnostics、benchmark、截图证据或 git。

## 15. Reference-fidelity local detail 实现边界

S768 在现有 Tauri 桌面壳中闭合了首个真实搜索垂直面：

1. 搜索页按 `search.png` 保留 240px sidebar、48px top bar、24px 内容 gutter、12px 卡片间距、14px 卡片圆角和四卡 viewport；每页只挂载 4 张卡片并通过分页继续导航，不回退为高密度表格，也不超过 8-card buffer cap。
2. 只保留搜索、简历来源、隐私与诊断三个后端已支持入口；演示状态选择器和无真实 handler 的性能、任务、设置入口不进入生产 UI。
3. 关键词、混合、语义直接使用 daemon mode；字段过滤使用现有 fulltext + typed filters 合同，不新增不存在的 daemon mode。
4. 结果选择后自动调用 authenticated `details` 与 `details/hydrate`。Rust bridge fail closed 校验 `resume-ir.detail-hydrate-response.v1`、local-only privacy flags、32 KiB 正文页和 128 KiB display path；WebView 最多保留 128 页正文并在未完整时明确标注截断。
5. `detail.png` 的 576px sheet、20% 遮罩、1px blur、border、shadow 和内部表面样式保持不变；本地正文和路径可直接显示，不增加与产品目的冲突的二次确认。
6. `design-qa.md` 和两张 public-synthetic 1440x900 capture 是视觉实现证据，不是 private GUI/manual、W0/W1、发布或 goal-complete 证据。真实敏感正文、路径、查询和候选结果仍不得进入截图、日志、diagnostics、benchmark 或 git。

## 16. Self-contained desktop daemon bootstrap 边界

S777 将原生桌面从开发机路径依赖推进到首个可打包 runtime composition：

1. 锁定 Tauri v2 的 release composition overlay 中，`bundle.externalBin` 只声明
   一个 `resume-daemon`；基础 `tauri.conf.json` 不声明 sidecar，因此 fresh checkout
   的 direct Cargo test/check/clippy/build 与 `tauri dev` 不依赖 ignored artifact。
   release/debug bundle wrapper 自动合并 overlay，build hook 必须在 Tauri build
   script 消费 sidecar 前，根据
   `TAURI_ENV_TARGET_TRIPLE` 构建根 workspace 的同目标 daemon，并写入已忽略的
   `target/tauri-sidecars/resume-daemon-<target-triple>[.exe]`；未知 target fail
   closed。首批只允许 arm64/x64 macOS 和 x64 MSVC Windows，不声称 universal
   macOS。Tauri release build wrapper 同时将 repo root 与 builder home 从 Rust
   产物路径中重映射；debug/dev 保留本地运行时发现语义。macOS sidecar 的 Cargo
   中间产物使用受 owner/type/mode 检查的固定非身份化 `/tmp` target；若该目录由
   其他用户占用或是 symlink，构建 fail closed。
2. packaged Rust 应用默认使用 Tauri `app_local_data_dir()`；
   `RESUME_IR_DATA_DIR` 和 `RESUME_IR_DAEMON_BINARY` 只允许 debug build 本地覆盖，
   release 不依赖也不读取这些覆盖。data dir、sidecar path、endpoint manifest 和
   bearer token 都不进入 WebView state、command response、diagnostics 或截图证据。
3. daemon 仍由 Rust 侧使用固定参数启动；不增加 JavaScript shell API、shell
   permission、第二个 `invoke_handler`、新 command、通配窗口、`core:default` 或远程
   capability。现有五个 command、main-only capability、CSP 和已批准视觉保持不变。
4. bundle composition 必须比较 staged 与 `.app` 内唯一 daemon 的 bytes/digest，
   校验 regular executable、目标 Mach-O 架构，并扫描拒绝 repo root 或 builder home
   字节；回执只声明该精确 scope 的
   `build_machine_identity_path_markers=0`，不得外推为二进制不存在任何通用绝对
   build prefix。清空
   `RESUME_IR_*` 的 synthetic native smoke 还必须证明 app-local data、daemon ready、
   status 和退出回收。

S777 只闭合 daemon bootstrap。embedding/OCR runtime、managed-root 持久化与
watch/rescan、Windows NSIS 实机、签名/notarization/updater 和完整 installer/product
闭环仍是后续独立切片。

## 17. Native offline embedding runtime 边界

S778 先移除 semantic/hybrid 对开发机 Python 环境的产品依赖，但不把运行时打包、
常驻协议与 installer 构成混为一个切片：

1. 新的 CPU-only Rust runtime 只读取已审核且显式指定的 multilingual-E5 ONNX、
   tokenizer 和 ONNX Runtime 本地文件；编译和运行均不得包含 Hugging Face 下载或
   运行时网络 fallback。
2. 内部 model identity 必须绑定不可变 runtime-pack revision，不能只用上游可变
   model id 与 dimension；换 pack 后不得复用旧向量。
3. 保持 `resume-ir-embedding-v1` 有界 command contract：每批最多 4 条、每条最多
   65536 bytes、输出最多 4 MiB；query/passage prefix、mean pooling 和 L2 normalize
   与已验证 E5 行为一致。daemon 为一个或多个文档生成超过 4 个全文/section input
   时必须保持顺序分批，全部分批成功后才发布向量；任一批失败时整组 job 保持可重试，
   不得放宽 runtime admission 或发布半组向量。
4. H0 默认只允许 1 个 ONNX intra-op thread。错误模型、维度、缺件、路径逃逸、
   malformed/oversized input、非有限或错误维度输出都 fail closed，且 stdout/stderr、
   diagnostics 和证据不得包含原文、向量或本地路径。
5. slice success 必须用真实模型和 synthetic 文本证明现有 daemon 的 semantic 与
   hybrid 可用，并证明 clean environment 不需要 Python、venv、HF cache 或网络。

S778 不声称 embedding 已常驻，也不声称模型/runtime 已进入 Tauri resources。
后继切片分别完成 resident stream/cancel recovery 与 target-specific resource
manifest、bundle verifier、macOS/Windows installer 原生证据。

实现选择必须使用已审核的 multilingual-E5-small dynamic-int8 pack，而不是 FP32
候选：语义 model identity 绑定模型、tokenizer 与 config 的固定 bytes/digest，目标平台
ORT library 作为独立 bundle 组件校验。macOS synthetic witness 的物理内存峰值必须低于
H0 512 MiB 私有/匿名红线；该证据不能替代 Windows H0，也不能替代后继 resident
runtime 的 latency、cancel 与 restart-recovery 验收。

## 18. Resident native embedding runtime 边界

S779 只闭合 daemon 生命周期内的模型常驻复用与恢复：

1. S778 one-shot command 继续只用于 preflight/maintenance；长期 daemon 使用一个
   supervisor 管理一个 native runtime generation，模型 ready 后才接 embedding 请求，
   但 daemon IPC、keyword 和 field 不等待模型启动。
2. daemon 与 runtime 使用显式 serde 类型和 length-prefixed framing。请求最多 2 MiB、
   响应最多 4 MiB、每批最多 4 条、每条最多 65536 bytes；输入角色只能是 query 或
   passage，wire 不携带文档 id、文件名、路径，错误只返回有限枚举和 retryable 标志。
3. supervisor 使用有界 interactive/background admission，并在请求边界优先 interactive。
   H0/H1/H2 默认 inference threads 分别为 1/2/3，CPU 只能下压。取消、超时、子进程
   退出或 malformed response 必须终止并回收旧 generation，随后重新加载；daemon
   关闭必须回收 runtime，不能遗留子进程。
4. semantic 在 runtime starting/restarting/unavailable 时返回可恢复的独立 partial 原因；
   hybrid 保留已有 BM25 有界结果并使用同一原因，不得冒充 search-index-not-ready。
   background embedding 失败保持整组 job retryable，不发布半组向量。
5. 自动化必须证明同一 daemon 的重复 semantic/hybrid 复用一个 generation，取消或
   超时后后继请求成功，以及 keyword/field 不触发也不等待 runtime。真实模型验收只用
   synthetic 文本和脱敏聚合；原文、query、向量、候选、路径、child command、stderr
   body、模型/runtime bytes 不得进入日志、diagnostics、截图或 Git。

S779 不包含 Tauri `bundle.resources`、Windows Job Object 实机证明、OCR runtime、
managed roots、DMG/NSIS、签名/notarization/updater，也不声明 installer 或完整产品完成。

## 19. Tauri native embedding resource composition 边界

S780 只闭合 macOS arm64 Tauri app 中已验收 native embedding 运行时与
不可变 qint8 pack 的自包含组合：

1. release `externalBin` 精确包含 daemon 与 native embedding runtime；
   `bundle.resources` 只从忽略的 target staging 目录复制已提交预期 manifest
   声明的六个文件，不得整目录携带 cache、证据或构建输出。
2. staging 与 bundle verifier 都校验 schema、pack/model identity、dimension、
   role/file 集合、bytes/SHA-256、MIT license review、network disabled、普通文件、
   无 symlink/path escape 及 Mach-O target architecture；只输出有界聚合 receipt。
3. release Rust 不读取 embedding command/model/dimension 环境覆盖，只从 app
   可执行文件同级定位 runtime，并通过 `BaseDirectory::Resource` 解析 pack；
   WebView 不获得 runtime path、resource path、token 或新 command 权限。debug-only
   开发覆盖不成为发布包依赖。
4. macOS arm64 clean-environment native witness 必须证明 app 启动自有 daemon、
   packaged runtime 完成 synthetic semantic/hybrid 并复用同一 resident generation，
   退出后进程树被回收，且不需要 Python、venv、HF cache、仓库路径或网络。

任何 model/runtime bytes、query/resume 原文、向量、候选、路径、token、cache、
stderr body 或私有截图进入 Git、日志、diagnostics 或公开证据即回滚。
S780 不证明 Windows/x64、Job Object、OCR、DMG/NSIS、签名/notarization/
updater、managed roots 或完整产品。

## 20. Tauri native OCR resource composition 边界

S781 只闭合 macOS arm64 Tauri app 中的自包含 OCR 引擎、语言包和
PDF 页渲染：

1. release `externalBin` 精确增加一个使用 CoreGraphics 的 PDF renderer；
   `bundle.resources` 只从忽略的 target staging 目录复制已提交预期
   manifest 声明的 Tesseract、递归 dylib closure、`eng+chi_sim` tessdata、
   TSV config 和许可证/notice。不携带 Poppler、Homebrew 路径、cache 或构建
   证据。
2. assembler、staging 和 bundle verifier 都校验 schema、pack/engine/renderer
   identity、语言集合、role/file/bytes/SHA-256、license/source review、network
   disabled、普通文件、无 symlink/path escape、arm64 Mach-O，以及所有非系统
   dylib 已改写为 pack-local loader path；只输出有界聚合 receipt。
3. native renderer 只通过三个显式环境字段接收内部 worker 请求，校验
   绝对普通 PDF、页码、DPI、64 MiB 输入、10000 像素边长和 10000000
   总像素预算，仅从 stdout 返回有界 PPM，错误不包含路径。
4. release Rust 忽略 OCR command 环境覆盖，只从 app sibling 定位
   renderer，并通过 `BaseDirectory::Resource` 定位 Tesseract/tessdata pack；
   daemon 只获得有界引数、`TESSDATA_PREFIX` 和单线程限制。WebView 不获得
   runtime path、resource path、token 或新 command 权限。
5. macOS arm64 必须连续完成两次无手工修复的 target `tauri build`，且
   isolated-HOME native App witness 必须用无文本层合成 PDF 证明导入、OCR、
   索引、keyword 搜索、队列归零和五项 diagnostics 隐私标志全部通过。

任何 runtime/OCR 原文、query/result、路径、token、cache、child stderr、
私有数据或截图进入 Git、日志、diagnostics 或公开证据即回滚。
S781 不证明 Windows/x64 OCR composition、Windows process containment/H0、
managed roots、bridge typed projection、签名/notarization/updater、installer lifecycle
或完整产品。

## 21. Windows per-user NSIS composition contract 边界

S782 先冻结 Windows 普通用户安装入口，并在 Windows runtime 尚未齐备时
拒绝生成误导性的部分安装包：

1. `tauri.windows.conf.json` 是锁定 Tauri v2 自动合并的 platform config；
   Windows bundle target 只能是 `nsis`，`installMode` 固定为 `currentUser`，
   不把管理员权限或 `Program Files` 作为默认安装前提。
2. WebView2 使用 `offlineInstaller` 且 silent 安装。下载只允许发生在独立构建
   环境获取 installer payload 的阶段；最终 `setup.exe`、首次安装和正常运行
   不得临时下载 WebView2、daemon、模型、OCR 或其他产品 runtime。
3. `allowDowngrades=false`，避免普通安装动作静默覆盖为更旧版本。升级、回滚和
   数据保留仍需后续 Windows native lifecycle 证据，不能由配置声明代替。
4. `x86_64-pc-windows-msvc` 的基础 Rust sidecar 规划可以存在，但完整 desktop
   composition 在 reviewed Windows embedding pack、OCR pack、PDF renderer 和
   Job Object process containment 同时可用前必须 fail closed，且不得先构建或
   staging 一个可被误认为可发布的 partial NSIS。
5. Windows H0 终端验证机只接收最终 installer；Git、Rust、Cargo、Node、Python、
   VC Build Tools、仓库源码、模型服务和环境覆盖只能属于独立构建/调试环境，
   不能成为安装或运行依赖。

S782 只证明锁定 schema 接受 per-user/offline Windows installer contract，且当前
缺件路径会明确拒绝。它不证明 Windows runtime bytes、PDF/OCR 可用、Job Object、
NSIS 已产出或执行、H0 原生流程、签名/updater、managed roots、bridge typed
projection、release readiness 或完整产品。

## 22. Windows static OCR source contract 边界

S794 为 Windows 自包含 OCR 闭包冻结可复现的 build-side 输入，但不把源码合同
冒充运行时或 installer：

1. 目标固定为 `x86_64-pc-windows-msvc`，Tesseract 5.5.2、Leptonica 1.87.0
   和 tessdata_fast 4.1.0 使用不可变 tag/commit、许可证与 checksum 身份；
   产品运行时禁止下载。
2. Tesseract 与 Leptonica 均要求 static MSVC runtime。只保留 PDF renderer 已输出
   P6 PPM 所需的 PNM decode；关闭 shared build、OpenMP、training、graphics、curl、
   archive、TIFF 及不必要图片 codec，避免把额外 DLL 或网络面带进 H0 安装包。
3. 产品协议固定为有界 PPM 输入、`--psm 6`、`eng+chi_sim` 和 TSV stdout；
   child owner 仍是已验收的 `ocr_tesseract` Job Object lane，stdout/stderr、超时与
   cancel 边界不得放宽。
4. 最终 `tesseract.exe` 必须是 x64、最多 64 MiB、仅依赖 Windows system DLL，
   并拒绝动态 MSVC/UCRT/OpenMP import。该 binary、build provenance、expected
   pack manifest 与 native OCR 证据必须在独立 Windows build 环境后续生成和审核。
5. Windows H0 只接收最终 `setup.exe`。S794 的 SSH 观察只确认 build 19045、
   7.76 GiB、8 logical processors、WebView2/sshd 可用；机器已有的 Node/Python
   不能成为产品依赖，最终验收必须使用净化环境并审计进程树。

S794 后 composition planner 仍 fail closed：在真实静态 ONNX Runtime、Tesseract、
PDFium bytes、expected manifests、最终 PE closure 和 native evidence 齐全前，不得
生成 partial NSIS。本切片不证明 Windows OCR、NSIS、H0 GUI、签名、updater 或
完整产品。

## 23. Windows daemon runtime/bundle target 边界

S804 只闭合 Windows daemon 的 SQLCipher/OpenSSL 自包含构建输入，不把独立
sidecar 目标三元组冒充为 Tauri application ABI：

1. Tauri app 与最终 NSIS bundle target 保持 `x86_64-pc-windows-msvc`；daemon
   runtime 使用 `x86_64-pc-windows-gnu`。两者只通过既有本地进程 IPC 通信，不共享
   FFI、Rust ABI、allocator 或动态 runtime，因此 receipt 必须分别记录 runtime 与
   bundle target，不能把 GNU executable 标成 MSVC build。
2. Tauri `externalBin` 要求的 staged 文件名仍使用 bundle target 后缀
   `resume-daemon-x86_64-pc-windows-msvc.exe`；内容验证必须证明 PE32+ x64、
   SQLCipher/OpenSSL 静态闭包，并只允许已审核的 Windows 10 系统 DLL 与 UCRT
   API-set import。`libcrypto`、`libssl`、`libgcc`、`libwinpthread`、动态 MSVC
   runtime 和未知 import 一律 fail closed。
3. 构建只能发生在独立 build host，使用锁定的 cargo-zigbuild/Zig、固定的安全临时
   target directory、release path remapping 与无 shell 的有界输出。产物不得包含
   repository root、builder home 或 builder account identity；receipt 不包含本机路径。
4. H0 仍只接收最终 `setup.exe`，不得安装 Git、Rust、Cargo、Node、Python、Docker
   或 build tools。S804 的 cross-build 与 PE closure 不是 H0 native execution、
   Job Object、完整 runtime composition、NSIS、升级/卸载、签名/updater 或产品完成
   证据；这些必须由后继 installer/native 切片证明。

## 24. Windows native ONNX Runtime builder 边界

S805 只闭合 static-MSVCRT `onnxruntime.dll` 的可复现原生构建所有权，不把 builder
合同冒充实际 runtime、model pack 或 installer：

1. 构建输入固定为 ONNX Runtime `v1.24.4`、commit
   `2d924974ef147392ced8409d36bd6d2e7fcc8a74` 的 recursively initialized clean
   checkout、已审核 source contract 和互不重叠的绝对 source/destination。tracked、
   untracked、submodule、license/notice 身份任一漂移都在构建前 fail closed。
2. 只接受 native Windows x64 VS 2022 Developer Prompt、Python >= 3.10、CMake >=
   3.28、MSVC 14.x 和 Windows SDK 10.x。owner 无 shell 调用官方
   `tools/ci_build/build.py`，固定 Release shared library、static MSVC runtime、
   CPU-only、telemetry-off、skip-submodule-sync 和单并发；官方 tests 必须成功。
3. 只接受 exact Release PE32+ x64 `onnxruntime.dll`，复用既有 export、Windows
   system-only import、dynamic CRT、license 和 provenance validator。candidate 先在
   sibling staging 校验，再原子 promote；失败恢复先前已验收 runtime root，只输出
   小于 4 KiB 的无路径 receipt。
4. 用户授权同一台 8 GiB Windows 机器分阶段复用。安装 Git/Node/Python/CMake/VS
   后它只是 build host，既有 H0 证据失效；生成所有 Windows artifacts 后必须 reset
   或 reprovision，并重新证明无开发依赖，才能再次作为只接收最终 `setup.exe` 的 H0。
   OrbStack/Linux container 不替代 native MSVC 环境，Docker 不进入该 lane。
5. S805 的 synthetic GREEN 只证明 builder admission、invocation、artifact、provenance
   和 rollback 合同。真实 Windows build 尚未执行，因此不证明 DLL bytes、model
   inference、OCR/PDFium、NSIS、H0 native flow、签名/updater 或完整产品。

任何源码/构建路径、账户身份、日志、runtime bytes、模型、凭证、私有数据或截图进入
提交证据即回滚；真实构建只可留下已审核产物和有界脱敏 provenance/receipt。

## 14. S807 native lifecycle and detail UX

The WebView no longer starts or kills the daemon. Tauri setup creates one
managed supervisor actor; UI may only call `get_daemon_lifecycle()` and the
explicit `retry_daemon()` half-open trigger. The fixed recovery budget and
backoff live in native code, not user settings.

The UI renders three independent axes: lifecycle
`starting/ready/recovering/circuit_open/blocked`, service
`ready/degraded/repairing`, and result freshness
`current/stale/interrupted`. Polling is serialized (`ready` every 5 seconds,
otherwise every second) and refreshes immediately on focus. During recovery,
results remain visible as context but actions pause. An epoch change marks
ranking potentially stale without automatic research. Interrupted detail keeps
already-rendered content but stops pagination until explicit resume;
`STALE_SELECTION` clears private detail/path/body state while retaining the
result list and requiring a fresh search. Existing four-card density remains a
visual acceptance constraint.

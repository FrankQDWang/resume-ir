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
scripts/local/run-performance-goal-validation.sh --plan --out <local-evidence-dir>
scripts/local/run-performance-goal-validation.sh --execute --resume-root "$RESUME_IR_PRIVATE_RESUME_ROOT" --query-artifacts "$RESUME_IR_QUERY_ARTIFACT_ROOT" --out <local-evidence-dir>
```

`--plan` 可以提交，`--execute` 只在本机跑，输出 local-only evidence。

## 4. GUI 验收

1. GUI 不直接打开私有 query set。
2. GUI 不显示 raw path，除非用户在本机明确打开 detail。
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
| center workspace | 100000 logical rows 的虚拟列表、20-60 viewport-visible rows、overscan <= 2x visible rows、stable row height、partial markers、selection state | 让 hover、loading 或长文本改变列表布局 |
| right detail panel | selected result detail、snippet、hydrate status、explanation | 默认暴露 raw local path |
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
| desktop >= 1200px | top command bar + left rail + center workspace + right detail panel + bottom status strip | root health、search input、20-60 row viewport result list、detail、diagnostics lane |
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
| row height | result row stable 44-56px，expanded row 不改变相邻 row height |
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

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

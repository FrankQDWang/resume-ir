# CI/CD 与发布打包

## 1. 验证与发布平台

当前仓库由单人维护，合并验证以本机为唯一执行面。GitHub Actions 不再
运行 PR、push、定时、安全、模型或 benchmark CI，避免与本机验证重复。

本机验证入口：

```bash
./scripts/ci/verify-local.sh --parallel
```

小切片先跑受影响的 focused checks，合并前再跑一次适当范围的本机验证。
发布仍保留显式手动触发的 GitHub Actions workflow；它不是合并门禁。

## 2. Workflow 总览

```text
.github/workflows/
  release.yml         # 仅手动发布
```

## 3. 本地合并验证

不配置 `pull_request`、`push` 或 `schedule` workflow，也不配置 required
status checks。PR 用于保留可审核 diff 和合并历史，不重复执行本机已经完成的
测试。

本地验证覆盖：

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features -D warnings`
3. `cargo test` 或 `cargo nextest run`
4. schema migration test
5. parser golden tests
6. license check
7. secret scan
8. changed docs check
9. minimal benchmark smoke test

本地常规验证只跑合成或小数据；真实目录 profile 和长时压测按性能合同单独执行。

## 4. 平台边界

当前交付目标仅为本机 macOS。平台构建、测试、安装和性能证据都在本机完成；
Windows 与 Linux 不属于当前交付门禁。

## 5. 性能基准

不运行夜间或定时 GitHub benchmark。性能实验在受控的本机实验窗口中显式运行，
并按当前 performance contract 记录基线、A/B 顺序、资源干扰和脱敏聚合证据。

任务：

1. 10 万脱敏语料导入。
2. 查询集 P50/P95/P99。
3. parser 成功率。
4. OCR smoke benchmark。
5. 向量召回 benchmark。
6. 索引大小和内存峰值。
7. 与上一夜对比，超过阈值 fail。

本地私有产物：

```text
bench-results/
  date.json
  query_latency.csv
  ingest_throughput.csv
  resource_usage.csv
  regression_report.md
```

## 6. Release workflow

触发：`vX.Y.Z` tag。

步骤：

1. 校验版本号和 changelog。
2. 构建 release 二进制。
3. 运行 release smoke test。
4. 打包 Windows MSI。
5. 打包 macOS pkg/dmg。
6. 代码签名。
7. macOS notarization。
8. 生成 SBOM。
9. 生成 checksums。
10. 上传 GitHub Release artifact。
11. 生成 release notes。
12. 可选：发布模型包 manifest。

## 7. 安装包策略

### Windows

推荐：MSI/WiX。

安装内容：

1. `resume-daemon.exe`
2. `resume-cli.exe`
3. worker 二进制。
4. 默认配置模板。
5. 可选 UI。
6. 卸载清理脚本。

注意：

1. 不默认删除用户数据目录。
2. 守护进程注册和自启动要可关闭。
3. 企业环境可能限制服务安装，保留 user-mode daemon。

### macOS

推荐：签名 pkg/dmg。

注意：

1. 必须处理 Gatekeeper、签名和 notarization。
2. 数据目录放用户 Library。
3. 后台进程使用 LaunchAgent，而不是强行系统级 daemon。
4. Apple Silicon 和 Intel 兼容策略要明确。

## 8. 自动更新

不要把程序、模型、词典、索引 schema 绑死在一个更新包里。

| 更新对象 | 策略 |
|---|---|
| 程序 | 小版本自动更新，大版本提示 |
| 模型包 | 独立 manifest + checksum |
| 词典 | 独立热更新，可回滚 |
| 索引 schema | 迁移前备份，不兼容则重建 |
| OCR 语言包 | 用户选择安装 |

## 9. 质量门禁

合并到 main 的门禁：

1. 按改动风险完成 focused checks，并在合并前完成适当范围的本机验证。
2. 公开推送前运行 `./scripts/ci/guard-public-repo.sh`。
3. PR 保留 diff 与合并记录，但不等待 GitHub status checks。
4. 不允许直接 push main。
5. 重大性能 regression 不允许合并。

## 10. CI 工具建议

| 工具 | 用途 |
|---|---|
| `cargo fmt` | 格式化 |
| `cargo clippy` | 静态 lint |
| `cargo nextest` | 更快测试运行 |
| `cargo deny` | license、advisory、重复依赖 |
| `cargo audit` | Rust 安全公告 |
| `sccache` | 编译缓存 |
| `CodeQL` | 安全分析 |
| `gitleaks` 或同类 | secret scan |
| `cargo llvm-cov` | 覆盖率 |
| SBOM 工具 | 发布物依赖清单 |

## 11. 发布通道

| 通道 | 用途 |
|---|---|
| nightly | 内部验证，可能不稳定 |
| alpha | 功能预览，小规模用户 |
| beta | 性能和兼容性验证 |
| stable | 正式发布 |
| lts | 企业稳定版本，可选 |

## 12. 回滚策略

必须支持：

1. 程序回滚。
2. 模型包回滚。
3. 词典回滚。
4. 索引 schema 不兼容时重建。
5. 元数据迁移失败时恢复备份。

升级前动作：

1. 写 migration plan。
2. 备份元数据。
3. 校验磁盘空间。
4. 迁移 dry-run。
5. 失败回滚。

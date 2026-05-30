# CI/CD 与发布打包

## 1. CI/CD 平台

首选 GitHub Actions。

原因：

1. 与 GitHub PR、CODEOWNERS、branch protection 集成最直接。
2. 支持 Windows/macOS/Linux runner。
3. 支持 workflow matrix。
4. 支持 artifact、cache、release、环境密钥。
5. 支持安全扫描、CodeQL、依赖更新等生态。

## 2. Workflow 总览

```text
.github/workflows/
  pr.yml              # 每个 PR 必跑
  ci-platform.yml     # 跨平台构建和测试
  bench-nightly.yml   # 夜间基准测试
  release.yml         # tag 发布
  model-eval.yml      # 模型转换、量化、质量评估
  security.yml        # 安全扫描、依赖审计、SBOM
```

## 3. PR workflow

触发：`pull_request`。

必跑 job：

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features -D warnings`
3. `cargo test` 或 `cargo nextest run`
4. schema migration test
5. parser golden tests
6. license check
7. secret scan
8. changed docs check
9. minimal benchmark smoke test

PR workflow 只跑小数据，不能跑百万级压测。

## 4. 跨平台 CI

矩阵建议：

```yaml
strategy:
  fail-fast: false
  matrix:
    os: [windows-latest, macos-latest]
    profile: [release]
```

扩展矩阵：

1. Windows x86_64。
2. macOS arm64。
3. macOS x86_64，如果仍要支持 Intel Mac。
4. Linux 仅用于工具链和服务器式基准，不作为用户目标平台。

如果 GitHub-hosted runner 对 Apple Silicon 覆盖不足，准备自托管 M 系列 runner。

## 5. 夜间基准

触发：`schedule` + 手动 `workflow_dispatch`。

任务：

1. 10 万脱敏语料导入。
2. 查询集 P50/P95/P99。
3. parser 成功率。
4. OCR smoke benchmark。
5. 向量召回 benchmark。
6. 索引大小和内存峰值。
7. 与上一夜对比，超过阈值 fail。

产物：

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

1. PR 必须通过必跑 CI。
2. 至少 1 名 code owner approval。
3. 涉及安全/隐私需 security owner approval。
4. 涉及索引 schema 需 index owner approval。
5. 涉及 release/installer 需 platform owner approval。
6. 不允许直接 push main。
7. 不允许合并 failing checks。
8. 重大性能 regression 不允许合并。

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

# 跨平台全盘发现与增量 Journal 契约

本文件冻结“指定目录”和“全盘/全电脑”发现语义。后续实现不得把平台 watcher 的最佳努力事件流当成可靠事实源。

## 1. Root Semantics

用户选择一个目录、一个卷或全电脑扫描时，系统都统一表示为 root set：

| Root kind | 含义 | 默认行为 |
|---|---|---|
| directory | 用户显式选择的目录 | 只扫描该目录树 |
| volume | 用户显式选择的本地卷 | 扫描该卷可访问目录 |
| all_computer | 用户显式授权的本地卷集合 | 枚举固定本地卷，不默认扫描网络卷 |

Root set 的公开证据只能包含 root count、volume count、hash 和状态，不提交本机路径。

## 2. Traversal Rules

1. symlink 不默认跨出 root；若目标仍在 root 内，可按 canonical identity 处理。
2. Windows reparse point、junction、OneDrive placeholder 必须标记，不默认强制下载或跨卷递归。
3. macOS package directory 是否展开必须由 file kind 策略决定，不靠路径后缀黑名单隐式处理。
4. cloud placeholder、partial download、权限拒绝、文件锁定都必须进入 manifest 状态和 GUI 可见原因。
5. 外置盘 offline 不批量 tombstone；进入 root offline，等待重新 online 或 bounded reconciliation。
6. path 只是 alias，stable identity 和 content fingerprint 才能决定 rename、replacement 和 reparse。

## 3. macOS Contract

macOS 使用 FSEvents 作为增量信号，不把它当成唯一事实源：

1. 记录 per-root event stream position 和 volume identity。
2. 检测 event gap、must-scan-subdirs、history dropped、root changed 时，标记 dirty subtree。
3. dirty subtree 进入 bounded reconciliation：manifest diff、content fingerprint、mutation batch。
4. rename 不能只靠 path event 判断，必须用 stable identity 和 content fingerprint 确认。
5. FSEvents 不可用时，fallback 到 periodic manifest diff，并在 status/diagnostics/GUI 中标明 degraded。

## 4. Windows Contract

Windows 使用 NTFS USN Journal 作为增量信号，不把它当成唯一事实源：

1. 记录 per-volume journal id、USN cursor、volume serial 和 file reference number。
2. journal id 改变、cursor 过期、USN gap、权限不足或非 NTFS 时，标记 volume dirty subtree。
3. reparse point、junction、OneDrive placeholder 和 long path 必须有独立状态，不允许造成循环扫描。
4. rename、hardlink、replacement 必须通过 stable file id、content fingerprint 和 path alias 三层对账。
5. USN 不可用时，fallback 到 periodic manifest diff，并在 status/diagnostics/GUI 中标明 degraded。

## 5. Fallback Reconciliation

Fallback 不是失败沉默模式。它必须：

1. 限制每轮扫描 budget，避免低配机器被全盘 diff 打满。
2. 输出 scanned_count、changed_count、dirty_subtree_count、skipped_count、permission_denied_count。
3. 把 root 状态暴露给 daemon status 和 GUI。
4. 对同一 dirty subtree 的重复失败进入 Loop blocked 证据，而不是无限重试。

## 6. First-Scan No-Miss Baseline

首次全量发现不是 watcher 测试。每个平台都必须先建立 no-miss baseline：

1. 在 synthetic root set 中放置普通文件、深层目录、rename、replacement、symlink/reparse、cloud placeholder、permission-denied、locked file 和 external volume/offline fixture。
2. 首次 traversal 产生 manifest rows、root aggregate、skipped classes 和 redacted diagnostics。
3. 第二次零变更 traversal 必须满足 `changed_manifest_rows=0`、`reparse_count=0`、`full_rebuilds=0`。
4. 对每类不可读或 placeholder 只记录 class/count/hash，不提交路径。
5. 首次 no-miss baseline 通过后，才允许把 FSEvents/USN 作为增量信号接入。

## 7. Incremental Journal Acceptance

macOS 和 Windows 必须分别证明：

1. create/modify/delete/rename/replacement 进入 mutation batch。
2. journal gap 不直接丢事件，而是转为 dirty subtree reconciliation。
3. watcher unavailable 会进入 degraded periodic manifest diff。
4. rename/hardlink/replacement 不只靠 path event 判断，必须通过 stable identity/content fingerprint/path alias 对账。
5. external volume offline 不批量 tombstone，重新 online 后才 reconcile。

## 8. Acceptance

后续 P5 平台 journal 切片必须至少覆盖：

1. macOS FSEvents gap -> dirty subtree -> bounded reconciliation。
2. Windows USN gap -> volume dirty subtree -> bounded reconciliation。
3. watcher unavailable -> periodic manifest diff degraded mode。
4. symlink/reparse loop 不逃逸 root。
5. cloud placeholder 不强制下载。
6. permission denied 可见且不阻塞其他 root。
7. external volume offline 不批量 tombstone。
8. full computer root set 只提交 redacted aggregate evidence。

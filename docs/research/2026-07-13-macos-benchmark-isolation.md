# macOS 开发机上的性能隔离与抗干扰测量

日期：2026-07-13

## 结论

同一台正在承担日常工作的 macOS 主机上，可以隔离 benchmark 的软件状态、数据、
缓存定义、进程指标和证据目录，但不能通过公开稳定的 macOS API 为 benchmark
独占若干 CPU 核，从而完全屏蔽其他应用与系统服务的影响。

因此不应继续把所有 mixed-import 工作都绑定到“整机必须近乎空闲”这一前置条件。
更合适的设计是分成两条证据通道：

1. **共享开发机诊断通道**：允许其他任务继续运行，使用目标进程 CPU 时间、RSS、
   I/O、stage metrics、进程采样和随机交错 A/B 来定位瓶颈、验证相对变化；不把
   共享主机 wall-clock 数值冒充绝对验收基线。
2. **独占主机验收通道**：只在专用实体 Mac、专用 bare-metal Mac，或自然出现的
   合格低负载窗口中产生 absolute wall-clock acceptance evidence。

这不是降低验收门槛，而是把“能指导开发的证据”和“能支持绝对性能声明的证据”
分开。当前以整机外部 CPU 超限为由拒绝全部 profile 的方法，把本可使用的
process-scoped 诊断证据也丢弃了。

## 一手资料结论

### 1. macOS affinity 不是独占核心

Apple SDK 的 `mach/thread_policy.h` 将 `THREAD_AFFINITY_POLICY` 标为
experimental，并明确说明 affinity tag 只是给 scheduler 的 thread-placement
**hint**，目标是尽可能共享 L2 cache。它不是 Linux `cpuset` 那样的排他 CPU
分区，也没有保证某个物理核不运行其他进程。

来源：

- Apple XNU `thread_policy.h`：
  <https://github.com/apple-oss-distributions/xnu/blob/main/osfmk/mach/thread_policy.h>
- 当前 macOS SDK 同一公开头文件可用以下命令复核：
  `rg -n -C 8 THREAD_AFFINITY_POLICY "$(xcrun --sdk macosx --show-sdk-path)/usr/include/mach/thread_policy.h"`

直接后果：不能用 affinity tag 把 resume-ir 固定到“不会受 Chrome、Python、
WindowServer 或系统 daemon 影响”的专属 P-core/E-core。

### 2. QoS、taskpolicy 与 nice 是优先级，不是资源保留

Apple 对 QoS 的定义是告诉系统工作的相对重要性，由系统据此优先安排和分配资源；
高优先级工作可以从低优先级工作取得更多资源。这说明 QoS 是竞争时的调度语义，
不是容量预留或隔离合同。

本机随系统提供的 `taskpolicy(8)` 只能设置磁盘 I/O policy、Darwin background
priority、QoS clamp、throughput tier 和 latency tier，并由子进程继承；
`renice(8)` 只改变 scheduling priority。两者都没有“独占 CPU”语义。

来源：

- Apple Energy Efficiency Guide，QoS 与有限资源竞争：
  <https://developer.apple.com/library/archive/documentation/Performance/Conceptual/power_efficiency_guidelines_osx/PrioritizeWorkAtTheTaskLevel.html>
- Apple 对 Apple silicon 调度的说明：
  <https://developer.apple.com/news/?id=vk3m204o>
- macOS 自带手册：`man taskpolicy`、`man renice`。

可用但有限的做法是只对 benchmark 自身使用 `taskpolicy -c utility`，让日常交互
保持响应；这会形成一个不同的 hardware/QoS lane，不能与普通 H2 运行混为一谈。
不应自动修改用户其他任务的优先级，更不应依赖 admin 权限恢复 `renice` 状态。

### 3. VM 和容器能限额，但不能隔绝宿主竞争

Apple Virtualization framework 允许配置 guest 可见的 `CPUCount` 和
`memorySize`。这些字段定义 guest 环境的 CPU 数和内存大小；Apple 文档没有提供
把这些 vCPU 排他绑定到 host 物理核心的保证。VM 能很好地隔离操作系统、文件系统、
依赖和缓存命名空间，但 vCPU 线程仍由同一 host scheduler 运行。

Docker Desktop 在 macOS 上同样只允许限制其 Linux VM 可用的 host CPU 和内存。
Docker 的 `--cpus`、`--cpu-quota` 和 `--cpuset-cpus` 约束的是 Linux VM/容器
内部调度。它们可以限制容器最多使用什么，却不能阻止 macOS 宿主应用占用承载
vCPU 的物理核心。

来源：

- Apple `VZVirtualMachineConfiguration`：
  <https://developer.apple.com/documentation/virtualization/vzvirtualmachineconfiguration>
- Apple Linux VM 示例中的 CPU/内存配置：
  <https://developer.apple.com/documentation/virtualization/creating-and-running-a-linux-virtual-machine>
- Docker Desktop host resource settings：
  <https://docs.docker.com/desktop/settings-and-maintenance/settings/>
- Docker Engine resource constraints：
  <https://docs.docker.com/engine/containers/resource_constraints/>

Linux 在真正拥有底层 CPU 控制权时可以创建 cgroup v2 isolated cpuset partition，
将 CPU 从 scheduler load-balancing domain 中移出。Linux kernel 文档明确推荐这种
运行时可调的隔离方式。macOS host 并不向 Docker VM 提供等价的物理核心排他合同，
所以不能把 guest 内的 cpuset 证明外推为 host 隔离。

来源：

- Linux kernel CPU isolation：
  <https://docs.kernel.org/admin-guide/cpu-isolation.html>
- Linux cgroup v2 cpuset partitions：
  <https://docs.kernel.org/admin-guide/cgroup-v2.html>

### 4. 进程级 profiler 可以在繁忙主机上保持归因

Apple 的 `sample` 可以按 PID 或进程名采样目标进程并写入单独文件；这不会把其他
应用的栈算进目标进程。完整 Xcode 安装还提供 `xctrace`，可以记录、导入、导出和
符号化 Instruments trace。

来源：

- macOS 自带 `sample --help`。
- Apple Xcode command-line tool reference：
  <https://developer.apple.com/documentation/xcode/xcode-command-line-tool-reference>

当前机器只有 Command Line Tools，未安装可用的完整 Xcode/xctrace；`sample`、
`/usr/bin/time -l`、程序自身 stage metrics 和结构化 tracing 是现在即可使用的
process-scoped 观测面。

### 5. 重复、warm-up、交错与异常值处理不能制造硬隔离，但能产生诚实的相对证据

Hyperfine 的官方项目文档提供 warm-up、多次运行、缓存准备、统计汇总和干扰/
异常值检测；Google Benchmark 的官方指南支持 warm-up、repetitions，以及 mean、
median、standard deviation 和 coefficient of variation。它们解决的是估计与报告，
不是从调度器取得专属资源。

来源：

- Hyperfine 官方仓库：<https://github.com/sharkdp/hyperfine>
- Google Benchmark user guide：
  <https://github.com/google/benchmark/blob/main/docs/user_guide.md>

对长达数十秒的 mixed import，更有效的是在同一时间段随机或交错运行 control 与
intervention，使背景负载成为成对观测的共同协变量，并同时报告全部 pair、最差
pair、median delta、离散程度和 host covariates。共享主机的 A/B 可以支持
“这个改动是否改善目标进程”这一判断，不能直接支持“产品绝对 P95 已达标”。

### 6. 独占实体 Mac 才是绝对验收的清晰边界

如果开发机必须持续承载其他任务，最干净的 absolute-baseline 方案是另一台专用
实体 Mac。它可以作为本地 self-hosted runner，只接收 resume-ir benchmark，且
不承载交互应用。GitHub 文档明确说明 self-hosted runner 由用户控制硬件、OS 和
软件环境。

云端替代是 EC2 Mac。AWS 文档明确说明 EC2 Mac 只以 Dedicated Host 上的
bare-metal instance 提供，一台 Dedicated Host 运行一台 Mac instance；M4
机型也已列入支持范围。它提供比共享开发机更清晰的硬件所有权，但最短分配周期为
24 小时，而且当前项目的私有语料不得在没有单独授权和加密传输合同的情况下上传。
因此 EC2 Mac 默认只适合 synthetic/public workload；私有 W1 更适合本地第二台
专用 Mac。

来源：

- GitHub self-hosted runners：
  <https://docs.github.com/en/actions/concepts/runners/self-hosted-runners>
- AWS EC2 Mac instances：
  <https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-mac-instances.html>
- AWS EC2 Mac FAQ：<https://aws.amazon.com/ec2/instance-types/mac/faqs/>

## 对 resume-ir 的建议方案

### A. 共享开发机诊断通道：现在就能使用

建议新增明确命名的 `shared_host_diagnostic` lane，保留以下合同：

1. release binary、冻结 corpus、接受的 owner-only model、worker 数、H-tier、
   warm/cold 定义和唯一 data directory 固定不变；blind holdout 继续封存。
2. correctness gate 仍然严格：五态计数、candidate-only admission、precision、
   contamination、completeness 和隐私边界有任何漂移即失败。
3. 每次记录目标进程及其子进程的 user/sys CPU time、wall time、peak RSS、I/O、
   page faults、stage metrics、queue wait 和 target stack summary。
4. host idle、thermal、memory pressure 和 pageout 作为 covariates 完整报告，不把
   `external_cpu > 20%` 自动转换成“所有进程级证据不存在”。
5. profile/hotspot 选择依赖目标进程 stack share、CPU time 和 stage share；
   不依赖一次整机 wall-clock。
6. 优化前后使用至少九个随机交错 pair；报告全部 pair、median、最差 pair、CV 和
   语义一致性。host 状态差异过大时该 pair 失效，但不连带删除其他有效 pair。
7. 该 lane 只允许 `diagnostic`、`relative_movement` 或 `no_conclusion`，禁止
   `absolute_baseline_accepted`、scale、release 或 goal-complete 声明。

这条通道允许开发机继续运行 Chrome、编辑器、其他 Codex 和 SeekTalent；它不会
假装这些负载不存在，而是只对 resume-ir 自己的资源消耗与相对变化负责。

### B. 本机机会式绝对验收通道

保留当前 strict host-validity gate，但只用于 `quiet_host_acceptance`：

1. runner 在后台等待一个合格的连续低负载窗口；等待期间不占用大量 CPU。
2. 窗口出现后自动执行 warm-up 和五次 formal repetitions。
3. 任一 formal run 受持续外部 CPU、serious/critical thermal、真实 memory pressure
   或显著 swap 影响时，记录为 invalid，不修改阈值。
4. 长期没有窗口只阻塞 absolute acceptance，不阻塞共享主机的 profiler 与相对
   优化工作。

### C. 专用 Mac 绝对验收通道

当需要稳定、可重复、无人值守的 absolute baseline 时：

1. 优先使用一台本地专用 Apple-silicon Mac，安装同版本 macOS/toolchain，关闭
   交互登录项，只运行 self-hosted benchmark runner。
2. 私有 corpus 只通过用户授权的本地只读路径进入，结果仍只发布 redacted
   aggregate。
3. 若采用 EC2 Mac，默认只运行 public synthetic；任何私有数据迁移必须另立
   privacy/transfer contract，不能由 benchmark runner 自行决定。

## 不建议的方案

- 不把 macOS affinity tag 描述为 CPU pinning 或 core reservation。
- 不通过提高 benchmark 优先级抢占开发机，这会干扰用户任务并改变产品运行形状。
- 不自动 renice、暂停或终止无关用户进程。
- 不把 Docker/OrbStack/Linux VM 中的 cpuset 当成 macOS host 物理隔离证明。
- 不因为 shared host 有背景负载而丢弃正确性、process CPU、RSS 和 profiler 证据。
- 不用异常值过滤隐藏全部不利样本；所有 pair 和 invalid reason 都必须保留。
- 不用共享开发机结果声明绝对性能、D10K/D100K/D1M、release 或 goal completion。

## 建议的下一实现切片

只修改 benchmark methodology，不动产品语义：

1. 给现有 variance runner 增加 `shared_host_diagnostic` 与
   `quiet_host_acceptance` 两种显式 mode。
2. 将 production-compatible model envelope validation 做成运行前 fail-fast，避免
   再把压缩训练 artifact 传给生产 loader。
3. 让 shared lane 生成 process-scoped stage/resource/profiler aggregate 和随机交错
   pair 结论；让 quiet lane 保留现有 absolute host gate。
4. 用 synthetic load/noise fixture 证明 shared lane 不会产生 absolute claim，
   quiet lane 仍会拒绝同样的 host noise。
5. 先用 shared lane 完成 #37 的 bottleneck attribution，再把 absolute wall-clock
   acceptance 留给机会式窗口或专用 Mac。

这个切片解决的是“开发机不能停，但 profile 也不能永远停”的根问题；它不改变
classifier、index admission、query semantics、private benchmark membership 或任何
performance threshold。

# Runtime 环境探测与容器轮询 CPU 开销优化 TODO

日期：2026-11-13

状态：待实施，作为下一项优先处理的性能工作。先做 A（native runtime 信息读取），再做 B（同轮容器状态查询合并），分别提交、分别测量。

目标：减少 OpenDAN 和 node_daemon 稳态运行时反复启动 shell、工具命令和 Docker CLI 的开销，保持环境信息、执行身份和容器状态判断正确。

范围约定：当前 INFO 日志量是预期行为，本单不调整日志级别、输出量、轮转或保留策略；不包含磁盘清理、容器限额、证书续期和扩容。

## 1. 现场依据

2026-11-13 对 `demo.buckyos.io` 做只读采样，机器为 2 vCPU / 3.8 GiB 内存，运行 12 个容器。下表采用排除日志检索和目录扫描影响后的 CPU 样本；任务创建历史来自检查前的 sar 数据。

| 指标 | 观测 |
| --- | --- |
| 整机 CPU | 30 秒稳定样本约 42%，其中内核态约 18%；近一周日均约 39%–50% |
| 任务创建 | 历史约 115 次/秒；包含进程和线程，不等同于 115 个独立业务进程 |
| OpenDAN 探测链 | 10 秒内，10 个 opendan 及其后代创建 996 个任务、执行 711 次外部程序，其中 72 次 bash；反复出现 cat、hostname、id、uname、tr、date |
| node_daemon 的 Docker 调用 | 同一 10 秒窗口启动 24 次 Docker CLI，其后代另创建 151 个任务；后者包含运行时线程 |
| 其他资源 | 可用内存约 2.9 GiB，磁盘使用率 60%，磁盘 I/O 等待约 0.3%；没有紧迫容量瓶颈 |

上述调用链是持续后台开销的明确贡献者，不能据任务数量推算它们占用了全部内核态 CPU，也不预先承诺整机 CPU 降到某个百分比。

源码核对基线：本地 `buckyos` HEAD `ee426e1e9`。尚未核实现场二进制、镜像与本地源码完全一致；实施验收前记录实际部署版本，避免把已有优化未部署与当前代码缺口混在一起。

## 2. 当前代码与已有优化

| 入口 | 当前行为 |
| --- | --- |
| [agent_tool/runtime/mod.rs](../src/frame/agent_tool/src/runtime/mod.rs) `Runtime::initialize()` | 已用 `tokio::sync::OnceCell<OpenedRuntime>` 缓存初始化结果，包括 `RuntimeInfo`、身份、cwd、PATH 和能力 |
| 同文件 `AgentRuntime for Runtime::info()` | 每次仍调用完整 local / SSH probe，最后只把 `current_time`、`timezone` 覆盖回已缓存的信息；SSH 还承担身份校验 |
| [agent_tool/runtime/ssh.rs](../src/frame/agent_tool/src/runtime/ssh.rs) `LOCAL_PROBE` / `local_probe()` | Unix 启动 shell 并执行一组外部工具；Windows 使用 PowerShell probe。命令组合与现场追踪吻合 |
| [lib_opendan/runner/drive.rs](../src/frame/lib_opendan/src/runner/drive.rs) 构造 `InputMaterial.runtime_status`；`Runtime::open()` | 都调用 `runtime.info()`，会把完整探测带入输入材料构建和工具初始化路径 |
| [agent_tool/runtime/node.rs](../src/frame/agent_tool/src/runtime/node.rs) `RemoteNodeRuntime::info()` | 已经从缓存克隆信息并在进程内更新时间，不属于本次 shell 探测问题 |
| [node_daemon/app_mgr.rs](../src/kernel/node_daemon/src/app_mgr.rs) `AppRunItem::get_state()` → [app_loader.rs](../src/kernel/node_daemon/src/app_loader.rs) `observe_deployment()` | 状态和健康度已合并观察，正常 Docker 路径仍逐应用执行一次 `container inspect` |
| [node_daemon/node_daemon.rs](../src/kernel/node_daemon/src/node_daemon.rs) 主循环及 `node_main` | 外层循环每轮休眠 5 秒，`node_main` 应用检查最多 4 个并发；逐容器查询的 CLI 启动量仍随应用数增长 |

补充边界：`drive.rs` 也调用 [lib_opendan/runtime/mod.rs](../src/frame/lib_opendan/src/runtime/mod.rs) 的 `runtimes_view()`，但 `lookup_rechecked()` 仅对符合条件的异常 node / desktop 状态做带时间间隔的重查，正常可用 runtime 不会因此无条件重新探测。本单不靠关闭这些恢复检查来省 CPU。

[已完成的 node_daemon CPU 优化](done/node-daemon-cpu-optimization-todo.md) 已落地 PID 快速检测、共享冷发现、状态/健康度合并、镜像 digest 缓存和 run.plist 批量写入。保留这些实现，本单针对仍存在的 CLI 启动开销继续优化。

## 3. A 首先消除 native info 的重复完整探测

- [ ] **A1 初始化与信息读取分开。** 复用现有 `OpenedRuntime` / `OnceCell`，完整 native probe 只负责 runtime 初始化。初始化失败保留重试语义，并发初始化只产生一次成功的探测；不要新增跨用户、跨 runtime 的全局环境缓存。
- [ ] **A2 时间读取走轻量路径。** 默认 native 环境的 `info()` 在进程内取得当前时间及相应时区，复用已有 `chrono` 等能力。初始化后的反复 `info()` 不再启动 bash、PowerShell 或工具子进程；不能把完整 probe 改成高频 TTL 刷新来替代消除开销。
- [ ] **A3 明确时间与时区语义。** 优先覆盖没有独立 TZ 覆盖的 native 环境；`RuntimeConfig.env["TZ"]` 与进程不同的情况单列测试。多个 runtime 不得通过修改进程全局 `TZ` 互相影响。保留时间和时区输出语义：Unix 当前为本地时间及 `%Z%z`，Windows 为 `DateTimeOffset.Now` 及 `TimeZoneInfo.Local.Id`，不能假定 chrono 的 `%Z` 与系统 date 的输出相同。验证系统校时、时区变化与夏令时，不能永久冻结初始化时的 UTC offset。特殊 TZ 首批可保留原路径并明确标记为零派生合同的例外；后续再用现有依赖或有界的轻量时钟查询收敛，不为首批引入新的时区库或全局缓存。
- [ ] **A4 保留环境初始化结果。** `id/kind/target`、hostname、os/arch、shell、cwd、PATH、platform、shell_kind、baseline 与工具能力继续来自目标 runtime 的初始化；`open()` 的工具合并逻辑保留，第一次 `open()` 不应为更新时间再做第二次完整 native probe。
- [ ] **A5 限定后端。** 首批只优化 native，包括容器中的 native。`remote_ssh` 的目标时间、身份校验、连接失败和 alias 改向拒绝行为保持；不能直接用调用方本机时间替代 SSH 目标时间，也不能一并删掉 `verify_identity()`。`remote_node` 已有轻量读取，不做重复重构。
- [ ] **A6 核实实际调用与实例生命周期。** 对 `drive.rs`、`Runtime::open()` 和 session runtime 构造做调用计数，确认一次初始化能跨输入批次复用；分别记录初始化 probe、信息读取、显式健康/能力检查，不能把所有现场 exec 都归因于单一函数。保留输入时效、事件驱动和异常 runtime 恢复检查。

验收重点：预热完成、无显式 TZ 特殊回退的 native runtime 连续及并发读取 `info()` 时，新增外部命令数为 0；时间正常推进，提示词环境字段正确。新增用例应验证真实行为与调用次数，覆盖初始化失败后重试、并发首次访问以及 runtime 之间的配置隔离。

## 4. B 再合并同一轮的 Docker 状态观察

优先采用同一轮批量 `container inspect`，复用现有命令执行、解析和状态判断。不把替换整个 Docker 接入层、引入新的 Docker SDK 作为首步。

- [ ] **B1 按轮收集查询目标。** 在 node_main 应用检查前取得本轮受管容器名，仅包含实际使用 Docker / Worker 的应用，排除 NativeAgent、NativeScript 和不支持的运行时。按执行后端分组，以一次或有界分批的 inspect 获取快照。现有 `DockerContainerInspectDoc` 没有 Name 字段，需要补充映射所需字段；按实际名称/ID 关联，不依赖输出顺序。无容器目标时不调用 Docker。正常查询量从每容器一次变为每批一次，分批只用于后端或参数长度限制。
- [ ] **B2 复用观察结果。** 把本轮快照传给 `AppRunItem` / `AppLoader` 的观察路径，沿用运行状态、健康度、镜像身份、generation、labels 和 exttool 卷匹配逻辑。保留已有 digest 缓存，覆盖 Docker 应用及实际使用容器 Worker 的 Agent / HostScript 路径。
- [ ] **B3 正确区分缺失与失败。** 批量命令可能部分成功；容器缺失、Docker 不可达、权限失败、非零退出但有部分输出、无效 JSON 都要有明确行为。不得把整体查询失败当作所有容器不存在，进而批量重建；必要时仅对未确定的项回退到现有单项查询。
- [ ] **B4 快照仅用于本轮观察。** 启动、停止、部署、重建等变更后使相关项失效，后续决策按需重新读取；启动前的不存在判断重新确认。新一轮重新观察，不能用长期 TTL 的运行状态掩盖外部停止、删除、替换和健康变化；镜像 digest 缓存与容器状态快照分开。
- [ ] **B5 保持跨平台接入。** 核对 [wslc.rs](../src/kernel/node_daemon/src/wslc.rs) 的投影逻辑：目前 inspect 投影要求恰好一个对象。批量路径需同步支持并测试；也可在明确不支持的后端保留现有单项路径，不影响 Linux 首批优化验收。
- [ ] **B6 合并 Worker 期望值的并发刷新。** [paios_images.rs](../src/kernel/node_daemon/src/paios_images.rs) `worker_expectation()` 已有 30 秒共享缓存，但锁在查询前释放，并发过期可能重复执行 image / volume inspect。复用现有锁或本轮共享结果合并相同配置的在途刷新，保留原有 30 秒变化可见性、`invalidate_expectations()` 和错误重试语义；单独统计收益，不把 image / volume 查询混进容器查询数。现有 mock runner 路径绕过该缓存，新增测试需能实际覆盖缓存命中、并发过期和失效，不以旧 mock 用例代替。

验收重点：相同 12 个健康、配置不变的容器，在支持批量查询的后端，一轮常态 container inspect 为 1 次；这不是所有 Docker 命令合计 1 次，image / volume 和恢复查询另计。规模较大时按明确批大小增长。容器被外部停止、删除、替换，镜像或目标 generation 改变，Docker 暂时不可达时，收敛结果仍正确。扩展现有 [test_app_loader.rs](../src/kernel/node_daemon/src/test_app_loader.rs) 的 `deployment_observation_*` 用例，覆盖批量部分失败、乱序结果和同轮失效；复用 WorkerExpectation 与 WSLC 测试。

## 5. 验证与交付

- [ ] 先记录现场二进制/镜像版本、Agent 与容器数、活跃工作负载、日志级别。A、B 分别测量，保持 INFO 配置和相近工作负载；容器常驻不等于 Agent 业务空闲，不把未知负载称为空载。
- [ ] 对比预热后的任务创建率、probe/exec 次数、Docker CLI 调用次数、整机 user/system CPU、主要进程 CPU 时间、主循环耗时。固定观测窗口至少数分钟，短时内核追踪用于归因，不与大范围日志检索或磁盘扫描并行。
- [ ] A 中已预热 native runtime 的反复 `info()` 调用不再产生完整 probe 链 exec；新 Session 初始化、显式检查、SSH 和标明的特殊 TZ 例外单独计数，不要求整机同类命令归零。B 的查询量按批次而非容器数增长。整机 CPU 收益报告实测值，同时验证新输入、工具调用、runtime 恢复和容器异常恢复延迟，没有靠降频业务处理换取低 CPU。
- [ ] 在 `src/` 下运行下面与实施范围匹配的检查；SSH、Windows/macOS、WSLC 等需要环境的验证分别记录已测与未测，不能用 Linux 单测代替跨平台实测。

```bash
cargo test -p agent_tool --lib runtime:: -- --test-threads=1
cargo test -p libopendan --test session_runtime --test runtimes -- --test-threads=1
cargo test -p node_daemon -- --test-threads=1
cargo build -p opendan -p node_daemon
```

- [ ] 在仓库根目录运行 `bash test/runtime_ssh/run.sh`，利用本地临时 sshd 回归 SSH 文件、超时、恢复和身份改向场景；普通 cargo 测试中被忽略的真实 SSH 用例需要这个入口，不能记成已经覆盖。该验证不连接生产服务器。
- [ ] 核对 `RuntimeInfo`、提示词模板和执行协议；本单以性能优化为目标，不新增公开字段或存储格式。若实现改变用户可见 runtime / 管理行为，同步检查 `opendan_agent_dev`、`buckyos_admin` 手册。
- [ ] 按仓库开发流程构建包含 `agent_tool` 的 `paios/aios` 镜像；涉及 node_daemon 时同时构建宿主模块。记录实际部署版本后再进行现场对比，不能以本地测试通过替代镜像更新与运行验收。

实施回写：待实施。本文仅完成现场证据整理和源码核对，未修改运行代码、未执行上述测试、未重新部署服务器。

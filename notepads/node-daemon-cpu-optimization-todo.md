# node_daemon 周期性 CPU 峰值优化 TODO

日期：2026-10-12

状态：2026-10-12 已完成代码实现与配套测试，buckyos-base 设备采样改动已发布到 main，buckyos 的普通 Git 依赖已更新并通过无本地覆盖的测试与构建复验。真实 Zone 部署验收与整轮测量尚未完成，见文末实施记录。历史 review 意见保留供对照。

目标：降低约每 5 秒一次的服务保活检查开销。优先采用已知 PID 快速检测，在 PID 失效后重新查找目标进程，确认不存在才启动；后续合并应用检查、批量写入运行状态并精简设备指标采集。

## 问题依据

Review 基线为 `buckyos main@505e35152`。主循环每轮休眠 5 秒后检查服务；实际周期还包含本轮工作的耗时。

- [service_pkg.rs](../src/kernel/node_daemon/src/service_pkg.rs) 的 `service_process_pids()` 每次调用 `System::new_all()`，随后又执行 `refresh_processes(All)`。`new_all()` 已完成一次全量采集，状态检查因此重复扫描全机进程，并采集不需要的 CPU、内存和 I/O 指标（sysinfo 0.31 的 `refresh_processes` 默认带 memory、cpu、disk_usage）。
- 本机日志最近 100 轮样本中，每轮有 14 次原生服务状态检查，对应 28 次全机进程扫描；周期中位数约 6.29 秒，扣除固定休眠后约 1.29 秒。这是墙钟耗时，不代表 CPU 时间或开发者现场的 CPU 占比。2026-10-11 复核 86 轮：14 个服务全部走原生 status，0 次脚本 status，步长约 6.3 秒，结论不变。
- `process_matches()` 对每个候选进程重复规范化同一个服务路径，进一步放大开销。每个候选进程最多 2 次 `canonicalize(spec.executable)`，本机约 485 个进程，一次扫描约千次文件系统调用。
- review 补充：`service_process_pids()` 是同步阻塞调用，而 `node_main` 用 `for_each_concurrent(4)` 并发检查 kernel 项，等于最多 4 次全量扫描同时占住 tokio 工作线程。
- review 补充：`is_pid_running()`（service_pkg.rs 约 650 行）同样 `System::new_all()`，停服务时在 `stop_process_tree` 里最多调用 3 次。不在热路径，但应一并改为单 PID 检测。
- review 补充：扣除休眠后的约 1.29 秒并非全部来自进程扫描，还包含 `load_node_config`、SN zone info 刷新、kernel 项每轮的 `ensure_exact_pkg_meta_indexed` 与 `pkg_env.load`、应用的 docker inspect 等。对优化效果的预期要以实测为准。

## 优先实施 PID 快速检测与失效后发现

确认采用以下流程，覆盖用户在两次轮询之间手工杀掉旧进程并启动新实例的情况：

1. 有已知 PID 时，只刷新和检查该 PID，校验进程存活与启动时间。僵尸、已退出进程或被复用的 PID 不算目标实例。
2. 没有 PID 记录，或已知 PID 失效、身份不符时，扫描当前目标进程。发现用户手工启动的匹配实例后，更新记录并继续管理，不重复启动。
3. 扫描确认目标不存在，且目标状态仍要求运行时，启动服务并保存新进程身份。
4. `node_daemon` 自身重启后，通过进程发现恢复已有服务的记录。

扫描与启动之间的竞态由服务现有的启动失败机制处理。启动失败或新进程退出后，下一轮仍按上述流程重新检查、发现并接管目标实例。**不为这个边界新增实例锁、文件锁或分布式锁。**

- [x] 将受管进程记录保留在跨轮存活的管理状态中；不能只存入每轮重新创建的 `ServicePkg` 或 `KernelServiceRunItem`。
- [x] 记录服务身份、PID、启动时间和必要的可执行文件信息；服务移除、目标版本或路径改变时使相应记录失效。
- [x] 快速检测只读取指定 PID 的必要字段，不调用 `System::new_all()` 或全表刷新。正确处理已退出 PID，不能误用 `sysinfo` 中残留的旧条目。
- [x] 发现路径使用最小字段的进程扫描，匹配策略采用下文 review 建议的现有三级规则。按此选择，纯进程名造成的误接管不在本次性能优化中收紧。
- [x] 将目标可执行文件路径规范化移到候选进程遍历之外，减少重复文件系统调用与字符串分配。
- [x] 合并同轮冷启动发现中可共享的扫描。启动前的“不存在”结论必须来自本次发现，不能使用上一轮或本轮较早阶段的空结果直接启动。
- [x] 保留已有的停止、退出、部署及端口健康检查语义；仅在目标要求运行且不存在匹配实例时启动。

主要入口：[service_pkg.rs](../src/kernel/node_daemon/src/service_pkg.rs)、[kernel_mgr.rs](../src/kernel/node_daemon/src/kernel_mgr.rs)、[node_daemon.rs](../src/kernel/node_daemon/src/node_daemon.rs)。

### review 补充：实施要点

**sysinfo 0.31 按 PID 刷新的坑**

- `ProcessRefreshKind::new()` 把 exe、cmd 设为 `UpdateKind::Never`。用 `System::new()` 加 `refresh_processes(ProcessesToUpdate::Some(&[pid]))` 拿到的进程 `exe()` 为 `None`、`cmd()` 为空。如果身份校验依赖这两项，会每轮失败并退回全表扫描，优化被静默抵消。需要 exe/cmd 时必须显式写 `refresh_processes_specifics(Some(&[pid]), ProcessRefreshKind::new().with_exe(UpdateKind::Always).with_cmd(UpdateKind::Always))`。
- `ProcessesToUpdate::Some(pids)` 在 Linux 实现里不清理死进程条目，只有 `All` 会走 `clear_procs`。每次检测用新建的 `System`，或检查 `refresh_processes` 的返回计数，不要复用长期存活的 `System` 查已退出 PID。
- 仓库内已有按 PID 刷新的用法可参考：`src/kernel/buckyos-api/src/kevent_ringbuffer.rs` 约 841 行。
- [x] 抽出单 PID 检测工具函数，供快速路径与 `is_pid_running()` 共用。

**身份校验口径：快速路径只比 pid + start_time**

- 升级覆盖二进制后，Linux 的 `/proc/<pid>/exe` 读出来带 ` (deleted)` 后缀，`canonicalize` 失败，严格比较 exe 会每轮失败；退回发现路径后又按 cmd0 或进程名把同一个进程接管回来，形成每轮一次全表扫描的退化循环。
- 因此：记录时保存 pid 与 `start_time()`，快速路径只校验进程存活、非僵尸、start_time 与记录一致。exe 与 cmd 匹配只属于发现路径，发现成功时顺带记录 exe 供日志与排错用，不作为每轮校验条件。
- “目标版本或路径改变时使记录失效”只是让记录失效并走一次发现，不等于重启旧进程。现有 `status()` 对任何匹配进程都返回 Started、不区分版本，本次保持这一语义不变；版本变化时的重启属于另外的需求。
- [x] 在记录结构与检测函数上写清上述口径，避免实现时顺手比对 exe。

**跨轮状态落点**

- `KernelServiceRunItem` 与 `ServicePkg` 都在 `node_main`（node_daemon.rs 约 1238 行）每轮新建，无法承载记录。
- 建议在 service_pkg.rs 放一个进程级注册表，`OnceLock<Mutex<HashMap<String, ManagedProcess>>>`，键为服务名（`pkg_id` 去版本后的唯一名）。`native_start` 的 `spawn_detached` 返回 pid 时（service_pkg.rs 约 333 行）立即登记 pid 与 start_time；`native_stop` 成功后清除记录。
- 通过 `start` 脚本启动的服务拿不到 pid，启动后的第一轮必须走发现路径，发现后再登记。本机当前 14 个 kernel 服务均无脚本，但这条路径仍存在。
- [x] 注册表与登记/清除时机按上述落地；服务从 node_config 移除时清除记录。

**覆盖范围说明**

- 快速路径不覆盖 `use_op_subcommand` 的服务（目前仅 smb-service），它们每轮仍执行 `<exe> status` 子命令，这是现状，本次不改。
- repo-service（4000）与 system-config（3200）的端口探测每轮保留，开销很小。
- 发现路径的扫描仍是同步阻塞调用，且可能 4 个并发。
- [x] 发现路径的全表扫描放进 `spawn_blocking`，或把扫描收敛为单次后再分发给同轮的多个服务。

### review 补充：匹配口径裁决

**发现路径的匹配口径。** 现有 `process_matches()` 是三级匹配：exe 规范化路径相等、cmd0 绝对路径相等或 cmd0 名字落在别名集合、进程名落在别名集合。第三级是纯进程名匹配，TODO 中“避免把同名无关进程接管为服务”意味着要去掉或收紧它。收紧后，由脚本或手工从其他路径启动的实例将不再被识别为目标，这是行为变更。

- 建议：本次发现路径沿用现有三级匹配，不收紧；快速路径靠 pid + start_time 本身不会误认，误接管风险只存在于发现那一轮，且与现状相同。收紧匹配作为独立改动另行评估。
- [x] 用户于 2026-10-12 明确确认：沿用现有三级匹配，本次只优化性能。同名无关进程误接管仍是原有发现路径的行为边界。

## 后续减少其他重复工作

### 合并应用状态和健康检查

[app_mgr.rs](../src/kernel/node_daemon/src/app_mgr.rs) 的 `get_state()` 分别调用 `status()` 和 `deployment_health()`，后者再次查询状态。正常 Docker 容器通常每轮执行两次 container inspect 和两次 image inspect。

- [x] 一次查询同时返回运行状态、健康度和运行实例身份，Docker、Agent、HostScript 路径复用该结果。
- [x] 仅在容器现有信息不足以匹配目标时查询镜像 digest，并按 image ID 缓存；容器或镜像变化时更新。
- [x] 正常容器路径尽量收敛为每轮一次 container inspect。测量时分别统计 `node_daemon`、Docker CLI 和 dockerd 的 CPU 开销。

主要入口：[app_loader.rs](../src/kernel/node_daemon/src/app_loader.rs)。

### 批量写入运行状态

正常运行的服务也会写入 `PlannedStart` 和 `AlreadyRunning` 两个状态；每次更新都重新读取、解析、序列化和替换整个 `run.plist`。

- [x] 在内存中汇总运行状态，每轮统一落盘；实际发生启动、停止、部署或失败时保留必要的状态更新。
- [x] 已经运行的服务不再先写入 `PlannedStart`，保留最终状态与必要的观测时间。

review 补充：

- `run_plist.rs` 已有进程内互斥锁 `RUN_PLIST_LOCK`，并发写不是问题，改动只为减少整文件重写次数。
- `buckyos stop`（`src/kernel/buckyos-api/src/node_control.rs` 的 run.plist 读取部分）依赖该文件的 `item_name`、`item_kind`、`run_state` 生成停机清单。批量落盘必须保证每轮最终状态仍写入文件，且项目集合完整，不能因为“本轮无变化”而跳过首次写入。

主要入口：[run_item.rs](../src/kernel/node_daemon/src/run_item.rs)、[run_plist.rs](../src/kernel/node_daemon/src/run_plist.rs)。

### 精简设备指标采集

约 30 秒一次的设备上报也执行 `System::new_all()` 后再 `refresh_all()`，但并不消费进程列表；这是独立于 5 秒保活检查的额外开销。

- [x] 复用 CPU 和内存采样器，只刷新需要的字段，去掉设备上报中的全机进程采集。
- [x] 保留跨轮 CPU 采样基线，确保 CPU 使用率由有效采样间隔计算。
- [x] 缓存 OS、CPU 品牌和 GPU 能力等静态信息，磁盘拓扑低频刷新；容量、使用率等动态指标继续更新。

review 补充：

- CPU 采样基线不是单纯优化，而是修正：现在 `new_all()` 与 `refresh_all()` 之间没有采样间隔，`cpu.cpu_usage()` 读到的值无效。
- 该代码在 buckyos-base 的 name-lib 中，`DeviceInfo::auto_fill_by_system_info()`（约 584 行）和 `DeviceMiniInfo::auto_fill_by_system_info()`（约 1304 行）是两份实现，均需改动。
- buckyos 通过 git 依赖 buckyos-base 的 main 分支，改动需先提交 buckyos-base，再在 buckyos 里 `cargo update -p name-lib` 并验证。
- [x] 两份 `auto_fill_by_system_info()` 同步改动；采样器放在调用方跨轮存活的状态里，不放在每次新建的 `DeviceInfo` 中。

主要入口：`node_daemon.rs` 的设备上报分支，以及依赖 `buckyos-base/src/name-lib/src/device.rs` 的 `DeviceInfo::auto_fill_by_system_info()`。

## 验收

- [x] 所有受管 PID 有效时，隔离测试确认原生服务保活路径全表扫描次数为 0；CPU 对比见文末。部署后约 5 秒周期的整轮日志与峰值测量仍待进行。
- [x] 隔离进程测试覆盖旧进程退出后的重新启动、外部替换实例接管及无重复启动。
- [x] 清空进程注册表模拟 daemon 重启后可接管已有服务；启动时间不符及僵尸进程被拒绝。未强制制造操作系统 PID 复用；沿用三级匹配，因此不承诺发现阶段拒绝同名无关进程。
- [x] 在最终发现与 spawn 之间注入外部实例，重复子进程因端口占用退出，随后接管外部 PID；测试钩子仅在 cfg(test) 下存在，无新增生产锁。
- [x] 目标状态为停止时不会自动拉起；版本或可执行路径变化后不会继续使用旧缓存。
- [x] 覆盖二进制（模拟升级，`/proc/<pid>/exe` 带 ` (deleted)`）后，快速路径仍命中，不退化为每轮扫描。
- [x] 后续优化保持容器健康度、镜像匹配、运行状态文件和设备指标的正确性；`buckyos stop` 仍能从 run.plist 得到完整清单。
- [ ] 部署后测量整轮 CPU 峰值、Docker CLI/dockerd CPU 与主循环墙钟耗时。已完成相同 14 个服务、同一宿主机进程环境下的隔离 CPU/扫描/耗时对比，及 mock Docker 调用次数断言。
- [x] 本地依赖覆盖验证及发布后的普通 Git 依赖复验均通过，包括完整 node_daemon 单测、name-lib 设备测试、run.plist 读取端测试与 node_daemon 可执行文件构建；隔离子进程覆盖服务退出、手工替换与清空 daemon 注册表后的恢复。真实 Zone 部署与重启验收仍待进行。

### review 补充：测量方法

本机基线（2026-10-11，16 核，约 485 个进程）：步长约 6.3 秒，每轮 14 次原生 status。实施前后用同样的命令对比：

```bash
LOG=$(ls -t /opt/buckyos/logs/node_daemon/node_daemon.*.log | head -1)
# 每轮原生 status 次数（按 pkg 分组）
tail -2000 "$LOG" | grep "native fallback: pkg=.*op=status" | sed 's/.*pkg=\([^ ]*\).*/\1/' | sort | uniq -c
# 步长中位数（秒）
tail -5000 "$LOG" | grep "main loop step" | cut -c1-18 \
  | python3 -c 'import sys,datetime,statistics as s; t=[datetime.datetime.strptime("2026-"+l.strip(),"%Y-%m-%d %H:%M:%S.%f") for l in sys.stdin]; print(s.median([(b-a).total_seconds() for a,b in zip(t,t[1:])]))'
# 进程 CPU 时间（取两次差值）
PID=$(pgrep -x node_daemon | head -1); cat /proc/$PID/stat | awk '{print "utime+stime(ticks):", $14+$15}'
```

- [ ] 实施后记录新的步长中位数、每轮扫描次数与 CPU ticks 增量，与基线并列写入本节。


## 2026-10-12 实施记录

### 代码与行为

- `service_pkg.rs`：进程级注册表按包环境和服务身份保存目标包、规范化路径、PID、启动时间及诊断用 exe。已知实例只按存活状态与 PID/start_time 验证，不把 exe 作为快速路径条件。使用新建 `System` 的指定 PID 最小字段刷新，停止等待复用同一个检测函数。
- `kernel_mgr.rs` / `node_daemon.rs`：每轮按 kernel 集合清除移除项，并保留单独保活的 system-config 与 cyfs-gateway。冷发现通过 `spawn_blocking` 扫描 exe/cmd，过滤线程 TID，仅记录真正的进程，多个服务共享正结果；缓存中的空结果不直接用于启动，启动前重新发现。端口失败时保留原有停止并重新启动逻辑；健康的外部替换实例直接接管。未新增实例锁。
- 保留 smb-service 的 status 子命令和 repo-service / system-config 的端口检查。`main loop work` 日志新增 `elapsed_ms`、`process_scans`、`pid_checks`、`fast_hits`，可在部署后区分整轮耗时与扫描次数。sysinfo 0.31 的指定 PID 刷新在 Linux 上仍遍历 `/proc` 目录；这里的 0 次扫描指不做 `ProcessesToUpdate::All` 的全表字段采集，不宣称零目录遍历。
- `app_loader.rs` / `app_mgr.rs`：一次观察返回运行状态、健康度、容器 ID 与运行身份。Docker、Agent、HostScript 共用这次结果；仅身份信息不足时查询镜像 RepoDigests。按 image ID 跨轮缓存，目标 digest 改变时重查，5 分钟过期，最多 256 项；失败和空元数据不缓存。正常容器路径一次 container inspect，标签或 image ID 已匹配时不执行 image inspect。
- `run_plist.rs` / `run_item.rs`：进程内缓存完整状态表，正常轮询在 node_main 结束时统一原子落盘；首次无变化也写入，保留历史项目。启动、停止、部署和失败状态立即写入，取消正常观察前的 PlannedStart/PlannedStop。文件协议未变。
- `buckyos-base/src/name-lib/src/device.rs`：完整和 mini 设备信息共用 `DeviceSystemInfoSampler`；node_daemon 主循环持有采样器。仅刷新 CPU/RAM，保留有效 CPU 间隔，缓存 OS/CPU/GPU 能力；GPU 使用量和磁盘剩余空间每次更新，磁盘拓扑每 5 分钟刷新。sysinfo 0.31 Linux 的 `Disk::refresh` 不重读总容量，文件系统扩容后的总容量最迟 5 分钟可见。现有无参入口仍可用于一次性调用。

没有新增第三方依赖，也没有修改对外协议、设备 JSON 字段或 run.plist 格式。

### 验证结果

本地配套验证使用 `/tmp/node-daemon-cpu-local.toml`，将 buckyos-base 的现有 Git 依赖临时 patch 到同级工作树；仓库 manifest 未写入机器专属路径。以下命令在 `buckyos/src` 执行：

```bash
cargo test --offline --config /tmp/node-daemon-cpu-local.toml -p node_daemon -- --test-threads=1
cargo test --offline --config /tmp/node-daemon-cpu-local.toml -p name-lib device::tests:: -- --test-threads=1
cargo test --offline --config /tmp/node-daemon-cpu-local.toml -p node_daemon benchmark_fourteen_managed_service_checks -- --ignored --nocapture --test-threads=1
```

- node_daemon：98 passed，3 ignored；忽略项中的性能测试单独运行通过，端口服务 fixture 由竞态测试启动。
- name-lib 设备模块：12 passed，含 CPU 采样基线、无进程采集、完整/mini 一致性及磁盘拓扑刷新测试。
- 8 个应用观察回归测试验证 container/image inspect 次数、健康度变化、容器与镜像替换、目标 generation/digest 变化、缺失元数据和失败重试。
- 进程测试使用临时复制的 sleep 程序，覆盖跨轮保留、共享冷发现、服务移除、退出/僵尸、手工替换、重启恢复、版本/路径失效、停止状态、端口异常恢复及被覆盖二进制。另用测试程序自身作为绑定端口的隔离服务，精确注入最终 scan/spawn 之间的竞态，验证重复进程失败后接管外部 PID；工作线程退出不触发新扫描。
- run.plist 测试覆盖首次完整写入、立即写入状态转换、嵌套批次、写入失败后重试；另运行 `cargo test --offline --config /tmp/node-daemon-cpu-local.toml -p buckyos-api run_plist -- --test-threads=1`，读取端 2 个测试通过。
- `cargo build --offline --config /tmp/node-daemon-cpu-local.toml -p node_daemon` 通过，已验证非测试入口构建。修改文件通过 rustfmt 与 git diff --check。未运行完整 workspace 测试或包含 SDK/Web 的全量 buckyos-build。

### 发布后普通 Git 依赖验证

`buckyos-base/main` 发布后，已在 `buckyos/src` 执行 `cargo update -p name-lib`，未传入本地 patch 配置。`Cargo.lock` 中 6 个 buckyos-base 包均使用 `git+https://github.com/buckyos/buckyos-base.git?branch=main#652978b211dd31e78755634a6a12fd05cef367d8`，没有 `patch.unused` 残留，依赖声明继续使用 `branch = "main"`。

以下复验均不使用临时本地依赖覆盖：

- `cargo test --locked -p node_daemon -- --test-threads=1`：98 passed，3 ignored。
- `cargo test --locked -p name-lib device::tests:: -- --test-threads=1`：12 passed。
- `cargo test --locked -p buckyos-api run_plist -- --test-threads=1`：2 passed。
- `cargo build --locked -p node_daemon`：通过。

以上命令均退出码 0；`git diff --check` 通过。未运行完整 workspace 测试或包含 SDK/Web 的全量 buckyos-build，未替换或重启真实 Zone。

### 隔离性能对比

同一次测试创建 14 个独立服务进程，sysinfo 报告约 1402 个进程／线程任务条目（包含 Linux task，不等同于 1402 个独立进程），连续执行 5 轮，每轮 14 次检查。旧路径复现 `System::new_all()` + `refresh_processes(All)` 以及候选进程匹配；新路径预先发现实例后检查记录的 PID。计时不含 5 秒休眠、包读取、网络请求、Docker 与设备上报。

| 5 轮合计 | 旧路径 | PID 快速路径 |
| --- | ---: | ---: |
| 全表扫描次数 | 140 | 0 |
| 进程 CPU ticks（utime + stime） | 2133 | 117 |
| 墙钟耗时 | 3167.439 ms | 99.007 ms |

CPU ticks 下降约 94.5%，检查墙钟耗时下降约 96.9%。CPU 时间包含进程所有线程，因此可大于墙钟耗时。这不是原基线 485 个进程下的部署后主循环结果，不能据此宣称主循环已从 6.3 秒降至某一数值。Docker 命令次数由 mock 验证，未测 Docker CLI/dockerd 实际 CPU 或峰值。

### 发布与部署验收进度

- [x] 用户于 2026-10-12 明确授权仅提交并推送 `buckyos-base/src/name-lib/src/device.rs` 到 `base/main`，并完成依赖更新。设备采样改动已提交并推送为 [`652978b211dd31e78755634a6a12fd05cef367d8`](https://github.com/buckyos/buckyos-base/commit/652978b211dd31e78755634a6a12fd05cef367d8)，已确认远端 `main` 指向该提交；本次提交仅包含 `device.rs`。
- [x] 在 buckyos 中执行 `cargo update -p name-lib`，已确认全部 buckyos-base 包通过普通 Git main 依赖使用上述发布提交。
- [x] 不使用临时 patch 完成测试与 node_daemon 可执行文件构建复验，结果见“发布后普通 Git 依赖验证”。
- [ ] 在真实 Zone 部署并记录新的主循环步长中位数、CPU ticks/峰值、扫描数，以及 Docker CLI/dockerd CPU。未替换或重启当前运行环境。

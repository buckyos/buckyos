# BuckyOS Tool App 模块开工前 Control Panel 改造 TODO

> 状态：待实施  
> 基线日期：2026-08-22  
> 产品需求真相源：`doc/buckyos_tool/modules/app.md`  
> 现有 Installer 设计清单：`notepads/app-installer-base-interface-todo.md`、
> `notepads/app-installer-v0.5-implementation-todo.md`

本文只列出 **buckyos-tool 的 app 模块开始实现前**，Control Panel 及其直接依赖的基础服务
必须补齐的能力。它不是 CLI 实现计划，也不重复 App Installer v0.5 已完成的通用 PIKG、
resolver、planner 和 deploy 工作。

Beta 2.2 是 breaking change：改造时不保留旧 `apps.install/apps.update` 双轨兼容层，也不允许
CLI 通过解析内部 Task JSON、system-config key 或 `app_instance_id` 拼接规则临时绕过缺口。

---

## 0. 开工门禁

满足下列门禁后，buckyos-tool 才能开始实现对应范围：

### Gate A：可以开始 CLI 类型和只读 client

- [ ] `InstallPlan` 文件语义、schema version、fingerprint、来源绑定和失效规则已经冻结。
- [ ] App 名称解析、安装作用域和歧义处理规则已经冻结。
- [ ] typed plan/status/update-availability 的服务协议已经冻结，不要求 CLI 读取内部 Task 数据。
- [ ] 协议同步到 `buckyos-api` 共享类型、`doc/App 安装协议.md` 和
  `doc/control_panel/Control_Panel_Service.md`。

### Gate B：可以实现 `app fetch/install/upgrade/status`

- [ ] 本文全部 P0 完成。
- [ ] 非管理员用户能创建、读取、确认、等待、重试和取消自己发起的 App Task。
- [ ] `fetch -> InstallPlan 文件 -> 首次安装` 和“不带 plan 的升级”均有服务端集成测试。
- [ ] 安装/升级成功能够证明目标版本已进入 scheduled 和 runtime，而不是旧实例仍为 Started。

### Gate C：可以宣称 app 模块首版完成

- [ ] 本文 P1 生命周期、批量升级和恢复项完成。
- [ ] DV 覆盖 Catalog、PIKG、升级、回滚、Control Panel 重启恢复和普通用户任务权限。
- [ ] `cargo test`、BuckyOS build 和 app installer DV 均通过。

---

## 1. 当前阻塞证据

| 优先级 | 当前实现 | 对 buckyos-tool 的阻塞 |
| --- | --- | --- |
| P0 | `TaskMgrInstallStore` 把事务快照和 `{percent}` 都写入 TaskManager `progress` | 进入 WaitingForApproval 后完整 plan 会被百分比覆盖，confirm/recovery 无法可靠读取 |
| P0 | 创建任务时忽略业务 `user_id`，owner 校验却使用 Task creator | 普通用户可能无法确认、等待、重试或取消自己刚创建的任务 |
| P0 | `apps.install` 固定走新装，`apps.update` 固定走升级 | 不支持“有 plan 才新装、无 plan 才升级”的动作矩阵 |
| P0 | 升级没有 installed/target identity 比较 | 相同版本会重部署，低版本没有稳定拒绝，`AppUpdateAvailability` 未接入 |
| P0 | 升级从全新安装默认参数重建 spec | mount、环境、资源池、权限选择、实例数和停止状态可能被重置 |
| P0 | Activate 只等待稳定路径下任意 `Started` 报告 | 旧版本报告可让新版本升级任务提前成功 |
| P0 | 没有 plan-only typed RPC，也没有公开 install status RPC | `fetch --plan`、`--dry-run` 和 CLI 确认摘要没有稳定数据来源 |
| P0 | PIKG 只有内部/通用上传路径，没有 App staging 完整协议 | 缺少 digest 回执、expiry、cleanup 和 plan/source 重新绑定 |
| P1 | 生命周期仍依赖 `app_instance_id`；stop 非 Task；restart 缺失 | 不能满足 App 名称选择器和统一任务跟踪 |
| P1 | start/uninstall 只在进程内 `tokio::spawn` | Control Panel 重启后任务可能永久悬挂 |

主要证据入口：

- `src/frame/control_panel/src/app_install_engine.rs`
- `src/frame/control_panel/src/app_install_driver.rs`
- `src/frame/control_panel/src/app_install_planner.rs`
- `src/frame/control_panel/src/app_install_deployer.rs`
- `src/frame/control_panel/src/app_install_runner.rs`
- `src/frame/control_panel/src/app_installer.rs`
- `src/frame/control_panel/src/app_servcie_mgr.rs`
- `src/frame/control_panel/src/main.rs`
- `src/kernel/buckyos-api/src/app_install.rs`
- `src/kernel/buckyos-api/src/app_mgr.rs`
- `src/kernel/task_manager/src/server.rs`

---

## 2. P0：冻结面向 Tool 的共享契约

### P0.1 InstallPlan 与 Inspection 分工

- [ ] 明确计划文件保存的是不可变安装语义，不保存 staging handle、服务端临时路径、token、
  明文 Secret 或动态进度。
- [ ] `InstallPlan` 至少绑定 App DID、AppDoc Object ID、PIKG digest（若有）、target snapshot、
  InstallParams、selected packages、最终配置和 fingerprint。
- [ ] 将 readiness、content location、estimated bytes、warning 等动态结果放入 typed
  `InstallInspection/InstallPlanStatus`，避免获取内容后悄悄改变已经确认的 Plan。
- [ ] 增加计划用途或动作字段，至少区分 `FreshInstall`、`Upgrade`、`Satisfied`；计划文件只允许
  用于 `FreshInstall`，默认升级计划只在当次升级预检和确认中使用。
- [ ] 计划 fingerprint 必须覆盖 App identity、source identity、target、参数、选择内容和最终
  配置；任何一项变化都必须产生新 fingerprint。
- [ ] 冻结 `PLAN_REQUIRED`、`PLAN_NOT_APPLICABLE`、`PLAN_STALE`、
  `DOWNGRADE_NOT_ALLOWED`、`AMBIGUOUS_APP_TARGET` 等稳定错误语义。
- [ ] bump `APP_INSTALL_SCHEMA_VERSION`；beta 2.2 不兼容读取旧计划或旧事务快照。

验收：

- [ ] 只看共享类型即可区分用户输入、Installer 推导结果、动态状态和运行结果。
- [ ] CLI 可以把 typed Plan 序列化成 JSON，再无损提交回来；服务端不信任客户端填写的 OS、
  arch、capabilities 或最终 AppSpec，必须重新校验。
- [ ] 与 `app-installer-base-interface-todo.md` 中 Target/Plan/Approval 未完成项合并收口，不再
  保留两套相互矛盾的类型草案。

影响入口：

- `src/kernel/buckyos-api/src/app_install.rs`
- `doc/App 安装协议.md`
- `doc/control_panel/Control_Panel_Service.md`

### P0.2 冻结 typed 服务协议

协议设计阶段先确定语义，再决定最终 RPC 名称。至少需要以下能力：

- [ ] `fetch/inspect`：解析 Catalog 或 staged PIKG，返回 App identity 和默认
  `InstallInspection`，不创建安装 Task。
- [ ] `plan recompute`：提交 target selector / InstallParams，返回新 plan 和新 fingerprint；
  不能在 confirm 内一边改参数一边批准用户没看过的计划。
- [ ] `install`：首次安装时接收 plan + source identity；已安装目标不允许消费首次安装计划。
- [ ] `upgrade inspect/submit`：根据已安装状态生成默认升级计划并确认；支持 Catalog 与 PIKG
  来源，不要求 CLI 构造 AppSpec。
- [ ] `install status`：按 task_id 返回 typed stage、plan/inspection、approval request、
  verification、error、available actions 和结果。
- [ ] `app status`：按 App DID + 安装作用域聚合 installed record、desired、active task、
  scheduled、runtime 和 readiness。
- [ ] `update availability`：单个和批量返回 installed/target AppDoc identity、版本、权限 diff、
  target compatibility 和可信状态。

协议不得要求调用方解析 TaskManager `message/progress/result` 的内部布局；Control Panel 负责把
内部 Task 映射为稳定 typed response。

---

## 3. P0：修复 TaskManager 事务持久化与权限

### P0.3 分离事务快照与展示进度

- [ ] `TaskMgrInstallStore::write_data` 和 `set_status` 不再互相覆盖同一个 JSON 字段。
- [ ] 定义单一持久 envelope，例如“完整 transaction + display progress”，所有写入保持完整
  schema；禁止单独写 `{percent}` 后让 `view_from_task` 把它当 `AppInstallTaskData`。
- [ ] `view_from_task` 明确 Pending/Running/Waiting/terminal 各阶段从哪里读取事务和终态结果，
  不再使用 `result.or(progress).unwrap_or(input)` 猜测数据类型。
- [ ] WaitingForApproval 后可完整读取默认 plan、fingerprint 和 approval request。
- [ ] Completed 后 `result` 保存 typed install result/status snapshot，不得只剩百分比。
- [ ] 任一 Stage 执行中崩溃，startup scan/sweep 能从最后一次完整事务快照恢复。
- [ ] 失败路径先持久化结构化错误，再写 Task terminal 状态；即使状态更新失败也不能丢事务。

### P0.4 绑定业务发起者与 Task ACL

- [ ] 创建 Task 时不再丢弃 `user_id/app_id` 业务身份。
- [ ] 明确采用 TaskManager delegated creator、on-behalf-of 或显式 ACL grant 中的哪一种；不得
  用 Control Panel 服务 token 的 creator 冒充最终业务 owner。
- [ ] owner/admin 校验基于经过认证的业务 principal 和冻结的 task ownership，不依赖自由文本。
- [ ] 普通用户能够 `task get/wait` 自己发起的 Task，并调用 confirm/retry/cancel。
- [ ] 其他普通用户不能读取或控制该 Task；Admin 行为保留审计记录。
- [ ] runner 继续使用 Control Panel 服务身份写进度，但 runner 权限不能扩大到业务调用者权限。

必须新增使用真实 TaskManager 语义的集成测试；只使用会 deep-merge 的 fake store 不算覆盖。

影响入口：

- `src/frame/control_panel/src/app_install_engine.rs`
- `src/frame/control_panel/src/app_install_runner.rs`
- `src/kernel/buckyos-api/src/task_mgr.rs`
- `src/kernel/task_manager/src/server.rs`（仅在现有 ACL/委托能力不足时修改）

---

## 4. P0：补齐默认计划、首次安装和升级判定

### P0.5 提供无安装副作用的默认计划能力

- [ ] Catalog 来源可以直接 Resolve/Inspect 并返回默认首次安装计划。
- [ ] PIKG 来源经受控 staging 后可以 Inspect；生成计划不得写 AppSpec、install record、proof
  或启动安装 Task。
- [ ] 默认首次安装计划只预选允许的默认项；必填配置缺失时返回 typed config issues，不能由
  CLI 或 Prepare 阶段猜值。
- [ ] Tool 修改 target selector / InstallParams 后由 Installer 重新计算 Plan 和 fingerprint。
- [ ] `--dry-run` 所需的 plan-only 路径不创建远程安装 Task。
- [ ] 提交计划时重新解析 source identity；AppDoc revision、PIKG digest、target snapshot 或
  final config 不一致时返回 `PLAN_STALE`，不静默重算后继续执行。

### P0.6 统一新装/升级动作矩阵

- [ ] 在一个权威判定点完成“App DID + 安装作用域 + plan 是否存在 + target identity”的动作
  决策，RPC handler、planner 和 deployer 不得分别猜测。
- [ ] 未安装 + 有 plan：首次安装。
- [ ] 未安装 + 无 plan：`PLAN_REQUIRED`。
- [ ] 已安装 + 有 plan：`PLAN_NOT_APPLICABLE`。
- [ ] 已安装 + 无 plan + target 更新：升级。
- [ ] 已安装 + 无 plan + identity/version 相同：同步 `Satisfied`，不创建空 Task、不重部署。
- [ ] 已安装 + 无 plan + target 更旧：`DOWNGRADE_NOT_ALLOWED`。
- [ ] 版本权威判定使用 AppDoc Object ID / document version 等不可变发布 identity；semver 只做
  展示和明确的升降级辅助，不能从 Repo 中自行挑一个“最高版本”。
- [ ] Catalog、local PIKG 和 URL PIKG 都进入同一动作判定；删除“install_package 只能新装、
  apps.update 只能 Catalog”的割裂。

### P0.7 默认升级计划必须继承当前安装状态

- [ ] 从当前 `AppServiceSpec` + `install_record` 还原升级基线 InstallParams。
- [ ] 保留 selected components、permission selection、data/cache/external mounts、
  Service Settings、bash envs、resource pool、expected instance count 和 App class。
- [ ] 保留当前启停期望；升级一个已停止 App 不得自动启动。
- [ ] 只合入新版本强制要求的变化；新增权限和不兼容配置必须进入 diff 并重新确认。
- [ ] Prepare/Deploy 只消费已批准升级计划，不重新调用全新安装的 `InstallParams::default()`。
- [ ] rollback 使用冻结的 previous spec/record，并明确恢复后的实际版本和运行状态。

影响入口：

- `src/frame/control_panel/src/app_install_driver.rs`
- `src/frame/control_panel/src/app_install_planner.rs`
- `src/frame/control_panel/src/app_install_deployer.rs`
- `src/frame/control_panel/src/app_installer.rs`
- `src/kernel/buckyos-api/src/app_install.rs`

---

## 5. P0：完成 PIKG staging 协议

- [ ] 在现有 NDM 上传能力之上冻结 App Installer staging contract，至少返回
  `staging_handle`、`pikg_digest`、size、created_at、expires_at。
- [ ] handle 不可猜测；服务端 canonical path 必须位于受控 staging root，外部请求不能提交
  任意服务端路径。
- [ ] `fetch --plan` 可以使用临时 staged PIKG 做 Inspect；导出的 Plan 只绑定 digest，不保存
  短期 handle。
- [ ] `install` 可以重新上传同一 PIKG；新 handle 的 digest 与 Plan 一致即可，不要求复用已
  过期 handle。
- [ ] staging finalize 后内容不可变；digest 比对失败不能创建 Plan 或安装 Task。
- [ ] 实现 TTL、容量限制、引用中的文件保护、成功/失败/取消后的清理和启动时垃圾回收。
- [ ] URL 由 CLI 获取字节，Control Panel 不接受未经约束的远程 URL 后自行下载。

影响入口：

- `src/frame/control_panel/src/main.rs`（NDM/staging 路由）
- `src/frame/control_panel/src/pikg.rs`
- `src/frame/control_panel/src/app_install_driver.rs`
- `src/kernel/buckyos-api/src/app_install.rs`

---

## 6. P0：让 Activate 证明目标版本已运行

这是跨 Control Panel、scheduler、node-daemon/runtime 的基础协议改造，不能只改
`wait_for_instance_started()`。

### P0.8 冻结 Deployment Identity

- [ ] 定义一次部署的不可变 identity，至少能关联 install task、AppDoc Object ID 和 spec
  revision/generation；PIKG App 还应能关联 digest 或等价内容 identity。
- [ ] identity 从 AppServiceSpec/desired 传到 scheduled instance config，再由实际 runtime
  report 原样上报。
- [ ] Static Web 也必须有带版本 identity 的物化/路由就绪证据，不能只检查目录存在或旧
  ServiceInfo key 存在。
- [ ] 共享类型是 beta 2.2 breaking change，不给缺少 identity 的旧报告做“视为成功”兼容。

### P0.9 收紧 Activate 与 rollback 完成条件

- [ ] Activate 只接受与当前 task 目标 deployment identity 完全匹配的 scheduled/runtime
  evidence。
- [ ] 旧版本遗留的 `Started`、目录、端口或路由记录不能满足新版本 Activate。
- [ ] installed record/proof 和 Task Completed 只能在目标 identity ready 后写入。
- [ ] Activate 超时/失败恢复 previous spec 后，必须等待 previous deployment identity 恢复，
  再把任务标为 RolledBack/Failed；不能报告目标版本成功。
- [ ] status 同时展示 desired target identity 和 observed runtime identity，便于 Tool 明确显示
  `upgrading/rolled_back/version_mismatch`。

影响入口：

- `src/kernel/buckyos-api/src/app_mgr.rs`
- `src/kernel/scheduler/src/app.rs`
- `src/kernel/scheduler/src/system_config_agent.rs`
- `src/kernel/node_daemon/src/app_loader.rs`
- `src/kernel/buckyos-api/src/runtime.rs`
- `src/frame/control_panel/src/app_install_deployer.rs`
- Static Web 的 scheduler/node-daemon/gateway 物化与上报路径

---

## 7. P0：提供稳定的名称选择与状态查询

### P0.10 App DID + 安装作用域选择

- [ ] 对外接受完整 App DID、BNS 短名和权威域名别名；域名形式必须先经名称服务解析，不能
  直接拼成三段 `did:bns:*`。
- [ ] 名称解析为 App DID 后，再在调用方可见的 zone/user 安装作用域中选择目标。
- [ ] 内部可以继续使用 `app_id@user_id` 做存储 key，但正式 App RPC 不要求 Tool 构造该值。
- [ ] 0 个匹配返回 NotInstalled/NotFound；多个匹配返回 `AMBIGUOUS_APP_TARGET` 和脱敏候选
  scope，不默认选 owner、当前用户或第一个结果。
- [ ] 生命周期和 status 使用同一 resolver，不能各自实现不同选择规则。

### P0.11 typed 状态 facade

- [ ] 正式注册并实现 `AppInstallStatusSnapshot` 查询，不让 SDK/Tool 解析 Task JSON。
- [ ] 正式注册并实现 `AppUpdateAvailability` 单项/批量查询。
- [ ] `app details/status` 聚合 install record、desired spec、active task、scheduled config、
  runtime report、readiness、verification 和 source summary。
- [ ] 明确区分 desired version、scheduled version、runtime version、last successful version 和
  rollback version。
- [ ] list/details/status 输出使用同一安装作用域与权限过滤。
- [ ] Catalog fetch 可以返回尚未安装 App 的 AppDoc identity/trust 摘要，不创建 Task。

影响入口：

- `src/frame/control_panel/src/app_servcie_mgr.rs`
- `src/frame/control_panel/src/app_installer.rs`
- `src/frame/control_panel/src/app_install_resolver.rs`
- `src/frame/control_panel/src/main.rs`
- `src/kernel/buckyos-api/src/app_install.rs`

---

## 8. P1：生命周期、批量升级与任务恢复

### P1.1 生命周期统一为可恢复 Task

- [ ] `stop` 返回 Task 或稳定的 synchronous satisfied 结果；不能只返回 `{ok:true}` 且让 CLI
  无法继续跟踪正在收敛的停止动作。
- [ ] 增加 `restart`，冻结 rolling/recreate 默认策略和多实例 readiness 门槛。
- [ ] `uninstall` 要求显式 retain/delete data；服务端不再用 `remove_data=false` 静默默认。
- [ ] start/stop/restart/uninstall 共用持久 runner；删除只靠 `tokio::spawn` 的执行路径。
- [ ] Control Panel 重启后的 startup scan/sweep 能恢复所有非终态 App lifecycle Task。
- [ ] 任务取消、重复请求和 idempotency key 有确定行为。

### P1.2 Catalog 批量升级

- [ ] 无参 upgrade 枚举当前调用方可管理的已安装 App，不包含不可见 user scope。
- [ ] 批量检查只使用权威 Resolve 结果，返回 typed availability 和默认升级 plan 摘要。
- [ ] 已满足项不创建 Task；实际更新项可以形成有明确 child task/result 的批量操作。
- [ ] 单项失败不丢失其它项结果；批量结果可恢复、可继续等待。
- [ ] PIKG/URL 单项升级仍走 `install <source>` 语义，不塞进 Catalog batch upgrade。

影响入口：

- `src/frame/control_panel/src/app_installer.rs`
- `src/frame/control_panel/src/app_install_runner.rs`
- `src/frame/control_panel/src/main.rs`
- `src/kernel/buckyos-api/src/taskdata.rs`

---

## 9. 测试 TODO

### 9.1 单元与组件测试

- [ ] TaskManager production adapter：完整事务写入后再更新百分比，数据仍可读。
- [ ] WaitingForApproval -> read plan -> confirm -> Completed round-trip。
- [ ] 在每个 Stage 写进度后模拟崩溃，恢复时能读取完整 transaction。
- [ ] 普通用户 owner、其他用户拒绝、Admin 审计三种 Task ACL。
- [ ] InstallPlan JSON round-trip、fingerprint tamper、AppDoc revision 变化、target 变化和 PIKG
  digest mismatch。
- [ ] 六格动作矩阵：未装/已装 × plan 有无 × higher/equal/lower。
- [ ] 升级完整保留 mount/env/settings/resource pool/instance count/stopped state。
- [ ] PIKG stage finalize、不可变、过期、GC、引用保护和容量限制。
- [ ] 旧 Started report 不满足新 deployment identity；正确版本报告才完成。
- [ ] Activate 失败后 previous identity 恢复，任务结果为 rolled back/failed。
- [ ] App DID scope 唯一选择、歧义、无权限和域名别名解析。
- [ ] start/stop/restart/uninstall 重启恢复和幂等。

### 9.2 DV 测试

- [ ] Catalog：`fetch -> default plan -> 修改选项 -> plan file -> install -> status`。
- [ ] 本地 PIKG：两次上传 digest 相同，首次 handle 过期后仍可用新 handle 安装。
- [ ] URL PIKG：CLI 获取字节；Control Panel 从未接收或打开客户端 URL/path。
- [ ] 同版本 install 返回 satisfied，无 Task、无 spec revision 变化。
- [ ] 低版本 install 返回 `DOWNGRADE_NOT_ALLOWED`，旧实例不中断。
- [ ] Catalog 升级和 PIKG 升级都保留原配置。
- [ ] 注入旧版本 Started report，升级不得提前成功；目标版本上报后才能完成。
- [ ] 新版本 Activate 失败，旧版本恢复并在 status 中可见。
- [ ] WaitingForApproval、Deploy、Activate 三个时点重启 Control Panel，任务均能恢复。
- [ ] 非管理员用户能完整执行 fetch/plan/install/wait；不能读取其他用户 Task。
- [ ] uninstall retain/delete data 两条路径，restart rolling/recreate 约定路径。

### 9.3 检查命令

```bash
cd src
cargo test -p control_panel app_install --no-fail-fast
cargo test -p task_manager
cargo test -p scheduler
cargo test -p node_daemon
cargo test
uv run buckyos-build.py

cd ..
cd test/app_installer_test
pnpm test
```

最终 DV 还应在 `uv run src/check.py` 正常的完整环境中执行；macOS Docker case 的
`/opt/buckyos` 文件共享限制必须明确记录，不能用跳过 Docker 掩盖协议失败。

---

## 10. 推荐落地顺序

1. **协议与类型**：P0.1-P0.2，先冻结 Plan/Inspection/Status/错误码。
2. **任务真相源**：P0.3-P0.4，先修 Task 持久化和业务 owner，否则后续确认/恢复测试都不可信。
3. **计划与动作判定**：P0.5-P0.7，完成 fetch plan、新装/升级矩阵和配置继承。
4. **PIKG staging**：§5，打通 plan digest 与安装时重新上传。
5. **版本级收敛**：P0.8-P0.9，联动 scheduler/node-daemon/runtime，消除旧版本假成功。
6. **查询 facade**：P0.10-P0.11，给 Tool 稳定 selector/status/update-check。
7. **生命周期与 batch**：P1.1-P1.2，统一可恢复 Task。
8. **单测 + DV + build**：§9 全部通过后，再开始 buckyos-tool app 模块写操作实现。

其中第 5 步是跨服务协议变更，应单独提交并让 scheduler/node-daemon 所有测试先通过；不要把
它隐藏在 Control Panel PR 中。

---

## 11. 明确不采用的临时方案

- [ ] 不让 Tool 直接调用 `apps.update` 来假装已经支持新升级语义。
- [ ] 不让 Tool 读取 `Task.progress` 内部 JSON 找 InstallPlan。
- [ ] 不让 Tool 自行比较 semver 后决定 install/update/downgrade。
- [ ] 不让 Tool 根据用户名拼 `app_id@user_id`。
- [ ] 不用旧版本任意 Started、目录存在或 ServiceInfo 存在作为升级成功证据。
- [ ] 不在升级时调用全新安装默认参数覆盖现有配置。
- [ ] 不把 staging handle 写进长期 InstallPlan。
- [ ] 不以 fake store 测试通过替代生产 TaskManager adapter 测试。
- [ ] 不新增 App 发布能力；`app publish` 不属于 buckyos-tool app 模块。

---

## 12. 完成定义

Control Panel 前置改造完成时，必须能够只通过正式服务接口演示：

```text
Catalog/PIKG source
    -> side-effect-free default InstallInspection
    -> export/import frozen InstallPlan
    -> fresh install with exact plan/source binding
    -> user-owned recoverable Task
    -> exact target deployment evidence
    -> typed final status

Installed App + newer Catalog/PIKG source
    -> default upgrade plan seeded from current settings
    -> confirm
    -> old version remains until switch
    -> exact new version readiness or explicit rollback
    -> typed final status
```

并满足：

- [ ] 不需要 buckyos-tool 知道任何 system-config key、Task 内部 JSON 或 AppSpec 构造规则。
- [ ] 不需要用户输入 `app_instance_id`。
- [ ] 首次安装、升级、已满足、拒绝降级和 stale plan 都有稳定机器可读结果。
- [ ] Control Panel 或执行节点重启不会让任务丢失或误报成功。
- [ ] 文档、共享类型、Control Panel、scheduler/node-daemon、SDK 和 DV Test 已同步。

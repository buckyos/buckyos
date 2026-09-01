# BuckyOS Tool App 模块开工前 Control Panel 改造 TODO

> 状态：待实施  
> 基线日期：2026-08-22  
> 产品需求真相源：[`buckyos-websdk/doc/modules/app.md`](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/app.md)<br>
> 现有 Installer 设计清单：`notepads/app-installer-base-interface-todo.md`、
> `notepads/app-installer-v0.5-implementation-todo.md`

本文只列出 **buckyos-tool 的 app 模块开始实现前**，Control Panel 及其直接依赖的基础服务
必须补齐的能力。它不是 CLI 实现计划，也不重复 App Installer v0.5 已完成的通用 PIKG、
resolver、planner 和 deploy 工作。

Beta 2.2 是 breaking change：改造时不保留旧 `apps.install/apps.update` 双轨兼容层，也不允许
CLI 通过解析内部 Task JSON、system-config key 或 `app_instance_id` 拼接规则临时绕过缺口。

### TaskMgr 2.0 执行基线

- TaskManager 是 App Task 的唯一持久真相源，保存不可变输入、事务快照、进度、终态结果、
  ownership 和 durable event。
- App Installer 不依赖 KMSG queue、TaskMgr runner inbox 或 `task_ready` 事件：已鉴权的
  Control Panel RPC 在 Task 持久后直接启动本地 runner，Control Panel 启动扫描和低频
  sweep 从 TaskManager 恢复遗漏任务。
- KEvent 只可作为 Tool/SDK 等待 Task 变化的低延迟加速；正确性始终以 TaskManager snapshot +
  durable event cursor 为准，runner 不依赖事件投递。
- Stage 副作用必须可幂等恢复，TaskManager revision/runner fencing 必须防止过期执行体写回；
  不得为恢复再引入第二套业务队列真相。

---

## 0. 开工门禁

满足下列门禁后，buckyos-tool 才能开始实现对应范围：

### Gate A：可以开始 CLI 类型和只读 client

- [ ] `InstallPlan` 文件语义、schema version、fingerprint、来源绑定和失效规则已经冻结。
- [ ] App 名称解析、安装作用域和歧义处理规则已经冻结。
- [ ] `AppInstallationId` 及 App DID + installation scope 到 system-config/service/runtime key 的
  内部映射已经冻结，不再以 `AppDoc.name` 冒充稳定安装身份。
- [ ] submit/confirm/retry/cancel 的 TaskMgr 2.0 语义、idempotency key 和并发冲突规则已经冻结。
- [ ] typed plan/status/update-availability 的服务协议已经冻结，不要求 CLI 读取内部 Task 数据。
- [ ] 协议同步到 `buckyos-api` 共享类型、`doc/App 安装协议.md` 和
  `doc/control_panel/Control_Panel_Service.md`。
- [ ] [`buckyos-websdk/doc/modules/app.md`](https://github.com/buckyos/buckyos-websdk/blob/main/doc/modules/app.md)
  中的动态 readiness/warning、Plan 可携带性、
  CLI 名称归一化和 Task retry 语义已与上述共享契约对齐。

### Gate B：可以实现 `app fetch/install/upgrade/status`

- [ ] 本文全部 P0 完成。
- [ ] 非管理员用户能创建、读取、确认、等待、重试和取消自己发起的 App Task。
- [ ] 同一不可变请求的重放返回同一 Task，同 App 并发 mutation 不会产生重复 spec、
  `app_index` 或重复部署副作用。
- [ ] `fetch -> InstallPlan 文件 -> 首次安装` 和“不带 plan 的升级”均有服务端集成测试。
- [ ] 安装/升级成功能够证明目标版本已进入 scheduled，且满足约定实例数、
  新鲜度和 health/readiness，而不是旧实例或陈旧 `Started` 报告。

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
| P0 | 存储 key、`app_instance_id` 和 service id 仍以 `AppDoc.name` 为主 | 同 scope 内同名、不同 App DID 会冲突，与产品定义的稳定 App DID 身份矛盾 |
| P0 | 安装 Task 使用随机 idempotency key，同 App 事务和 `app_index` 只靠扫描判断 | RPC 回包丢失或并发请求可以创建重复 Task 或重复节点配置 |
| P0 | `apps.install.retry` 尝试原地复活 Failed Task | TaskMgr 2.0 Terminal 为吸收态，retry 必须创建具有 `retry_of` 关系的新 Task |
| P0 | runner 只有进程内执行守卫，cancel/Deploy/rollback 尚缺完整的 revision fencing | 重启扫描、sweep 或多执行体竞态可能重复产生副作用；不能通过引入另一业务队列解决 |
| P0 | bootstrap/SystemBuiltin App 可以直接存在 AppSpec，没有 Installer Task/install record | list/status 和 lifecycle 无法区分 Installer-managed、bootstrap-managed 与系统内建 App |
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
- [ ] fingerprint 还必须覆盖 installation scope（Zone identity、owner user、AppClass）和目标
  selector snapshot；明确 Plan 哪些部分可跨调用方携带，哪些在换 Zone/用户后必须失效。
- [ ] 冻结 fingerprint 的 canonical serialization：字段顺序、`null`/缺省值、默认值、数字和
  unknown field 的处理必须在 Rust/TypeScript 之间一致；fingerprint 是等值/完整性标识，
  不是授权凭据，服务端仍必须重算和重新授权。
- [ ] 冻结 `PLAN_REQUIRED`、`PLAN_NOT_APPLICABLE`、`PLAN_STALE`、
  `DOWNGRADE_NOT_ALLOWED`、`AMBIGUOUS_APP_TARGET`、`IDEMPOTENCY_CONFLICT`、
  `APP_MUTATION_IN_PROGRESS`、`UNSUPPORTED_SCHEMA_VERSION` 等稳定错误语义。
- [ ] bump `APP_INSTALL_SCHEMA_VERSION`；beta 2.2 不兼容读取旧计划或旧事务快照。
- [ ] 修正产品文档将动态 readiness/warning 写入长期 Plan 文件的表述；Plan 只保存
  不可变语义，`readiness/content_location/estimated_bytes/warnings` 放在带时间和
  target snapshot 的 Inspection/Status 中。

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
- [ ] `submit` 接收调用方稳定生成的 idempotency key/request id；同 principal + key +
  immutable request 重放返回原 Task，输入不一致返回稳定冲突。
- [ ] 冻结“CLI 本地确认”与“Task WaitingForApproval”的分工：普通 submit 是否已视为
  批准，哪些策略仍会进入服务端授权门；`confirm` 只批准已展示 fingerprint，
  不允许同时修改 target/params。
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
- [ ] 事务 envelope 带独立 schema/revision，所有 runner write 使用 TaskMgr 2.0
  `expected_revision`/runner fencing；过期执行体不得覆盖新快照。
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
- [ ] 区分 Task creator/controller 与 App installation owner；Admin on-behalf-of 安装不能将
  Control Panel 服务身份、管理员身份和最终 App owner 混成一个字段。
- [ ] Failed/Completed/Canceled 等 Terminal Task 不原地复活；`retry` 创建新 Task，使用新
  idempotency key 并设置 `retry_of`/parent 关系，旧 Task 保留不可变结果。
- [ ] `cancel` 首先通过 TaskMgr 2.0 control request 持久取消意图；runner 在安全边界 ack，
  Deploy 后必须完成回滚/停止收敛才写 Canceled 终态。RPC 断开不得丢失取消意图。
- [ ] `task wait` 以 TaskManager snapshot + durable event cursor 补齐进度，KEvent 只做唤醒；
  事件丢失、重连或 Control Panel 重启都不影响终态可见性。

### P0.4A TaskMgr 2.0 runner 收敛

- [ ] RPC 只在 TaskManager 成功持久 Task 后才本地 `spawn_run`；客户端收到 `task_id` 即表示
  系统已承诺处理，后续 Control Panel 崩溃不会丢 Task。
- [ ] startup scan 和低频 sweep 只恢复 Pending/Running；WaitingForApproval/Paused 保持等待
  显式业务操作，Terminal 永不重启。扫描必须分页覆盖本 runner 的全部非终态 Task。
- [x] 执行正确性不依赖 KMSG queue、TaskMgr runner inbox、`task_ready` 或 KEvent；当前已删除
  相关配置、权限、队列创建和 ack 依赖。
- [ ] 单进程内重复 sweep 由 task_id 执行守卫合并；如 Control Panel 允许多实例，必须
  冻结“安装 runner Zone singleton”约束或使用 TaskMgr lease/runner epoch 做跨进程 fencing，
  不能用另一消息队列代替该一致性约束。
- [ ] 每个 Stage 恢复时通过 persisted output + deployment identity 判断是短路、继续还是
  回滚，重复执行不会重复分配资源、覆盖新 spec 或写错终态。

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
- [ ] 该权威点同时原子建立 `{Zone, installation scope, App DID}` mutation 序列化关系；
  install/upgrade/uninstall/start/stop/restart 不得对同一安装目标交叉产生副作用。
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
- [ ] `app_index` 及其它共享命名/端口资源通过 system-config CAS、专用序列或等价原子机制
  分配，不使用“扫描 max + 1”或进程内锁冒充 Zone 级一致性。

### P0.7 默认升级计划必须继承当前安装状态

- [ ] 从当前 `AppServiceSpec` + `install_record` 还原升级基线 InstallParams。
- [ ] 保留 selected components、permission selection、data/cache/external mounts、
  Service Settings、bash envs、resource pool、expected instance count 和 App class。
- [ ] 保留当前启停期望；升级一个已停止 App 不得自动启动。
- [ ] 只合入新版本强制要求的变化；新增权限和不兼容配置必须进入 diff 并重新确认。
- [ ] Prepare/Deploy 只消费已批准升级计划，不重新调用全新安装的 `InstallParams::default()`。
- [ ] rollback 使用冻结的 previous spec/record，并明确恢复后的实际版本和运行状态。
- [ ] 冻结升级切换策略：当前是 in-place/recreate、rolling 还是 blue-green；不得在只有一个稳定
  AppSpec/port 的实现上宣称“旧版本保持运行直到新版本 ready”。
- [ ] rollback 材料在目标版本健康且任务终态持久化前不得 GC；明确二进制/spec 回滚
  不等于业务数据迁移回滚，数据不可逆时不能报告为完全回滚。
- [ ] 冻结升级失败终态结果：明确区分 target failed + previous restored、rollback failed 和
  target partially running，不把 RolledBack 混成 Completed。

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
- [ ] staging handle 绑定上传 principal、Zone、digest 和用途，未授权的其他用户/服务不能
  inspect 或 consume；Admin on-behalf-of 的归属和审计必须明确。
- [ ] `fetch --plan` 可以使用临时 staged PIKG 做 Inspect；导出的 Plan 只绑定 digest，不保存
  短期 handle。
- [ ] `install` 可以重新上传同一 PIKG；新 handle 的 digest 与 Plan 一致即可，不要求复用已
  过期 handle。
- [ ] staging finalize 后内容不可变；digest 比对失败不能创建 Plan 或安装 Task。
- [ ] 实现 TTL、容量限制、引用中的文件保护、成功/失败/取消后的清理和启动时垃圾回收。
- [ ] 限额同时按 Zone 和 principal 计算；finalize/consume/GC 使用引用或租约防止活动 Task
  读到已删除文件。staging 只是内容边界，不承担 Task 分发或业务队列职责。
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
- [ ] runtime evidence 还必须带 instance/process epoch、node boot/session identity、`observed_at`
  和 expiry/heartbeat 语义；只有 identity 相等但已过期的 `Started` 不能算 ready。
- [ ] node-daemon 在 package install、Docker pull、deploy/start 失败时上报与 deployment identity
  关联的 typed failure，让 Activate/status 能区分节点部署失败与单纯超时。
- [ ] Static Web 也必须有带版本 identity 的物化/路由就绪证据，不能只检查目录存在或旧
  ServiceInfo key 存在。
- [ ] 共享类型是 beta 2.2 breaking change，不给缺少 identity 的旧报告做“视为成功”兼容。

### P0.9 收紧 Activate 与 rollback 完成条件

- [ ] Activate 只接受与当前 task 目标 deployment identity 完全匹配的 scheduled/runtime
  evidence。
- [ ] 旧版本遗留的 `Started`、目录、端口或路由记录不能满足新版本 Activate。
- [ ] 冻结 `expected_instance_count` 的成功策略：全部 ready、minimum-ready 还是 quorum；
  按决策输出每个 desired/scheduled/runtime instance 状态，不再“任意一个 Started”即成功。
- [ ] 保留停止期望的 App 升级时，Activate 验证目标 identity 已进入 scheduled/materialized
  且保持 Stopped，不强制等待 Started。
- [ ] Static Web 完成条件至少包含 node-daemon 物化目标 content identity 和 cyfs-gateway
  已加载目标 config generation 的 ack；旧目录或旧 route 不能通过。
- [ ] installed record/proof 和 Task Completed 只能在目标 identity ready 后写入。
- [ ] 区分必须的本地 installed record 和可选的外部 RepoService proof；Repo proof 失败按
  best-effort warning/后续补写处理，不应将已就绪的本地安装误判为失败。
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
- [ ] CLI 可以做语法归一化和传入 selector hint，但 Control Panel 是 App DID、安装作用域、
  installed target 和授权判定的最终权威，submit 时必须重新解析/校验。
- [ ] 名称解析为 App DID 后，再在调用方可见的 zone/user 安装作用域中选择目标。
- [ ] 定义 opaque `AppInstallationId`，从 canonical App DID + AppClass + owner/scope 确定性
  派生或由权威服务分配；`AppDoc.name` 只是展示/友好名，不得单独作为存储、运行或
  权限身份。
- [ ] system-config spec/install record key、service spec id、scheduled config、runtime report、
  RBAC subject、gateway server/route 和 availability policy 使用同一 installation identity 映射；
  如仍保留内部 `app_instance_id`，必须由 `AppInstallationId` 导出，不再由用户输入或
  `AppDoc.name@user_id` 拼接。
- [ ] Beta 2.2 全量构建、SystemConfigBuilder 和预安装入口直接生成新 identity，不增加旧
  `app_id@user_id` 的兼容读取层。
- [ ] 0 个匹配返回 NotInstalled/NotFound；多个匹配返回 `AMBIGUOUS_APP_TARGET` 和脱敏候选
  scope，不默认选 owner、当前用户或第一个结果。
- [ ] 生命周期和 status 使用同一 resolver，不能各自实现不同选择规则。
- [ ] 同一 scope 内 `AppDoc.name` 相同、App DID 不同的两个 App 能够同时安装、运行和
  独立管理，不发生 spec/service/RBAC/gateway 冲突。

### P0.11 typed 状态 facade

- [ ] 正式注册并实现 `AppInstallStatusSnapshot` 查询，不让 SDK/Tool 解析 Task JSON。
- [ ] 正式注册并实现 `AppUpdateAvailability` 单项/批量查询。
- [ ] `app details/status` 聚合 install record、desired spec、active task、scheduled config、
  runtime report、readiness、verification 和 source summary。
- [ ] 明确区分 desired version、scheduled version、runtime version、last successful version 和
  rollback version。
- [ ] list/details/status 输出使用同一安装作用域与权限过滤。
- [ ] 对没有 Installer Task/install record 的 bootstrap App 和 SystemBuiltin App 返回明确
  `management_origin`/`managed_by` 及可用 action，不把缺记录简单当作 NotInstalled。
- [ ] status 包含与 deployment identity 绑定的 node-daemon deployment error、report freshness、
  desired/ready instance count 和 Static Web gateway generation ack。
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
- [ ] delete data 使用 Installer 生成的 typed deletion manifest，只删除该 installation identity
  明确拥有的 data/cache；external mount、Secret 和共享资源默认绝不删除。
- [ ] start/stop/restart/uninstall 共用持久 runner；删除只靠 `tokio::spawn` 的执行路径。
- [ ] Control Panel 重启后的 startup scan/sweep 能恢复所有非终态 App lifecycle Task。
- [ ] 任务取消、重复请求和 idempotency key 有确定行为。
- [ ] lifecycle runner 沿用上述 TaskMgr 2.0 直接执行 + scan/sweep 模型，不引入 KMSG 或
  另一套 lifecycle queue。

### P1.2 Catalog 批量升级

- [ ] 无参 upgrade 枚举当前调用方可管理的已安装 App，不包含不可见 user scope。
- [ ] 批量检查只使用权威 Resolve 结果，返回 typed availability 和默认升级 plan 摘要。
- [ ] 已满足项不创建 Task；实际更新项可以形成有明确 child task/result 的批量操作。
- [ ] batch root/child 使用 TaskMgr 2.0 parent/root/retry 关系和稳定 idempotency key；批量重放
  不重复创建已承诺的 child Task。
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
- [ ] TaskManager revision fencing：两个过期 runner write 竞态时只有当前 revision/epoch 能成功。
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
- [ ] 同 scope 安装两个 `AppDoc.name` 相同但 App DID 不同的 App，验证 spec、service id、
  RBAC、gateway 和 lifecycle 完全隔离。
- [ ] 同一 submit 在 RPC response 丢失后使用同 idempotency key 重放，返回同一 Task；
  相同 key + 不同输入返回 `IDEMPOTENCY_CONFLICT`。
- [ ] 同 App 并发 install/upgrade/uninstall 只有一个取得 mutation ownership；不同 App 并发时
  `app_index` 不重复。
- [ ] Failed Task retry 产生新 task_id 和 `retry_of`，旧 Task 保持 Terminal；cancel 在 Deploy
  期间重启后继续回滚并最终 ack。
- [ ] startup scan/sweep 不借助 KMSG 恢复 Pending/Running，不自动执行
  WaitingForApproval/Paused/Terminal；重复 sweep 不重复产生 Stage 副作用。
- [ ] `expected_instance_count > 1`、运行报告过期、node-daemon typed failure、已停止 App
  升级和 Static Web gateway generation 就绪条件。
- [ ] bootstrap/SystemBuiltin App 无 install record 时的 list/get/status/action 权限和
  `management_origin` 输出。
- [ ] staging handle 跨用户/跨 Zone 读取拒绝，活动 Task 引用期间 GC 不会删除对应 PIKG。
- [ ] start/stop/restart/uninstall 重启恢复和幂等。

### 9.2 DV 测试

- [ ] Catalog：`fetch -> default plan -> 修改选项 -> plan file -> install -> status`。
- [ ] 本地 PIKG：两次上传 digest 相同，首次 handle 过期后仍可用新 handle 安装。
- [ ] URL PIKG：CLI 获取字节；Control Panel 从未接收或打开客户端 URL/path。
- [ ] 同版本 install 返回 satisfied，无 Task、无 spec revision 变化。
- [ ] 低版本 install 返回 `DOWNGRADE_NOT_ALLOWED`，旧实例不中断。
- [ ] Catalog 升级和 PIKG 升级都保留原配置。
- [ ] 注入旧版本 Started report，升级不得提前成功；目标版本上报后才能完成。
- [ ] 注入 identity 正确但已过期/属于旧 node boot session 的 Started report，仍不得成功。
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

1. **协议、类型与内部身份**：P0.1-P0.2 + P0.10，先冻结 Plan/Inspection/Status/错误码、
   `AppInstallationId` 和 installation scope 映射。
2. **TaskMgr 2.0 真相源与 runner 收敛**：P0.3-P0.4A，修 Task 持久化、业务 owner、
   retry/cancel/revision fencing，确认直接执行 + scan/sweep 恢复不依赖 KMSG。
3. **计划与动作判定**：P0.5-P0.7，完成 fetch plan、幂等 submit、并发序列化、
   新装/升级矩阵和配置继承。
4. **PIKG staging**：§5，打通 plan digest 与安装时重新上传。
5. **版本级收敛**：P0.8-P0.9，联动 scheduler/node-daemon/runtime，消除旧版本假成功。
6. **查询 facade**：P0.10-P0.11，给 Tool 稳定 selector/status/update-check，并覆盖
   bootstrap/SystemBuiltin management origin。
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
- [ ] 不为 App Installer 恢复引入 KMSG queue、TaskMgr runner inbox 或另一套业务队列。
- [ ] 不用每次随机生成的 idempotency key 冒充客户端重放保护。
- [ ] 不原地复活 TaskMgr 2.0 Terminal Task；retry 必须保留旧终态并建立新 Task 关系。
- [ ] 不以 `AppDoc.name`、`AppDoc.name@user_id` 或 system-config path 作为对外/稳定 App 身份。
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
- [ ] 同名不同 App DID 使用不同 `AppInstallationId` 完整隔离，bootstrap/SystemBuiltin
  App 也有明确的管理来源和操作权限。
- [ ] 首次安装、升级、已满足、拒绝降级和 stale plan 都有稳定机器可读结果。
- [ ] 请求重放不重复创建 Task，Task retry/cancel 符合 TaskMgr 2.0 Terminal/control 语义。
- [ ] Control Panel 或执行节点重启不会让任务丢失或误报成功。
- [ ] App Installer 的执行与恢复只依赖 TaskManager 持久状态 + Control Panel 直接 runner/
  scan/sweep，不依赖 KMSG queue 或事件投递的可用性。
- [ ] 文档、共享类型、Control Panel、scheduler/node-daemon、SDK 和 DV Test 已同步。

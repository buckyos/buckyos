# BuckyOS Desktop Lite 调度器解耦与回退验收 TODO

日期：2026-10-26；2026-10-29 按源码核对与裁决重写。

状态：**2026-10-30 已实施（未提交），Linux 本机 DV 通过（已有 Lite 重启、全新 Lite、完整版回归）；Windows / macOS 未实测。** T1 的普通节点加入守卫与 T2 的 control-panel 节点设置守卫都没有可挂的产品入口，未实现，改由 node_daemon 兜底补齐（见 §7.3）。实施记录与证据见 §7。

## 1. 核心要求

用户明确要求：**调度器没有必须的理由，不应该随手修改任何代码。完成改造后，应能回退 Lite 相关的调度器代码，当前 Lite 仍正常运行。**

Lite 应通过安装配置、节点范围、应用创建入口和节点执行能力形成产品边界。调度器继续按应用需求与节点能力工作，不负责识别产品版本、执行产品配额或同步产品配置。

- Lite 本机设备保持 `support_container=false`；UI 阻止增加普通 BuckyOS 节点，节点加入接口保持同一约束。
- 允许增加 BuckyOS Agent Daemon。它作为 Executor 提供 Agent 的执行环境，不成为应用调度节点，也不向调度池提供容器能力。
- 调度器拿不到支持容器的可选节点，原有 `need_container && !node.support_container` 过滤自然阻止容器 App/Agent 的部署。
- 单 Agent、用户应用类型和预置应用许可等产品策略放在 control-panel 等产品入口。原生进程的加载与生命周期由 node-daemon/AppLoader 负责。
- 保持现有 Lite 能力：基础服务、Desktop、预置脚本应用、一个原生 Agent、static web 安装/更新/卸载，以及 Agent Daemon 接入；不能靠删掉当前功能实现调度器回退。

本文替代 [Lite 实施 TODO](buckyos-desktop-lite-todo.md) 中要求调度器重复校验产品策略、同步产品档位及按 Lite 强制运行方式的设计；其它产品行为继续沿用。通用原生 Agent 能力见 [原生 Agent Service TODO](opendan-native-agent-service-todo.md)。

## 2. 已定裁决（2026-10-29）

核对结论：Lite 提交在调度器里新增的逻辑，除预置脚本的放置外都已有调度器之外的等价入口，可以直接回退。预置脚本在旧基线下放不上去不是 Lite 加出来的耦合，而是旧调度器自带的 author 白名单（一种写死的产品策略）。对此裁决如下。

- **D1 `need_container` 走 auto 语义。** 带 script 包的非 Agent 应用两种方式都能跑，所以不要求容器节点；落点后按该节点的 `support_container` 决定 `run_mode`：有容器给 Container，没有给 Native。不新增枚举值：`ServiceSpec.need_container` 只参与节点过滤，auto 对过滤器就是 false，差别只体现在 `app_run_mode` 的输出。
- **D2 旧 author 白名单随之删除。** `did:web:buckyos.{ai,io,org}` 才免容器的规则被 D1 取代。副作用：Full 版一台没有容器的节点以后会接收任何用户安装的脚本应用并 native 运行，没有 worker 容器隔离。用户裁决接受：目前只有调度器支持分布式，BuckyOS 整体尚不支持多 node，多 node 的问题到时一并看。
- **D3 调度器之外不再有第二个 run_mode 决策。** node_daemon 照 `run_mode` 执行（`Native` + script 包 → 现有 NativeScript 分支），不按产品自行把 Container 改成 Native。node_daemon 里"Lite 永不探测/加载/运行容器"的本机守卫保留，它是兜底不是决策。
- **D4 `system/product` 由 control-panel 启动时发布。** 剩余消费者只有 control-panel 自己（product.rs、agent_runtimes.rs）和经 `system.product.get` 的 Desktop；node_daemon 与 node_control 已直接读本机 `etc/product.json`。control-panel 已有写 `system/agent_slot` 的权限，在服务 RPC 之前同步一次，不会短暂按 Full 放行。
- **D5 `services/<spec>/schedule_status` 与 `unschedulable` 删除。** 全仓除调度器外没有读者，Desktop 也不读。
- **D6 scheduler.rs 的失败隔离/未放置重试默认一并回退。** 入口守住后 Lite 正常路径不会出现放不下的 spec；"一个坏 spec 拖垮整轮"是旧基线固有缺陷，见 §5 单列，不作为本次前提。
- **D7 Agent 不走 auto。** 仍由基线已有的 `native_agent_node` 决定（owner 的 host 默认 runtime + 节点 executor/owner_agents 开放），因为它取决于 owner 设置而不是节点能力。
- **D8 回退验收基线改写为"`ee65f12f8` 调度器 + D1/D2 两处通用改动"。** 其它调度器差异应为零（bucket_store 等后续无关提交除外）。

## 3. 已核对的基线与依赖

本次源码核对 HEAD：`88ea06f1643eeac6518705a9585c110671c89989`。实施时重新检查工作树与提交历史，不覆盖其它在途改动。

- Lite 引入提交：`1df46176aa8eb966aa9badbdc6c7a2a22220b691`（`support buckyos-desktop-lite`）。
- Lite 前调度器基线：其父提交 `ee65f12f8d8d027df3c0abf55529ff4e37d4a32c`。
- 更早的 `5af93e6e6` 已实现通用原生 Agent 调度，以上基线包含它。回退 Lite 改动不等于撤销原生 Agent 支持。
- Lite 之后触及调度器的无关提交：`4bb84f2d1`（bucket_store）、`2985ff934`（app/service 迁移 bucket store）、`127a1eaad`、`9589c91f6`（obj:// URL 与权限）、`de81183b5`。撤销 Lite 改动时按反向 patch 做，不能用旧文件整体覆盖。

Lite 提交触及的调度器文件与处理方向（已按 diff 逐项核对）：

| 文件 | Lite 引入的职责 | 处理 | 调度器之外的等价入口 |
| --- | --- | --- | --- |
| [main.rs](../src/kernel/scheduler/src/main.rs) | boot 初始列表写入及每次启动 `sync_product_profile` | 全部回退 | D4：control-panel 启动同步 |
| [install_plan_executor.rs](../src/kernel/scheduler/src/install_plan_executor.rs) | 执行安装计划前 `load_product` + `admit_app` | 全部回退 | control-panel 已在提交（`app_installer.rs` submit）、Acquire、Inspect（`app_install_driver.rs`）三处准入，不支持的任务停在 Inspect |
| [system_config_agent.rs](../src/kernel/scheduler/src/system_config_agent.rs) | `app_spec_admission`（产品准入、槽位复核、host runtime 复核）、按产品清 `need_container`、`best_instance_count.min(1)`、`schedule_status_actions`、`lite_tests` | 全部回退；`need_container` 推导按 D1 改写 | 槽位：`agent_mgr.rs` 建 Agent 同事务取槽；副本数：Agent spec 固定 `expected_instance_count: 1`；host runtime：`agent_mgr.rs` 创建与改设置两处拒绝无容器时默认 container |
| [app.rs](../src/kernel/scheduler/src/app.rs) | `app_run_mode` 按产品强制 Native；`build_app_service_config` Native 分支区分 agent/script 包 | 产品分支回退；`app_run_mode` 按 D1 改为按节点能力；Native 分支的 script 包校验保留（属 D1） | — |
| [scheduler.rs](../src/kernel/scheduler/src/scheduler.rs) | `unschedulable`、单 spec 失败隔离、`has_unplaced_specs` 重试、错误说明 | 全部回退（D5/D6） | §5 单列缺陷 |

## 4. 实施任务

### T0：最小改动与回退清单

- [x] 以 `ee65f12f8..1df46176a` 的调度器 diff 为底，生成反向 patch；逐 hunk 标注"回退 / D1 保留并改写"。除 D1/D2 两处外不得有其它调度器净差异。
- [x] D1 实现落点：`create_service_spec_by_app_config` 非 Agent 分支改为 `need_container = app_doc.pkg_list.script.is_none()`，删除 author 白名单；`app_run_mode` 增加 node_info 参数（调用方 `instance_app_service` 手里有 device_list），非 Agent 且有 script 包时按 `node_info.support_container` 给 Container/Native；`build_app_service_config` Native 分支保留"Agent 需 agent 包、脚本应用需 script 包"。
- [x] 为 D1 写两条通用单测（不含产品档案）：无容器节点上 script 应用得到 `run_mode=Native`；有容器节点上得到 Container。删除 `lite_tests` 模块。
- [x] 清点共享类型：`SpecScheduleStatus`/`spec_schedule_status_key`（buckyos-api app_mgr.rs）随 D5 删除；`ProductProfile`、`AgentSlot` 等保留给 control-panel/node_daemon，但 scheduler crate 不再引用。

### T1：产品入口固定节点范围与配置

- [x] control-panel 启动时（`main.rs` 构造 `ControlPanelServer` 之后、注册 RPC 之前）把 `etc/product.json` 同步到 `system/product`：读本机档案，与现值不同则写入；档案不可读则启动失败，不按 Full 继续。覆盖首次启动、待激活安装、已有 Lite 重启和产品更新。
- [x] 激活、开发配置生成、安装与重启后的设备文档/设备上报都保持 `support_container=false`（现有 `validate_topology_for_product`、`make_config.ts --edition lite`、`active_lib.ts` 已覆盖，复核即可），宿主安装了 Docker/wslc 也不改变能力。
- [ ] Lite 的节点 UI 阻止增加普通节点，对应加入接口给出明确结果；不靠调度器隐藏节点。本次核对未在 control-panel 找到独立的节点加入 RPC，实施时先定位实际入口（buckycli / node_active 设备注册路径）再加守卫。**（2026-10-30 未实现，待裁决：见 §7.3。）**
- [x] Agent Daemon 继续沿现有 Executor 注册、连接和授权路径接入，不写成可放置 AppService 的普通节点。

### T2：产品准入与单 Agent 限制留在控制面

- [x] 保留 control-panel 对新增服务应用、预置应用身份/版本及 static web 的现有产品策略（UI、RPC/CLI、更新、重试、恢复任务），复核回退调度器准入后没有任何入口能把容器 App spec 写进 `users/<owner>/apps`。
- [x] 唯一 Agent 槽位沿用现有语义：创建时原子取得，停止、停用或崩溃不释放；删除且确认运行时卸载完成后才释放。调度器不再查槽位。
- [ ] **新增守卫**：Lite 下若唯一节点的 `executor` 被关闭或 `owner_agents` 被收回，`native_agent_node` 返回 None，Agent spec 会变成 `need_container=true`，在旧基线下触发整轮中断（§5）。control-panel 的节点设置入口在产品无容器且该节点承载持槽 Agent 时拒绝此变更，给出明确错误。**（2026-10-30：没有这个入口，改为 node_daemon 兜底，见 §7.3。）**
- [x] host 配置变更、Agent 删除和任务恢复不会留下反复要求容器运行的 spec。
- [x] 安装失败反馈：无容器节点保证无法部署，不自动等于 inspect 立即失败；现有准入已在 Inspect 阶段给出 `UnsupportedTarget` + 原因，复核即可，不改核心调度算法。

### T3：预置脚本与原生 Agent 在改写后的调度器下运行

- [x] 用"旧基线 + D1"实际生成 systest 的 spec 和实例配置：`need_container=false`、落在 Lite 节点、`run_mode=Native`，实例配置含 script 包。
- [x] node_daemon 侧不新增判定：`resolve_runtime` 现有 `run_mode==Native && script 包 → NativeScript` 即可；确认 AppLoader 的端口、服务发现、日志、状态上报契约不变。
- [x] 原生 Agent 复用基线 `native_agent_node`；验证重启、删除和 host 条件变化不会触发 Docker/wslc。
- [x] node_daemon 保留无容器执行边界、原生进程管理与产品预置许可；调度器解耦不能扩大任意宿主程序的运行权限。

### T4：清理依赖与联动文档

- [x] 撤销 Lite 引入的调度器产品配置读取/同步、产品准入、槽位与产品副本限制、按产品强制 Native、`schedule_status` 等依赖，保留无关业务改动。
- [x] 不通过把产品判断搬进 scheduler 调用的公共 helper 来保留相同耦合。
- [x] 更新旧 Lite TODO 的现状与相关手册：`buckyos_admin`、`buckyos_app_dev`、`opendan_agent_dev`；说明配置归属（`etc/product.json` → control-panel 发布）、安装失败反馈和原生运行判断（脚本应用 auto：节点有容器用容器，否则 native）。只更新实际受影响内容。

## 5. 单列缺陷与遗留（不作为本次前提）

- 旧基线 `schedule_spec_change` 对单个放不下的 spec 用 `?` 抛错，`build_schedule_plan` 整轮失败，所有 Step2 放置同轮作废且每轮重复。D6 决定本次回退 Lite 提交里的隔离修复；是否单独修由后续评估，证据：Lite 提交 `scheduler.rs` 的 `a_spec_no_node_can_take_does_not_stop_the_others` 用例。
- 已放置的脚本应用实例不会因为节点后来有了容器而自动切换 run_mode，与 Agent 现状一致。
- D2 的 Full 侧副作用（无容器节点 native 运行第三方脚本）在多 node 支持时重新评估。

## 6. 回退验收是完成条件

以下验证必须实际执行并留证，不能只检查源码没有 `lite` 字符串，也不能只证明当前调度器能运行。

- [x] 在隔离 checkout/worktree 中组合"改造后的其它模块 + 撤销 Lite 调度器改动后的 scheduler"，保留后续无关业务变更；记录精确提交与反向 patch。
- [x] 再以 `ee65f12f8` 的调度器源码加 D1/D2 两处改动作为明确基线验证。D1/D2 的 diff 单独列出；bucket_store 等后续无关协议确需保留的差异逐项列明并单独验证；不得夹带其它 Lite 分支或 Lite 提交新增的通用修复。
- [x] 构建并安装真正回退后的 scheduler 可执行文件，记录源码版本、产物哈希、安装路径和实际运行进程，排除旧二进制未替换或后台仍运行新版的假通过。
- [x] 同时覆盖全新 Lite 安装/激活和已有 Lite 重启。不得靠上次新版 scheduler 留下的 `system/product`、Native 实例配置或已启动进程通过验收。

| 场景 | 通过条件 |
| --- | --- |
| 基础系统 | Desktop、网关、system-config、控制面等健康；产品信息保持 Lite（由 control-panel 发布） |
| 节点范围 | 普通节点不能加入；Agent Daemon 可加入并实际执行任务；调度器中没有支持容器的可选节点 |
| 预置脚本 | systest spec `need_container=false`，实例 `run_mode=Native`，由正式 AppLoader 原生托管，端口和网关访问正常；停止、崩溃恢复、重启可用 |
| 原生 Agent | 创建、对话、host 工具调用、产物交付及重启恢复正常；第二个 Agent 被产品入口拒绝，删除后可重新创建；关闭唯一节点 executor 被入口拒绝 |
| Static web | 安装、访问、更新、卸载完成，状态与实际路由一致 |
| 容器需求 | 容器 App/Agent 无法部署；正常入口在 Inspect 阶段反馈明确，不留下影响基础服务的无效调度任务 |
| 无容器依赖 | 无容器环境可运行；宿主已装 Docker/wslc 时仍无容器预载、启动或清理调用 |
| 完整版回归 | 原有容器应用、多 Agent 与节点扩展能力不受影响；有容器节点上的脚本应用仍得到 `run_mode=Container` |

- [ ] 按改动范围运行相关 Rust、配置生成、控制面/UI 检查，并用实际回退产物做 Lite DV；记录 Linux、Windows、macOS 的验证结果，未实测项明确保留。**（Rust 与 Linux DV 已做；Windows / macOS 未实测；配置生成、Desktop 未改动未跑。）**
- [x] 最终交付列出：非调度器改动、撤销的 Lite 调度器改动、保留的 D1/D2 diff、保留的独立差异及理由、回退构建与运行证据、未完成项。

完成判定：**Lite 正常运行不再依赖这次 Lite 改造新增的调度器逻辑；调度器相对 `ee65f12f8` 只保留 D1/D2 两处通用改动，实际回退后当前 Lite 功能仍成立。**

## 7. 实施记录（2026-10-30，未提交）

源码基线 HEAD `88ea06f16`。实施期间另有会话在同一工作树改 `node_daemon/src/{boot.rs,node_daemon.rs}` 与两份跨 Zone 通信 notepads（10-30 00:41 起），不属于本次改动；本次 DV 用的 node_daemon 产物在其改动之前构建（00:56，见 §7.4）。

### 7.1 调度器（撤销 + D1/D2）

做法：`git diff ee65f12f8 1df46176a -- src/kernel/scheduler` 反向 `git apply -R --3way`；只有 `system_config_agent.rs` 顶部 import 一处冲突（后续 `9589c91f6` 把 `rbac_literal` 移进 buckyos-api），手工合成 `rbac_literal, AgentPermissionMode, …`。核对：撤销后 `ee65f12f8 → 工作树` 的调度器差异与 `1df46176a → HEAD`（后续无关提交 `4bb84f2d1`、`2985ff934`、`127a1eaad`、`9589c91f6`、`de81183b5`）逐行一致，仅该 import 的换行不同。

撤销的 Lite 调度器逻辑（全部）：`main.rs` boot 写 `system/product` 与 `sync_product_profile`；`install_plan_executor.rs` `load_product` + 执行前 `admit_app`；`system_config_agent.rs` `app_spec_admission`、按产品清 `need_container`、Agent 副本 `min(1)`、`schedule_status_actions`、`lite_tests`；`app.rs` 按产品强制 Native；`scheduler.rs` `unschedulable`、单 spec 失败隔离、`has_unplaced_specs`、细化的无节点错误。scheduler crate 已不引用 `ProductProfile` / `AgentSlot` / `PRODUCT_PROFILE_KEY`（产物 `strings` 无 `product profile`、`system/product`、`schedule_status`；旧产物有）。

保留的 D1/D2（相对"撤销后"的全部调度器净差异）：
- `system_config_agent.rs` `create_service_spec_by_app_config`：`need_container = if is_agent { native_node.is_none() } else { pkg_list.script.is_none() }`，删除 author 白名单。
- `app.rs`：`app_run_mode(node_id, node_info, app_config, input)` —— Agent 走新拆出的 `agent_run_mode`（即基线 `native_agent_node` 判断，D7）；非 Agent 有 script 包且 `!node_info.support_container` → Native，否则 Container；`instance_app_service` 传入 `node_info`。`build_app_service_config` Native 分支保留"Agent 需 agent 包、脚本应用需 script 包"。
- `system_config_agent.rs` `reconcile_app_instance_specs`：Agent 的 run_mode 仍按 `agent_run_mode` 改写；非 Agent 保留放置时的 run_mode（§5 第二条"已放置实例不随节点能力切换"，同时避免它把 native 实例翻回 container）。
- 单测 `app_lifecycle_tests::a_script_app_runs_in_a_container_where_its_node_has_one_else_natively`（有/无容器两种节点：`need_container=false`、run_mode Container/Native、reconcile 稳定）。按 TODO 写的"两条"合成一条两轮循环。

### 7.2 调度器之外

- control-panel `product.rs` `publish_installed_product()`，`main.rs` 在 `ControlPanelServer::new()` 之后、安装恢复/RPC 注册之前调用：读 `etc/product.json`（`ProductProfile::load_installed`，缺文件 = 完整版），与 `system/product` 不同则写入，读不了或写不进则 control-panel 启动失败。
- buckyos-api：删除 `SpecScheduleStatus` / `spec_schedule_status_key`；`product.rs` 模块说明与 `AdmissionOrigin` 注释改为 control-panel 发布、调度器不识别产品。
- node_daemon `app_loader.rs` `observe_deployment`：run_mode 落到本节点跑不了的运行时（NotSupport）时，若本实例还有原生进程在跑就停掉它（warn `no longer runs natively here`），再报 NotExist + `not_supported`。单测 `a_native_process_does_not_outlive_a_switch_to_an_unsupported_runtime`（去掉修复即失败）。见 §7.3。
- 手册：`buckyos_admin.md` Lite 段（配置归属、Inspect 拒绝、脚本应用 auto、关闭 executor 的后果，去掉 `schedule_status`）；`buckyos_app_dev.md` 原生端口一句。`opendan_agent_dev.md` 无受影响内容，未改。旧 Lite TODO 顶部与 §6.1 表 T0/T1/T3/T4 行已标注撤销。

### 7.3 两个守卫没有入口（待裁决）与兜底

- **普通节点加入（T1）**：control-panel / Desktop 没有任何"增加节点"入口或 RPC；普通节点的加入路径是离线 `buckycli create_node_configs`（需 owner 私钥）生成节点身份，新节点自己的 node_daemon 登录后写 `devices/<name>/{doc,info}`。守卫只能放在 system-config / verify-hub 的设备登记处，等于把产品判断放进内核服务，本次未做，需用户定。现状：Lite UI 无加节点入口；若有人用 buckycli 把一台完整版节点接进 Lite Zone，它会以 `support_container=true` 进调度池（control-panel 仍在 Inspect 拒绝容器 App，Agent 创建仍拒绝 container 默认）。
- **关闭唯一节点 executor（T2 新增守卫）**：control-panel 不写 `nodes/<node>/settings`（`doc/arch/node_exector.md`：只有管理员直接写 system-config 与 Desktop 首启种子写），没有可挂的入口。DV 实测（§7.5）撤销调度器后的真实后果：已放置的 Agent 不会让整轮中断（现有实例已满足副本数，`schedule_spec_change` 不再放置），但 reconcile 把实例改成 container；node_daemon 的 Lite 守卫拒绝 container，而**旧原生 opendan 进程一直在跑**、每 5 s 报一次 NotSupport —— 这正是旧 Lite 调度器靠 `agent_host_runtime_required` 停掉 spec 掩盖的情况。修复放在 node_daemon（§7.2 第三条，与产品无关：实例换到本节点跑不了的运行时，就不再留着它原来的原生进程）。修复后：Agent 停止、实例 `exited` + `deploy_failed`（原因写明需 host runtime），调度器无 `No suitable node`；恢复 executor 后 run_mode 回 native、新进程拉起、对话正常。遗留：关闭期间 node_daemon 每 5 s 记一条 ERROR（与任何部署失败的 App 相同），spec 保持 `need_container=true` 直到恢复。若仍要入口级拒绝，需要先有一个写节点设置的产品入口。

### 7.4 构建与运行证据

| 产物 | 旧（10-29 22:35 rootfs，HEAD `88ea06f16` 源码，含 Lite 调度器） | 新（`0.7.0+build261030.main.88ea06f1643e.dirty`，`buckyos-build.py --skip-web -s …`） |
| --- | --- | --- |
| scheduler | `be9b1114…`（`strings` 含 `product profile` ×3、`schedule_status` ×2） | `1b8cb809c13e65e0…`（均为 0） |
| control_panel | `3cff8fbb…` | `08cb40d65abb6318…`（含 `publish product profile`） |
| node_daemon | `04f491de…` | `7ccbf30161e0b22e…`（00:32，§7.5 A–C 前半）→ `4ccee4086272b3fe…`（00:56，含 §7.3 修复，C 后半与 D） |

安装路径 `/opt/buckyos/bin/{scheduler/scheduler,control-panel/control_panel,node-daemon/node_daemon}`；每阶段用 `sha256sum /proc/<pid>/exe` 核对实际进程（三者均与新产物一致，启动时间晚于安装）。

### 7.5 Linux DV（本机 test.buckyos.io，root，x86_64，宿主装有 Docker）

A. 用旧产物 `start.py --all --edition lite` 造"已有 Lite"：`system/product` 由旧调度器写入；systest `native`、端口 10016；建 jarvis → native、占槽。

B. 已有 Lite 重启：先删 `system/product`，再 `start.py`（更新 + 重启，保留数据）换成新产物。control-panel 日志 `product profile: lite edition (system/product updated)`，`system/product` 恢复为 lite；systest、jarvis 都 `started`、仍 `native`（reconcile 不改写）；新调度器日志无放置失败；jarvis 对话：宿主 `id -u` = 65534、`uname -sr`，`lite_restart_ok.txt` 写在 host-runtime Session 目录并交付。

C. 全新 Lite（新产物 `start.py --all --edition lite`）：
- 产品：control-panel 发布 lite（调度器产物里已无写入代码）；`devices/ood1/info.support_container=false`（宿主有 Docker）。
- 预置脚本：调度日志 systest spec `need_container: false`（author `did:bns:buckyos`，旧白名单下会是 true），实例 `run_mode=native`，节点 Deno 监听 `0.0.0.0:10016`，网关 200；`kill -9` → `exited` → 约 15 s 新 pid、healthy；`apps.stop` → stopped、端口释放；`apps.start` → started，run_mode 仍 native。
- 原生 Agent：创建 jarvis → spec `need_container: false`、native、占槽；对话 + 宿主 `whoami`=nobody、文件交付；第二个 `agent.create` → `agent_slot_occupied`；`agent.delete` → 原生进程停、16 s 后槽位释放、此后无报错；再建 jarvis2 → native、占槽。关闭 / 恢复 executor 见 §7.3（修复前后各测一次）；恢复后 jarvis2 对话正常。
- Static web：lite-web 0.1.0 安装 → v1；0.1.1 升级 → v2；`app uninstall --data delete` → 约 36 s 后 403，`APP_NOT_INSTALLED`。
- 容器需求：docker App 与用户脚本 App `app fetch` 均 `target NOT_READY / install UNSUPPORTED_TARGET`（`product_container_app` / `product_service_app`），install 在 Inspect 被拒，`users/devtest/apps` 无新 spec。
- Agent Daemon：`executor.device.create` + 本机 `buckyos_agent_daemon run`，`executor.device.list` 显示 online 与能力；`devices` 仍只有 `ood1`，调度器节点只有 `ood1`。未经 Agent 实际下发命令（完整链路见 `test/agent_daemon/` 既有验收）。
- 无容器依赖：A–C 全程（00:29–01:03）`docker events` 0 条；node_daemon `product edition: lite (containers disabled)`、`paios images: this product runs no containers`。

D. 完整版回归（`start.py --all`，本机 DV 已恢复为完整版）：control-panel 发布 full；systest `need_container: false`、`run_mode` container（容器 `buckyos-app-buckyos-systest`）、网关 200；用户脚本 App inspect `OFFLINE_READY`、`need_container: false`、跑在 worker 容器；docker App `need_container: true`、容器运行；jarvis、friday 两个容器 Agent 同时 ready、无槽位。测试 App 与 friday 已删除，jarvis 保留。多节点扩展未测（单机）。

DV 旁注（与本次无关）：`start.py --all` 保留 `data/home`，同名 Agent 的 host-runtime Session 目录沿用（新 Zone 的 jarvis 续用了上一轮的 `ui-20261030-2`），`test_agent_chat_task.ts` 的 `agent_task` 断言因此失败，回复本身正常。

### 7.6 单元测试（`src/`，`-- --test-threads=1`）

- `cargo test -p scheduler`：78 + 21 全过。并行跑时 `agent_rbac_tests::agent_permissions_preserve_owner_limits_and_read_only_runtime_state`、`user_type_rbac_tests::user_types_and_system_groups_become_rbac_groups` 会失败，HEAD 并行跑同样失败（全局 RBAC 状态），另 HEAD 的 `scheduler_test::test_create_pod_instance_no_suitable_node` 因 Lite 改了错误文本而失败，撤销后恢复。
- `cargo test -p buckyos-api`：全过（331 + 其余）。
- `cargo test -p control_panel`：208 过，`sso_identity::account_identity_carries_ceiling_method_and_password_state` 为既有失败（旧 Lite TODO §6.3 已记）。
- `cargo test -p node_daemon`：157 过（含新用例）。
- 未跑：配置生成（`make_config_test.ts`）、publish、websdk、Desktop —— 本次未改这些模块。

### 7.7 未完成

- Windows（VM 113）、macOS（VM 114）未实测。
- §7.3 两个入口级守卫待裁决。
- D2 的实际效果比 §2 写的更严：完整版无容器节点会拿到 `run_mode=native` 的第三方脚本 App，但 node_daemon 只原生运行产品清单里的预置脚本（`native_script_manifest`），其它报 not admitted，不会无隔离运行；多节点时这类 App 可能被放到无容器节点而起不来，随 §5 多节点问题一并看。

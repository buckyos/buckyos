# OpenDAN 在 TaskMgr 2.0 重构后的补完 TODO —— 完成记录

> 基线：`b0f3796f`（beta2.2）。2026-08-10 实施完成，本文档从 TODO 转为完成记录 + 遗留清单。
> 范围只覆盖 **OpenDAN 侧**作为 Dispatch Runner 的缺口，以及它与 Task Dispatch Center 的接缝。
> `notepads/task-dispatch-center-todo.md` 仍是 2.0 之前的过期文档（claim_next 拉取模型口径），未重写。

## 实施摘要（全部已落地并通过测试）

| 项 | 结论 |
| --- | --- |
| P0-1 runner endpoint 容器不可达 | ✅ 挂到调度器按 AppIndex 分配的 instance 端口（env 注入），v1 明确同机模型 |
| P0-2 activate ACK 不证明启动 | ✅ 同步启动成功后才 ACK；失败保持 delivery 可重放 |
| P0-3 journal 假 fsync | ✅ 真 fsync（含父目录）；损坏 fail-loud 拒绝接单 |
| P1-1 sweep 全 zone 轮询 | ✅ 服务端 `runner_app_id` 过滤；默认间隔 5s→60s |
| P1-2 journal 只增不删 | ✅ activated 24h 保留窗口；stale reservation 15min 出容量 |
| P1-3 kernel token 外泄面 | ✅ per-lease delivery_token 替代；endpoint loopback 校验 |
| P1-4 二次鉴权证据链 | ✅ v1 结论落地（见下） |
| P2-1 KEvent 订阅 | ✅ bound-task 按需订阅（task + tree 双通道），sweep 保留兜底 |
| P2-2 dispatch_adapter 单测 | ✅ 11 个新单测（offer 幂等/capacity/exactly-once/崩溃恢复/token） |

## 各项实施细节

### P0-1 runner endpoint（`dispatch_adapter.rs`）

- 不再自开随机 loopback 端口。runner kapi 挂在 `0.0.0.0:$OPENDAN_SERVICE_PORT`
  （`Runner::new` 本身绑 UNSPECIFIED——原 TODO 说「绑址 127.0.0.1」有误，
  旧代码只是用 loopback listener 拿随机端口号，真正致命的是随机端口没有对应
  `-p` 发布规则）。
- **端口的真实来源是调度器的 AppIndex 分配**，不是 AppDoc 声明的 4060：
  `alloc_replica_instance_port`（`scheduler.rs:1066`）对 `"www"` + `app_index>0`
  返回 `app_index*16 + BASE_APP_PORT(10000)`——jarvis（index 1）实际是 **10016**；
  AppDoc 的 `inner 4060` 只是声明，被分配器覆盖。分配值经 `ReplicaInstance.service_ports`
  → node_config → node_daemon `select_agent_service_port()` → `-p {port}:{port}` +
  env `OPENDAN_SERVICE_PORT={port}` 注入容器。runner 信任 env 即自动跟随；
  **多 agent 双容器由 index 错开（10016/10032/…），天然不冲突**。
  代码里的 4060 fallback 只覆盖原生 frame-service 过渡形态（无 env、同 netns、
  自洽即通）。
- 上报 endpoint 固定为 `http://127.0.0.1:{port}/kapi/opendan-task-runner`——
  **宿主机（dispatcher）视角**：原生同机直达 loopback；容器形态经 `-p {port}:{port}`
  的 DNAT/docker-proxy 到容器。两种形态同一 URL。
- **跨节点结论：v1 限定同机**。dispatcher 侧 `validate_runner_endpoint` 只放行
  loopback，把这个限定变成硬校验（fail-fast 在 attach 时）。多节点 zone 必须走
  cyfs-gateway 路由 + 服务发现解析 endpoint（不再信任实例自报），属后续项。

### P0-2 activate 语义（`dispatch_adapter.rs::handle_activate_task`）

- ACK 前**同步**执行 `process_accepted_dispatch_task`（幂等驱动器，无 LLM 参与，
  亚秒级）；成功才把 journal 条目标为 `activated` 并 ACK。
- 启动失败 → 返回 RPC error 且 journal 不动：dispatcher 保持 `Activating`，
  backoff 后重放同一 delivery_id，重放会再次尝试启动。
- 并发重放由 in-flight 集合挡住（返回「in progress」错误让 dispatcher 稍后重试）；
  执行期间不持 accept 锁，offer 通道不被阻塞。
- dispatcher 侧原有的「activate RPC 失败 → 查 task phase 兜底」逻辑与此模型
  正好互补：执行成功但 ACK 丢失时，phase 已是 Running，dispatcher 判定已激活。
- `activated` 的旧语义（spawn 前标记）已废弃；owner sweep 保留为最后兜底。

### P0-3 + P1-2 journal（`DispatchBindingStore`）

- `put()`：write tmp → `sync_all` → rename → 父目录 fsync（unix）。
- `open()`：NotFound 才允许空启动；读错/解析错误返回 Err，
  `spawn_dispatch_target` 收到后**不注册 runner**（fail-loud，拒绝接单，
  错误信息提示操作者把文件移开以显式重置）。
- 清理：`activated` 条目保留 24h；未 activate 的 reservation 15min 后视为
  abandoned——立即不计入 capacity（读时过滤），下次 put 时物理删除。
  15min 上界覆盖 dispatcher 最大 backoff（300s）数倍。

### P1-1 sweep（`agent_task_executor.rs` + `task_store.rs` + `ListTasksReq`）

- `ListTasksReq` 新增 `runner_app_id` / `runner_target_id` 服务端过滤
  （task 表已有对应列 + `idx_task_runner_phase` 索引）；ACL 是逐 task 的
  post-filter，新字段只收窄结果集不绕权限。
- opendan sweep 带 `runner_app_id=own_app_id`，本地 `task_targets_agent`
  仍保留（多 agent 共享 app id 时的 per-agent 归属判定）。
- `poll_interval_ms` 默认 5_000 → 60_000（P0-2 + P2-1 落地后 sweep 只是兜底）。

### P1-3 delivery token + endpoint 校验（dispatcher `service.rs` + 协议）

- `AttachInstanceResult` / `RenewInstanceResult` 新增 `delivery_token`：
  `sha256(进程内随机 secret, target_id, instance_id, lease_epoch)` 派生，
  **不入库**。dispatcher 每次 push 重算出示（`RunnerCaller::call` 新增
  `auth_token` 参数），**kernel session token 彻底退出 runner 调用路径**。
- runner 侧校验：offer/activate 必须携带当前 lease 的 token，未 attach 前
  fail-closed。绑 0.0.0.0 之后这同时是 LAN 伪造 offer 的防线。
- dispatcher 重启 → secret 轮换 → 旧 token 失效窗口 ≤ 1/3 lease（下次 renew
  返回新 token 自愈）；窗口内 push 失败走 backoff 重试，链路自恢复。
- endpoint 校验：attach 时 + 每次 call 前双重 `validate_runner_endpoint`
  （http/https + loopback only）。

### P1-4 二次业务鉴权证据链——v1 结论

Envelope 本身不再追加凭据。v1 的信任链是：

1. **通道认证**：offer 能通过 delivery_token 校验 ⇒ 调用方持有 dispatcher
   为本 lease 派生的 secret ⇒ 就是 zone 内核 dispatcher 本体。
2. **快照可信**：dispatcher 写入 envelope 的身份字段全部来自它**验签过的**
   caller session token（`register_target`/`dispatch_task` 侧 fail-closed），
   runner 信 dispatcher ⇒ 信快照。
3. Target 二次校验保留业务维度（schema、target、on_behalf_of 非空、input 合法）。

过期/撤销语义由 lease epoch 承载（re-attach 即轮换）。可验证的端到端凭据
（caller token 转签 attestation）留给多节点/跨 zone 阶段，与服务发现解析
endpoint 同期做。

`auth_policy=ZoneUsers` + `approval_policy=Never` 的评估结论：**维持**。
agent.delegate/v1 是 agent 的正常入口，加审批门等于给每个普通请求加人工闸，
与产品预期相悖；理由已写入 `run_dispatch_target` 的注册注释。

### P2-1 bound-task KEvent（新文件 `task_event_pump.rs`）

- 单 reader 聚合订阅（模式同 `SessionEventPump`）；每个绑定 task 两条 pattern：
  `/task_mgr/{task_id}`（自身变更）+ `/task_mgr/tree/{root_id}`（子任务事件，
  human.input 应答走这条）。
- 纪律：**事件只是加速**。命中后从 TaskMgr 重读 task，走与 sweep 完全相同的
  幂等 `process_agent_delegate_task` 路径；pull_event 带 1s timeout；
  60s owner sweep 保留为丢事件兜底。
- 生命周期自收敛：终态变更自身会发最后一个事件 → 重读发现 terminal → unwatch。
  绝不做全局 `/task_mgr/**` 发现（beta2.2 删除是设计意图）。

### 落地后修复：runner 面鉴权从 zone-trusted 改为 owner 绑定（2026-08-10）

debug_jarvis 实跑暴露：register/attach 启动后立即成功，20s 后（1/3 lease）首次
renew 起全部 `No permission: requires a zone-trusted service identity`。

根因是 dispatcher M1 把 runner 面（register/attach/renew/detach）gate 在
zone-trusted 上，与 runtime 的 token 生命周期冲突：AppService 启动时持
node_daemon 注入的 device 签发 token（iss=设备名 → zone-trusted），但
`renew_token_from_verify_hub`（runtime.rs）只要 `iss != "verify-hub"` 就在首个
keep_alive tick（5s）强制换成 verify-hub 签发的正式 token → **任何 app service
的稳态身份永远不是 zone-trusted**，register/attach 能过纯属启动窗口时序运气。

修复（`dispatcher/service.rs`）：
- runner 面改为「已认证 app 身份 + first-writer-wins owner 绑定」：
  register 首次注册者的 verified `app_id` 盖为 owner；后续 register/attach/
  renew/detach 必须同 owner_app_id（`require_target_owner`）。
- zone-trusted 保留管理豁免（可代管注册但**不重盖 owner**，防管理编辑劫走 target）；
  管理面（disable_target/get_target/list_targets/operation_route*）维持 zone-trusted。
- owner 匹配刻意只用 app_id 不用 user_id：boot 窗口 token 的 user 是设备名、
  稳态是 owner 用户名，用 user 匹配会造成重启后永久死锁。
- 已知 v1 限制：zone 内已安装的恶意 app 可抢注**未注册**的 target_id（真 owner
  register 时会收到 loud 的权限错误暴露此事）；多用户 zone 下同 app_id 跨用户
  未隔离——后续引入 per-user app instance 身份再收紧。
- 回归测试：`app_identity_runs_the_full_runner_lease_lifecycle`（verify-hub 身份
  全链 + 跨身份刷新 re-register）、`foreign_app_cannot_touch_another_apps_target`
  （覆盖注册/attach/renew 劫持全拒 + 管理编辑不夺 owner）。

### P2-2 测试

- `dispatch_adapter.rs`：9 个单测（executor 依赖抽成 `DispatchTaskStarter` trait）
  —— offer 幂等 / token 拒绝 / capacity Busy / activate 失败不消费 delivery /
  replay exactly-once / 并发不双启动 / journal 重开重放 / 损坏 fail-loud / 清理。
- `task_event_pump.rs`：pattern 聚合 + 事件路由 2 个单测。
- `dispatcher/tests.rs`：非 loopback endpoint 拒绝 + push 携带 lease token 2 个新测试。

---

## 遗留项（未在本轮实施）

- [ ] **容器形态部署回归（DV 验收）**：以 Agent app 容器形态启动 OpenDAN，
      验证 offer/activate 实际打通。协议层已有单测，但 `-p` 发布 + DNAT 路径
      只能在 DV 环境验证，CI 无此基础设施。
- [ ] **进程级故障注入**：dispatcher 重启（token 轮换自愈窗口）、OpenDAN 重启、
      offer ACK 丢失、容器重建，均待 DV 验收。
- [ ] **多节点 dispatch**：gateway 路由 + 服务发现解析 endpoint + 可验证
      caller attestation，一揽子后续设计（v1 已用 loopback 校验硬限定同机）。
- [ ] **Control Panel 观察面/审批面**：dispatch 记录无 UI；`approval_policy`
      恒 `Never` 所以审批门仍未被真实使用（M4 的审批 API 在，缺 UI/websdk）。
- [ ] **P2-3 生态缺口（决策项）**：`dispatch_task` 仍无生产调用方——第一个真实
      caller 是 Workflow / control_panel / 外部 IM 入口需要产品决策；
      `operation_route` 无 seed 无 UI（默认路由指向哪个 agent 是 zone 管理决策，
      不宜代码硬 seed）。
- [ ] **`task-dispatch-center-todo.md` 重写**：仍是 2.0 之前口径，需标注失效或重写。

## 附：复核方法

```bash
# runner 注册点（应当只有 dispatch_adapter 一处）
grep -rn "register_target" --include="*.rs" src/ | grep -v "/target/"

# kernel token 不再流向 runner（KrpcRunnerCaller 应只用 auth_token 参数）
grep -n "get_session_token" src/kernel/task_manager/src/dispatcher/service.rs

# runner kapi 固定端口挂载
grep -n "OPENDAN_SERVICE_PORT\|Runner::new" src/frame/opendan/src/dispatch_adapter.rs

# 单测
cargo test -p opendan --lib dispatch_adapter
cargo test -p opendan --lib task_event_pump
cargo test -p task_manager
```

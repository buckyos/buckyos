# 01. 总览与设计差异点
欢迎阅读《BuckyOS 架构设计》

本文假设你是开发者，对常见的分布式操作系统，或则云计算的基础概念有一定了解，并希望能成为BuckyOS贡献者

如果你只是希望在BuckyOS上开发dapp,或是想让自己的硬件产品与BuckyOS兼容,我们也强烈建议你在首先阅读《BuckyOS SDK》后，再通读一遍本文，以建立对BuckyOS整体是怎么工作的有一个更完整的理解。


本文档回答三个问题：
1) BuckyOS 运行时系统“长什么样”；
2) 哪些组件构成最短关键路径；
3) 与常见系统相比，BuckyOS 最不一样的设计点是什么。

## 基础概念(建立新的心智模型)

把 BuckyOS 看成一个以“Zone”为单位的家庭/小团队分布式 OS：
- system-config 是 Zone 的唯一真相源（KV）。
- scheduler 是“确定性推导器”：从系统状态推导出一致的运行意图（instances、service_info、rbac、gateway 配置等），并写回 system-config。
- node-daemon 是“节点收敛器”：每台机器上循环读取 node_config，把本机拉到目标状态（安装、部署、启动、停止、升级）。
- cyfs-gateway 是“网络内核”：把 NAT、多网段、端口不确定等现实问题抽象掉，给 app/service 一个稳定入口。

这套结构的核心目的不是“更像云”，而是：在家庭/边缘不稳定网络里做到可用、可交付、可升级、可控。

## 系统交付形态（运行时视角）

- 多进程系统：核心能力以 kernel services + frame services + apps 组合交付（模块边界见 `src/README.md`）。
- 交付单位是 pkg：repo-server + pkg-index-db + chunk 负责分发面；node-daemon + pkg-env 负责节点落地。

## 最短关键路径（最小可用闭环）

当系统从“未激活 / 冷启动”进入可用状态，最短闭环通常是：

1) 激活：设备暴露 3182，完成 owner/zone 绑定，写入必要身份与 ZoneBootConfig（协议思路见 `new_doc/ref/notepads/设备激活协议.md`）。
2) 启动关键三件套：cyfs-gateway（网络底座） + system-config（状态存储） + node-daemon（执行代理）。
3) boot 调度：scheduler 在 boot 阶段生成 `boot/config`（ZoneConfig）及初始服务/权限/节点配置（`src/kernel/scheduler/src/main.rs`）。
4) 常规调度循环：scheduler 持续根据状态变化更新 node_config / service_info / rbac（`src/kernel/scheduler/src/system_config_agent.rs`）。
5) 节点收敛：node-daemon 执行安装/部署/运行，服务可被访问。

下面的“差异点”都围绕这条链路的可靠性与可理解性展开。

## BuckyOS 的“非典型”设计点（对比常见系统）

### 1) 用 system-config（KV）承载“控制面状态”，而不是堆一个强中心控制面

BuckyOS 把大量控制面复杂性收敛为一个可审计、可事务化的 KV 状态模型：
- 用户/设备/服务/应用/权限/调度结果都以 KV 形式表达。
- scheduler 基于 KV 推导结果并写回（而不是把关键状态留在内存或多服务协同里）。

代码/路径锚点：
- `boot/config`、`system/rbac/*`、`system/scheduler/snapshot`（见 `src/kernel/scheduler/src/system_config_agent.rs`）
- system-config 服务端：`src/kernel/sys_config_service/src/main.rs`

对比 K8s：
- K8s 也是 etcd + controllers，但它的“可用性假设”和家庭环境不同：
  - K8s 默认假设节点网络/时间/基础设施相对稳定；
  - BuckyOS 默认假设 NAT/多网段/公网不可达是常态，因此会把网关/隧道放进主链路。

收益：
- 状态“落盘、可检索、可回放”：很多疑难问题可以通过 KV 的变化和 scheduler snapshot 定位。
- 降低隐式耦合：组件之间靠 KV 接口协作，减少“你需要知道我内部在干什么”的复杂依赖。

代价：
- 任何需要强一致的“跨组件即时联动”都必须通过 KV/调度循环实现，设计上要刻意避免把实时性写死。

常见误解：
- 误解：system-config 只是配置中心。
- 实际：它承载的是控制面状态（含调度结果、权限、网关暴露等），是系统核心数据库。

### 2) scheduler 强调“确定性/幂等”，并把结果写回，而不是做事件驱动的局部优化

> 面向复杂计算的cacl_scheduler还未实现，但其实现是基于scheduler的

调度器关注幂等性（`scheduler.md`），实现中也通过 snapshot 对比来避免无效写入（`src/kernel/scheduler/src/system_config_agent.rs`）。

BuckyOS 调度不仅做 placement，还要同时推导：
- instance（node_config 里的运行意图）
- service_info（可达信息）
- RBAC policy（权限）
- gateway 配置（可暴露/可访问路径）

对比 K8s：
- K8s 的 scheduler 更专注于 placement；服务发现、网络暴露、证书、权限、配置分别由不同控制器/组件完成。
- BuckyOS 更像“系统一致性生成器”：把这些结果一起推导出来写回同一个真相源。

收益：
- 系统行为更可推理：你只要理解输入状态与推导规则，就能解释“为什么会这样跑”。

代价：
- 必须保持数据模型边界清晰，否则 scheduler 会被迫理解过多业务细节。

常见误解：
- 误解：调度器只管把服务放到哪台机器。
- 实际：它还负责生成 service_info、rbac 等一致性配置，决定“怎么被访问、谁能访问”。

### 3) 端口与访问范式：实例先启动再上报，服务发现后置（对家庭环境更友好）

BuckyOS 不假设“所有服务端口固定”，而是采用范式：
- instance 启动时从起始端口开始尝试 bind，成功后上报 instance_info（含实际端口）。
- scheduler 基于 instance_info + service_settings 构造 service_info。
- 客户端基于 service_info 选择最终 URL。

参考：
- `scheduler.md`（范式描述）

收益：
- 避免家庭环境里“端口冲突/端口占用/不可控”的大量运维问题。

代价：
- 观察与调试必须以 service_info 为准，不能“凭经验记端口”。

常见误解：
- 误解：看到某服务配置了起始端口，就当它固定。
- 实际：起始端口只是 hint；最终端口以 instance_info/service_info 为准。

### 4) 网络底座优先：cyfs-gateway + rtcp 把 NAT 视为默认场景

BuckyOS 的默认假设是：家庭网络拓扑不稳定、没有公网 IP、端口映射不可控。
因此访问链路优先通过网关抽象而不是假设固定 IP：
- NodeGateway 提供 `http://127.0.0.1:3180/kapi/<service_name>` 的一致入口（`buckyos-api-runtime.md`）。
- ZoneGateway 提供对外入口（HTTP/HTTPS/转发），并承担公网暴露的控制点（`zone-boot-config与zone-gateway.md`）。
- SN 提供 DDNS/证书挑战/转发协助（可选，见 `SN.md`）。

对比常见家庭 NAS：
- NAS 通常走“厂商云中继/单机入口/固定端口映射”，多设备协同和服务暴露的系统化程度有限。
- BuckyOS 以 Zone 为中心，把“外部访问/内网访问/跨网段访问”统一进网关与 service_info 模型。

收益：
- 网络现实被系统吞掉，app/service 不需要把 NAT 当成业务逻辑处理。

代价：
- 网关成为主链路依赖：NodeGateway/ZoneGateway 的稳定性和配置正确性至关重要。

常见误解：
- 误解：只要能 https 访问 ZoneGateway 就够了。
- 实际：很多内部访问路径与 rtcp/NodeGateway 强相关，尤其是“本机 127.0.0.1 一致入口”的模式。

### 5) Zone 为单位的 Secure Boot：ZoneBootConfig 把“集群信任”前置

ZoneBootConfig（JWT/可验证）是 BuckyOS 引导阶段的关键对象：
- 解决“设备如何知道自己属于哪个 Zone、信任谁、该连谁”。
- 引导阶段强调建立 OOD 之间的 rtcp tunnel（以及可选的 SN 辅助）。

参考：
- `zone-boot-config与zone-gateway.md`

对比：
- 家用 NAS/单机系统通常没有“集群级别”的引导信任链。
- 传统分布式系统常把“发现/信任”交给运维（写配置、发证书、开端口），而 BuckyOS 把它产品化为系统主链路。

收益：
- 家庭环境可复制：不用假设用户会做证书分发、端口规划、节点发现。

代价：
- owner 密钥/信任链变化会导致启动语义更严格（这是一种安全优先的取舍）。

常见误解：
- 误解：ZoneBootConfig 只是“启动参数”。
- 实际：它承载的是引导阶段的信任根和联通策略，决定系统能否安全启动。

### 6) 包交付优先：pkg-system 把“升级”当成系统一级能力（ready 语义）

BuckyOS 的交付假设不是“手工升级/运维”，而是：
- repo-server 持有 pkg-index-db 与 pkg 数据，负责把目标版本准备成 ready。
- node-daemon 基于 pkg-index-db 变化与 node_config 变化触发部署/升级。
- 通过“准备（ready）→ 写系统意图 → 调度 → 节点收敛”的链路降低失败面。

参考：
- `app-pkg-system.md`（ready/升级触发/主循环伪代码）

对比 K8s 镜像分发：
- K8s 假设镜像仓库可达（或用本地缓存/镜像预拉取优化），但家庭离线/断网/多架构场景更难保证。
- BuckyOS 把 ready 前置到 Zone 内 repo-server，强调“Zone 内 zero-depend”。

收益：
- 升级更像“系统自愈”：只要 repo-server ready，节点可以在不依赖公网的情况下反复尝试收敛。

代价：
- repo-server 的磁盘占用、GC 策略、索引体积等会成为系统长期运行的关键工程问题。

常见误解：
- 误解：升级失败就是 node-daemon 的问题。
- 实际：先分清是“repo-server 未 ready / 索引不一致”还是“节点部署脚本/运行时失败”。

### 7) 权限是主链路：verify-hub + RBAC 贯穿访问、调度与运行

- verify-hub 是 token 的签发与验证中心（`src/kernel/verify_hub/README.md`、`src/kernel/verify_hub/src/main.rs`）。
- RBAC 策略存储在 system-config（`system/rbac/*`），并会被缓存；notepads 记录传播延迟最长约 10s（`rbac.md`）。

对比常见系统：
- 很多系统把权限集中在“入口网关/控制面 API”，内部服务间调用默认信任。
- BuckyOS 更像 OS：内部服务调用同样需要携带可验证身份，并受 RBAC 限制。

收益：
- 权限模型一致：无论是公网访问、内网访问、服务间调用，都走同一套身份/策略语义。

代价：
- 权限调试必须理解“token 验证 + RBAC enforce + 缓存延迟”的组合，而不是只看一个入口层。

常见误解：
- 误解：改完权限立刻生效。
- 实际：存在缓存延迟，系统级一致性传播需要时间窗口（当前实现最长约 10s）。

## 关键数据结构

本节只列出“理解系统主链路必须知道”的字段，忽略大量实现细节。

### 1) node_config（调度器写、node-daemon 读）
对应代码：`src/kernel/buckyos-api/src/control_panel.rs`

```rust
pub struct NodeConfig {
    pub node_id: String,
    pub node_did: String,
    pub kernel: HashMap<String, KernelServiceInstanceConfig>,
    pub apps: HashMap<String, AppServiceInstanceConfig>,
    pub frame_services: HashMap<String, FrameServiceInstanceConfig>,
    pub state: NodeState,
}
```

含义：
- `kernel/apps/frame_services` 是“本机应该跑哪些实例、目标状态是什么”的一阶事实。
- node-daemon 的核心循环目标是让本机收敛到这些目标状态。

### 2) instance 上报（node-daemon 写、scheduler 读）
对应代码：`src/kernel/buckyos-api/src/app_mgr.rs`

```rust
pub struct ServiceInstanceReportInfo {
    pub instance_id: String,
    pub node_id: String,
    pub node_did: DID,
    pub state: ServiceInstanceState,
    pub service_ports: HashMap<String, u16>,
    pub last_update_time: u64,
    pub start_time: u64,
    pub pid: u32,
}
```

含义：
- `service_ports` 是“端口后置发现”的关键输入：实例启动后实际监听端口由这里上报。

### 3) service_info（scheduler 写，client/runtime 读）
对应代码：`src/kernel/buckyos-api/src/app_mgr.rs`

```rust
pub struct ServiceInfo {
    pub selector_type: String, // current: random
    pub node_list: HashMap<String, ServiceNode>,
}

pub struct ServiceNode {
    pub node_did: DID,
    pub node_net_id: Option<String>,
    pub state: ServiceInstanceState,
    pub weight: u32,
    pub service_port: HashMap<String, u16>,
}
```

含义：
- service_info 是“客户端该如何找到一个可用实例”的唯一依据（而不是固定端口/固定 IP）。

### 4) service spec 与安装配置（交付/运行时的粘合面）
对应代码：`src/kernel/buckyos-api/src/app_mgr.rs`

```rust
pub struct ServiceInstallConfig {
    pub data_mount_point: HashMap<String, String>,
    pub cache_mount_point: Vec<String>,
    pub local_cache_mount_point: Vec<String>,
    pub bind_address: Option<String>, // None => bind 127.0.0.1
    pub expose_config: HashMap<String, ServiceExposeConfig>,
    pub container_param: Option<String>,
    pub start_param: Option<String>,
    pub res_pool_id: String,
}
```

含义：
- `bind_address: None` 强化“默认只允许本机访问，需要通过 rtcp/网关转发”的安全策略。
- `expose_config` 决定哪些端口/子域名会被暴露给 Zone 内/公网。

## 关键流程伪代码（主链路）

伪代码不是实现细节复刻，而是为了表达系统的控制流与数据依赖。

### 1) Boot Scheduler：生成 boot/config 与初始系统配置
对应线索：`src/kernel/scheduler/src/main.rs`

```text
boot_scheduler():
  boot_jwt = env[BUCKYOS_ZONE_BOOT_CONFIG]
  if system_config.get("boot/config") exists:
    fail("already booted")

  init_map = build_init_list_by_template(boot_jwt)
  // includes: boot/config, services/*/spec, system/rbac/{model,base_policy}, verify-hub
  system_config.exec_tx(init_map)
```

### 2) Scheduler Loop：读状态 → 推导动作 → 写回结果
对应线索：`src/kernel/scheduler/src/system_config_agent.rs`

```text
scheduler_loop():
  loop:
    input = system_config.dump_configs_for_scheduler()
    ctx = build_scheduler_ctx(input)      // nodes/specs/users/instances...
    last = input.get("system/scheduler/snapshot")

    actions = schedule(ctx, last)
    if actions.is_empty():
      sleep
      continue

    kv_updates = actions_to_kv(actions)
    system_config.exec_tx(kv_updates)

    system_config.create("system/scheduler/snapshot", serialize(ctx))
    sleep
```

### 3) Node Convergence：node-daemon 拉本机到 node_config
对应线索：notepads 主循环描述：`new_doc/ref/notepads/app-pkg-system.md`

注意：这里用到了两个关键事实（在系统里很“非传统”）：
- node-daemon 不直接“问控制面该做什么”，而是只认 `node_config` 作为目标状态描述。
- 服务端口不是预分配的：实例先启动再上报 `ServiceInstanceReportInfo.service_ports`，随后由 scheduler 汇总成 `service_info`。

```text
node_daemon_main_loop():
  sync_pkg_index_db_from_repo_server()

  node_config = system_config.get("nodes/<node_id>/config")
  for inst in node_config.kernel/apps/frame_services:
    pkg = env.try_load(inst.pkg_id)        // pkg-index-db decides "latest"
    if pkg.not_installed:
      env.install(pkg)
      pkg.deploy()

    if pkg.status() != inst.target_state:
      pkg.control_to(inst.target_state)    // start/stop + kill old versions

    report = build_instance_report(pid, ports, state)
    system_config.set("services/<svc>/instances/<node>", report)

  sleep
```

### 4) Auth + RBAC：verify-hub 发 token，服务侧 enforce
对应线索：`src/kernel/verify_hub/README.md` + `new_doc/ref/notepads/rbac.md`

```text
service_request(req):
  token = req.session_token
  (userid, appid) = verify_token(token)         // trust keys include verify-hub

  allowed = rbac.enforce(userid, appid, req.resource, req.action)
  if !allowed:
    deny

  handle
```

### 5) Upgrade Pipeline：ready → 写意图 → 调度 → 收敛
对应线索：`new_doc/ref/notepads/app-pkg-system.md`

```text
upgrade(app_id, target_version):
  repo_server.sync_pkg_index_db()
  repo_server.prepare_ready(app_id, target_version)  // download deps/subpkgs/chunks

  system_config.set("users/<u>/apps/<app>/config", new_spec)
  trigger_scheduler()

  // scheduler updates node_config; node-daemon converges automatically
```

## 读完这章后你应该能回答
- “为什么 BuckyOS 不是一个简化版 K8s，也不是一个 NAS 上跑容器的套壳？”
- “为什么它把网关、Secure Boot、包系统、RBAC 放到主链路？”
- “出现问题时，我应该从 KV 状态、调度结果、还是节点收敛/ready 环节去定位？”

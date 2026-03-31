# 第三章：启动自己的 Zone

第二章里，我们从一次“在浏览器里访问 Zone 内 App 服务”的完整链路出发，认识了 Zone、Node、OOD、ZoneGateway、SN、Tunnel/RTCP 这些概念。

但如果你只是“看懂了别人 Zone 上的服务是怎么被访问的”，还不算真正理解 BuckyOS。
真正的分水岭是：你能不能从一台“什么都没有的设备”开始，让它加入一个 Zone，并让整个 Zone 进入可用状态。

本章会用“现有实现里的真实链路”回答这个问题：

- 设备为什么必须先“激活”（Activation）？
- 激活到底写了哪些文件？为什么不是直接写 system-config？
- ZoneBootConfig 在启动中扮演什么角色？
- 第一次启动时，system-config/scheduler/node-daemon 分别在做什么？

为了避免跑题，本章只覆盖“把一个 OOD 节点启动成可用 Zone”的主链路。多 OOD（2n+1）、公网可达性、跨网段访问等，会在后续章节展开。

## 1. 启动一切之前：你需要一个“信任根”

在 BuckyOS 里，“能连上”不是核心，“能安全地连上”才是核心。
因此系统启动的第一件事不是拉起服务，而是建立一个可验证的信任边界：

- 你加入的是哪个 Zone（`zone_did`）
- 这个 Zone 的 owner 是谁（`owner_did` / `owner_public_key`）
- 这台设备是谁（Device 的 document / JWT）

这套信息在实现里会被固化为一组落盘文件，作为后续 Secure Boot 的输入。

## 2. 激活（Activation）：把设备纳入某个 Zone 的信任边界

在未激活状态下，node-daemon 会提供一个“激活服务”，用于接收用户侧/控制面发来的激活请求：

- 默认端口：3182
- 实现：`src/kernel/node_daemon/src/active_server.rs`（`ACTIVE_SERVICE_MAIN_PORT`）
- 协议说明入口：`new_doc/ref/notepads/设备激活协议.md`

激活的结果不是写入 system-config（KV），而是写入“系统 etc 目录”（由 `get_buckyos_system_etc_dir()` 决定）。原因很现实：

- 激活发生在“系统控制面尚未建立”的阶段；此时 system-config 可能还不存在，也可能还不可信。
- 激活的目标是把最小可信信息固化下来，让后续启动链路可以在不依赖外部运维的前提下自举。

### 2.1. 激活的落盘结果（非常关键）

根据当前实现（`doc/arch/02_boot_and_activation.md` 对代码的归纳），激活阶段会写入这些关键文件：

- `node_private_key.pem`：本设备 Ed25519 私钥（`src/kernel/node_daemon/src/active_server.rs`）
- `node_identity.json`：设备与 Zone/Owner 的绑定信息（`src/kernel/node_daemon/src/active_server.rs`）
- `start_config.json`：后续 boot.template 渲染所需参数；激活会把 `ood_jwt` 写入其中（`src/kernel/node_daemon/src/active_server.rs`）
- `node_device_config.json`：DeviceConfig 的 JSON 形式，供启动阶段读取（`src/kernel/node_daemon/src/active_server.rs`）

其中 `node_identity.json` 是最核心的“粘合剂”。它把 `zone_did`、`owner_public_key`、设备文档 JWT 固化下来，让启动阶段可以验证“我是谁、我属于谁、我要加入哪个 Zone”。

### 2.2. NodeIdentityConfig（最小视图）

代码里写入的最小字段视图（节选自 `doc/arch/02_boot_and_activation.md`）：

```rust
let node_identity = NodeIdentityConfig {
    zone_did: zone_did,
    owner_public_key: owner_public_key,
    owner_did: owner_did,
    device_doc_jwt: device_doc_jwt.to_string(),
    zone_iat: (buckyos_get_unix_timestamp() as u32 - 3600),
    device_mini_doc_jwt: device_mini_doc_jwt.to_string(),
};
```

你可以把它理解为：后续所有启动动作，都必须能从这份文件出发推导出来，否则系统就无法做到“Zero-OPS 自举”。

## 3. ZoneBootConfig：Secure Boot 的最小可信输入

从设计意图上看，ZoneBootConfig 是“引导阶段的最小可信输入”，它用于解决：

- 我加入的 Zone 的 OOD 列表是什么？（单 OOD / 多 OOD）
- 是否启用了 SN？（影响公网可达性与域名解析模式）
- 这些信息是否带有 owner 可验证的签名？（避免被 DNS 污染/劫持）

实现里，node-daemon 会在启动阶段尝试拿到 ZoneBootConfig：

- 入口：`looking_zone_boot_config()`（`src/kernel/node_daemon/src/node_daemon.rs`）
- 正常路径：调用 `resolve_did(&node_identity.zone_did, Some("boot"))` 获取 zone_doc，再用 owner public key 验证并 decode（`src/kernel/node_daemon/src/node_daemon.rs`）

### 3.1. 一个很“实现细节但必须知道”的点：当前是 JSON 而不是 JWT

很多文档会把 ZoneBootConfig 讲成“JWT”，但当前实现里，scheduler `--boot` 读取的是环境变量 `BUCKYOS_ZONE_BOOT_CONFIG`，并按 JSON 反序列化：

- 读取点：`src/kernel/scheduler/src/main.rs`（`do_boot_scheduler()`）

也就是说：

- ZoneBootConfig 的“存储/传输形态”可能是 JWT
- 但 node-daemon 传递给 scheduler 的形态，是 `ZoneBootConfig` 结构体的 JSON 字符串（见 `doc/arch/02_boot_and_activation.md` 的归纳）

### 3.2. 离线/调试模式：从本地文件绕开解析

`looking_zone_boot_config()` 还提供了一个很实用的 debug 入口：

- 如果存在 `./<zone_raw_host_name>.zone.json`，会优先从本地加载并解析
- 代码：`src/kernel/node_daemon/src/node_daemon.rs`

这让你在离线环境里也能绕开 DNS/BNS/SN 的解析链路，直接验证后续 boot/scheduler 的行为。

## 4. 第一次启动：system-config + scheduler `--boot` 生成 `boot/config`

当设备已激活、ZoneBootConfig 可被解析之后，系统才进入“真正的启动”。

这一步的关键目标是：在 system-config（KV Source of Truth）里写入 `boot/config`，让整个 Zone 获得一个可被后续组件共同读取的“唯一真相”。

### 4.1. system-config：为什么它是“真相源”

`doc/arch/03_system_config.md` 给 system-config 的定位非常直接：它承载 `boot/config`、service spec、node_config、RBAC 模型与策略、调度快照等，并为 scheduler/node-daemon 提供批量 dump。

实现锚点：

- 服务端：`src/kernel/sys_config_service/src/main.rs`
- 固定端口：3200（`SYSTEM_CONFIG_SERVICE_MAIN_PORT: 3200`）

### 4.2. scheduler `--boot`：只在首次引导时运行一次

首次启动时，scheduler 的 `--boot` 分支会负责生成初始化 KV，并写入 system-config：

- 入口：`src/kernel/scheduler/src/main.rs`（`--boot` / `do_boot_scheduler()`）
- 关键约束：如果 `boot/config` 已存在，`--boot` 会失败（避免覆盖已初始化的 Zone）

在 `doc/arch/02_boot_and_activation.md` 里，这条链路被总结为：

1) 激活落盘（etc）
2) node-daemon 启动，解析 ZoneBootConfig
3) 启动 system-config
4) 发现 `boot/config` 不存在，则运行 `scheduler --boot`
5) `--boot` 渲染模板、生成 init_list，并通过 `SystemConfigBuilder` 写入一组核心 KV

### 4.3. `boot/config` 不只是 Zone 信息，它还是 trust_keys 的依赖

这点非常容易被忽略：

- scheduler `--boot` 在写入 KV 并完成一次 boot 调度后，会调用 `system_config_client.refresh_trust_keys()`（`doc/arch/02_boot_and_activation.md` 指向 `src/kernel/scheduler/src/main.rs`）
- system-config 服务侧会在 `handle_refresh_trust_keys()` 中读取 `boot/config`，把 owner key / verify-hub public key 加入信任列表（`src/kernel/sys_config_service/src/main.rs`）

因此，`boot/config` 的缺失/不完整会导致大量看起来“无关”的权限错误（token 校验失败 / NoPermission），这也是排障时经常遇到的根因。

## 5. 启动完成后的“常态”：调度收敛与访问入口

当 `boot/config` 写入完成后，系统进入常态循环：

- scheduler 周期性 dump system-config → 推导调度动作 → 事务写回（`doc/arch/04_scheduler.md`，实现锚点 `src/kernel/scheduler/src/system_config_agent.rs`）
- node-daemon 周期性读取 `nodes/<node>/config` 并收敛本机状态（安装/部署/启动/停止/升级）
- cyfs-gateway 提供 NodeGateway 的一致入口（默认 `http://127.0.0.1:3180/kapi/<service>`，详见 `doc/arch/06_network_and_gateways.md` 与 `doc/arch/08_api_runtime.md`）

从这一刻起，你的 Zone 才算“真正活了过来”。

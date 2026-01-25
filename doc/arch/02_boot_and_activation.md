# 02. 启动与激活（ZoneBootConfig / Secure Boot）

本章描述从“未激活设备”到“Zone 内可用节点”的关键链路。

## 激活（Device Activation）
激活是“把设备纳入某个 Zone 的信任边界”的过程。

当前实现要点：
- 设备在未激活状态提供激活服务，端口为 3182（`src/kernel/node_daemon/src/active_server.rs`，`ACTIVE_SERVICE_MAIN_PORT`）。
- 典型发现方式包括：扫描/连接 3182，或 UDP 广播探测（见 `new_doc/ref/notepads/设备激活协议.md`）。

### 激活产物（从代码可见的落盘结果）
激活阶段的“可持续状态”不是写 KV，而是写入 system etc 目录（由 `get_buckyos_system_etc_dir()` 决定，代码：`src/kernel/node_daemon/src/active_server.rs`）。这些文件会在下一次 node-daemon 启动时被读取，用来推导后续 boot 所需的环境变量与调度动作。

当前写入的关键文件（两条激活路径 `handle_active_by_wallet()` 与 `handle_do_active()` 都会写）：
- `node_private_key.pem`：本设备 Ed25519 私钥（`src/kernel/node_daemon/src/active_server.rs`）。
- `node_identity.json`：设备与 Zone/Owner 的绑定信息（见下方 `NodeIdentityConfig` 最小字段视图）。
- `start_config.json`：后续 boot.template 渲染所需的启动参数；激活会把 `ood_jwt` 写入其中（`src/kernel/node_daemon/src/active_server.rs`）。
- `node_device_config.json`：DeviceConfig 的 JSON 形式，供启动阶段读取（`src/kernel/node_daemon/src/active_server.rs`）。

注意：激活阶段不会写入 `boot/config`；`boot/config` 由首次启动时的 scheduler `--boot` 路径生成（`src/kernel/scheduler/src/main.rs`）。

### 关键数据结构：NodeIdentityConfig（最小字段视图）
对应代码：`src/kernel/node_daemon/src/active_server.rs`

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

含义（只解释主链路）：
- `zone_did`：用于启动阶段查询/验证 ZoneBootConfig。
- `owner_public_key` / `owner_did`：用于验证设备文档与 zone boot 文档的签名/一致性（具体校验逻辑以实现为准）。
- `device_doc_jwt`：设备 DeviceConfig 的 JWT（启动阶段会解码成 `BUCKYOS_THIS_DEVICE`）。

## ZoneBootConfig 的角色
ZoneBootConfig 是引导阶段的“最小可信输入”，其目标是确保系统能安全引导（Secure Boot）。

核心属性（以实现与 notepads 描述为准）：
- 它通常以 JWT 形式存在，并带有 owner 相关的可验证信息。
- 它包含 OOD 列表、可选 SN 信息等引导所需参数。

实现与设计说明：
- 设计讨论与安全边界：`new_doc/ref/notepads/再次整理zone-boot-config与zone-gateway.md`
- scheduler 在 boot 阶段读取 `BUCKYOS_ZONE_BOOT_CONFIG` 环境变量（`src/kernel/scheduler/src/main.rs`）

### 关键数据结构：ZoneBootConfig（最小字段视图）
本仓库内 `ZoneBootConfig` 的字段定义主要来自 `name_lib`，结构体字段在 notepads 中被明确列出（以该 notepad 为准）：`new_doc/ref/notepads/再次整理zone-boot-config与zone-gateway.md`

```rust
pub struct ZoneBootConfig {
    pub id: Option<DID>,
    pub oods: Vec<String>,
    pub sn: Option<String>,
    pub exp: u64,
    pub iat: u32,
    pub extra_info: HashMap<String, serde_json::Value>,

    pub owner: Option<DID>,
    pub owner_key: Option<Jwk>,
    pub gateway_devs: Vec<DID>,
}
```

### ZoneBootConfig 在启动链路中的“形态转换”
有一个容易误解的点：
- 设计上 ZoneBootConfig 常被描述为“JWT”；
- 但当前 scheduler `--boot` 的实现是从环境变量读取并按 JSON 反序列化（`serde_json::from_str`），也就是说 `BUCKYOS_ZONE_BOOT_CONFIG` 在当前实现里是“ZoneBootConfig 的 JSON 字符串”，而不是 JWT 字符串（`src/kernel/scheduler/src/main.rs`）。

而这个环境变量是在 node-daemon 的启动流程里设置的（同一处还会设置 `BUCKYOS_THIS_DEVICE`）：`src/kernel/node_daemon/src/node_daemon.rs`。

## Secure Boot 的核心逻辑
Secure Boot 在 BuckyOS 中要解决的问题不是“单机镜像可信”，而是：
- 设备如何验证自己加入的是正确的 Zone；
- 设备如何在网络不可信（DNS 污染、NAT、多网段）情况下建立可信连接；
- 多 OOD 场景下如何在 boot 阶段建立必要的互联。

根据 `new_doc/ref/notepads/再次整理zone-boot-config与zone-gateway.md` 的描述：
- OOD 会通过外部服务获取 ZoneBootConfig（DNS/BNS/其它可缓存 did 查询服务）。
- 验证包括“新旧版本比较 + owner 签名校验”。
- Boot 阶段需要与其它 OOD 建立 rtcp tunnel，并在满足条件后进入 system-config 启动阶段。

### Secure Boot Checks（从当前代码路径可见的约束点）
这里不复述 notepad 的完整设计，只列出“能从代码直接读到的 gate / hard-fail 条件”，用于理解为什么系统会卡在 boot：

- `BUCKYOS_ZONE_BOOT_CONFIG` 必须存在：scheduler `--boot` 若读不到会直接失败退出（`src/kernel/scheduler/src/main.rs`）。
- `boot/config` 只能在首次 boot/init 阶段创建：scheduler `--boot` 会先 `get("boot/config")`，存在则返回错误（`src/kernel/scheduler/src/main.rs`）。
- trust_keys 的刷新依赖 `boot/config`：scheduler `--boot` 在写入完 KV 并完成一次 `schedule_loop(true)` 后，会调用 `system_config_client.refresh_trust_keys()`（`src/kernel/scheduler/src/main.rs`）；system-config 侧会在 `handle_refresh_trust_keys()` 中读取 `boot/config` 并把 owner key / verify-hub public key 加入信任列表（`src/kernel/sys_config_service/src/main.rs`）。

从效果上看：`boot/config` 不是“单纯的 Zone 信息”，它还是 system-config 能否正确验证 session_token 的关键依赖（trust_keys 的来源之一）。

### 伪代码：notepad 视角的 Secure Boot 主链路
对应 notepad：`new_doc/ref/notepads/再次整理zone-boot-config与zone-gateway.md`

```text
secure_boot_for_ood():
  // 1) 获取 ZoneBootConfig（JWT）
  zone_boot_jwt = query_zone_boot_config_via(DNS | BNS | did-resolver)

  // 2) 验证（notepad 明确列出的两项）
  assert zone_boot_jwt is newer than cached            // "比已知的ZoneBootConfig更新"
  assert verify_owner_signature(zone_boot_jwt)         // "有ZoneOwner的签名"

  // 3) owner 公钥的可信来源（notepad 明确列出三种）
  owner_pk = choose_one_of(
    local_owner_pk_saved_at_activation,               // "激活时本地保存"
    query_owner_pk_via_BNS,                           // "通过BNS查询"
    query_owner_pk_via_DNS_TXT_PX0                    // "通过DNS的TXT记录(PX0)"
  )

  // 4) boot 阶段联通（multi-OOD 约束）
  if zone_boot_config.oods == 1:
    proceed_single_ood_boot()
  else if zone_boot_config.oods == 2n+1:
    keep_rtcp_tunnel_with_at_least_n_other_oods(n)
    only_then_enter_system_config_start()

  // 5) 直连尝试（notepad 给出候选信息源 + 端口 2980）
  ip = choose_ip_from(
    udp_broadcast,
    name_string_embedded_ip,
    dns_zone_hostname_or_device_subdomain,
    sn_query_device_info
  )
  connect_to(ip, 2980)
  exchange_device_config_and_verify(owner_pk)

  // 6) 直连失败时的中转尝试（notepad 给出 rtcp URL 形式）
  open_rtcp_stream("rtcp://<relay_did>/rtcp://<target_device_name>/")
```

## boot/config（ZoneConfig）的生成
boot/config（KV 路径 `boot/config`）是 system-config 中保存 ZoneConfig 的关键位置：
- scheduler 的 `--boot` 路径会检查 `boot/config` 是否已存在；不存在则生成初始化配置并写入（`src/kernel/scheduler/src/main.rs`）。
- boot 阶段会写入 RBAC 基础数据（`system/rbac/base_policy`、`system/rbac/model`），以及 verify-hub 相关配置（`src/kernel/scheduler/src/main.rs`、`src/kernel/scheduler/src/system_config_builder.rs`）。

实现线索：
- `src/kernel/scheduler/src/main.rs`：`do_boot_scheduler`、`create_init_list_by_template`。
- `src/kernel/scheduler/src/system_config_builder.rs`：`SystemConfigBuilder::add_boot_config`、`SystemConfigBuilder::add_verify_hub`。

### 关键 KV 路径（boot/init 阶段）
下面列出 `--boot` 会直接创建/间接生成的一组核心 KV key（以代码中的字符串常量/字面量为准）：

- `boot/config`：ZoneConfig（`src/kernel/scheduler/src/system_config_builder.rs`、`src/kernel/scheduler/src/main.rs`）。
- `system/rbac/base_policy`、`system/rbac/model`：RBAC 的初始模型与基础策略（`src/kernel/scheduler/src/main.rs`，以及模板渲染后兜底注入）。
- `system/verify-hub/key`：verify-hub 私钥（`src/kernel/scheduler/src/system_config_builder.rs`）。
- `services/verify-hub/spec`、`services/scheduler/spec`、`services/repo-service/spec`、`services/control-panel/spec`：核心 kernel service 的 spec（`src/kernel/scheduler/src/system_config_builder.rs`）。
- `users/root/settings`、`users/<admin>/settings`、`users/<admin>/doc`：默认账号与 owner doc（`src/kernel/scheduler/src/system_config_builder.rs`）。
- `devices/<ood>/doc`：OOD 的 device doc（JWT，来自 `start_config.json` 的 `ood_jwt`，`src/kernel/scheduler/src/system_config_builder.rs`）。
- `nodes/<ood>/config`、`nodes/<ood>/gateway_config`：节点初始配置（`src/kernel/scheduler/src/system_config_builder.rs`）。

### 关键数据结构：ZoneConfig（最小字段视图，按“代码可见字段”）
本仓库内 `ZoneConfig` 的完整字段同样来自 `name_lib`；本节只列出“在当前代码里被直接读取/依赖”的字段：

- `verify_hub_info.public_key`：system-config 在 `handle_refresh_trust_keys()` 里从 `boot/config` 取出并转换为 `DecodingKey`，用于信任 verify-hub token（`src/kernel/sys_config_service/src/main.rs`）。
- `owner` + `get_default_key()`：system-config 会把 owner 的默认 key 加入 trust_keys，并额外加入 `root`、`$default`（`src/kernel/sys_config_service/src/main.rs`）。

### 伪代码：activation -> boot scheduler -> 写入 boot/config（含 BUCKYOS_ZONE_BOOT_CONFIG）
这段伪代码以“控制流 + 数据依赖”为核心，刻意忽略实现细节。

```text
activation_server(3182):                        // src/kernel/node_daemon/src/active_server.rs
  receive {device_doc_jwt, device_private_key, owner_public_key, zone_did, ...}
  write /etc/node_private_key.pem
  write /etc/node_identity.json                 // NodeIdentityConfig
  write /etc/start_config.json                  // includes ood_jwt
  write /etc/node_device_config.json

node_daemon_boot():                              // src/kernel/node_daemon/src/node_daemon.rs
  node_identity = read /etc/node_identity.json
  device_doc    = decode(node_identity.device_doc_jwt, node_identity.owner_public_key)
  zone_boot_cfg = looking_zone_boot_config(node_identity)      // resolve via DNS/BNS/SN (impl)

  setenv BUCKYOS_ZONE_BOOT_CONFIG = json(zone_boot_cfg)        // NOT JWT in current impl
  setenv BUCKYOS_THIS_DEVICE      = json(device_doc)

  start system-config (3200)

  if system-config.get("boot/config") == KeyNotFound:
    setenv SCHEDULER_SESSION_TOKEN = device_session_token
    run scheduler --boot

scheduler --boot:                                 // src/kernel/scheduler/src/main.rs
  zone_boot_cfg_json = env[BUCKYOS_ZONE_BOOT_CONFIG]
  zone_boot_cfg      = json_parse(zone_boot_cfg_json)

  assert system_config.get("boot/config") == KeyNotFound

  init_list = render(/etc/scheduler/boot.template.toml, /etc/start_config.json)
  builder   = SystemConfigBuilder(init_list)
  builder.add_boot_config(start_config, verify_hub_public_key, zone_boot_cfg)
  ... add_verify_hub/add_scheduler/add_repo_service/add_control_panel/add_node ...
  init_list = builder.build()

  // ensure boot/config is a ZoneConfig that carries boot info
  zone_config = json_parse(init_list["boot/config"])
  zone_config.init_by_boot_config(zone_boot_cfg, zone_boot_cfg_json)
  init_list["boot/config"] = json_pretty(zone_config)

  for (k, v) in init_list:
    system_config.create(k, v)

  schedule_loop(boot=true)
  system_config.refresh_trust_keys()             // sys_refresh_trust_keys
```

### pitfalls / operational notes（短）
- `boot/config` 已存在时，scheduler `--boot` 会失败：这通常意味着系统已经完成过一次 boot/init（或 KV 被恢复过旧数据），需要先解释“为什么存在”再处理（`src/kernel/scheduler/src/main.rs`）。
- `boot/config` 与 trust_keys 强相关：即使 system-config 服务已启动，如果 `boot/config` 不完整/解析失败，会导致 `sys_refresh_trust_keys` 失败或 trust_keys 不全，从而引发大量 `NoPermission`/token 校验问题（`src/kernel/sys_config_service/src/main.rs`）。
- 首次启动的循环行为是预期路径：node-daemon 在读不到 `boot/config` 时会进入 BOOT_INIT 并反复尝试拉起 scheduler `--boot`（`src/kernel/node_daemon/src/node_daemon.rs`）。

## 与类似系统的差异点
- BuckyOS 把“集群级别引导”前置到一个可验证对象（ZoneBootConfig），把家庭网络的不确定性作为默认假设。
- 对比常见系统（K8s/传统集群）：BuckyOS 更强调在 boot 阶段就解决“可信身份 + NAT 下连通性”的问题，而不是假设有稳定的控制面网络。

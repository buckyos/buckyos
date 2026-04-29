## Zone Boot Config的设计

因为DNS的TXT Record长度限制，BuckyOS专门设计了3条TXT Record来共同构成可在Zone外可信引导Zone所需的最小信息，整体由name-lib的`ZoneBootConfig`承载：
- `BOOT=$ZONE_BOOT_CONFIG_JWT` —— ZoneBootConfig主体（owner私钥签名的JWT）
- `PKX=$OWNER_PUBLIC_KEY.X` —— Owner公钥（Ed25519的x分量），仅用于自验证BOOT签名
- `DEV=$GATEWAY_DEVICE_MINI_DOC_JWT` —— ZoneGateway/OOD的`DeviceMiniConfig`（owner签名的JWT），主要携带rtcp端口等boot阶段必须的字段

具体实现参考 `buckyos-base/src/name-lib/src/zone.rs` 的 `ZoneBootConfig`，以及name-client里的`parse_txt_record_to_did_document`。

```rust
// buckyos-base/src/name-lib/src/zone.rs
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ZoneBootConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<DID>,
    pub oods: Vec<OODDescriptionString>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sn: Option<String>,
    pub exp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<DID>,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,
    //------- 下面的字段不会序列化进BOOT这条TXT，而是单独保存在其它TXT Record中 ------------
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_key: Option<Jwk>, // 来自 PKX=0:xxxxxxx;
}
```

> 配套的辅助方法见同文件：`encode/decode`(JWT)、`device_is_ood`、`device_is_gateway`、`get_gateway_name`、`to_zone_config`等。

## ZoneBootConfig与ZoneConfig的区别

- ZoneBootConfig是对ZoneConfig的极致压缩，100%保存在Zone外
- ZoneConfig则通常是Zone启动后，由ZoneProvider实现
- Zone间通信一般只依赖 Remote ZoneConfig

99%的情况下，都不应该直接使用ZoneBootConfig,而是使用基于DNS TXT Record 转换得到的ZoneConfig

## ZoneBootConfig的 首要目标：确保系统能安全引导 (Secure Boot)
OOD的安全启动引导流程如下（实现位置：`src/kernel/node_daemon/src/node_daemon.rs` 的 `looking_zone_boot_config`）：

1. OOD通过DNS查询得到ZoneBootConfig(JWT格式)
   - 调试/离线场景：若`{zone_name}.zone.json`存在，会被优先加载，绕过DNS（见`looking_zone_boot_config`内的本地分支）
2. 对ZoneBootConfig进行验证：
    - 比已知的ZoneBootConfig更新（尽力防止DNS重放攻击）
    - 有ZoneOwner的签名（依赖`ZoneBootConfig::decode`使用Owner公钥校验）
3. 验证ZoneOwner的签名需要可信的获得其公钥，目前有以下来源：
    - OOD在激活时已经在本地保存了Owner公钥（`NodeIdentityConfig.owner_public_key`，复用DID-Document存储底层设施）
    - 通过BNS查询Owner的公钥（最权威），用以支持Owner在私钥丢失时更换公钥
    - 解析器已经基于`PKX`这条TXT对ZoneBootConfig的JWT进行了自验证（`parse_txt_record_to_did_document`）
4. ZoneBootConfig中只有1个OOD —— 单点启动引导（目前默认实现）
5. ZoneBootConfig中有2n+1个OOD —— OOD需要在boot阶段与n个其它OOD建立rtcp tunnel才会进入下一步。基于BootConfig里的SN信息和`OODDescriptionString`，尝试与其他OOD在Boot阶段建立连接；配置了SN则多一种通过SN中转的连接方法
6. Boot阶段，OOD会持续地尝试与其它OOD保持rtcp tunnel；当与n个ood keep tunnel成功后，才进入system config的启动阶段
7. OOD之间建立rtcp tunnel的方法：

   7.1 基于Device Name尝试直连（Zone内设备的直连几乎都走这个流程，核心实现见`src/kernel/node_daemon/src/finder.rs`）
   - 尝试拿到目标OOD的一个IP地址，候选来源：
     - UDP广播（`FINDER_SERVER_UDP_PORT = 2980`，广播间隔`FINDER_BROADCAST_INTERVAL_SECS = 2s`）
     - 兜底TCP扫描：同子网C段最多254个地址（见finder.rs顶部注释）
     - `OODDescriptionString`中显式携带的IP
     - `resolve_did("did:bns:$oodname.$zonename")`，得到包含IP的OOD DeviceDocument
     - 通过DNS A/AAAA查询：`zone_hostname`（只有1个gateway-ood时）或 `$oodname.$zonehostname`
     - 历史发现缓存：`finder_cache.json`，TTL = 7天（`FINDER_CACHE_TTL_SECS`）
   - 与该IP地址的RTCP端口（默认`2980`，由`DeviceMiniConfig.rtcp_port`覆盖）建立连接
   - 连接成功后进入RTCP握手
     - 使用双方的Device Public Key协商session key
     - RTCP握手时会对更完整的DeviceConfig进行交换验证（TODO：RTCP 安全机制未完全实现）

   7.2 尝试通过中转建立连接（此时无法keep tunnel）
   当无法直接建立连接时，会视是否有中转节点来尝试通过中转建立rtcp stream，open stream的形式：
   `rtcp://$中转设备did/rtcp://目标设备名/` （TODO：中转设备能解码rtcp流量，存在隐私泄露风险）
   - boot阶段可用的中转设备信息来自ZoneBootConfig，优先级 `WAN可达的OOD（net_id以wan开头）-> ZoneGateway-> SN`
     - 与中转设备建立rtcp tunnel的逻辑与7.1相同；连上后会`keep tunnel`
     - SN可能基于自己的逻辑阻止任意中转，只允许OOD到OOD的中转
8. 系统首次启动时，调度器会基于ZoneBootConfig构造正式的ZoneConfig，核心逻辑在
[system_config_builder.rs add_boot_config](src/kernel/scheduler/src/system_config_builder.rs:499)：
```rust
pub fn add_boot_config(
    &mut self,
    config: &StartConfigSummary,
    verify_hub_public_key: &Jwk,
    zone_boot_config: &ZoneBootConfig,
) -> Result<&mut Self> {
    let public_key_value = verify_hub_public_key.clone();
    let zone_did = DID::from_str(&config.zone_name)?;
    let mut zone_config = ZoneConfig::new(
        zone_did,
        DID::new("bns", &config.user_name),
        config.public_key.clone(),
    );

    let verify_hub_info = VerifyHubInfo { public_key: public_key_value };
    let boot_jwt = config.ood_jwt.clone().unwrap_or_default();
    zone_config.init_by_boot_config(zone_boot_config, &boot_jwt);
    zone_config.verify_hub_info = Some(verify_hub_info);
    self.insert_json("boot/config", &zone_config)?;
    Ok(self)
}
```
注：`ZoneConfig::init_by_boot_config`会把`oods`、`sn`、`exp`、`owner`、`owner_key`等字段拷贝到ZoneConfig，并保留`boot_jwt`原文用于后续校验（见name-lib `zone.rs init_by_boot_config`）。

### 非OOD(Node/Client)的启动流程(与ZoneBootConfig无关) `部分未实现`
1. Node启动时系统通常已经启动完成，因此Node启动的核心目标是连接上SystemConfig Service
2. Node可以基于OOD搜索流程，主动尝试连接OOD（可以避免ZoneGateway失效导致的内网不可用）
3. Node通过ZoneGateway也可以直接访问SystemConfig Service（优先rtcp）
4. 通过SystemConfig Service返回的OOD DeviceInfo，可以使用最佳的方法与OOD建立RTCP连接（尽量直连），提高后续访问的速度

> 当前代码现状：非OOD分支在 [node_daemon.rs](src/kernel/node_daemon/src/node_daemon.rs) 中获取SystemConfigClient的路径仍是`unimplemented!()`，仅OOD分支可独立完成boot。Node目前必须等系统启动后由OOD推送配置。

## ZoneGateway的定义与确定
- ZoneGateway通常是OOD，但也可以是普通Node。`ZoneBootConfig`里第一个`node_type.is_gateway()`为真的条目即为默认ZoneGateway（参见`ZoneConfig::get_default_zone_gateway`）
- 一个典型的小型混合配置：`oods: ["$ood1", "#gate1:210.35.22.1"]`，意为：内网`ood1`只承担OOD职责，公网`gate1`只承担Gateway职责（IP是210.35.22.1）。这样可以用一台廉价VPS充当ZoneGateway，从而摆脱对SN的依赖
- OOD和ZoneGatewayNode都持有zone hostname的tls证书，会启动tls stack（可选，但一般都有）
  - tls证书通过配置获得，通常配置 `$zonehostname + *.$zonehostname` 两本tls证书以支持https访问
  - 在有SN的情况下，SN收到tls连接请求后会转发到ood（ood上的zone-gateway配置包含tls协议栈）
- 通过rtcp访问zone-gateway后，再访问zone-gateway的http服务，是可靠的访问zone service的方法。该流程避免了对tls和传统CA的依赖
- ZoneGateway通过URL Router提供对Zone内所有服务的访问能力（实际上所有node-gateway共用同一套URL Router能力，只是非ZoneGateway通常只在`127.0.0.1`绑定，不对外提供服务）

## OODDescriptionString (OOD String)

实现见 `buckyos-base/src/name-lib/src/zone.rs`，结构如下：

```rust
pub struct OODDescriptionString {
    pub name: String,
    pub node_type: DeviceNodeType, // OOD | Gateway | OODOnly
    pub net_id: Option<String>,    // wan | wan_dyn | portmap | nat | lan1 | ...
    pub ip: Option<IpAddr>,
}
```

字符串文法：`[#|$]<name>[:<ip>][@<net_id>]`

| 前缀 | DeviceNodeType | 含义 |
| --- | --- | --- |
| 无前缀 | `OOD` | 既是OOD，也可承担Gateway职责（默认） |
| `#` | `Gateway` | 仅作Gateway，**不是OOD** |
| `$` | `OODOnly` | 仅作OOD，**不承担Gateway** |

净身位（net_id）规则：
- 显式指定时直接使用，例如 `@lan1`、`@wan`、`@wan_dyn`、`@portmap`、`@nat`
- 未指定但带IP时，自动设为 `wan`（即"有固定IP默认是WAN"）
- 未指定也无IP时为 `None`，等价于"未知LAN"

示例（与代码`tests`保持一致）：
- `ood1` —— OOD类型，net_id为None
- `ood1@lan1` —— OOD，处在lan1
- `ood1:192.168.1.8@lan` —— OOD，处在lan，IP为192.168.1.8
- `ood1:210.35.22.1` —— OOD，因有IP自动判定为wan
- `#gate1:210.35.22.1` —— 仅Gateway（非OOD），公网IP
- `#gate1@wan_dyn` —— 仅Gateway，动态公网
- `$ood2` —— 仅OOD（不会被选为ZoneGateway）

> 注意：旧文档中出现的 `wlan` / `unknown_lan` 等写法不在解析规则里，请勿使用。


### ZoneGateway Node的启动(非OOD）`TODO未实现`
1. ZoneGateway 有可能要做OOD之间通信的桥梁，因此rtcp stack中是先已zone-gateway逻辑启动，以支持OOD之间的中转连接
2. 任意OOD连接上来的时候，ZoneGateway也就完成了到OOD的连接任务
3. 如果系统里有多个ZoneGateway导致当前ZoneGateway没有OOD连接，则
    - 尝试与其他ZoneGateway连接，来访问SystemConfig
    - 实际上要走OOD的BOOT流程去尝试与OOD建立连接 （TODO：似乎没有必要）
 

### ZoneGateway与NodeGateway

核心区别：**ZoneGateway 有 tls stack，NodeGateway 没有**。两者共用同一套cyfs-gateway二进制，差别在配置（是否启用tls监听、是否绑定公开端口）。

```
浏览器 --https--> SN(无tls证书) --https--> ZoneGateway(有tls证书) --rtcp--> NodeGateway --upstream--> (App)Service 
浏览器 --https--> ZoneGateway(有tls证书) --rtcp--> NodeGateway --upstream--> (App)Service 
```

- NodeGateway的首要目标是运行node的rtcp stack
- 基于node rtcp stack，可以访问node上运行的各种传统tcp/udp服务
- 出于权限管理考虑，不少服务只允许绑定在`127.0.0.1`，因此只能通过rtcp(node_gateway)访问
- NodeGateway在`127.0.0.1:3180`提供http服务（默认端口`DEFAULT_NODE_GATEWAY_PORT = 3180`，定义见 [runtime.rs:41](src/kernel/buckyos-api/src/runtime.rs:41) ）。通过该端口可以以device的身份、通过rtcp协议访问Zone内的所有服务，这是ZoneGateway访问zone内服务的底层能力

## Zone内的Device之间建立连接

当系统启动后，Zone内的Device之间连接可以基于SystemConfig上保存的DeviceInfo，能做的选择更多

```
client --rtcp--> NodeGateway --upstream-->(App)Service
client --rtcp--> Zone中转节点(SN或gateway) --rtcp--> NodeGateway --upstream-->(App)Service
```

- 直连（优先）
通过DeviceInfo，可以明确的知道Device所在的局域网,尽量走直连

- 通过中转连接
如果不能直连，就要走中转。

考虑到与中转节点keep-tunnel可能会消耗中转节点宝贵的资源，下面是一种更复杂的中转模型（未实现)：
 
 `rtcp://$中转设备did/rtcp://目标LAN的GatewayNode/rtcp://目标Node名/`

- 目标Node处于目标LAN中
- 每个LAN中只有一个Node（通常是OOD）负责与中转节点保持连接，然后就可以通过上述rtcp url到达目标Node


## 与（另一个）Zone-Gateway建立连接
- 当同Zone的Device，在未连接OOD时尝试与ZoneGateway建立连接，也走该流程
- ZoneGateway支持http/https, 因此简单的使用 https://zoen_hostname/ 就能连接上正常工作的zone-gateway
- ZoneGateway必定支持rtcp
建立rtcp的标准流程 (`TODO:未完全正确实现`)
1. 通过zone-did查询得到可信的did-document,里面有gateway的device config jwt (包含rtcp port)
2. zone域名解析返回的IP 
3. 基于IP+rtcp port建立rtcp连接
对于“非完全端口映射环境”，可指定rtcp port可以与zone gateway建立直连 


### 与属于另一个Zone的Device建立连接 

> 注:因为安全原因，所有node-gateway的rtcp stack,默认只允许属于安全组的device连接(未实现)

有三种种方法:
- open_rtcp_tunnel("did:dev:$dev_pubkey") 适用于目标dev已经keep tunnel上来的逻辑，或则确定本地已有device document的情况此时可以直接复用
- open_rtcp_tunnel("did:bns:$devname.$zoneid") 最推荐的,第一次与属于任意zone的deivce建立连接的方法
- open_rtcp_tunnel("did:web:$devname.$zonehost") 与上一种方法相同，只是resolve_did逻辑实现不同

```rust
pub fn open_rtcp_tunnel(remote_did) {
  let device_doc = resolve_did(remote_did)
  if device_doc.ips.empty() {
    device_doc.ips.append(reslove(remote_did,"A"))
    device_doc.ips.append(reslove(remote_did,"AAAA"))
  }
  open_rtcp_tunnel_by_doc(device_doc)
}
```
resolve_did是cyfs://名字系统的关键函数，在其实现里，会根据did method的不同，选择不同的Provider. 目前系统支持2个provider

- bns|web 基于$zoneid，向Zone Provider查询(http协议)，zone必然已经启动
- bns 向BNS Global Provider查询，这是向一个智能合约进行查询标准，zone可以未启动

## 理解激活页面的Zone的接入方式

一共有8种连接路径 https://github.com/buckyos/BuckyOSApp/issues/16
下面是按“**A. net_id 四类 × B. 域名两类**”的写法，把你**第一段（无SN模式/有SN模式 + 各种家庭组合）**改成与第二段一致的描述方式；同时我把几个隐含前提补齐了（尤其是“无SN”的适用边界），并统一了一点术语（ACME / DDNS / keep tunnel）。

---

### A. 4 种访问路径（入口节点 OOD / ZoneGateway 的 net_id）

> 入口节点=对外提供访问与域名引导的节点（可以是 ZoneGateway，也可以是承担 ZoneGateway 职责的 OOD）。
>
> 是否`keep tunnel to SN`的判定见 [node_daemon.rs:864](src/kernel/node_daemon/src/node_daemon.rs:864) ：仅当配置了SN且`net_id`不以`wan`开头时才会`keep_tunnel`。`ddns_sn_url`只在`net_id`为`wan_dyn`或`portmap`时由激活流程自动写入（见 [active_server.rs:507](src/kernel/node_daemon/src/active_server.rs:507) ）。

#### A1. `nat`（最常见）

* 入口节点在 NAT 后，**无 443/2980 的公网可达能力**
* **必须依赖 SN 做中转/隧道（keep tunnel）**，否则外部无法稳定访问

#### A2. `portmap`

* 入口节点在 NAT 后，**可映射 2980 或其它指定端口，但无法映射 443**
* 需要 SN 的 **DDNS**（`ddns_sn_url`自动指向SN，当入口节点 IP/端口变化时刷新）
* HTTPS 流量无法直达入口节点：**需要 SN 转发/中转（HTTPS relay）**
* 入口节点通常仍需要与 SN **keep tunnel**（因为`net_id`不以`wan`开头）

#### A3. `wan_dyn`

* 入口节点具备公网可达能力，但 IP 不固定：

  * 能映射 443、2980（或公网动态 IPv6 可达）
* **不需要 SN 做中转**（代码中`net_id.starts_with("wan")`返回true，不keep_tunnel），但通常需要 **DDNS/引导服务**

  * 用 SN 二级域名 → SN 提供 DDNS（自动写入`ddns_sn_url`）
  * 用自有域名 → 用户可自建 DDNS，或仍用 SN 参与引导（见 B1）

#### A4. `wan`

* 入口节点具备公网固定可达能力（固定公网 IP / 稳定公网 IPv6）
* `net_id.starts_with("wan")`，因此**不keep_tunnel**
* 可做到 **完全不依赖 SN**（前提见 B1/WAN+自有域名）


### B. 2 种域名使用方式

#### B1. 用户自有域名

用户使用自己的域名（如 `example.com`）时，涉及 3 类记录/能力：

* A/AAAA：将域名（或子域）指向入口节点的公网地址
* TXT：配置 DID / PX0 / ZoneGatewayDeviceConfig（用于引导、校验、发现）
* 可选 NS：把某个子域（或整个域）NS 指向入口节点（`TODO:未实现`）

  * 可用于“对子设备的域名查询”（子设备域名解析由入口节点提供）
  * 可用于自动化证书（如果入口节点承担 ACME HTTP-01/DNS-01 逻辑）

**关键边界**

* **只有当 `net_id=wan`（或用户自建完整等价的 DDNS/证书/引导能力）时，自有域名才可能做到完全不需要 SN。**
* 当 `net_id != wan`（例如 `nat/portmap/wan_dyn`）时：

  * 你要么依赖 SN 提供 DDNS/引导/中转的某一部分
  * 要么用户自己额外部署等价服务（DDNS、证书签发、反向代理/中转等）

#### B2. 使用 SN 的二级域名

SN 提供一个二级域名（如 `xxx.buckyos.io`），并提供配套的引导能力：

* 自动注册/分配二级域名（自动在 BNS 上注册？）
* 域名解析：

  * 基于入口节点动态 IP 的 **DDNS**
  * 自动配置域名的 TXT：DID / PX0 / ZoneGatewayDeviceConfig
  * 支持对子设备的域名查询
  * 支持自动 ACME 获取 TLS 证书（SN 侧或配合入口节点）
* 转发/中转能力（按用户配置启用）：

  * HTTP 转发：把 http 流量转发到节点列表 A
  * HTTPS/端口受限场景转发：当 443 不可直达时，提供 https relay
  * 连接型转发：允许节点列表 B 设备与 SN keep tunnel，通过 SN 中转访问
  * device info 查询（`TODO:需要支持`）
  * rudp call/called（传统 P2P 打洞）

---

### C. 典型组合（4 × 2 的展开版）

> 每个组合回答：是否需要 SN（DNS/DDNS/证书/中转/隧道）+ 入口节点与 SN 的连接方式。

#### C1. `nat` + SN 二级域名（最常见家庭配置）

* SN：DDNS + TXT 自动配置 +（通常）证书 + **中转/keep tunnel**
* 入口节点：

  * ZoneBootConfig 设置 SN
  * 入口节点（OOD 或 ZoneGatewayNode）与 SN **keep tunnel**

#### C2. `nat` + 自有域名

* 由于入口节点不可公网直达，**仍需要 SN 的中转/keep tunnel**
* DNS 侧可由用户自有域名承担“命名”，但解析/引导仍要落到 SN 的可达方案上：

  * SN 配置自定义 hostname = 用户自有域名（等价“绑定域名”）
  * 其它同 C1（keep tunnel / 转发）


#### C3. `portmap` + SN 二级域名（只开放 2980 等端口）

* SN：DDNS + TXT 自动配置 + **HTTPS relay**（因为 443 不可达）+（通常）keep tunnel
* 入口节点：

  * DeviceConfig JWT 中声明可达端口（如 2980）（`TODO:需要支持`）
  * ZoneBootConfig 设置 SN
  * 为了支持 https 访问，通常仍需要 keep tunnel

#### C4. `portmap` + 自有域名

* SN 配置自定义 hostname = 用户自有域名
* 其它同 C3（核心仍是：443 不可达 → 需要 SN 的 https relay/隧道）

#### C5. `wan_dyn` + SN 二级域名（443/2980 映射或动态公网 IPv6）

* SN：主要提供 **DDNS + TXT 自动配置 +（可选）证书自动化**
* 入口节点：

  * ZoneBootConfig 设置 SN（用于 report device info/保持解析最新）
  * **通常不需要 keep tunnel**（可直连）

#### C6. `wan_dyn` + 自有域名

* 两种路径（二选一）：

  1. **用户自建 DDNS/证书/引导**：SN 可完全不参与
  2. 仍用 SN 做 DDNS/引导：SN 配置自定义 hostname = 自有域名
* 入口节点通常不需要 keep tunnel（能直连）

#### C7. `wan` + SN 二级域名（用户无域名但有固定公网）

* SN：可只做“二级域名解析/TXT/证书便利”，**不要求中转**
* 入口节点：

  * ZoneBootConfig 设置 SN（可选；report device info 成本低）
  * 入口节点直接以固定公网地址对外服务（无 keep tunnel）

#### C8. `wan` + 自有域名（唯一完全不需要 SN 的组合）

* 用户域名 A/AAAA 指向固定公网入口节点
* TXT 配置 DID / PX0 / ZoneGatewayDeviceConfig
* 可选 NS（`TODO:未实现`）：用于对子设备域名查询
* 证书：入口节点自建 ACME 或用户自管证书体系
* **SN 不参与 DNS / DDNS / 转发 / 隧道 / 中转**



## 关于故障的思考
### 简单环境，单OOD + 单ZoneGateway
- OOD掉线，系统不可写入，可通过ZoneGateway访问只读信息（通常是基于NDN发布的内容）
- ZoneGateway掉线， 系统不可被Zone外访问，同局域网的Zone内设备可以继续基于rtcp使用

### 高可用环境，3个OOD + 单ZoneGateway
- 任何一个OOD掉线，系统都会有问题，只有2个OOD挂了，系统才不可写入。可通过ZoneGateway访问只读信息（通常是基于NDN发布的内容）
- ZoneGateway掉线，如果3个OOD都在独立的LAN（多么SB的配置），会导致OOD通信失败，进而系统失效


## 一些实际问题
### 使用http/https访问zone gateway的逻辑，和zone boot config一点关系都没有
伪造tls证书(仿站)风险：
- 通过修改dns污染，篡改tls证书：无法实现，除非DNS污染能污染到证书颁发机构
- SN,虽然SN并不保存证书，但因为SN拥有域名解析，所以需要的时候SN总是可以约过用户申请新证书的
  - 该风险无解，根本上tls证书与域名的所有权(NS Server)绑定


## 下面是Device相关的定义，供参考
```rust
//DeviceMiniConfig(JWT) 只用于在DNS中拼接zoneconfig,其它时候不应该用
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct DeviceMiniConfig {
    #[serde(rename = "n")]
    pub name: String,
    pub x: String,
    //rtcp port
    #[serde(rename = "p")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtcp_port: Option<u32>,
    pub exp: u64,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,
}


#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct DeviceConfig {
    #[serde(rename = "@context", default = "default_context")]
    pub context: String,
    pub id: DID,
    #[serde(rename = "verificationMethod")]
    verification_method: Vec<VerificationMethodNode>,
    authentication: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    assertion_method: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    service: Vec<ServiceNode>,
    pub exp: u64,
    pub iat: u64,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,

    //--------------------------------
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_did: Option<DID>, // The zone did where the Device is located
    pub owner: DID,//owner did，原则上应该与zone的owner相同
    
    pub device_type: String, //[ood,server,sensor
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub device_mini_config_jwt:Option<String>,
    pub name: String,        //short name,like ood1
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtcp_port: Option<u32>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ips: Vec<IpAddr>, //main_ip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_id: Option<String>, // lan1 | wan, when None it represents lan0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ddns_sn_url: Option<String>,

    #[serde(skip_serializing_if = "is_true", default = "bool_default_true")]
    pub support_container: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub capbilities: HashMap<String, i64>,//capbility id -> resource value (like memory size, cpu core count, etc.)
}


//Device info一般是zone内给调度器使用
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct DeviceInfo {
    #[serde(flatten)]
    pub device_doc: DeviceConfig,
    pub arch: String,
    pub os: String, //linux,windows,apple
    pub update_time: u64,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub sys_hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_os_info: Option<String>,

    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub all_ip: Vec<IpAddr>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_info: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_num: Option<u32>, //cpu核心数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_mhz: Option<u32>, //cpu的最大性能,单位是MHZ
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_ratio: Option<f32>, //cpu的性能比率
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage: Option<f32>, //类似top里的load,0 -- core

    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_mem: Option<u64>, //单位是bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_usage: Option<u64>, //单位是bytes

    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_space: Option<u64>, //单位是bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk_usage: Option<u64>, //单位是bytes

    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_info: Option<String>, //gpu信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_tflops: Option<f32>, //gpu的算力,单位是TFLOPS
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_total_mem: Option<u64>, //gpu总内存,单位是bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_used_mem: Option<u64>, //gpu已用内存,单位是bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_load: Option<f32>, //gpu负载

}
```

    
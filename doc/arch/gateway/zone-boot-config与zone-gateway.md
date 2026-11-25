## Zone Boot Config的设计

因为DNS的TXT Record长度限制，BuckyOS专门设计了3条TXT Record来共同构成可在Zone外可信引导Zone所需的最小信息，整体由name-lib的`ZoneBootConfig`承载：
- `BOOT=$ZONE_BOOT_CONFIG_JWT` —— ZoneBootConfig主体（owner私钥签名的JWT）
- `PKX=$OWNER_PUBLIC_KEY.X` —— Owner公钥（Ed25519的x分量），仅用于自验证BOOT签名
- `DEV=$GATEWAY_DEVICE_MINI_DOC_JWT` —— ZoneGateway/OOD的`DeviceMiniConfig`（owner签名的JWT），主要携带rtcp端口等boot阶段必须的字段

这3条TXT由SN按BNS上的文档应答，**用户不需要在自己的DNS后台写BOOT / DEV**。自有域名的Zone首次启动使用激活时写到本机的 Zone 文档（Last Known Good State），不依赖BOOT TXT，见 [Booting 阶段 resolve-did-document 的特殊处理说明](../BuckyOS%20Booting阶段%20resolve-did-document的特殊处理说明.md) §14.1。用户自己写的归属证明是另一条记录：`_pkx.<域名>` TXT，值为 `PKX(<owner 公钥 x>)`，与这里的 `PKX=` 不同。

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
- ZoneConfig 是 Zone 启动后写入 system-config 的内部配置，包含可公开的 `zone_document` 以及内部字段
- Zone 间通信一般只应依赖公开的 ZoneDocument / DID Document

99%的情况下，都不应该直接使用 ZoneBootConfig，而是使用基于 DNS TXT Record 转换得到的 ZoneDocument；内部服务读取 `boot/config` 时先解析 ZoneConfig，再解包其中的 `zone_document`。

## ZoneBootConfig的 首要目标：确保系统能安全引导 (Secure Boot)
OOD的安全启动引导流程如下（实现位置：`src/kernel/node_daemon/src/zone_boot_resolve.rs` 的 `boot_resolve_zone_document`）：

1. OOD先读本机的Last Known Good State（激活时写入，`$BUCKYOS_ROOT/etc/<zone host>.zone_lkgs.json`），再发现Zone Document：先走`resolve_did`（自有域名时是`https://<域名>/.well-known/`），失败后查DNS TXT得到ZoneBootConfig(JWT格式)。都失败时用Last Known Good State继续启动
   - 调试/离线场景：若`{zone_name}.zone.json`存在，会被优先加载，绕过网络发现（见`load_debug_zone_document_override`）
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
    zone_document: &str,
) -> Result<&mut Self> {
    let mut zone_config = ZoneConfig::new(zone_document.to_string());
    zone_config.verify_hub_info = Some(VerifyHubInfo {
        public_key: verify_hub_public_key.clone(),
    });
    self.insert_json("boot/config", &zone_config)?;
    Ok(self)
}
```
注：`zone_document` 可解包为 `ZoneDocument` / DID Document，内部 `verify_hub_info` 不再放入公开文档。

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

激活时有三个互相独立的选择：

| 选择 | 可选值 | 决定什么 |
| --- | --- | --- |
| 接入方式（`net_id`） | `nat` / `portmap` / `wan_dyn` / `wan` | 流量怎么进来，见A |
| 域名和DNS | 默认域名；自有域名的SN托管 / 手工 / DNS API | 谁写DNS记录，见B |
| 证书来源 | 系统自动签发（`acme`）/ 用户上传（`upload`） | 要不要ACME，见B1.3 |

自有域名的托管信息是 `DomainHosting`（`src/kernel/buckyos-api/src/domain_hosting.rs`），激活时写入 `start_config.json` 的 `domain_hosting`，首次启动后存在 system-config 的 `system/domain_hosting`。默认域名没有这一项。

### 对SN的依赖分三件事

“走不走SN中转”“SN应不应答地址”“SN应不应答其它记录”是三件独立的事，不能用一个“需不需要SN”的布尔值表示：

| 事情 | 由什么决定 | 规则 |
| --- | --- | --- |
| 流量经SN中转 | 只看接入方式 | `nat`全部；`portmap`只有HTTP(S)，RTCP直连；`wan_dyn`平时直连，保留后备隧道；`wan`不用 |
| SN应答地址（A / AAAA） | 名字是否由SN解析：默认域名、SN托管，或记录CNAME到`<用户名>.web3.<SN>` | `nat`、`portmap`应答relay；`wan_dyn`应答设备上报的公网IP；`wan`应答签名文档里的固定IP |
| SN应答其它记录（证书挑战等TXT） | 域名的NS是否在SN：默认域名和SN托管 | 手工和DNS API不用SN的这一项 |

因此`wan`也可以用SN托管：SN应答固定IP和证书挑战，但不中转。走激活页面的用户都有SN账号，`ZoneDocument.sn`、`BootDocument.sn`和`DeviceDocument.ddns_sn_url`一律写入，它们不代表“要中转”。完全不用SN的Zone用`src/active.ts`命令行激活，见 `product/node_active/Node_Active_无SN命令行激活.md`。

---

### A. 4 种接入方式（入口节点 OOD / ZoneGateway 的 net_id）

> 入口节点=对外提供访问与域名引导的节点（可以是 ZoneGateway，也可以是承担 ZoneGateway 职责的 OOD）。
>
> 是否`keep tunnel to SN`只看`net_id`，`wan`以外都连，见 `src/kernel/node_daemon/src/sn_zone_info.rs` 的 `device_requires_sn_keep_tunnel`。地址和`self_cert`上报对所有接入方式都做。

| net_id | 入口节点的可达性 | SN中转 | 激活发布的文档 | SN应答的地址 |
| --- | --- | --- | --- | --- |
| `nat` | 没有公网入口 | 全部流量 | — | relay |
| `portmap` | 能映射RTCP端口（默认2980），不能映射443 | HTTP(S)；RTCP直连 | — | relay。RTCP直连地址在完整设备文档里 |
| `wan_dyn` | 443、2980公网可达，IP会变 | 平时不用，隧道作后备 | mini设备文档写`net_id = wan_dyn` | 设备上报的公网IP |
| `wan` | 443、2980公网可达，IP固定 | 不用，不连relay | `DeviceDocument.ips`和`ZoneDocument`的`gateway_ips`写用户确认的公网IP | 签名文档里的固定IP |

`portmap`如果应答设备地址，浏览器会连到没有映射的443，所以应答relay。IP会变的用户应选`wan_dyn`，不要选`wan`。

### B. 域名和DNS

#### B1. 用户自有域名

用户使用自己的域名（如`home.example.com`），按DNS由谁管理分三种。三种都能与四种接入方式组合。

| 托管方式 | 用户要做什么 | 系统做什么 | 证书 | 根域名 |
| --- | --- | --- | --- | --- |
| SN托管 | 写两次：先`_pkx` TXT，再把NS委派到`dns.<SN>` | SN应答地址和证书挑战 | 自动签发（默认）或上传 | 不支持。委派根域名会丢掉MX等记录 |
| 手工 | 自己写3条记录，自己更换证书 | 检测记录 | 只能上传 | `wan`可用；其它接入方式要子域名 |
| DNS API | 给一个DNS服务商的token | 写记录、写证书挑战、续期、修复缺失的记录 | 自动签发（默认）或上传 | `wan`可用；其它接入方式要子域名，DNS在Cloudflare的除外 |

服务商名单、申请token的步骤和权限建议见 [doc/user/dns-providers](../../user/dns-providers/README.md)。

##### B1.1 记录

手工和DNS API用同一份记录计划（`src/kernel/node_daemon/src/domain/plan.rs`），区别只是由用户写还是由节点写：

| 记录 | `nat` / `wan_dyn` / `portmap` | `wan` |
| --- | --- | --- |
| `_pkx.<域名>` TXT | `PKX(<owner 公钥 x>)` | 同左 |
| `<域名>` | CNAME `<用户名>.web3.<SN>` | A / AAAA 公网IP |
| `*.<域名>` | CNAME `<用户名>.web3.<SN>` | A / AAAA 公网IP |

SN托管只有`_pkx` TXT和NS `dns.<SN>`。

* relay按绑定关系路由，不要求NS委派到SN。CNAME到`<用户名>.web3.<SN>`后，动态IP和relay换IP都由SN处理，节点不用自己做DDNS。
* 不写BOOT / DEV TXT。`https://ood1.<域名>/.well-known/did.json`由Zone在boot之后自己提供，不用另外发布。
* 比通配记录更具体的旧记录（例如单独的`ood1`）会盖住通配记录，要删掉。
* Cloudflare上的记录要用“仅DNS”，不能开代理。

##### B1.2 归属与检测

* 归属：`_pkx`由节点按BNS上的OwnerDocument算出，在权威NS上看到之后再由SN `domain.bind`确认；`user.get_profile`的`user_domain`已经是这个域名就不再调。
* SN托管的顺序是先TXT后NS，都在激活之前完成。只看NS会让别人抢先绑定一个已经委派过来的域名。
* 激活前检测归属、NS、入口记录（含`ood1`、`sys`、`public`和一个随机子域）、证书挑战、CAA，全部通过才能激活；`commit_active`再检查一遍。规则见 `notepads/node-active-custom-domain-todo.md` §3.3.1。
* 检测直接问权威NS，节点要能访问外网的53端口（UDP，截断时TCP）。出口拦了53端口的网络里检测通不过。
* 激活后节点启动3分钟时检测一次，之后每6小时一次；有问题时每1小时一次。DNS API会顺带补回缺失的记录。
* token被服务商拒绝时通知owner；自动签发的证书续期失败持续30分钟才通知，到期前14天内仍失败再通知一次。上传的证书不通知。

##### B1.3 证书

* Zone需要两个名字：`<域名>`和`*.<域名>`。
* 自动签发：SN托管用`sn-dns`（DNS-01，挑战写到SN），DNS API用`dns-api`（挑战写到服务商）。生成规则看`domain_hosting`，不看接入方式，所以`wan`也有ACME。
* ACME目录默认Let's Encrypt生产环境。激活页配置`active_config.json`可以加可选的`acme_issuer`，激活时写进`domain_hosting.acme_issuer`，DV用staging时设它。
* 上传：检查后装到身份路径，`server.meta.json`的`renewal.manager`为`manual`，系统不续期、不提醒、不替换。
* `nat`、`portmap`的HTTPS由relay转发，前提是SN上的`self_cert`为true。它由网关按TLS栈实际加载到的证书上报（`self_cert_report`），与证书来源无关。

#### B2. 使用 SN 的二级域名（默认域名）

`<用户名>.web3.<SN>`，没有`domain_hosting`：

* 注册账号时分配，地址、TXT（BOOT / DEV / PKX）由SN按BNS文档和上报应答
* 证书用`sn-dns`自动签发
* relay转发HTTP / HTTPS / RTCP，按接入方式决定用不用

---

### C. 典型组合

每格写：DNS地址由谁应答 / 证书从哪来 / 流量怎么走。

| 接入方式 | 默认域名 | 自有域名 + SN托管 | 自有域名 + 手工 | 自有域名 + DNS API |
| --- | --- | --- | --- | --- |
| `nat` | SN应答relay / `sn-dns` / 全部经relay | SN应答relay / `sn-dns`或上传 / 全部经relay | CNAME到SN，应答relay / 上传 / 全部经relay | CNAME到SN，应答relay / `dns-api`或上传 / 全部经relay |
| `portmap` | 同`nat`，RTCP直连 | 同`nat`，RTCP直连 | 同`nat`，RTCP直连 | 同`nat`，RTCP直连 |
| `wan_dyn` | SN应答上报的IP / `sn-dns` / 直连 | SN应答上报的IP / `sn-dns`或上传 / 直连 | CNAME到SN，应答上报的IP / 上传 / 直连 | CNAME到SN，应答上报的IP / `dns-api`或上传 / 直连 |
| `wan` | SN应答固定IP / `sn-dns` / 直连 | SN应答固定IP / `sn-dns`或上传 / 直连 | A / AAAA固定IP / 上传 / 直连 | A / AAAA固定IP / `dns-api`或上传 / 直连 |

* `wan_dyn`保留到relay的后备隧道；`wan`不连relay。
* `wan` + 手工或DNS API时，SN不参与地址应答、证书和流量，只保留账号、归属确认、BNS发布和上报。
* 完全不用SN只有`wan` + 自有域名 + `src/active.ts`这一种，DNS和证书全部自己管。



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

    

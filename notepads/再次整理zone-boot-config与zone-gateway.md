## Zone Boot Config的设计

```rust
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ZoneBootConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<DID>,
    pub oods: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sn: Option<String>,
    pub exp: u64,
    pub iat: u32,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,
    //-------下面的字段，都不会序列化，而是分头保存在TXT Record里------------
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_key: Option<Jwk>, //PKX=0:xxxxxxx;
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub gateway_devs: Vec<DID>,

}

```
## ZoneBootConfig的 首要目标：确保系统能安全引导 (Secure Boot)
OOD的安全启动引导流程如下：
1. OOD通过外部服务查询得到ZoneBootConfig(JWT格式)
    - 外部服务目前有DNS和BNS
    - 原理上，任何支持w3c did查询的服务器，都可以缓存Zone-Boot-Config
2. 对ZoneBootConfig进行验证：
    - 比已知的ZoneBootConfig更新
    - 有ZoneOwner的签名,（可用did_document.auth_key 验证）
3. 验证ZoneOwner的签名需要可信的通过OwnerDID得到其公钥 目前的方法
    - OOD在激活的时候，已经在本地保存了Owner的公钥 （实际上是OwnerConfig,复用DID-Document存储的底层设施）
    - 通过BNS查询Owner的公钥（最权威）, 给了Owner更换公钥的机会（如果之前的私钥丢失的化）
    - 通过DNS的TXT记录(PX0)查询Owner公钥。实际上通过DNS查询ZoneBootConfig时，查询器已经基于PX0对ZoneBootConfig进行了自验证
4. ZoneBootConfig中只有1个OOD，则进行单点启动引导（目前的实现）
5. ZoneBootConfig中有2n+1个OOD，OOD需要和n个其它OOD在boot阶段建立连接。基于BootConfig中的SN信息和OOD的"Net String",尝试与其他OOD在Boot阶段建立连接。有配置SN会增加一种连接方法
6. Boot阶段，OOD会努力与其它OOD保存rtcp tunnel(实际上一直会)，当与n个ood keep tunnel成功后，才会进入system config的启动阶段，。
7. OOD之间建立连接的方法
7.1 基于Devcie's Name尝试直连（Zone内设备的直连几乎都是这个流程）
- 尝试得到目标OOD的一个IP地址 
    - UDP广播
    - NameString里包含IP地址
    - 通过DNS查询：zone_hostname 或 `推导设备子域名(TODO:要设计确定规则)`
    - 向SN查询(如果配置了SN)DeviceInfo (这一项也许是和上一项的合并)
- 与该IP地址的2980端口通信
- 与对方进行DevcieConfig交换 （`TODO：这一步有缺失`）
- 两端均基于自己的OwnerConfig里的公钥，对DeviceConfig进行验证，包括验证name是OOD
7.2 基于Device'Name 尝试通过中转建立连接 (此时无法keep tunnel)
当无法直接建立连接时，会视是否有中转节点来尝试通过中转建立rtcp stream,下面是尝试open stream
`rtcp://$中转设备did/rtcp://目标设备名/` (这种中转，中转设备是能解码rtcp上的内容，这可能会导致隐私泄露)
- boot阶段，可用的中转设备是 `bootconfig中标记为 SN 和 标记为WLAN的OOD，以及标记为ZoneGateway的Node`
    - 和中转设备建立rtcp tunnel的逻辑与7.1步骤相同，OOD一旦与中转节点连接成功，就会keep tunnel
    - SN可以基于自己的逻辑，阻止Device注册，或则阻止某些中转行为DeviceA 打开到 DeviceB的 stream
- 直接连接成功的OOD节点，也可能是WLAN节点（虽然bootconfig里没写），可以用来做中转节点
8. SystemConfig服务首次启动时，会根据ZoneBootConfig构造正式的ZoneConfig，下面是核心逻辑

```rust
pub fn init_by_boot_config(&mut self, boot_config: &ZoneBootConfig) {
    self.id = boot_config.id.clone().unwrap();
    self.oods = boot_config.oods.clone();
    self.zone_gateway = boot_config.oods.clone();
    self.sn = boot_config.sn.clone();
    self.exp = boot_config.exp;
    self.iat = boot_config.iat as u64;

    if boot_config.owner.is_some() {
        self.owner = Some(boot_config.owner.clone().unwrap());
    }
    if boot_config.owner_key.is_some() {
        self.verification_method[0].public_key = boot_config.owner_key.clone().unwrap();
    }
    self.extra_info.extend(boot_config.extra_info.clone());
}
```

下面是Device相关的定义，供参考
```rust
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
    pub device_type: String, //[ood,server,sensor
    pub name: String,        //short name,like ood1

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<IpAddr>, //main_ip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_id: Option<String>, // lan1 | wan ，为None时表示为 lan0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ddns_sn_url: Option<String>,
    #[serde(skip_serializing_if = "is_true", default = "bool_default_true")]
    pub support_container: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_did: Option<DID>, //Device 所在的zone did
    pub iss: String,
}


// describe a device runtime info
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

### 非OOD(Node/Client)的启动流程(与ZoneBootConfig无关) `未实现`
1. Node启动的时候，系统已经启动完成。因此Node在启动时的核心目标是连接上SystemConfig Service。
2. Node可以基于OOD搜索流程，主动尝试连接OOD (可以避免ZoneGateway失效导致的内网不可用)，尝试流程
3. Node通过ZoneGateway 可以直接访问SystemConfig Service(优先rtcp)
4. 通过SystemConfig Service返回的OOD DeviceInfo,可以使用最佳的方法与OOD建立RTCP连接（尽量直连）,提高后续访问的速度

## ZoneGateway的定义
- ZoneGateway通常是OOD，但可以是普通Node，系统里可以有多个ZoneGateway
- 在一个典型的小型系统中，使用单OOD，随后可以添加一个最便宜的VPS Node做ZoneGateway
- ZoneGateway持有zone hostname的tls证书,会启动tls stack(可选，但一般都有)
    - 在有SN的情况下，SN收到tls连接请求，会转发到的Node就是ZoneGateway 
    - 在无SN的情况下，DNS解析的结果，指向ZoneGateway
- 通过rtcp访问zonegateway后，再访问zone gateway的http服务，也是可靠的访问zone service的方法
- ZoneGateway通过URL rouer,提供了对Zone内所有服务的访问能力（每台机器上的cyfs-gateway都有这个能力，但通常不对外提供服务）

### ZoneGateway Node的启动(非OOD）`TODO未实现`
1. ZoneGateway 有可能要做OOD之间通信的桥梁，因此rtcp stack中是先已zone-gateway逻辑启动，以支持OOD之间的中转连接
2. 任意OOD连接上来的时候，ZoneGateway也就完成了到OOD的连接任务
3. 如果系统里有多个ZoneGateway导致当前ZoneGateway没有OOD连接，则
    - 尝试与其他ZoneGateway连接，来访问SystemConfig
    - 实际上要走OOD的BOOT流程去尝试与OOD建立连接 （TODO：似乎没有必要）
 
### ZoneGateway 的确定
- 在ZoneBootConfig中手工指定(目前的PX1=>应该变成额外的DeviceConfig JWT)
- 通过调度器自动构造`TODO 未实现`

## Zone内的Device之间建立连接
当系统启动后，Zone内的Device之间连接可以基于SystemConfig上保存的DeviceInfo，能做的选择更多

- 直连（优先）
通过DeviceInfo，可以明确的知道Device所在的局域网

- 通过中转连接
DeviceInfo中，说明了通过那个中转节点可以连通 Device。
考虑到与中转节点keep-tunnel可能会消耗中转节点宝贵的资源，下面是一种更复杂的中转模型：（目标Node处于目标LAN中）
 `rtcp://$中转设备did/rtcp://目标LAN的GatewayNode/rtcp://目标Node名/`
每个LAN中只有一个Node（通常是OOD）负责与特定的中转节点保持连接，然后就可以通过上述rtcp url到达目标Node


## 与（另一个）Zone-Gateway建立连接
- 当同Zone的Device，在未连接OOD时尝试与ZoneGateway建立连接，也走该流程
- ZoneGateway支持http/https, 因此简单的使用 https://zoen_hostname/ 就能连接上正常工作的zone-gateway
- ZoneGateway也必须支持rtcp (可以不依赖https zone-gateay的存在)
建立rtcp的标准流程 (`TODO:未完全正确实现`)
1. 通过zone-did查询得到可信的did-document,里面有gateway的device config jwt (包含rtcp port)
2. zone域名解析返回的IP (`多个怎么办?`)
3. 基于IP+rtcp port建立rtcp连接
对于“非完全端口映射环境”，可指定rtcp port可以与zone gateway建立直连 

### 为什么有PX1
- 当前实现的问题在于：PX1没有数字签名，可能因为DNS污染导致rtcp连接上fake节点！`TODO:将px1改成device info jwt`
- SN在开发时，并没有提供http接口给外部查询zone-gateway的device config,当时只能把gateway的公钥写在DNS中.不过这也让使用rtcp访问zone变成一种基础能力，而不是必须依赖https

### 与另一个Zone的Device建立连接
Device的Global DID确定

- did:dev:$device_public_key (可以转化为hostname)
- did:web:$device_name.dev.$zone_hostname /  $device_name.dev.$zone_hostname
- did:bns:$device_name.$zone_hostname

rtcp://$device_did/

从安全角度考虑，这不是一个推荐的行为，所以目前的所有流量，一定会通过ZoneGateway转发
从实现上，这是一个标准的两部流程
1. resolve device did-doc
2. 得到必要的ip信息后，尝试进行连接
- 直连
- 中转连接（基于device'owner zone-gateway或SN中转)


## ZoneBootConfig与ZoneConfig
- 通过ZoneBootConfig可以构造符合W3C标准的ZoneConfig
- ZoneBootConfig只在 OOD Boot和 连接另一个Zone的Gateway时必须用到
- Node在启动时要连接当前ZoneGateway，也有可能用到ZoneBootConfig

### `ZoneConfig有啥用？`
- 现在的ZoneConfig由调度器构造（没有签名），获取的时候已经连接上了SystemConfig,应该是ZoneInfo(`TODO 需要修改`)
- 没有255字节限制，可以用来保存Zone内的各种各样的全局信息，由调度器统一刷新
- 各个应用都可以基于ZoneConfig里的信息，调整自己的逻辑（尤其是有重要的基础服务的状态）

## Zone 的接入方式

### 无SN模式
- 有至少一个WLAN的ZoneGateway (可以是OOD)
- 用户有自己的域名
    - 将域名解析配置到ZoneGateway
    - 配置域名的TXT Record，设置DID,PX0,ZoneGatewayDeviceConfig
- 用户将自己域名的NS记录，指向ZoneGateway （可选?,`TODO:未实现`）
    - 可以支持自动ACMA获得TLS证书
    - 可以支持对子设备的域名查询

### 有SN模式
SN提供的基本能力
- 免费的二级域名注册（自动在BNS上注册?)
- 支持域名解析
    基于ZoneGateway 动态IP的D-DNS
    自动配置域名的TXT Record，设置DID,PX0,ZoneGatewayDeviceConfig
    支持对Zone子设备的域名查询
    支持自动ACMA获得TLS证书
- 支持http转发,根据用户配置,将http流量转发到指定的节点列表A上
- 支持rtcp转发，根据用户配置，允许节点列表B中的设备与sn keep tunnel。用户可以通过SN中转，连接节点列表B中的设备
- 支持device info查询？（`TODO:需要支持`）
- 支持rudp call/called(传统的P2P打洞)
用户可以根据配置，组合的启用SN的上述能力

#### NAT 后OOD + 二级域名
最常见的家庭配置
- SN 配置当前用户的自定义hostname为none (启用二级域名)
- ZoneBootConfig设置SN,OOD会与SN Keep tunnel
- ZoneGatewayNode会与SN keep tunnel(一般不会有NAT后的ZoneGateway)

#### NAT 后OOD + 自定义域名
- SN 配置当前用户的自定义hostname为自定义域名
其它同上

#### NAT 后OOD 只开放2980端口映射 + 二级域名
- 定义了端口映射的OOD或ZoneGatway，会在DeviceConfig JWT中说明自己的rtcp端口（`TODO:需要支持`）
其它同 ` NAT 后OOD + 二级域名` （为了支持https访问，还是要keep tunnel的）

#### NAT 后OOD 只开放2980端口映射 + 自定义域名
- SN 配置当前用户的自定义hostname为自定义域名
其它同上

#### NAT 后OOD 完全端口映射 + 二级域名
完全端口映射下，有一个OOD可以看成是“动态IP的WAN Node“，该OOD不会与SN keep tunnel,但是会定期report device info
- ZoneBootConfig中，OOD配置ood@wlan
- SN 配置当前用户的自定义hostname为none (启用二级域名)
- ZoneBootConfig设置SN

#### NAT 后OOD 完全端口映射 + 自定义域名
- SN 配置当前用户的自定义hostname为定义域名
其它同`NAT 后OOD 完全端口映射 + 二级域名`

#### WAN OOD/ZoneGateway + 二级域名
用户买了有固定IP的VPS，但没有买域名

- SN 配置当前用户的自定义hostname为none (启用二级域名)
- ZoneBootConfig中设置SN （定期report device info的成本很低，但不需要keep tunnel)
- ZoneBootConfig中设置OOD为OOD:ipv4 / zonegateway:ipv4

## OOD String 
- ood1 相当于 ood1@unknown_lan
- ood1@wlan ood1是处在waln的非固定IP设备 
- ood1:210.34.120.23 ood1是有固定IP的WLAN设备
- ood1@lan1 ood1是处在lan1的设备
- #ood1 该节点是zone-gateway节点？
- #node1 该节点是非ood zone-gateawy节点？

## 关于故障的思考
### 简单环境，单OOD + 单ZoneGateway
- OOD掉线，系统不可写入，可通过ZoneGateway访问只读信息（通常是基于NDN发布的内容）
- ZoneGateway掉线， 系统不可被Zone外访问，同局域网的Zone内设备可以继续基于rtcp使用

### 高可用环境，3个OOD + 单ZoneGateway
- 任何一个OOD掉线，系统都会有问题，只有2个OOD挂了，系统才不可写入。可通过ZoneGateway访问只读信息（通常是基于NDN发布的内容）
- ZoneGateway掉线，如果3个OOD都在独立的LAN（多么SB的配置），会导致OOD通信失败，进而系统失效



## 思考的一些实际问题
### 使用http/https访问zone gateway的逻辑，和zone boot config一点关系都没有
通过修改dns污染，篡改tls证书,（如何应对？）就是可以制造访站

### 目前给定hostname,建立rtcp的流程
- 需要ip
- 需要得到ip对应的device exchange key(px1)
- 通过hostname支持返回多个 device,由rtcp协议栈负责选择？

标准流程：
1. 通过Hostname -> zoee_did -> zone_did's owner （域名的Owner的通用过查询方法）`放入zone boot config最简单`
2. zone_did's owner -> owner's public key
DNS污染可以替换到另一个zone boot config上去？通过引入BNS ,可以防止篡改（交易域名的时候也需要更换BNS配置）
尽快变成都通过BNS合约来获取zone boot config,可以保证不被域名注册商篡改 --> 使用Rtcp访问zone更安全

3. zone_did -> gateway_devices config(多个)
目前计划采用px1的方案，但放入device config的jwt,用owner's public key对gateway_device config进行验证
bns方案则可以通过更完整的zone boot config直接得到。只需要验证zone boot config即可

使用DNS方案，通过预装的trust hostname ower方案，可以防止对一些关键服务的zone boot config进行“篡改”
通过向这些服务器发起bns查询，可以进一步提高对所有zone-did的保护







    
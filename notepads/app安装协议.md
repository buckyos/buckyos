# buckyos app安装协议
buckyos app安装协议的设计目的，是提供统一的方法，让第三方网页可以引导用户安装app到其buckyos上。核心流程有如下几种

## buckyos的app安装流程

### 点击安装

- 可以在任意第三方网页的任意位置，放置“安装 $APP_NAME"的按钮，点击后触发在buckyos上安装app的流程 
- 通过js检测用户当前环境是否安装了buckyos-service(有安装的目的地)
- 如果未安装，则跳到安装引导页（官方引导页是https://www.buckyos.ai/desktop/install.html
- 如果已经安装，则跳转到 cyfs://current_zone/kapi/control_panel?method=install_app&url=$APP_META_JSON_URL&$install_param1=xxxx&install_param2=xxxx
该链接会拉起本地desktop的native UI,并根据$APP_META_JSON_URL的内容展现app信息，进入安装到buckyso的流程

### 分享安装

已经安装app的用户也可以分享app给其好友，引导安装。分享方式有2大类4种

- 分享链接: 链接形式是 https://$USER_ZONE_HOST/share/share_app.html?id=$APP_OBJ_ID
该链接实际指向用户OOD的一个系统内置页面，页面展示了待分享app的必要信息。集成使用了`点击安装`的逻辑。
该链接的可访问性依赖用户OOD的可访问性
- 分享链接二维码: 以二维码形式分享上述链接
- 分享app文本:文本内容是一个包含了APP_META_JSON信息的TEXT,用户手工打开buckyos desktop客户端后，在‘添加app’页面粘贴该TEXT即可进入app安装流程
app文本的可用性依赖于buckyos的官方app source服务（如果app被source收录）或 app 开发者的ood的可用性。与分析app的用户的OOD可用性无关。
- 分析app文本二维码：用二维码编码了上述APP_META_JSON,用户用buckyos mobile版本客户端扫码后，可以进入app安装流程

### 内置应用商店安装 （还未开始实现）

- 已经运行的buckyos会自带一个默认的应用商店，该应用商店默认只有登录用户才能访问。
- 该应用商店可以用浏览器打开，也可以用buckyos desktop打开。在浏览器里打开时，安装流程不依赖 buckyos-desktop
- 应用商店的内容来自 `用户自管理APP_META_JSON + 应用源(source)list`, 应用商店在展示的时候，提供整合的去重展示。
- 用户可以通过url添加应用源。
- 所有通过`分享安装信息`得到可信APP_META_JSON,只要触发过安装，不管后续有没有安装成，都可以在`用户自管理APP_META_JSON`里看到
- 因为app安装协议是稳定的，因此用户可以很容易的安装第三方的应用商店。

## App安装过程中的下载
- 下载APP_META_JSON（或直接得到），这个由传播渠道确定，一般是来自分享源和应用源
- 解析APP_META_JSON后，进行实体下载，通常按下面顺序进行(标准cyfs://支持多源)
  - 主要目的是减少爆红App的作者的流量压力
  - 从docker.io等公共docker源下载
  - 从APP_META_JSON中配置的可验证下载源下载(常见的是git releases)
  - 从app源服务器下载（多源支持)
  - 从分享源下载(多源支持)
  - 从app的作者OOD下载（多源支持)
  
## app的信任机制

当触发app的安装流程后，系统弹出的第一个界面通常要展示app的关键信息。除了app的图标、名字、介绍外，最重要的信息就是安全信息。虽然App运行在安全沙盒中，但我们还是有基础的信任机制，帮助用户远离恶意应用。
buckyos的app信任机制主要由下面机制构成
- app的作者的信任级别. 除了常见的可信发行者机制，buckyos也有原生的联系人组机制，通过社交网络也可以提高作者的信任级别
- app如果被应用源收录（app可以被多个源收录），可以得到源对app的信任评级。
- 不同的应用源有不同的信任评级。
- app的来源的信任级别：比如是好友分享的app_meta，或则是某个第三方网站的推荐。注意第三方网站推荐app和第三方网站是一个源并给app信任评级是不同的

### 用户对app的信任进行调整的方法
- 通过系统面板，手工添加和调整app的作者，并设置不同的信任等级
- 通过系统面板，手工添加和调整应用源，并设置不同的信任等级
- 通过系统面板，手工添加 分享来源 的信任等级。注意分享来源的信任登记的上限有限制

## app的经济模型（还未开始实现）

`用户安装app成功`是经济模型的利益原点, buckyos通过简单透明的规则，将这个利益分配给`App作者、应用源、分享来源`。
`用户安装app成功`后，buckyos会自动把安装成功证明发送给App作者、应用源、分享来源。

安装证明
```json
{
    "action":"installed",
    "objid":"xxxx",//what
    "userid":"xxx",//who
    "device_id":"xxxx",//where

    "iat" : "",//创建证明的事件
    "exp" : "",//无意义
    "details" : {
        "referrer" : "",
        "curator" : "",
    }
}
```

购买的细节
1. 通过合约购买，注意购买一般购买的是一个系列（比如版本1.0，版本2.0），这里需要公共的让用户知道自己买了什么
2. 
购买证明：如果用传统网关支付完成，那么owner/curator需要开具购买证明。(有签名)
```json
{
    "action":"puared",
    "objid":"xxxx,"
}
```


### 获得BDT(BuckyOS DAO Token)奖励

利益相关方把`用户安装App成功`这个行为(批量)提交给BDT DAO合约后，将会根据合约规则得到BDT奖励。
BDT DAO根据BuckyOS的成熟度，逐步释放BDT，因为BDT总量有限，生态奖励的BDT的特点是
- 早期一个安装的奖励多，后期一个安装的奖励少
- 早期大应用奖励多，后期长尾应用的奖励相对固定

获得奖励的两种流程
```
用户支付完成
用户基于支付完成的收据，去向 作者/源 下载（向源下载的时候，需要在支付收据里有正确的源信息）

```


### 传统付费App

传统付费App在安装时触发应用源的统一支付网关，并引导用户使用传统支付手段支付。    
应用源按自己的分成比例，将收到的费用结算给App作者和分享来源。

### USDB付费App

如果App作者是标准BuckyOS作者（作者拥有自己的发布OOD),那么就可以一键打开USDB付款支持。
使用USDB付款，是通过BDT的标准App支付合约完成，该合约会按比例（比例由作者在一定范围内可调）将收入打给`App作者、应用源、分享来源`。
标准App支付合约会自动创建App安装证明，并触发BDT的奖励合约。

### 通过http 402支持其它付费模式

App作者也可以通过http 402协议，扩展自己的专属付费模式。

### App作者的确权

- 在生态早期，很多App是被兼容移植上来的，并不是真正的作者在BuckyOS上进行发布。这个时候，App作者不是标准作者，而是兼容作者。
- 兼容作者没有自己的官方OOD，相关行为必须依赖其应用源。所以兼容作者由应用源负责管理。
- 应用源必须公开自己的“兼容作者领取方案”，当兼容作者创建为标准作者后，应用源应该通过标准的BNS名字转移机制，将App的相关权益和收益，转给真正的作者。
- 对于无主作者，应用源应把作者设置为公益作者（作者为BDT DAO），分数到App作者的收益会直接进入BDT DAO，本质上属于整个生态。


### 一些思考
- 按`作者、源、分享来源` 的思路，可以把经济循环扩展到Video\Game\Music等 广泛的数字产品 的经济循环上。BuckyOS希望通过新的去中心基础设施+经济模型，建立新的"Internet Content Library"
- 只有买断，没有订阅。从"服务自有"的角度出发，BuckyOS的基础设施天然喜欢买断制，反对依赖中心化服务的订阅制。

收录行为由源签名
```json
{
    "name":"$appname",
    "objid":"app_pkg_id",
    "userid":"收录者账号",
    "action":"listed",
    "iat":"收录时间",
    "exp":"有效期",
    "directory":"dir_id",//很重要，收录到哪个列表里去了？
    "score" : "", //收录时的评分
    "details" : {
        //附加的介绍
        //被收录到的目录对象id
    }
}
```
在有收录列表id的情况下，也可以用用dirid + path 的模式来说明“这是一个我收录的对象”
dirObject也是一个content object,可以有签名(JWT)


分享行为,通常是设备的签名。极限的分数，需要用户签名
```json
{
    "objid":"app_pkg_id",
    "userid":"分享者账号",    
    "device_id":"did:dev:xxx",
    "action":"recommand",
    "iat":"xxx",
    "exp":"xxx",
    "score":"xxx",
    "details" : {

    }
}
```


### 经济循环里的一些负面行为
- 购买前先看用户的推荐（去中心的），并由AI汇总成分数
- 支付的时机：使用先支付再下载，但支付成功后下载失败怎么办？
    - 如何解决问题？求助其它用户？求助其它源？
    - 公开该行为，目的是导致 作者 / 源 的信用降低
- 评分体系如何防刷？(这个课题相当的大，应该单独拿出一章来说，和放BDT的领取本质上一类问题)
- 版权保护：buckyos的官方发行版本里有校验是否购买的逻辑，用户修改该逻辑，或分发去掉该验证逻辑的版本，都可能触犯版权法
- 

## APP_META_JSON的设计
```json
{
  "@schema": "buckyos.app.meta.v1",

    "pkg_name": "buckyos-filebrowser",
    "version": "2.27.0",
    "meta": {    
         "show_name": "File Browser",
        "icon_url": "https://example.com/icon.png",
        "homepage_url": "https://example.com",
        "support_url": "https://example.com/support","en": "A web-based file manager.", "zh": "一个基于 Web 的文件管理器。","license": "Apache-2.0" 
    },
    "pub_time": 1760000000,
    "exp": 0,
    "deps": {},
    "tag": ["file", "web", "nas"],
    "category": "app",

    "author": "Filebrowser Team",
    "owner": "did:bucky:authorxxxx",
    "curators" ["did:bns:curator1","did:web:gitpot.ai"],

    //付费应用填写，这个比较精细（站在对用户公开的角度来支持合适的细节），支持多种版本购买
    "economics": {
        "version" : "*", //购买的是所有版本， ^1.0 只购买1.0版本
        "revenue_split": { "author": 0.8, "source": 0.15, "referrer": 0.05 },
        "payment": { "usdb": {
            "prices" : "1.99",
            "contract" : "付款合约地址",// usdb有默认的付费合约地址，这里不应设置
        } }
    },

// install主要是列出app希望申请的资源
  "install": {
    "selector_type": "single",
    "install_config_tips": {
      "data_mount_point": ["/data"],
      "local_cache_mount_point": [],
      "service_ports": { "www": 80 },
      "container_param": null,
      "custom_config": {}
    },
    "services": [
      {
        "name": "www",
        "protocol": "tcp",
        "container_port": 80,
        "expose": {
          "mode": "gateway_http",
          "default_subdomain": "file",
          "default_path_prefix": "/",
          "tls": "optional"
        }
      }
    ],
    "mounts": [
      { "kind": "data", "container_path": "/data", "persistence": "keep_on_uninstall" },
      { "kind": "config", "container_path": "/config", "persistence": "delete_on_uninstall" },
      { "kind": "cache", "container_path": "/cache", "persistence": "delete_on_uninstall" }
    ],
    "network": { "bind_default": "127.0.0.1", "allow_bind_public": true }
  },
  "permissions": {
    "fs": {
      "sandbox": true,
      "home": {
        "private": { "read": false, "write": false },
        "public": { "read": true, "write": true },
        "shared": { "read": true, "write": true }
      }
    },
    "system": { "need_privileged": false, "devices": [], "capabilities": [] }
  }
}

```
是一个ContentObject(有 owner/收录者/传播者) 三要素，可以是DirObject?

0. app类型，目前支持2种:docker / static_web(比如只依赖钱包的智能合约前端页面)
1. docker 信息：, 最重要，符合docker hub规范，在一个url里同时支持amd64/arm64双架构，默认只支持amd64


2. app提供了哪些服务？ 最简单的就是http服务，其它服务因为不能通过gateway router,而是基于端口router的，因此在系统中有一定的独占性 
    WWW服务 -> 内部端口， 配置到哪个子域名？
    其它服务 -> 外部端口，内部端口
3. app有哪些数据目录需要mount,给这些目录属性。
    data -> 最重要，用户数据，卸载了也不会删除。data有“app沙盒” + 访问用户home (需要设置权限) 
      用户的home可以细分成 home(隐私数据) public （公开数据) shared (授权公开数据) 3大类，可以分别给于权限
    config -> app自己的配置，升级不会覆盖，但是卸载后会删除
    cache -> app的cache
4. app是否需要一些其它的能力（特殊的docker参数）

5. app的最小资源需求（最小CPU需求，最小内存需求，是否需要GPU，是否需要本地大语言模型目前不能设置）
6. app的一些展示信息 (标准meta)

7. app的作者信息
8. app的权益信息（是否需要付费）

### 三段格式
第一段 由App作者编写的，严格不可变，由App作者签名的AppDoc。按cyfs:// named-object的规范，可以得到AppObjId. 每次版本升级，该AppObjId都会改变
第二段 由App源编写的，有App源收录的签名（内容包含上一个AppDoc的JWT）。被收录是一个通用格式，可以嵌入与App源有关的一些附加信息（比如信用评级）
第三段 AppSpec, 基于AppDoc中的安装提示，用户最后确定了安装参数后，可以得到一个确定的用于安装的AppSpec
- 在buckyos内，app_full_id = app_name@username,指向一个AppSpec. 虽然从系统的定义角度，允许不同用户安装app的不同版本,单从目前复杂度管理的角度，系统限制一个app_name只能选择激活一个版本
- AppDoc（由app_name指定) + InstallParams = AppSpec 。注意Install Params目前安装后不能修改。（需要卸载后重装）


### App的唯一性
app的逻辑did如下did:bns:$app_name.$zoneid#$version_tag，该did一般指向一个确定版本的AppDoc（有确定的AppObjId)，可以通过访问该链接来确定这个版本是否是有效版本
app_name的命名规范是 `$作者名_友好名称`,用来防止冲突。app在安装后，默认的短域名名称是  $app_friendly_name-$username.$zone_hostname ｜  $app_friendly_name.$zone_hostname
buckyos并没有机制，来保障一个app_name是全网唯一的，但大部分应用源，会在收录app的时候会反向基于app_name中的作者名去构造`原始APP链接`，并进行验证。因此，在一个应用源里,app_name是唯一的。
AppDoc中包含签发时的owner公钥，因此，AppObjId，必然是全网唯一的，需要用一个全网唯一的标识来引用一个确定的AppDoc版本时，应该选择AppObjId
app_name也是pkg_id:系统在加载Pkgid时，有时会根据情况，添加平台信息变成pkg_full_id: aarch64-linux-nightly.pkg_id。 

### App的升级
App升级会带来App的版本变化，当使用did:bns:$app_name.$zoneid去查询AppDoc,如果AppDoc与本地的版本不同，则可以触发升级
根据系统的升级策略，用户需要手工确认后执行升级。对于有新的install params的升级，必须展现UI。升级流程基本相当于覆盖安装，因为下载检验的需要，用户基本上要完整下载新的安装包。


## 应用评论（Review，现在不实现）
- 用户在应用源上发表了针对某个ObjId的评论。该评论被保存在用户和应用源的OOD上，并有用户和应用源的相关签名。
- 独立的评论列表：收集针对特定DID（历史上该DID可能指向多个ObjId)的，来做多个OOD的评论，基于评论Id去重，本地AI过滤后，形成的列表。
- 上述列表，称作"Zone内筛选“列表，该列表可以使用传统网页公开访问。


## 相关参考

### 原安装协议
```json
{
    "app_id": "test_app",
    "app_name": "Test App",
    "version": "0.1.0",
    "author": "Test Author",
    "description": "This is a test app",
    "docker_image": "image name",
    "data_mount_point": {
        "/srv/": "home/"
    },
    "tcp_ports": {
        "www": 80,
    }
}
```


### BuckyOS内的关键数据结构
```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageMeta {
    pub pkg_name: String,
    pub version: String,
    pub description: Value,
    pub pub_time: u64,
    #[serde(default)]
    pub exp:u64,
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub deps: HashMap<String, String>,     //key = pkg_name,value = version_req_str,like ">1.0.0-alpha"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>, //pkg的分类,app,pkg,agent等
    pub author: String,
    pub owner:DID,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_id: Option<String>, //有些pkg不需要下载
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_url: Option<String>, //发布时的URL,可以不写
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_size: Option<u64>, //有些pkg不需要下载

    #[serde(flatten)]
    pub extra_info: HashMap<String, Value>,

}

pub struct AppDoc {
    #[serde(flatten)]    
    pub meta: PackageMeta,
    pub show_name: String, // just for display, app_id is meta.pkg_name (like "buckyos-filebrowser")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_icon_url: Option<String>,
    pub selector_type:SelectorType,
    pub install_config_tips:ServiceInstallConfigTips,
    pub pkg_list: SubPkgList,
}

#[derive(Serialize, Deserialize,Clone)]
pub struct ServiceInstallConfigTips {
    pub data_mount_point: Vec<String>,
    pub local_cache_mount_point: Vec<String>,


    //通过tcp_ports和udp_ports,可以知道该Service实现了哪些服务
    //系统允许多个不同的app实现同一个服务，但有不同的“路由方法”
    //比如 如果系统里app1 有配置 {"smb":445},app2有配置 {"smb":445}，此时系统选择使用app2作为smb服务提供者，则最终按如下流程完成访问
    //   client->zone_gateway:445 --rtcp-> node_gateway:rtcp_stack -> docker_port 127:0.0.1:2190(调度器随机分配给app2) -> app2:445
    //                                                                docker_port 127.0.0.1:2189 -> app1:445
    //   此时基于app1.service_info可以通过 node_gateway:2189访问到app1的smb服务
    //service_name(like,http , smb, dns, etc...) -> real port
    pub service_ports: HashMap<String,u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_param:Option<String>,
    #[serde(flatten)]
    pub custom_config:HashMap<String,serde_json::Value>,
} 

#[derive(Serialize, Deserialize,Clone)]
pub struct ServiceInstallConfig {
    //mount pint
    // folder in docker -> real folder in host
    pub data_mount_point: HashMap<String,String>,
    pub cache_mount_point: Vec<String>,
    pub local_cache_mount_point: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind_address: Option<String>,//为None绑定到127.0.0.1，只能通过rtcp转发访问
    //network resource, name:docker_inner_port
    #[serde(default)]
    pub service_ports: HashMap<String,u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_param:Option<String>,

    pub res_pool_id:String,
}


#[derive(Serialize, Deserialize,Clone)]
pub struct AppServiceSpec {
    pub app_doc: AppDoc,
    pub app_index: u16, //app index in user's app list
    pub user_id: String,

    //与调度器相关的关键参数
    pub enable: bool,
    pub expected_instance_count: u32,//期望的instance数量
    pub state: ServiceState,

    //App的active统计数据，应该使用另一个数据保存
    // pub install_time: u64,//安装时间
    // pub last_start_time: u64,//最后一次启动时间

    pub install_config:ServiceInstallConfig,
}

```
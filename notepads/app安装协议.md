

# BuckyOS App 安装协议规范 (Draft)

## 1. 协议概述

本协议旨在提供一套统一、去中心化的标准，允许（第三方）网页引导用户在 BuckyOS 上安装 App。协议涵盖了安装引导、多源下载、信任校验以及基于区块链的经济激励模型。

**术语定义：**

* **OOD (Owner Online Device):** 个人 AI 服务器 (Personal AI Server)，用户的核心计算节点。
* **Curator:** 应用收录源/收录人。
* **Referrer:** 推荐人/分享者。

---

## 2. App 安装交互流程

### 2.1 点击安装 (Web to Native)

此流程支持在任意第三方网页触发安装。

* **入口**：网页放置 `安装 $APP_NAME` 按钮。
* **交互逻辑**：
1. 按钮指向统一的 HTTPS 中间页（Gateway Page）。
2. **环境检测**：中间页尝试检测用户环境。
* *<TODO: 补充具体的 JS 检测方案，例如通过尝试唤起 `cyfs://` 协议并监听 `blur` 事件或超时机制，以应对现代浏览器的隐私限制。>*


3. **分支处理**：
* **未安装 BuckyOS**：跳转至官方引导页 (`https://www.buckyos.ai/desktop/install.html`)。
* **已安装 BuckyOS**：通过 URL Scheme 唤起本地 Desktop：
```text
cyfs://sys.current_zone/app/installer?method=install_app&url=$APP_META_JSON_URL&ref=$REFERRER_ID

```




4. **本地 UI**：Desktop 拉起 Native UI，解析 `$APP_META_JSON_URL` 展示 App 信息（图标、权限、评分），进入安装确认页。



### 2.2 分享安装 (Social Distribution)

利用社交网络进行裂变式传播。

1. **链接分享 (HTTPS)**:
* 格式：`https://sys.$USER_ZONE_HOST/share/share_app.html?id=$APP_OBJ_ID`
* 机制：指向用户 OOD 的托管页面，集成上述“点击安装”逻辑。
* *依赖：用户 OOD 在线且可公网访问。*


2. **二维码分享**:
* 内容：上述链接的 QR Code 编码。


3. **纯文本/短码分享**:
* 内容：包含 `APP_META_JSON` 关键信息的压缩文本或 JSON 字符串。
* 操作：用户打开 BuckyOS Desktop -> “添加 App” -> 粘贴文本 -> 解析并安装。
* *优势：不依赖分享者的 OOD 在线状态，依赖官方 Source 或 P2P 网络。*


4. **对象投递 (Inbox Push)**:
* 机制：基于 CYFS Content Network 的 `ActionObject` 投递。不依赖微信/Telegram 等传统信道。
* **对象定义 (Recommend Action)**:
```json
{
    "objid": "app_pkg_id",
    "userid": "did:bucky:sender_id",
    "device_id": "did:dev:xxx",
    "action": "recommend", // 修正了拼写 recommand
    "iat": 1769990599,
    "exp": 1801094599,
    "score": 5, // 推荐指数
    "details": {
        "comment": "这个App很好用，推荐给你"
    }
}

```


* 体验：目标用户在“消息/分享给我的”列表中看到卡片，一键安装。



### 2.3 内置应用商店安装

* **访问方式**：BuckyOS Desktop 内置或浏览器访问（需登录）。
* **数据聚合**：内容 = `用户自管理 APP_META_JSON` + `订阅的应用源 (Source List)`。
* **去重策略**：应用商店前端负责对多源的同一 App 进行聚合去重展示。
* **记录同步**：凡是触发过“安装”行为的 App（无论成功与否），都会记录在用户的自管理 Meta 列表中，方便后续找回或重试。

---

## 3. 分发与下载机制 (Distribution)

### 3.1 App Meta 解析与下载

* **第一步**：获取并解析 `APP_META_JSON`。
* **第二步**：实体下载。为减轻单点压力，采用 **多源回退 (Fallback)** 策略：
1. **公共 Docker 源** (如 docker.io)。
2. **可验证下载源** (Meta 中配置的 URL，如 GitHub Releases)。
3. **App 源服务器** (Source OOD)。
4. **分享源** (P2P，从分享者的 OOD 下载)。
5. **App 作者 OOD**。



### 3.2 完整性校验

* **版本锁定**：`APP_META_JSON` 必须包含特定版本的哈希校验值 (Digest/Checksum)。
* **校验流程**：无论从哪个源下载，系统在通过校验前不会执行安装。
* *<TODO: 明确 Meta JSON 中存储 Hash 的字段标准，例如 `digest: "sha256:xxxx"`。>*



---

## 4. 信任与安全机制 (Trust)

### 4.1 信任分级体系

1. **作者信任 (Author Trust)**:
* 基于 DID 的可信发行者认证。
* **社交信任**：通过“联系人组”机制，如果作者是用户的好友（或好友的好友），信任度提升。
* **第三方信用机构**：BuckyOS 支持接入多个信用查询 Oracle。


2. **收录源信任 (Curator Trust)**:
* 若 App 被高信誉的 Source (如 GitPot) 收录，继承该 Source 的背书。
* `AppMetaJson` 本身如果是被签名的收录证明，包含 `rank (0-100)` 评分。


3. **来源信任 (Referrer Trust)**:
* 区分 **Referrer** (谁推荐给我的) 和 **Curator** (谁收录了这个 App)。好友推荐的 App 会有更高的 UI 提示优先级。



### 4.2 用户干预

* 用户可在系统面板手动调整对特定 **Author**、**Source** 或 **Referrer** 的信任等级（白名单/黑名单）。

---

## 5. 经济模型 (Economics)

### 5.1 利益原点：安装成功证明

`用户安装 App 成功` 是生态产生价值的核心事件。系统自动生成证明并分发给利益相关方。

**安装证明 (JWT)**:

```json
{
    "action": "installed",
    "objid": "app_pkg_id_ver_1.0",
    "userid": "did:bucky:user_id",
    "device_id": "did:dev:device_id",
    "iat": 1769990599, // 安装时间
    "exp": 1801094599, 
    "details": {
        "referrer": "did:bucky:referrer_id", // 谁分享的
        "curator": "did:web:gitpot.ai"       // 哪个源收录的
    }
}

```

### 5.2 购买与支付

1. **购买对象**：通常购买 App 的特定版本或系列 (Version Range)。
2. **购买证明 (Receipt)**:
```json
{
    "action": "purchased", // 修正了拼写 puared
    "objid": "app_pkg_id",
    "buyer": "did:bucky:user_id",
    "tx_hash": "0x......"
}

```



#### 支付模式

* **传统付费**：通过应用源的网关支付（法币/信用卡）。源负责结算给作者。
* **USDB 付费 (Web3 Native)**：
* 前提：作者拥有标准 OOD。
* 流程：调用 BDT 标准支付合约。
* 分账：合约自动按比例 (`revenue_split`) 实时分账给 `Author`、`Source`、`Referrer`。


* **HTTP 402**：支持作者自定义付费网关。
* *<TODO: 补充 BuckyOS 对 HTTP 402 响应的标准处理流程 UI。>*



### 5.3 BDT (BuckyOS DAO Token) 激励

**机制**：将“安装证明”提交给 BDT DAO 合约以“挖矿”。
**释放曲线**：

* **时间衰减**：早期安装奖励高，后期降低。
* **长尾效应**：早期大应用奖励多，后期长尾应用保持固定基础奖励。

### 5.4 风险管理与确权

* **兼容作者 (Ported Apps)**：
* 早期由 Source 托管（作为兼容作者）。
* **权益转移**：Source 必须公开“认领协议”。当真实作者出现并验证 DID 后，通过 BNS 转移机制移交控制权和收益。
* **无主应用**：收益流入 BDT DAO 公共池。


* **负面行为防范**：
* **防刷量**：*<TODO: 需要引入抗女巫攻击机制 (Anti-Sybil)，例如要求安装证明必须来自“活跃度”达标的 OOD，或结合 PoW/Staking 门槛。>*
* **支付原子性**：解决“支付成功但下载失败”的问题（建议：资金托管模式，下载验证后释放资金，或通过其他 Source 补救下载）。



---

## 6. 核心数据结构定义

### 6.1 收录证明 (Inclusion Proof)

由 Curator 签名，证明该 App 已被收录。

```rust
// Rust Definition
pub const OBJ_TYPE_INCLUSION_PROOF: &str = "cyinc";

#[derive(Serialize, Deserialize, Clone)]
pub struct InclusionProof {
    pub content_id: String,      // 内容 ObjId
    pub content_obj: serde_json::Value, // 内容摘要
    pub curator: DID,            // 收录者 DID
    pub editor: Vec<String>,     // 具体编辑/操作员
    pub meta: Option<serde_json::Value>, 
    pub rank: i64,               // 评分 1-100
    #[serde(default)]
    pub collection: Vec<String>, // 收录集合/目录分类
    pub review_url: Option<String>,
    pub iat: u64,
    pub exp: u64,
}

```

### 6.2 APP_META_JSON (v1)

这是 App 分发的核心元数据。

```json
{
    "@schema": "buckyos.app.meta.v1",
    "did": "did:bns:filebrowser.buckyos",
    "name": "buckyos_filebrowser",
    "version": "2.27.0",
    "meta": {    
        "show_name": "File Browser",
        "icon_url": "https://example.com/icon.png",
        "homepage_url": "https://example.com",
        "support_url": "https://example.com/support",
        "description": {
            "en": "A web-based file manager.", 
            "zh": "一个基于 Web 的文件管理器。"
        },
        "license": "Apache-2.0" 
    },
    "pub_time": 1760000000,
    "exp": 0,
    "deps": {
        "buckyos_kernel": ">1.0.0"
    },
    "tags": ["file", "web", "nas"],
    "category": "app",

    "author": "Filebrowser Team",
    "owner": "did:bucky:authorxxxx",
    "curators": ["did:bns:curator1", "did:web:gitpot.ai"], // 修正了语法错误

    // 经济模型定义
    "economics": {
        "version": "*", // 购买授权范围
        "revenue_split": { 
            "author": 0.8, 
            "source": 0.15, 
            "referrer": 0.05 
        },
        "payment": { 
            "usdb": {
                "prices": "1.99",
                "contract": "default_payment_contract_address" 
            } 
        }
    },

    // 资源申请与安装配置
    "install": {
        "selector_type": "single",
        "services": [
            {
                "name": "www",
                "protocol": "tcp",
                "container_port": 80,
                "expose": {
                    "mode": "gateway_http",
                    "default_subdomain": "file"
                }
            }
        ],
        "mounts": [
            { "kind": "data", "container_path": "/data", "persistence": "keep_on_uninstall" },
            { "kind": "config", "container_path": "/config", "persistence": "delete_on_uninstall" }
        ],
        "network": { "bind_default": "127.0.0.1", "allow_bind_public": true }
    },
    
    // 权限声明
    "permissions": {
        "fs": {
            "sandbox": true,
            "home": {
                "private": { "read": false, "write": false },
                "public": { "read": true, "write": true }
            }
        },
        "system": { "need_privileged": false }
    }
}

```

### 6.3 App 类型变体

* **Static Web App**: `pkg_list` 仅包含 web 资源，无后端容器。
* **Agent**: 核心资产是 Prompt 和模型配置，`pkg_list` 可能为空或指向模型权重。

---

## 7. 命名与升级 (Naming & Lifecycle)

### 7.1 DID 与 唯一性

* **逻辑 DID**: `did:bns:$app_name.$zoneid#$version_tag`
* **App ObjId**: 全网唯一，基于内容寻址或 Owner 签名。
* **命名冲突**: BuckyOS 不强制全局唯一名称，但 Source 内部要求唯一。建议命名规范 `$author_$appname`。

### 7.2 升级流程

1. 客户端定期查询 `did:bns` 解析最新的 `AppDoc`。
2. 对比版本号。
3. **UI 触发**:
* 若涉及新的权限或配置参数 (`install params`) 变更 -> **强制弹窗确认**。
* 若仅代码更新 -> 可静默或弱提示升级。



---

## 8. 未来规划 (Roadmap)

* **评论系统 (Review)**: 去中心化评论上链，结合 AI 进行垃圾信息过滤和情感汇总。
* **版权保护 (DRM)**: 增强官方 Runtime 的校验逻辑，防止单纯的去校验版本分发。
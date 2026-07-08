# node_active 激活流程现状说明（草稿）

## 0. 文档说明

**这不是一份增量修订的 PRD，是一份从当前代码反推出来的"现状快照"。**

唯一相关的旧文档 [`product/archive/PRD(zh_CN)/1 Active BuckyOS.md`](../archive/PRD(zh_CN)/1%20Active%20BuckyOS.md) 设计于约两年前，描述的是硬件套装购买→BuckyOS Control App→Web2.5 SSO 账号体系→pubkey 免费 Zone 名的流程，与现状（浏览器向导 + 钱包/网页双路线 + SN-Auth 两阶段凭证 + BNS/EVM 写入）完全是两套设计，本文档不基于它增量修改，仅在个别处做历史对比脚注。

方法论：本文档所有结论均来自直接阅读代码得出，主要覆盖：
- `src/kernel/node_active/`（激活向导前端源码，React + Vite + TS）
- `src/kernel/node_daemon/src/active_server.rs`（`/kapi/active` RPC 服务端）
- `src/kernel/node_daemon/src/node_daemon.rs`（激活触发与激活后运行期逻辑）
- 姊妹仓库 `cyfs-gateway`（`/Users/liuzhicong/project/cyfs-gateway`）中 SN、BNS 的实际接口实现
- `notepads/sn-api-node-active-todo.md`、`notepads/sn-auth-dev-active-script.md` 两份工程决策记录

本文档**只描述"现在是什么"**，不包含目标/路线图/验收标准。如果要把它升级成正式 PRD（补目标、非目标、用户故事、验收标准），建议作为下一步单独做，不要和"对齐现状"这一步混在一起——上一份文档就是混在一起后来对不上号的。

标注 **[未接入UI]** 的能力表示后端/协议已实现但向导没有暴露对应入口，不代表规格缺失，只是现状描述。

---

## 1. 系统组成与触发方式

`node_active` 是内嵌在 `node_daemon` 二进制里的一个运行模式，不是独立进程。`node_daemon` 每次启动都会尝试加载 `<etc_dir>/node_identity.json`；加载失败时：

- 如果启动带了 `--enable_active` 参数 → 启动激活 HTTP/RPC 服务（`start_node_active_service`，端口 **3182**），静态托管激活向导页面（`/`）并提供 `/kapi/active` RPC；
- 如果没带 `--enable_active` → 只打印警告后退出，不会自动进入激活模式（这是启动方——安装器/systemd unit/desktop 包装层——的配置决定，`node_active` 自己不控制）。

前端源码目录 `src/kernel/node_active/`（`active_lib.ts` 核心逻辑 + `src/` 下的 React 组件）是**唯一源码**；`src/rootfs/bin/node-active/` 经 `diff -rq` 核对与前者的 `dist/` 构建产物**逐字节相同**，只是打包进 rootfs 镜像的产物目录，不是第二份实现，后续维护不要在这个目录改代码。

激活成功后，服务端写完身份文件会在 2 秒后**自杀退出进程**（`do_active`/`do_active_by_wallet` 两个 handler 各自起了一个 `sleep(2s); exit(0)` 任务），依赖外部 supervisor 拉起 `node_daemon` 正常态——`node_active` 的生命周期在浏览器这一侧到 SuccessStep 的倒计时页面为止，不存在进程内热切换。

---

## 2. 两条激活路线：探测方式与"钱包"的真实含义

路线选择在 `App.tsx` 初始化时**自动探测**，不是用户在 UI 上选的：

```
buckyos.getRuntimeType() !== AppRuntime  → 网页路线
buckyos.getRuntimeType() === AppRuntime 且 getCurrentWalletUser() 返回空 → 网页路线（兜底）
两者都满足 → 钱包路线
```

**"钱包"不是 MetaMask / WalletConnect。** 它是宿主原生壳注入到页面里的一个自定义 JS 桥对象 `window.BuckyApi`（`getCurrentUser()` / `signJsonWithActiveDid(payloads)` / `openExternalUrl(url)`），通过 vendored 的 `buckyos-websdk` 包间接调用。这个桥对象的**注入方（原生宿主）不在 buckyos 仓库里**——`node_active` 只是桥的消费者，谁在什么场景下把 `window.BuckyApi` 挂到页面上，是仓库外部的事情，写 PRD 时不要假设这是浏览器插件模式。

---

## 3. 激活向导分步详解

步骤顺序按路线分叉（`ActiveWizard.tsx`）：

- **网页路线（7 步）**：Security → Gateway → Domain → AIProvider → JarvisMsgTunnel → Review → Success
- **钱包路线（6 步）**：Gateway → Domain → AIProvider → JarvisMsgTunnel → Review → Success（**整个 Security 步骤被跳过**，账号信息从钱包预置）

| 步骤 | 适用路线 | 做什么 | 关键校验/分支 |
|---|---|---|---|
| **Security** | 仅网页 | 创建/登录 SN(BNS) 账号 + 设置本地管理员密码 | 用户名去抖检测可用性；≥7 位时去抖检查邀请/激活码；点 Next 时**自动判断注册还是登录**（用户名可用→注册，否则→登录），不是两个入口 |
| **Gateway** | 都需要 | 选网络拓扑：`BuckyForward`（SN 中转）/ `PortForward`（`full` 或 `rtcp_only` 子模式）/ `WAN`（固定公网 IP） | `rtcp_only` 模式下校验 RTCP 端口 1-65535 |
| **Domain** | 都需要 | 选 `{sn_username}.web3.buckyos.ai` 子域，或自有域名 | 自有域名走正则校验；非 WAN 网关下会提示 NS 委派信息 |
| **AIProvider** | 都需要 | 可选填 OpenAI / Claude / Google / GLM 的 API Token；若 SN 激活码已带 `llm_router` 权益，展示"已包含"的只读横幅 | 全部留空且无激活码权益时，主按钮变"跳过"并二次确认 |
| **JarvisMsgTunnel** | 都需要 | 可选填 Telegram Bot Token + Account ID（当前仅此一个通道） | 两个字段必须成对填写，只填一个会挡住提交 |
| **Review** | 都需要 | 最终确认页；`WAN`+自有域名时需先生成 DNS TXT 记录才能继续；网页路线在此**明文展示 owner 私钥**供用户自行备份（钱包路线不展示，私钥从不经过这个页面）；点击"Activate!"调用 `do_active` 或 `do_active_by_wallet` | — |
| **Success** | 都需要 | 120 秒倒计时页，提示设备正在完成重启，可复制新 Zone 链接 | — |

---

## 4. 身份与密钥模型

### 4.1 设备密钥（两条路线一致）

两条路线都会调用同一个 RPC `generate_key_pair`（服务端 `generate_ed25519_key_pair()`）在设备本地生成一对 Ed25519 密钥，作为设备身份密钥。这一点两条路线没有差异。

### 4.2 Owner 密钥（两条路线的核心差异，含一处需要注意的现状缺口）

| | 网页路线 | 钱包路线 |
|---|---|---|
| Owner 公私钥来源 | **同样调用 `generate_key_pair()` RPC** 在浏览器会话中临时生成 | 从 `getCurrentWalletUser().public_key` 读取，**从不生成**，`owner_private_key` 全程为 `null` |
| Owner 签名方式 | 私钥留在浏览器 state，最终随 `do_active` 请求体一起发给服务端，由服务端签发 zone/device JWT | 通过 `walletSignWithActiveDid(payloads)` 请求钱包签名，服务端只收到已签好的 JWT，从不接触私钥 |
| 管理员密码哈希 | 用户在 SecurityStep 输入，客户端 `hashPassword()` 后传服务端 | 要求钱包签名响应里带 `pwd_hash`；钱包没返回就直接抛错 `"missing password hash"` |
| 持久化 | 私钥只留在浏览器 React state，关闭标签页即丢失；服务端**不落盘 owner 私钥**，只落盘设备私钥/身份文件 | 私钥从未进入浏览器或服务端 |

**⚠️ 需要在正式 PRD 里明确决策的缺口**：`notepads/sn-api-node-active-todo.md` 里确认的设计意图是"网页路线用同一个助记词通过 NameLib 派生出 EVM 密钥和 Owner 密钥"。但经代码核实（`active_lib.ts:99-107`），**当前网页路线的 Owner 密钥就是一次裸的 `generate_key_pair()` RPC 调用，代码里找不到任何助记词生成/BIP39 派生逻辑**。也就是说：
- 设计意图（助记词同时派生 EVM+Owner 两把钥匙）和当前实现（只生成一把 Owner 用的 Ed25519 密钥，看不到 EVM 密钥生成）之间存在落差；
- 这与下面 §6.4 提到的"BNS EVM 写入在向导里完全没有入口"是同一个缺口的两面——网页路线目前事实上**没有**在向导里生成或获取 `bns_evm_private_key`，BNS 发布这条腿对网页路线用户来说当前不可达。

### 4.3 Owner DID / SN 账号体系

Owner DID 固定形如 `did:bns:{sn_username}`——即 Owner 身份直接等价于一个 SN(BNS) 账号，不是独立的密钥体系。

---

## 5. Zone 创建与网络拓扑

当前只有**创建**流程，代码里搜不到任何"导入已有 Zone / 恢复备份"的路径——这点上比两年前旧文档设想的更简单，彻底放弃了 pubkey 免费 Zone 名等设计。

- **Zone 名称**：默认 `{sn_username}.web3.buckyos.ai`；或自有域名（`DomainStep`）。
- **网关类型 → `net_id` 映射**（`get_net_id_by_gateway_type`）：

  | Gateway 选择 | `net_id` | SN 的角色 |
  |---|---|---|
  | `BuckyForward` | `nat` | SN 中转浏览器全部流量 |
  | `PortForward` + `full` | `wan_dyn` | SN 仅作动态公网 IP 的 DDNS 解析，不转发流量 |
  | `PortForward` + `rtcp_only` | `portmap` | SN 中转流量，仅 RTCP 控制通道走直连 |
  | `WAN` | `wan` | 完全不需要 SN 中转 |

- **是否需要 SN**（`is_need_sn()`）：只有 `WAN` + 自有域名这一种组合会完全跳过 SN（`sn_url:null`），其余组合都要求 `sn_access_token`。
- **单 OOD 限制**：`oods` 字段硬编码为 `["ood1"]`，当前向导**只支持单 OOD 激活**，没有多设备组 Zone 的引导流程（这与旧文档"3 台以上组集群"的产品设想不符，现状是先做单机）。
- **两种让 Zone 文档可解析的落地方式**：
  1. 默认路径：服务端自动通过 SN/BNS 发布（见 §6.4）；
  2. 仅 `WAN` + 自有域名时的手工路径：向导生成 `BOOT`/`DEV`/`PKX` 三段 JWT 文本，用户自行粘贴进自己的 DNS 服务商后台，粘贴完成前 Activate 按钮不可点。

---

## 6. SN / BNS 集成现状

SN、BNS 的实际接口定义在姊妹仓库 `cyfs-gateway`（`src/components/cyfs-sn/`、`src/components/bns-client/` 等），不在 buckyos 仓库里；下面是对 `node_active`/`node_daemon` 实际调用面的现状梳理，完整协议以 `cyfs-gateway/doc/SN/SN-API.md`（当前与代码一致，可信）为准。

### 6.1 三个 kapi 端点职责边界

| 端点 | 谁调用 | 职责 | 认证 |
|---|---|---|---|
| `/kapi/sn/auth` | 浏览器直连 | SN 账号系统：`register`/`login`/`refresh`/`logout`/`check_username`/`check_active_code` 等 | 账号态：access token（1h，`aud=sn`）+ refresh token（24h，`aud=sn-refresh`） |
| `/kapi/sn/deviceinfo` | 仅服务端（`node_daemon`/`active_server`），浏览器不直连 | 设备在线状态注册/上报、按 DID/hostname 查找 OOD（发现兜底，非调度真相源） | 见 §6.2，两种机制不通用 |
| `/kapi/bns` | 仅服务端 | BNS 文档读写（zone/boot/device_mini_doc 等） | 读匿名；写只接受签名好的 EVM 原始交易，不认任何 token（见 §6.4） |

`/kapi/sn` 根路径、旧的 `/kapi/sn/bns` 路径**不再路由任何方法**——这是一次没有做兼容层的破坏性变更（cyfs-gateway 侧原话："本版本是 breaking change，不要求兼容旧 RPC alias、旧 token 语义或旧 user_domain 绑定方式"），旧文档、旧脚本里出现的 `/kapi/sn/bns`、裸方法名（如 `register_user`、`bind_zone_config`）目前**完全打不通**。

### 6.2 两个不同的 SN 凭证机制——写文档/写代码时最容易搞混的点

激活期设备注册和激活后的运行期上报，走的是**两套完全不同的凭证机制**，不要混为一谈：

| | 激活期设备注册（一次性，在 `active_server.rs` 里） | 运行期在线上报（激活完成后，在 `node_daemon.rs` 主循环里） |
|---|---|---|
| 凭证 | `sn_access_token`（SN 账号态，1 小时有效，可用 refresh token 续） | 设备自己私钥签的短期 JWT（`generate_sn_device_token`），**不用 access token** |
| 为什么不能反过来 | SN 这时候还没见过这台设备，没有锚定公钥可比对，只能认账号态凭证 | 设备已经通过 BNS `device_mini_doc` 锚定了公钥，SN 可以直接验签，不再需要账号态凭证 |
| 获取方式 | 向导 SecurityStep 注册/登录时拿到；提交激活前 `acquire_sn_access_token()` 会先尝试 `refresh`，失败再重新登录一次（因为 1 小时有效期可能在用户填完向导前就过期了） | `node_daemon` 用本机已持久化的设备私钥现场签 |

**已废弃、不要再提的旧设计**：`sn_device_proof`（"设备自签 JWT 直接用于激活期注册"）是 2026-06-28 落地过一版的设计，9 天后（2026-07-06，commit `7d9f77d7`）被上面这套两阶段方案取代——现在 `sn_device_proof` 只residual 存在于日志脱敏字段名单里（防止老前端还发这个字段时泄露到日志），代码里已经没有任何地方真正生产或消费它。`notepads/sn-api-node-active-todo.md`（06-28）里写的"改用 sn_device_proof"是**中间态**，不是现状，写新文档时不要照抄那份 notepad 的这一条结论。

### 6.3 `device_mini_doc` 与"反 TOFU"验证

`device_mini_doc` 是一份 **owner 签名、发布在 BNS 上**的设备清单文档（`{version, devices: {device_name: {did, mini_config_jwt, role}}}`），SN 验证一次设备上报的流程：

1. 验证 JWT 自身签名（只证明"持有某把 Ed25519 私钥"，还不能证明身份）；
2. 检查有效期 ≤24 小时；
3. 从 `iss`（zone 域内 DID）解出 `(zone, device_name)`；
4. 要求该 zone 的 SN 账号存在且状态为 Active；
5. **关键锚定步骤**：独立地从 BNS 侧（设备文档 → `device_mini_doc` 聚合 → zone 文档内嵌的 devices map → 兼容表，按顺序查找）取出"权威公钥"；
6. 逐字节比对 token 里的公钥和权威公钥，任何不一致直接拒绝。

也就是说：**一台设备的 `device_mini_doc` 必须先通过 BNS 发布出去，它后续的 SN 上报才可能被接受**——SN 不会把"第一个自称是这台设备的请求"当真（无 TOFU）。

### 6.4 BNS 写路径：链上签名，非 token 授权

分层关系（从链到调用方）：

```
Bns.sol（链上合约，只认 msg.sender，是唯一权威）
  ← BNS-Indexer（只读事件投影）
  ← BNS-Server（/kapi/bns；写操作只有 tx.submit_raw 一个可用，纯转发已签名交易，不做任何鉴权）
  ← BNS-Client（构造未签名 ABI calldata）
  ← BNS-Controller（持有私钥，本地签好 EIP-1559 交易，管理 nonce，提交）
```

`/kapi/bns` 上除 `tx.submit_raw` 外的所有"写"形状的 RPC（`name.register`、`mutation.apply`、`document.publish` 等）在服务端都被**硬性禁用**，直接返回 `UNSUPPORTED_OPERATION`——协议里这些方法名还在，是因为内部封装库还用得到这些类型，不代表能直接调用。

`node_daemon`（`active_server.rs`）没有走 cyfs-gateway 里现成的高层封装 `SnBnsController`（那个封装目前唯一的生产调用方是 `cyfs-sn` 自己的 `auth.register` 引导流程），而是**直接手搓**：用 `bns-client` 的 `BnsEvmControllerClient` 拼 `zone`/`boot`/`device_mini_doc` 三份文档，本地签好交易后通过 `/kapi/bns` 的 `tx.submit_raw` 转发。Cargo 依赖上只加了 `bns-client`（`bns-indexer` 不是 buckyos 的直接或间接依赖，是 `bns-client` crate 自己测试用的 dev-dependency，`BnsIndexerClient` 类型是 `bns-client` 自己对远程 indexer 服务的客户端封装）。

**[未接入UI]** 这整条 BNS 发布能力（`publish_bns_zone_documents`）只有在请求里带了 `bns_evm_private_key` 才会触发，而**当前向导没有任何一步会收集这个字段**——目前只能通过 `node_daemon` 宿主环境变量（`BNS_PRIVATE_KEY` 等）或 `notepads/sn-auth-dev-active-script.md` 里那个 dev 脚本（`src/active_ood.ts`）触发。钱包路线这边同样缺一座桥：钱包桥目前只有 `walletSignWithActiveDid`，没有 EVM 原始交易签名接口，"钱包路线怎么做 BNS EVM 签名"还是待补的开放问题。

### 6.5 `auth.register` / `auth.login` 本身没有被去掉

容易误解的一点：`/kapi/sn/auth` 的账号系统（`register`/`login`/`refresh` 等）**依然存在且是当前设计的一部分**，被去掉的只是它对 BNS 状态的授权效力——账号 access token 不再能用来写 zone/boot/device_mini_doc 这些 BNS 文档，也不再是设备身份的锚（那是 BNS `device_mini_doc` 的职责）。`auth.register` 内部确实会触发一次 BNS 域名注册（用 SN 自己的运营方 EVM key），但那把运营方密钥被显式禁止碰 `zone`/`boot`/`device_mini_doc` 这三类文档，只用于最初的域名占位。

---

## 7. 已确认的现存问题（按影响排序）

1. **确认的现网 UI bug**：用户名校验失败但原因不是"已被占用"时，界面会展示字面字符串 `"error_name_invalid"` 而不是提示文案——这个 i18n key 在全部 9 个语言包里都不存在，`t()` 调用没传 `defaultValue`，i18next 会把 key 原样返回，JS 里写的兜底文案 `|| "Invalid name"` 因为返回值是 truthy 永远不会生效。服务端其实返回了更具体的 `message` 字段，但前端从未读取展示。
2. **没有再激活防护**：`save_local_device_identity_for_roots` 无条件写文件，不检查 `node_identity.json` 等文件是否已存在。唯一的防线是"只有加载身份失败时才会启动激活服务"这道启动期闸门——如果有人手工在已激活设备上重新以 `--enable_active` 启动 `node_daemon`，会静默覆盖现有身份和密钥。是否需要一个二次确认/硬拦截，是需要产品侧明确决策的点。
3. **SN 相关请求走 HTTP 而非 HTTPS**：两份 `active_config.json`（源码和打包产物里）都配的是 `"http_schema": "http"`，意味着浏览器发往 SN 的密码哈希、access/refresh token 默认走明文 HTTP。可能是有边缘 TLS 终止层这个仓库看不到，但值得在安全评审里明确确认，不要默认当作已经是 HTTPS。
4. **密码没有最小长度校验**：`error_password_too_short` 这个 i18n key 存在但代码里没有任何地方引用，当前只校验非空+两次输入一致。
5. **激活码/邀请码非空但校验未通过时，前端不拦截提交**：只在服务端注册/登录调用里失败后才报错，用户体验上是"提交了才发现不对"而不是实时拦截。

---

## 8. 已实现但向导未接入 / 被有意移除的能力

供后续写正式需求时甄别"要不要补 UI"还是"这本来就是废弃设计"：

- **访客访问 / 好友通行码**（`friend_passcode`、`enable_guest_access`）：数据模型和服务端参数透传都在，`SecurityStep` 里硬编码传空值/`false`，**没有任何 UI**。
- **"使用已有 SN 账号登录"作为独立入口**：不存在，当前是提交时自动判断注册还是登录；对应 i18n key 未使用，但它的姊妹 key（一句提示文案）还在用，文案内容却是"该入口预留，当前版本暂未实现"——文案本身是过时的、容易让人误解成"点了会报错"。
- **自定义/自建 SN 地址**：向导里不可配置，只能在部署期通过 `active_config.json` 设置，不支持按次激活自选。
- **OpenRouter Token 输入框**：数据模型和提交参数都在，但 UI 故意去掉了——有一条提交记录明确写着"Remove Open Router Input"，是主动移除，不是没做完。
- **BNS EVM 发布相关字段**：见 §6.4，服务端能力齐全，向导没有入口。
- **手工 DNS A/TXT 记录的分字段展示**：`ReviewStep` 现在是通用展示 `generate_zone_txt_records` 返回结果，不是旧设计里逐字段（A 记录/TXT 记录分开）展示，相关 i18n key 已不用。
- **AI Provider 步骤内的"填写 SN Active Code"弹窗**：`product/buckyos_desktop_installer/node_active集成OpenDAN.md`（这是一份**近期的、非过时的**增量规格，专门讲 AI Provider + Jarvis Msg Tunnel 两步，和本文档要归档的旧文档不是一回事）里设计了一个允许用户在 AI Provider 步骤内补填 SN Active Code 的弹窗，当前只实现了"已包含"的只读横幅，没有实现补填入口。
- **无自动化测试**：`node_active` 包内没有任何 `*.test.ts`/`*.spec.ts`。

---

## 9. 边界外确认：激活后的设备管理

明确排查过"Zone 激活后，怎么把第二台设备加进来"这个问题的现状：

- `product/control_panel/`、`product/desktop/` 下是纯文档目录，没有代码；实际前端在 `src/frame/desktop/src/app/`，实际后端在 `src/frame/control_panel/src/`。
- 唯一相关 UI 是设置页里的 **Cluster Manager**（`src/frame/desktop/src/app/settings/pages/ClusterManagerPage.tsx`）的 Nodes/Devices 子页——**只读**，数据来自本地 mock store，没有接后端真实 RPC，也没有"添加设备"/"重新激活"/"加入 Zone"的操作入口。
- 后端对应的 `zone_mgr.rs` 目前只做"当前这一台设备的自我状态汇报"，不是多设备花名册。

**结论：往一个已激活的 Zone 里加第二台设备，目前没有产品化流程**，如果正式 PRD 要覆盖这个场景，这是一块空白区域，不是"哪里已经做了但没写文档"。

---

## 10. 相关文档现状索引

| 文档 | 仓库 | 现状 |
|---|---|---|
| `product/archive/PRD(zh_CN)/1 Active BuckyOS.md` | buckyos | **完全过时**，不要作为基线 |
| `product/buckyos_desktop_installer/node_active集成OpenDAN.md` | buckyos | 近期有效的增量规格（AI Provider + Jarvis 两步），文件名具有误导性（看起来像讲 node_active 整体或 OpenDAN 集成，实际不是），但内容本身不过时 |
| `doc/arch/gateway/SN.md` | buckyos | 架构叙述准确，但**行号引用已漂移**（最后编辑于 2026-06-28，之后代码又经过十余次提交），引用代码位置前建议重新核实 |
| `cyfs-gateway/doc/SN/SN-API.md` | cyfs-gateway | 当前，与代码一致，是 SN 协议的权威参考 |
| `cyfs-gateway/doc/SN/SN-Auth.md`、`doc/BNS/SN-BNS-Contoller.md` | cyfs-gateway | 设计意图部分准确，但"当前实现映射"章节部分**已被代码超越**（例如声称 `auth.register` "尚未接入" BNS 写入，实际已经接入） |
| `cyfs-gateway/doc/BNS/BNS-签名边界改造-EVM-TX-TODO.md` | cyfs-gateway | 当前，是 BNS 写路径架构最可信的单一来源 |
| `cyfs-gateway/doc/SN/新SN核心流程整理.md` | cyfs-gateway | 当前，描述目标架构且与代码一致 |
| `cyfs-gateway/doc/old_sn/*` | cyfs-gateway | 完全过时，只作为"改造前"的历史对照 |
| `notepads/sn-api-node-active-todo.md` | buckyos | 2026-06-28 决策记录，**其中"改用 sn_device_proof"一条已被 07-06 的改动推翻**，其余密钥/BNS 约束仍然成立 |
| `notepads/sn-auth-dev-active-script.md` | buckyos | 2026-07-06 完成记录，当前 |

---

## 11. 变更时间线（帮助理解"为什么现在长这样"）

1. **06-28 之前**：SN 是一个职责混杂的服务，`/kapi/sn/bns` 一个路径处理用户+zone+设备+DNS+DID+管理，账号态 token 既管账号也管 BNS 状态，SN 自己存 owner 公钥、DID 文档、zone_config。
2. **06-28 首次落地**（commit `1ed03185`/`ff3da6d3`）：拆分为 `/kapi/sn/auth`（账号）+ `/kapi/sn/deviceinfo`（设备在线态）+ 独立的 `/kapi/bns`（BNS 文档）。`node_active` 不再持久化 access token；引入 `sn_device_proof`（设备自签 JWT），意图连激活期注册也用它替代 access token。
3. **06-28 当天后续**：`node_daemon` 接入 `bns-client`，`active_server` 获得 BNS 发布能力（`register_name`/`apply_mutations`）。
4. **07-06 "SN-Auth 两阶段"重做**（commit `7d9f77d7`）：发现激活期注册这一下 SN 还没见过设备、没法验自签 proof，于是把激活期注册改回**账号态短期 access token**（`sn_access_token`，附 refresh 机制），但运行期上报保持不变（仍是设备私钥签的 JWT，不受影响）。同一次提交补上了 `active_ood.ts` 这个 dev 脚本，第一次让 `do_active` 能在开发环境里被端到端跑起来验证。

---

## 附：主要代码位置索引

- `src/kernel/node_active/active_lib.ts` — 前端业务逻辑与 RPC 封装
- `src/kernel/node_active/src/App.tsx`、`src/components/ActiveWizard.tsx`、`src/components/steps/*.tsx` — 向导 UI
- `src/kernel/node_active/reademe.txt` — 最接近"设计说明"的一份内部笔记
- `src/kernel/node_daemon/src/active_server.rs` — `/kapi/active` RPC 服务端实现
- `src/kernel/node_daemon/src/node_daemon.rs` — 激活触发闸门 + 激活后运行期上报
- `src/kernel/buckyos-api/src/device_identity.rs` — 身份文件落盘
- `src/active_ood.ts` — 开发环境端到端激活脚本
- `cyfs-gateway/src/components/cyfs-sn/` — SN 服务端实现
- `cyfs-gateway/src/components/bns-client/`、`bns-evm/` — BNS 客户端与 EVM 签名
- `cyfs-gateway/src/components/cyfs-gateway-api/src/sn_client.rs` — SN 客户端封装

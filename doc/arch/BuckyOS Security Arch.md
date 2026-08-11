# BuckyOS Security 机制的整体设计

> 整理来源：基于一段关于 BuckyOS Security / 密钥管理 / 分布式权限模型的语音记录整理。  
> 术语修正：原语音转写中大量出现的“种 / 中 / 重 / 众 / zoom”等，在上下文中均指 **Zone**，即以家庭或组织为边界的一组分布式设备、服务、数据和身份关系。本文统一使用 **Zone**。  
> 名称说明：语音稿中出现了 `wlifiphub`、`walletpyhub`、`WorldFHub`、`WareHub` 等疑似语音识别误差。根据上下文，它们均表示“负责本地身份验证、登录态签发和 JWT issuer 的内核身份组件”。本文暂统一记为 **VerifyHub**；最终工程名需以代码和产品命名为准。

---

## 1. 设计背景与目标

BuckyOS 的 Security 设计首先服务于一个核心产品假设：**帮助普通人构建以家庭为中心的分布式系统**。虽然 BuckyOS 的操作系统形态也应支持企业或中大规模分布式系统，但第一优先级仍是让普通用户能安全、低摩擦地拥有自己的 Zone。

因此，本设计不是单纯讨论“密钥放在哪里”，而是围绕以下问题构建完整安全模型：

1. 用户如何持有最高权限，而不让最高权限密钥进入 Zone 内部设备。
2. Zone 内部如何区分 Root、Administrator、Normal User、Guest / Local User。
3. 内核组件、系统服务和普通 App 如何分层隔离。
4. 一个请求到达系统服务时，如何基于 **UserID + AppID + Resource Path + Action** 做统一鉴权。
5. 分布式节点之间、客户端设备与 Zone 之间、Zone 与公网之间如何建立可信连接。
6. 跨 Zone 的社交内容、应用发布、数字内容发布等场景，如何证明“内容是谁发布的”。

整体目标可以概括为：

- **Root Key 不入 Zone**：最高权限密钥只保存在用户钱包或兼容签名器中，不应出现在任何 Zone 设备的存储区或常驻内存中。
- **日常操作低摩擦**：普通管理操作通过 VerifyHub、JWT、管理员态完成，避免每个动作都要求用户掏出钱包。
- **关键操作强签名**：涉及 DID、BNS、链上状态、应用发布、严肃数字内容发布等操作，必须回到 Root / Global DID 级别签名。
- **请求级鉴权统一化**：所有系统服务都应通过统一 RBAC Policy 表做鉴权，不允许各服务各自发明权限模型。
- **App 默认不可信**：第三方 App 默认运行在容器中，权限来自 Manifest 声明和用户确认，原则上按用户隔离数据。
- **Zone 边界清晰**：Zone 是家庭或组织分布式系统的安全边界，设备、节点、用户、服务、TLS、Gateway、SystemConfig 都围绕 Zone 建模。

---

## 2. 核心概念

### 2.1 Zone

**Zone** 是 BuckyOS 中的核心安全与资源边界，可以理解为一个用户或组织控制下的分布式私有云。典型家庭 Zone 由以下对象组成：

- 若干 Node：真正参与计算、存储、服务承载的设备。
- 若干 Client Device：只发起请求的客户端设备，如手机、浏览器、桌面客户端。
- 一组 Kernel Components：如 SystemConfig、Gateway、NodeDaemon、Scheduler、VerifyHub 等。
- 一组 System Services：官方系统服务，服务多个用户。
- 多个 App Service / Container：普通应用服务，通常按用户隔离运行。
- 一套 Zone 级身份、TLS 证书、设备密钥、RBAC Policy、BootConfig 等。

Zone 的 Owner 通常由 Root Key 控制。Root Key 也可更新 BNS / DID / 链上相关核心配置。

### 2.2 DID / Global Identity

用户的根身份是 **Global DID**，其根密钥不依赖任何 Zone。一个用户可以：

- 在家庭 Zone 中拥有账号。
- 在公司 Zone 中拥有账号。
- 在学校 Zone 中拥有账号。
- 保持同一个 Global DID 作为自己的长期身份。

这意味着：Zone 管理员可以管理用户在本 Zone 内的数据和权限，但不应拥有该用户在其他 Zone 中的身份权限，也不能模拟该用户的 Global DID。

### 2.3 SystemConfig

**SystemConfig** 是 Zone 的配置与安全事实源。它至少负责保存：

- BootConfig。
- NodeConfig。
- RBAC Policy 表。
- VerifyHub 当前公开公钥。
- Gateway / Zone Gateway 相关配置。
- 某些高安全区域中的敏感数据，如 VerifyHub 私钥、TLS 证书私钥等。

SystemConfig 是明确的内核组件。一个 Node 加入 Zone 后，只有成功连接 SystemConfig、通过设备身份认证，并读取 BootConfig，才算完成 Booting，成为有效 Node。

### 2.4 VerifyHub

**VerifyHub** 是 Zone 内负责用户本地身份验证和登录态签发的内核身份组件。它提供比 Root Key 更日常化的认证方式，例如：

- 密码。
- PIN。
- Keystore。
- 未来可能接入的其他本地认证方式。

用户登录成功后，VerifyHub 使用自己的私钥签发 JWT。系统服务通过 JWT 的 `iss` 字段找到 VerifyHub，再从 SystemConfig 获取对应公钥，验证 JWT 的有效性。

VerifyHub 的密钥应可轮换。其他系统服务不需要理解 VerifyHub 的密钥轮换细节，只需要相信 SystemConfig 中当前 issuer 对应的公钥是真实事实源。

---

## 3. 权限分层模型

BuckyOS 的权限模型同时存在两条维度：**用户身份维度** 和 **软件运行维度**。

### 3.1 用户身份层级

| 层级 | 名称 | 说明 | 典型能力 |
|---|---|---|---|
| L0 | Root / Owner | Zone 的起点，通常由用户钱包中的 Root Key 控制 | 更新 DID / BNS / 链上核心配置；执行最高权限操作；恢复或迁移 Zone |
| L1 | Administrator | Root 用户的日常管理态 | 安装 App；管理用户；调整系统配置；执行常规管理任务 |
| L2 | Normal User | 标准成人用户，可基于 Global DID 入驻 Zone | 使用已安装 App；管理自己的数据；参与跨 Zone 交互 |
| L3 | Restricted / Local / Guest User | Zone 内创建的本地或受限账号，无全局 DID | 临时访问；企业员工账号；可由 Zone 管理者随时禁用 |

设计上，一个标准用户可以同时拥有：

- Global DID 的 Root 权限。
- 某个 Zone 中的 Administrator 权限。
- 日常使用时退化成 Normal User 的权限。

系统不鼓励日常长期处于 `su` / Administrator 状态。管理员需要时可以提权，但默认日常使用应接近普通用户态。

### 3.2 软件运行层级

| 层级 | 名称 | 说明 | 安全定位 |
|---|---|---|---|
| S0 | Root Key / Root Operation | 不常驻 Zone 内部的软件层，而是最高签名能力 | 只用于关键签名，不进入 Zone 存储 |
| S1 | Kernel Components | SystemConfig、Gateway、NodeDaemon、Scheduler、VerifyHub 等 | Zone 能运行起来所必需的最小内核面 |
| S2 | System Services | 官方系统服务，通常服务多个用户 | 权限高于普通 App，但低于 Kernel；需要谨慎隔离 |
| S3 | App Services | 第三方或用户安装的普通应用 | 默认不可信，容器化运行，权限由 Manifest + 用户确认控制 |

其中，Kernel Components 应尽可能少。越多组件进入内核面，系统攻击面越大，安全审计成本越高。

---

## 4. 密钥体系设计

### 4.1 Root Key

Root Key 是系统的最高权限起点，也是 Zone Owner 的密钥。它具备以下特征：

- 推荐通过钱包模式生成和保存。
- 通常由助记词派生私钥。
- 私钥保存在移动端钱包或桌面钱包中。
- 助记词由用户线下备份。
- 不应出现在任何 Zone 设备的存储区。
- 不应出现在 Zone 内任何长期运行进程的常驻内存中。
- 可用于更新 BNS / DID / 链上核心配置。
- 可用于 Zone 级恢复、迁移、关键授权等操作。

Root Key 的使用路径应是：用户在浏览器或系统界面触发关键操作，系统生成待签名请求，通过二维码、深链或类似机制拉起钱包，由钱包完成签名授权。

### 4.2 兼容 Web Signer 模式

为开发、测试或低摩擦场景，系统可保留非钱包兼容路径：

1. Web 页面生成类似 Root Key 的密钥。
2. 用户设置密码。
3. 私钥被密码加密后导出为文本。
4. 用户自行保存该加密文本。
5. 需要签名时，用户导入文本并输入密码，由 Web Signer 完成签名。

该路径产品上应被标注为兼容 / 开发 / 高风险模式，而不是普通用户的推荐路径。原因是它更依赖用户密码强度、浏览器安全、文件保存习惯和钓鱼防护能力。

### 4.3 VerifyHub Key

VerifyHub 持有一组 Zone 内登录态签发密钥：

- 私钥用于签发 JWT。
- 公钥发布到 SystemConfig。
- 系统服务根据 JWT 的 `iss` 字段从 SystemConfig 获取当前公钥。
- VerifyHub 密钥应支持周期性轮换。
- 多 VerifyHub 高可用部署时，可在 SystemConfig 中公布多个有效 issuer / public key。

当前设计中的重要争议点是：**VerifyHub 私钥泄露后的最高权限应是什么？**

候选设计：

| 方案 | 含义 | 优点 | 缺点 |
|---|---|---|---|
| A | VerifyHub 可签发 Administrator 级 JWT | 日常管理体验好，管理员输入密码即可完成大部分操作 | VerifyHub 私钥泄露后爆炸半径较大 |
| B | VerifyHub 只能签发 Normal User 级 JWT | 安全边界更清晰，管理操作必须 Root Key 签名 | 管理操作更繁琐，普通用户体验较差 |
| C | VerifyHub 默认签发 Normal，短时提权需 Root Key 或强认证 | 在安全和体验间折中 | 实现复杂，需要更精细的 session / capability 设计 |

语音稿中的倾向是：当前可先按 Administrator 级处理，但必须明确这是安全与易用性之间的权衡，并依赖密钥轮换降低长期风险。

### 4.4 Device Key

设备身份用于 Gateway 和节点间通信。设备可分为两类：

- **Node Device**：参与 Zone 内计算、存储、服务承载的节点。
- **Client Device**：手机、浏览器、桌面客户端等主要发起请求的设备。

Node Device 的公钥应注册在 SystemConfig 中。Gateway 接收 Zone 内流量时，可以根据设备身份验证请求是否来自可信 Zone 内设备。

Client Device 可通过设备描述符证明身份，该描述符需要 Administrator 签名。Client Device 的权限安全等级可低于 Node Device，但仍应在 Gateway 层被识别。

### 4.5 Gateway Communication Key

Gateway 是 Node 上所有跨节点流量的入口和出口。原则上，只有 Gateway 才应持有设备通信私钥。这样做的原因是：

- 避免每个进程都持有设备身份密钥。
- 所有访问非本节点服务的请求都必须经过 Gateway。
- Gateway 可以在流量层先区分 Zone 内流量和 Zone 外流量。
- Gateway 可作为第一道防火墙，阻断不应到达内部服务的请求。

### 4.6 Zone Gateway TLS Key

如果某个服务需要被公网浏览器访问，就需要由 **Zone Gateway** 承担公网入口职责。Zone Gateway 需要传统 TLS / X.509 证书能力。

TLS Key 的设计原则：

- TLS 私钥属于高安全密钥。
- 不应分散保存在所有节点上。
- 只有被授权承担 Zone Gateway 职责的 Node 可以读取。
- 多 Zone Gateway 高可用部署时，可共享同一组 TLS 证书。
- 证书获取逻辑与证书安全存储逻辑应分离。
- 不同 TID / DID / 域名映射方式可能需要不同的证书获取器。

如果 TLS 私钥泄露，攻击者可能伪造 Zone Gateway 或进行中间人攻击，因此它应和 VerifyHub 私钥一样放入 SystemConfig 的高安全区域，并有严格读写控制。

### 4.7 Zone Signing Key

跨 Zone 的内容发布、评论、社交交互等场景，可能不适合每次都使用用户 Global DID 的 Root Key 签名。此时可以使用 Zone 级签名能力代表用户的日常行为。

可分两类签名：

| 签名级别 | 使用密钥 | 适用场景 | 信任含义 |
|---|---|---|---|
| Root Signature | 用户 Global DID Root Key | 应用发布、重要内容发布、链上登记、收益相关内容 | 用户本人强确认 |
| Zone Signature | 用户绑定 Zone 的 Zone Key / VerifyHub 签名能力 | 评论、普通社交内容、日常跨 Zone 消息 | 用户认可该 Zone 可代表其日常行为 |

Zone Signature 的信任路径是：

1. 验证内容中的用户 DID。
2. 查询用户 DID Profile。
3. 确认用户 Profile 中绑定了某个 Zone。
4. 通过 Zone DID / 域名 / DID Well-Known 配置获取 Zone 公钥。
5. 验证内容 JWT 或签章是否由该 Zone 公钥对应私钥签发。

这意味着：Zone Owner 理论上可以伪造本 Zone 内用户的一些日常内容。因此，对于应用发布、收费内容、关键数字资产等严肃场景，仍应要求用户 Global DID Root Key 签名。

---

## 5. 统一鉴权模型

BuckyOS 的核心鉴权模型基于两条规则：

1. 当前请求代表哪个 **UserID**。
2. 当前请求来自哪个 **AppID**。

请求是否允许访问某个资源，由以下对象共同决定：

- UserID。
- AppID。
- Resource Path。
- Action / Method。
- RBAC Policy。
- JWT issuer 与签名验证结果。

### 5.1 UserID 的来源

UserID 可以由两种方式证明：

#### 方式 A：Global DID 自签名

用户使用 DID Document 中的公钥对应私钥，对 JWT 或请求进行签名。系统验证成功后，认为该用户获得了最高级别的用户身份凭证。该方式不依赖 Zone 内任何服务。

#### 方式 B：VerifyHub 签发 JWT

用户通过密码、PIN、Keystore 等方式登录 VerifyHub。VerifyHub 验证成功后签发 JWT。系统服务验证 JWT 的方式是：

1. 读取 JWT 的 `iss`。
2. 根据 `iss` 从 SystemConfig 获取 VerifyHub 公钥。
3. 验证 JWT 签名。
4. 从 JWT claim 中获得 UserID 和权限上下文。

该方式是日常使用路径。

### 5.2 AppID 的来源

AppID 表示“当前请求来自哪段代码 / 哪个应用”。AppID 在应用安装时确定。

可分三类：

| AppID 类型 | 说明 |
|---|---|
| `kernel` | 所有内核组件可统一使用 `kernel`，避免权限系统自身陷入循环依赖 |
| System Service AppID | 每个系统服务有确定的 AppID，安装或启用时进入 RBAC Policy |
| Third-party AppID | 普通 App 的身份，由安装记录和 Manifest 决定 |

对于浏览器来源的 App 请求，VerifyHub 或 Gateway 还需要结合请求来源域名判断 AppID，避免浏览器侧伪造。

### 5.3 RBAC Policy

RBAC Policy 保存在 SystemConfig 中，是统一权限事实源。

鉴权流程可以抽象为：

```text
request -> verify JWT -> derive UserID -> derive AppID -> locate Resource Path -> check RBAC Policy -> allow / deny
```

对于 `root + kernel` 组合，原则上拥有全部权限。但除该组合外，所有访问均应走标准 RBAC 判定。

RBAC Policy 采用 **pull 模型**：业务服务端每 **30 秒** 从 SystemConfig 定期刷新 Policy。这会带来最长 30 秒的授权传播延迟，是"允许一致性抖动"取舍的一部分（参见 §5.5）。

### 5.4 系统服务的责任

BuckyOS 并不存在一个完全自动的“硬栅栏”替系统服务完成所有权限隔离。系统服务必须承担以下责任：

- 设计清晰的 Resource Path。
- 对每个外部请求提取 UserID 与 AppID。
- 验证 JWT issuer 与签名。
- 调用统一 RBAC Policy 做鉴权。
- 在未授权时拒绝访问。
- 不绕过 SystemConfig 中的权限事实源。

这意味着系统安全不仅依赖 Policy 表本身，还依赖各系统服务正确实现拦截逻辑。

### 5.5 Token / Session 生命周期与吊销

VerifyHub 登录成功后签发的不是单个 JWT，而是绑定到同一 **session-id** 的一对令牌（token-pair）：

| 令牌 | TTL | 用途 |
|---|---|---|
| Access Token | 15 分钟 | 业务访问；系统服务可离线验签，VerifyHub 短时不可用时仍可用到过期 |
| Refresh Token | 7 天（device / user / service 当前不区分） | 向 VerifyHub 换取新的 token-pair |

JWT 至少包含 `iss` / `sub` / `aud` / `iat` / `exp` / `kid` / `token_use` / `session_id`，其中 `token_use` 标识 access / refresh / bootstrap，签名算法为 Ed25519（JWT 为 EdDSA）。系统当前不使用 `jti`。

生命周期策略：

- **Refresh Token Rotation**：每次 refresh 返回新的 refresh token，旧的随即作废。
- **Reuse Detection**：若检测到已作废的 refresh token 再次被使用，视为泄露信号，触发风控（至少要求重新登录，可升级为吊销整个 session）。
- access 与 refresh 共享同一 session-id；refresh 过程复用 session-id，不生成新的。

吊销以 **session-id 为主键**，VerifyHub 将其标记为无效。吊销的一致性语义直接体现"可用性优先"的取舍：

- **refresh**：强一致，吊销后立即失败。
- **access token**：若业务服务仅离线验签，吊销到业务侧完全生效存在最长 15 分钟（access TTL）的窗口。
- 高风险场景可改为调用 VerifyHub 在线校验（introspection），以获得即时吊销效果。

> 设计前提：BuckyOS 作为 NetworkOS 在安全与可用性之间明确取舍——允许 VerifyHub 短时不可用（access token 离线验签兜底），允许配置 / 授权传播延迟（如 RBAC 30 秒刷新）。单个 OOD 节点损坏不应导致 VerifyHub 永久不可用，因此其私钥存于 SystemConfig 高安全区以便在另一节点拉起。

### 5.6 Service 启动换票（bootstrap JWT）

System Service / App Service 启动时如何获得自己的身份令牌？BuckyOS 采用 **hosting device 背书 + 短时一次性 bootstrap JWT** 的方案，而不是让 service 自证身份：

1. service 的运行授权由其所在 Node / OOD（hosting device）背书。
2. hosting device 用 **device 私钥** 签发一枚短时一次性 bootstrap JWT，通过 ENV 注入给 service。
3. service 用该 bootstrap JWT 调用 `VerifyHub.login_by_jwt` 换取长期 token-pair，之后周期性 refresh。

bootstrap JWT 规范：

- 签名者：hosting device 私钥；`iss = $devicename`
- `aud = verify-hub`，`token_use = bootstrap`
- exp 极短（远小于 refresh TTL）
- `nonce`：随机一次性值，用于防重放
- `target_service_id`：目标服务标识，与 Scheduler 构造的 stream URL 中 target service id 对齐

VerifyHub 换票（`login_by_jwt`）校验：

```text
1. 基础 JWT 校验：exp / iat / aud / token_use
2. device 身份：从 SystemConfig 读取 device 公钥与状态（kv://nodes/$devicename/config），验签并校验状态
3. nonce 防重放：检查 (iss, nonce) 是否已用；去重记录 TTL 需覆盖 bootstrap 有效期 + clock skew
4. service 运行授权：从 SystemConfig / NodeConfig 读取该 device 允许运行的 service allowlist / 策略，
   校验 target_service_id 在授权范围内
```

校验通过后签发 token-pair。该方案的安全意义是：**device 自签 token 不作为业务服务的常规访问凭证**，service 必须经 VerifyHub 背书换票，降低单节点被攻破后的横向扩散风险（与 §4.4 Device Key、§5.2 AppID 呼应）。

### 5.7 sudo：用户自签特权主体

§3.1 提到"管理员需要时可以提权，但默认日常使用应接近普通用户态"。其落地机制是 **sudo token**——由 user 用自己的私钥（而非 VerifyHub）签名的 session token：

- 业务服务（api-runtime）常规只信任 zone-owner / VerifyHub 签名的 token；sudo 是 api-runtime 中的**特例路径**。
- 用户可声明 `sub = su_alice`；服务端从 SystemConfig 读取 alice 公钥（kv://users/alice/config）验签，并要求 `su_alice` 的签名公钥必须来自 `alice` 的配置（持钥即授权，不引入二次交互验证）。

由于 RBAC 表达式 `kv://users/{user}/*` 无法覆盖 `su_alice` 这类主体，sudo 采用显式规则：

```rbac
p, su_alice, kv://users/alice/key_settings, read|write, allow
g, alice, admin
g, su_alice, sudo
```

api-runtime 识别到 `su_` 前缀时的授权流程：

```text
1. 验签 sudo token（从 kv://users/{alice}/config 取公钥）
2. 先以普通主体 alice 执行 enforce
3. 若失败，再以 su_alice 执行 enforce（仅在显式授权范围内放行）
```

该路径不改变"业务服务常规仅信任 zone-owner / VerifyHub 签名 token"的总体信任域。

### 5.8 信任矩阵（摘要）

| 校验者 | 接受的签名者 / 公钥 | 用途 |
|---|---|---|
| SystemConfig | ENV 注入的当前 device pubkey（自举期） | boot / self-host bring-up |
| SystemConfig | zone-owner pubkey（BootConfig） | root 管理能力 |
| SystemConfig | VerifyHub pubkey（BootConfig） | 常规系统操作与统一登录体系 |
| VerifyHub | device pubkey（SystemConfig: kv://nodes/$devicename/config） | device 登录、service bootstrap 背书 |
| VerifyHub | user pubkey（SystemConfig: kv://users/$user/config） | JWT-based user 证明场景（sudo 不走 VerifyHub） |
| 业务 Service (api-runtime) | VerifyHub pubkey（BootConfig） | 常规业务 access / refresh token 验签 |
| 业务 Service (api-runtime) | zone-owner pubkey（BootConfig） | root 级 token 验签 |
| 业务 Service (api-runtime) | user pubkey（SystemConfig: kv://users/.../config） | sudo 特例路径 |

---

## 6. App 安全模型

### 6.1 安装权限

当前设计倾向是：只有 Root / Administrator 可以安装 App，普通用户只能使用已安装 App。

这样做有几个好处：

- 避免家庭 Zone 中多个用户安装不同版本的同一 App，造成状态和数据冲突。
- 降低恶意 App 进入系统的概率。
- 简化 App 生命周期管理。
- 让管理员承担应用信任决策。

### 6.2 Manifest 权限

普通 App 的权限来自：

1. App Manifest 声明。
2. 安装时管理员 / 用户确认。
3. 系统运行时 RBAC Policy 限制。
4. 必要时用户可主动收缩权限。

安装前可以引入信用体系或声誉系统，但即使 App 已经安装，也必须默认不可信。

### 6.3 容器隔离

普通 App 应运行在 Docker 或类似容器中。默认隔离原则：

- 一个 App 实例通常只服务一个用户。
- 用户 A 的 App 容器只能访问用户 A 的数据。
- 用户 B 使用同一个 App 时，应有独立容器或独立数据边界。
- 容器内进程不能读取 Host 上的内核服务数据目录。

### 6.4 家庭场景下的共享例外

家庭 Zone 中存在半社交需求。例如音乐播放 App 中，家庭成员可能希望看到彼此正在听什么歌。此类能力不应破坏默认隔离，而应被建模为显式权限：

- App 在 Manifest 中声明跨用户可见能力。
- 用户或管理员确认授权。
- RBAC Policy 中记录该能力。
- 系统服务按 Resource Path 做细粒度控制。

---

## 7. Kernel Components 与启动链路

### 7.1 Kernel 的最小目标

BuckyOS Kernel 的核心目标是：

1. 让 Node 能够加入 Zone。
2. 让 Node 能够连接 SystemConfig。
3. 让 Zone 能够根据 SystemConfig 管理节点资源。
4. 让服务、任务和用户态 App 能被调度并运行起来。

### 7.2 核心内核组件

| 组件 | 职责 | 安全要点 |
|---|---|---|
| SystemConfig | 配置事实源、BootConfig、RBAC、密钥公钥与敏感区 | 需要强写保护；敏感区需要强读写保护 |
| Gateway / CYFS Gateway | 节点流量入口与出口，区分 Zone 内外流量 | 持有设备通信密钥；提供第一层防火墙 |
| NodeDaemon | 每个 Node 上必跑的守护进程，负责加入 Zone 和执行调度指令 | 读取 NodeConfig，启动本节点承载的服务或任务 |
| Scheduler | 根据 SystemConfig 中的服务配置、Node 资源和状态做调度 | 写入期望状态到 SystemConfig |
| VerifyHub | 用户本地登录、JWT issuer、认证扩展点 | 私钥高安全存储；公钥发布；支持轮换 |
| k-event | 系统事件通知机制 | 支撑内核与服务间事件流 |
| k-message queue | 可靠消息队列 | 支撑分布式内部异步通信 |
| TaskManager / Workflow | 长任务、工作流、任务派发 | 是否属于内核组件仍需边界确认 |

### 7.3 Node Booting 流程

Node 加入 Zone 的过程可以抽象为：

```text
Node 启动
  -> 进入 booting 状态
  -> 根据外部配置 / 缓存 / 发现机制寻找 Gateway / SystemConfig
  -> 连接 SystemConfig
  -> 提交设备身份认证
  -> 读取 BootConfig
  -> 获取 VerifyHub 公钥、内核组件配置等
  -> 成为有效 Node
  -> 读取 NodeConfig
  -> 启动本节点应承载的服务 / 任务
```

当 Node 成功读取 BootConfig 后，才算真正进入 Zone 并可贡献物理资源。

### 7.4 Boot 阶段自举（首个 OOD 与 Scheduler）

§7.3 描述的是 Node 加入**已存在** Zone 的通用流程。Zone 首次 bring-up（首个 OOD）还有一段信任自举链路：

```text
OOD 启动 SystemConfig
  -> ENV 注入当前 device 公钥，作为 SystemConfig 的初始 trust key（自举用，打破循环依赖）
OOD 启动 Scheduler
  -> ENV 注入一枚由 device 私钥签名的 session-token
Scheduler
  -> 用该 session-token 通过鉴权，完成首次 SystemConfig 写入（初始调度结果 + 配置）
SystemConfig
  -> 持久化到 kv://（OOD 集群，规模 2n+1 的一致性存储）
```

由此，SystemConfig 的 trust key 有两类来源：

- **自举期**：ENV 注入的当前 device 公钥。
- **稳定态**：BootConfig 中的 zone-owner 公钥与 VerifyHub 公钥。

> SystemConfig 底层是类似 etcd 的一致性存储（`kv://`），仅运行在 OOD 集群上。VerifyHub 私钥存于 `kv://secrt/` 高安全区，正是为了在单个 OOD 节点损坏后能在另一节点重新拉起 VerifyHub（参见 §5.5 的可用性前提）。

---

## 8. Gateway 安全模型

Gateway 是 Zone 内分布式通信的关键安全边界。

### 8.1 Gateway 的基本职责

Gateway 负责：

- 接收进入本 Node 的所有跨节点流量。
- 发出本 Node 到其他 Node 的跨节点流量。
- 持有设备通信私钥。
- 判断流量是 Zone 内还是 Zone 外。
- 判断对方设备身份。
- 阻止未认证设备访问内部服务。
- 为内部服务提供第一层流量级隔离。

### 8.2 Zone 内流量

Zone 内 Node-to-Node 通信类似传统后台系统中的 S2S 通信。Gateway 应在协议层识别对端设备身份。

验证依据包括：

- 对端 DeviceID。
- SystemConfig 中登记的设备公钥。
- 请求中携带的设备签名证明。
- 设备是否属于当前 Zone。

如果某个服务不是公开服务，只服务 Zone 内对象，那么 Gateway 应在流量层就拒绝 Zone 外或未认证设备的访问。

### 8.3 Client Device 流量

Client Device 不是完整 Node，它可能只是用户手机、桌面 App 或浏览器。它可通过设备描述符声明身份，设备描述符应由 Administrator 签名。

Client Device 的权限可低于 Node Device，但仍需在 Gateway 层被识别并打上身份上下文。

### 8.4 Zone Gateway 公网流量

不是所有 Gateway 都必须承担 **Zone Gateway** 职责。Zone Gateway 是对公网提供 HTTP(S) 服务的入口。

公网访问路径通常为：

```text
Browser / Internet Client
  -> DNS / DID / BNS / SN Relay
  -> Zone Gateway
  -> 内部 System Service / App Service
```

Zone Gateway 需要 TLS 身份，因此 TLS 证书私钥必须安全保存。多 Zone Gateway 高可用部署时，可以让两个或多个授权 Gateway 共享同一组 TLS 证书，但不能让所有节点都能读取该证书。

---

## 9. SystemConfig 安全边界

SystemConfig 是最关键的内核组件之一。其安全风险分为两类。

### 9.1 写权限风险

如果攻击者能修改 RBAC Policy，就可以给自己或恶意 App 注入最高权限。此时整个权限系统会失效。

因此，RBAC Policy 的写权限必须严格限制在 Root / Administrator / Kernel 可信路径中。

**稳定态判定与写权限收口（实现现状）**：现行约定是只要 BootConfig 存在，即认为系统处于稳定态。目标策略是在稳定态限制 device-key 仅 read（或受限写），敏感写（RBAC、信任根、关键配置）仅 root / VerifyHub 可执行。**该写限制当前尚未实现，是高优先级加固点**。此外 `kv://boot/config` 仅 boot 阶段可写、运行态仅 root 可写——具备其写权限即等价于系统最高权限（Root of Trust）。

### 9.2 读权限风险

SystemConfig 高安全区域可能保存 VerifyHub 私钥、TLS 私钥等敏感数据。如果攻击者能读取这些数据，即使不能改 Policy，也可能伪造登录态或公网服务身份。

因此，高安全区域不仅需要防写，还需要防读。

### 9.3 分布式服务与 Host OS 的双重防御面

每个系统服务都有两个防御面：

1. **分布式资源访问面**：服务通过 BuckyOS 权限系统保护自己管理的资源。
2. **本地 Host 文件系统面**：服务最终运行在某台机器上，其本地数据目录必须被 Host OS 权限保护。

普通 App 在容器中，默认不应读取 Host 文件系统。但系统服务不一定都在容器内，因此建议：

- Kernel 服务使用独立高权限用户运行。
- System Service 使用低权限系统用户运行。
- 不同系统服务使用不同 Unix 用户 / 用户组。
- Kernel 数据目录不允许 System Service 用户读取。
- 敏感目录按最小权限原则设置。

---

## 10. 用户身份与账号模型

### 10.1 标准 Global DID 用户

标准用户先拥有自己的 Global DID，再加入某个 Zone。其特征是：

- 根身份不属于任何 Zone。
- 可加入多个 Zone。
- 可关闭自己在某个 Zone 内的账号。
- Zone 管理员可管理其本 Zone 数据，但不能控制其全局身份。

### 10.2 Zone Local User

Local User 的根身份属于 Zone。它适用于：

- Guest 账号。
- 临时使用账号。
- 企业员工账号。
- 不希望用户带走身份的组织场景。

Local User 的最高权限掌握在 Zone 管理者手中。管理员删除或禁用该账号后，该账号在依赖该 Zone 身份的服务中应立即失效。

### 10.3 企业场景

企业希望员工身份代表企业成员身份，而不是个人永久身份。因此企业 Zone 可以创建 Local User 或 Restricted User，并在员工离职时禁用该账号。

这与个人社交网络中的 Global DID 不同：

- Global DID 表示个人长期身份。
- Enterprise Local User 表示组织授予的临时成员身份。

---

## 11. 跨 Zone 内容签名与 SSNS 场景

### 11.1 内容发布的两种证明方式

在去中心化社交或内容网络中，系统不能依赖中心服务器证明“这条内容是谁发的”。因此内容本身需要可验证签名。

内容发布可分为两种证明方式：

1. **用户 Root 签名**：用户用 Global DID Root Key 签名内容哈希或内容对象。
2. **Zone 签名**：用户绑定的 Zone 使用 Zone 级密钥或 VerifyHub 能力为用户日常内容签名。

Root 签名强度最高，但不能频繁要求用户使用。Zone 签名适合评论、点赞、普通消息等高频场景。

### 11.2 Alice / Bob 评论流程示例

场景：Bob 打开 Alice 的社交 App 页面，想评论 Alice 发布的内容。

基本流程：

```text
Bob Browser
  -> 打开 Alice OOD / Alice Zone 上托管的 Social App 页面
  -> Bob 点击评论
  -> Alice App 判断 Bob 是否有评论权限
  -> 可选：发起联合登录，确认 Bob 的 UserID
  -> 查询 Alice Zone 中 ContactManagement / Friend List / Block List
  -> 若允许评论，则进入内容创建流程
  -> Bob 对评论内容进行签章
  -> 评论提交到 Alice App 的数据服务
  -> 其他用户读取 Thread 时验证评论签名
```

这里有两个相互独立的安全问题：

1. **Bob 能不能在 Alice 的系统中发表评论**：这是登录、好友关系、Block List、权限策略问题。
2. **这条评论是不是 Bob 真实发布的**：这是内容签名和可验证性问题。

联合登录只能证明访问者是谁，不能自动证明内容是由 Bob 签过的。签章流程则证明内容确实由 Bob 的钱包或 Bob 绑定的 Zone 生成。

如果 Bob 既没有浏览器钱包，也没有可代表他的绑定 Zone 私钥，那么他就无法发布需要签章的内容。

---

## 12. 主要攻击面与防护策略

| 攻击面 | 风险 | 防护策略 |
|---|---|---|
| Root Key 泄露 | 攻击者获得 Zone 最高权限及链上更新能力 | 钱包保存；助记词离线备份；Root Key 不入 Zone；关键操作二次确认 |
| Web Signer 私钥文本泄露 | 被密码破解或钓鱼导入后可签名 | 标注高风险；仅兼容 / 开发使用；强密码；尽量推荐钱包路径 |
| VerifyHub 私钥泄露 | 可伪造登录态，可能达到 Administrator 权限 | 高安全区域保存；周期轮换；短 JWT TTL；明确最大授权级别 |
| SystemConfig RBAC 被写入 | 任意提权，权限系统失效 | 写权限最小化；Root / Admin / Kernel 路径控制；审计日志 |
| SystemConfig 敏感区被读取 | TLS / VerifyHub 私钥泄露 | 高安全区域防读；Host OS 权限隔离；按组件拆分用户组 |
| System Service 漏洞 | 越权读取系统或多用户数据 | 统一鉴权库；资源路径规范；低权限运行；服务间隔离 |
| 恶意 App | 读取用户数据、跨用户泄露、诱导授权 | 容器隔离；Manifest 权限；安装前信誉；运行时可收缩权限 |
| Client Device 被攻破 | 冒用客户端身份访问服务 | Client 权限低于 Node；设备撤销；Gateway 层识别和限制 |
| Node Device 被攻破 | 可能影响本节点承载服务 | 关键内核节点严格管理；不要默认信任所有节点；敏感密钥最小分发 |
| Zone Gateway TLS 私钥泄露 | 公网身份伪造、中间人攻击 | 仅授权 Gateway 可读；证书轮换；高安全存储 |
| 多管理员冲突 | 长任务或系统状态互斥，造成分布式一致性问题 | 当前先限制单 Administrator；未来引入锁、事务和审批流 |

---

## 13. 仍需决策的问题

1. **VerifyHub 私钥的最大权限边界**  
   泄露后到底等同 Administrator，还是只等同 Normal User？这是安全与体验的核心权衡。

2. **Administrator 操作是否必须 Root Key 二次签名**  
   安全性更好，但会增加大量操作摩擦。可考虑分级：高风险操作必须 Root，低风险操作允许 VerifyHub。

3. **多 Administrator 是否支持**  
   当前倾向只允许一个 Administrator。多管理员在分布式系统和长任务场景下会引入互斥、冲突和状态协调问题。

4. **System Service 是否容器化**  
   当前系统服务可不跑 Docker，但这增加 Host OS 层攻击面。未来可评估部分系统服务容器化或沙盒化。

5. **家庭共享与用户隐私的边界**  
   默认应按用户隔离，但家庭场景确实存在共享需求。建议通过显式权限而不是默认打通。

6. **Zone Signature 的适用范围**  
   哪些内容可以接受 Zone 代表用户签名，哪些必须要求 Global DID Root Signature，需要产品和协议共同定义。

7. **TLS 证书获取器与存储器分离**  
   不同域名 / DID / TID 解析路径需要不同证书获取策略，但证书私钥的存储和授权读取应统一。

8. **Kernel Component 边界**  
   k-event、k-message queue、TaskManager、Workflow 是否全部属于 Kernel，需要按依赖关系和安全面继续收敛。

---

## 14. 建议的工程落地清单

为了把以上设计变成可实现、可测试、可审计的系统，建议形成以下工程规格文档：

1. `BuckyOS Terminology.md`  
   统一 Zone、Node、OOD、DID、BNS、SystemConfig、Gateway、VerifyHub 等术语。

2. `BuckyOS Key Inventory.md`  
   列出所有密钥类型、生成方式、保存位置、轮换策略、泄露影响和恢复方式。

3. `BuckyOS JWT and Identity Claims.md`  
   定义 JWT issuer、subject、UserID、AppID、scope、ttl、audience、nonce 等字段。

4. `BuckyOS RBAC Policy Spec.md`  
   定义 Policy 表结构、Resource Path 命名规范、Action 集合、Allow / Deny 优先级。

5. `BuckyOS SystemConfig Secure Area.md`  
   定义高安全区域的读写控制、本地文件权限、备份、加密、迁移和审计机制。

6. `BuckyOS Gateway Security Spec.md`  
   定义 Zone 内流量、Client 设备流量、公网流量、SN Relay、TLS、设备身份验证流程。

7. `BuckyOS App Sandbox and Manifest.md`  
   定义 App Manifest 权限、容器隔离、跨用户访问权限、安装与卸载安全规则。

8. `BuckyOS Cross-Zone Signature Spec.md`  
   定义 Global DID 签名、Zone 签名、内容哈希、Thread 评论验证、App 发布签名等协议。

9. `BuckyOS Security Audit Checklist.md`  
   为 Kernel、System Service、App、Gateway、SystemConfig 提供代码审计和运行时审计清单。

---

## 15. 总结

BuckyOS Security 机制的核心不是单个密钥，而是一套围绕 **Zone** 建立的分布式信任系统：

- Root Key 是最高权限，但必须保持在 Zone 外。
- VerifyHub 提供日常认证体验，但其权限边界和密钥轮换必须严格设计。
- SystemConfig 是权限和配置事实源，必须同时防止非法写入和敏感读取。
- Gateway 是流量和设备身份边界，负责把 Zone 内外流量隔离开。
- RBAC Policy 是统一鉴权中心，所有系统服务都必须遵守。
- App 默认不可信，应容器化、按用户隔离，并通过 Manifest 和 Policy 授权。
- Global DID 与 Zone 账号要清晰分离，使用户可跨多个 Zone 保持长期身份。
- 跨 Zone 内容需要签名，且应区分 Root Signature 与 Zone Signature 的信任等级。

这套设计的最终目标，是让普通用户也能拥有一个可用、可恢复、可扩展、可跨 Zone 协作的个人分布式系统，同时避免把最高权限密钥暴露在日常运行环境中。

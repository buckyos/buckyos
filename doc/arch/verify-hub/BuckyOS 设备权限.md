# BuckyOS 设备权限思考

> 状态：讨论稿  
> 来源：基于设备权限相关口述讨论整理  
> 说明：OOD、Gateway、Node、Client Device 部分主要来自现有架构；Sensor / IoT 部分属于尚未落地的架构推演。文中的“建议”不代表已经完成实现。

## 1. 问题背景

BuckyOS 当前的权限体系主要站在“资源使用与申请”的角度工作：当外部主体操作某个资源时，系统按请求（per request）识别请求对应的用户和应用，再判断该操作是否被允许。

传统模型可以近似写成：

```text
PermissionDecision = f(UserID, AppID, Resource, Operation)
```

这在单机操作系统中很自然。资源、进程和用户都发生在同一台机器上，设备通常只是执行环境，不是独立的权限主体。

但 BuckyOS 是分布式操作系统。一个 Zone 由多台硬件设备共同组成，不同设备在系统中承担不同职责、拥有不同信任级别，也可能运行完全相同的系统程序。因此，仅靠 `UserID + AppID` 已不足以表达真实的安全边界。

最直接的例子是 NodeDaemon：

- 每台设备都运行 NodeDaemon；
- 不同设备上的 NodeDaemon 是同一个程序；
- 当前它们的 `AppID` 都可以是 `kernel`；
- 但运行在 OOD、Gateway、Node、Client Device 上的 NodeDaemon 显然不应拥有相同权限。

历史实现中，为了适配原有 RBAC 接口，曾使用：

```text
AppID  = kernel
UserID = DeviceID
```

这种做法在工程上能够工作，但它把设备身份临时塞进了用户字段，掩盖了“设备本身也是权限主体”这一事实。

本文试图回答以下问题：

1. BuckyOS 中设备应如何分类？
2. DeviceID 是否应成为权限判断的一等输入？
3. Device、User、App 三类身份应如何组合？
4. Client Device 的 owner 与实际登录用户是什么关系？
5. Sensor / IoT 设备应该拥有哪些最小权限？
6. System Config 应承担什么、不应承担什么？

---

## 2. 当前得到的核心结论

### 2.1 DeviceID 必须进入权限判定

设备身份不能只是注册信息或审计字段。对于分布式系统，它必须参与实际授权：

```text
PermissionDecision =
    f(DeviceID, UserID, AppID, Resource, Operation)
```

原因不是“设备等于用户”，而是：

> 相同应用在不同设备上运行时，设备角色和设备组是区分其系统权限的关键依据。

### 2.2 DeviceID 不应长期伪装成 UserID

长期模型中应把以下身份分开：

```text
Device Principal
User Principal
App / Service Principal
```

短期为了兼容，接口可以继续接收设备 DID，但至少要保留明确的 principal kind；中长期应把 `UserID` 扩展为独立的 `DeviceID`、`UserID` 和 `AppID`，或引入带类型的 `Principal`。

### 2.3 默认是多重约束，而不是权限并集

普通资源访问的安全默认值应是“所有适用约束同时满足”：

```text
allow =
    DevicePolicyPassed
    AND AppPolicyPassed
    AND UserPolicyPassed
    AND ResourcePolicyPassed
```

如果把设备、应用、用户权限直接求并集，则任何一个身份拥有的高权限都可能把其他维度的限制冲掉，风险过大。

但也不应把所有请求机械地固定为三者集合的简单交集。部分资源天然只依赖设备身份，部分系统服务没有用户身份，另一些策略需要判断组合条件，例如：

```text
AppID == kernel
AND DeviceID ∈ kernel_devices
```

因此，更准确的表达是：

```text
allow = Policy.evaluate(RequestContext)
```

由资源策略明确声明哪些身份维度适用，以及它们之间是“与”、条件匹配，还是经显式授权后的例外。

### 2.4 用户可以覆盖“默认 owner 绑定”，但不能抹掉设备身份

Client Device 可以默认把 owner 作为请求用户，但当用户提供了明确、可验证的用户凭证时，可以把实际操作用户切换成该认证用户。

应当允许：

```text
DefaultUser = DeviceOwner
AuthenticatedUser = Admin
EffectiveUser = Admin
```

但不应当变成：

```text
DeviceID 被 Admin 身份覆盖或删除
```

设备限制仍然必须参与判定。需要突破 owner 权限上限时，应使用范围明确、可过期、可审计的临时授权或“break-glass”凭证，而不是隐式提升。

---

## 3. 设备分类与角色关系

设备分类中，有些身份可以合并，有些身份互斥。

当前讨论中明确的关系是：

- OOD 与 Gateway 可以由同一台设备同时承担；
- Gateway 也可以独立存在，不一定是 OOD；
- Node 与 OOD / Gateway 互斥；
- 一台设备一旦选择成为普通 Node，就明确不承担 OOD 或 Gateway 的核心职责。

为了避免把“类别”和“可叠加角色”混在一起，建议采用两层表达：

```text
DeviceClass:
    KernelDevice
    Node
    ClientDevice
    Sensor

KernelRole:
    OOD
    Gateway
```

其中：

```text
KernelDevice.kernel_roles ⊆ { OOD, Gateway }
KernelDevice.kernel_roles != ∅
```

即一台 Kernel Device 可以是：

```text
OOD
Gateway
OOD + Gateway
```

而 Node 不与上述 KernelRole 叠加。

### 3.1 总览

| 设备类别 | 核心定位 | 主要能力 | 主要限制 |
|---|---|---|---|
| OOD | System Config / 分布式内核根状态节点 | 参与核心状态维护，保证系统最小可用性 | 不应被当作普通调度 Node |
| Gateway | Zone 的公网入口与跨网络协调节点 | HTTP/HTTPS 入口、跨 NAT 连接、持有对应入口密钥 | 与 SN 语义不同；不是普通 Node |
| Node | 可信服务承载节点 | 上报资源，接受 Scheduler 调度，运行服务或多数 Kernel Service | 不承载 System Config，也不承担 Gateway 核心角色 |
| Client Device | 用户侧服务消费者 | 以设备证书进入 Zone，发现并直连服务，建立 RTCP 隧道或 VPN | 默认权限不应超过 owner；不作为系统负载宿主 |
| Sensor / IoT | 受限资源提供者 | 声明接口、提供状态、在最小范围内写自身信息或发事件 | 不应拥有广泛系统写权限，也不宜承担完整复杂 RBAC |

---

## 4. Kernel Device

### 4.1 OOD

OOD 沿用既有体系中的术语。在 BuckyOS 中，它承担传统分布式系统中控制面核心节点的职责。

OOD 的中心任务是维护 System Config。整个系统以成功连接并读取 System Config 作为关键启动标志。多个 OOD 共同维护高可用的核心状态，目标是以 `2N + 1` 个节点容忍 `N` 个节点故障。

一致性算法可能受到跨网络、跨 NAT、延迟和硬件条件影响，未必最终采用传统 Raft；这一点属于分布式一致性设计，不在本文定案。对权限模型而言，关键事实是：

> OOD 是 System Config 的受信任物理承载者和核心状态维护者。

因此，只有 OOD 角色可以参与 System Config 的核心维护流程。

### 4.2 Gateway

Gateway 同时承担两类职责：

1. **Zone 外部入口**  
   外部 HTTP/HTTPS 流量通过 DNS 到达 Zone 后，通常由 Gateway 进入系统。

2. **跨网络协调与转发**  
   位于不同 NAT 后的 OOD 或其他设备需要互联时，可以通过用户自建 VPS 上的 Gateway 协调，而不必完全依赖中心化基础设施。

Gateway 还可能持有 Zone 对外服务所需的 HTTPS 证书或对应私钥，因此它属于 Kernel Device，而不是普通中转节点。

### 4.3 Gateway 与 SN 的区别

SN 是外部协调器，可以帮助设备发现、建链或穿透，但它不因此成为 Gateway：

```text
SN != Gateway
```

主要区别包括：

- SN 不属于该 Zone 的 Kernel；
- SN 不因提供协调能力而持有 Zone 的 HTTPS 证书；
- OOD 之间可以通过 SN 互联，也可以通过 Gateway 互联；
- “可以协助连接”不等于“拥有 Zone 内核权限”。

### 4.4 Kernel Device 的权限不是无差别超级权限

OOD 和 Gateway 都属于 Kernel Device，但仍应按具体职责细分资源权限。例如：

- System Config 共识与根状态维护：要求 OOD 角色；
- Zone 对外 HTTPS 私钥：要求 Gateway 角色；
- 某些共同 Kernel Secret：可以要求 `kernel_devices` 组；
- NodeDaemon 的普通设备注册更新：只要求设备自身权限。

因此，`KernelDevice` 是高信任级别，不代表所有 Kernel Device 对所有核心资源无条件等权。

---

## 5. Node：可信服务承载设备

### 5.1 Node 的启动与注册流程

Node 不参与 Kernel 的持续维护。它的典型启动流程是：

```text
启动
  -> 等待连接 System Config
  -> 携带设备证书注册或更新自身
  -> 上报资源与状态
  -> 等待 Scheduler 分配负载
```

设备不能未经授权自动加入系统。系统管理员或 Root User 先离线签发设备证书，设备再凭证书自行完成上线。

一个可行的注册语义是：

```text
if device_path 不存在:
    验证证书后创建设备记录
else if 证书有效 and 设备未被吊销:
    更新该设备记录
else:
    拒绝
```

这一流程服务于大规模组织运营：

- 管理者可以批量签发证书；
- 设备无需逐台人工在线添加；
- 只要设备名称或路径不冲突，就可以自行注册；
- 吊销属于异常控制流程。

### 5.2 设备证书的信任含义

在当前模型下，管理员愿意给 Node 签发设备证书，隐含表示系统接受：

- 该设备的身份；
- 该设备作为 Zone 成员；
- 该设备上报的资源状态在当前信任模型下可用于调度；
- 该设备可以承载系统分配的负载。

这是一种运营信任，而不一定等同于硬件级远程证明。未来若引入 TPM、Secure Enclave 或远程证明，可以进一步把“身份可信”和“运行状态可信”分开。

### 5.3 Node 能承载什么

Node 是可信的 Service Host。Scheduler 可以将多种负载放到 Node 上，包括：

- 普通应用服务；
- 系统基础服务；
- 多数可调度的 Kernel Service；
- 无状态或可重建的系统组件。

但 Node 明确不承载两类确定性 Kernel 核心角色：

- System Config；
- Gateway。

因此：

```text
Node ≠ OOD
Node ≠ Gateway
```

Node 可以是 Kernel Service 的运行容器，但不是 Kernel Root 的物理承载者。

### 5.4 Kernel 与 Kernel Service

当前讨论形成了一个有用的工作区分。

#### Kernel

系统最小可用性所依赖、必须在部署或启动阶段确定存在的核心部分，例如：

- System Config；
- Gateway；
- 负责恢复关键调度组件的 OOD 决策逻辑。

#### Kernel Service

属于系统基础设施，但允许短暂单点故障，通常可由 Scheduler 放置，例如：

- Scheduler；
- WorkflowHub / VerifyHub；
- SLog / KLog；
- Message Queue；
- Task Manager；
- DFS 等基础服务。

这些服务的共同特点是：

- 可能被 Kernel 或其他基础服务依赖；
- 但不必都成为 System Config 同等级别的强一致核心；
- 状态可以写回 System Config 或其他可靠存储；
- 进程本身通常可重启、重建或迁移。

Scheduler 是特殊的 Kernel Service：首次调度必须有人把它拉起来，但完成首次调度后，其结果已经写回 System Config。Scheduler 暂时停止时，已调度负载仍可继续运行，系统主要损失的是对新资源和新负载变化的响应能力。

---

## 6. Client Device：带 owner 的服务消费者

### 6.1 核心用途

Client Device 解决的是用户终端安全访问 Zone 内服务的问题。

例如，用户希望在笔记本电脑上访问位于家庭网络或私有集群中的 Samba/SMB 文件服务。SMB 不适合直接暴露公网，虽然可以通过 Gateway 做端口映射，但这只应作为应急方案。

推荐路径是：

```text
Client Device
  -> 使用设备证书连接 System Config
  -> 发现目标服务所在 Node
  -> 使用 RTCP 建立点到点隧道
  -> 在本机映射为 SMB、VPN 或其他本地接口
```

### 6.2 从 Zone 外到 Zone 内

Client Device 第一次启动时，对系统拓扑一无所知，可能先通过 Gateway 连接 System Config。

一旦完成设备认证并获得系统拓扑，它就成为 Zone 内的已认证设备，后续原则上可以通过 RTCP 点到点直连目标设备或服务，而不必让全部流量继续绕过 Gateway。

因此 Gateway 主要承担：

```text
Zone 外 -> Zone 内
```

而已认证 Client Device 后续可以走：

```text
Zone 内设备 -> Zone 内设备
```

### 6.3 Owner 绑定原则

Client Device 的证书应明确 owner，例如：

```text
Alice-MacBook
owner = Alice
```

默认安全原则是：

```text
ClientDevice.EffectivePermission
    ⊆ DeviceOwner.Permission
```

其含义是：设备证书证明“这个设备被 Alice 授权加入系统”，不能自然推导出设备拥有超过 Alice 的权限。

对于新协议，可以把 owner 作为默认用户，从而减少重复登录：

```text
Device Certificate
    -> Device Owner
    -> Default User
```

但这只是默认绑定，不是永远不可改变的强绑定。

### 6.4 传统协议的双层认证

Samba/SMB 自带用户名和密码，因此一条访问链路可能经历多层判断：

```text
1. RTCP 层验证 Client Device 身份
2. Samba 服务判断该设备是否允许连接
3. Samba 协议验证用户名和密码
4. Samba 服务以 UserID + AppID(Samba) 请求 DFS
5. DFS 对最终文件资源做权限判断
```

这时可能出现：

```text
DeviceOwner = Alice
SMBUser     = Bob
```

传统协议允许这种情况，新协议则可以默认将 owner 与用户绑定。

### 6.5 显式用户切换与临时提权

必须保留一个现实场景：管理员自己的电脑损坏，但需要借用家庭成员的笔记本修复系统。

此时：

- 设备链路是可信的；
- 设备 owner 不是管理员；
- 管理员可以在链路上提供自己的确定性凭证；
- 系统应允许把实际操作用户切换为管理员。

推荐语义是：

```text
device_id       = FamilyMember-Laptop
device_owner_id = FamilyMember
actor_user_id   = Admin
```

而不是把设备身份覆盖掉。

如果 Client Device 默认受 owner 权限上限约束，则管理员需要额外的临时委托或 break-glass 能力。该能力至少应具备：

- 明确作用域；
- 明确操作或资源范围；
- 短期有效；
- 可撤销；
- 完整审计；
- 不永久改变设备 owner。

### 6.6 多用户终端问题

Client 侧 Gateway / Agent 在不同操作系统上的运行方式可能不同：

- Windows 上可能是普通用户进程；
- macOS 上可能是系统服务；
- 系统服务所在设备本身又可能有多个本地用户。

因此，`DeviceOwner`、`LocalSessionUser` 与 `AuthenticatedBuckyOSUser` 不能隐式视为同一个概念。多用户设备上需要明确建立“本地会话到 BuckyOS 用户”的绑定，并避免系统服务把一个本地用户的凭证复用给另一个本地用户。

### 6.7 VPN 场景

手机等 Client Device 还可以通过 RTCP 连接某个 Node 上的 VPN 服务，使手机表现得像接入该 Node 所在 LAN。

这是一种可选产品能力，不改变 Kernel 权限模型。系统仍应优先鼓励服务级直连，而不是强制所有 Node 都通过 VLAN 合并成一个大二层网络。

---

## 7. Sensor / IoT：尚未落地的架构推演

### 7.1 定位

Sensor 不是负载宿主，也不是普通用户终端，而是资源或接口提供者。例如：

- 摄像头；
- 红外传感器；
- 温度传感器；
- 门锁；
- 智能插座。

设备加入系统后，可以在 DeviceInfo 中声明自身能力和接口，并在运行时维护实时状态，供授权服务读取或订阅。

### 7.2 Push 与 Pull 的权限差异

#### Pull 模型

```text
IoT Hub / Service -> Sensor API
```

优点：

- Sensor 不必主动写业务系统；
- 权限主要集中在读取方；
- 业务状态由 IoT Hub 管理。

问题：

- 简单实现可能退化成轮询；
- Sensor 必须判断读取请求是否来自有效主体；
- 权限验证逻辑被下放到能力有限、系统不可控的设备上。

#### Push 模型

```text
Sensor -> Configured Target / Event Endpoint
```

优点：

- Sensor 最了解自身状态；
- 事件可以即时上报；
- 不需要持续轮询。

问题：

- Push 本质是写操作；
- 写操作必须有明确权限；
- 若让 Sensor 直接写任意 System Config 或业务资源，会扩大攻击面。

### 7.3 Sensor 的最小权限

当前推演倾向于只给 Sensor 极小的系统写权限：

```text
1. 更新自己的 DeviceInfo
2. 在自己拥有的 KeyEvent 路径下 fire event
3. 向经过配置的目标服务提交有限事件或状态
```

例如：

```text
/devices/{device_id}/info
/keyevents/devices/{device_id}/...
```

不应允许：

```text
写其他设备信息
写任意 System Config 路径
直接修改用户资源
直接写任意应用状态
```

### 7.4 DeviceInfo 不能成为高频遥测数据库

每个设备可以更新自己的 DeviceInfo，这是合理的基础权限。但如果 Sensor 把每次温度变化、动作检测或摄像头状态都写入 DeviceInfo，System Config 就会退化为应用数据服务。

System Config 更接近：

```text
Registry
Control Plane State
Kernel Coordination State
```

它应以低频写、稳定读取和表达系统最终状态为主，而不是承担：

```text
Telemetry Stream
Event Log
High-frequency Sensor State
```

因此 DeviceInfo 更适合保存：

- 设备类型；
- 能力声明；
- 接口描述；
- Endpoint；
- 固件版本；
- owner；
- 低频在线状态；
- 低频配置或健康状态。

高频数据应进入 IoT Hub、消息系统、事件系统或专用应用存储。

### 7.5 IoT Hub 的作用

更合理的结构是：

```text
Sensor
  -> 声明能力
  -> 与 IoT Hub 握手
  -> Push 事件或响应 Pull
  -> IoT Hub 负责缓存、规则、存储和复杂权限
```

这样，Sensor 只需要实现最小身份认证和有限授权，复杂 RBAC 由可控的软件服务承担。

---

## 8. DeviceID 作为一等权限主体

### 8.1 为什么 AppID 不够

所有设备上的 NodeDaemon 都可以使用：

```text
AppID = kernel
```

如果授权只看 AppID，那么普通 Node 上的 NodeDaemon 和 OOD 上的 NodeDaemon 将无法区分。

假设 System Config 中保存一个核心私钥，只允许 Kernel Device 访问，则策略应至少是：

```text
AppID == kernel
AND DeviceID ∈ kernel_devices
```

普通 Node 即使运行同一个 NodeDaemon，也会因为 DeviceID 不在 Kernel Device 组而被拒绝。

### 8.2 逻辑隔离与物理隔离

核心密钥应同时依赖：

1. **逻辑权限隔离**  
   非 Kernel Device 的 DeviceID 无法通过授权。

2. **物理放置隔离**  
   System Config 和对应密钥不运行、不落盘在普通 Node 上。

这形成：

```text
Permission Isolation
+
Physical Placement Isolation
```

两层防护。

### 8.3 DeviceID 应保留到最终资源服务

设备身份不应只在 RTCP 握手时验证完就丢弃。后续调用链应把经过认证的 DeviceID 以不可伪造的方式传递到最终执行授权的服务。

资源服务必须知道：

- 请求来自哪台设备；
- 设备属于什么类别或设备组；
- 请求代表哪个用户；
- 请求由哪个应用或服务发出；
- 是否带有委托或临时提权凭证。

不能只相信普通 HTTP Header 中由上游随意填入的 `DeviceID`。跨服务传播的 RequestContext 应由可信入口签名、封装或通过受保护的内部通道传递。

---

## 9. 完整的，可用于权限验证的 RequestContext

```text
RequestContext {
    request_id

    device_id
    device_class
    device_roles
    device_groups
    device_owner_id

    actor_user_id
    authenticated_user_id
    authentication_strength

    app_id
    service_id
    workload_id

    delegation_id
    elevation_scope
    elevation_expire_at

    resource
    operation
}
```

字段含义：

- `device_id`：实际发起链路的设备，始终保留；
- `device_class / roles / groups`：从可信系统状态解析，不由客户端自报决定；
- `device_owner_id`：设备的默认 owner；
- `actor_user_id`：本次请求实际代表的用户；
- `authenticated_user_id`：显式认证得到的用户；
- `app_id / service_id`：发起业务操作的应用或服务；
- `delegation / elevation`：范围明确的委托或临时提权；
- `resource / operation`：最终被访问资源和操作。

对于纯系统服务调用，`actor_user_id` 可以为空；对于 Client Device 的默认请求，可以由受信任入口按策略将 `actor_user_id` 设为 `device_owner_id`。

---

## 10. 权限组合模型

### 10.1 默认规则

对普通资源访问，建议采用“适用约束全部通过”的默认规则：

```text
allow =
    ResourcePolicy.matches(
        device,
        app_or_service,
        user,
        delegation,
        operation
    )
```

或从权限集合角度近似表达为：

```text
EffectivePermission =
    DeviceConstraint
    ∩ AppConstraint
    ∩ UserConstraint
    ∩ DelegationConstraint
```

其中某个维度不适用时，不应构造一个虚假身份，而应由策略明确标记该维度可缺省。

### 10.2 不建议默认求并集

以下做法风险很高：

```text
EffectivePermission =
    DevicePermission
    ∪ AppPermission
    ∪ UserPermission
```

因为只要其中一个主体权限很高，就可能突破其他主体的限制。

### 10.3 组合策略比三次独立查表更重要

实际策略经常依赖身份组合，而不是三次独立求值。例如：

```text
allow read kernel_secret when
    app == kernel
    and device in kernel_devices
```

或：

```text
allow update device_info when
    target_device == request.device_id
    and app == kernel
```

因此实现上更适合由统一的 Policy Decision Point 接收完整 RequestContext，再返回允许、拒绝和约束结果，而不是让每个服务随意决定“先查 App，再查 User，再查 Device，最后取交集还是并集”。

---

## 11. 典型授权流程

### 11.1 NodeDaemon 更新自身信息

```text
DeviceID = node-17
AppID    = kernel
Operation = update
Resource  = /devices/node-17/info
```

策略：

```text
AppID == kernel
AND Resource.device_id == Request.device_id
```

NodeDaemon 只能更新自己的设备信息，不能更新其他设备。

### 11.2 OOD 读取核心密钥

```text
DeviceID = ood-1
AppID    = kernel
Resource = /system/secrets/kernel-key
```

策略：

```text
AppID == kernel
AND DeviceID ∈ kernel_devices
AND DeviceRole 满足该密钥的具体要求
```

普通 Node 上同样的 NodeDaemon 会被拒绝。

### 11.3 Client Device 访问 SMB 文件

```text
DeviceID       = alice-macbook
DeviceOwner    = alice
AuthenticatedUser = alice
AppID          = samba
Resource       = /dfs/home/alice/document
```

授权链：

```text
设备证书有效
AND 设备允许连接 Samba
AND Samba 用户认证成功
AND AppID=samba 对目标 DFS 路径有访问资格
AND UserID=alice 对目标文件有访问资格
```

### 11.4 借用设备进行管理员修复

```text
DeviceID       = bob-laptop
DeviceOwner    = bob
AuthenticatedUser = admin
AppID          = repair-tool
Delegation     = scoped-break-glass-token
```

策略：

```text
设备仍需满足最低可信要求
AND 管理员认证强度满足要求
AND 临时授权未过期
AND 操作在授权作用域内
AND 全程审计
```

这里是“显式用户与临时授权覆盖默认 owner 绑定”，不是“Admin 身份抹去 bob-laptop 的 DeviceID”。

### 11.5 Sensor 发出自身事件

```text
DeviceID = pir-living-room
AppID    = device-agent
Operation = fire
Resource  = /keyevents/devices/pir-living-room/motion
```

策略：

```text
Resource.device_id == Request.device_id
AND event_path 在该设备自有命名空间内
```

---

## 12. 安全不变量

后续实现应尽量保持以下不变量：

1. **相同 AppID 不等于相同权限。**  
   `AppID=kernel` 不能让所有设备自动获得 Kernel 权限。

2. **DeviceID 必须贯穿请求链。**  
   用户显式登录后也不能把设备身份丢掉。

3. **Client Device 默认不超过 owner 权限。**  
   超出 owner 的能力必须有显式用户认证和额外授权。

4. **提权必须是可证明的例外。**  
   不允许因为链路已经建立就隐式获得管理员权限。

5. **Node 与 Kernel Device 的边界是物理和逻辑双重边界。**  
   Node 不承载 System Config / Gateway，也不能通过同名 NodeDaemon 访问其核心密钥。

6. **设备只能默认修改自己的设备记录。**  
   自身路径以外的写操作必须有额外策略。

7. **System Config 不是高频业务数据库。**  
   Sensor 遥测、事件流和应用状态应进入专用服务。

8. **设备组和角色来自可信状态。**  
   客户端不能通过自报 `device_role=OOD` 获得权限。

9. **吊销必须优先于已有注册。**  
   已吊销设备即使持有旧证书，也不能继续更新或接入。

10. **授权决策必须可审计。**  
    至少记录 Device、User、App、资源、操作、策略结果和委托信息。

---

## 13. 兼容现有实现的迁移建议

### 阶段一：明确类型

现有接口若只能接收一个 `UserID`，至少将其概念改为：

```text
Subject {
    kind: user | device | service
    id: DID
}
```

避免仅凭字符串格式猜测它是用户还是设备。

### 阶段二：引入独立 DeviceContext

在所有系统服务调用中增加经过认证的：

```text
device_id
device_groups
device_roles
```

保留现有 UserID / AppID 逻辑，同时让关键资源开始强制检查 DeviceID。

优先迁移：

- System Config；
- Kernel Secret；
- DeviceInfo；
- KeyEvent；
- Node 资源上报；
- 调度与负载控制接口。

### 阶段三：统一 RequestContext 与策略入口

将 Device、User、App、Delegation 统一交给一个策略评估接口，避免每个服务自行决定求交、求并或覆盖关系。

### 阶段四：去除 `UserID = DeviceID` 的语义歧义

最终接口中：

```text
device_id != user_id
```

设备调用没有用户时，`user_id` 可以为空；需要表达系统服务身份时使用 `service_id / app_id`，而不是伪造用户。

---

## 14. 仍需决策的问题

### 14.1 设备模型

- 是否正式采用 `DeviceClass + RoleSet` 两层模型？
- Client Device 与 Sensor 是否永远互斥，还是未来允许复合设备？
- Gateway-only 设备与 OOD+Gateway 设备的密钥范围如何区分？

### 14.2 权限语义

- 哪些资源只看 Device？
- 哪些资源要求 Device + App？
- 哪些资源要求 Device + App + User？
- 是否需要支持基于组合条件的 ABAC，而不只依赖传统 RBAC 组？

### 14.3 用户覆盖与提权

- 显式用户登录是否只替换默认 owner，还是可以突破设备 owner 上限？
- Break-glass 凭证由谁签发？
- 最大有效期、作用域和审计要求是什么？
- 设备本身不可信时，管理员凭证是否仍允许通过该设备执行高危操作？

### 14.4 Client 多用户

- 系统服务模式下，如何绑定本地 OS 会话与 BuckyOS 用户？
- 凭证如何隔离，避免跨本地用户泄漏？
- 一个设备证书是否只允许一个 owner，还是支持组织、家庭和共享设备？

### 14.5 Sensor

- 最终优先 Push、Pull，还是事件订阅？
- KeyEvent 是否适合承担 Sensor 的轻量事件出口？
- DeviceInfo 的低频写入上限和字段边界是什么？
- Sensor 是否必须实现本地权限判断，还是只接受预配置 IoT Hub？
- 不具备通用操作系统的设备如何安全保存设备私钥和完成升级？

### 14.6 证书与吊销

- 设备证书的签发链、轮换和过期策略是什么？
- 离线设备如何及时获得吊销信息？
- 设备重装、转让和 owner 变更时如何处理原 DeviceID？

---

## 15. 暂定设计判断

本轮讨论可暂时收敛为以下判断：

```text
1. DeviceID 是 BuckyOS 权限系统的一等输入。
2. DeviceID、UserID、AppID 应分开建模。
3. OOD 与 Gateway 可组合；Node 与二者互斥。
4. Node 是可信服务宿主，但不是 Kernel Root。
5. Client Device 默认代表 owner，权限不应自然超过 owner。
6. 显式用户凭证可以切换实际用户，但不能抹掉设备限制。
7. 超越 owner 的操作需要显式、短期、可审计的授权。
8. Sensor 只应拥有自身信息、自有事件路径和指定目标的最小写权限。
9. System Config 只承载控制面与低频最终状态，不承载高频业务数据。
10. 核心系统服务必须基于完整 RequestContext 做多主体授权。
```

最核心的一句话是：

> 在 BuckyOS 中，请求不是单纯由“某个用户通过某个 App”发起，而是由“某台设备上的某个 App，在某个用户或服务身份下”发起。设备身份不能被用户身份替代，也不能在进入系统后被丢弃。

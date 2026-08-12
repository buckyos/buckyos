# BuckyOS Booting 阶段 resolve-did-document 的特殊处理说明

## 1. 背景

在 BuckyOS 中，`node_daemon` 在启动阶段最重要的首要工作之一，是解析当前节点所在 Zone 的 **Zone Document**。

Zone Document 对 `node_daemon` 来说不是普通配置，而是决定整个节点行为的核心运行配置。它描述了 Zone 的关键拓扑与运行结构，例如：

- 当前 Zone 中有哪些 OOD / Node；
- 不同节点在 Zone 中承担什么角色；
- OOD 之间是否需要组成共识集群；
- 当前节点应该以什么方式加入系统；
- 后续系统服务应该如何启动。

因此，Booting 阶段的 resolve-did-document，尤其是 resolve 自己的 Zone Document，并不是一次普通的 DID Document 查询，而是整个系统启动链路中的关键自举步骤。

---

## 2. 标准 DID Resolve 流程的隐含前提

标准 DID Resolve 流程通常隐含一个前提：

> 被 resolve 的对象已经存在，并且其所属系统已经正常运行。

也就是说，当我们 resolve 别人的 DID Document 时，流程大致是：

```text
Client
  │
  ▼
Resolver
  │
  ▼
Authority / Network / Cache
  │
  ▼
DID Document
  │
  ▼
Verify
  │
  ▼
Return Result
```

这个流程假设：

- Resolver 已经可以正常工作；
- 目标对象的权威源已经可访问；
- 网络路径已经建立；
- 系统已经处于 Runtime 状态；
- 调用方只需要拿到并验证 DID Document。

因此，标准 DID Resolve 是一个 **Runtime 行为**。

---

## 3. BuckyOS Booting 阶段的问题：自举问题

当 BuckyOS 的 `node_daemon` 启动时，它需要 resolve 自己所在 Zone 的 Zone Document。

但这里存在一个自举矛盾：

```text
需要 Zone Document
      │
      ▼
才能知道如何启动 Zone

但是

需要 Zone 已经启动
      │
      ▼
标准 Resolver 才能按常规方式 resolve Zone Document
```

换句话说：

> Resolve 别人的 DID Document，是假设别人已经存在；resolve 自己的 Zone Document，则是为了把自己所在的系统跑起来。

因此，Booting 阶段 resolve 自己的 Zone Document 不能被视为标准 Resolve 的简单特例，而应该被视为系统生命周期中的 **Bootstrap 行为**。

---

## 4. Boot Resolve 与 Runtime Resolve 的区别

BuckyOS 中可以将 resolve-did-document 分成两种语义。

### 4.1 Runtime Resolve

Runtime Resolve 用于系统已经正常运行后的普通 DID Document 解析。

其特点是：

- 系统已经启动；
- Resolver 已经可用；
- 网络拓扑已经建立；
- Service Registry / Authority / Cache 等机制可以正常工作；
- 调用方对返回结果如何使用具有开放性。

Runtime Resolve 的职责是：

> 发现并返回一个经过验证的 DID Document。

---

### 4.2 Boot Resolve

Boot Resolve 用于节点启动阶段解析自己的 Zone Document。

其特点是：

- 当前 Zone 可能尚未完全运行；
- Resolver 所依赖的部分系统能力可能尚未建立；
- 当前节点需要依赖最小可信信息恢复系统；
- resolve 的结果会直接驱动节点后续启动行为；
- 返回结果必须满足当前启动状态机的约束。

Boot Resolve 的职责不是简单地获取“最新”文档，而是：

> 获取一个当前节点能够安全接受、并且能够继续维持系统可恢复性的 Zone Document。

---

## 5. 核心设计原则：Resolve 是发现，Accept 是状态机决策

Booting 阶段可以继续复用标准 DID Resolve 的部分能力，例如：

- Document 获取；
- Document 格式检查；
- 签名验证；
- DID 绑定关系验证；
- revision（`iat`）判断。

但 Booting 阶段真正特殊的地方不在于 “resolve” 本身，而在于：

> Resolve 结果不能被无条件采用，必须进入 Boot State Machine 进行判断。

可以抽象为：

```text
Resolve DID Document
        │
        ▼
Verify Document
        │
        ▼
Compare With Current State
        │
        ▼
Can Smoothly Evolve?
        │
   ┌────┴────┐
   │         │
  Yes        No
   │         │
   ▼         ▼
 Accept    Ignore
```

因此，Boot Resolve 的核心原则是：

> Resolver 负责发现，Boot State Machine 负责决定是否接受。

---

## 6. Booting 阶段隐含的系统升级语义

Booting 阶段 resolve Zone Document 并不只是启动初始化，它还隐含了一种系统升级语义。

因为 Zone Document 是可能变化的，例如：

- OOD 数量变化；
- OOD 拓扑变化；
- 共识模型变化；
- 节点角色变化；
- 系统服务布局变化。

但是，BuckyOS 不会因为 resolve 到一个新版本 Zone Document 就立即采用它。

BuckyOS 只接受一种变化：

> 当前运行态能够平滑演进到的新状态。

也就是说，Boot Resolve 的目标不是“拿到最新版本”，而是“拿到一个当前节点能够安全过渡到的版本”。

---

## 7. 平滑升级原则

BuckyOS Booting 阶段的首要升级原则是：

> 只接受可平滑升级的 Zone Document 变化。

例如：

```text
3 OOD Raft Cluster
        │
        ▼
5 OOD Raft Cluster
```

这类变化可以基于 Raft membership change 进行平滑过渡，因此可以被接受。

但下面这种变化不能被视为平滑升级：

```text
1 OOD
  │
  ▼
3 OOD Raft Cluster
```

因为它不仅是节点数量变化，而是整个运行模型发生了质变：

- 从单节点模式进入多节点共识模式；
- Leader Election 语义发生变化；
- Commit Rule 发生变化；
- Quorum 规则发生变化；
- 数据复制和一致性模型发生变化。

这类变化不能通过普通 Boot Resolve 自动完成。

因此，节点在 Booting 阶段如果 resolve 到此类 Zone Document，应当将其视为不可接受结果，而不是强行采用。

---

## 8. 两种 Boot 状态：首次启动与正常重启

Boot Resolve 的处理结果取决于当前节点是否曾经成功启动过。

### 8.1 首次启动 / 初始化启动

首次启动时，节点没有可用的历史运行态。

此时如果无法获得一个可接受的 Zone Document，系统无法继续初始化。

处理逻辑为：

```text
First Boot
    │
    ▼
Resolve Zone Document
    │
    ├── Success + Acceptable
    │         │
    │         ▼
    │    Initialize System
    │
    └── Failed / Invalid / Unacceptable
              │
              ▼
          Boot Failed
```

首次启动时，失败是合理结果，因为系统尚未建立任何 Last Known Good State。

---

### 8.2 正常重启 / Warm Restart

如果节点曾经成功运行过，那么本地已经存在一个可用的历史运行态。

这个状态包括但不限于：

- 上一次成功接受的 Zone Document；
- 上一次成功运行的 OOD membership；
- 上一次共识状态；
- 上一次系统服务布局；
- 已经建立的本地身份和信任根。

此时 resolve Zone Document 的目的不是决定系统能否启动，而是判断是否需要进行一次平滑演进。

处理逻辑为：

```text
Warm Restart
    │
    ▼
Load Last Known Good State
    │
    ▼
Resolve Zone Document
    │
    ├── Success + Smoothly Evolvable
    │         │
    │         ▼
    │    Accept New Document
    │
    └── Failed / Invalid / Not Smoothly Evolvable
              │
              ▼
       Ignore New Result
              │
              ▼
       Continue With Last Known Good State
```

正常重启时，即使 resolve 失败，或者 resolve 到一个不能平滑演进的新 Zone Document，也不应该导致系统不可用。

此时正确行为是：

> 忽略该结果，继续使用 Last Known Good State 启动系统。

---

## 9. Last Known Good State 原则

BuckyOS Booting 阶段应始终遵循 Last Known Good State 原则：

> 一个已经正常运行过的系统，其上一次成功运行状态就是当前最重要的安全堡垒。

新的 Zone Document 必须证明自己能够在不破坏该状态的前提下完成演进。

如果不能证明，则拒绝接受。

也就是说：

```text
Known Good State > Unknown New State
```

这条原则的目的不是保守，而是保证远程系统的最低可用性。

---

## 10. 可恢复性优先：Recoverability First

BuckyOS 可以被视为一种远程操作系统。

它通常没有本地键盘、鼠标和显示器，用户主要通过：

- Browser；
- Web UI；
- Remote API；
- Remote CLI；
- 其他远程管理入口；

与系统交互。

对于这类系统，最严重的问题不是配置错误本身，而是：

> 配置错误导致系统失去最后的远程管理入口。

这类似传统服务器管理中的一个经典事故：错误修改 `iptables` 后，SSH 再也无法连接。

如果是 VPS，用户还可能通过云厂商控制台恢复；如果是裸金属服务器，则可能必须进入机房修复。

BuckyOS 不能假设用户总有物理访问能力，因此 Booting 阶段必须保证：

> 一个已经正常运行的系统，不会因为一次 Zone Document 更新而变成完全不可恢复。

因此，Boot Resolve 的最高优先级不是配置新鲜度，而是系统可恢复性。

---

## 11. 对不可接受 Zone Document 的处理

如果 Booting 阶段 resolve 到的新 Zone Document 存在以下情况，应当拒绝接受，并在 Warm Restart 场景下继续使用 Last Known Good State：

| 情况 | 处理 |
| --- | --- |
| Resolve 失败 | Warm Restart 下忽略；First Boot 下失败 |
| 签名验证失败 | Warm Restart 下忽略；First Boot 下失败 |
| DID 绑定关系不正确 | Warm Restart 下忽略；First Boot 下失败 |
| `iat` 早于 LKGS，或同一 `iat` 对应不同内容 | 作为回滚或 revision 冲突忽略 |
| OOD membership 变化不可平滑处理 | 忽略 |
| 共识模型发生质变 | 忽略 |
| 可能导致系统失去管理入口 | 忽略 |
| 涉及 Root Trust 替换 | 忽略，必须进入维护流程 |

这些情况不应被视为普通错误，而应被视为：

> Resolve 到了一个当前启动状态机不能接受的结果。

---

## 12. Owner Document 的特殊处理

Owner Document 是 BuckyOS Root Trust 的核心。

按照标准 DID 流程，Owner Document 理论上应该从权威源 resolve，例如链上合约、权威服务或其他 DID authority。

但是在 BuckyOS Booting 阶段，这个逻辑不能直接套用。

原因是：

> 对于自己的系统而言，很多时候“权威源”就是系统自己，或者依赖系统已经正常运行。

更重要的是，Owner Document 的变化不是普通配置变化，而是 Root Trust 的变化。

如果 Booting 阶段每次都重新 resolve Owner Document，那么一旦 Owner Document 被更新、误配置、丢失或被恶意替换，可能导致：

- Root Public Key 改变；
- 内部权限系统整体失效；
- ACL / RBAC 判断全部变化；
- 已有系统服务签名无法验证；
- 节点无法判断自己是否仍然属于该 Owner；
- 整个系统进入不可管理状态。

因此，BuckyOS Booting 阶段不应在线 resolve Owner Document。

---

## 13. Boot 阶段只使用本地 node_identity 中的 Owner Document

BuckyOS 的 Owner Document 应在 Activation 阶段写入本地 `node_identity` 文件。

Booting 阶段应机械性地、且仅使用这份本地 Owner Document：

```text
node_identity
     │
     ▼
Owner Document
     │
     ▼
Boot Trust Root
```

也就是说：

> Booting 阶段永远使用激活时建立的 Owner Document，不再重新 resolve Owner Document。

这保证了：

- Root Trust 在 Boot 过程中稳定；
- 系统不会因为外部 Owner Document 变化而自毁；
- 权限体系不会在启动过程中突然崩塌；
- 已经可运行的系统具备可恢复性。

---

## 14. Activation 与 Boot 的职责边界

BuckyOS 应清晰区分 Activation 和 Boot。

### 14.1 Activation：建立身份与信任

Activation 是机器从“没有身份”变成“有身份”的过程。

Activation 阶段可以执行：

- Resolve Owner Document；
- 验证 Owner 身份；
- 写入 `node_identity`；
- 建立本地 Root Trust；
- 初始化节点身份；
- 将节点加入某个 Zone。

Activation 的职责是：

> 建立信任。

---

### 14.2 Boot：恢复已有信任

Boot 阶段不负责重新建立信任，也不负责在线替换 Root Trust。

Boot 阶段只负责：

- 读取 `node_identity`；
- 使用本地 Owner Document 作为 Root Trust；
- resolve Zone Document；
- 判断 Zone Document 是否可接受；
- 恢复或平滑演进系统状态。

Boot 的职责是：

> 在已有信任基础上恢复系统。

因此可以总结为：

> Activation creates trust. Boot restores trust. Runtime evolves state.

或者：

> Boot never creates trust, and Boot never changes trust. Boot only restores previously established trust.

---

## 15. Owner Document 更新必须进入维护模式

虽然从 DID 体系设计上看，Owner Document 可能可以通过链上合约或其他权威接口更新，但这对 BuckyOS 来说不是普通在线升级。

Owner Document 更新属于：

> Root Trust Migration。

它会影响整个系统的根权限、签名验证和内部访问控制，因此不能通过 Boot Resolve 自动完成。

如果确实需要更换 Owner Document，必须进入受控维护流程，例如：

```text
Stop Cluster
     │
     ▼
Maintenance / Recovery Mode
     │
     ▼
Replace node_identity Owner Document
     │
     ▼
Verify New Root Trust
     │
     ▼
Restart Cluster
```

只有通过显式维护命令或恢复模式手工刷写 `node_identity`，新的 Owner Document 才能生效。

这类操作不属于 Smooth Upgrade，也不属于普通 Boot 行为。

---

## 16. 推荐状态机

Booting 阶段可以抽象为如下状态机：

```text
Start
  │
  ▼
Load node_identity
  │
  ├── Failed
  │      ▼
  │   Need Activation / Recovery
  │
  ▼
Load Local Owner Document
  │
  ▼
Load Last Known Good State
  │
  ├── Not Exists
  │      ▼
  │   First Boot Path
  │
  └── Exists
         ▼
      Warm Restart Path
```

### 16.1 First Boot Path

```text
First Boot
  │
  ▼
Resolve Zone Document
  │
  ├── Failed / Invalid / Unacceptable
  │        ▼
  │     Boot Failed
  │
  └── Success + Acceptable
           ▼
      Persist As Known Good State
           │
           ▼
      Start System
```

### 16.2 Warm Restart Path

```text
Warm Restart
  │
  ▼
Resolve Zone Document
  │
  ├── Success + Smoothly Evolvable
  │        ▼
  │     Accept New State
  │        ▼
  │     Persist New Known Good State
  │        ▼
  │     Start / Continue System
  │
  └── Failed / Invalid / Not Smoothly Evolvable
           ▼
      Ignore Resolve Result
           ▼
      Start / Continue With Last Known Good State
```

---

## 17. 实现建议

### 17.1 明确区分 Resolve Result 和 Accept Result

不要让 Resolver 直接决定系统状态。

建议将流程拆成两层：

```text
resolve_did_document() -> ResolveResult
boot_accept_document() -> AcceptResult
```

其中：

- `ResolveResult` 表示文档是否被成功发现和验证；
- `AcceptResult` 表示当前 Boot State Machine 是否接受该文档。

示例：

```rust
enum ResolveResult<T> {
    Found(T),
    NotFound,
    InvalidSignature,
    InvalidBinding,
    NetworkError,
    AuthorityUnavailable,
}

enum AcceptResult<T> {
    Accepted(T),
    Ignored { reason: IgnoreReason },
    Fatal { reason: FatalReason },
}
```

注意：

- `ResolveResult::Found` 不等于 `AcceptResult::Accepted`；
- 对 Warm Restart 来说，很多错误都应该是 `Ignored`，不是 `Fatal`；
- 对 First Boot 来说，同样的错误可能是 `Fatal`。

---

### 17.2 将 Smooth Upgrade 检查显式化

建议为 Zone Document 增加明确的兼容性检查：

```rust
fn check_zone_smooth_upgrade(
    current: &ZoneState,
    candidate: &ZoneDocument,
) -> SmoothUpgradeResult
```

可能的返回值包括：

```rust
enum SmoothUpgradeResult {
    Compatible,
    IncompatibleConsensusModelChanged,
    IncompatibleMembershipChange,
    IncompatibleRoleChange,
    IncompatibleTrustRootChanged,
    IncompatibleUnknownReason,
}
```

这样可以避免把不可接受配置简单归类为普通 resolve 失败。

---

### 17.3 保留明确日志

当新 Zone Document 被忽略时，应记录明确日志，例如：

```text
Resolved zone document version 42, but ignored because membership change from 1 OOD to 3 OOD is not a smooth upgrade.
Continue booting with last known good zone document version 37.
```

日志中应至少包含：

- 当前使用的 Zone Document version；
- resolve 到的 candidate version；
- 忽略原因；
- 当前是 First Boot 还是 Warm Restart；
- 是否继续使用 Last Known Good State。

---

### 17.4 不要在 Boot 中在线更新 Owner Document

Boot 流程中应避免以下行为：

```text
Boot
  │
  ▼
Resolve Owner Document From Authority
  │
  ▼
Replace Local Root Trust
```

正确行为是：

```text
Boot
  │
  ▼
Read Owner Document From node_identity
  │
  ▼
Use It As Local Root Trust
```

Owner Document 的更新必须走 Activation / Maintenance / Recovery 流程。

---

## 18. 非目标

Booting 阶段 resolve-did-document 的特殊处理不应被理解为：

- 另起一套 DID Resolve 标准；
- 绕过签名验证；
- 永远拒绝新版本配置；
- 不支持 Zone Document 演进；
- 不支持 Owner Document 更新；
- 将本地缓存作为永久权威源。

它真正表达的是：

> 在 Booting 阶段，系统必须先保证可恢复性，再考虑配置更新。

---

## 19. 总结

BuckyOS Booting 阶段 resolve-did-document 的特殊处理，本质上是为了解决三个问题：

1. **Bootstrap 问题**  
   Resolve 自己的 Zone Document 时，系统尚未完全运行，不能简单套用 Runtime Resolve 假设。

2. **Smooth Evolution 问题**  
   Zone Document 的变化可能意味着系统升级，但 Boot 阶段只接受能够平滑演进的变化。

3. **Recoverability 问题**  
   BuckyOS 是远程系统，必须避免因为错误配置、恶意配置或 Root Trust 变化导致系统失去最后的管理入口。

最终设计原则可以概括为：

> Boot Resolve does not mean accepting the latest document.  
> Boot Resolve means accepting the latest document that can safely evolve from the current trusted state.

中文表达为：

> Boot Resolve 不是追随最新配置，而是在已有可信状态基础上，接受一个能够安全平滑演进的新配置。

更高层的系统设计原则是：

> Activation 建立信任，Boot 恢复信任，Runtime 演进状态。

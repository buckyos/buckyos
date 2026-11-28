# AICC 作为另一个 AICC 的 Provider TODO

日期：2026-11-17 立项；2026-11-27 按 owner 的新想法（§0）改写，代码基线 `2ef6a8e0a`。状态：owner 已于 11-27 答复 §6 的 Q1–Q13，答复已写进正文（§6.1）；据答复新出现的 N1–N3（§6.2）待答；尚未实施。

目标：Zone A（下游）的 AICC 把 Zone B（上游）的 AICC 加成一个 Provider。A 的应用、Agent、AIWorkspace 照常调用 A 的 AICC；A 的路由选中“来自 B 的模型”时，请求交给 B 的 AICC 执行，B 用自己的 API key 调真实厂商。B 向谁开放、开放什么、怎么收费，用 B 现有的用户组、用户策略和 Credit 表达。同一套机制还让 B 的用户能用 API Token 把自己的 AI 资源以标准协议开放给其他软件。

相关文档：[用户策略与 Credit 实现说明](../doc/aicc/aicc_user_policy.md)、[协议客户端清单](../doc/aicc/protocol_clients.md)、[Provider 冻结设计](../doc/aicc/frozen_provider_implementation.md)、[Trait/Protocol 冻结设计](../doc/aicc/frozen_trait_protocol.md)、[用户接口冻结文档](../doc/aicc/frozen_user_api.md)、[用户类型](../product/users_and_agents/UserType.md)、[跨 Zone 通信](../doc/sdk/cross-zone-communication.md)、[用途路由 TODO](aicc-user-centric-routing-todo.md)、[自定义 Provider 自动探测 TODO](aicc-custom-provider-auto-probe-todo.md)。

> 2026-11-23 起 AICC 不自动换模型或 Provider，Provider 不可用时返回真实错误，备选只来自手工配置；这部分现在由[用途路由 TODO](aicc-user-centric-routing-todo.md) 承接。本文所有“失败后怎么办”都按它处理。

## 0. owner 的想法（2026-11-27 原话）

> - 添加 Provider 的时候，可以添加另一个 AICC 作为 Provider。
> - 需要成为 AICC Provider 的 AICC，流程是需要先建立用户组，然后将一个用户组 policy enable 成 provider（传统的 endpoint + token 模式）或 buckyos 标准的 Limit User 机制
> - AICC 成为 Provider 后，按该用户（组）使用进行计费
> - 协议上，AICC 的 kRPC API 必定向下兼容，因此高版本的 AICC 可以添加低版本的 AICC 实现的 Provider，低版本的 AICC 添加高版本的 AICC 实现的 Provider，只能使用自己能用的模型
> - 如果管理员允许，用户可以创建自己的 API Token，可以把自己在 AICC 中的资源，用传统的协议暴露给其它软件使用。相当于系统不但实现了各种 AI 协议客户端，还挑了一些出来实现了 AI 协议的服务端

**与 11-17 版的关系。** 11-17 版要求上游为外部 Zone 新建一类“Peer 主体”，包括对外授权表、`peer:<did>` 可见性和 peer 产物命名空间。新想法改为：外部调用者就是上游的普通账号，用户组、用户策略和 Credit 照常生效。11-17 版说的最大缺口（External 在上游没有账户、home bucket 和额度）因此不用再补。11-17 版的 Q1–Q10 没有答复，按新想法重排为 §6。其中三项处理如下：Q2（精确模型直通）和 Q7（输入全部内联）按原建议沿用，原话第 4 条“只能使用自己能用的模型”也是按精确模型说的；Q6（peer 产物命名空间）改用账号后不再需要。

## 1. 结论与我对原话的理解

1. **上游不新增系统账号类型。** 对外提供 AI 从用户组开始：管理员建一个组，写组策略（开放哪些 Provider 和模型、价格系数、limit），再选接入方式。
2. **两种接入方式：**
   - **方式 T（endpoint + token）：** 管理员为这个组签发 API Token。一个组 token 就是一个独立用户：策略跟随组，有自己的 Credit 余额（Q1）。下游 AICC，或任何支持标准协议的软件，填 endpoint 和 token 调用。管理员允许时，用户也能在自己名下签发 token，这就是子账号（Q1，原话第 5 条）。
   - **方式 L（绑定 DID 的账号）：** 上游有一个绑定了下游 DID 的账号，类型不限（Q3）。管理员只要允许它对外使用即可，不签发 token（Q2）。下游 AICC 用跨 Zone 断言登录成这个账号，两边不交换任何密钥。
3. **计费不新增扣费路径。** 组 token 按组策略求值，扣它自己的余额；方式 L 扣那个账号的余额；子账号 token 扣谁见 N1。全组共用一个额度池不在本单范围，owner 11-27 已定以后由虚拟 Provider 实现。下游把上游扣掉的 Credit 当成这个 Provider 的成本（§4.11）。
4. **Provider 永远不做路由（Q7）。** 下游的形状沿用 11-17 版：一个 Known Provider `zone-aicc`、一个协议客户端 `aicc-krpc`（wire 就是上游的公共 kRPC）、一个 discovery `aicc-models`。下游只发精确模型，上游只执行那个模型。
5. **AICC kRPC 从当前版本起向下兼容（Q5，§4.8）。** 这是仓库“默认不做向下兼容”规则的明确例外。高版本下游按上游的合同版本降级请求；低版本下游只识别自己目录里有的模型和 API 类型。
6. **协议服务端。** AICC 在 `/kapi/aicc/` 下实现几种标准 AI 协议的服务端，用 API Token 鉴权，只接受精确模型。第一批是 OpenAI Chat Completions、Models、Embeddings（Q6）。另一个好处：没有 `aicc-krpc` 的旧版本 AICC，可以在自定义 Provider 入口重开后，把新版本 AICC 当作普通的 OpenAI 兼容 Provider 加进来。
7. **只做 Immediate，不支持流式（Q8）。** 原生长任务（视频等）需要上游新增任务观察接口，放到第二期。

## 2. 场景与不做

场景（主场景不再影响鉴权和计费的设计，不必先定）：

- **S1 家庭或朋友：** 一家人里只有一个 Zone 配了 API key，其他成员的 Zone 借用。可以免费（系数 0），也可以充值。
- **S2 组织：** 部门 Zone 统一使用总部 Zone 的 AICC，集中采购、集中审计，用组策略和 limit 管控。
- **S3 官方或商业：** 服务商运行一个 BuckyOS AICC，向用户 Zone 出售推理，Credit 充值走现有的支付网关接入（[用户策略](../doc/aicc/aicc_user_policy.md) §5）。
- **S4 同一用户的多个 Zone：** 例如 Lite 桌面节点或测试 Zone 借用主 Zone。
- **S5 让其他软件使用自己的 AI 资源：** 在支持自定义 OpenAI 兼容接口、且能发非流式请求的软件或脚本里，填 base URL 和 API Token（原话第 5 条）。

本期不做：

- **逻辑模型委托。** Provider 永远不做路由（Q7）：`aicc-krpc` 和协议服务端都只接受精确模型，路由全部在下游。
- **流式。** AICC 统一不支持流式（Q8）。
- **全组共用的额度池**（虚拟 Provider）。
- **金钱结算。** Credit 的充值和支付网关已经有了，本单不改动。
- **多级链（A→B→C）。** 第一期禁止（§4.9、Q9）。

## 3. 现状（2026-11-27 核对）

### 3.1 上游可以直接使用的

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| 用户策略与 Credit | 策略主体有 `type:*`、`group:<id>`、`user:<id>`、`app:`、`user_app:`；组这一层有多条时取对用户有利的值；每个账号一个 Credit 余额；每次上游尝试之前准入、之后扣费 | `doc/aicc/aicc_user_policy.md`；`frame/aicc/src/policy/mod.rs`、`service/policy.rs` |
| Limited 账号 | 受限会话含 `aicc` scope（`kernel/buckyos-api/src/hybrid_sso.rs:136`）；RBAC 组 `limited` 只能读 `obj://config/services/aicc/info`（`rbac_config.rs:209`）；`type:limited` 默认没有策略，能用什么完全由组策略决定 | `product/users_and_agents/UserType.md` §2.5 |
| 系统用户组 | Control Panel 的 `user_group.*` 方法，仅管理员可用；成员关系记在用户 settings 的 `groups` 字段 | UserType.md §4 |
| AICC 调用者解析 | User 和 App 会话经 `active_account` 得到 `CallerPrincipal::Account { user_type, groups }`；External 会话一律返回 `resource_not_permitted` | `frame/aicc/src/service/identity.rs:217-270` |
| 产物存放 | 写进调用者账号的 `user/aicc/<tenant>/outputs` 桶 | `frame/aicc/src/resource/mod.rs:88-100` |
| 网关 | 通用的 `/kapi/<service>/*` 路由（`src/rootfs/etc/boot_gateway.yaml:394`），`/kapi/aicc/*` 不改网关就能到达 AICC | — |
| DID 到账号的映射 | `services/control_panel/native_login_bindings/<sha256(did)>`，另外按 `users/<id>/doc` 的 DID 查找（`kernel/verify_hub/src/main.rs:1281` 的 `account_for_did`）。目前只有浏览器 DID 联合登录（`login_by_remote_verify_hub`，目标是 Control Panel）用到它 | `hybrid_sso.rs:12` |
| 用户自建密钥的先例 | nfs-server 的 S3 兼容接口：用户为某个 App 签发 `(access_key_id, secret)`，可撤销、可设有效期、可用 `scope` 收窄权限；`/s3/*` 与 `s3.<zone>` 的 SigV4 由服务自己校验 | `frame/nfs_server/src/bucket/rpc.rs:318-326`；`doc/arch/应用对象桶 Bucket 设计.md` 第 972 行 |

### 3.2 上游缺的

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| 跨 Zone 断言登录 | 结果总是 External 会话（`SessionSubject.subject = requester`），不查这个 DID 有没有对应账号；`cross_zone_target_scopes` 只有 `msg-center` 和 `homestation`；`SessionCeiling::external()` 不含 `Aicc` | `verify_hub/src/main.rs:2528` 起；`cross_zone_auth.rs:25`；`hybrid_sso.rs:148` |
| 把归属别的 Zone 的 DID 加成本 Zone 账号 | `user.invite.create` 可以定向邀请 DID，但接受时要求对方的 OwnerDocument 绑定了本 Zone（`owner_is_bound_to_zone`），这是“入驻”的语义。UserType.md §3.1 的“管理员可以为原生 DID 单独开通本 Zone 登录”还只是拟议 | `frame/control_panel/src/user_mgr.rs:714`、`:1870` |
| 长期 token | AICC 和 verify-hub 都没有 kRPC 长期 token | — |
| AICC 的 HTTP 入口 | 只处理 POST：`/kapi/aicc/artifact/open` 单独处理，其余 POST 一律当作 kRPC；不处理 GET，也不按路径分发 | `frame/aicc/src/service/mod.rs:348-368` |
| 请求体上限 | 32 MiB，在检查 token 之前整段读入；代码注释的前提是“AICC 只在 Zone 内可达” | `service/mod.rs:165-176` |
| 合同版本 | 没有任何版本号。`aicc_client.rs` 有 94 处 `deny_unknown_fields`，`aicc_quota.rs` 有 51 处；`frozen_user_api.md` 第 249 行写着“Beta 2.2 直接切换，不保留旧字段 alias” | `kernel/buckyos-api/src/` |
| 读取产物 | `artifact/open` 只接受 `url` 和 `artifact_id`，不能按 `obj_id` 读自己的产物 | `service/mod.rs:178-184` |
| 上报费用的币种 | Provider Rules 的 `reported_cost { currency, semantics }` 按 profile 写，同一 profile 的不同实例不能用不同币种；`credit_rates` 拒绝币种 `CREDIT` | `frame/aicc/src/catalog/schema.rs:992`；`service/policy.rs:1091` |

### 3.3 下游（11-17 研究结论中仍然成立的部分，已按重构后的代码核对）

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| Provider 的组成 | Known Provider（`driver_metadata/known-providers/*.json`）+ Provider Rules + 协议客户端 + discovery + 实例。discovery 行为注册在 `registry.rs:30`，连接行为在 `:534`（目前只有 `standard`、`sn`）；SN 用 `dynamic_login_behavior_id` 登录，`instance_fields.account` 同时允许 `api_key` 和 `dynamic_login` 两种方式 | `frame/aicc/src/provider/builtin/registry.rs`；`known-providers/sn.known-provider.json` |
| 协议客户端清单 | 22 个，`doc/aicc/protocol_clients.md` 由测试生成；新增客户端要同时改清单 | `protocol/client_catalog_tests.rs` |
| 自定义 Provider | 本版本已关闭添加入口；“只填 endpoint 和 token、自动探测”的 TODO 未实施 | `notepads/aicc-custom-provider-auto-probe-todo.md` |
| 用途路由 | `same_model` 按 `model_driver_id` + `origin_model_id` 找“同一模型的另一个 Provider”（未实施）。上游 AICC 的模型只要带上原厂身份，就能当备用渠道 | `notepads/aicc-user-centric-routing-todo.md` §3.4 |
| 模型列表 | `models.list` 按调用者过滤，每个模型带调用者的 `credit_price`；不含 `canonical_fields` 和执行模式，还附带很大的 catalog | `service/management.rs:739` |
| 原厂身份 | `DiscoveredModel` 没有 `model_driver_id`、`origin_model_id` 字段 | `provider/inventory.rs:405` |
| exact model 格式 | `<provider_model_id>[:variant]@<instance>`；`provider_model_id` 不允许含 `@`，实例名只允许 `[A-Za-z0-9-_.]` | `provider/mod.rs` |
| 跨 Zone 客户端 | `CrossZoneClient` 用设备密钥签 60 秒的断言，对方 verify-hub 发会话；先走 RTCP，失败回退 HTTPS，自动续期；没有对外取出会话 token 的方法 | `kernel/buckyos-api/src/cross_zone_client.rs:190` |
| 执行模式 | xllm 只用 Immediate；流式和长任务的结果在上游 TaskMgr 里，跨 Zone 读不到 | `execution/mod.rs` |
| 上游 TaskMgr 的记录 | 小请求的原文会写进上游 TaskMgr 的 task input | `execution/mod.rs`、`service/ports.rs` |

## 4. 方案

### 4.1 总体

```text
方式 L：
Zone A 应用 / Agent ──kRPC──> A.AICC ──路由──> "openai-main/gpt-5.1:reasoning-high@zone-b"
                                   │ aicc-krpc（按 B 的合同版本降级）
                                   ▼
          跨 Zone 断言登录（主体是 A 的 owner DID）──> B.verify-hub
                                   │ 这个 DID 绑定了 B 的账号 zone-a（在组 peers 里，策略允许对外使用）
                                   │ → 发账号会话，只含 aicc scope
                                   ▼
          B /kapi/aicc  chat.completions.create  exact_model = "gpt-5.1:reasoning-high@openai-main"
                                   │ 按账号 zone-a 的策略准入，扣它的 Credit
                                   ▼
                                 OpenAI

方式 T：同上，凭据换成 B 为组 peers 签发的组 token（HTTPS + Bearer），不经过 verify-hub；
        按组策略准入，扣这个 token 自己的 Credit。
其他软件：POST https://<zone-b>/kapi/aicc/openai/v1/chat/completions（model 是精确模型，不能流式）
          Authorization: Bearer <API Token>
```

- **下游负责：** 全部路由和“不可用时”的备选（用途路由 TODO）、本 Zone 用户的策略与 Credit、用量记录、把产物存进自己的桶。
- **上游负责：** 这个调用主体能用什么、价格系数、limit、扣费；按请求里的精确模型真实调用；自己的用量记录。
- **两边各算各的账：** 下游用户扣下游的 Credit；下游这个 Provider 的成本，就是上游扣掉的 Credit（§4.11）。

### 4.2 上游：外部调用者与开放流程

**外部调用者**（Q1、Q2、Q3、Q7）。下面三种统称“外部调用者”：

| 调用者 | 怎么来 | 策略 | 扣谁的 Credit | 用量归属 |
| --- | --- | --- | --- | --- |
| 组 token | 管理员为某个用户组签发，即原话的“把用户组 policy enable 成 provider” | 这个组的策略；管理员可以再写 `token:<id>` 策略单独覆盖，作用同 `user:` 层 | 它自己的账户 `token:<id>`：一个组 token 就是一个独立用户 | `token:<id>` |
| 子账号 token | 用户在自己名下签发，要求管理员允许（`provider_access`） | 父账号的有效策略，只能收紧：`token:<id>` 策略，收紧规则同用户自己写的 `user_app` 策略 | 待定（N1，建议扣父账号的余额） | 父账号；operator 记 `token:<id>` |
| 方式 L 的账号会话 | 下游以绑定的 DID 跨 Zone 登录；账号类型不限；要求管理员允许（`provider_access`）；不签发 token | 账号自己的有效策略 | 账号自己 | 账号；业务 App 记 `aicc` |

共同规则：

- **Provider 不做路由（Q7）。** 外部调用者只能调用精确模型的 typed 推理方法，`helper.*` 和 `route.resolve` 一律拒绝。上游只执行请求里的那个模型：同一模型的重试照常，但不走“不可用时”策略和 fallback，既不换模型也不换 Provider。路由全部在下游。
- **不支持流式（Q8）。** 请求要求流式（`execution_mode = stream`，或协议服务端的 `stream: true`）一律拒绝。
- **能调用的方法：** 精确模型的 typed 推理方法、`models.list`（普通视图和 Provider 视图，都只列精确模型）、`artifact/open`、`policy.status`、`credit.account.get`。其余方法一律拒绝。
- **不能链**（§4.9）。

**管理员流程（目标交互）：**

1. 在 Users & Agents 里建用户组，例如 `peers`。在 AI Center › 策略里给 `group:peers` 写策略：开放哪些 Provider 和模型、价格系数、limit。
2. 方式 T：在 AI Center › API Token 为 `peers` 签发一个组 token，名字写对方（例如“zone-a”）。在点数页给这个 token 充值，或者把系数设成 0（免费）。把 endpoint 和 token 交给对方。
3. 方式 L：为对方的 DID 建一个账号，或者使用已有账号，放进 `peers`；在组策略里打开“允许对外使用”（`provider_access`）。把本 Zone 的 DID 或域名告诉对方。不签发 token。
4. 用户自己开放：管理员在用户所在的层（`type:*`、`group:*`、`user:*`）打开 `provider_access` 后，用户可以在自己名下签发子账号 token。

**开关 `provider_access`**（Q13 定名）：

- `PolicyBody` 加一个布尔字段，只在用户侧三层有意义，按现有规则求值：取设置了它的最高一层；组这一层有多条时取 `true`。App 侧策略和 `token:<id>` 策略都不能写这个字段。
- 它管两件事：用户能否签发和使用子账号 token；账号能否被方式 L 使用（这个理解待 N3 确认）。组 token 不看这个开关：管理员为组签发 token，本身就是开放。
- 关掉后，已签发的子账号 token 和已登录的方式 L 会话立即不能用。准入时检查开关，不删除 token。默认策略不打开。
- 拒绝时返回 `resource_not_permitted`，`details.reason = "provider_access_disabled"`，做法同现有的 `unpriced`。

**计费：** 不新增扣费路径。调用者解析成上表的主体后，准入、钉价、扣费、limit 计数都走现有的 `PolicyService`。要改的是让组 token 成为策略和 Credit 的一种主体：

- 策略求值：组 token 的用户侧只有 `group:<G>` < `token:<id>` 两层，没有 `type:*` 层，也没有 App 侧的层。
- Credit：账户、账本和计数器的用户键用 `token:<id>`。`credit.recharge`、`credit.adjust`、`credit.account.get`、`credit.ledger.list` 都要接受它，`credit.account.list` 要列出组 token。
- 组策略上的 limit，每个组 token 单独计一份，与组成员相同。

### 4.3 API Token

由 AICC 自己签发、自己校验，不经过 verify-hub（Q4，做法同 S3 key）。形态按行业惯例：Bearer 密钥，明文只在创建时显示一次，有名字，可设有效期，可随时撤销，显示最后使用时间。

- **存储：** AICC 库新增 `aicc_api_token` 表，存储版本从 4 升到 5，在打开时升级（同 3 → 4 的做法）。字段：`token_id`、`kind`（`group` / `sub_account`）、`group_id?`（组 token）、`owner_user_id?`（子账号 token）、`name`、`secret_hash`（sha256）、`created_by`、`created_at`、`expires_at?`、`revoked_at?`、`last_used_at`。node_updater 的 AICC reader 要同步识别新版本（它对 schema 4 的处理见 `aicc-user-policy-credit-todo.md` §12）。
- **格式：** `bkai_<token_id>_<secret>`（Q13）。`token_id` 用于查行，`secret` 至少 32 字节随机数。
- **方法**（类型放在 `buckyos-api`，新建 `api_token_methods`）：

  | 方法 | 参数 | 权限 |
  | --- | --- | --- |
  | `api_token.create` | `name, expires_at?`，再加 `group_id`（组 token）或 `user_id?`（子账号 token，缺省为本人） | 组 token 只有管理员能建。子账号 token：本人，要求本人的有效策略 `provider_access = true`；管理员可以为任何用户建 |
  | `api_token.list` | `group_id?, user_id?` | 管理员看全部；用户只看自己名下的 |
  | `api_token.revoke` | `token_id` | 管理员；子账号 token 的所有者 |

- **组被删除：** 组 token 随之不能用（准入时发现组不存在就拒绝），不自动撤销。
- **校验：** `RuntimeAuthorizer::resolve`（`service/identity.rs`）在调用 `enforce_opened` 之前，先看 token 是否带这个前缀。带前缀时，先查到这一行，用常量时间比较核对 hash，再确认没有撤销、没有过期。之后按种类：
  - 组 token：组存在 → 主体为 `token:<id>`。
  - 子账号 token：父账号 Active（复用 `active_account`），且 `provider_access` 为真 → 主体为父账号，operator 记 `token:<id>`。
  - `CallerPrincipal` 要能表达组 token，可以新增一个变体，也可以扩展 Account，由 CodeAgent 定。
- **token 放在哪里：** kRPC 请求放在 `sys[1]`；HTTP 入口（`artifact/open`、协议服务端）放在 `Authorization: Bearer`；Claude 族协议服务端另外认 `x-api-key`，这是该协议的惯例。
- **`last_used_at`：** 写入要节流，例如每个 token 每分钟最多写一次，不在每次调用时写库。

### 4.4 方式 L：绑定 DID 的账号 + 跨 Zone 断言登录

**上游 verify-hub：**

- `cross_zone_target_scopes` 加 `"aicc" => [Aicc]`。
- `handle_login_by_cross_zone_assertion` 处理 target `aicc` 时，断言和 custody 验证通过后，用 `account_for_did(requester)` 查账号：
  - **Active：** 发账号会话。`subject = user_id`；上限是账号类型的上限与 `[Aicc]` 的交集；`auth_method = cross_zone_assertion`；最长时长同联合会话，30 天。
  - **Inactive：** 拒绝，原因 `AccountState`。
  - **Unknown：** 拒绝，不发 External 会话。External 在 AICC 里本来什么都用不了，提前拒绝，错误更清楚。
  - `msg-center` 和 `homestation` 的行为不变，仍然发 External 会话。
- 账号类型不限（Q3），会话只有 `aicc` scope。

**上游 Control Panel：** 需要能把“归属别的 Zone 的 DID”绑定到本 Zone 的一个账号。现有的定向邀请是入驻语义，要求对方的 OwnerDocument 绑定本 Zone（`user_mgr.rs:714`），不适用于这里。新增一个管理员方法：建账号时（或对已有账号）写入 `native_login_binding`，不要求对方文档绑定本 Zone。对方仍要用自己 Zone 的设备签名证明自己持有这个 DID。这就是 UserType.md §3.1 里“管理员可以为原生 DID 单独开通本 Zone 登录”的拟议，owner 已同意（Q2），先只用于 AICC。动手前先做 Z0-3，确认现有代码有没有可用的路径。

**上游 AICC：** 账号会话已经走 `active_account` 这条路，不用改。只加两条：`auth_method = cross_zone_assertion` 的会话要求 `provider_access` 为真；按 §4.2 外部调用者的共同规则限制方法。

**下游：**

- AICC 进程加载设备私钥，建一个 `CrossZoneClient`（Z0-4）。
- 断言的主体用下游 Zone owner 的 DID（Q2）。owner DID 的文档指向下游 Zone，`check_custody` 按 SameZone 通过（Z0-5 核实）。
- 传输有两种做法，由 CodeAgent 按 Z0-8 的结果选，选定后写回本文：
  - **a.** 新增连接行为 `zone-aicc`：请求经 `CrossZoneClient::request_to` 发送（先 RTCP、失败回退 HTTPS、自动登录续期）。这需要在“codec → 传输”之间加一个注入点，是 11-17 版的方案。
  - **b.** 给 `CrossZoneClient` 加一个取会话 token 的方法，做成 dynamic login resolver，与 SN 的 `dynamic_login_behavior_id` 同一个模式；请求走普通的 `HttpTransport` + HTTPS。改动更小，但不用 RTCP。
  - 倾向 b，前提是 Z0-8 确认上游 Zone 的 HTTPS 入口在目标场景下都可达。

### 4.5 协议服务端

- **入口：** 都放在 `/kapi/aicc/` 下，不改网关。
  - OpenAI 族的 base URL 是 `https://<zone>/kapi/aicc/openai/v1`。第一批：`POST /chat/completions`、`GET /models`、`POST /embeddings`。第二批：`POST /responses`、`POST /images/generations`。
  - Claude 族的 base URL 是 `https://<zone>/kapi/aicc/anthropic`，提供 `POST /v1/messages`，放第二批。
  - 路径和批次按建议（Q6）。`AiccHttpServer::serve_request`（`service/mod.rs:348`）先按路径分发，其余 POST 再交给 kRPC；还要支持 GET。
- **鉴权：** 只认 API Token（组 token 或子账号 token，§4.3），不接受会话 cookie。调用者是外部调用者，§4.2 的共同规则全部适用。
- **清单：** 服务端清单和客户端清单一样，代表“我们对主流 AI API 类型的理解”，属于需要发版本才能改的部分。在 `doc/aicc/protocol_clients.md` 加“协议服务端”一节，由测试校验注册表与表格一致。
- **实现：** 服务端 codec 与客户端 codec 方向相反。它把外部 wire 解成 AICC 的 typed 精确模型请求（`chat.completions.create` 等），调用现有的推理入口（与 kRPC 同一条路：准入、扣费，不经过路由），再把结果编回外部 wire。wire 结构尽量复用 `protocol/openai_chat_completions.rs` 等文件里已有的类型。
- **不支持的参数：** 返回 400 并写明参数名，不静默丢弃，与 AICC“不改写、不丢弃”的原则一致。
- **`model` 字段：** 只接受精确模型名（Q7）。不是精确模型的值（包括用途名）返回 404，统一说模型不存在。`/models` 只列这个 token 能用的精确模型。
- **`stream: true`：** 返回 400，说明 AICC 不支持流式（Q8）。不做伪流式。
- **错误：** AICC 错误码映射成 HTTP 状态和该协议的错误格式。建议：`resource_not_permitted` → 404（统一说模型不存在，不透露它是否存在）；`quota_exhausted` → 429；`credit_insufficient` → 402；`invalid_request` 类 → 400；上游 provider 错误 → 502；`timeout` → 504。完整的表由 CodeAgent 按各协议的官方文档定，写进 `protocol_clients.md`。
- **多模态输入：** data URL 直接内联；http(s) URL 按现有资源规则物化，受现有 `ResourceLimits` 和 32 MiB 上限约束。
- **输出产物：** 第一批不做图片。第二批再定图片按 base64 还是外部可读的 URL 返回。

### 4.6 下游：`zone-aicc` Provider

| 组件 | 内容 |
| --- | --- |
| Known Provider `zone-aicc` | 显示名“BuckyOS AICC”（Q13）。两种凭据，做法同 SN 的 `instance_fields.account`：方式 T 用 `bearer`，endpoint 是对方的 `https://<host>/kapi/aicc`；方式 L 用跨 Zone 登录，连接字段是对方的 Zone DID 或域名，没有密钥。`discovery_behavior_id = aicc-models` |
| Provider Rules `zone-aicc` | 每个 api_type 绑定到同名 typed method 的 operation；`reported_cost` 见 §4.11；不用 `model_driver_overrides`，原厂身份由 discovery 给出 |
| 协议客户端 `aicc-krpc` | 一个独立的客户端，进 `protocol_clients.md`（层：B 官方独有；平台：BuckyOS；对自定义开放：否，只供 `zone-aicc` 实例使用）。每个 operation 对应一个 typed method。codec 先把内部 `AiccCall` 降级到上游的合同版本（§4.8）再序列化；错误按 §4.14 归一；注册自定义的 `ArtifactDownloadProtocol`（POST + JSON body） |
| Discovery `aicc-models` | 调用上游的 Provider 视图（§4.7），产出 `DiscoveredModel`，记下上游的合同版本和库存 revision |
| 实例 settings | endpoint 或 Zone DID、凭据方式、enabled、超时 |

- **添加入口：** Desktop 的“添加 Provider”加一张“BuckyOS AICC”卡片，不依赖自定义入口重开。自动探测 TODO 实施后，如果探测流程认出对方是 AICC（调 Provider 视图成功即是），就直接建成 `zone-aicc` 实例。为此要把 BuckyOS AICC 加进那份 TODO §5 第 5 条“先支持哪些网关”。
- **模型身份与命名（沿用 11-17 版）：**
  - 下游的 `provider_model_id` 取 `<上游实例名>/<上游 provider_model_id>`，exact model 形如 `openai-main/gpt-5.1:reasoning-high@zone-b`，lowering 时还原成 `gpt-5.1:reasoning-high@openai-main`。上游实例名不含 `/`，按第一个 `/` 拆分没有歧义；上游 `provider_model_id` 本身带 `/`（如 OpenRouter）也不影响。
  - `DiscoveredModel` 加原厂身份字段 `model_driver_id`、`origin_model_id`。discovery 直接用上游给的身份匹配下游的 Model Driver，不按名字猜。下游目录里没有这个模型，它就不进可执行库存，只留诊断。这是现有规则，也正是原话里“低版本只能使用自己能用的模型”。
  - **variant：** 上游的 typed inference 不接受 `provider_options`，所以下游只保留上游实际列出的 variant。Z0-6 确认 `InventoryBuilder` 能否按 discovery 结果限定 variant，不能就补上。
  - 同一原厂模型经上游的多个实例出现时，在下游是不同的 exact model，可以由用途路由的 `same_model` 互作备选。

### 4.7 上游的 Provider 视图

`models.list` 加 `view: "provider"`（或者新增一个方法），只给 Provider 使用，内容要精简。

每个模型给出：

- `exact_model`、`model_driver_id`、`origin_model_id`、`provider_model_id`、`variant`、`api_types`、`capabilities`、`operations`；
- `canonical_fields`；
- 每个 operation 的执行模式，用来判断能否 Immediate；
- 视频事实；
- 调用者的 `credit_price`（§4.11）；
- 可用状态。

顶层给出：

- `contract_version`（§4.8）；
- 库存 revision，discovery 可以据此跳过没有变化的刷新。

列出范围：只列这个调用者“可用”的精确模型（策略可见，且不是 unpriced），不列用途，并排除来自 `zone-aicc` 的模型（§4.9）。方式 T 和方式 L 调用的是同一个视图。

### 4.8 合同版本与向下兼容

原话第 4 条要求 AICC 的 kRPC API 向下兼容。这与仓库 AGENTS.md 的“默认不做向下兼容”、`frozen_user_api.md` 第 249 行的“直接切换，不保留旧字段 alias”冲突，需要写成明确的例外。owner 定从当前版本开始（Q5），“当前版本”的边界见 N2。覆盖范围按 Q5 的建议（owner 没有提出异议）：推理方法、Provider 视图、`artifact/open`、以后的任务观察接口和协议服务端；`provider.*`、`policy.*` 等管理方法不在承诺内。

规则：

- **合同版本号。** `AICC_CONTRACT_VERSION` 是一个整数，放在 `buckyos-api` 的 `aicc_client.rs`。当前版本为 1；本单新增 Provider 视图等接口时加 1，之后每改一次再加 1。Provider 视图返回它。`aicc-krpc` 的每个请求在 kRPC 保留参数里带上下游自己的版本，做法同 `on_behalf_of`：由 `AiccServerHandler` 取出，不进 DTO。
- **能当 Provider 的最低版本。** 版本 1 的 AICC 没有 Provider 视图、API Token 和跨 Zone AICC 会话，不能被添加为 Provider。下游遇到这样的上游，提示对方升级。
- **只增不改。** 承诺范围内的接口只能加方法、加可选请求字段（缺省时保持旧行为）、加响应字段、加枚举值；不能改名、删除或改变语义。请求 DTO 保留 `deny_unknown_fields`：上游宁可拒绝不认识的字段，也不能悄悄忽略。
- **高版本下游添加低版本上游。** `aicc-krpc` 维护一张表，记录每个可选请求字段、方法、枚举值从哪个合同版本开始有。discovery 时，把上游版本表达不了的能力和 API 类型从库存里去掉，路由就不会选到。直接调用用到了上游没有的字段时，下游本地返回 `unsupported_*`，不发给上游。这张表随客户端一起发版本。
- **低版本下游添加高版本上游。** 下游发的是旧请求，上游按承诺接受。上游按请求里带的下游版本降级响应：不返回那个版本之后才有的字段和枚举值。枚举尤其要注意，旧版本解析不了未知的枚举值。请求没带版本时按 1 处理。
- **模型。** 下游只识别自己 Model Driver 目录里有的模型（§4.6）；上游的新 API 类型，下游不认识就丢掉。
- **验收。** 加一组“合同快照”测试：每个合同版本的请求和响应样例存成 golden。之后版本的服务端必须接受旧请求，并给旧版本的调用者返回它能解析的响应。做法同协议客户端的 wire 快照。

### 4.9 链与环

- **第一期禁止链（Q9）。** 对经 API Token 或跨 Zone 登录来的调用者，上游既不在 Provider 视图里列出、也不执行 `provider_profile_id = zone-aicc` 的模型。这样即使 A 和 B 互相添加，也不会成环。协议服务端同样适用。
- **不另加 hop 参数。** 调用者是不是外部来的，由会话方式（API Token 或跨 Zone 登录）判断。

### 4.10 资源与产物

**输入**（沿用 11-17 版）：

- `aicc-krpc` 的所有资源输入都声明为 `BytesOnly`：下游先物化，再 base64 内联。原因是上游读不到下游的 Named Object，下游的 URL 也可能是内网地址或需要鉴权。
- 受上游 32 MiB 请求体上限约束，而 base64 会膨胀到 4/3。大视频输入第一期不支持，返回明确的错误。
- 公网 URL 直传放到第二期。

**输出：**

- 上游把产物写进调用主体的 AICC 桶（账号是 `user/aicc/<user>/outputs`，组 token 见 Z0-7），返回 `named_object`。扩展 `artifact/open`，让它接受 `obj_id`，只允许读调用者自己桶里的产物；API Token 会话和跨 Zone 会话都能调用。下游经自定义的 `ArtifactDownloadProtocol` 读出字节，写进自己调用者的桶。
- 上游返回 `url`（上游登记过的 Provider URL）时，同样经上游的 `artifact/open` 读取；返回 `base64` 时直接使用。embedding 的结果也以产物返回（`spec.embedding`），走同一条路径。
- 这些产物占用调用主体的桶容量，保留期沿用 AICC 桶现有的清理规则（Z0-7）。

### 4.11 价格与成本

- **上游：** 调用结果里的 `cost`（`AiResponse.cost` 或 `usage.cost`）返回这次给这个账号扣的 Credit，币种写 `CREDIT`。Provider 视图里每个模型带调用者的 `credit_price`（已有的 `ModelCreditPrice`）。
- **下游：** 把上游的 Credit 当成这个实例的 Provider 成本。
  - discovery 把 `credit_price` 换成 `DiscoveredModel.pricing`，币种用这个实例专属的 `credit:<上游 Zone 主机名>`（Q12），不同上游的 Credit 不会混在一起。
  - `reported_cost` 读取上游返回的扣费，币种同上。
  - 下游管理员在价目表的 `credit_rates` 里给这个币种设汇率（例如 1:1），或者给这个 Provider 设系数。没设就是 `unpriced`，对要付费的用户不可用，这与现有的“没有价格不等于免费”一致。
- **需要的改动：** `reported_cost` 的币种现在按 profile 写死（`catalog/schema.rs:992`），要改成能由实例（discovery）给出。

### 4.12 执行模式（沿用 11-17 版）

| 情况 | 第一期 | 说明 |
| --- | --- | --- |
| Immediate | 做 | 上游返回 `succeeded` 和结果。下游的超时要比普通 Provider 长：跨 Zone，上游还可能对同一模型重试。网关的空闲超时要在 DV 里实测（Z0-8） |
| 上游返回 `running` | 排除 | 上游有些 operation 是原生长任务（视频、OpenAI background）。discovery 按执行模式只保留能 Immediate 的组合。万一仍收到 running，按 provider_error 失败，并记下上游的 task_id |
| NativeTask（视频等） | 第二期 | 需要上游新增任务观察接口：`task.get` 返回状态和最终结果，只有任务创建者能调用；`cancel` 已经有了。下游 `NativeTaskCodec` 的 submit / status / result / cancel 映射到这些方法 |
| Stream | 不做 | AICC 统一不支持流式（Q8）：`aicc-krpc` 不声明流式 operation，上游对外部调用者拒绝流式请求 |

- **幂等：** 下游把自己的 `<task_id>:<attempt>` 作为上游的 `idempotency_key`，连接中断后重发不会重复扣费。需要确认上游用同一个 key 重放已完成的任务时返回原结果（Z0-9）。
- **取消：** 下游在 Immediate 请求进行中断开连接，并不会取消上游的任务。下游取消时，如果已经拿到上游的 task_id，就调用上游的 `cancel`。

### 4.13 provider_state 与 thinking（沿用 11-17 版）

- 上游返回的块带着上游的坐标，而下游的 `bind_provider_state_source` 会把它覆盖成下游坐标（`zone-aicc` / `aicc-krpc` / 原厂 / 模型）。
- 下游把这些块回放给同一个 exact model 时，上游必须看到原来的坐标，才会原样回放。做法：
  - **provider_state：** `aicc-krpc` decode 时把上游的原块整个包进下游的 `provider_state.value`，保留上游的 `source`；encode 时（目标仍是 `aicc-krpc`）解包还原。由上游自己按它的四元组判断是本源块还是外来块。
  - **thinking：** 上游坐标放进 `Thinking.provider_metadata`，encode 时还原。
- 下游坐标不含上游实例名，所以“原厂和模型相同、上游实例不同”的两个下游 exact model 坐标一样。这不会出错：下游只负责放行，最终由上游判断。

### 4.14 错误映射

| 上游错误 | 下游处理 |
| --- | --- |
| `quota_exhausted`、`credit_insufficient` | **不能原样透传**，否则下游的调用方（xllm、OpenDAN）会以为是用户自己的额度或余额不够，去提示 owner。映射成 provider 错误“上游额度不足”或“上游余额不足”，details 带上游的 `reset_at_ms`。是否换到别的模型，由下游的“不可用时”策略决定（用途路由 TODO） |
| `resource_not_permitted`（含 `provider_access_disabled`、`unpriced`） | 上游收回了授权，或改了价目表：按 provider 错误处理，并触发一次库存刷新 |
| `quota_unavailable` | 可重试的 provider 错误 |
| `context_length_exceeded` | 原样透传，语义相同且不可重试 |
| `invalid_request`、`schema_validation_failed`、`invalid_method`、`unsupported_*` | 有了合同版本，不应再出现；出现就说明降级表有漏洞。按 provider 错误处理，health 记为“合同不兼容”，并带上两边的版本号 |
| `provider_error`、`timeout`、`no_provider_available` | 透传 `retriable` 和 retry-after，只对同一模型重试 |
| kRPC `NoPermission`、token 错误 | 方式 L：`CrossZoneClient` 自动重新登录一次。方式 T：token 已被撤销或过期，Provider 标为 unhealthy，界面提示更换 token |
| 网络不可达 | 和普通 Provider 的网络错误一样处理 |

### 4.15 隐私与安全

按“先能力再安全”的原则，以下都记为已知风险：

- **请求内容进入上游。** 下游用户的提示词和图片会进入上游：上游 TaskMgr 保存小请求的原文，上游的 trace 和用量里也有记录，上游管理员都能看到。添加 Provider 的界面要明确告知。外部调用的任务是否只存摘要，见 Q11。
- **不传最终用户。** 下游不把自己的最终用户和 App 告诉上游（Q10），上游只看到“账号 zone-a 通过 aicc 调用”。
- **API Token 泄露。** 任何拿到 token 的人都能消耗它的 Credit：组 token 消耗它自己的余额，子账号 token 按 N1 而定。靠撤销、有效期、`token:<id>` 策略收紧和 limit 兜底。
- **请求体在鉴权前读入。** AICC 在验证 token 之前就把最多 32 MiB 的请求体整段读入内存（`service/mod.rs:165-176`）。开放给外部以后，这会成为被外部消耗内存的入口。第一期记为已知风险，以后改成在 HTTP 入口先验 token 再读请求体。

### 4.16 管理界面

**上游：**

- Users & Agents：新建账号时可以填“对方 DID”（方式 L，§4.4）；用户组照旧。
- AI Center › 策略：策略编辑器在用户侧三层加“允许对外使用”开关（`provider_access`）；组 token 可以单独写 `token:<id>` 策略。
- AI Center › API Token（新页面）：管理员为用户组签发组 token，查看全部 token（含子账号 token），可以撤销；显示种类、所属组或父账号、余额（组 token）、最后使用时间和关联的用量。
- AI Center › 点数：账户列表里出现组 token，可以充值。
- 普通用户：只有自己的有效策略打开了开关，才在自己的 AI 设置里看到“我的 API Token”，可以签发子账号 token 并收紧它（模型、limit）；页面提供 base URL 的复制按钮（第一批只有 OpenAI，第二批加 Claude）。

**下游：**

- AI Center › 添加 Provider 加“BuckyOS AICC”，可选“对方 Zone（无需密钥）”或“endpoint + token”。
- 保存前用 `provider.validate` 真实登录一次并列出模型。失败时分别提示“对方还没有为你开通”“token 无效”“对方版本太旧（没有 Provider 视图）”。

**两边的 usage 页：** 上游按账号和 token 查看用量；下游这个 Provider 的成本列显示上游的 Credit。

## 5. 与其它 TODO 的关系

- **用户策略与 Credit（已实施）：** 本单加一个策略字段、一种策略与 Credit 的主体（组 token）和一张 token 表，扣费路径不变。
- **用途路由 TODO：** 在下游，上游 AICC 是普通 Provider，“不可用时”策略、手工标记故障、`same_model` 都适用。在上游，外部调用者的请求不走这些策略（Provider 不做路由，Q7）。
- **自定义 Provider 自动探测 TODO：** 把 BuckyOS AICC 列为要支持的网关，探测到就建 `zone-aicc` 实例。反过来，协议服务端让 AICC 自己也成为一个可被探测的 OpenAI 兼容网关。
- **协议客户端重构：** `aicc-krpc` 按一个协议客户端注册，进客户端清单；服务端清单加在同一份文档里。

## 6. owner 裁决（2026-11-27 答复）

表格与引用行是原问题和 owner 的原话答复，保留不改。

| # | 问题 | 建议 |
| --- | --- | --- |
| Q1 | “将一个用户组 policy enable 成 provider”，我理解为策略里的一个开关（暂名 `provider_access`），只在用户侧三层设置，同时管 API Token 和跨 Zone 登录。对吗？默认策略要不要给 `type:admin` 打开（管理员开箱就能给自己建 token）？ | 是一个开关；默认都不打开 |
> 这个的意思是，管理员创建这个api token的时候，follow的是这个用户组的配置。一个api token可以视作一个独立用户。 在用户名下创建一个api token就是子账号的概念了
| Q2 | 方式 L：下游用哪个 DID 登录上游？上游怎么把这个 DID 绑定到账号？ | 用下游 Zone owner 的 DID；上游新增“管理员为外部 DID 建 Limited 账号”，不要求对方文档绑定本 Zone（即 UserType.md §3.1 的拟议，先只用于 AICC） |
> 按建议。这种添加方法上游不需要创建api token,只要允许就好了
| Q3 | 方式 L 的账号必须是 Limited 吗？ | 不限类型，会话只有 `aicc` scope；文档和界面推荐 Limited |
> 不限类型
| Q4 | API Token 由 AICC 自己签发和校验（同 S3 key），还是做成 verify-hub 的通用长期 token？ | AICC 自己管 |
> AICC 自己管，这个固定API Token是follow行业管理
| Q5 | 向下兼容从哪个版本开始、覆盖哪些方法？ | 从本功能发布时的合同版本 1 开始；只覆盖推理方法、Provider 视图、`artifact/open`、以后的任务观察接口和协议服务端；`provider.*`、`policy.*` 等管理方法不在承诺内。在 `frozen_user_api.md` 写明这个例外；AGENTS.md 要不要加一句，由 owner 定 |
> 当前版本
| Q6 | 协议服务端第一批做哪些、路径是什么？ | 第一批 OpenAI Chat Completions + Models + Embeddings，base URL `https://<zone>/kapi/aicc/openai/v1`；第二批 Responses、Claude Messages（`/kapi/aicc/anthropic`）、Images |
> 按建议

| Q7 | 协议服务端的 `model` 字段接受什么？ | 精确模型和用途名（`llm.chat` 等，按 token 持有者的路由选）；`/models` 两种都列 |
> 只接受精确的模型名，路由在下游的aicc里。provider永远不做路由

| Q8 | 协议服务端收到 `stream: true` 怎么办？ | 第一期伪流式（执行完一次按 SSE 发出）；真流式随第二期 |
> 我们统一不支持流式

| Q9 | 是否允许链（A→B→C）？ | 第一期禁止：上游对外部调用者不列出、不执行来自 `zone-aicc` 的模型 |
> 按建议

| Q10 | 下游是否把自己的最终用户和 App 告诉上游？ | 不传 |
> 按建议

| Q11 | 上游对外部调用者的任务是否只存摘要？ | 先不做，记为已知风险 |
> 按建议
| Q12 | 下游记录上游成本用什么币种？ | 每个上游一个币种 `credit:<上游 Zone 主机名>`，由下游管理员设汇率 |
> 按建议
| Q13 | 命名：profile `zone-aicc`、客户端 `aicc-krpc`、显示名“BuckyOS AICC”、策略字段 `provider_access`、token 前缀 `bkai` 是否可以？ | — |
> 按建议

### 6.1 答复如何写进正文

- **Q1：** API Token 分两种（§4.2、§4.3）。管理员为用户组签发的组 token 是一个独立用户：策略跟随组，有自己的 Credit。用户在自己名下签发的是子账号 token。`provider_access` 因此不再是“把组变成 Provider”的开关，只管两件事：用户能否签发子账号 token（原话的“如果管理员允许”），以及账号能否被方式 L 使用（Q2 的“只要允许就好了”，见 N3）。
- **Q2、Q3：** 方式 L 不签发 token，账号类型不限（§4.4）。
- **Q4：** AICC 自己管，形态按行业惯例（§4.3 开头）。
- **Q5：** 合同版本 1 是当前版本；覆盖范围按建议，owner 没有提出异议（§4.8）。边界见 N2。
- **Q7：** Provider 永远不做路由。外部调用者只能用精确模型，上游不走“不可用时”策略和 fallback（§4.2 的共同规则、§4.5）。
- **Q8：** AICC 统一不支持流式。协议服务端拒绝 `stream: true`，第二期也不做流式（§4.5、§4.12）。
- **Q6、Q9–Q13：** 按建议。

### 6.2 据答复新出现的问题

| # | 问题 | 建议 |
| --- | --- | --- |
| N1 | 子账号 token 的花费扣谁的余额？ | 扣父账号的余额（OpenAI 的项目密钥、OpenRouter 的密钥都是这样），token 只用 `token:<id>` 策略里的 limit 限制上限，不另开余额 |
| N2 | “当前版本”的边界在哪里？工作区里另一个会话正在按用途路由 TODO 删除 `AiccFallbackRule`、`AiccSessionLogicalProfile`、`cost_first` 等公共类型（未提交）。如果按现在的 HEAD 冻结，这些删除就不能再做 | 以 0.7.0 发布为界：合同版本 1 等于 0.7.0 发布时的接口，发布前的调整（含路由 TODO 的删除）照常进行 |
| N3 | 方式 L 的“允许”，是否就是账号有效策略里的 `provider_access`？ | 是。账号类型不限以后，家庭成员已有的 User 账号也可能绑定了 DID，需要一个开关表明“这个账号可以被别的 Zone 的 AICC 使用” |

## 7. 任务

括号里是任务依赖的裁决；不依赖裁决的可以先做。

### Z0 验证（先做，结论写回 §3）

- [ ] Z0-1 target `aicc` 的跨 Zone 登录改发账号会话：`FamilyClaims::cross_zone`（`verify_hub/src/family_store.rs:68`）能否带账号类型的上限；会话刷新（`handle_refresh_token`）时，账号停用或类型变化能否按现有账号会话的规则收紧。
- [ ] Z0-2 账号会话 + `aicc` scope 能否通过 AICC 的 `enforce_opened(..., Aicc)` 和 RBAC（Limited 只有 `obj://config/services/aicc/info` 的读权限）。
- [ ] Z0-3 把归属别的 Zone 的 DID 绑定到本 Zone 账号，现有代码有没有可用路径：`user_mgr.rs` 的定向邀请、`require_unmapped_native_did`、`resource_share.rs` 里的 `native_login_binding`。没有就按 Q2 新增。
- [ ] Z0-4 AICC 进程能否调用 `load_device_private_key`：需要什么权限；多节点 Zone 里由哪台设备签名。
- [ ] Z0-5 AICC 所在设备签 owner DID 的断言时，`check_custody` 是否按 SameZone 通过。
- [ ] Z0-6 `InventoryBuilder` 能否按 discovery 结果限定 variant；给 `DiscoveredModel` 加原厂身份对库存构建和三组快照的影响。
- [ ] Z0-7 产物桶：账号的 AICC 桶（`user/aicc/<user>/outputs`）对 Limited 账号能否创建、配额多少；组 token 不是系统账号，`BucketId::user(..., tenant, ...)` 能否用 `token:<id>` 作 tenant。不能的话，定下组 token 的产物存在哪里，写回 §4.10。
- [ ] Z0-8 跨 Zone 的长 Immediate 请求（5–10 分钟）经 RTCP 和 HTTPS 网关会不会被断开；上游的 HTTPS 入口在 SN 中继场景下是否可达。结论决定 §4.4 的传输选 a 还是 b。
- [ ] Z0-9 上游用同一个 `idempotency_key` 重放已完成的任务时返回什么。

### Z1 上游：账号侧

- [ ] Z1-1 `PolicyBody.provider_access`：类型（`aicc_quota.rs`）、求值（`policy/mod.rs`）、校验（App 侧和 `token:<id>` 策略不能写）、`policy.effective` 显示来源；默认策略不变。（N3）
- [ ] Z1-2 API Token：存储（schema 4 → 5，node_updater 同步）、两种 token、`api_token.*` 方法与共享类型、`RuntimeAuthorizer` 校验、`last_used_at` 节流。
- [ ] Z1-3 组 token 作为策略与 Credit 主体：策略主体 `token:<id>`（对组 token 是覆盖层，对子账号 token 是只能收紧的层）；组 token 的求值只有 `group:<G>` < `token:<id>`；Credit 账户、账本、计数器用 `token:<id>` 作键，`credit.*` 方法接受它；子账号 token 的扣费按 N1。
- [ ] Z1-4 verify-hub：`cross_zone_target_scopes` 加 `aicc`；target `aicc` 发账号会话（§4.4）；测试覆盖 Active / Inactive / Unknown，以及 msg-center 的行为不变。
- [ ] Z1-5 Control Panel：管理员为外部 DID 建账号，或把 DID 绑定到已有账号（Z0-3）。
- [ ] Z1-6 AICC 外部调用者的共同规则（§4.2）：方法白名单、只接受精确模型、不走“不可用时”策略和 fallback、拒绝流式、检查 `provider_access`；用量归属按 §4.2 的表。
- [ ] Z1-7 Provider 视图（§4.7）和合同版本（§4.8）；对外部调用者排除 `zone-aicc` 模型（§4.9）。
- [ ] Z1-8 `artifact/open` 接受 `obj_id`（只读调用者自己的产物），并接受 API Token。
- [ ] Z1-9 调用结果里返回本次扣费（§4.11）。

### Z2 上游：协议服务端

- [ ] Z2-1 `AiccHttpServer` 按路径分发、支持 GET；用 API Token 鉴权。
- [ ] Z2-2 OpenAI Chat Completions 服务端 codec（含 tools、多模态；只接受精确模型；`stream: true` 返回 400）、`/models`、`/embeddings`。
- [ ] Z2-3 错误映射表。
- [ ] Z2-4 `protocol_clients.md` 加服务端清单，并加生成和校验测试。
- [ ] Z2-5 第二批：Responses、Claude Messages、Images。

### Z3 下游：`zone-aicc` Provider

- [ ] Z3-1 metadata：known provider（两种凭据）和 provider rules。
- [ ] Z3-2 `aicc-krpc` 客户端：每个 Immediate typed method 对应一个 operation；按上游合同版本降级（§4.8 的版本表）；provider_state 包装（§4.13）；错误映射（§4.14）；自定义 `ArtifactDownloadProtocol`；进客户端清单。
- [ ] Z3-3 凭据：方式 T 的 bearer；方式 L 的 `CrossZoneClient`（§4.4 的 a 或 b）。
- [ ] Z3-4 `aicc-models` discovery：原厂身份、variant 限定、价格换算成实例币种、`canonical_fields`、视频事实、合同版本与 revision。
- [ ] Z3-5 `reported_cost` 的币种改为可由实例给出（§4.11）。（Q12）
- [ ] Z3-6 资源输入用 `BytesOnly`；产物下载后存进本地桶。
- [ ] Z3-7 幂等 key 和取消；route trace 记下上游 task_id、上游 exact model、上游合同版本。
- [ ] Z3-8 `provider.validate`、`provider.add` 对新 profile 的校验，以及 §4.16 的三种失败提示。

### Z4 兼容（N2）

- [ ] Z4-1 `AICC_CONTRACT_VERSION`（当前为 1，本单落地时为 2）和 kRPC 保留参数（下游版本）；上游按调用者的版本降级响应；下游遇到没有 Provider 视图的上游时提示升级。
- [ ] Z4-2 合同快照测试（§4.8 的验收）。
- [ ] Z4-3 在 `frozen_user_api.md` 第 249 行和 `frozen_trait_protocol.md` 写明这个例外。

### Z5 界面与 CLI

- [ ] 上游：策略开关、API Token 页面（组 token 与子账号 token）、点数页列出组 token、普通用户自己的 token 页面、新建账号时填 DID。
- [ ] 下游：“BuckyOS AICC”卡片、两种接入方式、校验失败的提示。
- [ ] websdk：`AiccClient` 加 `api_token.*` 方法和 TS 类型；CLI 加 `buckyos ai token create|list|revoke`。
- [ ] e2e：在 Desktop 的 `playwright.aicc.config.ts` 下补用例。

### Z6 第二期

- [ ] 上游任务观察接口（`task.get`）；下游 NativeTask codec；视频等长任务。
- [ ] 公网 URL 直传；链（如果 Q9 放开）；HTTP 入口先验 token 再读请求体。

### Z7 文档与手册

- [ ] `doc/aicc/`：`aicc_user_policy.md`（开关、两种 token、组 token 主体、外部调用者的共同规则、跨 Zone 会话）、`frozen_provider_implementation.md` 的内置表、`frozen_trait_protocol.md`、`frozen_user_api.md`、`protocol_clients.md`、provider operation bindings golden。
- [ ] `doc/sdk/cross-zone-communication.md`：AICC 加入白名单、发账号会话。
- [ ] `product/users_and_agents/UserType.md`：管理员把外部 DID 绑定到本 Zone 账号（类型不限，先只用于 AICC）。
- [ ] 手册（按 AGENTS.md 的联动表）：`buckyos_admin`（开放流程、token、添加 BuckyOS AICC）；`aigc_*` 里的错误说明如有变化一并改。

## 8. 测试与 DV

- **合同测试：** 同一进程起两个 AICC：上游用 `AiccServerHandler` 加一个假 Provider，下游用 mock transport 指向它。覆盖 discovery、命名还原、每个 Immediate method、§4.14 的错误映射、provider_state 回放、产物下载、幂等重发，以及合同版本降级（高版本下游加低版本上游、低版本下游加高版本上游各一组）。
- **策略：** `provider_access` 的分层求值；关掉开关后，子账号 token 和方式 L 会话立即被拒；组 token 按组策略求值、扣自己的余额、组 limit 单独计一份；子账号 token 只能收紧，扣费按 N1。
- **token：** 明文只在创建时显示一次；只存 hash；撤销和过期立即生效；组被删除后组 token 被拒。
- **外部调用者：** 管理方法、`helper.*`、`route.resolve` 被拒；非精确模型被拒；流式请求被拒；上游模型不可用时返回真实错误，不按上游的“不可用时”策略换模型。
- **verify-hub：** target `aicc` 的 Active / Inactive / Unknown 三种情况；msg-center、homestation 仍然发 External 会话。
- **协议服务端：** 用官方 OpenAI SDK（Python 或 Node）和 curl 调第一批端点，包括 tools、图片输入、`stream: true` 被拒和错误码。
- **防环：** A、B 互相添加，确认库存里没有对方转来的模型。
- **DV：** 用 PVE 三 Zone 环境（VM 300–303）。
  1. alice 配真实 key 做上游：建组 `peers` 并写策略；为 bob 的 DID 建账号放进 `peers`，打开 `provider_access`；为 `peers` 签发一个组 token 给 charlie，并给它充值。
  2. bob 用方式 L 添加，charlie 用方式 T 添加。
  3. 跑 xllm 或 OpenDAN Agent 对话，以及一次 embedding。
  4. 用 OpenAI SDK 拿 alice 名下的子账号 token 直连协议服务端（非流式）。
  5. 确认上游的用量和扣费、下游记录的成本。
  6. 停用上游的 Provider，确认返回真实错误且不自动换模型。

## 9. 风险

- 跨 Zone 的长 Immediate 连接可能被网关切断，要到第二期有了任务观察接口才有兜底。
- 合同降级表漏掉一项，就会出现“上游拒绝新字段”或“下游解析不了新枚举值”，合同快照测试是唯一的防线。
- 上游的额度或余额错误如果映射错了，下游 owner 会收到“你的额度用完了”这种误导提示。
- 上游管理员能看到下游用户的请求内容。
- API Token 泄露会直接消耗持有者的 Credit。
- 32 MiB 请求体上限限制了多图和长音频输入；对外开放后，它也是外部可以消耗服务内存的地方。
- AICC 不支持流式：只能发流式请求的外部软件用不了协议服务端；非流式的长回答也可能撞上外部软件自己的超时。

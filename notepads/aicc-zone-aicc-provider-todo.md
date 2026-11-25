# AICC 作为另一个 AICC 的 Provider TODO

日期：2026-11-17。状态：立项。本文是研究结论加方案草案，§4 待 owner 裁决，尚未实施。

目标：Zone A（下游）的 AICC 把 Zone B（上游）的 AICC 当成一个 Provider Instance。A 的应用、Agent、AIWorkspace 照常调用 A 的 AICC。A 的路由选中“来自 B 的模型”时，请求经跨 Zone kRPC 交给 B 的 AICC 执行，B 再用自己的 API key 调用真实厂商。开放给谁、开放哪些模型、给多少额度，都由 B 的管理员决定。

> **2026-11-23 修订**：AICC 不自动换模型或 Provider，Provider 不可用就修 Provider，fallback 只来自管理员手工配置（[aicc_router.md §9](../doc/aicc/aicc_router.md#9-fallback-与失败处理)、[改造 TODO](aicc-explicit-fallback-todo.md)）。本文原来写的“允许 failover”已按此改为“返回真实错误”。

相关文档：[Provider 冻结设计](../doc/aicc/frozen_provider_implementation.md)、[Trait/Protocol 冻结设计](../doc/aicc/frozen_trait_protocol.md)、[用户接口冻结文档](../doc/aicc/frozen_user_api.md)、[Token 额度](../doc/aicc/aicc_token_quota.md)、[跨 Zone 通信](../doc/sdk/cross-zone-communication.md)。

## 0. 结论先行

1. **能做，而且可以不改冻结架构的形状。** 上游 AICC 在下游就是三样东西：一个新的 Known Provider（暂名 `zone-aicc`）、一个新的 Protocol Adapter（`aicc-krpc`，wire 就是上游的公共 kRPC）、一个新的 discovery（读上游的模型列表）。不需要新的“大 Provider trait”。
2. **建议“精确模型直通”。** 下游的库存就是上游对它可见的精确模型。路由、管理员配置的 fallback、额度都在下游做，上游只执行固定的 exact model，不再路由第二次。不建议把逻辑模型（`llm.chat`）整个委托给上游。
3. **鉴权复用现有的跨 Zone 断言登录。** msg-center 和 HomeStation 已经在用 `CrossZoneClient`：下游用本机设备密钥签一个断言，上游 verify-hub 据此发 External 会话。上游要改两处：跨 Zone 白名单加入 `aicc`；AICC 不再一律拒绝 External 会话，而是查“对外授权”。
4. **第一期只做 Immediate**，覆盖 LLM、embedding、decision、图片等同步方法。流式和原生长任务（视频）的结果都在上游 TaskMgr 里，跨 Zone 读不到，需要上游 AICC 新增任务观察接口，放到第二期。
5. **媒体输入一律在下游转成字节内联**，因为上游读不到下游的 Named Object。这受上游 32 MiB 请求体上限约束。上游产出的 artifact 是上游 Zone 里的 Named Object，需要上游新开“按 obj_id 读取”的通道。
6. **最大的缺口在上游侧。** External DID 在上游没有账户，也没有 home bucket。用量归属、可见性、额度、产物存放都要为“外部调用者”补一类主体。

## 1. 场景与不做

可能的场景如下。主场景由 owner 定（Q1），它影响鉴权粒度和计费：

- **S1 家庭或朋友：** 一家人里一个 Zone 配了 API key，其他成员的 Zone 借用。
- **S2 组织：** 部门 Zone 统一使用总部 Zone 的 AICC，集中采购、集中审计。
- **S3 官方或商业：** 官方或服务商运行一个 BuckyOS AICC，向用户 Zone 出售推理。它可以替代或补充 sn-business 里现有的 LiteLLM 版 `sn_aicc`。
- **S4 同一用户的多个 Zone：** 例如 Lite 桌面节点或测试 Zone 借用主 Zone。

本期不做：

- 逻辑模型委托，即下游只发 `llm.chat`、由上游选模型。
- 多级链式转售（A→B→C）。默认上游不把“来自 AICC Provider 的模型”再开放给外部调用者，见 §3.10。
- 非 BuckyOS 调用方用 API key 调用 Zone 的 AICC。这是另一个需求（Zone 签发长期 key），系统里目前没有 kRPC 长期 key 机制。
- 上游的金钱结算。本期只做 token 额度和用量记录。

## 2. 现状（2026-11-17 研究结论）

| 事项 | 现状 | 位置 |
| --- | --- | --- |
| Provider 组成 | Known Provider、Provider Rules、Protocol Adapter、Discovery、Instance 五部分；新增 wire 走冻结流程 §7.2 | frozen_provider_implementation.md |
| 最接近的先例 | `sn` Provider：OpenAI 兼容 wire 加 `dynamic_login`（用设备 JWT 登录 SN） | `src/frame/aicc/src/provider/builtin/sn.rs`、`protocol/sn_openai.rs` |
| SN 后端 | sn-business 的 `sn_aicc` 基于 LiteLLM，不是 BuckyOS AICC。“AICC 作为 Provider”目前没有任何实现 | `sn-business/src/modules/sn_aicc` |
| 上游可用的 wire | 公共 kRPC `POST /kapi/aicc`，typed DTO 全部 `deny_unknown_fields`；artifact 读取走 `POST /kapi/aicc/artifact/open`，返回流式字节，鉴权用 Bearer 或 X-Auth | `src/kernel/buckyos-api/src/aicc_client.rs` |
| 模型列表 | `models.list` 返回无类型 JSON。每个模型带 `exact_model`、`model_driver_id`、`origin_model_id`、`provider_model_id`、`variant`、`api_types`、`capabilities`、`operations`。**不含** `canonical_fields`、价格、执行模式、健康状态；还附带很大的 `catalog` 和 `directory` | `service/mod.rs:1675`、`service/management.rs:728` |
| 可见性 | 普通视图的 `models.list` 已按调用者过滤 | `service/management.rs:820` |
| exact model 格式 | `<provider_model_id>[:variant]@<instance>`。`provider_model_id` 不允许含 `@`，实例名只允许 `[A-Za-z0-9-_.]` | `provider/mod.rs:594`、`provider/mod.rs:791` |
| 流式 | 公共接口没有流协议。`stream` 模式的增量写进 TaskMgr task data，调用方订阅 task event | `execution/mod.rs:2378`、`aicc_api设计.md` §2.5 |
| 长任务 | 返回 `status=running`，调用方经 TaskMgr 或 kevent 等待结束（workflow 用 `wait_for_task_end_kevent`） | `src/kernel/workflow/src/adapters/aicc.rs:396` |
| 现有调用方用法 | xllm 只用 Immediate | `src/frame/agent_tool/src/xllm.rs:5493` |
| 产物 | 上游把结果写进调用者的 `outputs` bucket，返回 `ResourceRef::NamedObject` | `resource/mod.rs:1409` |
| 费用 | `AiUsage.reported_cost` 和 Provider Rules 的 `reported_cost` 策略已经存在（OpenRouter 在用） | `catalog/schema.rs:958` |
| provider_state / thinking | 坐标是 `provider_profile_id`、`adapter_type`、`origin_provider`、`origin_model` 四项。`bind_provider_state_source` 会递归覆盖所有块的 `source`。`Thinking` 有 `provider_metadata` 字段可以放额外数据 | `protocol/provider_state.rs:3`、`aicc_client.rs:2406` |
| AICC 身份 | 对 External 会话一律返回 `resource_not_permitted` | `service/identity.rs:261` |
| 跨 Zone 登录 | `CrossZoneClient` 用设备密钥签 60 秒断言，对方 verify-hub 的 `login_by_cross_zone_assertion` 发 External 会话（access 15 分钟，refresh 7 天，整个会话最长 30 天）。先走 RTCP，失败回退 HTTPS，token 自动续期 | `buckyos-api/src/cross_zone_client.rs:189`、`cross_zone_auth.rs:561` |
| 跨 Zone 白名单 | `cross_zone_target_scopes` 只有 `msg-center` 和 `homestation`；`SessionCeiling::external()` 不含 `SessionScope::Aicc` | `buckyos-api/src/cross_zone_auth.rs:25`、`hybrid_sso.rs:138` |
| 长期 key、信任 Zone 列表 | 都没有。kRPC 没有 API key；Contact 只用于消息投递策略 | — |
| 上游 TaskMgr 记录 | 小请求的原文会写进上游 TaskMgr 的 task input（`TaskRequestView::Inline`） | `execution/mod.rs:1016`、`service/ports.rs:402` |

## 3. 方案

### 3.1 总体

```text
Zone A 应用 / Agent ──kRPC──> A.AICC ──路由──> exact model "openai-main/gpt-5.1:reasoning-high@zone-b"
                                                 │ aicc-krpc codec
                                                 ▼
                          CrossZoneClient（先 RTCP）──> B /kapi/aicc  chat.completions.create
                                                         exact_model = "gpt-5.1:reasoning-high@openai-main"
                                                         B.AICC ──> OpenAI
```

- **下游负责：** 路由和管理员配置的 fallback、本 Zone 用户的可见性和额度、用量记录、把产物存进自己的 bucket。
- **上游负责：** 对外授权（谁能用、能用哪些模型、多少额度）、真实调用、自己的用量记录。
- 上游收到的是固定的 exact model，不做二次路由。这就是 typed inference 现有的语义。

### 3.2 组件（按冻结流程 §7.2）

| 组件 | 内容 |
| --- | --- |
| Known Provider `zone-aicc` | 显示名“其他 Zone 的 AI”。连接字段是对方 Zone 的 DID 或主机名，不是普通 base_url。凭据 contract 是新增的 `cross_zone_assertion`，不需要用户输入密钥。`discovery_behavior_id = aicc-models`，`connection_behavior_id = aicc-cross-zone` |
| Provider Rules `zone-aicc` | 每个 api_type 绑定到同名公共 method 的 operation；声明 `reported_cost` 策略。不使用 `model_driver_overrides`，原厂身份由 discovery 给出（§3.3） |
| Protocol Adapter `aicc-krpc` | 新协议族，不从 OpenAI 派生。每个 operation 对应一个公共 typed method。codec 把内部 `AiccCall` 序列化成上游 DTO，把上游 typed response 解回 `ProtocolOutput`，错误按 §3.11 归一。注册自定义 `ArtifactDownloadProtocol`（POST 加 JSON body，属于冻结文档允许的“请求 method 不同”的情形） |
| Discovery `aicc-models` | 调用上游模型列表，产出 `DiscoveredModel`（§3.3、§3.4） |
| 连接与传输 | 请求经 `CrossZoneClient` 发送（先 RTCP、失败回退 HTTPS、自动登录和续期），不能用通用 `HttpTransport` 直连。现有链路是“codec → HttpTransport”，这里需要在 Provider 行为注册表里加一个传输注入点。登录、缓存、RTCP 回退都复用 `CrossZoneClient`，不复制 |
| Instance settings | 对方 Zone DID、以谁的身份调用（Q4）、enabled、超时 |

### 3.3 模型身份与命名

- **下游的 `provider_model_id`** 取 `<上游实例名>/<上游 provider_model_id>`，例如 `openai-main/gpt-5.1`。上游实例名不含 `/`，所以按第一个 `/` 拆分没有歧义；上游 `provider_model_id` 本身带 `/`（如 OpenRouter）也不影响。
- **下游的 exact model** 形如 `openai-main/gpt-5.1:reasoning-high@zone-b`。lowering 时还原成上游的 `gpt-5.1:reasoning-high@openai-main`。
- **原厂身份：** 上游已经给出 `model_driver_id` 和 `origin_model_id`，discovery 直接用它们匹配下游 Model Driver，不按模型名猜。
  - `DiscoveredModel` 目前没有原厂身份字段，有两种补法：加字段，或由 discovery 生成实例级 `model_driver_overrides`。倾向加字段，更直接。
  - 如果下游 catalog 里没有这个原厂模型（上游 metadata 比下游新），该模型不进入可执行库存，只保留诊断。这和现有规则一致。
- **variant：** 上游 typed inference 不接受 `provider_options`，所以下游不能靠 adapter options 设置 reasoning effort，只能选上游已有的 variant exact model。下游只保留上游实际列出的 variant。需要确认 `InventoryBuilder` 能否按 discovery 结果限定 variant 集合，不能就补上（Z0-5）。
- 同一原厂模型经上游多个实例出现（比如官方渠道和 OpenRouter 各一个），在下游是不同的 exact model，互不自动切换；需要备用时由下游管理员配置 fallback。

### 3.4 上游的模型列表接口

现有 `models.list` 缺字段，体积又太大。建议上游新增一个面向 Provider 的精简视图（新 method，或 `models.list` 的 `view: "provider"`）。

每个模型给出：

- 现有字段：`exact_model`、`model_driver_id`、`origin_model_id`、`provider_model_id`、`variant`、`api_types`、`capabilities`、`operations`。
- 需要补的字段：
  - `canonical_fields`：路由要求会用到，比如 TTS 的 voice；
  - 每个 operation 的执行模式：用来判断 Immediate 是否可用；
  - 价格：下游按 `cheap` 排序要用，`DiscoveredModel.pricing` 已经有位置；
  - 视频事实；
  - 可用状态。

列表顶层给出：

- 上游 AICC 的接口合同版本；
- 库存 revision。discovery 可以据此跳过没有变化的刷新，类似 SN 的 ETag。

对外部调用者，列表中去掉来自 `zone-aicc` 的模型，用于防链（§3.10）。

### 3.5 鉴权

推荐方案：复用跨 Zone 断言登录。

下游要做的：

- AICC 进程加载设备私钥（今天不加载），建一个 `CrossZoneClient`，以 `Principal { did, app_id: "aicc" }` 调用 `client.call(zone_b, "aicc", ...)`。
- `did` 用 Zone 身份（Zone owner DID）还是调用者本人的 DID，见 Q4。两种都能通过 `check_custody`（同 Zone 规则）。
- artifact 下载这类非 kRPC 请求，用 `CrossZoneClient` 取得的 bearer 放进 `Authorization` 头。

上游要做的：

- `cross_zone_target_scopes` 加 `"aicc" => [Aicc]`。`SessionCeiling::external()` 也要加入 `SessionScope::Aicc`，或者确认跨 Zone 断言会话不受这个 ceiling 限制（Z0-1）。
- `identity.rs` 对 External 会话不再一刀切。`auth_method = cross_zone_assertion` 且命中“对外授权”时，生成新的调用者主体 `CallerPrincipal::Peer { did, zone }`；没有授权仍返回 `resource_not_permitted`。
- AICC 自己校验 `token.target` 是 `aicc`，因为 `verify_session_token_opened` 不检查这一项。
- 核实 RBAC 的 `enforce_verified` 是否放行 External DID 访问 `system:aicc`，不放行就加规则（Z0-2）。

不选的方案：

- **上游给下游建本地账号并签发长期 key：** 系统里没有 kRPC 长期 key 机制，要另建一套，而且 key 要存在下游 settings 里。
- **S2S 加密 kRPC（`/s2s/<api>`）：** 有库，但没有任何服务和网关接入。

### 3.6 上游侧：外部调用者主体

External DID 在上游没有账户，而现有的可见性、额度、用量、产物都是按本 Zone 账户设计的。需要补以下几项：

- **对外授权表：** `did → enabled, note`。管理员在 AI Center 里添加“外部 Zone”。
- **可见性：** 新增主体 `peer:<did>`，允许项沿用 `all_providers`、`provider_all_models`、`provider_models`，默认什么都不开放。
- **额度：** 对象 `user` 的 `user_id` 直接使用外部 DID。需要核实规则校验是否要求本 Zone 账户（Z0-4）。建议管理员添加外部 Zone 时一并设一个周期额度。
- **用量：** `user_id` 记外部 DID。如果 Q4 选择转发实际用户，`operator_id` 记下游转来的用户。usage 页按外部调用者分组。
- **TaskMgr：** task creator 是外部 DID，需要核实 TaskMgr 是否接受（Z0-4）。
- **产物存放：** 外部 DID 没有 home bucket。建议上游把这类产物放在 AICC 自管的 bucket 命名空间 `peers/<did>/` 下，设保留期（例如 24 小时）。下游拿到结果后立即下载，存进自己调用者的 bucket。见 Q6。

### 3.7 资源

**输入：**

- 上游读不到下游的 Named Object；下游的 URL 也可能是内网地址或需要鉴权。所以 `aicc-krpc` 的所有资源输入都声明为 `BytesOnly`：下游先物化，再以 base64 内联。
- 受上游 32 MiB 请求体上限约束，而 base64 会膨胀到 4/3。大视频输入第一期不支持，返回明确错误。
- 公网 URL 是否直传，放到第二期（Q7）。

**输出：**

- 上游返回 `named_object`：扩展上游 `artifact/open`，让它接受 `obj_id`（只允许读调用者自己的产物）。下游通过自定义 `ArtifactDownloadProtocol` 读取字节，再按普通产物写进下游 bucket。
- 上游返回 `url`（上游登记过的 Provider URL）：同样经上游 `artifact/open` 读取。
- 上游返回 `base64`：直接使用。
- embedding 结果也以产物返回（`spec.embedding`），走同一条路径。

### 3.8 执行模式

| 情况 | 第一期 | 说明 |
| --- | --- | --- |
| Immediate | 做 | 上游返回 `succeeded` 和结果。下游超时要比普通 Provider 长（跨 Zone，且上游可能还会对同一模型重试），网关空闲超时要在 DV 里实测（Z0-7） |
| 上游返回 `running` | 排除 | 上游有些 operation 是原生长任务（视频、OpenAI background）。discovery 根据执行模式只保留能 Immediate 的组合。万一仍收到 running，按 provider_error 失败，并记下上游 task_id |
| NativeTask（视频等） | 第二期 | 需要上游新增任务观察接口：`task.get` 返回状态和最终结果，权限限于任务创建者；`cancel` 已有。下游 `NativeTaskCodec` 的 submit / status / result / cancel 映射到这些方法 |
| Stream | 第二期 | 需要上游新增流式数据端点，形式类似 `artifact/open`，例如 `POST /kapi/aicc/task/events`（NDJSON，带游标），把上游 task data 里的 delta 推回来，下游转成 `ProtocolEvent::Delta`。现有调用方（xllm）只用 Immediate，所以优先级低 |

- **幂等：** 下游把自己的 `<task_id>:<attempt>` 作为上游的 `idempotency_key`，连接中断后重发不会重复计费。需要确认上游用同一个 key 重放已完成任务时会返回原结果（Z0-6）。
- **取消：** Immediate 请求进行中，下游断开连接并不会取消上游。下游取消时如果已拿到上游 task_id，就调用上游 `cancel`。

### 3.9 provider_state 与 thinking

- 上游返回的块带着上游坐标，而下游的 `bind_provider_state_source` 会把它覆盖成下游坐标（`zone-aicc` / `aicc-krpc` / 原厂 / 模型）。
- 下游把这些块回放给同一个 exact model 时，上游必须看到原坐标才会原样回放。做法：
  - **provider_state：** `aicc-krpc` decode 时把上游原块整个包进下游 `provider_state.value`，保留上游的 `source`；encode 时（目标仍是 `aicc-krpc`）解包还原。上游自己按它的四元组判断是本源块还是外来块。
  - **thinking：** 上游坐标放进 `Thinking.provider_metadata`，encode 时还原。
- 下游坐标不含上游实例名，所以“原厂和模型相同、上游实例不同”的两个下游 exact model 坐标一样。这不会出错：下游只负责放行，最终判断由上游做。

### 3.10 防环与链

- **默认不允许链：** 对外部调用者（Peer 主体），上游既不暴露也不执行 `provider_profile_id = zone-aicc` 的模型。这样即使 A 和 B 互相配置，也不会成环。
- **再加一道保险：** 增加一个 kRPC 顶层保留参数 `aicc_peer { origin_zone, hop }`，做法同 `on_behalf_of`：由 `AiccServerHandler` 取出，不进 DTO。上游看到 hop 超限就拒绝。
- 以后是否支持链，见 Q5。

### 3.11 错误映射

| 上游错误 | 下游处理 |
| --- | --- |
| `quota_exhausted`、`quota_budget_exceeded`、`budget_exceeded` | **不能原样透传。** 否则下游调用方（xllm、OpenDAN）会以为是用户自己的额度用完，并去提示 owner。要映射成 provider 错误（上游额度不足），details 带上游的 reset 时间；该 Provider 的健康状态标记为额度耗尽，直到 reset |
| `resource_not_permitted` | 上游收回了授权：按 provider 错误处理，并触发一次库存刷新 |
| `context_length_exceeded` | 原样透传，语义相同且不可重试 |
| `invalid_request`、`schema_validation_failed`、`invalid_method`、`unsupported_*` | 下游已经校验过还被拒，基本是两边版本不一致：按 provider 错误处理，Provider 健康状态记为“合同不兼容” |
| `provider_error`、`timeout`、`no_provider_available` | 透传 `retriable` 和 retry-after；只对同一模型重试 |
| kRPC `NoPermission`、token 错误 | 凭据错误，Provider 标为 unhealthy。`CrossZoneClient` 已有一次自动重登 |
| 网络不可达 | 和普通 Provider 的网络错误一样 |

### 3.12 版本差异

- 两个 Zone 的 AICC 版本不会一样，而 DTO 是 `deny_unknown_fields`，Beta 2.2 也不做兼容别名。好在可选字段大多是 `skip_serializing_if = None`，只有下游真正用到了上游不认识的新字段才会被拒。
- 精简模型列表返回上游的合同版本，下游把它记进 health，被拒时提示对方升级。不做字段降级。

### 3.13 隐私

- 下游用户的提示词和图片会进入上游：上游 TaskMgr 会保存小请求的原文，上游 trace 和 usage 也有记录，上游管理员都能看到。
- 添加这类 Provider 的界面要明确告知。是否要求上游对 Peer 主体的任务只存摘要，见 Q8。

### 3.14 管理界面

- **下游**（AI Center › 添加 Provider）：新增“其他 Zone 的 AI”，输入对方 Zone 的 DID 或域名，选择身份方式。保存前用 `provider.validate` 实际登录一次并列出模型；失败时提示“对方还没有授权你”。
- **上游**（AI Center 新增“外部 Zone”页）：添加 DID，设置开放范围（复用可见性编辑器）和额度（复用额度规则），查看用量。

## 4. 待 owner 裁决

| # | 问题 | 建议 |
| --- | --- | --- |
| Q1 | 主场景是 S1–S4 中的哪个？ | — |
| Q2 | 路由：精确模型直通，还是逻辑模型委托？ | 精确模型直通 |
| Q3 | 鉴权：跨 Zone 断言登录，还是由 Zone 签发长期 key？ | 跨 Zone 断言登录 |
| Q4 | 上游看到的身份：Zone 身份（Zone owner DID），还是每个调用者本人的 DID？后者要求上游逐个授权，但能按人计量 | 第一期用 Zone 身份 |
| Q5 | 是否允许链（A→B→C）？ | 第一期禁止 |
| Q6 | 上游产物放在 AICC 自管的 peer 命名空间并设保留期？ | 是，保留 24 小时 |
| Q7 | 第一期输入是否全部内联字节、不支持大媒体？ | 是 |
| Q8 | 上游对 Peer 任务是否只存摘要？ | 是。按“先能力再安全”也可以先不做，记为已知风险 |
| Q9 | 新增的公共接口（精简模型列表、`artifact/open` 支持 obj_id，第二期的 `task.get` 和任务事件流）都属于冻结合同变更，要按冻结规则同步文档、SDK 和 TS。是否同意？ | — |
| Q10 | 命名：profile `zone-aicc`、adapter `aicc-krpc`、显示名“其他 Zone 的 AI”，是否可以？ | — |

## 5. 任务

### Z0 验证（先做，结论写回本文）

- [ ] Z0-1 把 `aicc` 加进跨 Zone 白名单后，External 会话能否通过 `enforce_opened(..., Aicc)`；`SessionCeiling::external()` 要不要加 `Aicc`。
- [ ] Z0-2 External DID 调用 `system:aicc` 时 RBAC 的结果。
- [ ] Z0-3 AICC 进程能否 `load_device_private_key`：权限如何；多节点 Zone 里用哪台设备签名。
- [ ] Z0-4 TaskMgr 是否接受外部 DID 作为 creator；额度规则的 `user` 对象是否要求本地账户。
- [ ] Z0-5 `InventoryBuilder` 能否按 discovery 限定 variant；给 `DiscoveredModel` 加原厂身份对 inventory 构建的影响。
- [ ] Z0-6 上游用同一个 idempotency_key 重放已完成任务时返回什么。
- [ ] Z0-7 跨 Zone 的长 Immediate 请求（5–10 分钟）经 RTCP 和 HTTPS 网关会不会被断开。

### Z1 上游（被调用方）

- [ ] Z1-1 跨 Zone 白名单加 `aicc`；identity 生成 `CallerPrincipal::Peer`；校验 `token.target`。
- [ ] Z1-2 对外授权的存储和管理 method（`peer.list` / `add` / `update` / `delete`，仅管理员）。
- [ ] Z1-3 可见性主体 `peer:<did>`；额度对象支持外部 DID；用量归属。
- [ ] Z1-4 精简模型列表（§3.4）；对 Peer 去掉 `zone-aicc` 模型。
- [ ] Z1-5 Peer 产物命名空间和保留期清理；`artifact/open` 支持 `obj_id`。
- [ ] Z1-6 保留参数 `aicc_peer`（hop）。
- [ ] Z1-7 Peer 任务只存摘要（等 Q8 定了再做）。

### Z2 下游（调用方）

- [ ] Z2-1 metadata：`zone-aicc` 的 known provider 和 provider rules。
- [ ] Z2-2 `aicc-krpc` adapter plugin：每个 Immediate typed method 一个 operation；encode / decode；provider_state 包装（§3.9）；错误映射（§3.11）；自定义 `ArtifactDownloadProtocol`。
- [ ] Z2-3 `CrossZoneClient` 传输注入（connection behavior）；凭据 contract `cross_zone_assertion`。
- [ ] Z2-4 `aicc-models` discovery：`DiscoveredModel` 原厂身份、variant 限定、价格、`canonical_fields`、视频事实。
- [ ] Z2-5 资源输入 `BytesOnly`；输出产物下载后存入本地 bucket。
- [ ] Z2-6 幂等 key、取消；在 route trace 里记上游 task_id 和上游 exact model。
- [ ] Z2-7 settings、`provider.validate`、`provider.add` 对新 profile 的校验。

### Z3 界面

- [ ] 下游“添加 Provider”表单；上游“外部 Zone”页；usage 页增加外部调用者。

### Z4 第二期

- [ ] 上游 `task.get` 和任务事件流端点；下游 NativeTask 与 Stream codec；视频等长任务。
- [ ] 公网 URL 直传；链（如果 Q5 放开）。

### Z5 文档

- [ ] 同步更新：`frozen_provider_implementation.md` 的内置表、`frozen_trait_protocol.md`（新 method 和 endpoint）、`frozen_user_api.md`、`aicc_token_quota.md`（Peer 主体）、provider operation bindings golden、`doc/sdk/cross-zone-communication.md`（AICC 加入白名单）。

## 6. 测试与 DV

- **合同测试：** 同一进程起两个 AICC。上游用 `AiccServerHandler` 加一个假 Provider，下游用 mock transport 指向它。覆盖 discovery、命名还原、每个 Immediate method、§3.11 的错误映射、provider_state 回放、产物下载、幂等重发。
- **鉴权测试：** 未授权的 External 会话返回 `resource_not_permitted`；授权后可见性和额度生效；token 过期后能重新登录。
- **防环测试：** A、B 互相配置，确认 discovery 里不出现对方转来的模型。
- **DV：** 用 PVE 三 Zone 环境（VM 300–303）。alice 配真实 key 做上游，bob 添加 Provider。跑一轮 xllm / OpenDAN Agent 对话、一次图片生成、一次 embedding，确认上游的用量和额度可见；断开上游，确认返回真实错误且不自动换模型，配置了 fallback 时才切换。

## 7. 风险

- 跨 Zone 的 Immediate 长连接可能被网关切断，要到第二期有了任务观察接口才有兜底。
- 版本不一致造成的 schema 拒绝无法自动修复，只能提示升级。
- 上游额度错误如果映射错了，下游 owner 会收到“你的额度用完了”这种误导提示。
- 上游管理员能看到下游用户的请求内容。
- 32 MiB 请求体上限限制多图和长音频输入。

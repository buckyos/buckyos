# AICC 用户视角的模型路由改进 TODO

日期：2026-11-27。状态：2026-11-28 已实施（未提交、未部署、真实 Zone DV 未跑），实施记录见 §11。owner 答复了 §9 的 Q1–Q13 和 §9.1 的 N1、N2，全部已实施。

基线：`fcd23bfb4`，加上当天工作区里未提交的 AICC 改动（百炼改名等，不涉及路由）。下文行号按这份工作区记录，动手前先复核。

本文**并入并取代** [AICC 去掉自动换模型、改为显式 fallback TODO](aicc-explicit-fallback-todo.md)（11-23 立项，代码一行没动）：

- 它的 T1–T3（去掉执行期切换、健康状态不参与选择、去掉内置 fallback）成为本文的 R0；其中删除 `llm.fallback` 一项按 Q13 改为保留。
- 它的 T4（`target_exact` / `target_logical`）被本文的三种“不可用时”策略取代（§3.4）。
- 它的 T5、T6 并入本文的 R3、R6、R7。
- 它 §5 的 Q1（触发条件）、Q3（保留内置权重）、Q5（移除 `parent` 与 `allow_exact_model_fallback`）、Q6（可选参数不参与选模型）按它的建议执行，除非 owner 在 §9 另行说明；Q2、Q4、Q7 被本文的 Q1、Q12、Q13 取代。

规范以 [aicc_router.md](../doc/aicc/aicc_router.md) §9–§12 为准；本文实施后要同步修订这几节（§8）。

---

## 1. 目标与原则

### 1.1 owner 原话（2026-11-27）

> 站在用户角度的 AICC 模型路由改进
>
> - 核心是支持用户理解并能正确调整 usecase（默认路由配置 → 管理员配置 → 用户配置）
> - 添加 Provider 的首要目标是覆盖所有的 usecase
> - 默认路由是稳定的，usecase → model@provider，正常情况下，只有添加或删除 Provider 会影响，Provider 不可用不会影响
> - 通过手工配置：
>   - 可以允许某个 Provider 不可用后，路由到下一个提供同模型的 Provider，这种情况适用于多个 Provider提供同一个模型
>   - 可以允许某个 Provider 不可用后，按其 disable 的逻辑到下一个不会更贵的模型
>   - 可以允许某个 Provider 不可用后，按其 disable 的逻辑路由下一个模型
> - 用户可以配置多个路由策略组（包含多个 usecase 的配置），并可以按 scope 应用。比如特定 App 应用，比如特定 LLM session（含 child-session）应用

### 1.2 继承的原则

- **11-23 的路由原则仍然有效**（router §9.1）：相同配置、库存和请求得到相同的模型与 Provider；Provider 不可用就修 Provider；系统不自动换模型；换模型只来自手工配置。本文的三种策略就是这里说的“手工配置”。
- **默认允许 breaking change，不做旧格式兼容**（仓库 `AGENTS.md`）。
- **机制越少越好。** 能用已有部件表达的不加新机制；每个新机制在 §9 列出不加它的选项。
- **体验按手机评估。** 目标用户很多没有 PC，AI Center 的用途表、策略组和添加 Provider 的覆盖步骤都要在手机宽度下好用。

### 1.3 我对原话的理解（owner 已确认，裁决见 §9）

- **usecase（下文称“用途”）就是逻辑目录里 kind 为 `task` 的节点**，来自 `service/model_defaults.rs:594` 起的 `builtin_logical_model_definitions()`，共 29 个：
  - `decision`
  - `llm.chat`、`llm.plan`、`llm.code`、`llm.swift`、`llm.summarize`、`llm.translate`、`llm.vision`
  - `embedding.text`、`embedding.multimodal`、`rerank`
  - `image.txt2img`、`image.img2img`、`image.inpaint`、`image.upscale`、`image.bg_remove`
  - `vision.ocr`、`vision.caption`、`vision.detect`、`vision.segment`
  - `audio.tts`、`audio.asr`、`audio.music`、`audio.enhance`
  - `video.generate`、`video.edit`、`video.extend`、`video.upscale`
  - `agent_runtime.computer_use`

  `llm.fallback` 保留为“兜底 LLM”：不算普通用途，默认关闭，管理员打开后为没有模型的 LLM 用途兜底（Q13，§3.6）。规格和家族目录（如 `llm.gpt-mini`、`image.txt2img.flux`）仍可以被请求直接使用，但不出现在用途表里，也不能单独配置。
- **“路由”是一张表：用途 → 一个精确模型 `model@provider`。** 用户在 AI Center 看到的、能调整的就是这张表；权重、规格、家族这些内部结构只在高级视图里出现。
- **“按其 disable 的逻辑”**指：把出故障的那个 Provider 实例视作停用，路由重新算一次，得到的就是“下一个候选”（Q2：一律按 Provider 停用，不区分是不是单个模型的故障）。所以备选目标完全由现有的默认路由逻辑推出来，不需要再单独配置 fallback 目标。三种策略因此取代 `target_exact`、`target_logical` 和 `parent`（Q10）。
- **“路由策略组”和用户层是同一种内容**：一组“用途 → 选择 + 不可用策略”的配置。区别只在于用户层总是生效，策略组只在绑定的 scope 内生效。
- **“覆盖所有的 usecase”**指：每个用途至少有一个能用的精确模型。添加 Provider 时要让用户看到“加上它以后，哪些用途有了模型、哪些用途换了模型、哪些仍然没有”。

---

## 2. 现状（2026-11-27 核对）

| 方面 | 现在 | 差距 |
| --- | --- | --- |
| 配置层 | 注册表支持 factory / system / user / session 四层（`model/mod.rs:400-405`、`:456-459`）。生产路径只用 factory 和 system：`ServiceModelAssembler` 把系统配置 `settings.session_config` 放进 **session 槽**（`service/model_defaults.rs:526-534`、`service/settings_runtime.rs:242-245`），`user` 槽只在测试里用 | 没有用户层；系统配置的字段名叫 `session_config`，和请求里的 session overlay 名字相同、含义不同 |
| 请求携带的 overlay | 请求里的 `session_overlay` 叠加在系统注册表上，每次请求克隆整个注册表（`service/inference.rs:141-147`，`model/mod.rs:478`）。但路由策略求值时，请求的 overlay **替换**系统配置，而不是叠加（`service/inference.rs:214-217`） | 请求只要带一个 overlay，系统配置里的 `policy`（包括 `locked` 的值）就失效。这是现有缺陷，R1 一起修 |
| 命名 profile | `AiccRouteOverlay` 已有 `logical_profiles` + `active_logical_profile`（`buckyos-api/src/aicc_client.rs:2112` 起），但只能放在同一份 overlay 里，由发请求的一方选，没有 scope 绑定 | 不能“给某个 App / 某个会话用某组配置” |
| 自动切换 | 11-23 TODO §2 的 G1–G8 全部仍在：执行期候选循环（`execution/mod.rs:1635-1638`）、额度拒绝后换候选（`service/inference.rs:1169-1225`）、健康状态参与过滤（`routing/mod.rs:1013` `hard_filter_runtime`，`health.rs:4-7` 的冷却与熔断）、`runtime_failover` 与 `allow_fallback` 默认 `true`（`routing/policy.rs:95`、`:97`，`buckyos-api/src/aicc_client.rs:2269`、`:2273`）、七个 LLM 用途回退到 `llm.fallback`（`model_defaults.rs:966`） | 违反“Provider 不可用不影响路由” |
| 健康状态怎样改变路由 | 运行状态过滤写在展开阶段的 `accept` 里（`routing/mod.rs:565-588`），目录被过滤空以后，注册表沿 fallback 链继续找（`model/mod.rs:786` 起） | 故障会让同目录的低权重组、以及 `llm.fallback` 被选中 |
| 池内排序 | `cost_first` 用请求的 token 估算 × 库存价格算成本（`service/inference.rs:1410-1430`），还有 session 历史软优先（`routing/mod.rs:1156-1159`） | 同一个用途可能因为请求大小（阶梯价）或会话历史选到不同模型 |
| 修改入口 | `routing.update` 只收 `provider_weights` 和 `routing_commands`（`service/management.rs:257` 起），权限是 `write` `RESOURCE_SETTINGS`。Desktop 路由页（`frame/desktop/src/app/ai-center/RoutingPage.tsx`）只能存 `item_weight` 命令，fallback 只显示不能改；导航里路由页 `adminOnly: false`，但它改的是整个 Zone 的配置。websdk 的 `RoutingUpdateRequest` 不能发 `routing_commands`（`buckyos-websdk/src/aicc_client.ts:489`、`:579`）。CLI `buckyos ai routing` 只读 | 没有“按用途选模型”的入口；没有用户自己的入口；没有不可用策略的入口 |
| 覆盖 | `provider.validate` 会算出“只有这个 Provider 时每个 LLM 用途的候选”（`service/settings_runtime.rs:588-620`，只算 LLM、只算这一个 Provider）。添加向导只显示 `llm.chat` 的候选数量（`frame/desktop/src/app/ai-center/components/providers/wizard/StepValidation.tsx:148`），确认页没有路由信息 | 看不到“加上它以后整个 Zone 的用途表” |
| 会话 | 请求里只有不透明的 `session_id`，AICC 只用它记上一次选中的模型（`storage/mod.rs:343-390`）。Agent 对话路径根本不传 `session_id`（`frame/agent_tool/src/xllm.rs:5534-5563`，`frame/llm_context/src/deps.rs:23-44`）。子会话关系只在 lib_opendan：`Origin.parent_session`（`frame/lib_opendan/src/protocol/config.rs:70-92`），沿链计算深度（`api.rs:832-847`），`create_sub_session`（`api.rs:1053`、`:1129`） | AICC 不知道会话之间的父子关系 |
| App 身份 | `AuthorizedCaller.app_id`（`service/mod.rs:593-601`），由 RPC 会话令牌得出：普通 App 是 AppId，Agent 令牌是 agent_id（`service/identity.rs:271-286`），同一个 Agent 的所有会话共用一个 app_id | 可以直接做 App scope |
| 价格 | 库存价格来自 Provider 配置或发现（`InventoryPricing`），有价格的只有 12 份内置 Provider 配置；调用前用 `estimate_model_cost` 估算、`catalog.cost_in_usd` 折算（`service/inference.rs:1410-1430`） | 很多模型没有价格，“不会更贵”要定义价格未知时怎么办（Q3） |
| 存储 | AICC RDB `STORAGE_SCHEMA_VERSION = 3`，版本不符时拒绝启动（`storage/mod.rs:29`、`:319-326`）。可见性策略存在 `aicc_visibility_policy`，以 `type:` / `group:` / `user:` 主体为键 | 新增表要升存储版本，和用户策略 TODO 的 Q14 一起处理 |
| 默认打开 fallback 的调用方 | `frame/aiworkspace/aiws/host.js:31` 的 `MEDIA_POLICY`；`tools/buckyos-agent/lib/aicc.ts:144-145`（`rootfs/libexec/buckyos-agent/lib/aicc.ts` 是同一份拷贝） | R0 删除 |

### 2.1 能复用的部件

- **“停用某个 Provider 再算一次”**：路由策略已有 `blocked_provider_instances`（`routing/policy.rs:73` 起），在请求的策略上追加被停用的实例，就是“按其 disable 的逻辑”。
- **管理员锁定**：`LockedValue<T>`（`buckyos-api/src/aicc_client.rs:1930`）已支持 `"x"` 和 `{"value": "x", "locked": true}` 两种写法，策略合并时已经遵守 `locked`（`routing/policy.rs` `apply`）。
- **预览与解释**：`routing.preview`（每个路径的选中模型与 trace）、`trace.query`、`RoutingPage` 的候选树。
- **覆盖计算**：`provider.validate` 的 `routing_candidates`。
- **可见性**：`access.allows_model`（`visibility.rs`），备选模型同样要经过它。
- **事件**：`AiccEventLog`（`service/events.rs`）与 `events.list`，Desktop 首页已有事件卡片。
- **RDB 主体键的写法**：`aicc_visibility_policy` 的 `user:<id>` 等。
- **会话链**：lib_opendan 的 `Origin.parent_session` 与沿链遍历。

---

## 3. 用户看到的模型

### 3.1 用途表

任何能使用 AI 的用户打开 AI Center 的“用途”页，看到的是**对自己生效的**用途表。每一行：

| 用途 | 当前模型 | 来源 | 不可用时 | 状态 |
| --- | --- | --- | --- | --- |
| 编码 `llm.code` | `gpt-codex-5.2@openai` | 管理员 | 同模型换 Provider → `gpt-codex-5.2@openrouter` | 已覆盖 |
| 总结 `llm.summarize` | `gpt-5.5-mini@sn` | 默认 | 不切换 | 已覆盖 |
| 规划 `llm.plan` | `gpt-5.5-mini@sn` | 兜底 LLM | 不切换 | 未覆盖：没有满足上下文要求的模型，由兜底 LLM 提供 |
| 生成视频 `video.generate` | — | — | — | 未覆盖：没有接入能生成视频的模型 |

- “来源”是决定这一格的那一层：默认 / 管理员 / 我 / 策略组名。被管理员锁定的格子显示锁，不能改。
- “不可用时”同时显示策略和**按现在的配置会换到谁**（假设当前模型所在的 Provider 不可用时算出来），让用户在出事之前就知道会发生什么。
- 用途表可以切到某个策略组的视角（“这个组生效时是什么样”），也可以切到某个 App 的视角。
- 管理员另有一个“Zone 默认”视角，编辑的是管理员层。

### 3.2 稳定性契约

```text
用途 U 在 scope S 下的路由 = f(默认配置, 管理员层, 用户层, S 绑定的策略组,
                             已接入并启用的 Provider 集合及其静态库存, Model Driver metadata)
```

一次具体请求在此之上只再受**请求自身的硬约束**影响（tool call、视觉、上下文长度、`local_only` 等“没有就做不了”的要求），而且结果是确定的。

**不影响路由的东西**：健康状态、熔断、冷却、延迟观测、上游配额余量、动态成本估算、请求的 token 数、可选的调节参数（11-23 TODO Q6）。会话历史见 §9.1 N2。

**会改变默认路由的事件**：添加、删除、启用、停用 Provider；Provider 的模型列表刷新、Model Driver metadata 更新、管理员的权重命令。后几类通常是常规的默认模型升级，是用户期待的，直接生效，不钉住、不要求确认（Q1）。Provider 故障不在其中；管理员手工把 Provider 标记为故障也不改变路由，只改变执行（§3.4）。

### 3.3 层次与优先级

```text
默认配置 < 管理员层 < 用户层 < App 绑定的策略组 < 会话绑定的策略组
```

- 每一格（某个用途的“模型”或“不可用时”）由设了它的最高一层决定；没设就跟随下一层。
- 管理员层里 `locked` 的格子，任何更高的层都不能改（沿用 `LockedValue`）。
- 请求只能**收紧**：关闭不可用策略、`local_only`、屏蔽 Provider、加硬性能力要求。请求不能选用途的模型、不能打开不可用策略（Q10）。请求直接写精确模型 `model@provider` 时绕过用途表，这一点不变，也从不切换。
- 所有层选的模型都必须对调用者可见；不可见的模型在用途表里标“你无权使用”，路由跟随下一层（Q8）。

### 3.4 不可用时的三种策略

每个用途有一个 `on_unavailable`，默认 `none`：

| 值 | 界面文案 | 备选怎么得出 |
| --- | --- | --- |
| `none` | 不切换 | 不换，返回真实错误 |
| `same_model` | 同一个模型换 Provider | 与当前模型身份相同（`model_driver_id` + `origin_model_id`）、在别的 Provider 实例上、静态可用且对调用者可见的精确模型；顺序按 Provider 权重降序、再按 Provider 在配置里的顺序 |
| `not_pricier` | 换下一个不更贵的模型 | 停用原 Provider 后重新路由；结果比当前模型贵或价格未知，就把它也排除再算，直到找到一个不更贵的或没有候选（§4.5） |
| `next_model` | 换下一个模型 | 停用原 Provider 后重新路由，得到的第一个结果 |

三种策略都把原模型所在的 Provider 实例视作停用再找备选（Q2），不区分是整个 Provider 故障还是单个模型故障。所以只接了 SN 一家时没有任何备选，这是预期的——单 Provider 没有可换的。

**执行规则**（沿用 router §9.2、§9.3）：

1. 先对当前模型做同模型重试和同 Provider 的重新登录。
2. 仍然失败、属于触发类别、且 `on_unavailable` 不是 `none`，才换到备选，**每次请求最多换一次**（Q4）。触发类别沿用 router §9.2：重试后仍超时 / 5xx / 连接失败、模型不存在或已下线、上游配额耗尽、用户额度规则拒绝该模型。
3. 不触发：400、401、403、409、取消、已产生用量的失败、异步任务已经拿到 remote task ID 之后的失败。
4. 备选也失败就返回错误，错误里同时给出原模型和备选模型的失败原因。
5. 备选同样受原请求的能力、隐私、`local_only`、可见性和额度约束。
6. 系统不根据健康状态提前跳过原模型（Q5）。Provider 长期故障由管理员停用它（默认路由随之重算）；临时故障可以手工**标记为故障**：路由和用途表不变，请求不再发给它，配置了策略的用途直接用备选，没配置的立即返回“管理员已标记该 Provider 故障”（§4.4）。

### 3.5 路由策略组与 scope

- 用户可以建多个策略组，每组是一份“用途 → 选择 + 不可用策略”，内容和用户层相同。
- 绑定到 **App**：通过这个 App（或 Agent，`app_id = agent_id`）发出的调用生效。
- 绑定到 **LLM 会话**：这个会话和它的所有子会话（沿 `parent_session` 链向下）生效；子会话自己绑了别的组就用自己的。
- 同一个调用同时命中 App 绑定和会话绑定时，会话绑定优先。
- 策略组属于建它的用户，只对这个用户的调用生效。第一期管理员不建共享组、不替所有用户给 App 绑组（Q8）。

### 3.6 兜底 LLM（`llm.fallback`，Q13）

- 用途表里保留一行“兜底 LLM”，默认关闭，只有管理员能打开。打开的含义是：这个 Zone 至少有一个 LLM 可以用来维护系统。
- 打开后，七个 LLM 用途（`llm.chat`、`llm.plan`、`llm.code`、`llm.swift`、`llm.summarize`、`llm.translate`、`llm.vision`）中**静态上没有候选**的，由兜底 LLM 提供。已覆盖的用途不受影响；Provider 故障也不会转到兜底 LLM（故障走 §3.4）。
- 兜底 LLM 本身像普通用途一样展开，内置的 38 个小模型规格（`model_defaults.rs:1143` 起）是它的默认内容；各层也可以给它选模型、设不可用策略。经兜底提供的请求使用兜底 LLM 这一行的配置。
- 请求也可以直接使用 `llm.fallback`。
- 经兜底提供必须看得见：响应的 `route_trace` 标出“由兜底 LLM 提供”和原用途；使用它的会话显示警告（例如“规划没有配置模型，正在使用兜底模型 X”）；用途表里该用途显示“未覆盖，由兜底 LLM 提供”。

---

## 4. 设计

### 4.1 用途配置的数据形状

管理员层、用户层、策略组用同一种内容：

```json
{
  "usecases": {
    "llm.code": {
      "model": { "value": "gpt-codex-5.2@openai", "locked": true },
      "on_unavailable": "same_model"
    },
    "llm.summarize": { "on_unavailable": "not_pricier" },
    "image.txt2img": { "model": "flux-2-pro@fal" },
    "llm.fallback": { "enabled": true }
  }
}
```

- `model`：精确模型；不写表示跟随下一层（Q7 讨论是否允许只选模型、不选 Provider）。
- `on_unavailable`：`none` / `same_model` / `not_pricier` / `next_model`；不写表示跟随下一层。
- 两个字段都用 `LockedValue`，只有管理员层的 `locked` 有效，其它层写了 `locked` 校验时拒绝。
- `enabled`：只用于 `llm.fallback`，只能写在管理员层（§3.6）；写在别的用途或别的层时校验拒绝。
- 字段只是配置，不保证资源存在。所选模型不在库存里时配置保留，用途表标“所选模型已不可用”，这一格跟随下一层，并发事件通知（不迁移到别的模型）。
- 校验：用途必须存在；模型的 API 类型必须和用途一致，且满足用途自身的硬要求（`min_line`、`tool_json_requirement` 等）。

**存放：**

| 层 | 位置 |
| --- | --- |
| 默认 | 代码：`builtin_logical_model_definitions()` + `builtin_logical_tree_overlay()`，不变 |
| 管理员层 | system-config `services/aicc/settings`：`session_config` 改名为 `routing`，新增 `usecases` 字段；`routing_commands`、`provider_weights` 保留在这一层，作为管理员的高级调权手段（Q11） |
| 用户层 | AICC RDB 新表 `aicc_routing_layer`，主体 `user:<user_id>` |
| 策略组 | AICC RDB 新表 `aicc_routing_group`（`group_id`、`owner`、`name`、`content_json`、`version`、`updated_at_ms`） |
| App 绑定 | AICC RDB 新表 `aicc_routing_binding`（`owner`、`scope = app:<app_id>`、`group_id`） |
| 会话绑定 | lib_opendan 的会话配置（§4.6，Q9） |

新增表要把 `STORAGE_SCHEMA_VERSION` 升到 4，会让已部署的 Zone 起不来（2026-11-28 owner：这个版本是 breaking change，用户统一重新安装）；与 [用户策略与 Credit TODO](aicc-user-policy-credit-todo.md) 的 Q14 合并成同一次升级，不要各升一次。

### 4.2 求值

```text
resolve(用途 U, 调用者 C, 请求 R):
  cfg = 按 §3.3 合并 U 在 [默认, 管理员, 用户(C.user), App 组(C.user, C.app), 会话组(R.route_group)] 上的两格
        （每格记下来源层和是否被锁）
  静态可用(m) = m 通过静态硬过滤（库存、能力、operation、Provider 已启用、凭据已配置、隐私、
               信任、预算、可见性）且满足 R 的硬约束
  if cfg.model 已设 且 静态可用(cfg.model):       primary = cfg.model，来源 = cfg.model 的层
  elif 静态展开(U, R) 非空:                       primary = 展开结果，来源 = 默认
  elif U 是七个 LLM 用途之一 且 兜底 LLM 已打开:  return resolve(llm.fallback, C, R)，并标记“由兜底 LLM 提供”
  else:                                           无候选（no_candidate_model，带用途名）
  return primary, cfg.on_unavailable
```

- “静态展开”就是 router §10.4 的两阶段算法，只用静态条件（R0 的 T2）。池内排序见 Q12 与 §9.1 N1。
- 用户层、策略组只改两格，**不再克隆注册表**：`with_session_overlay` 的每请求克隆（`service/inference.rs:141-147`）随请求 overlay 一起删除（Q10）。
- 路由策略（`policy`）的层次同样是 默认 < 管理员 < 请求（请求只能收紧），修掉 `service/inference.rs:214-217` 的“替换”。
- 规格、家族路径的请求不读用途配置，按静态展开，结果同样确定。

### 4.3 默认路由不钉住（Q1）

默认路由每次按静态输入计算，不存快照。Provider 刷新模型列表、metadata 更新带来的变化直接生效。

添加 Provider 的向导仍然要预演“加上以后”的用途表（R5）：用现有库存加上新 Provider 的库存构建一次注册表，对每个用途跑 §4.2。`provider.validate` 现在已经这样为单个 Provider 构建注册表（`service/settings_runtime.rs:588-620`），扩展成“全部现有库存 + 新的”即可。

### 4.4 不可用时的执行

替换现在的候选循环（`execution/mod.rs:1635-1638`）和 `inference.rs` 的 `failover` 列表：

- 路由阶段只产生 `primary`，同时把 `on_unavailable` 和“备选怎么算”所需的输入（用途、请求硬约束、调用者）带进执行请求；**不预先**算好一串备选。
- 执行阶段在 §3.4 的触发条件成立时才计算备选：把原 Provider 实例加入停用集合 → 按策略算出一个备选（`same_model` 直接筛；`not_pricier` / `next_model` 用“请求策略 + `blocked_provider_instances`”重新路由一次）→ 重新做额度准入、资源物化、可见性检查 → 执行一次。
- 备选的计算复用路由器，不复制一份展开逻辑。
- 异步任务（视频等）只在拿到 remote task ID 之前允许切换。
- 用量只记在实际执行的模型上；失败的原模型如果没有产生用量就不记。
- **手工标记故障**（Q5）：`ProviderSettings`（`settings/mod.rs:41` 起）新增 `marked_faulty: bool`，由 `provider.update` 设置。被标记的 Provider 仍参与路由和用途表；执行时选中的模型在它上面就不发请求，直接按 Provider 不可用处理：有策略走备选，没有就返回带 Provider 名的错误。算备选时也排除被标记的 Provider。标记和清除都记事件。

### 4.5 价格比较（`not_pricier`）

- “不更贵”：备选的每个计费维度单价都不高于原模型（LLM 是输入、输出、缓存读写；媒体是各自的计费单位），按 `catalog.cost_in_usd` 折成同一币种比较（Q3）。
- 任何一边价格未知、币种无法折算、计费维度对不上，都算“不能证明不更贵”，排除。
- 价格取库存里的 `InventoryPricing`，和调用时 `PinnedPricingSnapshot` 用的是同一份。价格结构和计算函数与 [Credit TODO](aicc-user-policy-credit-todo.md) §3.3 共用（`catalog/schema.rs` 的 `Pricing`），不另写一套。比较的是 Provider 成本价；Credit 价目表上线后是否对普通用户改用 Credit 价，届时另议（Q3）。
- 用途表上显示的“不可用时会换到谁”也按这个规则算；算不出时写“没有已知价格不高于它的模型”，提示管理员给模型补价格或改用 `next_model`。

### 4.6 策略组、绑定和会话

- **App 绑定**存在 AICC：调用时按 `(caller.user_id, caller.app_id)` 查。
- **会话绑定**存在 lib_opendan 的会话配置里（建议，Q9）：
  - 会话配置新增 `route_group`（组 ID）。
  - lib_opendan 调 AICC 时，沿 `parent_session` 链找最近一个设了 `route_group` 的会话（复用 `api.rs:832-847` 的遍历），把组 ID 放进请求。调用时才查链，所以给父会话改组立刻对已有的子会话生效。
  - 链路：lib_opendan → `llm_context` 的 `LlmInferenceRequest`（`deps.rs:23-44`，新增字段）→ `agent_tool/src/xllm.rs:5534-5563` 构造 AICC 请求时带上 → AICC 请求新增 `route_group`。
  - AICC 校验：组必须属于这次调用的额度用户（`caller.user_id`）；不属于就当作没传，并在 trace 里标出（不报错，避免把别人的组 ID 当探针）。
- 删除策略组时，同时删除它的 App 绑定；会话里残留的组 ID 在调用时被忽略并在 trace 标出。
- 会话绑定的界面在会话自己的设置里（OpenDAN WebUI / Jarvis 对话的会话菜单），列出用户的策略组供选择；AI Center 的策略组页显示“已绑定到哪些 App”，会话绑定不在那里列出（AICC 不知道）。

### 4.7 kRPC 方法（草案）

| 方法 | 谁能调 | 说明 |
| --- | --- | --- |
| `routing.usecases` | 所有能用 AI 的用户 | 返回对调用者生效的用途表：每行的当前模型、来源层、每层的值、锁、`on_unavailable` 及其来源、“不可用时会换到谁”、覆盖状态与原因。参数可选 `view`：`self`（默认）/ `zone`（管理员层，需管理员）/ `group:<id>` / `app:<app_id>` |
| `routing.usecase.set` | 管理员写管理员层；用户写自己的层和自己的组 | `{target: "zone" \| "user" \| "group:<id>", usecase, model?, on_unavailable?, enabled?（只用于 `llm.fallback`、只能 `zone`）, clear?: ["model" \| "on_unavailable"], revision}` |
| `routing.groups.list` / `.create` / `.update` / `.delete` | 用户本人 | 只能看、改自己的组 |
| `routing.bindings.list` / `.set` / `.remove` | 用户本人 | App 绑定 |
| `provider.update` | 管理员 | 新增 `marked_faulty`（§4.4） |
| `routing.preview` | 不变 | 保留给高级视图；参数补 `view`，与 `routing.usecases` 一致 |
| `routing.get` / `routing.update` | 管理员 | 保留 `routing_commands` 与 `provider_weights` |

- 写操作都带版本号做并发检查（管理员层沿用 `settings_revision`，RDB 里的内容各自带 `version`）。
- websdk 同步类型和字段白名单（`buckyos-websdk/src/aicc_client.ts:543-596`），顺手补上 `RoutingUpdateRequest.routing_commands`。

### 4.8 Trace、用量、错误、通知

- trace 新增：每一格的来源层（`usecase_source`: `default` / `snapshot` / `zone` / `user` / `group:<id>`）、命中的 App 绑定或会话组、`on_unavailable` 及其来源；发生切换时记 `fallback_from`、`fallback_to`、`fallback_policy`、`failure_kind`、原模型的上游错误；经兜底 LLM 提供时记原用途。公开的 `RouteTrace`（`buckyos-api/src/aicc_client.rs:2885`，推理响应里已经返回）新增一个类型化字段 `fallback: {kind: "coverage" | "unavailable", from, to, policy?}`，调用方（xllm、lib_opendan）据此显示警告，不要塞进 `extra`。
- 用量记录标出“由不可用策略切换而来”，AI Center 用量页可以按此筛选。
- 错误：没有配置切换时返回原模型的真实错误（Provider 实例名、精确模型、上游状态码与原文），不改写成“没有可用模型”。`no_candidate_model` 只表示没有接入满足请求的模型，错误详情里给出用途名，并指向“用途”页。
- 通知：Provider 或模型连续失败、所选模型从库存消失、Provider 被标记或清除故障、发生了策略切换，都记事件；同一故障恢复前只记一次。是否推送 System Notify 沿用 11-23 TODO T5 的做法。

### 4.9 界面（Desktop › AI Center）

按手机宽度设计：表格在窄屏下变成每个用途一张卡片，“当前模型 / 来源 / 不可用时”竖排。

- **用途页**（新，导航里放在 Routing 前面，普通用户可见）：§3.1 的表；点一行打开编辑面板——选模型（只列出对自己可见、满足用途要求的模型，按家族分组，标价格）、选不可用策略（四个选项各附一句解释，并实时显示“会换到谁”）、恢复为跟随下一层。选模型的交互后续还会继续优化，帮用户完成手工调整（Q7）。兜底 LLM 单独一行，管理员可以开关。顶部切换视角：我 / 某个策略组 / 某个 App；管理员多一个“Zone 默认”。
- **策略组页**（新）：组列表、编辑（复用用途页的表）、绑定到 App（从 App / Agent 列表选）。
- **Routing 页**（现有）：改名为“路由细节”或并入用途页的高级视图，继续显示候选树、权重和 trace；只对管理员显示写操作。
- **添加 Provider 向导**：在验证之后、确认之前加一步“用途覆盖”：对比“现在”和“加上以后”的用途表，列出新覆盖的用途、默认模型会变的用途（每条可以取消勾选，取消就把旧模型写进管理员层）、仍未覆盖的用途。确认页显示摘要。
- **Provider 详情页**：新增“标记为故障”开关，说明“路由不变；配置了不可用策略的用途直接改用备选，其余直接报错；长期故障请停用 Provider”。
- **首页**：未覆盖的用途数量（以及其中由兜底 LLM 提供的）、被标记故障的 Provider、最近的策略切换，各一行，点进对应页。
- **会话里的兜底警告**：OpenDAN WebUI 与 Jarvis 对话收到 `route_trace.fallback.kind = coverage` 时，在会话里显示警告（§3.6）。
- 管理员界面里 fallback 的说明改成三种策略的说明；`not_pricier` 与 `next_model` 的说明里写明“系统按已知价格判断，价格未知的模型不会被选为‘不更贵’”。

### 4.10 CLI 与 Agent 手册

- `buckyos ai routing`（`buckyos-websdk/cli/modules/ai.ts:40-44`）改成显示用途表；新增 `buckyos ai usecase set <usecase> [--model m@p] [--on-unavailable none|same_model|not_pricier|next_model] [--zone|--group <id>]` 与 `buckyos ai routing-group …`。
- `tools/buckyos-agent` 的 `--no-fallback` 只对应“请求关闭不可用策略”。
- 手册：`apps/jarvis_agent/agent/cmd_skills/buckyos_admin/references/providers.md`（用途表、三种策略、故障排查、添加 Provider 后检查覆盖）、`cmd_skills/xllm/SKILL.md` 与 `references/runs.md`（改写 `llm.fallback` 的说法：兜底 LLM，默认关闭，只为没有模型的 LLM 用途兜底）。

---

## 5. 任务

按顺序做，每步单独可合入、单独验收。

### R0 去掉自动切换（= 11-23 TODO 的 T1–T3）

- [ ] 执行引擎只跑选中的模型：保留同模型重试和重新登录；删除候选循环、quota failover、`inference.rs` 的 `failover` 列表（`execution/mod.rs:1635-1638`、`:1830-1861`，`service/inference.rs:1169-1225`）。
- [ ] 删除 `runtime_failover`、`allow_exact_model_fallback`（API、`EffectiveRoutingPolicy`、websdk 类型与白名单、Desktop `aicc_mgr.ts:2358-2364` 的默认值）。`allow_fallback` 改名为请求侧的“关闭不可用策略”，默认不关闭（R3 之前没有策略可关）。
- [ ] 硬过滤拆成静态与运行状态：`hard_filter_runtime`（`routing/mod.rs:1013`）里只留 `enabled`、凭据已配置；`model_available` 冷却、`health`、熔断移出路由，只用于展示、告警和 R3 的失败分类。请求显式 `max_latency_ms` 仍是该请求的硬门槛。
- [ ] 删除内置 fallback：`parent` 模式整个移除（`model_defaults.rs:945`，`AiccFallbackMode::Parent`）；七个 LLM 用途到 `llm.fallback` 的回退先关掉（`model_defaults.rs:966`，相当于兜底 LLM 未打开，R1 再加开关）；`llm.fallback` 目录和它的 38 个规格保留（`:1143` 起，Q13）；同步 `model_defaults.rs` 头部的目录契约注释（有测试逐项比对）。
- [ ] `no_candidate_model` 只表示静态无候选；选中模型故障时返回上游真实错误。
- [ ] 一次成功或管理员修改该 Provider 配置后，立即清除它的故障标记。
- [ ] 删除默认打开 fallback 的调用方设置：`aiws/host.js:31`（及 `aiws_test.js:281`、`:288` 的断言）、`tools/buckyos-agent/lib/aicc.ts:144-145` 与 `rootfs/libexec` 里的拷贝。
- [ ] 改测试断言，清单见 11-23 TODO T6 第一条。

### R1 用途配置与三层

- [ ] `AiccSettings.session_config` 改名 `routing`（`settings/mod.rs:30-38`）；`ServiceModelAssembler` 把它放进 `system` 槽（`model_defaults.rs:526-534`）。
- [ ] 新类型 `AiccUsecaseRoute { model: Option<LockedValue<String>>, on_unavailable: Option<LockedValue<OnUnavailable>>, enabled: Option<bool> }`（`enabled` 只用于 `llm.fallback`） 与 `AiccUsecaseRoutes { usecases: BTreeMap<String, AiccUsecaseRoute> }`，放进 `buckyos-api/src/aicc_client.rs`；管理员层的 `routing` 增加 `usecases`。
- [ ] RDB：`aicc_routing_layer`、`aicc_routing_group`、`aicc_routing_binding`，存储版本升到 4（与用户策略 TODO Q14 同一次）。
- [ ] 求值（§4.2）：新模块 `routing/usecase.rs`，纯函数，输入各层内容 + 注册表 + 调用者 + 请求，输出 `primary` 与每格来源。
- [ ] 修路由策略分层：请求的策略叠加在管理员层上而不是替换（`service/inference.rs:214-217`）。
- [ ] kRPC：`routing.usecases`、`routing.usecase.set`（§4.7），websdk 同步。
- [ ] 校验（§4.1）。
- [ ] trace 增加来源层。
- [ ] 兜底 LLM（§3.6）：管理员层 `usecases."llm.fallback".enabled`；打开后七个 LLM 用途静态无候选时转到 `llm.fallback`。复用注册表现有的回退链（`model/mod.rs:786` 起），只留这一条内置的边，`AiccFallbackRule` 不再对外暴露（Q10）。`RouteTrace.fallback`（`kind = coverage`）。

### R2 默认路由稳定

- [x] 删除 `cost_first`（Q12）：`AiccSchedulerProfile::CostFirst`、请求侧 `RoutePolicyProfile::Cheap`、`scheduler_profiles.cost_first`、websdk 与 Desktop 里的对应值。`EffectiveRoutingPolicy` 的默认 profile（`routing/policy.rs:90` 起）和 `decision` 等用 `CostFirst` 的内置定义改成 `balanced`（`rg CostFirst src/frame/aicc/src` 找全）。
- [x] 其余 profile 的 `cost` 分量、会话历史软优先：按 N1、N2 删除（§11.7）。

### R3 不可用时的三种策略

- [ ] `OnUnavailable` 四个值；R0 留下的请求侧开关改为“关闭不可用策略”。
- [ ] 失败分类（触发 / 不触发，§3.4 规则 3），集中在一个函数里，单元测试逐类覆盖。
- [ ] 备选计算（§3.4、§4.4）：`same_model` 筛选、`not_pricier` 与 `next_model` 复用路由器加停用集合重算。
- [ ] 价格比较（§4.5）。
- [ ] 执行：每次请求最多一次；重新准入额度、物化资源、检查可见性；错误同时带两边原因；用量归到实际模型。
- [ ] `routing.usecases` 里每行的“不可用时会换到谁”（按 Provider 级故障预演）。
- [ ] 手工标记故障（§4.4）：`ProviderSettings.marked_faulty`、`provider.update`、执行时直接按不可用处理、算备选时排除、事件。

### R4 策略组与 scope

- [ ] 策略组、App 绑定的存储与 kRPC（§4.7）。
- [ ] 求值里加入 App 组与会话组两层。
- [ ] AICC 请求新增 `route_group`（推理、helper、`route.resolve`），websdk 同步；所有权校验与 trace 标注（§4.6）。
- [ ] 删除请求里的 `session_overlay`，以及 `AiccRouteOverlay` 中只为它存在的 `logical_profile`、`logical_profiles`、`active_logical_profile`、`inherit`、`revision`、`ttl_seconds`（Q10）。
- [ ] lib_opendan：会话配置新增 `route_group`；沿 `parent_session` 链取值；经 `llm_context` → `agent_tool/src/xllm.rs` 传给 AICC。浏览器 / Deno 版 xllm（websdk）在宿主提供会话信息时同样透传。
- [ ] 会话设置界面里选择策略组（OpenDAN WebUI、Jarvis 对话）。
- [ ] 兜底警告：xllm 把 `route_trace.fallback` 透传到 `llm_context` 的结果，lib_opendan 写进会话，OpenDAN WebUI / Jarvis 对话显示（§3.6）。

### R5 覆盖与添加 Provider

- [ ] `provider.validate` 的 `routing_candidates` 改成“加上这个 Provider 以后整个 Zone 的用途表”：所有 29 个用途（不只 LLM），每个用途给出加之前、加之后的模型和覆盖状态；按 §4.3 构建注册表预演。
- [ ] 向导新增“用途覆盖”一步，取消勾选的变化写进管理员层（§4.9）。
- [ ] 首页与用途页显示未覆盖的用途和原因（没有接入该 API 类型的模型 / 有模型但不满足用途要求，例如上下文不够、没有 tool call）。
- [ ] 删除或停用 Provider 前，确认框列出会受影响的用途（由谁接替或变成未覆盖）。

### R6 界面、CLI、手册

- [ ] 用途页、策略组页、Routing 页调整、首页三行（§4.9）；i18n 新键写齐现有的所有语言。
- [ ] CLI（§4.10）。
- [ ] 手册（§4.10）。

### R7 测试与验收

- [ ] 单元：层次合并与锁；请求只能收紧；不可见模型跟随下一层；兜底 LLM（未打开时无候选即报错；打开后只为静态无候选的 LLM 用途兜底；故障时不触发；trace 带标记）；手工标记故障（有策略走备选、无策略立即报错、用途表不变）；三种策略在单 Provider、同模型多 Provider、多模型多 Provider 下的备选；`not_pricier` 遇到价格未知、币种不同、阶梯价；每次请求最多切换一次；失败分类表。
- [ ] 回归（来自 11-23 TODO）：两个 Provider 同池，第一个连续失败 10 次，第二个收到 0 次请求；404 之后立刻再请求仍选同一模型；没有配置策略时任何失败都不换模型。
- [ ] 稳定性：Provider 刷新出一个权重更高的新版本模型，默认路由随之升级；Provider 连续失败（未标记故障）时默认路由不变。
- [ ] 策略组：App 绑定生效；会话绑定对子会话生效、子会话自己的绑定优先；用别人的组 ID 不生效。
- [ ] AICC 快照测试（wire / 库存 / 发现）保持不变：本 TODO 不应改变任何对上游发出的请求。
- [ ] 验收用例与旧断言：`test/aicc_test/acceptance/run_t1_gateway.ts`、`logical_entrypoint_baseline.json`、`doc/aicc/maintenance/acceptance/cases/` 下断言 failover 的用例。
- [ ] 真实 Zone DV：
  1. 只接 SN：用途表与覆盖正确；停掉 SN，所有请求返回真实错误，没有换模型。
  2. SN + 一个直连 Provider（同一模型两边都有）：`llm.code` 设 `same_model`，停掉直连 Provider，切到 SN 上的同一模型，trace 与用量正确。
  3. `not_pricier`：备选比原模型贵时不切换；补上价格后切换。
  4. 建两个策略组，分别绑到 Jarvis 和一个 Agent 会话，在该会话里派生子会话，确认各自生效。
  5. 添加 Provider：向导显示覆盖变化；取消其中一条后，该用途保持旧模型（来源显示“管理员”）。
  6. 只接一个小模型并打开兜底 LLM：`llm.plan` 由兜底提供，Jarvis 会话里出现警告。
  7. 把直连 Provider 标记为故障：设了 `same_model` 的用途直接切到 SN，没设的立即报错，用途表不变；清除标记后恢复。

---

## 6. 不做

- 不做任何基于健康、延迟、价格估算的自动选择或切换；不可用策略只在用户显式打开后生效。
- 不做按请求参数（如图片 `aspect_ratio`）换模型。
- 第一期不做“只选模型不选 Provider”（Q7），后续靠界面帮用户完成手工调整。
- 不做默认路由快照（Q1）。
- 不根据健康状态提前跳过原模型（Q5）。
- 不做跨 API 类型的切换。
- 不让请求方（App）替用户选模型或打开不可用策略；App 想用特定模型就直接请求精确模型。
- 不在 AICC 里存会话绑定（Q9）。

## 7. 与其它 TODO 的关系

- [显式 fallback TODO](aicc-explicit-fallback-todo.md)：被本文取代，见文首。
- [用户策略与 Credit TODO](aicc-user-policy-credit-todo.md)：可见性决定用途表里能选哪些模型；存储升级合并为一次；Credit 价目表上线后 `not_pricier` 的价格来源见 Q3。两者的主体写法保持一致（`user:<id>`、`app:<app_id>`）。
- [配置订阅更新 TODO](aicc-config-subscription-todo.md)：metadata 订阅更新直接改变默认路由（Q1：常规的默认模型升级），不需要确认。
- [自定义 Provider 自动探测 TODO](aicc-custom-provider-auto-probe-todo.md)：探测完成后同样走 R5 的覆盖步骤。
- [AICC 当另一个 AICC 的 Provider](aicc-zone-aicc-provider-todo.md)：远端 AICC 作为一个 Provider 实例参与用途表，三种策略同样适用；远端自己的路由不在本文范围内。

## 8. 文档同步

随代码一起改：

- `doc/aicc/aicc_router.md`：§9.3 的模式表改成 `on_unavailable` 四个值，补兜底 LLM 与手工标记故障；§9.4 示例改写；§10.4 写明池内只用静态输入；§11、§12 重写（AICC 维护用户层、策略组与 App 绑定，请求只能收紧，删除 `session_overlay` 与 `SessionLogicalProfile`）；§18 验收场景补充。
- `doc/aicc/aicc 逻辑模型目录.md`、`aicc_api设计.md`、`frozen_user_api.md`（新方法与字段）、`aicc_runtime_durable_data_schema.md`（新表）、`aicc-models-mgr.md`。
- `product/ai_center/AI_Center_PRD.md` §5.2（添加 Provider 的覆盖步骤）、§5.5 与 §9（用途页、策略组）。
- 手册见 §4.10。

## 9. owner 裁决（2026-11-27）

括号里是我的建议，`>` 后是 owner 的答复；正文已按答复改写。

- **Q1 默认路由要不要钉住。** “只有添加或删除 Provider 会影响”要求 Provider 刷新出新模型、metadata 更新时默认路由也不变。（建议：采用 §4.3 的快照——变化只提示、管理员确认后生效；被钉住的模型消失时立即重算并通知。不加机制的选项：不做快照，只靠 R0 去掉运行时输入，Provider 刷新和 metadata 更新会直接改变默认路由，事件里记一笔。）
> 不用钉住，这种更新通常是常规的默认模型升级更新。是用户期待的
- **Q2 “按其 disable 的逻辑”停用的是什么。**（建议：按失败类别——连接失败、超时、5xx、重新登录后仍认证失败停用整个 Provider 实例；模型不存在、模型级配额耗尽只停用这个精确模型。否则只接 SN 的用户遇到单个模型 404 时，`next_model` 也无模型可换。）
> 应是是 视作provider disalbe. 这个主要是说明路由是怎么跳到下一个候选的
- **Q3 “不会更贵”的口径。**（建议：逐维度单价都不高于原模型，按 `cost_in_usd` 折算；任何一边价格未知即视为更贵；用 Provider 的成本价，也就是 Zone 实际付的钱。Credit 价目表上线后，是否对普通用户改用他实际被扣的 Credit 价另议。备选：按本次请求的 token 估算比较总价——更宽松，但结果随请求大小变化。）
> 按建议。正在推进的 buckyos/notepads/aicc-user-policy-credit-todo.md 有关于价格的设计
- **Q4 一次请求最多切换几次。**（建议：1 次，沿用 router §9.3 规则 5。同模型有三个 Provider 时也只试一个备选。）
> 按建议
- **Q5 已知故障时是否直接跳过原模型。** 原模型的 Provider 已被标为故障（连续失败熔断）时，每个请求都要先等它超时才切换。（建议：只在这个用途打开了不可用策略时，直接用备选并在 trace 标“因已知故障跳过”；没打开策略时照常发给原模型。这会让健康状态在用户选择了策略时影响路由，属于“通过手工配置允许”。）
> 故障的provider,管理员应该手工配置成disalbe. provider可以手工配置成故障允许跳过
- **Q6 普通用户能否打开三种策略。**（建议：能，管理员可以在管理员层锁定 `on_unavailable` 禁止。Credit 计费上线前，用户打开 `next_model` 产生的费用由 Zone 承担，如果你担心这一点，可以改成 `next_model` 只有管理员能设。）
> 按建议
- **Q7 用途的“模型”能否只选模型、不选 Provider。** 例如选“claude-opus-5.5”，由同模型的多个 Provider 按默认顺序决定。（建议：第一期只选精确模型 `model@provider`，多 Provider 的情况用 `same_model` 策略表达。）
> 按建议，这个UI上肯定会进一步优化，帮助用户完成手工调整
- **Q8 层次与管理员的范围。**（建议：默认 < 管理员 < 用户 < App 组 < 会话组，管理员锁定的格子任何层不能改；所选模型对调用者不可见时这一格跟随下一层并在用途表上标出。第一期管理员不建共享策略组、不替所有用户给 App 绑组，只用管理员层 + 锁。）
> 按建议
- **Q9 会话绑定放在哪里。**（建议：放在 lib_opendan 会话配置，调用时沿 `parent_session` 链取最近的，请求带 `route_group`；AICC 只存策略组和 App 绑定。备选：AICC 也存会话绑定，请求带会话链；好处是 AI Center 能列出全部绑定，坏处是 AICC 要跟着会话的生命周期清理。）
> 按建议
- **Q10 删除请求里的 `session_overlay` 和旧的 fallback 目标模式。** 现在没有实际调用方发 `session_overlay`（Agent 对话、buckyos-agent、AIWS 都不发）。（建议：整体删除，连同 `SessionLogicalProfile`、`logical_profiles`、`active_logical_profile`；请求的收紧项都放进 `RoutePolicy`。`target_exact`、`target_logical`、`parent` 一起删除，由三种策略取代。）
> 按建议
- **Q11 `routing_commands` 和权重怎么办。** 它们是现在路由页唯一能改的东西。（建议：保留为管理员层的高级调权手段，影响的是默认展开和快照；用户层和策略组不开放。）
> 按建议
- **Q12 池内排序和会话历史。** 现在 `cost_first` 用请求的 token 估算排序，还按会话历史软优先，都会让同一用途选到不同模型。（建议：排序只用静态单价，价格未知的排在已知之后、再按默认顺序；删除会话历史软优先和 `aicc_session_route_history` 表。）
> cost_first 这个可以删除了。实际上效果很差
- **Q13 未覆盖的用途。** 去掉 `llm.fallback` 后，只接了一个小模型的 Zone 里 `llm.plan`、`llm.code` 可能没有候选。（建议：不恢复内置兜底；向导和首页列出未覆盖的用途，并为每个给出“用 X 覆盖”的建议，用户确认后写进管理员层。建议只在 API 类型相同、满足该用途硬要求的模型里选。）
> llm.fallback展示保留，系统管理员启用该配置后意味着至少可以有一个llm用来维护系统。（使用llm.fallback的session应该有警告信息）

### 9.1 裁决带出的问题（2026-11-28 已答复）

两项都只影响 R2，已按建议实施（§11.7）。

- **N1 删了 `cost_first` 以后，其余 profile 的 `cost` 分量怎么办。** `balanced` 0.5、`quality_first` 0.3、`latency_first` 0.5、`local_first` 0.3、`strict_local` 0.5 都带成本（`routing/mod.rs:1103-1119`），成本又是按请求的 token 估算的，同一用途仍可能因为请求大小选到不同模型。（建议：池内排序整个不用价格，价格只用于 `not_pricier`、预算上限和展示；排序按偏好权重、规范字段匹配质量、本地优先，最后按默认顺序。`cost` 分量和 `estimated_cost` 参与评分的路径一起删掉。备选：保留 `cost` 分量，改用与请求无关的静态单价。）
> 按建议，我们实测限制基本只有local_first和strict_local。 
- **N2 会话历史软优先。** Q12 的后半——删除会话历史软优先和 `aicc_session_route_history` 表——没有答复。它让同一会话倾向上一次用过的模型；默认路由变成确定性以后，它只在池里有多个同分候选时才改变选择。（建议：删除。Agent 对话路径本来也不传 `session_id`。）
> 按建议

按裁决推出的细则，不阻塞实施，有异议再改：

- 兜底 LLM 只为七个 LLM 用途在静态无候选时兜底，Provider 故障时不触发；经兜底的请求用兜底 LLM 这一行的配置（含它自己的不可用策略）；开关只在管理员层（§3.6）。
- 被手工标记故障的 Provider，没有配置策略的用途立即报错，而不是照常发请求（§4.4）。
- 默认 profile 由 `cost_first` 改为 `balanced`（R2）。
- 触发类别沿用 11-23 TODO Q1 的建议，其中包括“用户额度规则拒绝该模型”（§3.4 规则 2）。它不是 Provider 不可用；如果你认为三种策略只应在 Provider 不可用时生效，删掉这一项即可。

## 10. 验收

- 用户在手机上打开 AI Center，能看懂每个用途现在用哪个模型、这个选择来自哪一层、Provider 出问题时会发生什么。
- 默认路由只随配置和静态库存变化（加删启停 Provider、Provider 模型列表刷新、metadata 更新、权重命令）；Provider 故障和手工标记故障都不改变它。
- 没有打开不可用策略时，任何故障都不换模型，错误里有 Provider、模型和上游原因。
- 三种策略只在手工打开后生效，每次请求最多换一次，trace、用量、事件都能看到原模型、实际模型和原因。
- 策略组按 App 和会话（含子会话）生效，并且只对它的主人生效。
- 添加 Provider 时，向导显示整个 Zone 的用途覆盖变化。
- 打开兜底 LLM 后，没有模型的 LLM 用途仍能工作，会话里看得到警告。

## 11. 实施记录（2026-11-28）

代码未提交、未部署；真实 Zone DV（R7 最后一项）没跑，需要重启本机 Zone。

### 11.1 落点

**共享类型（`buckyos-api`）**
- 新模块 `kernel/buckyos-api/src/aicc_routing.rs`：`routing_methods`、`OnUnavailable`、`AiccUsecaseRoute(s)`、`RouteFallback`（`kind = coverage | unavailable`）、用途表的请求与响应、策略组与 App 绑定、`UsecaseCoverageChange`。
- `aicc_client.rs`：删 `AiccFallbackMode`、`AiccFallbackRule`、`AiccSessionLogicalProfile`、`AiccLogicalTreeOverlay`、`OverlayMergeMode`、`AiccSchedulerProfile::CostFirst`（默认改 `balanced`）、`RoutePolicyProfile::Cheap`、`RoutePolicy.allow_fallback/runtime_failover`（换成 `no_fallback`）、`AiccPolicyConfig` 的三个 fallback 字段、`AiccRouteOverlay` 的 profile / `inherit` / `revision` / `ttl_seconds`、请求里的 `session_overlay`（换成 `route_group`）、`RouteResolveResponse.fallback_attempts`、`ProviderValidateResponse.routing_candidates`（换成 `usecase_coverage`）、`AiccErrorCode::FallbackNotAllowed`；加 `AiccRouteOverlay.usecases`、`RouteTrace.fallback`、`marked_faulty`。
- `aicc_usage_log.rs`：`AiccUsageEvent.fallback_from`、`UsageQueryFilters.fallback_only`。

**AICC**
- 用途表求值 `frame/aicc/src/routing/usecase.rs`：层合并与锁（`UsecaseLayers::cells`）、`resolve`（各层选的模型从高到低试，不可用跟随下一层，再到默认展开，七个 LLM 用途静态无候选且兜底 LLM 打开时转 `llm.fallback`）、`unavailable_fallback`（三种策略；原 Provider 与被标记故障的 Provider 视作停用）。测试在 `routing/usecase/tests.rs`。
- 价格比较 `policy/pricing.rs` 的 `not_pricier`：逐计费维度，候选的最高单价（含阶梯、规则、时段）不高于原模型的最低单价，经 `cost_in_usd` 折算；价格未知、币种无汇率、维度对不上都不算。
- 注册表 `model/mod.rs`：去掉 fallback 链、`with_session_overlay`、profile overlay，`RegistryLayers` 只剩 factory / system。`model_defaults.rs`：定义不再带 fallback，`CostFirst` 改 `Balanced`，头部契约注释第 6 条改写，`assemble_registry`。
- 路由 `routing/mod.rs`：健康、熔断、冷却不再参与过滤（`CandidateRuntimeState` 去掉 `health`、`model_available`，Provider 未运行并入 `enabled`）；`RoutingRequest` 加 `excluded_providers`、`excluded_models`；`route_pinned`；去掉精确模型 fallback 与 `fallback_candidates`。策略分层 `routing/policy.rs`：directory < zone < request，请求只收紧（`tighten`）。
- 执行 `execution/mod.rs`：`UnavailableFallback` 代替 failover 列表，每次请求最多换一次；`unavailable_trigger` 集中做失败分类；备选也失败时 `with_original_failure` 把两边原因放进错误；`fallback_from` 跟着绑定写进用量。
- 推理入口 `service/inference.rs`：`usecase_layers`（zone 来自 settings，用户层、App 绑定的组、请求的 `route_group` 来自 RDB，`route_group` 不属于调用者时忽略并在 trace 标出）、`RouteContext`、`InferenceFallback`（只在需要时算备选，重新物化资源，额度准入在引擎里）、被标记故障时直接用备选或报 `no_provider_available`、额度预检被拒时同样换一次、`Preview`（`routing.preview`、用途表、覆盖预演共用）。
- 用途表 RPC `service/inference/usecases.rs`；zone 层写入在 `service/management.rs` 的 `handle_usecase_rpc`（走 `mutate_settings`）。`provider.validate` 返回 `usecase_coverage`；`provider.update` 支持 `marked_faulty`（只改这一项时不重新验证）、记事件、清该 Provider 的健康记录。
- 存储：`storage/policy.sql` 末尾加 `aicc_routing_layer`、`aicc_routing_group`、`aicc_routing_binding`，`schema.sql` 的 `aicc_usage_event` 加 `fallback_from`；版本仍是 4（与用户策略 TODO 的 Q14 同一次），v3 升级时一起加。`storage/routing.rs` + 测试。
- 设置：`AiccSettings.session_config` 改名 `routing`；`ProviderSettings.marked_faulty`。事件：`unavailable_fallback`、`provider_marked_faulty`、`provider_fault_cleared`、`usecase_model_missing` / `usecase_model_recovered`（zone 层选的模型离开库存，注册表重建时检查，恢复前只记一次）、`usecase_updated`、`model_failing` / `model_recovered`（健康记录里连续失败熔断或模型已下线时记一次，成功后记恢复；健康记录只用于展示、告警和请求的 `max_latency_ms`）。

**其它**（由并行的子任务完成，见 §11.4）：websdk 类型与 CLI、`buckyos-agent`、`aiws/host.js`、Desktop AI Center、lib_opendan 会话 `route_group` 与兜底警告、OpenDAN WebUI、文档与手册、验收用例。

### 11.2 与正文不同或正文没写的取舍

- **注册表里没有回退边。** §R1 写“复用注册表现有的回退链，只留这一条内置的边”；实现把兜底 LLM 放到用途表求值里（§4.2 的伪代码就是这样写的），注册表的 fallback 机制整个删掉，代码更少。
- **兜底 LLM 只带请求自身的硬要求**，不带原用途的门槛（§3.1 例子里不满足上下文要求的模型由兜底提供）。所以 `llm.vision` 也可能由不支持视觉的小模型兜底，除非请求本身声明了视觉要求；由兜底 LLM 提供 `llm.chat` / `llm.plan` / `llm.vision` 时，联网搜索按 `llm.fallback` 处理（关闭）。
- **上下文放不下不触发兜底**：用途因请求太大而无候选时仍返回 `context_length_exceeded`，让调用方缩小请求。
- **请求里的调度 profile 仍可改池内排序**（锁定时忽略）。严格按 §3.2 只有硬约束能影响请求，但删掉 `RoutePolicy.profile` 不在任务里，和 N1 一起定。
- **请求的屏蔽列表叠加在管理员的上面**，请求的允许列表与管理员的同时生效；成本上限只能调低；`local_only` 只能打开。原来请求的策略整体替换系统配置，管理员锁定的值会让任何带 `policy` 的请求报错，这个缺陷一起修了。
- **不可用策略只在 AICC 内按用途路由的请求上生效**（`helper.*`）。`route.resolve` 之后再用精确模型发的 typed 请求（buckyos-agent 的媒体命令就是这样）不会切换。要覆盖它们，需要让 typed 请求接受用途名，没做。
- **`same_model` 还要求同一个思考预设**（variant 相同），否则不算同一个模型。
- **被标记故障的 Provider 没有策略时**报 `no_provider_available`，details 带 `reason: marked_faulty`、Provider 和精确模型。
- **用途表的 `self` 视图**只含 zone 层和用户层，不含调用方 App 绑定的组；`routing.preview` 同样。
- **用户层和策略组里写 `locked` 校验拒绝**；写管理员已锁定的格子返回 `policy_denied`。
- **CLI 的 `--zone` 改成 `--zone-default`**：`--zone` 和 CLI 的全局选项冲突（交互 shell 里报错）。

### 11.3 验证

- `cargo test -p aicc -- --test-threads=1`：839 通过（含 wire / 库存 / 发现三组快照，快照未变）。新增用例：层合并与锁、不可用模型跟随下一层、默认路由不受延迟观测影响、三种策略在单 Provider / 同模型多 Provider / 多模型多 Provider 下的备选、被标记故障的 Provider 不作备选、`not_pricier` 遇到价格未知 / 汇率折算 / 无汇率币种 / 阶梯价 / 维度不一致、兜底 LLM 只为 LLM 用途兜底、请求只能收紧、无策略时失败 10 次第二个 Provider 收到 0 次请求、每次最多换一次且错误带两边原因、用量归到实际模型并带 `fallback_from`、失败分类表、用户层与策略组存储（版本、归属、删除组同时删绑定）。
- `cargo check --workspace --tests` 通过；`cargo test -p buckyos-api --lib aicc`、`-p scheduler aicc`、`-p node_updater aicc` 通过。
- 没跑：真实 Zone DV（§5 R7 的 7 个场景），需要部署新 AICC 并重启本机 Zone。

### 11.4 并行子任务的结果

- **websdk 与 CLI**（`buckyos-websdk/src/aicc_client.ts`、`src/xllm/providers/aicc.ts`、`cli/modules/ai.ts`）：类型和字段白名单同步；新方法 `getRoutingUsecases`、`setRoutingUsecase`、`list/create/update/deleteRoutingGroup`、`list/set/removeRoutingBinding`；`routing.update` 可以发 `routing_commands`；浏览器 / Deno xllm 的 `AiccLlmClient` 可带 `route_group`（Deno 宿主暂时没有来源）。CLI：`buckyos ai routing [--view …] [--json]` 显示用途表，`buckyos ai usecase set … [--zone-default|--group <id>] [--lock]`，`buckyos ai routing-group list|create|delete|bind|unbind`。`dist` 已重新构建。验证：jest 1433 过、1 失败（`file_meta` 的 PDF 样例在仓库外，与本单无关）；`test:cli` 182 过；`test:xllm:deno` 69 过。
- **buckyos-agent / aiws**：`--no-fallback` 对应 `no_fallback: true`，去掉 `cheap`；个人站翻译（`lib/model.ts`）发 `no_fallback: true`，保持原来“从不换模型”的行为；`aiws/host.js` 不再发策略。`deno test -A lib/` 136 过；aiws 测试 10 过。`rootfs/libexec/buckyos-agent` 是构建产物，要重新构建。
- **Desktop AI Center**（`src/frame/desktop/src/app/ai-center/`）：新页面“Usecases”（导航在 Routing 前，所有人可见；手机宽度下每个用途一张卡片；编辑面板选模型按厂商分组、显示 Provider 成本价，四个策略各带说明；“Needs attention”筛选；只有 Zone default 视图有锁定勾选）、“Routing groups”（建组、改名、删除、编辑组内用途、绑定 App / Agent）；Routing 改名“Routing details”，保留候选树、权重和 trace，写操作只对管理员；兜底 LLM 是表下单独一张卡片；添加 Provider 向导新增“Usecases”一步（取消勾选的变化在 Provider 建好后写进 Zone default，非管理员只读）；Provider 详情的 Routing 标签有“标记为故障”开关，列表和概览显示标记；删除 / 停用确认列出受影响的 Zone default 用途（用 `without_provider` 对比）；首页三行；用量页“由不可用策略切换”筛选。顺带修了共享侧栏在手机上标题被状态栏盖住的问题。验证：`pnpm check` 过；`pnpm lint` 与基线相同（6 个旧错误，新文件无）；`playwright.aicc.config.ts` 25/25（新增 5 个）；`tests/e2e/pages/ai-center.spec.ts`、`desktop-i18n.spec.ts` 的 9 个失败在改动前就有（找不到 “Est. Cost” 按钮、各语言缺 3 个 `taskCenter.tasks.*` 键）。新键写齐 10 种语言。未做：Models 页的权重控件仍没限制为管理员。替 owner 定的界面取舍：模型按厂商分组、价格显示 Provider 成本价（不是 AI Credit 价）、编辑面板显式保存、“会换到谁”保存后才更新（接口不能预览未保存的选择）。

- **OpenDAN 会话链路**：`SessionConfig.route_group`（可选字段，没升 schema，node_updater 测试过）；控制命令 `route_group` 与 Agent Service kRPC `session.route_group {sid, route_group}`；沿 `parent_session` 链取值的 `libopendan::api::route_group_of`（每次推理时查，改父会话的组立刻对子会话生效），由 `runner/route.rs` 的 `RoutedLlm` 包住 run 的 LLM 客户端；`llm_context` 的 `LlmInferenceRequest.route_group` 与 `LlmClient::infer_routed`（返回 `route_trace`，默认实现，原有客户端不用改）；xllm 只对逻辑模型发 `route_group`。兜底提醒：`SessionState.route_fallbacks` 按 (用途, 模型) 每会话一次；能回复对话的会话在回复前发一条 `notice`（机器意图 `opendan.route_fallback`，群聊不发），文字来自 Jarvis i18n，只有 en / zh。界面：OpenDAN WebUI 会话详情的“Routing group”选择（显示“继承自父会话”）和警告条；Desktop MessageHub 里与自己 Agent 对话的 Agent 面板选择策略组，提醒消息显示为警告卡片。验证：`cargo test -p llm_context` 108 过、`-p libopendan --test route_group` 3 过、`-p opendan` 过、`-p agent_tool` 370 过（3 个 grep 用例因本机没有 `rg` 失败，基线如此）；OpenDAN WebUI playwright 70/70。未做：会话 shell 里跑的 `xllm` 命令不带会话的组；独立配置的 `RunnerDeps.summarizer` 不走组；Work 会话的提醒不在 Desktop 工作卡片上显示；OpenDAN WebUI 的 Agent App 令牌能否调 AICC `routing.groups.list` 没在真实 Zone 验证（不能时选择器显示不可用，仍可选“无”）。
- **文档与手册**：`doc/aicc/aicc_router.md`（v0.4，§8–§22）、`aicc 逻辑模型目录.md`、`aicc_runtime_durable_data_schema.md`、`aicc_api设计.md`、`frozen_user_api.md`、`aicc-models-mgr.md`、`maintenance/` 下的设置、调用指南等，`product/ai_center/AI_Center_PRD.md` §5.2 / §5.5 / §9，手册 `buckyos_admin/references/providers.md`、`xllm/references/runs.md`；验收用例 `doc/aicc/maintenance/acceptance/` 与 `test/aicc_test/acceptance/*.ts`（`deno check` 的 41 个旧错误前后相同，离线自测 166/166）。历史文档（`archive/`、review 记录、路线图、旧 fallback TODO）没改。

### 11.6 合并后的复核（2026-11-28）

`cargo check --workspace --tests` 通过；`cargo test -p aicc -- --test-threads=1` 839 过（§11.7 之后 837：会话历史的存储和评分用例随功能删除）；`-p buckyos-api --lib` 430 过；`-p llm_context` 108 过；`-p libopendan --test route_group` 3 过；`-p opendan` 过；`-p agent_tool` 370 过、3 个 grep 基线失败。Desktop `pnpm check` 过、`playwright.aicc.config.ts` 25/25；OpenDAN WebUI `pnpm check` 过。

### 11.5 风险与待确认

- 设置字段 `session_config` 改名 `routing`、存储表结构变化：owner 2026-11-28 定这个版本是 breaking change，用户统一重新安装，不做迁移。node_updater 的 AICC reader 审计范围（`policy.sql`、`schema.sql`、设置解析）内的变化按 readers.toml 的流程在周 review 时登记。
- 兜底提醒的 Agent 文字只有 en / zh，其它语言的对话里不发提醒。
- 新 CLI 子命令、Desktop 页面、OpenDAN 的组选择都没在真实 Zone 上跑过。

### 11.7 N1、N2（2026-11-28 owner 答复后）

- **N1 按建议**：池内排序不再用价格。`AiccSchedulerProfileWeights` 去掉 `cost`，评分只剩 quality、preference、cache、local；默认权重 `balanced` quality 0.5 / preference 0.5，`quality_first` 0.7 / 0.3，`latency_first` preference 0.5 / local 0.5，`local_first` preference 0.3 / local 0.7，`strict_local` 同 `balanced`（另外只允许本地 Provider）。owner 说明实际起作用的限制基本只有 `local_first` 和 `strict_local`；去掉成本以后 `latency_first`、`quality_first` 与 `balanced` 差别很小，是否删掉这两个 profile 和请求侧的 `fast` / `quality` 可以再定。价格只用于 `not_pricier`、成本上限和展示（`estimated_cost` 仍在 trace 里）。
- **N2 按建议**：删除会话历史软优先和表 `aicc_session_route_history`（schema 4 不建；v3 升级时删掉）；请求上的 `session_id` 只为它存在，一并删除（`buckyos-api` 的 route.resolve、helper、typed 请求，websdk 类型、白名单和校验，`agent_tool` 媒体理解里的传递，验收脚本）。`ComputerEnvironment.session_id` 是 computer environment 的会话，不受影响。
- 同时去掉了“已经是 v4 的库启动时补表补列”的兼容代码（owner：统一重新安装）。
- 文档：`aicc_router.md` §10、§11.5、§12.3，`aicc_runtime_durable_data_schema.md`，`aicc_api设计.md`，`frozen_user_api.md`，`aicc-models-mgr.md`，`aicc_e2e_test_requirements.md` §7.6，验收用例 `l1_routing_stability_health_ignored`。
- 验证：`cargo check --workspace --tests` 过；`cargo test -p aicc -- --test-threads=1` 837 过；`-p buckyos-api --lib` 430 过；`-p agent_tool` 370 过（3 个 grep 基线失败）；websdk 重新构建，`aicc_client` 与 xllm jest 312 过；Desktop `pnpm check` 过、`playwright.aicc.config.ts` 25/25；OpenDAN WebUI `pnpm check` 过。`test/aicc_test/acceptance` 的 `deno check` 本机缺 `node_modules`（`@types/node`），没跑。

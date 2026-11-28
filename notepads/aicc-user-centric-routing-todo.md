# AICC 用户视角的模型路由改进 TODO

日期：2026-11-27。状态：立项，代码未实施；§9 的问题待 owner 答复。

基线：`fcd23bfb4`，加上当天工作区里未提交的 AICC 改动（百炼改名等，不涉及路由）。下文行号按这份工作区记录，动手前先复核。

本文**并入并取代** [AICC 去掉自动换模型、改为显式 fallback TODO](aicc-explicit-fallback-todo.md)（11-23 立项，代码一行没动）：

- 它的 T1–T3（去掉执行期切换、健康状态不参与选择、去掉内置 fallback）原样保留，成为本文的 R0。
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
>   - 可以允许某个 Provider 不可用后，路由到下一个提供同模型的 Provider，这种情况适用于多个 Provider
>   - 可以允许某个 Provider 不可用后，按其 disable 的逻辑到下一个不会更贵的模型
>   - 可以允许某个 Provider 不可用后，按其 disable 的逻辑路由下一个模型
> - 用户可以配置多个路由策略组（包含多个 usecase 的配置），并可以按 scope 应用。比如特定 App 应用，比如特定 LLM session（含 child-session）应用

### 1.2 继承的原则

- **11-23 的路由原则仍然有效**（router §9.1）：相同配置、库存和请求得到相同的模型与 Provider；Provider 不可用就修 Provider；系统不自动换模型；换模型只来自手工配置。本文的三种策略就是这里说的“手工配置”。
- **默认允许 breaking change，不做旧格式兼容**（仓库 `AGENTS.md`）。
- **机制越少越好。** 能用已有部件表达的不加新机制；每个新机制在 §9 列出不加它的选项。
- **体验按手机评估。** 目标用户很多没有 PC，AI Center 的用途表、策略组和添加 Provider 的覆盖步骤都要在手机宽度下好用。

### 1.3 我对原话的理解（待 owner 确认，见 §9）

- **usecase（下文称“用途”）就是逻辑目录里 kind 为 `task` 的节点**，来自 `service/model_defaults.rs:594` 起的 `builtin_logical_model_definitions()`，共 29 个：
  - `decision`
  - `llm.chat`、`llm.plan`、`llm.code`、`llm.swift`、`llm.summarize`、`llm.translate`、`llm.vision`
  - `embedding.text`、`embedding.multimodal`、`rerank`
  - `image.txt2img`、`image.img2img`、`image.inpaint`、`image.upscale`、`image.bg_remove`
  - `vision.ocr`、`vision.caption`、`vision.detect`、`vision.segment`
  - `audio.tts`、`audio.asr`、`audio.music`、`audio.enhance`
  - `video.generate`、`video.edit`、`video.extend`、`video.upscale`
  - `agent_runtime.computer_use`

  `llm.fallback` 不是用途，随内置 fallback 一起删除（R0）。规格和家族目录（如 `llm.gpt-mini`、`image.txt2img.flux`）仍可以被请求直接使用，但不出现在用途表里，也不能单独配置。
- **“路由”是一张表：用途 → 一个精确模型 `model@provider`。** 用户在 AI Center 看到的、能调整的就是这张表；权重、规格、家族这些内部结构只在高级视图里出现。
- **“按其 disable 的逻辑”**指：假设出故障的那个 Provider（或那个精确模型）被停用，路由重新算一次，得到的就是“下一个模型”。所以备选目标完全由现有的默认路由逻辑推出来，不需要再单独配置 fallback 目标。三种策略因此取代 `target_exact`、`target_logical`、`parent` 和 `llm.fallback`（Q2、Q10）。
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

- **“停用某个 Provider 再算一次”**：路由策略已有 `blocked_provider_instances`（`routing/policy.rs:73` 起），在请求的策略上追加被停用的实例，就是“按其 disable 的逻辑”。按模型停用可以用 `exact_model_weights = 0` 的同一条硬过滤（`routing/mod.rs` `hard_filter_model`）。
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
| 生成视频 `video.generate` | — | — | — | 未覆盖：没有接入能生成视频的模型 |

- “来源”是决定这一格的那一层：默认 / 管理员 / 我 / 策略组名。被管理员锁定的格子显示锁，不能改。
- “不可用时”同时显示策略和**按现在的配置会换到谁**（假设当前模型所在的 Provider 不可用时算出来），让用户在出事之前就知道会发生什么。
- 用途表可以切到某个策略组的视角（“这个组生效时是什么样”），也可以切到某个 App 的视角。
- 管理员另有一个“Zone 默认”视角，编辑的是管理员层。

### 3.2 稳定性契约

```text
用途 U 在 scope S 下的路由 = f(默认配置, 管理员层, 用户层, S 绑定的策略组,
                             已接入并启用的 Provider 集合, 默认路由快照)
```

一次具体请求在此之上只再受**请求自身的硬约束**影响（tool call、视觉、上下文长度、`local_only` 等“没有就做不了”的要求），而且结果是确定的。

**不影响路由的东西**：健康状态、熔断、冷却、延迟观测、上游配额余量、动态成本估算、请求的 token 数、会话历史、可选的调节参数（11-23 TODO Q6）。

**会改变默认路由的事件**：添加、删除、启用、停用 Provider（管理员的动作）。Provider 的模型列表变化和 Model Driver metadata 更新只产生“有更新可用”的提示，管理员确认后才生效（§4.3，Q1）。

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
| `not_pricier` | 换下一个不更贵的模型 | 停用故障对象后重新路由；结果比当前模型贵或价格未知，就把它也排除再算，直到找到一个不更贵的或没有候选（§4.5） |
| `next_model` | 换下一个模型 | 停用故障对象后重新路由，得到的第一个结果 |

“故障对象”是 Provider 实例还是精确模型，按失败类别决定（Q2）：连接失败、超时、5xx、认证被拒（动态登录重试后仍失败）停用整个 Provider 实例；模型不存在 / 已下线、模型级配额耗尽只停用这个精确模型。只接了 SN 一家的用户，按 Provider 停用后没有任何备选，这是预期的——单 Provider 没有可换的。

**执行规则**（沿用 router §9.2、§9.3）：

1. 先对当前模型做同模型重试和同 Provider 的重新登录。
2. 仍然失败、属于触发类别、且 `on_unavailable` 不是 `none`，才换到备选，**每次请求最多换一次**（Q4）。
3. 不触发的错误：400、401（重新登录后仍是）以外的认证错误、403、409、取消、已产生用量的失败、异步任务已经拿到 remote task ID 之后的失败。
4. 备选也失败就返回错误，错误里同时给出原模型和备选模型的失败原因。
5. 备选同样受原请求的能力、隐私、`local_only`、可见性和额度约束。
6. 已知故障时是否直接跳过原模型（不等超时）见 Q5。

### 3.5 路由策略组与 scope

- 用户可以建多个策略组，每组是一份“用途 → 选择 + 不可用策略”，内容和用户层相同。
- 绑定到 **App**：通过这个 App（或 Agent，`app_id = agent_id`）发出的调用生效。
- 绑定到 **LLM 会话**：这个会话和它的所有子会话（沿 `parent_session` 链向下）生效；子会话自己绑了别的组就用自己的。
- 同一个调用同时命中 App 绑定和会话绑定时，会话绑定优先。
- 策略组属于建它的用户，只对这个用户的调用生效。管理员能否建全 Zone 共用的组、能否替所有用户给某个 App 绑组，见 Q8。

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
    "image.txt2img": { "model": "flux-2-pro@fal" }
  }
}
```

- `model`：精确模型；不写表示跟随下一层（Q7 讨论是否允许只选模型、不选 Provider）。
- `on_unavailable`：`none` / `same_model` / `not_pricier` / `next_model`；不写表示跟随下一层。
- 两个字段都用 `LockedValue`，只有管理员层的 `locked` 有效，其它层写了 `locked` 校验时拒绝。
- 字段只是配置，不保证资源存在。所选模型不在库存里时配置保留，用途表标“所选模型已不可用”，这一格跟随下一层，并发事件通知（不迁移到别的模型）。
- 校验：用途必须存在；模型的 API 类型必须和用途一致，且满足用途自身的硬要求（`min_line`、`tool_json_requirement` 等）。

**存放：**

| 层 | 位置 |
| --- | --- |
| 默认 | 代码：`builtin_logical_model_definitions()` + `builtin_logical_tree_overlay()`，不变 |
| 默认路由快照 | AICC RDB 新表（§4.3） |
| 管理员层 | system-config `services/aicc/settings`：`session_config` 改名为 `routing`，新增 `usecases` 字段；`routing_commands`、`provider_weights` 保留在这一层，作为管理员的高级调权手段（Q11） |
| 用户层 | AICC RDB 新表 `aicc_routing_layer`，主体 `user:<user_id>` |
| 策略组 | AICC RDB 新表 `aicc_routing_group`（`group_id`、`owner`、`name`、`content_json`、`version`、`updated_at_ms`） |
| App 绑定 | AICC RDB 新表 `aicc_routing_binding`（`owner`、`scope = app:<app_id>`、`group_id`） |
| 会话绑定 | lib_opendan 的会话配置（§4.6，Q9） |

新增表要把 `STORAGE_SCHEMA_VERSION` 升到 4，会让已部署的 Zone 起不来；与 [用户策略与 Credit TODO](aicc-user-policy-credit-todo.md) 的 Q14 合并成同一次升级，不要各升一次。

### 4.2 求值

```text
resolve(用途 U, 调用者 C, 请求 R):
  cfg = 按 §3.3 合并 U 在 [默认, 管理员, 用户(C.user), App 组(C.user, C.app), 会话组(R.route_group)] 上的两格
        （每格记下来源层和是否被锁）
  静态可用(m) = m 通过静态硬过滤（库存、能力、operation、Provider 已启用、凭据已配置、隐私、
               信任、预算、可见性）且满足 R 的硬约束
  if cfg.model 已设 且 静态可用(cfg.model):       primary = cfg.model，来源 = cfg.model 的层
  elif 快照[U] 存在 且 静态可用(快照[U]):         primary = 快照[U]，来源 = 默认
  else:                                           primary = 静态展开(U, R)，来源 = 默认
  return primary, cfg.on_unavailable
```

- “静态展开”就是 router §10.4 的两阶段算法，只用静态条件（R0 的 T2）。池内排序见 Q12。
- 用户层、策略组只改两格，**不再克隆注册表**：`with_session_overlay` 的每请求克隆（`service/inference.rs:141-147`）随请求 overlay 一起删除（Q10）。
- 路由策略（`policy`）的层次同样是 默认 < 管理员 < 请求（请求只能收紧），修掉 `service/inference.rs:214-217` 的“替换”。
- 规格、家族路径的请求不读用途配置，按静态展开，结果同样确定。

### 4.3 默认路由快照

为了做到“只有添加或删除 Provider 会影响”，默认层的结果要钉住（Q1 是否采用；不采用时删掉本节，只靠 R0 去掉运行时输入）：

- 表 `aicc_default_route_snapshot`：`usecase`、`exact_model`、`basis`（计算时的库存修订号与 metadata generation）、`computed_at_ms`、`reason`（`provider_added` / `provider_removed` / `provider_enabled` / `provider_disabled` / `admin_accepted_update` / `pinned_model_gone` / `initial`）。
- **重算时机**：`provider.add`、`provider.delete`、`provider.update` 改了 `enabled` 之后，对全部用途重算并写入；管理员在界面上接受“更新可用”之后写入。
- **不重算、只提示**：Provider 刷新模型列表、metadata 更新、权重命令变化之后，后台对比“现在重算会得到什么”和快照；有差异就记一条事件 `default_route_update_available`（列出每个变化的用途，旧模型 → 新模型），界面显示“有 N 个用途的默认模型可以更新”，管理员逐条或一起接受。
- **被钉住的模型从库存里消失**（上游下线、Provider 刷新后不再提供）：这个用途立即重算并写入，记 `pinned_model_gone` 警告事件。不能让快照指向一个不存在的模型。
- 快照只决定默认层；管理员、用户、策略组选了模型的格子不受它影响。
- 添加 Provider 前，向导用同一份重算逻辑预演“加上以后”的用途表（§4.9）。

### 4.4 不可用时的执行

替换现在的候选循环（`execution/mod.rs:1635-1638`）和 `inference.rs` 的 `failover` 列表：

- 路由阶段只产生 `primary`，同时把 `on_unavailable` 和“备选怎么算”所需的输入（用途、请求硬约束、调用者）带进执行请求；**不预先**算好一串备选。
- 执行阶段在 §3.4 的触发条件成立时才计算备选：按失败类别得到故障对象 → 按策略算出一个备选（`same_model` 直接筛；`not_pricier` / `next_model` 用“请求策略 + `blocked_provider_instances` / 模型禁用”重新路由一次）→ 重新做额度准入、资源物化、可见性检查 → 执行一次。
- 备选的计算复用路由器，不复制一份展开逻辑。
- 异步任务（视频等）只在拿到 remote task ID 之前允许切换。
- 用量只记在实际执行的模型上；失败的原模型如果没有产生用量就不记。

### 4.5 价格比较（`not_pricier`）

- “不更贵”：备选的每个计费维度单价都不高于原模型（LLM 是输入、输出、缓存读写；媒体是各自的计费单位），按 `catalog.cost_in_usd` 折成同一币种比较（Q3）。
- 任何一边价格未知、币种无法折算、计费维度对不上，都算“不能证明不更贵”，排除。
- 价格取库存里钉住的 `InventoryPricing`，和调用时 `PinnedPricingSnapshot` 用的是同一份。Credit 价目表上线后是否改用 Credit 价见 Q3。
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
| `routing.usecase.set` | 管理员写管理员层；用户写自己的层和自己的组 | `{target: "zone" \| "user" \| "group:<id>", usecase, model?, on_unavailable?, clear?: ["model" \| "on_unavailable"], revision}` |
| `routing.groups.list` / `.create` / `.update` / `.delete` | 用户本人 | 只能看、改自己的组 |
| `routing.bindings.list` / `.set` / `.remove` | 用户本人 | App 绑定 |
| `routing.default_snapshot.get` / `.accept` | 读：所有人；接受：管理员 | 快照与“更新可用”的差异；`accept` 可以只接受部分用途 |
| `routing.preview` | 不变 | 保留给高级视图；参数补 `view`，与 `routing.usecases` 一致 |
| `routing.get` / `routing.update` | 管理员 | 保留 `routing_commands` 与 `provider_weights` |

- 写操作都带版本号做并发检查（管理员层沿用 `settings_revision`，RDB 里的内容各自带 `version`）。
- websdk 同步类型和字段白名单（`buckyos-websdk/src/aicc_client.ts:543-596`），顺手补上 `RoutingUpdateRequest.routing_commands`。

### 4.8 Trace、用量、错误、通知

- trace 新增：每一格的来源层（`usecase_source`: `default` / `snapshot` / `zone` / `user` / `group:<id>`）、命中的 App 绑定或会话组、`on_unavailable` 及其来源；发生切换时记 `fallback_from`、`fallback_to`、`fallback_policy`、`failure_kind`、原模型的上游错误。
- 用量记录标出“由不可用策略切换而来”，AI Center 用量页可以按此筛选。
- 错误：没有配置切换时返回原模型的真实错误（Provider 实例名、精确模型、上游状态码与原文），不改写成“没有可用模型”。`no_candidate_model` 只表示没有接入满足请求的模型，错误详情里给出用途名，并指向“用途”页。
- 通知：Provider 或模型连续失败、被钉住的模型消失、发生了策略切换，都记事件；同一故障恢复前只记一次。是否推送 System Notify 沿用 11-23 TODO T5 的做法。

### 4.9 界面（Desktop › AI Center）

按手机宽度设计：表格在窄屏下变成每个用途一张卡片，“当前模型 / 来源 / 不可用时”竖排。

- **用途页**（新，导航里放在 Routing 前面，普通用户可见）：§3.1 的表；点一行打开编辑面板——选模型（只列出对自己可见、满足用途要求的模型，按家族分组，标价格）、选不可用策略（四个选项各附一句解释，并实时显示“会换到谁”）、恢复为跟随下一层。顶部切换视角：我 / 某个策略组 / 某个 App；管理员多一个“Zone 默认”。
- **策略组页**（新）：组列表、编辑（复用用途页的表）、绑定到 App（从 App / Agent 列表选）。
- **Routing 页**（现有）：改名为“路由细节”或并入用途页的高级视图，继续显示候选树、权重和 trace；只对管理员显示写操作。
- **添加 Provider 向导**：在验证之后、确认之前加一步“用途覆盖”：对比“现在”和“加上以后”的用途表，列出新覆盖的用途、默认模型会变的用途（每条可以取消勾选，取消就把旧模型写进管理员层）、仍未覆盖的用途。确认页显示摘要。
- **首页**：未覆盖的用途数量、“默认模型有更新可用”、最近的策略切换，各一行，点进对应页。
- 管理员界面里 fallback 的说明改成三种策略的说明；`not_pricier` 与 `next_model` 的说明里写明“系统按已知价格判断，价格未知的模型不会被选为‘不更贵’”。

### 4.10 CLI 与 Agent 手册

- `buckyos ai routing`（`buckyos-websdk/cli/modules/ai.ts:40-44`）改成显示用途表；新增 `buckyos ai usecase set <usecase> [--model m@p] [--on-unavailable none|same_model|not_pricier|next_model] [--zone|--group <id>]` 与 `buckyos ai routing-group …`。
- `tools/buckyos-agent` 的 `--no-fallback` 只对应“请求关闭不可用策略”。
- 手册：`apps/jarvis_agent/agent/cmd_skills/buckyos_admin/references/providers.md`（用途表、三种策略、故障排查、添加 Provider 后检查覆盖）、`cmd_skills/xllm/SKILL.md` 与 `references/runs.md`（删掉 `llm.fallback` 的说法）。

---

## 5. 任务

按顺序做，每步单独可合入、单独验收。

### R0 去掉自动切换（= 11-23 TODO 的 T1–T3）

- [ ] 执行引擎只跑选中的模型：保留同模型重试和重新登录；删除候选循环、quota failover、`inference.rs` 的 `failover` 列表（`execution/mod.rs:1635-1638`、`:1830-1861`，`service/inference.rs:1169-1225`）。
- [ ] 删除 `runtime_failover`、`allow_exact_model_fallback`（API、`EffectiveRoutingPolicy`、websdk 类型与白名单、Desktop `aicc_mgr.ts:2358-2364` 的默认值）。`allow_fallback` 改名为请求侧的“关闭不可用策略”，默认不关闭（R3 之前没有策略可关）。
- [ ] 硬过滤拆成静态与运行状态：`hard_filter_runtime`（`routing/mod.rs:1013`）里只留 `enabled`、凭据已配置；`model_available` 冷却、`health`、熔断移出路由，只用于展示、告警和 R3 的失败分类。请求显式 `max_latency_ms` 仍是该请求的硬门槛。
- [ ] 删除内置 fallback：七个 LLM 用途改 strict、删除 `llm.fallback` 及其 38 个规格、`parent` 模式整个移除（`model_defaults.rs:945`、`:966`、`:1143` 起，`AiccFallbackMode::Parent`）；同步 `model_defaults.rs` 头部的目录契约注释（有测试逐项比对）。
- [ ] `no_candidate_model` 只表示静态无候选；选中模型故障时返回上游真实错误。
- [ ] 一次成功或管理员修改该 Provider 配置后，立即清除它的故障标记。
- [ ] 删除默认打开 fallback 的调用方设置：`aiws/host.js:31`（及 `aiws_test.js:281`、`:288` 的断言）、`tools/buckyos-agent/lib/aicc.ts:144-145` 与 `rootfs/libexec` 里的拷贝。
- [ ] 改测试断言，清单见 11-23 TODO T6 第一条。

### R1 用途配置与三层

- [ ] `AiccSettings.session_config` 改名 `routing`（`settings/mod.rs:30-38`）；`ServiceModelAssembler` 把它放进 `system` 槽（`model_defaults.rs:526-534`）。
- [ ] 新类型 `AiccUsecaseRoute { model: Option<LockedValue<String>>, on_unavailable: Option<LockedValue<OnUnavailable>> }` 与 `AiccUsecaseRoutes { usecases: BTreeMap<String, AiccUsecaseRoute> }`，放进 `buckyos-api/src/aicc_client.rs`；管理员层的 `routing` 增加 `usecases`。
- [ ] RDB：`aicc_routing_layer`、`aicc_routing_group`、`aicc_routing_binding`、`aicc_default_route_snapshot`（R2 用），存储版本升到 4（与用户策略 TODO Q14 同一次）。
- [ ] 求值（§4.2）：新模块 `routing/usecase.rs`，纯函数，输入各层内容 + 注册表 + 调用者 + 请求，输出 `primary` 与每格来源。
- [ ] 修路由策略分层：请求的策略叠加在管理员层上而不是替换（`service/inference.rs:214-217`）。
- [ ] kRPC：`routing.usecases`、`routing.usecase.set`（§4.7），websdk 同步。
- [ ] 校验（§4.1）。
- [ ] trace 增加来源层。

### R2 默认路由稳定

- [ ] 池内排序只用静态输入（Q12）：去掉 `estimate_model_cost` × 请求 token 估算参与排序、去掉 session 历史软优先（`routing/mod.rs:1156-1159`）及 `aicc_session_route_history` 表（如果 owner 同意删除 session 历史，见 Q12）。
- [ ] 默认路由快照（§4.3，Q1）：重算时机挂在 `provider.add` / `.delete` / `.update(enabled)` 的成功路径上；后台对比任务挂在 Provider 刷新、metadata 更新、`routing.update` 之后；事件 `default_route_update_available`、`pinned_model_gone`；`routing.default_snapshot.get/accept`。
- [ ] 首次升级到这个版本时用当前配置算一次快照（`reason = initial`）。

### R3 不可用时的三种策略

- [ ] `OnUnavailable` 四个值；R0 留下的请求侧开关改为“关闭不可用策略”。
- [ ] 失败分类（Provider 级 / 模型级 / 不触发），集中在一个函数里，有单元测试逐类覆盖（Q2）。
- [ ] 备选计算（§3.4、§4.4）：`same_model` 筛选、`not_pricier` 与 `next_model` 复用路由器加停用集合重算。
- [ ] 价格比较（§4.5）。
- [ ] 执行：每次请求最多一次；重新准入额度、物化资源、检查可见性；错误同时带两边原因；用量归到实际模型。
- [ ] `routing.usecases` 里每行的“不可用时会换到谁”（按 Provider 级故障预演）。
- [ ] Q5 答复后实现或不实现“已知故障直接跳过”。

### R4 策略组与 scope

- [ ] 策略组、App 绑定的存储与 kRPC（§4.7）。
- [ ] 求值里加入 App 组与会话组两层。
- [ ] AICC 请求新增 `route_group`（推理、helper、`route.resolve`），websdk 同步；所有权校验与 trace 标注（§4.6）。
- [ ] 删除请求里的 `session_overlay`，以及 `AiccRouteOverlay` 中只为它存在的 `logical_profile`、`logical_profiles`、`active_logical_profile`、`inherit`、`revision`、`ttl_seconds`（Q10）。
- [ ] lib_opendan：会话配置新增 `route_group`；沿 `parent_session` 链取值；经 `llm_context` → `agent_tool/src/xllm.rs` 传给 AICC。浏览器 / Deno 版 xllm（websdk）在宿主提供会话信息时同样透传。
- [ ] 会话设置界面里选择策略组（OpenDAN WebUI、Jarvis 对话）。

### R5 覆盖与添加 Provider

- [ ] `provider.validate` 的 `routing_candidates` 改成“加上这个 Provider 以后整个 Zone 的用途表”：所有 29 个用途（不只 LLM），每个用途给出加之前、加之后的模型和覆盖状态；复用 §4.3 的重算逻辑。
- [ ] 向导新增“用途覆盖”一步，取消勾选的变化写进管理员层（§4.9）。
- [ ] 首页与用途页显示未覆盖的用途和原因（没有接入该 API 类型的模型 / 有模型但不满足用途要求，例如上下文不够、没有 tool call）。
- [ ] 删除或停用 Provider 前，确认框列出会受影响的用途（由谁接替或变成未覆盖）。

### R6 界面、CLI、手册

- [ ] 用途页、策略组页、Routing 页调整、首页三行（§4.9）；i18n 新键写齐现有的所有语言。
- [ ] CLI（§4.10）。
- [ ] 手册（§4.10）。

### R7 测试与验收

- [ ] 单元：层次合并与锁；请求只能收紧；不可见模型跟随下一层；快照的重算与“更新可用”；三种策略在单 Provider、同模型多 Provider、多模型多 Provider 下的备选；`not_pricier` 遇到价格未知、币种不同、阶梯价；每次请求最多切换一次；失败分类表。
- [ ] 回归（来自 11-23 TODO）：两个 Provider 同池，第一个连续失败 10 次，第二个收到 0 次请求；404 之后立刻再请求仍选同一模型；没有配置策略时任何失败都不换模型。
- [ ] 稳定性：Provider 刷新出一个权重更高的新版本模型，默认路由不变、出现“更新可用”；接受后才变。
- [ ] 策略组：App 绑定生效；会话绑定对子会话生效、子会话自己的绑定优先；用别人的组 ID 不生效。
- [ ] AICC 快照测试（wire / 库存 / 发现）保持不变：本 TODO 不应改变任何对上游发出的请求。
- [ ] 验收用例与旧断言：`test/aicc_test/acceptance/run_t1_gateway.ts`、`logical_entrypoint_baseline.json`、`doc/aicc/maintenance/acceptance/cases/` 下断言 failover 的用例。
- [ ] 真实 Zone DV：
  1. 只接 SN：用途表与覆盖正确；停掉 SN，所有请求返回真实错误，没有换模型。
  2. SN + 一个直连 Provider（同一模型两边都有）：`llm.code` 设 `same_model`，停掉直连 Provider，切到 SN 上的同一模型，trace 与用量正确。
  3. `not_pricier`：备选比原模型贵时不切换；补上价格后切换。
  4. 建两个策略组，分别绑到 Jarvis 和一个 Agent 会话，在该会话里派生子会话，确认各自生效。
  5. 添加 Provider：向导显示覆盖变化；取消其中一条后，该用途保持旧模型（来源显示“管理员”）。

---

## 6. 不做

- 不做任何基于健康、延迟、价格估算的自动选择或切换；不可用策略只在用户显式打开后生效。
- 不做按请求参数（如图片 `aspect_ratio`）换模型。
- 不做“只选模型不选 Provider”的选择（除非 Q7 改变）。
- 不做跨 API 类型的切换。
- 不让请求方（App）替用户选模型或打开不可用策略；App 想用特定模型就直接请求精确模型。
- 不做会话绑定在 AICC 里的存储（除非 Q9 改变）。

## 7. 与其它 TODO 的关系

- [显式 fallback TODO](aicc-explicit-fallback-todo.md)：被本文取代，见文首。
- [用户策略与 Credit TODO](aicc-user-policy-credit-todo.md)：可见性决定用途表里能选哪些模型；存储升级合并为一次；Credit 价目表上线后 `not_pricier` 的价格来源见 Q3。两者的主体写法保持一致（`user:<id>`、`app:<app_id>`）。
- [配置订阅更新 TODO](aicc-config-subscription-todo.md)：metadata 订阅更新会触发 §4.3 的“更新可用”对比，不直接改变路由。
- [自定义 Provider 自动探测 TODO](aicc-custom-provider-auto-probe-todo.md)：探测完成后同样走 R5 的覆盖步骤。
- [AICC 当另一个 AICC 的 Provider](aicc-zone-aicc-provider-todo.md)：远端 AICC 作为一个 Provider 实例参与用途表，三种策略同样适用；远端自己的路由不在本文范围内。

## 8. 文档同步

随代码一起改：

- `doc/aicc/aicc_router.md`：§9.3 的模式表改成 `on_unavailable` 四个值；§9.4 示例改写；§10.4 写明池内只用静态输入；§11、§12 重写（AICC 维护用户层、策略组与 App 绑定，请求只能收紧，删除 `session_overlay` 与 `SessionLogicalProfile`）；§18 验收场景补充。
- `doc/aicc/aicc 逻辑模型目录.md`、`aicc_api设计.md`、`frozen_user_api.md`（新方法与字段）、`aicc_runtime_durable_data_schema.md`（新表）、`aicc-models-mgr.md`。
- `product/ai_center/AI_Center_PRD.md` §5.2（添加 Provider 的覆盖步骤）、§5.5 与 §9（用途页、策略组）。
- 手册见 §4.10。

## 9. 待 owner 确认

括号里是建议；没有答复前不实施对应部分。

- **Q1 默认路由要不要钉住。** “只有添加或删除 Provider 会影响”要求 Provider 刷新出新模型、metadata 更新时默认路由也不变。（建议：采用 §4.3 的快照——变化只提示、管理员确认后生效；被钉住的模型消失时立即重算并通知。不加机制的选项：不做快照，只靠 R0 去掉运行时输入，Provider 刷新和 metadata 更新会直接改变默认路由，事件里记一笔。）
- **Q2 “按其 disable 的逻辑”停用的是什么。**（建议：按失败类别——连接失败、超时、5xx、重新登录后仍认证失败停用整个 Provider 实例；模型不存在、模型级配额耗尽只停用这个精确模型。否则只接 SN 的用户遇到单个模型 404 时，`next_model` 也无模型可换。）
- **Q3 “不会更贵”的口径。**（建议：逐维度单价都不高于原模型，按 `cost_in_usd` 折算；任何一边价格未知即视为更贵；用 Provider 的成本价，也就是 Zone 实际付的钱。Credit 价目表上线后，是否对普通用户改用他实际被扣的 Credit 价另议。备选：按本次请求的 token 估算比较总价——更宽松，但结果随请求大小变化。）
- **Q4 一次请求最多切换几次。**（建议：1 次，沿用 router §9.3 规则 5。同模型有三个 Provider 时也只试一个备选。）
- **Q5 已知故障时是否直接跳过原模型。** 原模型的 Provider 已被标为故障（连续失败熔断）时，每个请求都要先等它超时才切换。（建议：只在这个用途打开了不可用策略时，直接用备选并在 trace 标“因已知故障跳过”；没打开策略时照常发给原模型。这会让健康状态在用户选择了策略时影响路由，属于“通过手工配置允许”。）
- **Q6 普通用户能否打开三种策略。**（建议：能，管理员可以在管理员层锁定 `on_unavailable` 禁止。Credit 计费上线前，用户打开 `next_model` 产生的费用由 Zone 承担，如果你担心这一点，可以改成 `next_model` 只有管理员能设。）
- **Q7 用途的“模型”能否只选模型、不选 Provider。** 例如选“claude-opus-5.5”，由同模型的多个 Provider 按默认顺序决定。（建议：第一期只选精确模型 `model@provider`，多 Provider 的情况用 `same_model` 策略表达。）
- **Q8 层次与管理员的范围。**（建议：默认 < 管理员 < 用户 < App 组 < 会话组，管理员锁定的格子任何层不能改；所选模型对调用者不可见时这一格跟随下一层并在用途表上标出。第一期管理员不建共享策略组、不替所有用户给 App 绑组，只用管理员层 + 锁。）
- **Q9 会话绑定放在哪里。**（建议：放在 lib_opendan 会话配置，调用时沿 `parent_session` 链取最近的，请求带 `route_group`；AICC 只存策略组和 App 绑定。备选：AICC 也存会话绑定，请求带会话链；好处是 AI Center 能列出全部绑定，坏处是 AICC 要跟着会话的生命周期清理。）
- **Q10 删除请求里的 `session_overlay` 和旧的 fallback 目标模式。** 现在没有实际调用方发 `session_overlay`（Agent 对话、buckyos-agent、AIWS 都不发）。（建议：整体删除，连同 `SessionLogicalProfile`、`logical_profiles`、`active_logical_profile`；请求的收紧项都放进 `RoutePolicy`。`target_exact`、`target_logical`、`parent` 一起删除，由三种策略取代。）
- **Q11 `routing_commands` 和权重怎么办。** 它们是现在路由页唯一能改的东西。（建议：保留为管理员层的高级调权手段，影响的是默认展开和快照；用户层和策略组不开放。）
- **Q12 池内排序和会话历史。** 现在 `cost_first` 用请求的 token 估算排序，还按会话历史软优先，都会让同一用途选到不同模型。（建议：排序只用静态单价，价格未知的排在已知之后、再按默认顺序；删除会话历史软优先和 `aicc_session_route_history` 表。）
- **Q13 未覆盖的用途。** 去掉 `llm.fallback` 后，只接了一个小模型的 Zone 里 `llm.plan`、`llm.code` 可能没有候选。（建议：不恢复内置兜底；向导和首页列出未覆盖的用途，并为每个给出“用 X 覆盖”的建议，用户确认后写进管理员层。建议只在 API 类型相同、满足该用途硬要求的模型里选。）

## 10. 验收

- 用户在手机上打开 AI Center，能看懂每个用途现在用哪个模型、这个选择来自哪一层、Provider 出问题时会发生什么。
- 只有添加、删除、启用、停用 Provider 和管理员接受“更新可用”会改变默认路由；Provider 故障、metadata 更新、Provider 模型列表刷新都不会直接改变它。
- 没有打开不可用策略时，任何故障都不换模型，错误里有 Provider、模型和上游原因。
- 三种策略只在手工打开后生效，每次请求最多换一次，trace、用量、事件都能看到原模型、实际模型和原因。
- 策略组按 App 和会话（含子会话）生效，并且只对它的主人生效。
- 添加 Provider 时，向导显示整个 Zone 的用途覆盖变化。

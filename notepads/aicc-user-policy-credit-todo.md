# AICC 用户策略（配额中心）与 Credit 计费充值 TODO

2026-11-27 立项，未实施，代码基线 `fcd23bfb4`。

本文把现有的 **Token 额度** 和 **资源可见性** 合并成一套“用户策略”，并新增 **价目表、Credit 余额、扣费、充值 API**。它取代 [AI Center Token Quota 与资源可见性 PRD](../product/ai_center/AI_Center_Quota_PRD.md) 中的额度模型（周期 / 总量 token 规则、预占、总量账户与充值）和可见性策略的存放方式；该 PRD 中仍然有效的部分（调用身份与归属、许愿格 AICC 归属、隐藏资源不泄露、错误只按错误码判断）在 §2.2 列出。

实施方先读 §1、§2，再按 §9 的步骤做。§10 的问题答复之前，只做 §9 里标了“不依赖决策”的步骤。owner 答复后把裁决写进 §10 对应条目，再改正文。

## 1. 目标与原则

### 1.1 owner 原话（2026-11-27）

> ## AICC的用户用量控制和计费充值
>
> 现在的额度 + 可见性 要合并成 AICC的用户策略，根本上是管理员为用户配置的”配额中心“。
>
> 管理员可以给用户(组) / app添加策略，用户只能给自己的app添加策略
>
> 用户(组)来自buckyos系统，AICC不维护
>
> 最简单的策略是不限额，随便用。但系统总是会面向用户，app和用户组记录用量.
>
> 策略的内容(余额为0或则到达窗口Limt，则不可用，有多个limit的时候，都生效）
>
> - provider是否可见？价格系数是多少，有几个时间窗口limit?
> - 模型是否可见？价格怎么算?有几个时间窗口limit?
> - 扣费来自usage(一次基于 model@provider的使用），因此价格最终会核算到 model@provider
> - 配置模型价格的机制，可以允许管理员先配置价格再加provider
>
> 用户可以有余额(统一是credit)，使用有价格的模型后，会自动扣除。余额可以是负数 （一次usage太大）
>
> 管理员可以查询用户余额，并给用户充值
>
> 充值是一个API，允许管理员接入真实的支付网关，根据支付结果来给给特定用户充值

### 1.2 原则

- **P1 一套策略。** 可见性、价格系数、时间窗口 limit 都是“策略”的字段，存在同一张表里、用同一套求值。删除现有的 `visibility.*`、`quota.*` 两套接口和存储。
- **P2 主体来自系统。** 用户、UserType、系统用户组都从 system-config 读（现在的 `service/identity.rs` 已经这样做），AICC 不保存成员关系，不做缓存以外的副本。
- **P3 谁能配。** 管理员可以给 UserType 组、系统用户组、用户、App、“用户 + App”配策略；普通用户只能给自己的 App（含自己的 Agent）配策略。
- **P4 默认不限。** 开箱策略是“不限额、随便用”（沿用现在的 `type:admin` / `type:user` 全部可见）。不配置 limit 就没有 limit。
- **P5 总是记账。** 不管策略是什么，每次 usage 都按用户、App、用户组记录用量和 Credit 金额。
- **P6 不可用的条件只有三个**：资源不可见；余额 ≤ 0 且本次模型有价格；任何一条适用的窗口 limit 已达到。多条 limit 同时生效。
- **P7 价格核算到 model@provider。** 扣费的依据是一次 usage，价格最后落到这次实际执行的精确模型 `provider_model_id[:variant]@provider_instance_name`。
- **P8 价格可以先于 Provider 配置。** 价目表按与 Provider 无关的模型身份配价，之后加进来的 Provider 只要映射到这个模型就有价格。
- **P9 余额统一是 Credit，可以为负。** 有价格的使用自动扣除；一次 usage 太大可以把余额扣成负数。
- **P10 充值是 API。** 管理员手工充值和支付网关回调走同一个幂等接口；AICC 不接支付、不管钱和 Credit 的换算。

已有裁决里和本单相关、继续有效的：

- AICC 不自动换模型或 Provider（[aicc_router.md §9](../doc/aicc/aicc_router.md)）。被策略拒绝的候选不能触发自动切换，只有管理员手工配置的 fallback 才能改用别的模型（[显式 fallback TODO](aicc-explicit-fallback-todo.md)）。
- 价格宁可不准确也不估算（[配置订阅 TODO](aicc-config-subscription-todo.md) P8）。没有显式配置的地方一律没有价格。
- 默认允许 breaking change，不做旧格式兼容（`AGENTS.md`）。存储升级方式见 §10 Q14。
- 机制越少越好：能用已有部件和配置表达的，不加新机制。

### 1.3 我对原话的理解（待 owner 确认，见 §10）

- **“余额为 0 或到达窗口 limit 则不可用”是调用前的门槛，不是预占。** 调用前只看“现在余额是否 > 0、每条 limit 现在是否还没用满”，调用后按实际 usage 扣。所以一次大调用、或者余额很少时的并发调用，可以把余额和窗口用量扣过头；这正是“余额可以是负数（一次 usage 太大）”。现在的预算估算、预占、待核对、“到期按预占全额扣”全部删掉（Q1）。
- **“价格系数”是乘在价目表价格上的倍数。** 本次扣费 = 价目表里这个模型的 Credit 价 × 适用的 Provider 系数 × 适用的模型系数。系数 0 等于免费。
- **“模型价格怎么算”** 指价格的来源：价目表里按模型身份配的价、按精确模型配的例外价、（可选）Provider 实际成本按管理员设定的汇率折成 Credit（Q7），以及策略里的模型系数。
- **策略的叠加按“越具体越优先、App 侧只能收紧”：** 用户侧的三层（UserType 组 < 系统用户组 < 用户）里，更具体的一层设了某个字段就以它为准；App 侧的策略（App、用户 + App，不管是管理员还是用户自己配的）只能再加限制，不能放宽。这改变了现在可见性“按组取并集、不能对个人拒绝”的语义，见 Q2。
- **组上的 limit 对每个成员分别计算**，不是全组共用一份；App 上的 limit 仍是所有用户共用（与现在一致）。见 Q3。
- **只有用户有余额。** App、Agent、用户组没有余额；Agent 的消耗扣其所属用户，许愿格按运行归属扣（沿用现在的归属）。

## 2. 现状（2026-11-27 核对，基线 `fcd23bfb4`）

| 方面 | 现在的代码 | 与目标的差距 |
| --- | --- | --- |
| 可见性 | `src/frame/aicc/src/visibility.rs`；表 `aicc_visibility_policy`（主体 `type:*` / `group:*`，允许项三种），修订号在 `aicc_state.visibility_revision`；默认策略首次建库时写入（`visibility_seeded`） | 与额度分开存、分开管理（P1）；只能授予不能对个人收紧；不能给用户、App 配 |
| 额度规则 | `quota/mod.rs`、`storage/quota.rs`、`storage/quota.sql`、`service/quota.rs`；对象 `user` / `app` / `user_app` × 资源 `all` / `provider` / `model`；周期（4h/24h/72h/week）与总量两种，单位都是 token | 没有 Credit；组不能配额度；单位只有 token |
| 准入 | `ExecutionEngine::admit`（`execution/mod.rs`）→ `QuotaService::admit`：`quota::attempt_budget` 按 UTF-8 字节估输入上界 + 输出上限，写 `aicc_quota_attempt` 并原子预占全部计数器 / 账户；推理入口另有只读的 `QuotaService::precheck` | 预占、预算估算、待核对、到期全额扣都不再需要（§1.3）；只覆盖 `Llm` / `Decision` / `Embedding*`（`quota::is_metered`），图片、音视频不受控 |
| 结算 | `QuotaService::settle` 只用 token 数；`AttemptSettlement::Usage` 只带 usage。`finish_success` 里先 `settle` 再算 `completion_cost`；启动失败但带 usage 的尝试在 `execute` 的尝试循环里结算，费用到 `finish_failure_with_usage` 才算 | 结算时要能拿到 Provider 成本（Q7 的汇率来源依赖它） |
| 总量账户与充值 | `aicc_quota_account` / `aicc_quota_grant`，按“对象 + 资源”一个 token 账户，`recharge_id` 幂等 | 换成每个用户一个 Credit 余额；充值要能给支付网关用 |
| 管理权限 | `QuotaService::manages()` = Admin/Root 账户或任何 System 主体；Admin 名下的 App 实例会被识别为 Admin 账户（`AuthorizedCaller::is_admin`） | 充值涉及真钱，不能让管理员装的任何 App 都能充值（Q12） |
| 价格 | 只有 Provider 成本：库存里的 `InventoryPricing`（discovery / Provider Rules `model_pricing`），调用时钉成 `PinnedPricingSnapshot`（`execution/mod.rs`），完成时 `completion_cost` 算出 `AiCost { amount, currency }`，Provider 上报的实际费用优先 | 没有面向用户的价格；没有与 Provider 无关的价目表 |
| 用量 | `aicc_usage_event`（`storage/schema.sql`）有 `user_id`、`caller_app_id`、`finance_*`、`operator_id`、`attribution_json`；`usage.query` 的分组没有用户组 | 不记录调用时所属的组；没有 Credit 金额 |
| 身份 | `RuntimeAuthorizer` 把会话解析成 `AuthorizedCaller`（用户、UserType、组、业务 App、`on_behalf_of`）；每次准入读 system-config，只缓存 Agent owner | 已满足 P2，不用改 |
| 错误码 | `AiccErrorCode::{QuotaExhausted, QuotaBudgetExceeded, QuotaUnavailable, ResourceNotPermitted, AttributionUnavailable}`（`buckyos-api/src/aicc_client.rs`），详情 `QuotaDenialDetails` | 去掉 `QuotaBudgetExceeded`，新增余额不足，详情改形状（§3.7） |
| 存储版本 | `storage/mod.rs` 的 `STORAGE_SCHEMA_VERSION = 3`；版本不符时 AICC **拒绝启动**（`UnsupportedSchema`），没有升级路径 | 升到 4 会让所有已部署 Zone 的 AICC 起不来，见 Q14 |

### 2.1 能复用的部件

| 部件 | 位置 | 用在哪 |
| --- | --- | --- |
| `Pricing`（token 各维度、`unit` + `amount`、`tiers`、`rules`、`addons`） | `catalog/schema.rs` | 价目表条目直接用它，`currency` 写 `CREDIT` |
| `PinnedPricingSnapshot::from_pricing` / `completion_cost` | `execution/mod.rs` | 调用时钉住 Credit 价格，完成时按 usage 算 Credit 金额，与 Provider 成本同一套代码 |
| `matched_rule_amount` + `request_match_context` | `call/mod.rs` | 价目表条目里的 `rules`（按质量、尺寸等请求参数计价）选价 |
| `visibility::grants` / `access_for` / `grant_list` / `validate_entries` | `visibility.rs` | 可见性求值和预览的“授权来源”，改造成按层覆盖（§3.2） |
| `QuotaPeriod` 固定 UTC 窗口、`aicc_quota_counter` 的形状、`aicc_quota_audit` | `buckyos-api/src/aicc_quota.rs`、`storage/quota.sql` | 窗口 limit 计数器、变更审计 |
| 充值的幂等写法（同号同参只入账一次、同号不同参拒绝） | `storage/quota.rs` 的充值函数 | Credit 充值 |
| `AccountFacts`、`UserDirectory::{account, user_apps, defined_groups}` | `service/identity.rs` | 主体、用户自己的 App 校验 |
| `AiccOnBehalfOf` / `AiccAttribution` | `buckyos-api/src/aicc_quota.rs` | 许愿格、Agent 的扣费归属，不改 |
| RBAC 策略尾部 `system/rbac/policy`（`overlap_rbac_policy`） | `buckyos-api/src/rbac_config.rs` | 管理员授权支付网关的 App 调用充值接口 |

### 2.2 旧 PRD 中继续有效的部分

- §3.1 的 App / Agent / 系统服务归属规则（AppId 等于 AgentId；Agent 的消耗计入所属用户；系统服务代执行时保留业务 App），§3.3 许愿格 AICC 归属全部保留。
- §3.4 中“最终模型列表 = 策略允许 ∩ Provider 实际清单”“服务端过滤、不靠前端隐藏”“隐藏资源不能经由逻辑路由 / fallback / Usage / Trace / 错误被枚举”“策略无法求值时返回权限状态不可用、不放行”保留；“按组取并集、不能对个人拒绝”被 §3.2 替换（Q2）。
- §5.1 固定 UTC 窗口定义保留。
- §7.3 中“用户 / App 总额耗尽时换模型不能绕过”“额度恢复不自动重放任务”、§9.6 “他人只看到通用提示”保留。
- 被替换：§5.3 总量额度、§6 的预占口径、§7.1 第 3–5 步、§7.2 的待核对与暂估、§7.4 的“本次预算超额”。

## 3. 设计

### 3.1 策略

一条策略 = 主体 + 来源 + 内容。

**主体**

| 主体 | 写法 | 谁能配 | 作用于 |
| --- | --- | --- | --- |
| UserType 组 | `type:admin` / `type:user` / `type:limited` | 管理员 | 该类型的每个用户 |
| 系统用户组 | `group:<group_id>` | 管理员 | 组内每个用户 |
| 用户 | `user:<user_id>` | 管理员 | 这个用户 |
| App | `app:<app_id>` | 管理员 | 所有用户通过这个 App（或 Agent）的调用，limit 全 Zone 共用 |
| 用户 + App | `user_app:<user_id>/<app_id>` | 管理员；用户本人（只能是自己的 App / Agent，校验沿用现在 `validate_draft` 的口径） | 该用户通过该 App 的调用 |

前三种是“用户侧”，后两种是“App 侧”。同一 `(来源, 主体)` 只有一条策略；用户给自己的 App 配的是 `source = user` 的 `user_app` 策略，管理员给同一个组合配的是 `source = admin` 的另一条，两条同时生效。

**内容（草案）**

```json
{
  "policy_id": "pol_3f2a…",
  "source": "admin",
  "subject": "group:students",
  "enabled": true,
  "all_providers": false,
  "limits": [ { "period": "24h", "unit": "credit", "amount": "20" } ],
  "providers": {
    "openrouter": {
      "visible": "selected_models",
      "price_coefficient": 1.2,
      "limits": [ { "period": "week", "unit": "credit", "amount": "50" } ]
    },
    "local-ollama": { "visible": "all_models", "price_coefficient": 0 }
  },
  "models": {
    "gpt-6.1-sol@openrouter": {
      "visible": true,
      "price_coefficient": 0.5,
      "limits": [ { "period": "4h", "unit": "token", "amount": "200000" } ]
    }
  },
  "version": 3
}
```

- `all_providers`：当前及以后加入的全部 Provider 的全部模型可见（即现在的“全部 Provider（跟随清单）”）。
- `providers.<实例>.visible`：`all_models`（跟随清单）/ `selected_models`（只有 `models` 里 `visible: true` 的精确模型）/ `hidden`。不写表示这一层对该 Provider 的可见性没有意见。
- `models.<精确模型>.visible`：`true` / `false`；不写表示没有意见。精确模型必须属于对应 Provider（沿用 `validate_entries` 的校验）。
- `price_coefficient`：非负小数，不写表示没有意见。
- `limits`：挂在顶层的作用于全部资源，挂在 Provider / 模型下的只作用于该资源。每条 `{ period, unit, amount }`：`period` 沿用 `QuotaPeriod` 的四种；`unit` 为 `credit` 或 `token`（Q5）；`amount` 为非负十进制字符串，`0` 表示禁止使用。
- App 侧策略不允许写 `price_coefficient`（Q4）；用户自己的 `user_app` 策略只允许写可见性收紧（`hidden` / `false` / `selected_models`）和 `limits`。
- 字段只是配置，不保证资源存在。所指 Provider 或模型不在清单里时策略保留、在界面标“资源不可用”，不迁移到别的模型（沿用旧 PRD §4.1）。

**默认策略**（首次建库写入，之后是普通策略，系统不自动恢复）：`type:admin` 与 `type:user` 各一条 `all_providers: true`，其他什么都不写；`type:limited` 没有策略。效果就是“不限额、随便用”，价目表为空时也不扣费。

### 3.2 求值：一次调用的有效策略

输入：额度归属用户 U（含 UserType、组）、业务 App A、精确模型 M（属于 Provider P）、当前清单。输出：是否可见、Provider 系数、模型系数、适用的 limit 列表，以及每一项的来源策略（用于预览和拒绝说明）。

**用户侧（按层覆盖）**

1. 层次从低到高：`type:<U 的类型>` → U 所在的每个 `group:*` → `user:U`。只看 `enabled` 的策略。
2. 每个字段由设了它的最高一层决定。字段的粒度：
   - 可见性：先看模型 M 的 `visible`，没有意见再看 P 的 `visible`（`all_models` → 可见，`hidden` → 不可见，`selected_models` → 该层只允许它列出的模型），都没有意见再看 `all_providers`（`true` → 可见）。三项都没意见的层跳过。全部层都没意见 → 不可见（无策略不等于开放）。
   - Provider 系数：P 的 `price_coefficient`；模型系数：M 的 `price_coefficient`。都没有意见 → 1.0。
   - limit：以 `(资源, period, unit)` 为键，最高一层设了这个键就以它为准；不同键全部生效（例如 4h 与 week 同时生效）。
3. 组这一层有多个组同时设了同一字段时，取对用户最有利的值：可见优先于不可见，系数取最小，同键 limit 取最大（Q2）。

**App 侧（只能收紧）**

4. 依次叠加 `app:A`、管理员的 `user_app:U/A`、用户自己的 `user_app:U/A`：
   - 可见性：任何一条说 M 不可见（`hidden`、`false`、或 `selected_models` 而没列出 M）就不可见；不能让用户侧不可见的资源变可见。
   - limit：全部追加到适用列表，不覆盖用户侧。`app:A` 的 limit 计数用 App 全局计数器，`user_app` 的用该用户该 App 的计数器。
5. 最后与 Provider 实际清单取交集（停用的 Provider 没有清单）。

**系统主体**：没有代表用户的 System / Device 主体不受策略约束、不扣费、只记用量（沿用现在）。

**计数器作用域**：用户侧所有层的 limit 都计在 U 自己的计数器上，所以组上的 limit 是“每个成员各 N”（Q3）；没有“全组共用”的计数器。

求值放在新模块 `src/frame/aicc/src/policy/`（纯函数，不碰数据库），取代 `visibility.rs` 和 `quota/mod.rs` 中的规则部分。可见性过滤的所有生效点不变：`models.list`、`provider.list`、路由解析与预览、推理调用（主模型与 fallback 都检查）、诊断和 trace 的隐藏处理。

### 3.3 价目表

价目表是 Zone 级的一份配置，只有管理员能改，和策略分开存。

**条目**

| 键 | 含义 | 用途 |
| --- | --- | --- |
| `model:<model_driver_id>:<origin_model_id>` | 与 Provider 无关的模型身份（`ModelUid` 的前两段） | 主要配价方式；在任何 Provider 加入之前就能配（P8）。界面从 Model Driver 目录里选，不从清单里选 |
| `exact:<provider_model_id>[:variant]@<instance>` | 精确模型 | 某个渠道的例外价；不带 variant 时匹配该模型的所有 variant |

每个条目的值是一个 `Pricing`，`currency` 固定为 `CREDIT`，可以用 token 各维度、按单位计价（张、秒、字符、次）、`tiers`、`rules`、`addons`；`time_windows` 也可以用。token 单价在接口里按每百万 token 填写、存储时按每 token（与现有 `Pricing` 一致）。

可选的第三种来源（Q7）：`credit_rates: { "USD": 100, "CNY": 14 }`，表示“Provider 成本每 1 USD 折 100 Credit”。只用管理员显式写出的币种，不自动换汇。

**单价解析**（库存构建或调用解析时，对 model@provider 取第一个可用的）：

1. `exact:` 条目；
2. `model:` 条目，前提是该清单模型映射到了 Model Driver（`model_driver_id` / `origin_model_id` 非空）；没映射上的模型只能用 `exact:`；
3. （Q7）Provider 成本 × `credit_rates[币种]`：调用时用钉住的 Provider 价格或 Provider 上报的实际费用，币种没有汇率就跳过；
4. 无价格。

**一次 usage 的扣费**

```text
credit = completion_cost(钉住的 Credit 价格, usage) × Provider 系数 × 模型系数
```

- 准入时把 Credit 价格（第 1、2 种）或汇率（第 3 种）和两个系数钉进 `PinnedProviderTask`，和现在钉 Provider 价格的位置相同，所以价目表和策略在调用中途修改不影响这次扣费，恢复执行也能算出同一个数。
- 金额四舍五入到 1e-6 Credit，存为整数（Q9）。
- **无价格的模型不扣费**，用量照记，扣费记录标 `unpriced`；可见但无价格的模型在价目表页和 Models 页提示“N 个可见模型没有价格”。是否允许管理员把无价格模型设为不可用见 Q6。
- “有价格”的判断（用于 P6 的余额门槛）：单价解析得到非空 Credit 价格（第 3 种：钉住了 Provider 价格或 Provider 会上报费用，且币种有汇率），并且两个系数的乘积 > 0。
- Credit 价格与 Provider 成本是两回事：usage 同时保存 Provider 成本（现有 `finance_*`）和 Credit 金额，路由的 `cost_first` / `max_estimated_cost` 仍只看 Provider 成本。
- 以后可以把配置订阅 TODO 的相对价格（Model Driver `price_groups` + `relative_pricing`）接进来，让管理员只写“某价格组的 1.0 = N Credit”。那要等该 TODO 的 A5，本期不做。

### 3.4 余额与充值

- **账户**：每个用户一个 Credit 账户，余额是整数（1e-6 Credit 为单位），可以为负。没有账户行的用户余额为 0；第一次充值或扣费时建行。新用户不自动赠送（Q10）。
- **流水**：充值、调整、扣费都形成持久记录，余额 = 全部流水之和，账户行是在同一事务里维护的缓存。
- **充值** `credit.recharge { user_id, recharge_id, amount, note?, external_ref? }`
  - `amount` 是正的十进制字符串；`recharge_id` 由调用方生成，作为幂等键：同号同参只入账一次并返回原结果，同号不同参拒绝（沿用现有充值的写法）。
  - `external_ref` 是给支付网关记的自由 JSON（网关名、订单号、实付金额与币种），AICC 只保存、不解释。
  - 成功响应表示已持久入账，返回入账后的余额；超时后按 `credit.recharge.get { recharge_id }` 查原结果。
- **调整** `credit.adjust`：带符号的金额、必须写备注，用于退款、冲正（Q11）。
- **权限**：充值和调整检查 RBAC 资源 `obj://aicc/credit/recharge` 的 `write`。默认策略给 `admin` 角色；支付网关是管理员安装的 App 或服务，由管理员在 `system/rbac/policy` 的尾部给它的身份加一行授权。不再因为“Admin 名下的 App 实例”就允许充值（Q12）。
- **查询**：管理员 `credit.account.list`（全部用户及余额，含没有账户行的用户，余额 0）、`credit.account.get { user_id }`、`credit.ledger.list { user_id, 时间范围, cursor }`；普通用户只能查自己。
- **支付网关的接入方式**（AICC 之外，写进文档即可）：网关 App 负责下单、收款、把钱换成 Credit 数量；收到支付成功回调后用订单号作 `recharge_id` 调 `credit.recharge`；退款时调 `credit.adjust`。AICC 不认识货币、不做汇率。

### 3.5 准入与扣费

**准入**（每次上游尝试之前，以及推理入口的预检；两处共用一个只读函数，取代现在的 `precheck` 与 `admit`）：

1. 用 §3.2 求出有效策略：M 不可见 → `resource_not_permitted`。
2. 每条适用 limit 读对应计数器的当前窗口：`used >= amount` → `quota_exhausted`。
3. 本次“有价格”（§3.3）且余额 ≤ 0 → `credit_insufficient`。
4. 通过后把以下内容钉进 `PinnedProviderTask`（随执行记录持久化）：额度归属用户、业务 App、准入时刻（决定窗口归属）、主体快照（UserType 与组，用于用量按组记账）、Credit 价格快照与两个系数、需要更新的计数器作用域（只包括有 limit 的作用域）。

准入不写数据库。没有预算估算、没有预占，`quota::attempt_budget`、`MEDIA_INPUT_TOKENS` 之类的常量、`QuotaBudgetExceeded` 全部删除（Q1）。所有 API 类型都走同一个准入：图片、音视频也受余额和 Credit limit 约束；token limit 只累计 token 计量的调用（`is_metered` 的四种），其余计 0。

**扣费**（结算时）：

1. 先算 Provider 成本（现有 `binding.completion_cost`），再算 Credit 金额。所有以 `AttemptSettlement::Usage` 结算的位置（`finish_success`，以及 `execute` 尝试循环里启动失败带 usage 的分支）都要在结算时带上 Provider 成本。
2. 一个事务：插入扣费记录（`attempt_id` 唯一，重复结算直接返回）→ 账户余额减去 Credit 金额 → 每个钉住的计数器作用域在准入时刻所属的窗口上累加 Credit 与 token。
3. 结果未知（超时、断连）：写一条 `unknown_usage` 记录，金额 0。之后如果拿到真实用量（原生长任务恢复、迟到的回调），把这条记录改成实际金额一次，余额和计数器同步补扣。不再按预算暂估。
4. 明确失败且未消耗：不写扣费记录。明确失败但 Provider 已上报 usage：照扣（沿用现在“失败但已计费”的处理）。
5. 管理员、用户的调用都照扣，不隐式豁免；要豁免就在策略里把系数设为 0。

**计数器**：沿用 `aicc_quota_counter` 的思路，键 = 消费对象（`user` / `app` / `user_app`）+ 资源（`all` / `provider:<p>` / `model:<m>`）+ 周期 + 窗口起点，值为已用 Credit 与已用 token。只维护有 limit 引用的作用域。新建或启用一条 limit 时，从扣费记录按准入时刻重建该作用域的当前窗口（一次 `SUM`）；准入从不聚合明细。过期窗口按现有保留期清理。

**并发**：同一用户的并发调用都会通过门槛，可能一起把余额或窗口扣过头。这是 §1.3 的预期行为，写进已知风险。

### 3.6 用量记录

- `aicc_usage_event` 增加 `credit_micro`、`credit_status`（`charged` / `unpriced` / `free` / `unknown_usage`）；新表 `aicc_usage_event_subject(event_id, subject, created_at_ms)` 记录调用时额度归属用户的 `type:*` 与 `group:*`，索引 `(subject, created_at_ms)`。组成员后来变了不改历史。
- `usage.query`：`UsageQueryGroup` 加 `user_group`，`UsageQueryFilters` 加 `user_groups`，`UsageAggregate` 加 `credit_total`（十进制字符串）与 `credit_complete`（所有事件都有确定金额时为 true）。按组查询只对管理员开放。
- 失败且未上报 usage 的尝试仍不进 usage 表（与现在一致），扣费记录里也没有它。

### 3.7 错误码与拒绝详情

| `AiccErrorCode` | 含义 | 详情 |
| --- | --- | --- |
| `resource_not_permitted` | 不可见，或逻辑路由没有可见候选 | 不含隐藏资源身份（不变） |
| `quota_exhausted` | 某条适用 limit 当前窗口已用满（含 amount 为 0） | `blocking: [{ policy_id, source, subject, resource, period, unit, used, amount, reset_at_ms }]`；`reset_at_ms` 只在所有阻断 limit 都会恢复时给出 |
| `credit_insufficient`（新增） | 本次模型有价格、余额 ≤ 0 | `{ balance, provider_instance_name, exact_model }`；建议动作：联系管理员充值 |
| `quota_unavailable` | 策略、余额或计数器读不到 | 可重试，不当成不限（不变） |
| `attribution_unavailable` | 无法确定额度归属用户 | 不变 |

- 删除 `quota_budget_exceeded`。`is_policy_refusal` 加入 `credit_insufficient`。
- `credit_insufficient` 必须和上游的“Provider 余额不足 / insufficient credits”（`error/mod.rs` 里的上游错误归类）分开，文案也要区分“你的 Credit 余额”和“Provider 账户余额”。
- 只有额度归属用户本人和管理员能看到详情；其他人（群成员、许愿格协作者）只看到通用提示（沿用旧 PRD §9.6）。

### 3.8 kRPC 方法

删除 `quota.*` 与 `visibility.*` 全部方法，新增（名称可在实施时微调，改了要同步 §9 的调用方清单）：

| 方法 | 权限 | 说明 |
| --- | --- | --- |
| `policy.list { scope: mine \| manage }` | `manage` 需管理员 | `mine`：作用于本人的全部策略（管理员策略只读）和本人的 `user_app` 策略 |
| `policy.get { policy_id }` | 管理员或相关用户 | 含变更记录（非管理员裁剪） |
| `policy.upsert { draft, expected_version? }` | 按 §3.1 主体表 | 新建或修改；`expected_version` 做乐观并发 |
| `policy.delete { policy_id, expected_version }` | 同上 | 硬删除，审计保留 |
| `policy.effective { user_id?, app_id? }` | 本人；管理员可指定任意用户 | 每个可见模型的可见来源、有效 Credit 单价与来源、适用 limit 及状态、余额；也用于保存前预览（可带 `draft`） |
| `policy.status { app_id? }` | 任何能用 AICC 的会话 | 本人余额、各 limit 状态、`near_limit_count` / `exhausted_count`（取代 `quota.query`，80% 阈值不变） |
| `price.get` / `price.update { expected_revision, upserts, deletes, credit_rates? }` | `get` 所有人（只返回调用者可见模型相关的条目），`update` 管理员 | 价目表 |
| `credit.account.get { user_id? }` / `credit.account.list` / `credit.ledger.list` | §3.4 | 余额与流水 |
| `credit.recharge` / `credit.recharge.get` / `credit.adjust` | `obj://aicc/credit/recharge` write | §3.4 |

`models.list` 的每个模型增加调用者的有效 Credit 单价（或“无价格”）；管理员 `view: "manage"` 时另给价目表原价与来源。

### 3.9 存储（草案）

在 AICC 的 rdb 里（`STORAGE_SCHEMA_VERSION` 3 → 4）：

| 表 | 内容 | 取代 |
| --- | --- | --- |
| `aicc_policy` | `policy_id`、`source`、`subject`、`body_json`、`enabled`、`version`、创建 / 修改人与时间；`UNIQUE(source, subject)` | `aicc_quota_rule`、`aicc_visibility_policy` |
| `aicc_price_entry` | `entry_key`（`model:…` / `exact:…`）、`pricing_json`、`note`、修改人与时间；`credit_rates` 与修订号放 `aicc_state` | 新增 |
| `aicc_credit_account` | `user_id`、`balance_micro`、`version`、`updated_at_ms` | `aicc_quota_account` |
| `aicc_credit_entry` | 充值与调整：`entry_id`、`user_id`、`kind`、`amount_micro`、`balance_after_micro`、`recharge_id`（唯一）、`external_ref_json`、`operator`、`note`、`created_at_ms` | `aicc_quota_grant` |
| `aicc_credit_charge` | 每次上游尝试一条：`attempt_id`（唯一）、`task_id`、`usage_event_id`、`user_id`、`app_id`、`operator_id`、`provider_instance_name`、`exact_model`、`admitted_at_ms`、`tokens`、`credit_micro`、`status`、`pricing_json`（钉住的价格与系数）、`settled_at_ms` | `aicc_quota_attempt` |
| `aicc_quota_counter` | 列改为 `used_credit_micro`、`used_tokens`，去掉 `reserved` / `pending` | 原表 |
| `aicc_quota_audit` | 不变（目标类型加 `policy`、`price`、`credit`） | — |
| `aicc_usage_event` 加列、`aicc_usage_event_subject` | §3.6 | — |

扣费记录和充值流水是账本，永久保留；Usage 明细清理不影响余额。`src/kernel/node_updater/readers.toml` 中 `id = "aicc"` 一项的 `schema` 改为 `4`，`audit` 加上新文件。

## 4. 调用方与联动清单

改协议的同一变更里要一起改的地方（AGENTS.md “改协议、字段、命名、存储结构”）：

| 位置 | 改什么 |
| --- | --- |
| `src/kernel/buckyos-api/src/aicc_quota.rs` | 删旧类型与 `quota_methods`，新增策略、价目表、Credit 类型与方法名；`AICC_ON_BEHALF_OF_*`、`AiccAttribution*` 保留 |
| `src/kernel/buckyos-api/src/aicc_client.rs` | 错误码（§3.7）、`AiccClient` 的方法 |
| `src/kernel/buckyos-api/src/aicc_usage_log.rs` | §3.6 的字段 |
| `src/kernel/buckyos-api/src/rbac_config.rs` | `obj://aicc/credit/recharge`，并补测试（admin 可以、普通用户和普通 App 不可以、尾部授权后网关身份可以） |
| `src/frame/aicc/src/` | `policy/`（新）、`visibility.rs` 与 `quota/` 删除或并入、`storage/quota.rs` + `quota.sql`、`service/quota.rs`（改名为 policy 服务）、`execution/mod.rs` 的准入 / 结算 / 钉价、`service/inference.rs` 的预检、`service/management.rs` 的 usage 查询、`service/mod.rs` 的方法分发 |
| buckyos-websdk `src/aicc_client.ts`、`src/xllm/core/run.ts`、`src/xllm/providers/aicc.ts` | 客户端方法、`aiccRefusalOf` 的类别、xllm 的拒绝映射 |
| buckyos-websdk `cli/modules/ai.ts`、`cli/modules/aiws.ts` | `buckyos ai quota` 改为 `buckyos ai policy` / `buckyos ai credit`（含管理员充值，方便脚本和网关调试）；aiws 的拒绝子码 |
| `src/frame/agent_tool/src/xllm.rs`、`xllm_vectors.rs` | `RefusalKind` 去掉预算超额、加余额不足 |
| `src/frame/lib_opendan/src/runner/outcome.rs`、`baseline.rs` | Turn 失败提示：余额不足给 owner 的私聊提示，群聊仍是通用提示 |
| `src/frame/aiworkspace/server/src/wish/stages.rs`、`core/src/freshness.rs`、`aiws/README.md` | 子码 `AI_QUOTA_BUDGET_EXCEEDED` 删除，加 `AI_CREDIT_INSUFFICIENT` |
| `src/tools/buckyos-agent/`（`lib/result.ts`、`lib/model.ts`、`commands/ai_quota.ts`、`aicc-tool.ts`、`readme.md`） | 同上；`ai_quota` 改为输出余额与 limit 状态 |
| Desktop `src/frame/desktop/src/`：`api/aicc_quota.ts`、`app/ai-center/{QuotaPage,VisibilityPage,HomePage,ModelsPage,PlaygroundPage}.tsx`、`components/quota/*`、`components/home/QuotaHomeCard.tsx`、`components/models/UnexposedModelsNotice.tsx`、`components/providers/wizard/ProviderAccessSection.tsx`、`datamodel/quota.ts`、`hooks/use-quota-api.ts`、`mock/*`、`i18n/ai-center-quota.ts`、`tests/datamodel/aicc-quota.test.ts`；AIWorkspace `app/aiworkspace/api/session.ts`、`executor/infer.ts` | §6 |
| cmd_skills `xllm`（`references/runs.md`）、`aigc_image`、`aigc_audio`（`references/audio.md`）、`aigc_video`、`buckyos_admin` | 错误码与建议动作；`buckyos_admin` 加策略、价目表、充值的管理说明 |
| 文档 | §8 |

websdk 要先推，buckyos 的构建才会用到新的客户端。

## 5. 不做

- 全组共用的 limit 池（Q3）、自然月等新周期、滚动窗口。
- 有效期的临时额度包、赠送规则、自动续费。
- AICC 内的支付、货币、汇率换算（只有 Q7 的管理员显式汇率）。
- Session 级 limit（Agent 已能用 `user_app` 策略单独限制）。
- 相对价格（等配置订阅 TODO 的 A5）。
- 预占和预算估算（Q1）。

## 6. 界面（Desktop › AI Center）

- **策略**页取代“额度”“可见性”两页：主体列表（UserType 组在前，系统用户组，用户，App）→ 主体详情：全部 Provider 开关、每个 Provider 的可见性 / 系数 / limit、展开后每个模型的可见性 / 系数 / limit、顶层 limit。按具体用户（可加 App）预览有效结果，每一项标出来自哪条策略（`policy.effective`）；保存前预览新增和移除的资源，管理员改到自己时提示“你自己将无法使用以下资源”（沿用）。普通用户只看到“我的 App”：给自己的 App / Agent 加收紧和 limit，管理员策略只读。
- **价目表**页（管理员）：按 Model Driver 列出模型身份，填 Credit 单价（每百万 token 或按单位）；精确模型例外；（Q7）币种汇率；“N 个可见模型没有价格”的提示和筛选。
- **Credit**页：管理员看全部用户余额（可按组筛选）、充值对话框（金额、备注、单号自动生成、预览入账后余额）、调整、流水；普通用户看自己的余额与流水。
- **Usage**：增加 Credit 列和“按用户组”分组；统计行的“设置额度”改为“设置策略”，预填主体与资源。
- **Models / Playground**：显示调用者的有效 Credit 单价或“无价格”；余额不足、limit 用满的标记与拒绝说明。
- **Home**：余额与接近上限 / 已用满的提示，和 Provider 上游配额预警分开展示。
- **Add Provider 向导**：“开放范围”一步改为写策略；另显示“这个 Provider 的模型中 N 个已有价格、M 个没有价格”。
- 桌面与移动端、浅色与深色、Desktop 现有全部语言（`i18n/ai-center-quota.ts` 的现有格式）。

## 7. 已知风险

| 风险 | 后果 | 缓解 |
| --- | --- | --- |
| 不预占（Q1） | 余额很少时的并发或大调用会把余额、窗口扣成负数或超出 | 这是 P9 的预期；管理员用 limit 和系数控制 |
| 无价格模型不扣费（Q6） | 新加的 Provider 若映射不到已配价的模型，在补价前对余额为 0 的用户也可用 | 价目表先于 Provider 配置（P8）；“无价格”提示；必要时用可见性收紧 |
| 结果未知不暂估 | 永远拿不到 usage 的调用不扣费 | 记 `unknown_usage`，管理员可在流水里看到并用 `credit.adjust` 补扣 |
| 执行中进程崩溃 | 准入不写库，若恢复流程也没走到结算，这次尝试没有扣费记录 | 实施时确认执行记录恢复后会调用一次结算；做不到的写进实施报告 |
| 按层覆盖改变可见性语义（Q2） | 某组策略里写了 `hidden` 会盖掉 `type:user` 的全部可见 | 预览显示每一项的来源；默认策略只在 type 层 |
| 管理员也被扣费 | 配了价格后，余额为 0 的管理员自己也用不了有价格的模型 | 首次保存价目表时提示，并提供“为 `type:admin` 设系数 0”的一键操作 |
| Credit 与 SN Credit、Provider credits 同名 | 用户混淆 | 文案统一为“AI Credit / AI 点数”，错误码与上游余额错误分开 |

## 8. 文档同步

- 新写 `doc/aicc/aicc_user_policy.md`（实现说明：策略、求值、价目表、准入与扣费、存储、方法），删除 `doc/aicc/aicc_token_quota.md`。
- `product/ai_center/AI_Center_Quota_PRD.md` 顶部注明被本单取代的章节（§2.2），或由 owner 决定改写为 v2。
- `product/ai_center/AI_Center_PRD.md`、`AI_Center_Frontend_Prototype.md` 中额度、可见性的入口与页面。
- `doc/aicc/provider_pricing_sources.md`：加一节说明 Credit 价目表与 Provider 价格是两套，Provider 价格规则不变。
- `doc/aicc/aicc_agent_cli_tools.md`、`doc/llm_context/xllm_ts_runtime.md`、`local_llm_context_protocol.md`、`LLM Context 设计.md`：错误类别。
- `doc/aicc/README.md` 的文档索引；`product/users_and_agents/UserType.md` 若提到 AICC 可见性则同步。
- 支付网关接入说明（§3.4 最后一条）放进 `aicc_user_policy.md`。

## 9. 实施步骤

**阶段 A：不依赖决策，可以先做**

- [ ] A1 价目表与 Credit 算价（纯函数）：`aicc_price_entry` 的类型与校验（`currency` 必须是 `CREDIT`、模型身份格式、`exact:` 属于存在的实例时校验 variant）；单价解析第 1、2、4 种；用 `PinnedPricingSnapshot::from_pricing` 钉价、`completion_cost` 算额、乘系数、取整到 1e-6。单测覆盖：模型身份价对两个 Provider 实例都生效；`exact:` 覆盖模型身份价；未映射到 Model Driver 的模型只能用 `exact:`；按张 / 按秒 / `tiers` / `rules`；系数 0；取整。
- [ ] A2 Credit 账户与流水：`aicc_credit_account`、`aicc_credit_entry`，充值幂等（同号同参、同号不同参、并发两笔）、余额可负、`credit.recharge.get`。单测覆盖旧 PRD Q30、Q35 的同类场景。
- [ ] A3 用量记账：`aicc_usage_event` 加列、`aicc_usage_event_subject`、`usage.query` 的 `user_group` 分组与 `credit_total`。
- [ ] A4 让结算拿得到 Provider 成本：`AttemptSettlement::Usage` 增加 `Option<AiCost>`，`finish_success` 改成先算费用再结算，尝试循环里启动失败带 usage 的分支用同一个 `binding.completion_cost` 算。现有额度行为不变，为 B4 铺路；跑现有执行引擎测试确认无回归。

**阶段 B：依赖 Q1–Q6、Q13**

- [ ] B1 共享类型：`aicc_quota.rs` 改写（策略、limit、有效策略视图、拒绝详情）、`AiccErrorCode` 增删、`aicc_usage_log.rs`。
- [ ] B2 `policy/` 求值模块（§3.2），单测至少覆盖 §11 的 C01–C12。
- [ ] B3 存储 v4：`aicc_policy`、`aicc_credit_charge`、计数器改列、默认策略播种；删除旧额度表与可见性表；升级方式按 Q14。
- [ ] B4 准入与扣费（§3.5）：合并 `precheck` / `admit` 为只读准入；钉进 `PinnedProviderTask`；结算事务；`unknown_usage` 的补扣；计数器重建；删除 `attempt_budget`、预占、核对任务中的暂估部分（周期切换相关的 `pending_period` 一并删除，limit 修改立即生效）。
- [ ] B5 kRPC（§3.8）与权限、RBAC 资源；`models.list` 的价格字段。
- [ ] B6 依赖 Q7：单价解析第 3 种与 `credit_rates`。
- [ ] B7 依赖 Q11、Q12：`credit.adjust`、充值权限从“管理员账户”改成 RBAC 检查。

**阶段 C：调用方与界面（B1、B5 稳定后并行）**

- [ ] C1 websdk 客户端、CLI、xllm TS；先推 websdk。
- [ ] C2 Rust 侧调用方：`agent_tool` xllm、`lib_opendan`、`aiworkspace`、`buckyos-agent`。
- [ ] C3 Desktop 界面（§6）与 e2e / datamodel 测试、mock。
- [ ] C4 cmd_skills（§4 表）与文档（§8）、`readers.toml`。

**阶段 D：验证**

- [ ] D1 `cargo test -p aicc -- --test-threads=1`、`cargo test -p buckyos-api -- --test-threads=1`、`agent_tool` / `libopendan` / `aiworkspace` 相关用例；websdk 测试；Desktop `pnpm check`、`pnpm test:e2e` 的 AI Center 部分。基线就失败的用例在报告里单列。
- [ ] D2 本机 Zone DV（部署新 AICC 要 root 重启，需要用户配合）：见 §11 “DV”。
- [ ] D3 实施报告：落点、偏差、未验证项、净行数构成（生产 / 测试 / 文档分开统计；预期额度部分的生产代码减少，因为预占、核对、总量账户被删除）。

## 10. 待 owner 定的问题

每题给出我的建议，括号里是理由。

1. **调用前只看门槛、不预占。** 建议是：删除预算估算、预占、待核对暂估和 `quota_budget_exceeded`，并发与大调用可以扣成负数。（原话“余额可以是负数（一次 usage 太大）”只有在不预占时才会发生；同时删掉一大块代码。）另一个做法是保留预占，只把单位改成 Credit。
2. **策略叠加按层覆盖。** 建议用户侧 type < group < user，更具体的一层设了的字段以它为准；同一层多个组取对用户最有利的值（可见优先、系数取小、同键 limit 取大）；App 侧只能收紧。（管理员给某个用户配“某 Provider 不可见”时，期望它生效；现在的并集语义做不到，旧 PRD §15 也把“不能对个人拒绝”列为风险。）另一个做法是保持可见性并集、只有系数和 limit 按层覆盖。组内冲突也可以改成“不利优先”。
3. **组上的 limit 对每个成员分别计算。** 建议是，本期不做全组共用的 limit 池；用户组的用量只用于统计（P5）。（组在这里表示“一类用户”；共用池需要另一种计数器和界面。）
4. **App 侧策略能否带价格系数。** 建议本期不能，系数只在用户侧。（余额属于用户，价格是“这个用户付多少”；给 App 打折可以以后再加。）
5. **limit 的单位。** 建议 `credit` 与 `token` 都支持，界面默认 `credit`。（无价格的模型只能用 token 限制；现有的 token 周期规则也能原样表达。）
6. **无价格的模型。** 建议照常可用、不扣费、记 `unpriced`，并在界面提示。另一个做法是在策略里加一个“无价格模型不可用”开关。（前者没有新机制，也保证开箱“随便用”；后者能堵住 §7 第二条风险。）
7. **要不要“Provider 成本 × 汇率”这种价格来源。** 建议要，管理员按币种写 `credit_rates`，只用于价目表里没有条目的模型。（OpenRouter、SN 等会上报实际费用，管理员不必逐个模型配价；汇率是管理员显式写的，不算估算。）不要的话价格只来自价目表。
8. **价目表的模型键。** 建议 `<model_driver_id>:<origin_model_id>`，同一模型的各 variant 同价，例外用 `exact:`。（这是 AICC 里与 Provider、协议客户端都无关的模型身份，Model Driver 目录在加 Provider 之前就有。）
9. **Credit 的精度与表示。** 建议存储用 1e-6 Credit 的整数，接口用十进制字符串，界面显示两位小数（不足 0.01 显示“< 0.01”）。Credit 是 Zone 内的单位，不对应任何货币。
10. **新用户的初始余额。** 建议为 0、不自动赠送；需要赠送时由管理员或网关调充值。
11. **退款与冲正。** 建议提供 `credit.adjust`（带符号、必须写备注、同一权限）。（接真实支付网关就会有退款。）
12. **充值权限。** 建议改为 RBAC 资源 `obj://aicc/credit/recharge`：admin 角色默认有，网关身份由管理员在 RBAC 策略尾部授权；不再因为“Admin 名下的 App 实例”就能充值。（现在任何 Admin 安装的 App 都会被识别成管理员。）是否要给“授权网关”做界面，建议本期不做，写进 `buckyos_admin` 手册。
13. **用户自己的 App 策略能写什么。** 建议只能写可见性收紧和 limit，不能写系数、不能放宽。
14. **存储从 3 升到 4。** 现在版本不符时 AICC 拒绝启动，所有已部署 Zone（含 demo）都会受影响。建议加一个一次性的 3 → 4 步骤：删除旧额度表与可见性表、usage 表加列、建新表、写默认策略；旧额度规则和 token 账户不迁移。另一个做法是要求清空 AICC 的 rdb，但那会丢掉全部用量历史。
15. **管理员的默认系数。** 建议 `type:admin` 默认不设系数（即 1.0，和普通用户一样扣），首次保存价目表时提示并提供一键设 0。（沿用旧 PRD “管理员不隐式豁免”。）

## 11. 验收

**离线（cargo test 与前端测试）**

| 编号 | 场景 | 预期 |
| --- | --- | --- |
| C01 | 开箱 Zone，价目表为空，管理员加第一个 Provider | Admin 与 User 全部可见、不扣费、无 limit；Limited 不可见；用量按用户、App、组记录 |
| C02 | `type:user` 全部可见，`group:students` 把 `openrouter` 设为 `hidden` | 学生看不到 openrouter，其他用户可见；预览标出来源是 `group:students`（Q2） |
| C03 | 用户属于两个组，一个设 openrouter 系数 1.2、一个设 0.8 | 有效系数 0.8；`user:` 层设 1.0 时为 1.0 |
| C04 | 管理员给 `user:bob` 设某模型不可见，`type:user` 全部可见 | bob 看不到也调不了，逻辑路由不选它，错误不泄露它 |
| C05 | App 侧：`app:writer` 只允许 `openai-main`，用户侧全部可见 | 经 writer 调用只能用 openai-main；直接调用不受影响 |
| C06 | 用户给自己的 Agent 加 `user_app` 策略：4h 10 Credit | 只约束该 Agent；用户不能给别人的 App 或系统里不属于他的 App 建策略；不能写系数 |
| C07 | 同一用户同时有 `type:user` 每日 100 Credit、`user:` 每日 500 Credit、`user:` 每周 1000 Credit | 每日按 500、每周 1000 同时生效 |
| C08 | `app:writer` 每日 1000 Credit 被多个用户共用 | 各用户的消耗累计到同一窗口，用满后所有用户经 writer 都被拒 |
| C09 | 余额 0.5 Credit，一次调用实际花 3 Credit | 准入通过，结算后余额 −2.5；下一次有价格的调用返回 `credit_insufficient`；免费模型（系数 0 或无价格）仍可用 |
| C10 | 窗口 limit 10 Credit 已用 9.9，一次调用花 2 | 通过，窗口用量 11.9，之后同窗口被拒，`reset_at_ms` 为窗口结束；limit 为 0 时不给恢复时间 |
| C11 | 价目表只配了模型身份价，先配价后加两个映射到它的 Provider | 两个 Provider 上的该模型都有价格；`exact:` 例外只影响一个；Provider 系数分别生效 |
| C12 | 按张计价的图片、按秒计价的视频 | 按 usage 扣 Credit；token limit 不计它们，Credit limit 计 |
| C13 | 调用中途修改价目表和策略系数 | 本次按准入时钉住的价格与系数扣；新调用用新值 |
| C14 | 同一尝试重复结算、服务重启后恢复结算 | 只扣一次；余额与计数器一致 |
| C15 | 结果未知，之后拿到真实 usage | 先记 `unknown_usage` 0，之后补扣一次；拿不到就保持 0 |
| C16 | 新建一条 limit 时本窗口已有消耗 | 计数器从扣费记录重建，不从零开始 |
| C17 | 充值：同号重试、同号不同参、两笔并发、超时后按号查询 | 只入账一次 / 拒绝 / 累加 / 返回原结果 |
| C18 | 普通用户调充值；管理员装的普通 App 调充值；尾部授权后的网关身份调充值 | 拒绝 / 拒绝（Q12）/ 成功 |
| C19 | 策略或余额读失败 | `quota_unavailable`，不放行 |
| C20 | 各类拒绝 | 错误码各自独立；SDK / CLI / xllm / OpenDAN / AIWorkspace 只按错误码分类；`credit_insufficient` 与上游余额不足分开 |
| C21 | 群聊里别人让 Agent 干活导致 owner 余额不足 | 消耗计入 owner；群成员只看到通用提示；owner 私聊收到余额不足提示 |
| C22 | 许愿格 owner 模式由协作者运行 | 扣 owner 的余额；协作者看不到 owner 的余额与策略 |
| C23 | `usage.query` 按 `user_group` 分组 | 用调用时的组快照，组成员后来变化不改历史；非管理员不能按组查 |
| C24 | 存储 3 → 4（按 Q14） | 旧库能启动，用量历史保留，默认策略存在 |

**DV（本机 Zone，需要 root 重启 AICC，要用户配合）**

- 用便宜的真实模型走一遍：配价目表（模型身份价）→ 加 Provider → 普通用户余额 0 被拒 → 管理员充值 → 调用成功、余额按 usage 扣减、Usage 页 Credit 与 Provider 成本都对得上。
- 一次大输出调用把余额扣成负数，之后被拒；免费模型仍可用。
- 组策略：建一个系统用户组，给它隐藏一个 Provider 并设每日 Credit limit，用组内和组外两个用户分别验证。
- 网关模拟：用一个脚本身份，在 RBAC 尾部授权后调 `credit.recharge`（带 `external_ref`），重复调用只入账一次；未授权时被拒。
- Desktop 策略、价目表、Credit 三页在桌面与手机宽度、深浅色下各走一遍主要操作。

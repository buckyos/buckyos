# AICC 用户策略、价目表与 Credit：实现说明

- 需求与裁决：[AICC 用户策略与 Credit 计费 TODO](../../notepads/aicc-user-policy-credit-todo.md)（owner 2026-11-27 定案）。它取代 [AI Center Quota PRD](../../product/ai_center/AI_Center_Quota_PRD.md) 的额度模型与可见性存放方式；该 PRD 中调用身份、许愿格归属、隐藏资源不泄露、只按错误码判断的部分仍然有效。
- 共享类型与方法名：`src/kernel/buckyos-api/src/aicc_quota.rs`（`policy_methods`）；错误码：`aicc_client.rs` 的 `AiccErrorCode`；用量：`aicc_usage_log.rs`。
- 服务实现：`src/frame/aicc/src/`：求值 `policy/mod.rs`、价目表与算价 `policy/pricing.rs`（纯函数）；存储 `storage/policy.rs` + `storage/policy.sql`；方法、准入与扣费 `service/policy.rs`；执行引擎 `execution/mod.rs`（`ChargePlan` 钉在 `PinnedProviderTask.charge`）。
- 本文记录当前实现；行为以代码和测试为准。

## 1. 调用身份与归属

`RuntimeAuthorizer`（`service/identity.rs`）把会话解析成 `AuthorizedCaller`，规则与 Quota PRD §3 相同：

| 会话主体 | 额度归属用户 | 业务 App | 说明 |
| --- | --- | --- | --- |
| User（含 Limited） | 会话 `sub` | 会话 `app` | 读取账户的 UserType 与系统用户组；非 Active 与 `root` 账户拒绝；External 会话没有本 Zone 的 AI 资源 |
| App 实例 | 实例 owner | AppInstanceId 的 app_id | |
| Agent | AgentSpec 的 owner（缓存） | AgentId | Agent 的消耗扣其 owner |
| System / Device | 自身 | 服务名 | 不受策略约束、不扣费，只记用量 |
| System + `on_behalf_of` | 被代表的用户 | 服务名（目前只有 `aiworkspace`） | 只有 RBAC 允许 `obj://aicc/on_behalf_of` write 的主体可以代表 |

`ExecutionRequest` / `ExecutionRecord` 带 `principal: Option<PolicyPrincipal>`（用户、UserType、调用时的组），System 主体为 `None`。用量的 `operator_id` / `attribution` 不变。

## 2. 策略与求值

一条策略 = 来源（`admin` / `user`）+ 主体 + `enabled` + `body`，同一 `(source, subject)` 只有一条（`aicc_policy`，`UNIQUE(source, subject)`），`version` 做乐观并发。

| 主体 | 写法 | 谁能配 |
| --- | --- | --- |
| UserType 组 | `type:admin` / `type:user` / `type:limited`（Root 算 admin） | 管理员 |
| 系统用户组 | `group:<id>`（必须是已定义的组） | 管理员 |
| 用户 | `user:<id>` | 管理员 |
| App | `app:<app_id>` | 管理员 |
| 用户 + App | `user_app:<user>/<app>` | 管理员；用户本人（只能是自己的 App / Agent / aiworkspace / 用量里出现过的 App，`source = user`） |

`body`（`PolicyBody`，未写的字段表示这一层没有意见）：

```json
{
  "all_providers": true,
  "price_coefficient": 1,
  "limits": [{ "period": "24h", "unit": "credit", "amount": "20" }],
  "providers": {
    "openrouter": { "visible": "selected_models", "price_coefficient": 1.2,
                    "limits": [{ "period": "7d", "unit": "credit", "amount": "50" }] },
    "local-ollama": { "visible": "all_models", "price_coefficient": 0 }
  },
  "models": { "gpt-6.1-sol@openrouter": { "visible": true, "price_coefficient": 0.5,
              "limits": [{ "period": "5h", "unit": "token", "amount": "200000" }] } }
}
```

- `providers.<p>.visible`：`all_models` / `selected_models`（只有 `models` 里 `visible: true` 的）/ `hidden`；`models.<m>.visible`：`true` / `false`；`all_providers: true` 跟随清单（`false` 视为没有意见）。
- `price_coefficient`：非负；Provider 下的覆盖顶层；模型系数与 Provider 系数相乘；`0` 即免费。App 侧策略不能写系数。
- `limits`：`period` 为 `5h` / `24h` / `7d`，`unit` 为 `credit`（最多 6 位小数）或 `token`（整数），`amount` 为十进制字符串，`0` 禁止使用；同一位置同一 `(period, unit)` 只能有一条。
- 用户自己的 `user_app` 策略只能收紧：不能写 `all_providers`、`all_models` 和系数，`visible: true` 只能配合同一 Provider 的 `selected_models`；不能提及自己看不到的资源。
- 策略可以引用清单里还没有的 Provider / 模型；`policy.list` 对管理员在 `unavailable_resources` 里标出。

**求值**（`policy::Evaluator`，输入：主体、业务 App、`provider` + `exact_model`）：

1. 用户侧三层 `type:*` < 用户所在的 `group:*` < `user:*`，只看 `enabled` 的策略。每个字段取设了它的最高一层；组这一层多条同时设了，取对用户有利的值（可见优先、系数取最小、同键 limit 取最大）。
2. 可见性：先看模型的 `visible`，再看 Provider 的 `visible`，再看 `all_providers`；全部层都没意见 → 不可见。
3. 系数：Provider 系数和模型系数各自按第 1 条求值，都没有意见为 1.0。
4. limit：以 `(资源, period, unit)` 为键按第 1 条覆盖，不同键全部生效。资源为顶层（`all`）、Provider 下或模型下。用户侧全部计在用户自己的计数器上（组上的 limit 是每个成员各一份）。
5. App 侧依次叠加 `app:A`、管理员的 `user_app:U/A`、用户自己的 `user_app:U/A`：任何一条说不可见就不可见；limit 全部追加（`app:A` 用 App 全局计数器，`user_app` 用该用户该 App 的计数器）。
6. 与 Provider 实际清单取交集（停用的 Provider 没有清单）。

`Access`（`policy::Access`）是一个调用者的结果：可见的 Provider → 模型，以及其中未定价（§3）的模型。`models.list`、`provider.list`、usage / trace 的隐藏处理、路由预览看“可见”；逻辑路由的候选、直接调用看“可用”（可见且不是未定价）。

**默认策略**：首次建库（含 3 → 4 升级）写入 `type:admin`、`type:user` 各一条 `{ "all_providers": true, "price_coefficient": 0 }`（`aicc_state.policy_seeded`），之后是普通策略，系统不自动恢复。开箱全部可见、显式免费、没有 limit；`type:limited` 没有策略。

## 3. 价目表与算价

价目表（`aicc_price_entry` + `aicc_price_rate`，修订号 `aicc_state.price_revision`）只有管理员能改，与策略分开：

| 键 | 含义 |
| --- | --- |
| `model:<model_driver_id>:<origin_model_id>` | 与 Provider 无关的模型身份，所有映射到它的 Provider 都用这个价；可以先于 Provider 配置 |
| `exact:<provider_model_id>[:variant]@<instance>` | 某个渠道的例外价；不带 variant 时覆盖该模型的所有 variant；实例已存在时校验模型与 variant 存在 |

条目值是一个 Provider 价格形状的 `Pricing`（`currency` 固定 `CREDIT`，可用 token 各维度、`tiers`、按单位 `unit` + `amount` + `rules` + `addons`、`time_windows`）。**接口里 token 单价按每百万 token**，存储按每 token（`pricing::scale_token_rates`）。`credit_rates` 是币种 → 每单位 Provider 成本折多少 Credit（例如 `{"USD": 100}`），只用管理员写出的币种。

**单价解析**（`PriceList::unit_price`，第一个可用的）：`exact:`（先带 variant 再不带）→ `model:`（要求该清单模型映射到了 Model Driver）→ Provider 成本 × `credit_rates[币种]`（列表时看清单价格的币种或 Provider rules 的 `reported_cost` 币种；调用时看钉住的 Provider 价格或上报费用）→ 无价格。

**状态**（`PriceList::state`）：系数乘积为 0 → `free`；条目价格全为 0、或汇率为 0、或 Provider 价格全为 0 → `free`；有价 → `priced`；否则 `unpriced`。**没有价格不等于免费**：`unpriced` 的模型对这个调用者不可用，`models.list` 照常列出（`credit_price.state = "unpriced"`），逻辑路由不选它，直接调用返回 `resource_not_permitted`，`details.reason = "unpriced"`。

**一次 usage 的扣费**（`PinnedCreditPrice::charge`）：`credit = 价格(usage) × Provider 系数 × 模型系数`，四舍五入到 1e-6 Credit，存为整数（micro-Credit）。价格按 Credit 条目时用 `PinnedPricingSnapshot::completion_cost`（与 Provider 成本同一套代码）；按汇率时用这次的 Provider 成本（上报费用优先），币种必须与汇率的币种一致。算不出金额（usage 缺价格需要的维度，或按汇率时 Provider 没给出成本）记 `unpriced`、金额 0。Credit 与 Provider 成本是两套数：usage 两个都存，路由的成本评分、`max_estimated_cost` 和 `not_pricier` 的价格比较只看 Provider 成本。

## 4. 准入与扣费

**准入**（`PolicyService::evaluate_call`，只读，不写数据库）在推理入口对选中的模型做一次（`precheck`；被拒绝且用途配置了“不可用时”策略时，对算出的备选再做一次），并在每次上游尝试前再做一次（`AdmissionPort::admit`）：

1. 不可见 → `resource_not_permitted`（不带资源身份）。
2. 钉价（`pin_price`）：Credit 条目在这时用请求参数匹配 `rules`（`call.pricing_context`）；系数乘积不为 0 而没有价格 → `resource_not_permitted`（`reason: "unpriced"`）。
3. 每条适用的 limit 读计数器当前窗口（没有窗口或窗口已到期为 0），`used >= amount` → `quota_exhausted`。token limit 只约束 token 计量的调用（`llm`、`decision`、两种 embedding），其余调用只受 Credit limit 和余额约束。
4. 本次可能付费（`needs_payment`：系数乘积 > 0 且价格是 Credit 条目或汇率 > 0）且余额 ≤ 0 → `credit_insufficient`。
5. 通过后返回 `ChargePlan`：尝试 id、任务、用量事件 id、归属用户、业务 App、operator、准入时刻、当时的 `type:*` / `group:*`、钉住的价格与两个系数、要累加的计数器作用域（只包括有 limit 引用的）。它随 `PinnedProviderTask.charge` 持久化，所以调用中途改价目表或策略不影响这次扣费，恢复执行也算出同一个数。

没有预算估算、没有预占：余额很少时的并发调用或一次大调用可以把余额扣成负数、把窗口扣过头（owner 裁决）。

**扣费**（`AdmissionPort::settle` → `AiccStorage::settle_charge`，一个事务）：先锁（或建）用户的账户行，然后：

- 没有这次尝试的扣费记录 → 插入 `aicc_credit_charge`（`attempt_id` 唯一），余额减去金额，按准入时刻把 Credit 与 token 加到每个钉住的计数器上。
- 已有记录且状态是 `unknown_usage`、这次带来了真实 usage → 改成实际金额一次，余额和计数器同步补扣。
- 其余情况（重复结算、服务重启后再结算）不变。

结算的来源：成功（`finish_success`，先算 Provider 成本再结算）、启动失败但上报了 usage、结果未知（超时、断连、恢复时找不到结果：`unknown_usage`，金额 0）。明确失败且未消耗（Provider 没接受）不写记录。管理员与用户都照扣，要豁免就在策略里把系数设为 0。

**窗口**（从首次使用起算）：每个“计数器 + 资源 + 周期”一行（`aicc_quota_counter`）。没有活动窗口时，下一次扣费的准入时刻就是新窗口起点，长 5h / 24h / 7d；到期后等下一次使用。结算时：准入时刻落在这一行的窗口内（即使此刻已到期）→ 累加；早于窗口起点（上个窗口里准入、结算前新窗口已开）→ 不计入任何窗口，余额照扣；晚于窗口结束或还没有这一行 → 以准入时刻开新窗口。新建 limit 时同作用域同周期已有窗口就沿用其用量，没有就从下一次使用开始计，不追溯。

## 5. Credit 账户

- 每个用户一个余额（`aicc_credit_account.balance_micro`，可以为负）；没有行的用户余额为 0，新用户不赠送。
- 充值与调整是 `aicc_credit_entry`（`request_id` 唯一）；扣费是 `aicc_credit_charge`。余额行与它们在同一事务里维护。账本永久保留，Usage 明细清理不影响余额。
- `credit.recharge { user_id, recharge_id, amount, note?, external_ref? }`：`amount` 为正的十进制字符串；`recharge_id` 是幂等键，同号同参（含 note、external_ref）返回原记录（`replayed: true`），同号不同参返回 `idempotency_conflict`；成功即已持久入账。超时后用 `credit.recharge.get { recharge_id }` 查。
- `credit.adjust { user_id, adjust_id, amount, note }`：带符号、不为 0、必须写备注，用于退款、冲正、补扣 `unpriced` / `unknown_usage`。
- 权限：充值、查询充值、调整检查 RBAC `obj://aicc/credit/recharge` 的 `write`。默认策略只给 `admin` 角色；“Admin 名下的 App”不因此获得权限。操作人记为用户，经 App 时为 `<user>@<app>`。

**支付网关接入**（AICC 之外）：网关是管理员安装的 App 或服务，负责下单、收款、把钱换算成 Credit 数量。管理员在 `system/rbac/policy` 尾部给它加一行 `p, app:<gateway_app_id>,obj://aicc/credit/recharge,write,allow`（系统服务写 `system:<service>`）。收到支付成功回调后，用订单号作 `recharge_id` 调 `credit.recharge`，`external_ref` 记网关名、订单号、实付金额与币种（AICC 只保存）；网络失败就用同一个 `recharge_id` 重试或先 `credit.recharge.get`；退款时调 `credit.adjust`（负数）。AICC 不认识货币、不做汇率。

## 6. 错误码

| `AiccErrorCode` | 含义 | `details` |
| --- | --- | --- |
| `resource_not_permitted` | 不可见；或逻辑路由没有可用候选；或模型没有价格而调用者要付费 | 不可见时没有；未定价时 `ResourceUnpricedDetails { reason: "unpriced", provider_instance_name, exact_model }` |
| `quota_exhausted` | 某条适用 limit 当前窗口已用满（含 amount 为 0） | `QuotaDenialDetails { provider_instance_name, exact_model, blocking: [{ policy_id, source, subject, resource, period, unit, used, amount, reset_at_ms? }], reset_at_ms }`；`reset_at_ms` 只在所有阻断 limit 都会自己恢复（amount 不为 0、有窗口）时给出 |
| `credit_insufficient` | 本次可能付费且余额 ≤ 0；与 Provider 账户的“余额不足”无关 | `CreditInsufficientDetails { balance, provider_instance_name, exact_model }` |
| `quota_unavailable` | 策略、价目表、余额或计数器读不到；可重试，不当成不限 | `QuotaUnavailableDetails { scope, occurred_at_ms }` |
| `attribution_unavailable` | 无法确定额度归属用户 | 不变 |

这些码（`is_policy_refusal`）走 kRPC 的结构化错误，客户端只按 `code` 分类。只有额度归属用户本人和管理员看到详情；群成员、许愿格协作者只看到通用提示（OpenDAN、AIWorkspace 的文案层负责）。

## 7. kRPC 方法

方法名见 `policy_methods`，请求 / 响应类型同名于 `aicc_quota.rs`：

| 方法 | 参数 | 权限 |
| --- | --- | --- |
| `policy.list` | `scope: mine \| manage` | `mine`：作用于本人的策略（管理员策略只读）与本人的 `user_app` 策略；`manage` 需管理员。非管理员看不到策略里自己看不到的 Provider / 模型 |
| `policy.get` | `policy_id` | 管理员或相关用户；带变更记录（非管理员只有自己的策略有） |
| `policy.upsert` | `draft { source?, subject, enabled?, body }`, `expected_version?` | 按 §2 主体表；新建不带版本，修改必须带当前版本（不符返回 `settings_revision_conflict`） |
| `policy.delete` | `policy_id, expected_version` | 同上；硬删除，审计保留 |
| `policy.effective` | `user_id?, app_id?, draft?` | 本人；管理员可指定任意用户。每个可见模型的可见来源、系数来源、Credit 价格与状态、适用 limit 与窗口、余额；带 `draft` 时另给 `added` / `removed`（可用模型的增减）、`affected_users`、`caller_loses` |
| `policy.status` | `app_id?` | 任何能用 AICC 的会话（含受限会话）：余额、limit 状态、`near_limit_count` / `exhausted_count`（80%） |
| `price.get` | — | 所有人；非管理员只看到与自己可见模型有关的条目，管理员另有 `credit_rates` 与 `unpriced_models` |
| `price.update` | `expected_revision, upserts, deletes, credit_rates?` | 管理员（设置写权限） |
| `credit.account.get` | `user_id?` | 本人；管理员可指定 |
| `credit.account.list` | `group?` | 管理员；全部账户（含没有账户行的，余额 0） |
| `credit.ledger.list` | `user_id?, start_time_ms?, end_time_ms?, cursor?, limit?` | 本人；管理员可指定。充值、调整、扣费按时间倒序 |
| `credit.recharge` / `credit.recharge.get` / `credit.adjust` | 见 §5 | `obj://aicc/credit/recharge` write |

`models.list` 的每个模型带调用者的 `credit_price`（`ModelCreditPrice`：状态、来源、条目、Credit 价格、汇率、两个系数）；管理员 `view: "manage"` 时每个模型另有 `accessible`（可见）与 `usable`（可用）。

## 8. 存储

`STORAGE_SCHEMA_VERSION = 4`（`storage/mod.rs`）。新库建全套表；版本 3 的库在打开时做一次性升级：删除 `aicc_quota_rule`、`aicc_quota_counter`、`aicc_quota_account`、`aicc_quota_grant`、`aicc_quota_attempt`、`aicc_visibility_policy` 和可见性的 `aicc_state` 键，`aicc_usage_event` 加 `credit_micro`、`credit_status`，建 `policy.sql` 的新表，再写默认策略。旧额度规则和 token 账户不迁移，用量历史保留。其它版本仍拒绝启动。

| 表 | 内容 |
| --- | --- |
| `aicc_policy` | 策略；`UNIQUE(source, subject)` |
| `aicc_price_entry` / `aicc_price_rate` | 价目表条目（每 token 存储）/ Credit 汇率 |
| `aicc_credit_account` | 余额缓存（micro-Credit） |
| `aicc_credit_entry` | 充值与调整；`request_id` 唯一 |
| `aicc_credit_charge` | 每次上游尝试一条；`attempt_id` 唯一；`pricing_json` 存钉住的价格与系数 |
| `aicc_quota_counter` | 每个作用域 + 周期一行：窗口起止、已用 Credit 与 token |
| `aicc_quota_audit` | 策略、价目表、Credit 的变更审计 |
| `aicc_usage_event_subject` | 用量事件在调用时的 `type:*` / `group:*`，`usage.query` 的 `user_group` 分组与 `user_groups` 过滤用它（只对管理员开放）；组成员后来变化不改历史 |

`aicc_usage_event` 的 `credit_micro` / `credit_status`（`charged` / `free` / `unpriced` / `unknown_usage`）是该任务最终那次尝试的扣费；`UsageAggregate` 增加 `credit_total` 与 `credit_complete`（有 `unpriced` 或 `unknown_usage` 时为 false）。

## 9. 调用方

- websdk：`AiccClient` 的 policy / price / credit 方法（`AICC_POLICY_METHODS`）、`aiccRefusalOf`；CLI `buckyos ai policy | policy-effective | policies | prices | credit | credit-ledger | credit-recharge | credit-recharge-get | credit-adjust`。
- `buckyos-agent`：`ai_quota` 输出余额与 limit 状态（`policy.status`）；AICC 失败输出 `error_code` 与 `details`。
- xllm（Rust `agent_tool` 与 TS）：拒绝映射为 `ProviderFailure::Refused { code }`（`quota_exhausted`、`credit_insufficient`、`resource_not_permitted`、`attribution_unavailable`），不重试；`quota_unavailable` 为临时错误。
- OpenDAN：拒绝的 Turn 以 `error.kind = "ai_quota"` 结束；私聊向 owner 发 `ai_credit_insufficient_owner` / `ai_quota_limited_owner` / `ai_resource_not_permitted_owner`，群聊只发 `ai_quota_limited`。
- AIWorkspace：拒绝映射为子码 `AI_QUOTA_EXHAUSTED` / `AI_CREDIT_INSUFFICIENT` / `AI_RESOURCE_NOT_PERMITTED` / `AI_ATTRIBUTION_UNAVAILABLE`，协作者只看到通用文字。
- Desktop：AI Center › 策略 / 价目表 / 点数页面，Usage 的 Credit 列与按用户组，Models / Playground 的价格标记与拒绝说明，首页余额与 limit 提示，Add Provider 的开放范围。

## 10. 已知限制

- 不预占：并发或大调用可以把余额、窗口扣过头（预期行为）。
- 准入在每次尝试前读一次策略、价目表、计数器与余额（SQLite 内的小表），不做缓存。
- 进程在准入之后、`set_running` 持久化 binding 之前崩溃时，这次尝试没有扣费记录；已持久化 binding 的任务在恢复时会以真实 usage 或 `unknown_usage` 结算。
- 一个任务只有一条用量事件，记在实际执行的模型上（由“不可用时”策略切换而来时带 `fallback_from`）；失败但已上报 usage 的尝试有扣费记录，但不单独出现在 usage 明细里（这类失败不会触发切换）。
- 未代表他人的 System / Device 主体不受策略约束、不扣费。
- 用户目录除 Agent owner 外不缓存，每次准入读取 system-config。

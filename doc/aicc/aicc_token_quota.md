# AICC Token 额度与资源可见性：实现说明

- 需求：[AI Center Token Quota 与资源可见性 PRD](../../product/ai_center/AI_Center_Quota_PRD.md)（v1.0 定稿）
- 共享类型与方法名：`src/kernel/buckyos-api/src/aicc_quota.rs`；错误码：`aicc_client.rs` 的 `AiccErrorCode`
- 服务实现：`src/frame/aicc/src/`（`service/identity.rs`、`visibility.rs`、`quota/mod.rs`、`storage/quota.rs`、`service/quota.rs`、`execution/mod.rs`）
- 本文记录当前实现的协议、存储与准入语义；行为以代码和测试为准。

## 1. 调用身份与归属

每个 AICC 请求先由 `RuntimeAuthorizer`（`service/identity.rs`）把会话解析成 `AuthorizedCaller`：

| 会话主体 | 额度与可见性主体 `user_id` | 业务 App `app_id` | 说明 |
| --- | --- | --- | --- |
| User（含 Limited） | 会话 `sub` | 会话 `app`（Desktop 等） | 读取账户的 UserType 与系统用户组；非 Active 账户与 `root` 账户拒绝；External 会话没有本 Zone 的 AI 资源（`resource_not_permitted`） |
| App 实例 | 实例 owner | AppInstanceId 的 app_id | 按 owner 的类型与组过滤资源 |
| Agent | AgentSpec 的 owner（缓存） | AgentId | Agent 不单独设可见性策略 |
| System / Device | 自身身份 | 服务名 | 不受可见性限制；额度只匹配以该身份为对象的规则（通常没有） |
| System + `on_behalf_of` | 被代表的用户 | 服务名（目前只有 `aiworkspace`） | 只有 RBAC 允许 `obj://aicc/on_behalf_of` write 的主体可以代表，当前只授予 `system:aiworkspace` |

- `on_behalf_of` 是 kRPC 顶层保留参数（`AICC_ON_BEHALF_OF_PARAM`），`AiccServerHandler` 取出后放进 task-local，`current_aicc_on_behalf_of()` 读取。内容为 `AiccOnBehalfOf { user_id, operator_id?, mode, run_ref? }`；`mode` 为 `direct / workspace_owner / current_user`，`run_ref` 记录 workspace / 实体 / run。
- Rust 客户端 `AiccClient::with_on_behalf_of`、xllm 的 provider 配置键 `on_behalf_of`、websdk 均会透传该参数。
- 受限会话（Limited / External）新增 `SessionScope::Aicc`，Limited 的会话上限包含它，External 不包含；RBAC 给 `limited` 加了 `obj://config/services/aicc/info` 的读权限。
- 用量记录（`AiccUsageEvent`）新增 `operator_id` 与 `attribution`：归属用户是 `user_id`，实际发起者是 `operator_id`。

## 2. 资源可见性

- 策略存于 `aicc_visibility_policy`，主体为 `type:admin` / `type:user` / `type:limited` 和 `group:<group_id>`。允许项三种：`all_providers`（跟随清单）、`provider_all_models`（某 Provider 全部模型，跟随清单）、`provider_models`（指定 exact model）。
- 用户的最终可见集 = 其 UserType 主体与所在系统用户组主体的允许项并集 ∩ 各 Provider 实际模型清单（`visibility::access_for`）。Root 视为 `type:admin`。停用的 Provider 不进入清单。
- 首次迁移时写入默认策略：`type:admin` 和 `type:user` 为 `all_providers`，`type:limited` 为空。之后不会再自动写入（`aicc_state.visibility_seeded`）。
- 策略修改用 `aicc_state.visibility_revision` 做乐观并发，`visibility.update` 必须带 `expected_revision`。
- 过滤生效点：`models.list`、`provider.list`、路由解析与预览、推理调用（主模型与 fallback 都检查）、额度规则草稿；诊断和路由 trace 对非管理员隐去不可见资源。管理员的 `models.list` 支持 `view: "manage"`，返回全量并标记自己是否可用。
- 规则所指资源对查看者不可见时，视图把资源替换成 `hidden`，状态为 `resource_unavailable`。
- 未向任何组开放的模型（`unexposed_models`）由 `visibility.get` 返回，供管理员提示。

## 3. 额度规则

- 对象：`user`（某用户的全部调用）、`app`（某 App 全部用户共享）、`user_app`（某用户的某 App）。资源：`all`、`provider`、`model`。对象 + 资源 = 规则的身份，编辑不可修改。
- 类型：`periodic`（4h / 24h / 72h / week，固定 UTC 窗口：4h、24h 从 Unix 纪元起算，72h 与 week 从 `1970-01-05 00:00 UTC` 起算）与 `total`（总量账户，不自动恢复）。
- 来源：`admin` 规则只能由管理员（或通过设置 RBAC 的系统主体）创建；`user` 规则只能是本人对自己 App 的周期额度（`user_app`，App 必须是本人的 App / Agent、`aiworkspace` 或本人用量里出现过的 App）。
- 同一来源、同一对象 + 资源 + 周期只能有一条启用规则；修改周期写入 `pending_period`，在当前窗口结束时切换（与已有规则冲突时标记 `pending_conflict`，不切换）。
- 总量账户按 `scope_key`（对象 + 资源）唯一，`account_id` 为其哈希。删除或停用总量规则不删除账户；同一范围重建规则时恢复原账本，不再次发放初始额度。只有充值能增加 `granted_tokens`；充值以 `recharge_id` 幂等，同 id 不同内容返回冲突。
- 所有规则变更、充值和可见性修改写 `aicc_quota_audit`。

## 4. 账本存储

均在 AICC 的 rdb（sqlx Any，默认 SQLite，可配 Postgres），schema 见 `storage/quota.rs` 的 `QUOTA_SCHEMA`，`STORAGE_SCHEMA_VERSION = 3`：

| 表 | 内容 |
| --- | --- |
| `aicc_quota_rule` | 规则；软删除；`version` 乐观并发 |
| `aicc_quota_counter` | 周期计数器：每个（范围, 周期, 窗口）的已结算 / 预占 / 待核对 |
| `aicc_quota_account` | 总量账户：累计发放、已结算、预占、待核对、起点 |
| `aicc_quota_grant` | 初始发放与充值流水 |
| `aicc_quota_attempt` | 每次上游尝试的预占：预算、状态、实扣、核对期限、命中的计数器与账户 |
| `aicc_quota_audit` | 变更记录 |
| `aicc_visibility_policy`、`aicc_state` | 可见性策略与修订号等状态 |

- 计数器只在有启用的周期规则维护该范围时存在；规则启用或新建时由 attempt 账本（无 attempt 的历史用量取 usage 表）重建当前窗口。
- 总量账户从不按历史重建；规则停用或删除后，账户仍随调用计数。
- 已结算且早于所有窗口的 attempt 与过期计数器在核对时清理（保留 8 天）。

## 5. 准入与结算

只有 `Llm`、`Decision`、`EmbeddingText`、`EmbeddingMultimodal` 计入 token 额度（`quota::is_metered`），其他 API 类型不受额度准入。

**预算**（`quota::attempt_budget`）：请求文本按 UTF-8 字节数计输入上界，每个媒体输入另计 4096、每条消息另计 16，再加输出上限：调用给出的 `max_output_tokens`（AICC 补默认值时即该值）→ 模型元数据 `max_output_tokens` → 8192 兜底（此时预算为软上限）。

**准入**发生在执行引擎每次上游尝试之前（`ExecutionEngine::admit`，attempt id 为 `<task_id>:<index>:<same_model_attempt>`）：

1. 推理入口先按可见性过滤候选并预检（`QuotaService::precheck`），全部候选不可用时直接返回拒绝，不建任务。
2. 一次尝试命中最多 9 个范围（3 种对象 × 3 种资源），所有启用规则都必须满足：周期规则 `settled + reserved + budget ≤ limit`，总量账户 `settled + reserved + budget ≤ granted`。
3. 写事务先 INSERT attempt（SQLite 由此取得写锁），再对每个计数器 / 账户做带条件的 UPDATE，任一失败整体回滚；冲突重试 3 次。并发下不会超发。
4. 某个候选被额度拒绝时，若路由允许运行时 failover 则尝试下一个候选，否则把拒绝返回调用方。

**结算**（`ExecutionEngine::settle` → `quota_settle`）：

- 成功并带 usage：按实际 token 结算，超出预算的部分形成欠额（`debt_tokens`），下次充值先补足。
- 明确失败且未消耗：释放预占。
- 明确失败但上游已在上报中给出 usage（流式输出中途报错、异步任务失败但已被计费）：按上报值结算并计费，同时写入 Usage 账本与 cost 快照；已产出的残缺交付物丢弃、任务仍标记失败，并且该次尝试不重试、不 failover，避免同一份消耗被重复计费。
- 结果未知（超时、取消竞态、连接中断）：转为待核对，预占保留到期限（普通调用 1 小时；原生长任务 30 分钟轮询上限 + 1 小时）。到期仍无结果按全额预算扣除。之后若得到真实用量，修正为实际值。
- 核对任务 `spawn_reconciler` 周期运行，同时执行到期的周期切换。

失败且未上报 usage 的尝试只在额度账本中出现，不写 Usage 表；Usage 表记录成功完成的调用，以及「失败但上游已上报 usage」的那些调用（带 `finance_snapshot`，对应任务状态仍为失败）。

## 6. 拒绝与错误码

| `AiccErrorCode` | 含义 | 调用方建议 |
| --- | --- | --- |
| `quota_exhausted` | 某条规则已无余量（含上限为 0） | 周期额度等 `reset_at_ms`；总量额度联系管理员充值 |
| `quota_budget_exceeded` | 有余量但不足本次预算 | 降低 `max_output_tokens` 或缩短输入 |
| `quota_unavailable` | 额度状态读写失败，未执行调用 | 稍后重试（可重试） |
| `resource_not_permitted` | 模型 / Provider 不在可见集 | 换模型或请管理员开放 |
| `attribution_unavailable` | 无法确定归属用户（代表请求的用户不存在等） | 修正调用身份 |

- 这些拒绝以 kRPC `RPCErrors::Detailed { code, message, data }` 返回，`data` 是完整 `AiccError`，额度拒绝的 `details` 为 `QuotaDenialDetails`：`required_tokens`、`max_allowed_tokens`、`provider_instance_name`、`exact_model`、`blocking`（阻断规则及其状态、是否可自动恢复）、`reset_at_ms`（所有阻断规则都可恢复时才给出）。其他 AICC 错误仍按原格式返回。
- 只有归属用户本人与管理员能看到阻断规则的明细；代表他人运行的服务（AIWorkspace）与 Agent 会话只向其他人显示通用提示。
- 判断只用错误码，不匹配错误文本：Rust `AiccError::from_krpc_error`、websdk `aiccRefusalOf`、Desktop `quotaRefusalOf`、llm_context `ProviderFailure::Refused(RefusalKind)`。

## 7. kRPC 方法

方法名见 `quota_methods`，请求 / 响应类型同名于 `aicc_quota.rs`：

| 方法 | 参数 | 权限 |
| --- | --- | --- |
| `quota.query` | `app_id?` | 任何可用 AICC 的会话：作用于本人（或本 App 调用）的规则及状态，含 `near_limit_count` / `exhausted_count` |
| `quota.rule.list` | `scope: mine \| manage`, `include_deleted` | `manage` 需管理员 |
| `quota.rule.get` | `rule_id` | 管理员或规则相关用户；含同时生效的约束、账户、流水、变更记录（非管理员裁剪） |
| `quota.rule.preview` | `draft` | 同创建权限；返回当前状态与冲突规则 |
| `quota.rule.create` / `update` / `delete` | `draft` / `rule_id + expected_version + 修改项` / `rule_id + expected_version` | 见 §3 来源规则 |
| `quota.account.get` | `account_id` | 管理员或账户所属用户 |
| `quota.account.recharge` | `account_id, recharge_id, amount_tokens, note?` | 管理员（设置写权限） |
| `quota.recharge.get` | `recharge_id` | 管理员；用于超时后按原单号查询 |
| `visibility.get` / `update` / `preview` | — / `expected_revision, subject, entries` / `user_id? 或 draft?` | 管理员（`update` 需设置写权限） |

`models.list` / `provider.list` 增加 `view`，`usage.query` 增加 `zone_wide`（仅管理员）。

## 8. 调用方

- websdk：`AiccClient` 的 quota / visibility 方法、`aiccRefusalOf`；CLI `buckyos ai quota [--app]`。
- `buckyos-agent`：`ai_quota` 命令；AICC 失败输出 `error_code` 与 `details`。
- xllm（`agent_tool`）：把 AICC 拒绝映射为 `ProviderFailure::Refused`，结果的错误类别为对应码，不重试；`quota_unavailable` 视为临时错误。
- OpenDAN：拒绝的 Turn 以 `error.kind = "ai_quota"` 结束、不重试；私聊向 owner 发 `ai_quota_limited_owner` / `ai_resource_not_permitted_owner`，群聊只发 `ai_quota_limited`；Turn 任务失败码为 `ai_quota`。
- AIWorkspace：`ws.set_wish_aicc`（`workspace_owner` 默认 / `current_user`，需 manage）；run 受理时固定归属并存入 `detail.aicc`，AICC 调用带 `on_behalf_of`；拒绝映射为子码 `AI_QUOTA_EXHAUSTED` / `AI_QUOTA_BUDGET_EXCEEDED` / `AI_RESOURCE_NOT_PERMITTED` / `AI_ATTRIBUTION_UNAVAILABLE`，协作者只看到通用文字。
- Desktop：AI Center › 额度 / 可见性页面、首页提示、Usage 统计行“设置额度”、Add Provider 开放范围、Models / Playground 的额度标记与拒绝说明；AIWorkspace 权限面板的归属设置与许愿格运行的归属显示。

## 9. 已知限制

- 未代表他人的 System / Device 主体不受可见性约束，也通常没有额度规则。
- 用户目录除 Agent owner 外不缓存，每次准入读取 system-config。
- 输入按 UTF-8 字节估算，对多字节文字偏保守。
- 历史 usage 只在没有 attempt 记录的时段计入窗口重建。
- 额度拒绝不主动通知 Agent owner（只在会话里回复）；邮件 / 推送通知不在本期。

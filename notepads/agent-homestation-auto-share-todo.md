# Agent 默认 HomeStation、同 Zone 互相关注与“自动分享成果” TODO

日期：2026-10-23

状态：v0.3。2026-10-24 §3–§7 全部实施完成并在本机 Zone 完成 DV（记录见 §10.4）；未提交 git。

需求原文（用户）：

> 在创建 Agent 的时候，也默认 enable home station，并且与主人互相关注。同 zone 内的 agent 之间可以默认互相关注。默认开启“Agent 自动分享成果”。此选项开启后，Agent 在完成一个大的任务后（一般是 analyze intent 创建的用户任务），会围绕产物往自己的 homestation 发一条动态，一般是隐私无关的信息。目的是促进未来的 Agent 通过该网络互相交流复杂任务的成功经验（skill 自学习）。

依据：

- [BuckyOS Agent 的初始化流程 PRD](<../product/users_and_agents/BuckyOS Agent的初始化流程PRD.md>) §3.2、§5.4、附录 B.4（AgentSettings）、附录 C。
- [HomeStation 协议与实现](<../doc/homestation/HomeStation 协议与实现.md>) v0.2：§1（“Agent 等没有用户记录的身份没有 HomeStation”）、§2.5（关注声明）、§5.5（来源与关注、好友派生）、§9 差异表（“Agent：Zone 内的 Agent 没有 HomeStation”）。
- [HomeStation 架构设计](<../doc/homestation/BuckyOS HomeStation 架构设计.md>) §4.4、§4.6、§7.6。
- [意图分析 TODO](opendan-analyze-user-intent-todo.md)（`try_analyze_user_intent` → `agent-session create-worksession … --report final`）；[Agent 通知人 TODO](opendan-agent-notify-todo.md)（`NotifyHost` / `ZoneNotify` / `LocalNotify` 的宿主分层，本文的 `ShareHost` 同构）。
- 用户原则：[先能力后安全](../../.claude/projects/-home-lzc-project/memory/feedback-capability-before-security.md)（早期安全顾虑写成已知风险，不加审批）；Zone 服务默认多用户。

读者：实施本文的 code agent。路径相对 `buckyos/`。

## 1. 目标与边界

1. 每个 Agent 创建后默认拥有自己的 HomeStation：`/home/<agent 短名>`、门户 `/homestation/<agent 短名>`，由现有 homestation 服务按用户一样服务（独立数据库、发表流、投递入口）。
2. 同 Zone 关系默认互相关注：Agent ↔ 它的主人；Agent ↔ 同 Zone 的其他 Agent。关系由 HomeStation 自动派生（像好友派生关注一样），不需要人操作，Agent 删除后自动撤回。
3. “Agent 自动分享成果”默认开启：用户任务（UI Session 经意图分析创建、`--report final` 的 Work）成功结束并提交最终报告后，Agent 用 LLM 把“做了什么、怎么做成的、产物是什么”写成一条**不含隐私**的公开动态，发到自己的 HomeStation；产物按 obj_id 附带（能注册的才附）。
4. 两个开关都在 `AgentSettings`，创建向导与 Agent 详情页可改：`homestation`（默认 true）、`auto_share_results`（默认 true）。关掉 `homestation` 时 Agent 不再被当作发表者（也不再分享）。

不做：

- 跨 Zone 的 Agent 关注与投递（HomeStation 跨 Zone 本来就未做）。
- Agent 自己的 Message Center 联系人（Agent 站点的“好友派生”只对主人与同 Zone Agent 生效；Agent 的 `friends`/`group` 受众不可用，Agent 发表只用 `public`）。
- 自动分享的内容审查/审批流程（按“先能力后安全”：隐私过滤只靠提示词与规则，记为已知风险 §8）。
- Agent 之间读到动态后“自学习”的消费侧（订阅、评价、提炼 skill）——本文只做产生动态与网络关系；消费侧后续单独 TODO。

## 2. 术语

- **Agent 短名**：`agent_short_name(agent_id)`，即 AgentDID 的第一段（`jarvis` of `jarvis.alice.buckyos.io`），与用户名共享 `<name>.<zone>` 命名空间（control_panel `AgentNameIndex` 已保证不冲突），所以可直接作 `/home/<user>` 路径段。
- **zone 依据**：HomeStation 来源记录 `sources.basis` 新增的一种依据 `"zone"`，与 `"friend"` 同类：系统派生、用户不能单独取消（`unfollow` 返回 `zone_basis`）。
- **用户任务**：`SessionKind::Work` 且 `origin.parent_session` 是一个 `SessionKind::Ui` 的会话、`origin.report = final`（意图分析创建的 Work 就是这种形态）。

## 3. 共享类型与 control_panel（已实施）

- `buckyos-api` `AgentSettings` 新增 `homestation: bool`（`default_true`）、`auto_share_results: bool`（`default_true`），`shares_results()` = `enabled && homestation && auto_share_results`。旧记录缺字段时按 true 读（与“默认开启”一致；仓库规则不做迁移，这是 serde default 不是兼容分支）。
- control_panel `agent.create` 新增可选布尔参数 `homestation`、`auto_share_results`（默认 true，进 fingerprint）；`agent.update` 接受这两个字段；`agent.get/list` 的 `settings` 视图带这两个字段。
- 不改 RBAC：HomeStation 自己校验 session token，网关 `/kapi/homestation` 不做 RBAC；`object.register` 走 `obj://ndm/objects` read，`agent` 角色已有。

## 4. HomeStation：Agent 作为发表者（服务侧）

落点 `src/frame/homestation`。

### 4.1 用户表

- `users::ZoneUser` 增加 `agent: Option<ZoneAgentRef { agent_id: String, owner_did: String }>`（`None` 为人）。
- `SystemConfigUsers::load`：在现有用户之后，遍历 `users/<owner>/agents/<agent_id>`：读 `spec`（`buckyos_api::AgentSpec`，无 spec = 创建中，跳过）、`settings`（`AgentSettings`，`enabled && homestation` 才服务）、`profile`（`AgentProfile::resolved_display_name`）。`user = agent_short_name`，`did = spec.agent_doc.id`，`zone_feed_writer = false`（Agent 默认不进 Zone 汇总列表；管理员可用 `zone_feed_writers` 覆盖），`agent = Some{agent_id, owner_did: spec.agent_doc.owner}`。短名不满足 `valid_user_segment` 或与已有用户名重复时跳过并 warn。
- `UserRegistry` 不变（60s TTL，miss 时 5s 重载）：新建 Agent 的站点在首次访问或下一次刷新时打开。

### 4.2 站点打开与刷新

- `Host` 新增 `refresh_loop`（服务模式 60s 一次；standalone 由 `workers` 控制）：`users.list()` → 对未打开的用户 `open()`；若用户集合（did 集）与上次不同，则对**所有**已打开站点调用 `sync_zone_relations()`。首次 `open_all` 后立即跑一次。
- `Station` 需要知道自己是谁：`StationConfig` 增加 `agent: Option<ZoneAgentRef>`；builder 从 `ZoneUser` 复制。

### 4.3 zone 依据的关注派生（`sources.rs`）

`Station::sync_zone_relations()`：

- 期望集合 `want: Vec<(did, name)>`：
  - 我是 Agent：主人（`owner_did`，在用户表里找到才算）+ 同 Zone 其他 Agent（`agent.is_some() && did != 我`）。
  - 我是人：我名下的 Agent（`agent.owner_did == 我的 did`）。
- 对 `want` 逐个 `follow_did(did, name, "zone", None)`（内部已处理已存在来源只追加依据、发表/恢复关注声明并投递）。
- 清理：`basis LIKE '%zone%'` 且不在 `want` 的来源，去掉 `"zone"` 依据；依据为空则撤回关注声明并删来源（复用 `sync_friends` 的 stale 逻辑，抽成一个 `retire_basis(basis, keep: &[did])` 共用）。
- `unfollow` 对只剩 `zone` 依据的来源返回 `"zone_basis"`（与 `friend_basis` 同构）；`friend_basis` 判断改为“剩余依据里有 friend 或 zone”。
- 触发：`housekeeping_loop` 启动时（在 `sync_friends` 之后）、`Host::refresh_loop` 发现用户集合变化时、`admin.run {task: "friends"}` 时一起跑。
- 主人 → Agent 还会经 msg-center 的 `zone_agent` 好友关系（`upsert_zone_agent_contacts` 置 `FriendState::Friend`）走 `friend` 依据；两种依据并存，互不影响。

### 4.4 鉴权：Agent 的调用者身份

`auth.rs::session_caller`：

- `TokenPrincipalKind::Agent`：`did = sub`（canonical DID）、`principal = sub`。
- `TokenPrincipalKind::App`：若 `AgentId::parse(claims.target.appid_claim())` 成功且该 AgentId 是用户表里的 Agent（`ZoneUser.agent.agent_id`），则 `did = 该 Agent 的 did`（Agent 的运行 App 以 Agent 身份发表，不是以主人身份）；否则维持原逻辑（App 替账号说话）。
- `AccountDids` trait 增加 `async fn agent_did(&self, agent_id: &str) -> Option<String>`（默认 None）；`RuntimeAuth` 持有 `Arc<UserRegistry>` 实现它。standalone `StaticTokens` 不变（测试直接给 Agent 一个 token）。
- `Host::station_for_caller` 不变（按 did 找站点）。

### 4.5 门户/资料

- `/home/<user>/profile`、`profile.get`、`portal.*` 的用户条目增加 `"kind": "agent" | "person"`，Agent 再带 `"owner": <owner did>`。前端本期不改。

### 4.6 投递

`delivery.rs` 的 `known_without_home` 对 Agent 现在返回 false（它在用户表里），无需改。注意 `directory.home_of(agent_did)`：`hosted_here` 判断 DID 是否属于本 Zone 子域（`<name>.<zone>`）→ `by_did_fresh`，已覆盖。

### 4.7 standalone / 测试

- `StandaloneUser` 增加 `agent: Option<{agent_id, owner_did}>`；`build` 把它放进 `ZoneUser.agent` 与 `StationConfig.agent`。
- `tests/common/mod.rs::Net` 增加 `new_zones_with_agents(..., agents: &[(zone_owner, agent_short_name)])`：Agent did `did:test:<name>`，`agent_id = "<name>.<owner>.test"`，token `tok-<name>`；`new_zones` 委托它。
- 新测试 `tests/agents.rs`：
  1. Agent 有主页：`GET /home/<agent>/profile` 200、`kind = agent`；`ui.bootstrap` 以 Agent token 调用得到自己的 home。
  2. 互相关注：`admin.run friends`（或刷新）后，主人 `sources.list` 含 Agent（basis 含 zone），Agent `sources.list` 含主人与另一个 Agent；`followers` 两边都有；`sources.unfollow` 返回 `zone_basis`。
  3. Agent 发公开动态后，主人与另一 Agent 的 `candidates.list`/`reading.list` 能看到（同 Zone 投递）。
  4. 从用户表移除 Agent（StaticUsers 可变）并刷新后，关注声明撤回、来源删除。
- 现有测试 `friends_without_homestation`（`did:test:<name>-kid` 无主页）保持。

### 4.8 文档

`doc/homestation/HomeStation 协议与实现.md`：§1 第 38 行改写（Agent 也是发表者，开关 `AgentSettings.homestation`）、§5.5 增加“zone 依据”、§9 差异表 Agent 行改为“已支持；Agent 无自己的 Message Center 联系人，受众只用 public”。

## 5. libopendan / opendan：自动分享成果

落点 `src/frame/lib_opendan`、`src/frame/opendan`。

### 5.1 ShareHost（宿主分层，与 `notify.rs` 同构）

新模块 `lib_opendan/src/share.rs`：

```rust
pub struct SharePost { pub key: String, pub session_id: String, pub text: String, pub tags: Vec<String>,
                       pub artifacts: Vec<ShareArtifact { name: String, obj_id: String }> }
pub struct ShareReceipt { pub entry: Option<String>, pub obj_id: Option<String> }
#[async_trait] pub trait ShareHost: Send + Sync {
    async fn enabled(&self) -> bool;                       // 读开关（Zone：AgentSettings.shares_results()）
    async fn publish(&self, post: &SharePost) -> Result<ShareReceipt>;
}
```

- `ZoneShare`（libopendan，Zone 内）：`enabled()` 用 `buckyos_api` runtime 读自己的 `AgentSettings`（agent 角色对 `users/{user}/agents/{agent}/settings` 有 read；loader 已有 `records::settings`，可从 opendan 侧注入一个读取闭包以免重复）；`publish()`：`runtime.get_zone_service_krpc_client("homestation")` → `publish.create { key, input: { text, tags, audience: {kind: "public"}, category: "work", attachments } }`；附件先对每个 `cyfile:` obj_id 调 `object.register {objId, metadata:{mime}}`，失败的产物只在正文里保留名字。kRPC 失败 → `Err`，由调用方记日志，不重试（发表任务本身在 HomeStation 侧幂等：`key = share:<session_id>`）。
- `LocalShare`（`--dev` 与测试）：写 `<AgentRoot>/shares/<key>.json`。
- `HostDeps.share: Option<Arc<dyn ShareHost>>` → `RunnerDeps.share`（`None`：不分享）。opendan `main.rs` Zone 模式给 `ZoneShare`，dev 模式给 `LocalShare`。

### 5.2 触发条件与时机

`runner/outcome.rs::after_run_end`，在 `report(sh, status)` 之后、`flush_outbox` 之前（不阻塞交付：`tokio::spawn`）：

- `next.finished && next.outcome == Some(Succeeded)`；
- `s.config.session.kind == Work`，`origin.parent_session` 的注册项 `kind == Ui`（`sh.agent().sessions().lookup`），`origin.report == Some(Final)`；
- `final_report(run)` 为 `Some`；
- `deps.share.is_some() && share.enabled().await`。
- 幂等：run.json `host.extra.shared = true` 写入后不再分享（恢复/重放不重复发）。

### 5.3 内容生成（LLM 一次调用）

`share.rs::compose(llm, model, input) -> Result<Option<SharePost>>`，与 `LlmSummarizer` 同样的 `LlmInferenceRequest`（`force_json = true`），输入：objective、最终报告正文、产物名列表、所用 cmd_skills 名、turns 数；系统提示要点：

- 面向其他 Agent 写“经验帖”：任务类型、关键步骤与决策、用了哪些命令/技能、验证方法、踩过的坑；不超过 1200 字符（HomeStation 正文上限 2000）。
- **禁止**出现：人名、账号、邮箱、电话、地址、具体文件路径、主机名、密钥/令牌、聊天原文、报告里的业务数据与数字细节；涉及个人事务（健康、财务、家庭、日程）或无法去隐私时输出 `share: false`。
- 输出 JSON：`{ "share": bool, "reason": string, "text": string, "tags": [string] }`。`share=false` 或 `text` 空 → 不发，日志 `opendan.share.skipped`。
- 标签固定加 `agent-experience`，模型标签最多 5 个。

### 5.4 产物

`ReportSubmission.artifacts[*].obj_id` 为 `cyfile:` 的进入 `SharePost.artifacts`；没有 obj_id（dev 模式）的只留名字。正文末尾由 ZoneShare 拼接产物名（不拼路径）。

### 5.5 测试（`lib_opendan/tests/share.rs`）

用 `common::Env` + `ScriptedLlm` + `LocalShare`：

1. UI Session 下用 `agent-session create-worksession … --report final` 创建 Work，驱动到 `report` 结束 → `shares/share:<sid>.json` 存在，text 来自脚本的 compose 回复，附带 tags 含 `agent-experience`。
2. compose 回复 `share: false` → 无文件、日志有 skipped。
3. 直接创建的 Work（无 UI 父）或 `--report none` → 不触发；失败结束不触发。
4. 同一 run 重放 `after_run_end`（crash 恢复路径的现有测试模式）只发一次。

## 6. Desktop UI

- `app/agent-setup`：`model.ts` 草稿加 `homestation: true`、`autoShareResults: true`；`IdentityPages.tsx` “访问”区下新增“社交”小节两个开关（`agentSetup.social.homestation` / `agentSetup.social.autoShare` + hint）；`ConfirmPage.tsx` 列出；`toCreateRequest` 带 `homestation`、`auto_share_results`。
- `users-agents` 详情页：Access 卡后新增“社交”卡，两个可切换开关（`store.updateAgentSettings(agentId, {homestation?, autoShareResults?})` 复用 `updateAgent`）；`api/user_mgr.ts` 类型与 `updateAgent` 参数；`control_panel_mock.ts` 默认值与 `agent.update`。
- i18n `i18n/agent-setup.ts` en/zh。
- e2e：`agent-setup` mock 用例补一条“默认开启、可关闭并出现在确认页”；详情页用例补切换。

## 7. PRD 回写

`BuckyOS Agent的初始化流程PRD.md`：§3.2 默认值表加两行（HomeStation 主页 **开启**；自动分享成果 **开启**）；§5.4 改名“通信与社交开关”，补两行；附录 C 追加 C.5“2026-10-23 HomeStation 与自动分享”。

## 8. 已知风险（不在本期处理）

- 自动分享的去隐私只靠提示词与硬规则；模型可能漏删细节。缓解：动态只对公开受众，主人在 MessageHub/HomeStation 能看到并 `entry.withdraw`；后续可加“发表前由主人确认”开关。
- Agent 的 HomeStation 由 OOD 设备密钥代签（与用户相同的 Zone 托管）。
- Agent 发表不进 Zone 汇总列表（`zone_feed_writer=false`），避免 Agent 刷屏公共门户。
- Agent 删除后其 HomeStation 数据库留在 `users/<短名>/`，不自动删除（与用户删除一致）。

## 9. 实施进度

- [x] §3 共享类型与 control_panel（2026-10-23）
- [x] §4 HomeStation（2026-10-24：users/host/sources/auth/standalone/tests/agents.rs 2 例 + 全套通过；实现文档 §1/§4/§5.5/§9 已改）
- [x] §5 libopendan / opendan（2026-10-24：`lib_opendan/src/share.rs`+`runner/share.rs`、`RunHandle::mark_shared`、opendan `RecordsShareSwitch`；tests/share.rs 5 例通过；ZoneShare 未对真实 HomeStation 验证）
- [x] §6 Desktop（2026-10-23：向导社交小节、详情页社交卡、mock、i18n、e2e 14/14）
- [x] §7 PRD（§3.2、§5.4、B.4 settings 表、附录 C.5）；HomeStation 实现文档随 §4
- [x] DV（2026-10-24，§10.4；DV 修掉 3 个问题）

## 10. 实施记录（2026-10-24）

### 10.1 落点

| 范围 | 文件 |
| --- | --- |
| 共享类型 | `kernel/buckyos-api/src/app_schema.rs`：`AgentSettings.homestation`、`auto_share_results`、`shares_results()` |
| control_panel | `agent_mgr.rs`：`AgentCreateRequest` 两个 flag（进 fingerprint）、`settings()`、`settings_view`、`handle_agent_update`；单测 `create_request_is_normalized_and_fingerprinted`；DV 脚本 `test/test_control_panel/test_user_mgr.ts` 两条断言/用例 |
| HomeStation | `users.rs`（`ZoneUser.agent`、`load_agent`、`agent_by_app`）、`host.rs`（`refresh_users`/`refresh_loop`、`user_kind`）、`sources.rs`（`sync_zone_relations`、`sync_derived`、`DERIVED_BASES`、`zone_basis`）、`auth.rs`（`AccountDids::agent_did`、Agent/App 主体）、`stream.rs`+`api.rs`（profile `kind`/`owner`）、`standalone.rs`（`StandaloneUser.agent`、`Standalone.users`）、`main.rs`、`tests/common/mod.rs`（`new_zones_with_agents`）、新 `tests/agents.rs` |
| libopendan | 新 `src/share.rs`（`ShareHost`/`ShareSwitch`/`compose`/`ZoneShare`/`LocalShare`）、新 `src/runner/share.rs`（`after_finish`）、`runner/outcome.rs` 一行调用、`session/runs.rs`（`mark_shared`/`shared`）、`runner/mod.rs`+`host/mod.rs`（`share` 依赖）、`xagent_cli.rs` 与各测试的 `HostDeps` 字面量；新 `tests/share.rs` |
| opendan | `loader.rs`（`LoaderEnv.share`）、`records.rs`（`RecordsShareSwitch`）、`main.rs`（Zone→`ZoneShare`，`--dev`→`LocalShare`）、`tests/loader.rs` |
| Desktop | `agent-setup/{model,IdentityPages,ConfirmPage}.tsx`、`users-agents/.../AgentDetailPage.tsx`、`datamodel/store.ts`（`setAgentSocial`）、`api/user_mgr.ts`、`api/control_panel_mock.ts`、`i18n/agent-setup.ts`、e2e `agent-setup.spec.ts`/`users-agents.spec.ts` |
| 文档 | PRD §3.2/§5.4/B.4/C.5；`doc/homestation/HomeStation 协议与实现.md` §1/§4/§5.5/§9 |

### 10.2 与 §4–§6 的差异

- `Host::refresh_users` 还会关闭已离开用户（被删 Agent、关掉 `homestation`）的站点，否则缓存里的站点继续应答；其后台 worker 在 `Arc` 释放前仍在跑（已知泄漏，数据库留盘）。
- standalone 的用户注册表 TTL 从 3600s 改为 0（StaticUsers 每次重读，测试需要动态增删）；服务模式仍 60s。
- `RuntimeAuth` 去掉了 `Default`，构造需传用户注册表。
- 分享的失败窗口：结束提交之后、分享之前崩溃则丢这一条（`after_run_end` 不在提交后重放，与感知/报告补偿同类）；提交前的故障与已标记 `shared` 的 run 只发一次（tests/share.rs 覆盖）。
- `ShareInput.cmd_skills` 取 `SessionConfig.prompt.cmd_skills`（冻结选择），不从 worklog 推导。
- 向导与详情页：关掉 HomeStation 开关时自动分享开关置灰（UI 侧的依赖表达，后端不强制）。
- 未改 OpenDAN WebUI、门户前端对 `kind` 的展示。

### 10.3 验证与未做

- 已跑：`cargo test -p homestation -- --test-threads=1`（全部通过，含 agents 2 例）；`cargo test -p libopendan --test share --test report --test notify --test outbound`（5/7/7/19 通过）；`cargo test -p opendan --test loader`（24 通过）；`cargo test -p buckyos-api`（286 通过）；`cargo test -p control_panel`（194 通过，1 失败 `sso_identity::tests::account_identity_carries_ceiling_method_and_password_state` 为既有失败：工作区里别处未提交的改动给 scopes 加了 `Aicc`，与本任务无关）；`cargo check --tests` 七个相关 crate 通过；desktop `pnpm check` 通过、`pnpm lint` 无新增告警、两份 e2e 14/14。
- 未做 DV：需要重建 `homestation`/`control-panel`/`scheduler` 二进制与 aios 镜像（`./build_aios --local-test`）并 `start.py`；验收项：新建 Agent 后 `GET https://<zone>/home/<agent>/profile` 为 `kind=agent`，主人的 HomeStation `sources.list` 出现 Agent（basis 含 zone）；在 MessageHub 让 Jarvis 做一个经意图分析创建的任务，结束后 Agent 主页出现一条 `agent-experience` 动态且无隐私信息；`agent.update {auto_share_results:false}` 后不再发。`ZoneShare` 的 `object.register`/`publish.create` 请求形状未对真实 HomeStation 验证。

### 10.4 DV 记录（2026-10-24，本机 test.buckyos.io）

部署：`buckyos-build.py -s homestation control_panel desktop` + `./build_aios --local-test --reuse-cargo-target` + `start.py`。驱动脚本 `/root/.claude/jobs/81cbdea1/tmp/dv/hs_dv.ts`（agents/get/set/create/hs/profile/index/dan），任务驱动 `test/test_opendan/test_agent_ask.ts send`（`BUCKYOS_TEST_AGENT_ID=xiaobai.test.buckyos.io`）。

已验：
- 旧 Agent xiaobai 的 `agent.get` settings 显示 `homestation=true`、`auto_share_results=true`（缺字段按默认读）。
- `GET /home/xiaobai/profile` 200，`kind=agent`、`owner=did:bns:devtest`，`following=1`（主人）；devtest `sources.list` 出现 xiaobai，`basis=["friend","zone"]`。
- 新建 Agent xiaohong（`agent.create`，默认两开关 true）就绪约 1 分钟后，homestation 不重启即开出 `/home/xiaohong`（refresh_loop），`following=2`（主人 + xiaobai）；devtest 的来源也出现 Xiaohong（zone 依据）。
- 真实任务：MessageHub 发“写 fib.txt”请求 → analyze_intent 创建 Work → 40s 内完成并回报 → `after_finish` 触发 compose。

DV 通过的验收项（修掉下面三个问题之后）：
- 真实任务（MessageHub → analyze_intent → Work → 最终报告）结束后 4 秒内 `opendan.share.published`，xiaobai 主页出现一条公开动态：`publication_category=work`、tags `["agent-experience","shell","wc","unicode","脚本验证"]`，正文只讲做法（`wc -l -w -m`、`bash -n`、边界测试），无路径/人名/数据；主人 devtest 的 `reading.list` 收到这条动态（同 Zone 投递）。
- `agent.update {auto_share_results:false}` 后再做一个任务：Work 正常结束，无 share 日志，主页仍只有 1 条；再改回 true。
- 关注投递：devtest → xiaobai/xiaohong/alice/bob 的关注声明与 Head 全部 `accepted`，四人 `followers=1`。Agent 站点自己发出的关注（xiaobai/xiaohong → 主人与彼此）在退避到期后也全部 `accepted`：最终 devtest `followers=2`，xiaobai、xiaohong 各 `followers=2, following=2`，三者两两互关。
- `profile.posts` 不计 `category=work` 的条目（既有语义：`posts` 只数无分类正文），所以 Agent 主页 `posts=0` 而 feed 有 1 条。

DV 修掉的三个问题：
1. compose 调 LLM 报 `OpenAI invalid_request_error: Web Search cannot be used with JSON mode`：`create_run_llm` 继承了会话的 web_search 能力；compose 请求现在 `disable_capabilities = [web_search, tool_call]`（`share.rs`）。
2. Zone 内所有关注声明/拉取 401（alice/bob 也一样，与本任务无关的既有问题）：读者证明由 OOD 设备密钥签名，`NameDirectory::signer_key` 用 `name_client::resolve_auth_key(did:web:ood1…)` 解析时 name-client 把未指明类型的 DID 默认按 zone 文档查（`doc_type not specified … defaulted to zone not found`，resolve-did 重构后行为）。本 Zone 设备现在改为 `resolve_did(…, Some(DidDocType::Device))` + `parse_did_doc().get_auth_key(fragment)`（与 msg-center `cyfs_dispatch.rs` 一致）。同一原因也让接收方对投递对象报 `signing-key-unavailable`（outbox `last_result`），修后 `entry.retry_delivery` 即 accepted。复现方法：用 `/opt/buckyos/security/<device>/authentication.private.pem` 签一个 `iss=<user> aud=cyfs://<zone>/home` 的 EdDSA JWT，`curl -H "Authorization: DID <jwt>" http://127.0.0.1:4130/home/<user>/feed?mode=display` 读响应里的 message。
3. Agent 容器发表报 `error sending request for url (http://172.17.0.1:4130/kapi/homestation)`：服务模式的 homestation 只监听 `127.0.0.1:4130`，容器经 docker 网关访问不到；改为 `0.0.0.0:{HOMESTATION_SERVICE_PORT}`（`main.rs`，与 control-panel 等服务一致；本 Zone 源站映射仍是 127.0.0.1）。

部署坑：`pkill` 后 node-daemon 在 1 秒内就把旧二进制拉起，`cp` 会 `Text file busy`；要 `cp` 到 `.new` 再 `mv -f` 覆盖，然后再 kill 一次，并用 `md5sum /proc/<pid>/exe` 确认进程跑的是新文件。`start.py` 之后容器用的镜像以 `docker inspect --format '{{.Image}}'` 对照 `docker images --no-trunc local/aios-test:latest-amd64`。


# Self-host Group 应用扩展实施 TODO

日期：2026-10-24。状态：**D-1～D-6 已于同日由用户拍板（§1、§9）；§11 官方例子（sys-test 客服群用例）同日新增，D-8 同日拍板按推荐，EXT-10 / EXT-11 已实施（见 §11.6）。D-5 选备选，SSO-10 `login_by_app_access_token` 移入本批（§6 EXT-09）。D-7 已于同日由用户确认按推荐方案（兑换入口放 CP，见 §10 偏差 1）。阶段 A～D 的代码与文档已于同日实施，单测 / Playwright mock 通过；同日部署到本机单 Zone（test.buckyos.io）完成 DV，见 §10。** 本文把 [应用扩展架构设计](<../doc/message_hub/BuckyOS Self-host Group的应用扩展架构设计.md>)（下称架构稿，2026-09-30 定稿后未再修改）对照当前源码，列出已落地部分、决策和拆出的任务。前置依赖混合 SSO 已于 2026-10-22 实施并通过 DV（[混合 SSO TODO](hybrid-sso-implementation-todo.md) §11）。

目标：补齐架构稿五层中唯一未动的 **L2 嵌入式应用侧面板**，收尾 L1 展示配置与 L3 Hook 的 UI 配套和凭据模型，并让 **L4A 自定义 UI 能服务外部客户**（App 代登录）；使“L1 + L2 + L3”和“L1 + L4A + L3”两种组合在真实 Zone 可用。L4B 只修正文档状态。

依据：

- 架构稿：边界、四层扩展、故障隔离、安全边界、§12.2 待明确事项。
- [Self-Host-Groupv2](../doc/message_hub/Self-Host-Groupv2.md) §3 Group Configuration、§4.3 访问路径限制、§7 扩展点、§14 待确认事项。
- [扩展 Self-host Group：应用开发指南](<../doc/sdk/扩展self-host-group.md>)（下称 SDK 指南，核对基线 2026-10-01 `573f7924`）：各层“已实现 / 待实现”判定与当前 wire。
- [Session Only Member 设计](<../doc/message_hub/Session Only Member 设计.md>)（下称 SOM）§4～§5、§7、§12 与 [混合 SSO 技术契约](<../doc/arch/verify-hub/混合SSO技术契约.md>)（下称契约）§3、§5～§7、§13：App 代登录的旧设计、受限会话、专属邀请记录与 family。
- [混合 SSO TODO](hybrid-sso-implementation-todo.md) SSO-10 条目：本文 EXT-09 继承其验收，原条目标为移入本文。
- [群 v2 后台接口](../src/frame/msg_center/doc/group-v2-backend.md)。

源码核对基线：2026-10-24 工作区（含混合 SSO 未提交改动）。本轮只做静态核对，未运行测试、未在 Zone 上复现任何问题。

阅读顺序：§0 现状，**§1 决策（D-1～D-7 已定）**，§2 交付顺序，§3～§6 任务，§7 验收，§8 文档联动，§9 决策回写记录，§10 实施记录，**§11 官方例子（sys-test 客服群用例，D-8 已定，§11.6 实施记录）**。

## 0. 现状：架构稿五层对照源码

| 层 | 架构稿定位 | 当前实现 | 本批要补的 |
| --- | --- | --- | --- |
| L1 配置型 | Group Configuration 是对外契约 | `GroupConfiguration`（[group_types.rs](../src/frame/msg_center/src/group_types.rs)）：`schema_version=1`、`required_features`、`roles`、`default_session` / `session_templates`、`limits`、`display`、`access`、`extensions`；`group.create` 原子建群，`group.apply_config` 带 `expected_revision` + 幂等键 | `display` 是无 schema 的 `Value`，Message Hub（GroupPanel / SessionDetails）不渲染任何展示配置；schema 冻结按 D-6 |
| L2 应用侧面板 | Message Hub 加载应用 HTML 面板 | **未实现**。后端只保存 `extensions: Value` 与 `SessionRules.panels: Vec<String>`，`validate` 不检查面板声明；Message Hub 无加载代码，`MessageHubAppPanel.tsx` 是桌面承载 Message Hub 自身的入口 | 整层：声明、加载、上下文、来源校验、故障隔离 |
| L3 Hook | Process Chain 式逻辑扩展 | HTTP Hook 已实现（`run_group_hooks`，[group_service.rs](../src/frame/msg_center/src/group_service.rs)）：`join` / `post` / `read` 三点，`deny` / `native_only`，`may_grant` 动态许可，`read` 结果缓存，`post` 结论随幂等结果保存；拒绝不可被故障策略放行 | 出站凭据模型（D-3）；`group.check_access` 不跑 `post` Hook；Message Hub 不展示 Hook 绑定；通用 Process Chain 不在本批 |
| L4A 自定义 UI | 复用 Message Center，可限制访问路径 | `AccessConfig.allowed_clients: Any / Only(appid)` 已实现，比对 token 的 `appid`；App 子域页面可直接调 `/kapi/msg-center`（网关按路径转发服务，见下） | Message Hub 的 appid 粒度（D-4）；外部客户进 App 的登录方式（D-5 / D-7，EXT-09） |
| L4B 独立实现 | 协议互操作 | 群 CYFS HTTP 路径已实现；Joined Group 绑定手工配置、30 秒轮询 | 不在本批；只修正文档状态 |

**混合 SSO 已交付、本批直接复用的能力：**

- Limited / External 受限会话以 `SessionScope::MsgGroup` 进 `group.*`（group_service.rs `group_rpc` 经 `serve_opened`），群和 Session ACL 再判；拒绝带 D-4 原因码。
- `membership.guest_entry.auth_methods {password, self_register, did_login}`、`group.describe_entry` 公开入口；专属邀请由 CP 以管理员会话调 `group.invite_session_guest`。
- Message Hub 访客页 `/messagehub/guest?group=&session=`（[GuestRoute.tsx](../src/frame/desktop/src/app/messagehub/guest/GuestRoute.tsx)）：只显示访客所在 Session，复用 `ConversationHistoryPane` / `ConversationComposer`。
- 群附件按 `context_path` 授权读取；新建访客账号不产生个人投递记录。
- 专属邀请的整套机制可被 App 代登录复用：记录结构 `ExclusiveInvite`（[hybrid_sso.rs](../src/kernel/buckyos-api/src/hybrid_sso.rs)）已预留 `InviteIssuer::App { app_instance_id }`；凭据只存摘要、版本化、`consumed_by_password`；`verify-hub.login_by_exclusive_invite`（[verify_hub main.rs](../src/kernel/verify_hub/src/main.rs) `handle_login_by_exclusive_invite`）签发受限 token pair、family 来源 `SessionSource::ExclusiveInvite`；受限 delegate 子 family 随父 family 失效；`app_availability` 判定已存在。

**本轮核对新发现的源码事实：**

1. **网关按路径转发服务，不看 host。** [boot_gateway.yaml](../src/rootfs/etc/boot_gateway.yaml) `get_service_info_from_req` 只匹配 `/kapi/<service>` 等路径即设 `SERVICE_ID`，在 `get_app_info_from_req` 之后、`check_oauth` 之前返回。因此 App 子域上的页面可以用自己 App 的 host-only cookie 会话直接调 `/kapi/msg-center`，msg-center 看到 `principal_kind=user`、`appid=<该 App>`。L2 面板与 L4A 自定义 UI 共用这一身份路径，不需要新的面板凭据。
2. **系统 SPA 的 appid 是 `control-panel`。** `AuthTarget::appid_claim()`（[app_identity.rs](../src/kernel/buckyos-api/src/app_identity.rs)）对 `System{service_id}` 返回 service_id。Message Hub、Desktop、AI Workspace 都在根域 `System{control-panel}` 会话下，`allowed_clients` 无法单独表达“Message Hub”；v2 §4.3 “Message Hub 也是一个 AppId”与 SDK 指南 §6.2 的示例目前不成立，`test_group_service.rs` 里的 `"messagehub"` 是假值。
3. **Hook 出站带 msg-center 自身的服务 token。** `run_group_hooks` 把 `get_buckyos_api_runtime().session_token` 以 `Bearer` 放进 `Authorization`，发往群主在配置里写的任意 `service` URL（只校验 http/https、无用户名密码、不跟随重定向）。`users` 组默认可建群，所以任何普通用户配一个指向自己服务器的 Hook 即可取得 msg-center 的服务身份。架构稿 §6.6 / §9.5 的“Hook 运行主体及其可访问数据”正是这一项。
4. **架构稿的“同源”在浏览器上不成立。** 面板由群主 Zone 的 App 子域提供，Message Hub 在根域；两者本来就不是浏览器同源。架构稿 §5.5 / §9.2 的“与群可信来源同源”只能落为协议层来源校验，SDK 指南 §4 末段已指出这一点。
5. **外部访客目前进不了 App。** 契约 §3.4 / §9.2：External 会话不能 delegate 到 App（v1），Limited 的 App 可用性默认拒绝、须经系统用户组开放。专属邀请会话也只签给 control-panel target（verify-hub 测试 `exclusive_invite_login_issues_restricted_sessions_for_control_panel_only`）。所以“客服应用用 L4A 自定义 UI 服务外部客户”当前走不通，这就是 SSO-10 的位置（D-5）。
6. **msg-center 对自托管 Zone 的解析缺陷仍在。** `network_directory.rs` 只认 `binded_zone_list`（混合 SSO TODO §11 发现③），面板来源校验若走同一解析，自托管 Zone 的群会误判。

## 1. 决策

D-1～D-6 由用户于 2026-10-24 拍板（原文见 §9）。D-7 是 D-5 选备选后新出现的协议决策，待拍板。实施中若需改变已定项，先回写本节再改任务。

| 编号 | 问题 | 已定 / 推荐 | 状态 | 影响 |
| --- | --- | --- | --- | --- |
| D-1 | L2 面板的上下文传递与认证 | 面板以 `<iframe>` 加载 `entry_url`，上下文走 URL query（`group_did`、原始 `session_id`、规范 `session` 地址、`locale`），会话切换时 Message Hub 换 `src`；面板用自己 App 的会话调 `/kapi/msg-center`，Message Hub 不向面板传任何凭据或 viewer 身份（v2 §14#5 已定）。`postMessage` 通道不在本批 | **已定（按推荐）** | EXT-02、EXT-03；SDK 指南 §4 |
| D-2 | 面板可信来源的判定基准 | **以 Self-host Group 的 owner 为基准**：`entry_url` 为 https，host 形如 `<app>.<host_zone>` 或 `<app>-<host_zone>`，`host_zone` 等于 Group DID Document 的 `host`（群主 Zone）；msg-center 在 `apply_config` 时校验并拒绝不符的声明，Message Hub 加载前再核一次 Doc。v1 不做跨来源接受 UX，外部来源直接拒绝。用户补注：**Agent 可以定制自己 Session 的侧边页，但那不是群扩展**，不走群配置；见 §2 范围说明 | **已定** | EXT-01、EXT-02、EXT-03；架构稿 §5.5 / §9.2 修订 |
| D-3 | Hook 出站凭据 | 每个 `HookBinding` 带 `secret`（配置写入时由 msg-center 生成、只回显一次），请求改为 `X-BuckyOS-Hook-Timestamp` + `X-BuckyOS-Hook-Signature: HMAC-SHA256(secret, timestamp.body)`；不再发送 msg-center 服务 token。App 侧用 secret 验签，不依赖 BuckyOS runtime | **已定（按推荐）** | EXT-05；SDK 指南 §5.3；`cyfs_dispatch.group_hook_authorization` 去除 |
| D-4 | 客户端身份粒度 | v1 接受 `control-panel` 代表全部系统 SPA；`allowed_clients.only` 写 `control-panel` 即“允许系统界面”，文档明说它同时放行 Desktop / AI Workspace 等根域页面。列已知限制 | **已定（按推荐）** | EXT-07；v2 §4.3、SDK 指南 §6.2 修订 |
| D-5 | 外部客户与 L4A 的关系 | **选备选：`login_by_app_access_token`（SSO-10）移入本批。** App 以自身身份为外部账号建立 / 复用 Limited 账号并取得登录凭据，外部客户在 App 子域页面兑换成指向该 App 实例的受限会话，再以本人身份调 `group.*`。Message Hub 访客页继续保留为无自定义 UI 时的默认入口 | **已定** | §2 阶段 D、EXT-07、EXT-09；SOM §5、§10；契约 §5～§7 |
| D-6 | Group Configuration 契约冻结时机 | 面板声明（EXT-01）与 `display` schema（EXT-04）落定后，再把 `schema_version=1` 定版并写进 v2 §3；在此之前仍按 0.7.0 允许 breaking，不写迁移 | **已定（按推荐）** | EXT-01、EXT-04；架构稿 §4.4、v2 §3.2 |
| D-7 | App access token 的验证协议（SOM §12 第二项；SSO-10 第一条要求“先选定验证契约，不能让 token 自报任意校验 URL”） | **已定（按推荐）：App 以服务身份向 CP 领取一次性登录码，复用专属邀请机制。** App 后端用自己的 runtime 服务 token 调 CP `user.app_login.issue {external_account_id, user_id?, resource?, return_path?}`：CP 校验调用者是 Zone 内已安装、owner 开启了“代登录”的 App 实例，为 `(app_instance_id, external_account_id)` 查找或新建 Limited 账号（不入系统组），写一条 `issuer.kind="app"` 的 `ExclusiveInvite`（凭据 32 字节随机、只存摘要、短有效期），返回 `{user_id, code}`；App 页面以 `code` 调 `verify-hub.login_by_app_access_token {code, nonce, target: App 实例}`，verify-hub 走 `login_by_exclusive_invite` 的判定表，签发 `auth_method=app_access_token`、target=该 App 实例的受限 token pair。不登记公钥、没有回调 URL，“app access token”就是这枚一次性 code。 | **已定（10-24 用户确认按推荐）**，兑换入口与推荐稍有不同：verify-hub 方法保持 CP-only，浏览器调 CP 公开方法 `auth.app_login.exchange`（见 §10 偏差 1） | EXT-09 |
| | | 备选 A：App 登记公钥（身份域登记 UI + `AppDoc` 扩展），token 为 App 自签 JWT，verify-hub 离线验签。多一套登记与轮换，App 侧要会签 JWT。 | | |
| | | 备选 B：安装时登记校验回调 URL，verify-hub 收到不透明 token 后回调 App 校验。回调地址来自安装登记而非 token，但 verify-hub 多一条出站依赖，且 App 不可用时登录不可用。 | | |

D-7 之下由实现者给出可评审结果、不需用户再拍板的工程契约：Limited 账号 `user_id` 命名（建议 `<app_id>-<base32(sha256(app_instance_id + external_account_id))[:12]>`）与 Profile 来源；`external_account_id` 只存摘要；code 有效期（建议 10 分钟、单次消费、按 nonce 幂等重试）；scopes 取 `msg_group` + `account_self`；代登录开关落在 App 的 `install_settings`；限流与审计字段；App 停用 / 卸载 / 关闭开关后 family 的撤销路径复用 `revoke_families`。

## 2. 交付顺序与范围

1. **阶段 A**：EXT-01 面板声明与校验、EXT-05 Hook 凭据。两者都改 `GroupConfiguration`，一次改完再测。
2. **阶段 B**：EXT-02 Message Hub 面板宿主、EXT-03 来源校验与故障隔离、EXT-04 `display` 渲染、EXT-06 Hook 的 UI 配套。
3. **阶段 C**：EXT-07 文档与 SDK 指南回写、EXT-08 DV 与样例应用。
4. **阶段 D（D-7 拍板后，可与 A / B 并行，改动在 CP / verify-hub / websdk）**：EXT-09 App 代登录。EXT-08 的样例应用在阶段 D 完成后补 L4A 外部客户用例。

**不在本批**：通用 Process Chain、其它 Hook 点、Hook 返回消息修改或持久化权限变更、跨 Zone 客户端身份证明、Joined Group 自动绑定、多副本与备份、面板 `postMessage` 通道、跨来源面板接受 UX、External（DID 登录）→App delegate。Session 邀请的独立版本与有效期（混合 SSO SSO-12 未完成项）不属于扩展架构，另行安排。

**Agent 会话侧边页（用户补注，不在本批）**：Agent 为自己的 Session 定制侧边页是 Agent 的能力，来源是 Agent 自身而非群配置，不受 D-2 约束。EXT-02 的面板宿主组件应当与“面板从哪来”解耦（面板列表由实体类型决定：群从 `group.list_sessions` 的生效面板取，Agent 将来从 Agent settings 取），避免届时重写加载、失败态和 Error Boundary。本批只做群来源。

## 3. A：配置契约

### EXT-01 / P0：面板声明

- [x] 在 `GroupConfiguration.extensions` 下定义 `panels: [{ id, title, entry_url, scope: "group" | { sessions: [sid] } }]`（v2 §7.2 `PanelDecl`），`validate` 校验：`id` 唯一且为 DNS 标签、`entry_url` 为 https 且 host 满足 D-2、`scope.sessions` 引用存在的 Session；`SessionRules.panels` 只能引用已声明的 `id`。
- [x] D-2 的 host 判定需要群 Doc 的 `host`；`apply_config` 在事务内已有 `GroupState.host`，直接比对，不走 `network_directory` 解析。
- [x] `group.get_doc` 公开摘要不含面板；`group.list_sessions` 返回读者可见 Session 的生效面板列表（Session 规则引用 ∪ `scope=group`）。
- [x] 面板上限写入 `GroupLimits`（建议每群 ≤ 8），超限 `invalid-panel-decl`。
- [x] `required_features` 增加 `panels`。
- [x] 单测：声明合法 / 非法 host / 引用不存在的 id / 超限；`apply_config` patch 整体替换 `extensions.panels` 的行为写进后台接口文档。

### EXT-05 / P0：Hook 出站凭据（按 D-3）

- [x] `HookBinding` 增加 `secret`：`apply_config` 写入时若为空由服务生成 32 字节随机值；`get_config` 对非 Owner 隐去，Owner 也只在写入响应里看到一次；提供 `group.rotate_hook_secret {session_id?, index}`。
- [x] 出站请求去掉 `Authorization: Bearer <msg-center token>`，改为 `X-BuckyOS-Hook-Timestamp` 与 `X-BuckyOS-Hook-Signature`（HMAC-SHA256 over `timestamp + "." + body`）；body 增加 `group_host` 供 App 侧区分来源 Zone。
- [x] 删除 `cyfs_dispatch.group_hook_authorization` 及其“无凭据即不可用”分支。
- [x] 单测：签名正确 / 错误 / 时间戳偏差超 5 分钟；现有 `deny` / `native_only` / `may_grant` / 幂等用例保持通过。
- [x] SDK 指南 §5.3 给出 Deno 验签样例。

## 4. B：Message Hub 与 L1 展示

### EXT-02 / P0：面板宿主

- [x] 在 `GroupPanel` / `SessionDetails` 旁增加面板区（桌面右侧栏 Tab，手机进入二级页），按 `group.list_sessions` 返回的生效面板列出 Tab，默认折叠。宿主组件接收抽象的面板列表，不直接依赖群 store（见 §2 Agent 侧边页说明）。
- [x] 以 `<iframe sandbox="allow-scripts allow-same-origin allow-forms" referrerpolicy="origin">` 加载 `entry_url?group_did=&session_id=&session=&locale=`；切换 Session 时按 D-1 换 `src`。不向面板传 token、viewer DID、cookie。
- [x] 加载失败（onerror、超时 10 秒无 `load`）显示“面板不可用”并提供重试；面板区崩溃不影响会话列表和消息视图（Error Boundary 包住面板区）。
- [x] 访客页 `/messagehub/guest` 同样显示访客所在 Session 的面板。
- [x] mock 模式提供一个本地面板样例页；Playwright 用例覆盖显示、切换、失败态。
- [ ] 真实浏览器验证：面板 App 子域在 iframe 内无会话时的 `check_oauth` 302 与桌面免密 `prepare_app_launch` 行为；iframe 内 host-only cookie 的送达。结论写进 SDK 指南 §4。（10-24 未做：SDK 指南 §4 按网关规则写了预期，标明未复核）

### EXT-03 / P0：来源校验与故障隔离

- [x] Message Hub 加载前核对 `entry_url` host 与群 Doc 的 `host`（D-2），不符不加载并提示“来源不受信”；msg-center 已拒绝的声明不应出现，此处是第二道。
- [x] 自托管 Zone（owner DID = Zone DID）的 host 解析不能走 `binded_zone_list`，复用 `buckyos_api::owner_canonical_zone`；同时修 `network_directory.rs` 的同类问题或在 TODO 里登记。
- [ ] 验证“面板服务停掉，聊天照常”：DV 里关掉面板 App 容器，Message Hub 仍能收发。（10-24 未做：Playwright mock 覆盖了失败态下聊天照常；真实容器未验）

### EXT-04 / P1：`display` 渲染

- [x] 定 `display` schema：`links: [{title, url}]`、`files: [{title, obj_id}]`、`featured_members: [did]`、`announcement`（群级与 Session 级各一份，Session 覆盖群）。`validate` 校验 url 为 https、obj_id 合法、成员属于群。
- [x] GroupPanel / SessionDetails 渲染上述四项；`files` 经现有对象访问层按 `context_path` 读取；`featured_members` 只是展示标记，不授予任何能力（架构稿 §9.5）。
- [x] 按 D-6 在此之后定版 `schema_version=1`。

### EXT-06 / P1：Hook 的 UI 配套

- [x] `group.check_access(action: "session.post")` 对配置了 `may_grant` 的 Session 返回 `allowed: "hook_dependent"`，Message Hub 据此不禁用输入框，提交被 `hook-denied` 拒绝时在气泡上提示。
- [x] 群信息页展示“本群绑定了 N 个业务 Hook（join / post / read）”与各 Hook 的 service host，`post` Hook 另加“业务服务可读取消息内容”说明（v2 §7.3 可见性要求）。

## 5. C：身份与访问路径

### EXT-07 / P1：客户端身份与文档（按 D-4、D-5）

- [x] 文档层面把 `allowed_clients.only` 的值定义为 token `appid`，系统界面统一为 `control-panel`；`test_group_service.rs` 的 `"messagehub"` 改为真实值。
- [x] SDK 指南 §6 改写：外部客户两条路径（Message Hub 访客页；App 经 EXT-09 代登录后用自定义 UI），`allowed_clients.only=[<app>]` 加 `control-panel` 的含义；§4 从“待实现”改为实现后的 wire。
- [x] 契约 §13 已知风险补：`allowed_clients` 不能区分系统 SPA；External（DID 登录）仍不能进 App。

## 6. D：App 代登录（SSO-10，按 D-5 / D-7）

### EXT-09 / P0：`login_by_app_access_token`

继承混合 SSO TODO SSO-10 的六条要求与验收；下列任务按 D-7 推荐方案拆分，D-7 改选备选时只替换前两条。

- [x] **身份域与开关**：App 的 `install_settings` 增加 `app_login: { enabled: bool }`，由该 App 实例 owner 在 CP 开关；CP 校验调用者 token 是 `principal_kind=app`、`appid` 对应已安装实例、开关打开，否则 `zone_method_disabled` / `source_revoked`。
- [x] **签发**：CP `user.app_login.issue {external_account_id, user_id?, resource?, return_path?}`：按 `(app_instance_id, sha256(external_account_id))` 查绑定表（system-config `services/control_panel/app_identities/<app_instance_id>/<digest>` → `user_id`），无则创建 Limited 账号（不入系统组、不写 default zone、`password_state=unset`），写绑定；写 `ExclusiveInvite` 记录 `issuer: App { app_instance_id }`、`credential` 版本化摘要、有效期 10 分钟、`resource` 可选；返回 `{user_id, code, expires_at}`。`user_id` 显式传入时必须已绑定到同一 `(app_instance_id, external_account_id)`，不得指定任意账号（SSO-10 第二条）。
- [x] **兑换**：verify-hub `login_by_app_access_token {code, nonce, target}`（公开方法，限流）：按 §5.3 判定表处理不存在 / 撤销 / 账号状态 / 已设密码 / 版本 / 过期；`target` 必须是签发该 code 的 App 实例（`app_availability` 对 Limited 开放此实例）；签发 `auth_method=app_access_token`、`restricted=true`、`scopes=[msg_group, account_self]` 的 token pair，family 来源 `SessionSource::AppLogin { app_instance_id, invite_id, version }`；code 单次消费、同 nonce 幂等重试返回原结果、换 nonce 拒绝。
- [x] **会话上下文**：`AuthMethod` 增加 `AppAccessToken`；`CurrentIdentity` 带 `auth_method`；`refresh` 重读绑定与开关，开关关闭或 App 停用 / 卸载返回 `source_revoked`，复用 `revoke_families`。受限 delegate 维持契约 §3.4：App 后端用自己的服务身份调系统服务，不转发客户会话。
- [x] **浏览器流程**：App 页面调 websdk 新增 `loginByAppAccessToken({code, nonce})`，成功后 cookie 落在 App 子域（host-only，与现有 callback 一致）；`return_path` 由 App 自定。页面无 BuckyOS 会话时必须可达：核对 App 子域 `check_oauth` 对该入口路径的放行方式（公开路径登记还是 App 自行承载登录页）。
- [x] **msg-center 侧**：该账号以 Limited 进 `group.*`，不需要新分支；`accept_session_invitation` / `submit_guest_request` 由 App 在签发时经 `resource` 预置邀请（CP 以 App owner 会话调 `group.invite_session_guest`，与专属邀请 `retry_resource` 同一路径）。
- [x] **限流与审计**：签发按 App 实例每分钟上限、兑换按来源 IP 上限；记录签发 / 兑换 / 撤销事件，不存 code 明文与 `external_account_id` 原文。
- [x] **测试**：verify-hub 单测（判定表、target 不符、重放、开关关闭后 refresh 拒绝）；CP 单测（跨 owner 同 AppId 不能互管、`user_id` 指定越权拒绝、重复签发复用账号）；websdk jest；msg-center 断言新账号无个人投递记录。
- [x] **验收（继承 SSO-10）**：跨 owner 同 AppId 冒认、跨身份域同账号、伪造主体、错误 target、重复消费、来源停用和旧会话提权均失败；Message Hub 不增加自己的兑换中心或用户凭据库。（10-24：verify-hub / CP 单测覆盖；真实 Zone 未验）

### EXT-08 / P0：DV 与样例应用

- [x] 写一个最小样例 App（`test/test_msg_center/group_ext_sample/`，Deno 静态页 + Hook 服务 + 代登录后端）：L1 脚本建“客服群”并写入面板与 `display`，面板页读 `group.list_messages` 显示工单摘要，Hook 服务验签后按白名单放行 `join`，代登录后端为模拟 CRM 账号领取 code 并在自定义页面展示工单 Session。
- [x] DV 脚本：建群 → 面板在 Message Hub 与访客页显示 → 停面板容器聊天照常 → Hook 验签失败视为不可用、`decision: deny` 不被放行 → 外部客户经专属邀请进访客页看到面板 → 外部客户经 App 代登录在 App 子域看到同一 Session 并发言 → 关闭 App 的 `app_login` 后 refresh 被拒。
- [ ] 真实 Zone 验证 msg-center 容器能访问 App 子域 URL（容器网络、自签证书）；记录需要的网关或 hosts 处理。（10-24 未做）

## 7. 验收矩阵

| 场景（架构稿 §12.1） | 应验证结果 |
| --- | --- |
| L1 生成器退出 | 群与面板声明保持；日常收发不调用生成器 |
| 面板服务不可用 | 面板显示不可用，消息视图与发送正常；访客页同样 |
| 外部来源面板被写进配置 | `apply_config` 拒绝；手工改库后 Message Hub 第二道拒绝加载 |
| Hook 正常拒绝 / 超时 / 验签失败 | `deny` 终止；超时与验签失败按 `on_unavailable`；`native_only` 下原生权限继续；出站请求不含 msg-center 服务 token |
| 配了 `may_grant` 的 Session | 输入框不禁用，提交结果以服务为准 |
| `allowed_clients.only=["<app>"]` | 该 App 子域页面可访问，根域 Message Hub 被拒；加 `control-panel` 后系统界面放行 |
| 外部客户（Limited / External）经访客页 | 可见面板与 `display`；拿不到 viewer 身份 |
| 外部客户经 App 代登录 | 只能进签发 code 的 App 实例；在该 App 内以本人 Limited 身份读写被邀请的 Session；换设备重新领取 code 映射同一账号；App 关闭代登录后旧会话 ≤15 分钟失效、refresh 拒绝 |
| 自定义 UI 限制通用客户端访问 | 限制落实在 `allowed_clients`，不只是隐藏 Message Hub 入口 |

## 8. 文档联动

- 架构稿：§5.5 / §9.2 “同源”改为 D-2 的来源校验；§7.2 补 App 代登录是“受认可的应用访问路径”的实现；§12.2 表增加状态列（术语、Group Configuration、Hook、协议互操作已由 v2 §3 / §7 / §11 回答，面板、应用身份、权限语义由本 TODO 处理）；文首加“实施状态见本 TODO”。
- v2：§3.2 增加 `extensions.panels` 与 `display` schema；§4.3 按 D-4 修正；§7.2 / §7.3 按 EXT-01 / EXT-05 回写；§14 关闭 #5，#3 保持开放。
- SDK 指南：核对基线更新到实施后 commit；§4 改为实现说明；§5.3 换成签名验签；§6 增加代登录流程与 `control-panel` 语义；补混合 SSO 之后的访客入口。
- 后台接口文档：`apply_config` 的面板 / display / hook secret 字段、`rotate_hook_secret`、错误码。
- 契约：§2 主体表加 `auth_method=app_access_token`；§5 增加 App 签发的邀请记录与 `login_by_app_access_token` 判定；§6 撤销矩阵加“App 代登录关闭 / 卸载”行；§7 family 来源加 `AppLogin`；§12 持久状态加 `app_identities` 绑定表；§13 按 EXT-07 补已知风险。
- SOM：§5 改为“已实施，契约见混合 SSO 契约 §5”，§10 第 3 项与 §12 前两项标注 D-7 结论。
- 混合 SSO TODO：SSO-10 条目标注“2026-10-24 移入 self-host-group-extension-todo EXT-09”。

## 9. 决策回写记录（2026-10-24）

用户在 §1 表格下逐项批注，原文：D-1 “按推荐”；D-2 “这个主要是看 self-host-group 的 owner，不过有一个功能是 agent 可以定制他自己的 session 的侧边页（但这个不算是 group 扩展）”；D-3 “按推荐”；D-4 “按推荐”；D-5 “login_by_app_access_token”；D-6 “按推荐”。

据此改动：D-2 的判定基准明确为群主 Zone，并把 Agent 会话侧边页记为范围外的相关能力（§2）；D-5 改为选备选，SSO-10 从“不在本批”移到阶段 D，新增 EXT-09 与 D-7；§7 验收与 §8 文档联动随之增加代登录条目；EXT-08 样例应用增加代登录后端。

同日稍后：D-7 用户确认按推荐（§10 偏差 1）。§11 官方例子提出后，用户对 D-8 批注“按推荐，开始实施 EXT-11 和 EXT-10”，实施记录见 §11.6。

## 10. 实施记录（2026-10-24）

**范围与结论。** 阶段 A～D 的代码与 §8 的文档回写已完成，均未提交；Rust 单测、websdk jest、Message Hub Playwright（mock）通过。用户于同日确认 D-7 后，重建 msg-center / control-panel / verify-hub / Desktop，`start.py` 重启本机单 Zone（test.buckyos.io，保留数据）并完成 DV：

- `test/test_msg_center/group_ext_sample/dv.ts` 全部通过（§1 面板声明 / display / secret 隐藏 / 外部来源拒绝 / `hook_dependent`；§2 HMAC 验签、业务拒绝、轮换后旧 secret 不可用；§3 专属邀请客户看到面板与 display；§4 App 代登录：以 `buckyos-systest@devtest` 的 App 服务会话签 code、`auth.app_login.exchange` 得到 App 子域 callback、跟随 callback 取到客户 cookie、`auth.current_identity` 为 `app_access_token` 受限会话、能进被邀请 Session、`user.list` 被拒、同 nonce 重试 / 换 nonce 拒绝、关开关后签发被拒）。App 服务会话由 OOD 设备私钥签登录断言经 `login_by_jwt` 取得（容器 env 里那枚断言已被 App 消费）。
- `src/frame/desktop/tests/dv/group-extensions.spec.ts`（`playwright.dv.config.ts`）：真实浏览器里群详情 / Session 详情 / 访客页加载 App 子域面板（以已装的 `buckyos-systest` 子域充当面板来源，iframe 里渲染出该 App 首页，`src` 只带 `group_did` / `session` / `session_id` / `locale`）、网络层不可达的面板显示“不可用”而同一 Session 照常发言、群级与 Session 级 display 渲染、访客页侧栏同样显示 display 与两个面板。**通过**（截图 `test-results/dv-group-ext-{panel,session-dead-panel,guest}.png`）；修掉的问题见下。

**主要落点**

| 部分 | 位置 |
| --- | --- |
| 共享 | `buckyos-api/src/group_hook.rs`（HMAC 签名 / 验签、随机 secret、头名）；`hybrid_sso.rs`（`AuthMethod::AppAccessToken`、`SessionSource::AppLogin`、`InviteCredentialState::Consumed`、`APP_LOGIN_*` 键与 `AppLoginSettings` / `AppIdentityBinding`、`app_login_code`）；`verify_hub_client.rs`（`AppAccessTokenLoginRequest`、`RevokeFamiliesRequest.source_app_instance_id`、`login_by_app_access_token` 客户端与 trait） |
| msg-center（A） | `group_types.rs`：`PanelDecl` / `PanelScope` / `GroupExtensions` / `DisplayConfig`、`HookBinding.secret`、`GroupLimits.max_panels`、`validate(host)`、`validate_panels`、`panel_host_trusted`、`fill_hook_secrets` / `redact_hook_secrets`、`GroupState::validate_config_refs` / `session_panels`；`group_service.rs`：create / apply_config / create_session / update_session 生成 secret 并只回显一次（`hook_secrets`）、`group.rotate_hook_secret`、`get_config` / `list_sessions` / 变更事件去 secret、`list_sessions` 带 `panels` / `display`、`run_group_hooks` 改签名出站、`check_access` 的 `hook_dependent`；`cyfs_dispatch.rs` 删除 `group_hook_authorization`；`doc/group-v2-backend.md` |
| Message Hub（B） | `src/app/messagehub/extensions/`（`model.ts` 宿主无关的面板 / display / hook 模型与 D-2 判定、`GroupExtensionPanels.tsx` iframe 宿主 + Error Boundary + 手机二级页、`GroupDisplaySection.tsx`）；`GroupPanel.tsx`、`SessionDetails.tsx`、`guest/*` 接入；`api/store.ts`（`hostDid`、`hook_dependent`、`hook-denied` 气泡）；mock（`public/messagehub/mock-panel.html`、`mock/store.ts`、`mock/guest.ts`）；`tests/e2e/pages/messagehub-extensions.spec.ts` |
| CP（D） | `app_login.rs`：`user.app_login.issue`、`auth.app_login.exchange`（公开）、`apps.app_login.get/set`、`AppLoginLimiter`、绑定表与账号命名；`RpcAuthPrincipal.principal_kind`；`exclusive_invite.rs` 复用项改 `pub(crate)`、`issue_credential_with_ttl` |
| verify-hub（D） | `handle_login_by_app_access_token`（判定表 + 内存消费表 `AppLoginCodes` 按 nonce 幂等）、`recheck_principal` 对 `AppLogin` 来源跳过可用性策略改查开关与运行态、`revoke_request_matches`；`directory.rs` 新增 `ZoneDirectory::app_login_available`；`family_store.rs` `FamilyClaims::restricted` |
| websdk | `src/app_login.ts`（`issueAppLoginCode`、`loginByAppAccessToken`、`completeAppAccessTokenLogin`）、`hybrid_sso.ts` `AuthMethod` 加值、`tests/app_login.test.ts` |
| 样例与 DV | `test/test_msg_center/group_ext_sample/`（README、`setup_group.ts`、`hook_server.ts`、`panel/index.html`、`app_backend.ts`、`dv.ts`） |
| 文档 | 架构稿（文首状态、§5.5、§7.2、§9.2、§9.3、§12.2 状态列）、v2（§3.2、§4.3、§7.2、§7.3、§14#5）、SDK 指南（文首、§1、§3.2、§4、§5.2～5.4、§6.2、§6.4、§8）、后台接口文档、契约（§2.1、§3.1、§5.6、§6、§7、§12、§13）、SOM（状态、§5、§10、§12）、混合 SSO TODO SSO-10 |

**验证**

- `cargo test -p buckyos-api -p verify_hub -p control_panel -p msg_center -- --test-threads=1`：buckyos-api 288、verify-hub 37、msg-center 181（+1 ignored）全部通过；control-panel 198 通过、1 失败为既有的 `sso_identity::tests::account_identity_carries_ceiling_method_and_password_state`（AICC 额度加入 `aicc` scope 后 `current_identity_account.json` fixture 未更新，与本批无关，未改）。`cargo check --workspace --tests` 通过。
- websdk：`npx jest tests/app_login.test.ts tests/hybrid_sso` 通过。
- Desktop：`pnpm check` 通过；`pnpm lint` 仅既有的 6 个 messagehub 之外的错误；`npx playwright test messagehub-extensions.spec.ts messagehub-group.spec.ts messagehub-guest.spec.ts messagehub.spec.ts` 54 通过、3 失败均在 `messagehub.spec.ts` 且与本批无关（两例重跑通过，一例是 Playwright 50MB buffer 限制）。
- `deno check` 四个样例脚本通过；DV 未在真实 Zone 执行。

**与原任务的偏差**

1. **D-7 兑换入口**：原文把 verify-hub `login_by_app_access_token` 写成“公开方法，限流”。实现保持 verify-hub 方法 CP-only（与 `login_by_exclusive_invite` 一致，单闸口原则），浏览器调 CP 公开方法 `auth.app_login.exchange {code, nonce, return_path?}`，CP 代调 verify-hub、复用 pending nonce → `/sso_callback` 让 cookie 落到 App 子域，并把记录 `credential.state` 置为 `consumed`。限流在 CP（兑换按来源 IP 30/分钟，签发按 App 实例 60/分钟）。D-7 由用户于 10-24 确认按推荐。
2. **开关存放处**：任务写“落在 App 的 `install_settings`”，但代码里没有按 App 实例的 install_settings（只有 Zone 级 `system/install_settings`）。改为独立键 `services/control_panel/app_login/<app_instance_id>`，RPC `apps.app_login.get/set`。
3. **账号命名**：`<app label>-<16 hex of sha256(digest)>`（建议中的 base32 改为 hex，避免新依赖）。
4. **Hook secret 作用面**：除 `default_session` / 模板外，Session 的 `rule_overrides.hooks` 也会生成 secret（`create_session` / `update_session` 回显 `scope: "session:<sid>"`），`rotate_hook_secret {session_id}` 先找覆盖项、再找模板。
5. **HMAC 实现**：为不引入 `hmac` crate，`buckyos-api::group_hook` 用 `sha2` 手写 HMAC-SHA256（RFC 4231 用例单测）。
6. **`display.files`**：Message Hub 以 viewer 的对象访问层按 `obj_id` 打开，配置级文件没有消息 `context_path`，真实 host 可能拒绝（UI 显示“不可用”）；按 context_path 读取配置级文件的授权缺口待定。
7. **CP 受限闸口**：`restricted_method_access` 原本只放 CP（根域）受限会话；App 签入的会话 target 是 App 实例，DV 时 `auth.current_identity` 被拒 `session_kind_not_allowed`，已把 `auth_method=app_access_token` 且 target 为 App 的用户会话视同自助会话（契约 §5.6 已注）。
8. **App 服务会话在群里的主体**：msg-center 原来对 `principal_kind=app` 不解析 DID（只认 `principal_dids` 映射），CP 以 App 身份预置 Session 邀请时得 `not-found`。改为 App 服务会话在 `group.*` 中以 owner DID 行事、`appid` 作客户端（`group_actor`），后台接口文档已注。
9. **面板可达性探测**：Chromium 对跨源 iframe 的网络错误页也触发 `load`，`onerror` / 超时判不出“服务停了”；宿主加载前先 `fetch(entry_url, {mode:'no-cors'})` 探测（注意 no-cors 不能配 `redirect:'manual'`），失败即“面板不可用”。
10. **`hook_secrets` 仅在生成时出现**：写操作结果里没有生成新 secret 时不带该字段（`submit_guest_request` 的幂等比较要求结果形状稳定）。

**DV 修掉的问题（10-24）**

1. CP 受限闸口不认 App target 的受限会话（§偏差 7）。
2. msg-center 不解析 App 服务会话的主体，App 预置 Session 邀请 `not-found`（§偏差 8）。
3. 面板 iframe 对“服务不存在”的主机也报 `load`，宿主误判为就绪（§偏差 9，加 no-cors 探测）。
4. `auth.app_login.exchange` 原按 `app_id.<zone>` 拼 App 来源，实际子域是 App 的 `sub_hostname`（如 `buckyos-systest`），改为从 `node_gateway_info.json` 反查。
5. DV 脚本里 bob 的口令与种子不符，改为用 sudo 新建成员账号；真实 Zone UI 语言是中文，Playwright 用 `localStorage` 固定 `en` 并用中英双语正则选择。

**遗留 / 未验证**

- 宿主对面板只能识别网络层失败：真实 Zone 的网关对不存在的 App 子域返回空 403（DV 截图里是空白帧），停掉的容器会是 5xx 页，都在 iframe 内原样显示，不会变成“面板不可用”；要判 HTTP 层故障需要面板 `postMessage` 就绪握手（不在本批）。Playwright DV 用 `route.abort` 制造网络层失败来验不可用态。
- 面板的真实业务页仍未以独立 App 装到子域（DV 用 systest App 的首页充当面板，验证的是加载 / 失败 / 上下文传递，不是面板内容）；App 子域 iframe 内 `check_oauth` 302 与 host-only cookie 送达、停容器（而非不存在的主机）后的聊天照常、代登录会话关闭开关后浏览器侧的 refresh 拒绝未在真实环境验证。
- `panels` 的 `scope.sessions` 只能引用具名 Session；默认 Session 靠 `scope=group`。
- `network_directory.rs` 对自托管 Zone 的解析缺陷（§0 发现 ⑥）未改：Message Hub 侧已按 `owner_canonical_zone` 规则实现 host 解析（`zoneHostOfDid`），msg-center 侧 `apply_config` 用事务内 `GroupState.host` 不经过它。
- `allowed_clients` 仍不能区分系统 SPA（D-4 已知限制）。

## 11. 官方例子：sys-test 客服群用例（2026-10-24 新增）

用户要求（原文）：“在 sys-test 中增加一个用例，1）点击按钮创建群聊，包含 alice,bob + 客户，并为该群聊创建一个自定义页面；2）产生一个链接发给客户，客户点击该链接后，能以客户的身份加入群聊。客户如果不修改密码，链接 48 小时有效。因此客户点链接进来的时候会提示他‘设置密码’还是跳过，直接使用。修改密码后点击链接后，需要先输入密码才能进入聊天。”

这是本 TODO 交付能力的**官方例子**：一个真正安装在 Zone 里的 App（`buckyos-systest`，[src/apps/sys_test](../src/apps/sys_test/)）用“L1 配置 + L2 面板 + 客户链接”服务外部客户。它取代 `group_ext_sample` 里只能用脚本跑的部分成为开发者照抄的范本；`group_ext_sample` 保留 Hook（L3）与 App 代登录 code（L4A）两件 sys-test 不覆盖的事。

### 11.1 用例流程（以用户视角）

前置：Zone 已装 `buckyos-systest`，点按钮的人是该 App 实例的 owner（devtest），alice、bob 是本 Zone 用户。sys_test 页面新增一组 **`support_group`（“客服群与客户链接”）**，三个按钮：**创建客服群**、**生成客户链接**、**清理**。

1. **创建客服群**（页面，用户会话）。页面以当前用户在 App 子域的会话调 msg-center `group.create`：群名“客服群（sys-test）”，邀请 alice、bob（同 Zone 自动接受）；配置复用 `group_ext_sample/common.ts` 的 `supportGroupConfig` 骨架去掉 Hook：`required_features=[sessions, session_guests, panels]`，`default_session` 用 `support` 模板（`allow_guests=true`、`post=all_participants`、`rules.panels=["tickets"]`），`display.announcement`，`extensions.panels=[{id:"tickets", title:"工单", entry_url:"https://buckyos-systest.<zone>/support/panel", scope:"group"}]`。**“自定义页面”就是这块面板**：由 sys_test 后台托管 `GET /support/panel`（照 `group_ext_sample/panel/index.html` 写，读 `group_did` / `session_id` / `session` / `locale` 四个 query），内容从 sys_test 后台 `GET /sdk/appservice/support/summary?group_did=&session_id=` 取：后台以 App 服务会话（按 §10 偏差 8 以 owner DID 行事）调 `group.list_messages`，只返回条数、最近一条时间与参与者数，不返回正文。页面展示群 DID、成员状态与 Message Hub 深链。
2. **生成客户链接**（页面 → 后台，App 服务会话）。页面输入客户称呼与外部标识（如 `crm:1001`），后台调 CP 签发一条 **48 小时专属邀请链接**（机制见 §11.3 D-8），`resource={kind:"group_session", group_did, session_id:<默认 Session>, title:"客服工单"}`，`return_path=/messagehub/guest?group=<did>&session=<sid>`；CP 以 App 身份 `group.invite_session_guest` 预置 Session 邀请（§6 EXT-09 已有路径）。链接只在本次响应出现一次，页面用现有 `OneTimeLink` 的方式展示并可复制；同一外部标识再次生成复用同一 Limited 账号（绑定表）。至此“alice、bob + 客户”齐了：alice、bob 是群成员，客户是该 Session 的 Session Only Member（v2 §4、SOM），不进群成员表，这是设计使然。
3. **客户点链接（未设密码）**。`https://<zone>/sso/invite/<id>#s=<secret>` → Desktop 公开路由 `InvitePage` → `auth.invite.login` → 受限会话（target control-panel，`auth_method=exclusive_invite`）→ 回 `return_path` 的访客页 `/messagehub/guest`：顶部出现现有的非阻塞 `FirstPasswordPrompt`（“你的账号还没有密码，设置一个…” + **设置密码** 按钮；不点就是“跳过，直接使用”，不弹阻塞对话框）；页面进入 `invited` 态，点“加入”接受 Session 邀请后聊天，侧栏显示 `display` 与“工单”面板。48 小时内再点链接都能直接进；过期后 InvitePage 显示 `expired`。
4. **客户设置密码后再点链接**。`auth.invite.set_password` 把记录置 `consumed_by_password`，其它免密 family 被撤销；再点链接 → 判定表 `password_required` → InvitePage 显示“用密码登录”按钮 → `/login?redirect_url=<return_path>` → 密码登录（Limited 普通会话）→ 回到同一访客页继续聊天。alice、bob 在 Message Hub 群详情里看到同一面板与客户的发言。
5. **清理**：删群、撤销该客户的专属邀请记录（账号保留，重复运行复用）。

### 11.2 现状对照

| 流程 | 现状 | 缺口 |
| --- | --- | --- |
| 建群 + 面板声明 + Session 规则（步骤 1） | msg-center `group.create` / `PanelDecl` / `display` 已实现（EXT-01 / 04）；App 子域用户会话可直接调 `/kapi/msg-center`（§0 发现 1）；`buckyos-systest` 的 `www` 端点 `allow_guest: true`，面板页在 iframe 内无会话也能加载 | sys_test 无此页面与后台路由；面板内容只能由后台以 App 服务身份取（面板页上没有 viewer 身份，也拿不到根域 cookie） |
| 面板在 Message Hub 与访客页显示（步骤 3、4） | EXT-02 / 03 已实现，访客页同样显示；DV 已用 `buckyos-systest` 子域充当面板来源（§10） | 无 |
| 48 小时链接、设密码提示、设密后要密码（步骤 3、4） | 专属邀请全套已实现：`EXCLUSIVE_INVITE_TTL_SECS = 48h`、`InvitePage`、`FirstPasswordPrompt`、`password_required` → “用密码登录”、`consumed_by_password` + `revoke_families`（契约 §5.2～5.4） | **签发方**：`user.exclusive_invite.create` 要求 `require_control_panel_admin`（CP target 的管理员用户会话）。sys_test 页面是 App target 的用户会话、后台是 App 服务会话，都不满足。App 现有的 `user.app_login.issue` 签的是 10 分钟一次性 code、兑换后 `consumed`、cookie 落 App 子域、不走 InvitePage，三条语义都不符 |
| 预置 Session 邀请 | `user.app_login.issue` 带 `resource` 时 CP 以 App 身份调 `group.invite_session_guest`（§10 偏差 8） | 复用即可 |
| 开关 | `apps.app_login.set` 要求 owner 的 CP 用户会话（`is_control_panel_session`） | Desktop 目前没有这个开关的 UI（grep 无消费方）；sys_test 页面在 App 子域也调不了。用例前置里要么由 DV 脚本以管理员会话打开，要么补 Users and Agents 的开关 UI（见 EXT-11） |
| 重发 | `resend_decision` 对非 `InviteIssuer::User` 的记录返回 `not_applicable`；通知走 `to_user=issuer.user_id` | App 签发的链接过期后客户不能申请重发；例子里由 sys_test“重新生成链接”代替（同外部标识 → 同账号、新记录） |
| 撤销 | `revoke_request_matches` 的 `source_app_instance_id` 只匹配 `SessionSource::AppLogin` | App 关闭开关 / 卸载时，经 App 签发链接登录的 `ExclusiveInvite` family 不会被一并撤销 |

### 11.3 决策 D-8（已定，2026-10-24 用户“按推荐”）：App 如何签发 48 小时客户链接

| 编号 | 问题 | 推荐 | 影响 |
| --- | --- | --- | --- |
| D-8 | sys_test 这类 App 没有 CP 管理员会话，谁来签 48 小时专属邀请链接 | **已定（按推荐）：`user.app_login.issue` 增加 `kind: "code" \| "link"`（缺省 `code`，现有行为不变）。`link`：有效期 `EXCLUSIVE_INVITE_TTL_SECS`（48h）、记录与 code 同结构（`issuer.kind="app"`、`channel.kind="app"`、绑定表、`resource`、限流、审计都复用），返回 `{user_id, link: "https://<zone>/sso/invite/<id>#s=<secret>", expires_at, new_account}`，不返回 code。客户走现有 `/sso/invite` 路径：CP `auth.invite.login` → verify-hub `login_by_exclusive_invite`（已核实不按签发方过滤，判定表原样适用，target 为 control-panel），设密码 / 过期 / 替换语义全部继承；兑换不置 `consumed`。开关、停用、卸载仍以 `app_login` 为准：`recheck_principal` 对 `issuer.kind="app"` 的 `ExclusiveInvite` family 也查开关与 App 运行态，`revoke_request_matches` 的 `source_app_instance_id` 对这类 family 比对记录的 `issuer.app_instance_id`（需要 family 来源带 `app_instance_id`，或 verify-hub 撤销时回读记录）。** | EXT-11；契约 §5.6 增加 `kind=link` 一行与判定差异；SDK 指南 §6 的 L4A 之外多一条“App 签发根域客户链接”路径 |
| | | 备选 A：用例由管理员会话驱动（Deno 脚本登录 devtest 调 `user.exclusive_invite.create`），sys_test 页面只展示。不满足“点击按钮”，而且例子教的是“开发者得写脚本”，不是 App 能力 | |
| | | 备选 B：sys_test 页面让用户再输一次管理员口令直接向 verify-hub 换 CP 会话。App 页面持有管理员口令，违反契约 §3.4 的 App 边界，不采用 | |
| | | 备选 C：沿用 10 分钟 code（现状）。48 小时、设密码提示、设密后要密码三条都不成立 | |

D-8 之下由实现者定、不需拍板：`link` 模式是否允许 `return_path` 为空（缺省到访客页）；同一外部标识 48 小时内重复签 `link` 是新记录还是 rotate（建议新记录，旧记录 `superseded`，与 CP 的 rotate 同义）；`auth.invite.request_resend` 对 App 记录保持 `not_applicable`，由 App 自己重签。

### 11.4 任务

#### EXT-11 / P0：App 签发 48 小时客户链接（按 D-8）

- [x] CP `app_login.rs`：`user.app_login.issue` 增加 `kind`；`link` 用 `issue_credential_with_ttl(1, now, EXCLUSIVE_INVITE_TTL_SECS)`，响应复用 `exclusive_invite.rs` 的 `invite_link()` 拼链接；`auth.app_login.exchange` 对 `link` 记录拒绝（`entry_unavailable` + `target_mismatch`，code 与 link 不互换）；`auth.invite.describe` / `auth.invite.login` 对 App 记录照常。
- [x] verify-hub：`recheck_principal` 对来源为 `ExclusiveInvite` 且记录 `issuer.kind="app"` 的 family 查开关与 App 运行态（复用 `app_login_available` / `check_app_runtime_session`）；`revoke_request_matches` 让 `source_app_instance_id` 命中这类 family。CP 关闭开关时的 `revoke_families` 路径不变。
- [x] 单测：`link` 的 TTL 为 48h、不被 exchange 消费、设密码后 `password_required`、关开关后 refresh 拒绝；现有 code 用例不变。
- [x] websdk `app_login.ts`：`issueAppLoginCode` 加 `kind` 参数（或新增 `issueAppCustomerLink`），返回类型区分 `code` / `link`。
- [x] Users and Agents → Apps 增加“允许该 App 为外部客户签发登录”开关（调 `apps.app_login.get/set`），否则例子的前置只能靠脚本。（落在 App Service 应用详情页，见 §11.6）
- [x] 契约 §5.6 与 SDK 指南 §6 回写。

#### EXT-10 / P0：sys_test 用例 `support_group`

- [x] `web/src/test_groups.ts` 新增分组 `support_group`（非 backendOnly，但三个动作是按钮而不是“全部运行”用例）：**创建客服群**（用户会话 `group.create`，群名、alice / bob 邀请、§11.1 配置；结果卡片显示群 DID、成员状态、Message Hub 深链）、**生成客户链接**（输入称呼 + 外部标识 → `POST /sdk/appservice/support/customer_link` → 展示一次性链接与 `expires_at`，并提示“客户未设密码前此链接 48 小时内可直接进入”）、**清理**（删群 + 撤销该群下由本 App 签发的记录）。alice、bob 用户名可改，缺省 `alice` / `bob`。
- [x] `main.ts` 新增路由：`GET /support/panel`（面板页，静态 HTML，读四个 query；无 App 会话时不报错，只显示摘要）、`GET /sdk/appservice/support/summary`（App 服务会话 `group.list_messages` → 条数 / 最近时间 / 参与者数）、`POST /sdk/appservice/support/customer_link`（校验调用者是本页面的用户会话且为 App 实例 owner，再以 App 服务会话调 `user.app_login.issue {kind:"link", external_account_id, show_name, resource, return_path}`）、`POST /sdk/appservice/support/cleanup`。后台不记录链接、secret 与外部标识原文。
- [x] `dapp_meta/app.json`：若 msg-center RBAC 拒绝 App 服务会话的 `group.list_messages`，补 `permissions`；面板页路径与 `www` 端点 `allow_guest: true` 保持。（实际缺口在用户会话建群，见 §11.6 偏差 3）
- [x] README 增加“客服群官方例子”一节：流程、前置（装 App、owner 打开 `app_login` 开关、alice / bob 存在）、已知限制（客户是 Session Only Member 不是群成员；面板摘要接口对持有群 DID 的任何人可读，只给计数不给正文；sys_test 实例 owner 才能签链接）。
- [x] 自动化：
  - 后端 DV `test/test_msg_center/group_ext_sample/dv.ts` 增加第 6 节或新建 `test/test_sys_test/support_group_dv.ts`：以 devtest 会话调 sys_test 页面同样的 `group.create`；用 App 服务会话（取法见 §10）调 `user.app_login.issue {kind:"link"}`；断言 `expires_at - issued_at == 48h`；用链接 `auth.invite.login` 得受限会话、接受 Session 邀请、`list_sessions` 看到面板；`auth.invite.set_password` 后再 `auth.invite.login` 得 `credential_state/password_required`；`login_by_password` 后仍能 `group.list_messages`；关开关后 refresh 被拒。48 小时过期不等待，靠 TTL 单测。
  - 浏览器 DV `src/frame/desktop/tests/dv/support-group-link.spec.ts`（`playwright.dv.config.ts`，`BUCKYOS_TEST_APP_HOST=buckyos-systest`）：devtest 在 sys_test 页面点两个按钮取到链接；新 context 打开链接 → 访客页出现 `sso-first-password`、点“加入”、发言、侧栏“工单”面板加载；点“设置密码”保存 → 新 context 再开链接 → 出现 `sso-invite-password-login` → 密码登录回到访客页；alice 的 context 在 Message Hub 看到客户发言与面板。
- [x] 文档：架构稿 §12.2 / SDK 指南 §1 把 sys_test 用例登记为官方例子，`group_ext_sample/README.md` 说明分工。

### 11.5 验收补充（并入 §7）

| 场景 | 应验证结果 |
| --- | --- |
| App 签发 `kind=link` | 链接 48 小时内多次可用；兑换不消费；`auth.app_login.exchange` 拒绝 link 记录；App 关开关后已登录会话 ≤15 分钟失效 |
| 客户未设密码 | 访客页顶部出现设置密码提示，可忽略继续聊天；面板与 `display` 可见；拿不到 `group_doc` |
| 客户设密码后 | 原链接 → `password_required` → 密码登录 → 同一访客页；其它设备的免密 family 已撤销 |
| 同一外部标识重复签发 | 同一 Limited 账号、新记录；旧链接 `link_replaced` 或自然过期 |
| sys_test 非 owner 用户点“生成客户链接” | 后台拒绝（不以 owner 的 App 身份替别人的群签发） |

### 11.6 实施记录（2026-10-24）

**范围与结论。** EXT-11（机制）与 EXT-10（sys_test 用例）已实施，未提交；Rust 单测、websdk jest、Desktop Playwright（mock）通过；本机单 Zone（test.buckyos.io）重建 control-panel / verify-hub / msg-center / desktop / buckyos_systest 并 `start.py` 保留数据重启后，后端 DV `test/test_sys_test/support_group_dv.ts` 五步全过；浏览器 DV `src/frame/desktop/tests/dv/support-group-link.spec.ts` 在重建 msg-center（偏差 3 的 RBAC 放行）后**通过**：devtest 在 sys_test 页面点“创建客服群”“生成客户链接”；客户新 context 打开链接 → 访客页出现设置密码提示、点“加入”、侧栏“客服工单”面板（`src` 为 App 子域 `/support/panel`）就绪、发言；点“设置密码”保存后，第三个 context 再开链接 → “用密码登录”→ 登录页预填用户名 → 密码登录回到同一会话看到自己的留言；devtest 在 Message Hub 的 `客服工单` Session 看到客户留言。截图 `test-results/dv-support-group-{owner,guest,password,hub}.png`。

**落点**

| 部分 | 位置 |
| --- | --- |
| 共享 | `buckyos-api/src/hybrid_sso.rs`：`InviteCredentialKind { Link, Code }`（`InviteCredential.kind`，`#[serde(default)]`=link）；`SessionSource::AppLogin` 注释改为“App 签发的凭据（code 或 link）”；`rbac_config.rs` 默认策略加 `app` 组 `obj://msg-center/group create`、`group_inbox/* read`（见偏差 3） |
| CP | `app_login.rs`：`IssueKind { Code, Link }`、`IssueParams.kind`、`issue_app_login` 按 kind 取 TTL 与 `credential.kind`、响应 `code` 或 `link`（`IssuedAppLogin::credential_field`，链接用 `exclusive_invite::invite_link` + `zone_root_origin`）、`auth.app_login.exchange` 对非 code 记录 `entry_unavailable/credential_kind`；`exclusive_invite.rs`：`issue_credential_with_ttl` 多一个 kind 参数，`invite_link` 改 `pub(crate)` |
| verify-hub | `exclusive_invite_decision` 多一个期望 kind（排在撤销之后、账号状态之前，不符 `entry_unavailable/credential_kind`）；`login_by_exclusive_invite` 对 `issuer.kind=app` 的记录先查 `app_login_available`，family 来源用 `SessionSource::AppLogin`（因此 `revoke_request_matches` 的 `source_app_instance_id` 与 refresh 时的开关 / 运行态复核不用改）；`recheck_principal` 只在 target 为 App 实例时跳过 `resolve_user_auth_target`；新测试 `app_issued_links_sign_in_on_the_zone_root_and_follow_the_app_switch` |
| websdk | `src/app_login.ts`：`issueAppCustomerLink`（`kind:"link"`，返回 `IssuedAppCustomerLink`）、`issueAppLoginCode` 显式发 `kind:"code"`；`tests/app_login.test.ts` |
| Desktop | `api/sso.ts`：`fetchAppLoginState` / `setAppLoginEnabled`；`app-service/pages/DetailPage.tsx`：`AppLoginSwitch`（只有安装账号看到，`useCurrentUser` 比对 `owner_user_id`）；`api/control_panel_mock.ts` 增加 `apps.app_login.*`；`i18n/app-service.ts` `app22.appLogin.*`；`tests/e2e/pages/app-service.spec.ts` 新用例 |
| sys_test | `support_group.ts`（后台：`GET /support/panel`、`GET /sdk/appservice/support/summary`、`POST /sdk/appservice/support/customer_link`，失败响应带 kRPC `code` / `data`）；`web/src/support_group.ts`（卡片与三个按钮）；`main.ts` 接路由并构造 `SupportContext`；`prepare.mjs` 复制新文件；`web/src/styles.css` 输入框样式；README |
| DV | `test/test_sys_test/support_group_dv.ts`；`src/frame/desktop/tests/dv/support-group-link.spec.ts` |
| 文档 | 契约 §5.6（`credential.kind`、`kind=link` 一段、续期一句）；SDK 指南 §1 表、§6.4 第三条路径与官方例子；架构稿 §12.2；`group_ext_sample/README.md` 分工；本文 §1 / §9 / §11 |

**验证**

- `cargo test -p verify_hub`（38 通过）、`cargo test -p buckyos-api --test hybrid_sso_fixtures`（fixture 加 `kind`）、`cargo test -p buckyos-api rbac_config`（28 通过）、`cargo test -p control_panel`（199 通过，仍只有既有的 `sso_identity::...carries_ceiling...` 失败，与本批无关）。
- websdk `npx jest tests/app_login.test.ts`（4 通过）。
- Desktop `pnpm check`；`npx playwright test tests/e2e/pages/app-service.spec.ts`（20 通过，含新用例）。
- sys_test `deno check main.ts`、`tsc --noEmit`；`buckyos-build.py -s buckyos_systest` 通过。
- 后端 DV（真实 Zone）：开关 → 建群（面板 = `https://buckyos-systest.test.buckyos.io/support/panel`）→ 后台签 48h 链接（无页面会话 401；`expires_at` 距今 48h±2 分钟；同外部标识复用账号）→ 链接登录两次得到不同会话、`auth_method=exclusive_invite` 受限、接受 Session 邀请、只见 1 个 Session 且带面板、发言、摘要接口计数但不含正文、`auth.app_login.exchange` 对链接 `credential_kind` 拒绝 → 设密码后链接 `password_required`、密码登录可进同一 Session、新签链接也不再登录 → 关开关后签发 `access_denied/zone_method_disabled/app_login_disabled`。

**偏差 / 决定**

1. **链接登录的 family 来源**：D-8 文字写“`ExclusiveInvite` family 也查开关”；实现改为 App 签发的链接登录直接用 `SessionSource::AppLogin{app_instance_id, invite_id, version}`，复用已有的 refresh 复核与按实例撤销，不加字段。claims 仍是 `auth_method=exclusive_invite`。
2. **kind 判定位置**：`credential.kind` 不符的拒绝排在“记录不存在 / 撤销”之后、账号状态之前；verify-hub 既有测试里“用户签的邀请当 code 兑换”从 `target_mismatch` 改为 `credential_kind`。
3. **RBAC（浏览器 DV 发现）**：App 子域上的用户会话（`appid=app:buckyos-systest…`）调 `group.create` 被 zone RBAC 拒绝——enforce 是 user 侧与 app 侧的合取，`app` 组此前没有 `obj://msg-center/group` 规则（App `permissions.scope_path` 也不会翻译成 msg-center 策略）。按“先能力后安全”在默认策略给 `app` 组加 `group create` 与 `group_inbox/* read`；没有改 `dapp_meta/app.json`。已知风险：任何 App 子域页面都能以用户身份建群（群内权限仍由 msg-center 判）。
4. **sys_test 页面登录**：App 的 `www` 端点 `allow_guest: true`，直接打开页面是“未登录”，要点页面的 Login 走 SDK SSO 到根域登录页再回来；浏览器 DV 按此驱动。
5. **开关 UI 位置**：任务写 Users and Agents，实际放在 App Service 的应用详情页（那里才有按实例的页面）。
6. **清理**不撤销链接记录（App 没有可调的撤销方法），记录到期自然失效；README 已注。

**坑**

- sys_test 的 `pnpm update buckyos` 从 GitHub main 取 SDK，本地 websdk 改动不在其中，所以 sys_test 直接用 `getServiceRpcClient('control-panel').call('user.app_login.issue', {kind:'link'})`，不依赖 `issueAppCustomerLink`。
- `start.py` 会从新构建的 pikg 重新部署内置 App 容器（挂载的 `.pkg` 哈希即新包的 `pkg_objid`），不必手工拷文件。
- Playwright 的 `locator.isVisible({timeout})` 不等待；跨页跳转后要用 `waitFor({state:'visible'})`。
- 后台把 kRPC 失败只转成文本会丢掉 `code` / `data`，DV 按文本判断违反“错误只按枚举判断”；改成原样带出。

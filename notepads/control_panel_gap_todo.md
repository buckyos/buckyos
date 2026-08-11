# Users & Agents：control_panel 实现缺口 & 开发 TODO

> 基于 `product/users_and_agents/Users_Agents_PRD.md` + `UserType.md` + `product/my_netowrks/My_Network_PRD.md` 对照
> `src/frame/control_panel`（后端）/ `src/frame/msg_center`（社交图后端）/ `src/frame/desktop`（前端）现状整理。
> 面向下一步 Code Agent。按优先级排序，每条给出涉及文件 / 建议 RPC / 验收点。

---

## 0. 产品边界（先看这里——决定一个需求归不归 control_panel）

MyNetwork PRD §15.3 / §20 已把社交关系从 Users & Agents 拆出。后端归属随之分层：

```
Users & Agents（control_panel）= Zone 内部实体：User / Agent / Self-hosted Group 的
                                  身份、配置、权限、运行状态、登录
My Network（后端在 msg_center） = 外部关系网络：Contacts / Friends / Joined Groups /
                                  Collections / Requests / Import / DID 发现
Home Station                    = 内容与 Feed
MessageHub                      = 消息与会话（control_panel/message_hub.rs 是其 UI 代理）
```

**判定规则**：
- 「Zone 内可登录用户 / Agent / 我自己 Host 的群的系统配置」→ **control_panel**（本文 P0~P2）。
- 「联系人 / 好友 / 加入别人的群 / 分组整理 / 导入 / 关系状态」→ **My Network = msg_center**（本文 §X，多数已实现，**不要在 control_panel 重造**）。
- Self-hosted Group 拆两半：**群实体本身**（成员/角色/消息对象）在 msg_center `group_mgr`；**托管/服务/系统权限状态**在 control_panel（见 TODO-8）。

---

## 0.1 现状速览

**control_panel 后端 RPC**（`src/frame/control_panel/src/user_mgr.rs` + `main.rs`）
- 用户：`user.list / get / create / update / update_contact / delete / change_password / change_state / change_type`
- Agent：`agent.list / get / set_msg_tunnel / remove_msg_tunnel`
- 登录/SSO：`auth.login` + `/sso_callback /sso_refresh /sso_logout`（`sys_auth_backend.rs`）
- MessageHub UI 代理：`chat.bootstrap / contact.list / message.list / message.send`（`message_hub.rs`，数据源是 msg_center）

**数据模型**（`src/kernel/buckyos-api/src/control_panel.rs`）
- `UserSettings { user_id, type(UserType), show_name, password, state(UserState), res_pool_id, contact? }`
- `UserType = Admin | User | Root | Limited | Guest`，`UserState = Active | Suspended | Deleted | Banned`

**msg_center 已实现的社交图后端**（My Network 复用，**非 control_panel 缺口**）
- `contact_mgr.rs`：`import_contacts / merge_contacts / list_contacts / get_contact / update_contact / block_contact / grant_temporary_access / check_access_permission(AccessGroupLevel) / resolve_did / get/set_group_subscribers / upsert_zone_user_contacts`
- `group_mgr.rs`：`create_group / update_group_profile / invite_member / request_join / approve_member / reject_member / remove_member / update_member_role / list_members / subgroups / list_groups_by_member / check_group_access`

**前端**：桌面 `users-agents` UI 目前**完全 mock**（`mock/store.ts`），未接后端。My Network 前端见 `src/frame/desktop/src/app/my-network`。

---

## P0 — 正确性 / 安全（必须先修，否则权限模型整体失效）

### TODO-1　修复认证主体 user_type 被硬编码为 Root
- **问题**：`sys_auth_backend.rs:921` 返回 `RpcAuthPrincipal { user_type: UserType::Root, .. }` 写死为 Root。
- **后果**：`require_admin` / `require_self_or_admin`（`user_mgr.rs:47-68`）对所有登录用户恒通过 → 任何普通用户能创建/删除/改类型/改他人密码。`UserType.md` 整套权限模型形同虚设。
- **做什么**：签发 principal 时读 `users/{sub}/settings` 的真实 `user_type` 填入；解析失败拒绝而非默认 Root。区分 zone-owner/root（zone 外私钥）与普通登录用户。
- **验收**：普通 User 调 `user.create` / `user.delete` 返回无权限；admin 正常。

### TODO-2　Limited 用户限制落地
- **问题**：`handle_user_change_password`（`user_mgr.rs:493`）不检查 user_type；`UserType.md` 明确 limited「不允许修改密码」（含未成年）。
- **做什么**：(a) self 改密码时 `user_type==Limited` 拒绝（admin 代改按产品确认）；(b) `UserSettings` 增可选限制位（如 `allow_password_change`）或以 user_type 推导。
- **验收**：Limited 用户自助改密码被拒。

---

## P1 — Zone 用户主流程缺口（PRD §12.1 本期必须）

### TODO-3　一级 BNS/DID 邀请加入流程（完全缺失）
- **PRD**：§8.1 选项1 + §8.2 全流程；`UserType.md` §新建。
- **现状**：`user.create`（`user_mgr.rs:214`）只支持「本地带 password_hash 账户」；无邀请链接、无 pending、无 `binded_zone_list` 校验、无 pending→active 激活。
- **做什么**（建议新增 RPC）：
  - `user.invite.create`（admin）：生成邀请链接 + 写 pending 记录（默认 user_type / 权限组 / App / 有效期）。
  - `user.invite.get`：落地页读 target zone 信息 / 风险提示。
  - `user.invite.accept`：查被邀请 BNS 的 ownerconfig，确认 `binded_zone_list` 含本 zone → 激活 → 写 UserSettings + 默认组 + App。
  - `UserState` 需扩 `Pending`。
- **依赖**：`binded_zone_list` 校验走 DID-Profile 解析协议（见 §Y），`owner_is_bound_to_zone(did, zone)`。
- **安全红线**：绝不在本系统输入外部身份原始密码；激活以 BNS ownerconfig 为准。

### TODO-4　二级 DID 本地账户自动创建 OwnerConfig
- **Doc**：`UserType.md` §新建「二级用户:自动创建全套 OwnerConfig」。
- **现状**：`user.create` 只写最小 doc JSON（`user_mgr.rs:273`），无密钥/OwnerConfig。
- **做什么**：本地账户创建时生成完整 OwnerConfig（密钥对、`did:bns:{user}.{zone}`）。复用 scheduler/buckyos-api 既有逻辑。
- **验收**：`users/{id}/doc` 是可解析 OwnerConfig；二级 DID 可被 resolve。

### TODO-5　默认基础组强制加入
- **PRD**：§7.3「所有本空间用户默认属于同一不可移除基础组」。
- **现状**：`user.create` 仅 Admin 时 append `g,{user},admin`（`user_mgr.rs:296`），普通用户不入基础组。
- **做什么**：create / invite.accept 时把普通用户加入 `users` 组（如 `g,{user},users`），管理员加入 `admin` 组，接口不允许移除默认组。

### TODO-6　Profile / Settings 分区（Settings 留本地，Profile 读取走协议）
- **PRD**：§5.6 / §8.4；`UserType.md` §通过 did 获取 profile。
- **现状**：`user.update`（`user_mgr.rs:323`）只改 `show_name`；无 profile 字段，无合并读取。
- **做什么**：
  - `user.profile.get / set`；**get 的双源合并（current-zone + BNS，BNS 优先）下沉到 name-client DID-Profile 解析协议**（见 §Y），control_panel 只调。
  - `set` 区分本地字段（直接写）与 BNS/链上字段（确认/成本/延迟提示）。
  - Settings（账号状态/类型/凭证/密码策略/默认组/可用 App）留 control_panel。
- **注意边界**：这里是 **Zone 用户自己的 profile**；**联系人的 profile 展示属 My Network/msg_center**，别混。

---

## P2 — Agent 与 Self-hosted Group 系统管理（Users & Agents 专属）

### TODO-7　Agent 写操作 + 运行态信息
- **PRD**：§7.2（DID/Profile/Binding/Settings + work log、UI/Work Session 数、Workspace 数）。
- **现状**：只有 `agent.list/get/set_msg_tunnel/remove_msg_tunnel`，无创建/更新/删除、无 profile、无运行态。
- **做什么**：`agent.create / update / delete`、`agent.profile.get/set`，运行态聚合（对接 `ui_session_mgr` / agent runtime：session / workspace 计数、最近日志）。

### TODO-8　Self-hosted Group 系统/托管状态管理　【暂缓 / 待定边界】
- **状态**：**暂缓**。不在本轮 control_panel 开发范围，先标注、不开工。
- **暂缓原因（两组件有现实矛盾，需先定边界再动手）**：
  - Self-hosted Group 牵扯 **资源管理**（托管/服务/配额）与 **匿名访问**（public resource / guest 写入策略），这两点恰好踩在两个组件的价值取向冲突上：
  - **control_panel 关注严格的管理**（RBAC、资源边界、可登录主体、审计）；
  - **msg_center 关注产品体验**（加群顺滑、匿名可读、评论低摩擦、关系驱动）。
  - 同一个「Self-hosted Group」既要被 control_panel 当受控资源管，又要被 msg_center 当社交对象用，职责切分没定清前强行实现会两边打架。
- **PRD 线索**：MyNetwork §2.2 / §4.4 / §8（托管/服务/系统权限归 Users & Agents，加入/退出/浏览归 My Network）。
- **现状**：群实体本身在 msg_center `group_mgr`（create/members/roles 完整）；control_panel 侧无托管/服务/系统权限视图。
- **解封前置**：先产出一份「Self-hosted Group 跨组件职责 & 资源/匿名访问边界」设计，回答：资源/配额谁是真相源、匿名读写策略谁定、RBAC 与社交 access level 如何不冲突。

### TODO-9　用户 / Agent 的 Message Tunnel Binding 管理
- **PRD**：§7.1 / §8.5（Self & Agent 详情页的 Binding 区，状态/最近同步/异常态）。
- **现状**：`user.update_contact`（`user_mgr.rs:371`）整包覆盖 bindings；Agent 有 `set/remove_msg_tunnel`，用户侧无对等。
- **做什么**：`user.set_msg_tunnel / remove_msg_tunnel`（与 agent 对称）；binding 增 `status / last_sync_at`。**注意**：tunnel 运行态在 msg_center（`msg_tunnel.rs` / `tg_tunnel.rs`），control_panel 只管「实体声明了哪些绑定」，状态向 msg_center 取。

---

## X — 已移交 My Network（msg_center），**control_panel 不再承接**

> 这些是上一版 todo 误判为 control_panel 缺口的项。按产品边界归 My Network，且多数 msg_center 已实现。
> 列在此处是为了**防止 Code Agent 在 control_panel 重造**。需要时另起 My Network 的 gap 文档。

| 旧 TODO | 归属 | msg_center 现状 |
|---|---|---|
| 集合（Collection：我的好友/手工集合/动态视图） | My Network | 联系人分组/订阅 `get/set_group_subscribers` 有；Static/View Collection 待 My Network 设计 |
| 实体群-外部 Joined Group（加入/退出/浏览） | My Network | `group_mgr`：`request_join / list_groups_by_member / list_members` 已有 |
| 联系人导入（CSV/XML/TXT） | My Network | `contact_mgr::import_contacts` 已有 |
| 手工合并联系人 | My Network | `contact_mgr::merge_contacts` 已有 |
| DID 单向/双向关系 | My Network | `contact_mgr::check_access_permission` + `AccessGroupLevel` 已有 |
| 手机号/邮箱 → DID 发现 | My Network | 未实现，My Network 排期 |
| Block / 临时访问 | My Network | `block_contact / grant_temporary_access` 已有 |

---

## Y — 横切依赖：DID-Profile 解析协议（name-client）

- TODO-3 的 `binded_zone_list` 校验、TODO-6 的 profile 双源合并，统一下沉到 buckyos-base **name-client** 的「DID → profile/owner-config」协议。
- 需求已提取（决策：`binded_zone_list` 进 OwnerConfig，`default_zone_did = binded_zone_list[0]`；用户 profile 用扁平 LinkedIn 式 schema）。
- control_panel 只调 `resolve_user_profile(did)` / `owner_is_bound_to_zone(did, zone)`，不在本地手搓 BNS 解析。

---

## Cross-cutting

- **CC-1**　`user.list`（`user_mgr.rs:118`）不过滤 `state==Deleted`，软删用户仍计入。确认是否按 state 过滤 + `include_deleted` 参数。
- **CC-2**　`change_type` 把 admin 降级时不回收 RBAC（`user_mgr.rs:650` 靠 scheduler reconcile）。确认是否可接受。
- **CC-3**　桌面 `users-agents` UI 全 mock，需「API client 封装 + 真实 RPC 替换 mock」接线任务（依赖 P0~P2 端点稳定）。
- **CC-4**　模型扩展：`UserState::Pending`（TODO-3）；`UserSettings` profile/binding 状态字段（TODO-6/9）。在 `buckyos-api/src/control_panel.rs`，beta2.2 允许破坏性改动。

---

## 建议落地顺序

1. **P0（TODO-1,2）** — 安全闸门。
2. **模型扩展（CC-4）+ §Y 协议** — `UserState::Pending`、Profile 字段、name-client 解析协议，为 P1 铺路。
3. **P1（TODO-3,4,5,6）** — 邀请加入 / OwnerConfig / 默认组 / Profile，Zone 用户主流程。
4. **P2（TODO-7,9）** — Agent 写操作 + 用户/Agent Binding。（**TODO-8 Self-hosted Group 暂缓**，需先定跨组件边界）
5. **CC-3 接线** — UI 接真实后端。
6. **My Network（§X）** — 另起文档，主要在 msg_center，与 control_panel 解耦推进。
7. **TODO-8 解封** — 待「Self-hosted Group 跨组件职责/资源/匿名访问边界」设计定稿后再排。

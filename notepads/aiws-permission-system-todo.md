# AIWS 权限系统实施 TODO

依据：[AIWS 权限系统整理需求](aiws-permission-system-prd.md)（下称“需求”），2026-11-13 owner 要求按需求完成实现。

基线：buckyos `fba849305`。允许 breaking change，不迁移旧的授权行（需求 §4.2）。

状态：实施中。每完成一项在本文 §3 记一笔；偏差记在 §4。

## 1. 设计落点

### 1.1 存储

`local.sqlite`（服务控制状态，需求 §4.3）：

| 表 | 内容 |
| --- | --- |
| `acl_entries` | `(subject, scope_entity_id)` → `allow`、`deny` 位图，`updated_by`、`updated_at`。`scope_entity_id` 为空是 `/`。主体：principal、`@<组 ID>`、`@all`、`@guest`、`@creator`、`@pub:<发布 ID>` |
| `acl_groups` | `group_id` → 标题、说明、建立者、时间 |
| `acl_members` | `(group_id, member)` → `via_entry`、加入者、时间。成员：principal、`group:<id>`、`contact:<did>#<tag>`、`*` |
| `acl_macro_bindings` | 许愿格 ID → 组、`union` / `intersection` |
| `acl_audit` | 只追加的操作记录：时间、操作者、按谁判定、动作、对象（实体 ID 和当时的路径）、改前、改后 |
| `local_meta.anonymous` | 公共匿名访问开关 |
| `local_meta.acl_rev` | 权限变化计数，变更流用 |
| `share_entries` | 改为指向目标组：`group_id`、`max_joins`、`joins`；去掉能力模板和范围 |

旧的 `grants` 表在打开时删除，不迁移。

`doc.sqlite`：`entities.created_by`（需求 R6）；`commits.attribution_json`（按谁判定、触发者、执行者、应用者，需求 R16、R31）。存储版本 6 → 7，打开时补列，`created_by` 由创建 Commit 的写入者补出。

### 1.2 判定

`core/src/access.rs` 的 `Access` 改为按层、按档判定（需求 §4.6）：

- 每层（实体 ID，`""` 是 `/`）存放适用于调用者的条目：主体、档（单人 / 组与 `@creator` / `@guest` 与 `@all`）、允许、拒绝。
- 从实体沿祖先到 `/`，每一层按档决定尚未决定的能力。
- `@creator` 的条目只在被判断实体的 `created_by` 等于调用者时生效。
- owner 恒为全部能力；可见范围（`user:` 等）照旧另判。
- 同一套逻辑给出解释（每项能力由哪一条决定）。

`store/src/acl.rs` 负责：读取权限数据、按调用者算主体集合、批量写入、查询与解释、组与成员、匿名开关、宏绑定、操作记录。

### 1.3 接口

新方法（替代 `ws.grant` / `ws.revoke` / `ws.list_grants`，不保留旧接口）：

| 方法 | 说明 |
| --- | --- |
| `acl.set` | 批量写条目：`changes: [{ subject, resource, allow, deny }]`，原子生效，一条审计 |
| `acl.query` | `by: entity`（`resource`、`expand?`）、`by: subject`（`subject`）、`by: explain`（`resource`、`principal?`）|
| `acl.groups` | 列组；管理者另带成员 |
| `acl.set_group` / `acl.delete_group` | 建组、改标题说明、删组 |
| `acl.members` | `add` / `remove` 成员 |
| `acl.set_anonymous` | 公共匿名访问开关 |
| `acl.bind_macro` | 宏的组绑定 |
| `acl.audit` | 读操作记录 |

`acl.query` 另有 `by: all`（管理者看全部条目，其他人看自己的），给客户端列表和 CLI 用。

入口：`ws.create_entry { group_id, methods, expires_at?, max_joins? }`、`ws.close_entry { entry_id, remove_members? }`；`ws.join_entry` 访客也能用（带浏览器自己的 `gt_` 令牌）。

变更流：`ws.get_info.acl_rev`、`doc.wait_changes { after_acl_rev }` 在权限变化时返回、kevent `…/aiworkspace/<ws>/acl`。

## 2. 分步（需求 §8）

- [x] 第 0 步：S1、S3、S5、S6
- [x] 第 1 步：R1–R9、R17–R20（模型、组、固定组、创建者、拒绝、入口加入组、批量接口、查询与解释、变更流、操作记录）
- [x] 第 2 步：R21–R25（方法表、按实体判断、移动的记录、调用者矩阵、泄露）
- [x] 第 3 步：R10–R12、R41、R42（owner 与管理者部分）；Clone TODO 的 C3、C4、C6
- [x] 第 4 步：R13–R16、R28–R36
- [ ] 第 5 步：R27、R37、R38、R44 已做；R26 只做了 AIWS 一侧（inner-path 发布、访问计数、固定版本），不可变发布与 MyContent 登记受阻于 ShareContentMgr（§4 D1）
- [ ] 第 6 步：R39、R40、R42（Agent 部分）、R43——依赖 PRD 第二期与 B-6，本次不做

## 3. 实施记录

### 3.0 第 0 步（2026-11-13）

- S1：见第 1 步的发布改造（`publication_versions`）。
- S3 / R38：`server/src/source.rs` 的 `http_get` 只抓公网地址：按解析后的 IP 判断（回环、私网、链路本地、CGNAT、组播、文档地址都拒绝），IP 钉住防重绑，重定向逐跳检查（最多 5 跳），`localhost` 与 Zone 自己的主机拒绝。`Limits.fetch_private` 只给本地测试。
- S5 / R15：run token 只在所属尝试仍是当前尝试、运行处于 `running` 时有效；过期的令牌回业务错误 `RUN_REVOKED`（执行器据此停止）。
- S6 / R30 一部分：托管派发失败不再把领取令牌写进运行详情；`proc.claim` 只接受上下文指定的执行者（托管：配置的 Agent，按 DID 或 AgentId 认；本地：发起者）。

### 3.1 第 1 步（2026-11-13）

- 判定：`core/src/access.rs` 重写（层、档、档内拒绝优先、`@creator`、解释、`bounded_by` 给宏的交集）；`Access::granted` 给测试用。
- 创建者：`EntityRow.created_by`；计划器在 `entity.create` 记 `PlanEnv.creator`（按谁判定），与写入者不同时写进规范化操作，副本重放取回；存储 6 → 7 打开时补列（由创建 Commit 的写入者补出，查不到记 owner），`commits.attribution_json` 预留给第 4 步。`fixtures/vectors/apply-log.json` 重录（状态摘要含创建者）。
- 存储：`acl_entries`、`acl_groups`、`acl_members`、`acl_macro_bindings`、`acl_audit`、`publication_versions`，入口表改为指向组；旧 `grants`、模板式 `share_entries` 打开时删除。
- `store/src/acl.rs`：读取、判定（含 Agent 未配置时按 owner）、`acl.*` 全部方法、操作记录、`acl_rev`、权限设计的导出与装入、`macro_access`（交集）。
- 入口（R8）、owner 专属（R17：删除、托管配置、覆盖式恢复、`manage` 的授予与收回、带 `manage` 组的成员）、能力上限（R18）、purge 一并删条目（R20）。
- 发布改为 `@pub:<entity_id>` 条目，读者走发布路由（R26 的 inner-path 部分）；`?fixed=` 只认本发布给出过的版本（S1）；`…/referrers` 限大小、每个发布最多 256 条（V3）。
- 模板（`templates/<id>/permissions.json`，受 digest 保护）、Clone、导出包（`manifest.permission_design`）在有 `manage` 时带权限设计，新建时装入（R10–R12，Clone TODO C3、C4、C6）；覆盖式恢复保留原工作区的权限和创建者。
- 服务端：`acl.*` 分派与方法级别、Agent owner 从 AgentSpec 取（`auth::agent_owner`）、静态令牌加 `agent` 类、`acl.query` 关于他人时由服务现查成员关系、`wait_changes` 带 `acl_rev`、kevent `acl`、放映只有工作区级写权限才加锁（R22 一项）、`ws.get_info` 的托管配置只给管理者（V1）。
- 演示会话：每个链接是运行工作区里的一个逻辑组，撤销即删组（R27 一项）。
- Desktop：`api/legacyGrants.ts` 把权限面板、分享对话框的旧“授权”形状映射到条目和组（`public` → 匿名访问 + `@guest`；`*`、`group:`、`contact:` → 单成员逻辑组；入口预设 → `share-<caps>` 组），界面不改；设置页“公开的工作区”改为看匿名访问和 `@guest`。e2e 用例改用 `acl.*`。
- 测试：store `sso.rs`、`guest.rs` 按新模型重写；contract 加 `acl.*` 步骤并重录；README“Permissions”与方法表。

### 3.2 第 2 步（2026-11-13）

- R21：`server/src/methods.rs` 的 `METHODS` 是全部方法的表（级别、判断位置、所需能力）；dispatch 在进入 handler 前按表做通用检查（可以拥有、owner、成员、`/` 上的能力、参数指名实体上的非读能力），handler 保留自己更细的检查；服务 README 的授权表由它生成（contract 测试比对，`AIWS_REGEN=1` 重写）。`METHOD_LEVELS` 删除，contract 测试改用 `METHODS`。
- R22：许愿格分析 / 编码要许愿格上的 `update`，运行要 `run` 或任何写能力（不论本地、托管）；`require_run` 按许愿格判断；看到、应用别人的运行要许愿格上的 `update` 或 `manage`（只有 `run` 只能看自己的）；暂存文件只要求“某处有写能力”，最终 Commit 按落点判断；放映只有工作区级写权限才加锁。
- R23：`tree.move` 后有主体对被移动实体新得到能力时，Commit 结果带 `visibility_widened`，操作记录写 `visibility.widened`。
- R24：`store/tests/acl.rs`（7 个用例：路径跟随与查询、拒绝的层与档、私有子树与 `@owner` / `@creator` / R17、Agent 按 owner 与单独配置、移动记录与变更流、调用者矩阵覆盖 ls / find / 路径解析 / body / SQL / 发布读者、权限设计随 Clone 与包传播），加上 store `sso.rs` / `guest.rs`、server `sso.rs` / `guest.rs` / `pages.rs` / `source.rs` 的改写。
- R25：V1（托管配置只给管理者）、V2（工作区主机的页面路由只按名字解析匿名可读的画布）、V3（引用登记限大小与条数）、V4（写入者读不到的同级实体不参与重名判断；阶段工具的 `path_of` 已经按祖先可读打印，无需再改）、V5（幂等重放前先确认还能进入工作区）。搜索与 SQL 已按读权限过滤（矩阵用例覆盖）。

### 3.3 第 3 步（2026-11-13）

- 模板、Clone、导出包的权限设计见第 1 步；`ext.export_template` 返回 `dropped_permission_entries`；`template.schema.json` 的说明未改（文件布局由 `verify_templates` 允许 `permissions.json`）。
- CLI（websdk `cli/modules/aiws.ts`）：`buckyos aiws acl ls|show|who|explain|groups|entries|audit`（读）、`buckyos aiws acl-edit set|rm|group-add|group-rm|member-add|member-rm|anonymous|bind|entry-add|entry-close|publish|unpublish`（写）；`cli/tests/aiws_test.ts` 加了映射用例。
- 手册：`aiworkspace_template_dev`（权限命令、模板带权限设计、HTML 代码要 manage）、`aiworkspace_run`（阶段权限、`zone.fetch` 只读且需声明）。

### 3.4 第 4 步（2026-11-13）

- R14 代行身份：运行上下文记 `trigger_identity`（类别、是否受限、DID、Agent 的 owner）；`wish::delegated` 按它重建调用者并现查成员关系，用于领取、汇报、应用和 run token；数据源刷新、会话写入改用 `Caller::internal(owner)`（服务以自己的名义为 owner 做事，Commit 仍记触发者）。
- R16：Commit 的 `attribution_json` 记按谁判定；操作记录记 `judged_as`。
- R28：`Workspace::macro_access`（触发者 ∩ 交集绑定的组；并集本期不扩权）；`proc.apply` 按宏身份授权（`CommitOpts.authorize_as`），不按点击的人。
- R29：见 R22。R30：应用时复核托管配置摘要和上下文有效期；`limits.max_runs_per_day`、`max_tool_iterations` 在服务端执行；领取令牌与领取者见第 0 步。R31：Commit 和运行详情记触发者、执行者、应用者和绑定。R32：`doc.commit` 带 `run_id` 拒绝。R33：落点根只来自输出声明与已有结果；`location` 只在没有声明画布时、且只认显示该许愿格的 Block 所在画布；输入只在契约声明了派生列时作为“精确目标”可写。R34：`aiws/host.js` 只放行 GET / HEAD 和 `declare.zone` 列出的前缀（`ZONE_READ_ONLY`、`ZONE_NOT_DECLARED`），三处执行器共用。R35：`llm_cache` 按 AI 归属用户分目录。R36：R18 已覆盖。

### 3.5 第 5 步（2026-11-13）

- R27：链接可设 `expires_at`；撤销链接时经它加入的参与者行删除、组删除；`presenter` 不是当前调用者时必须是 Zone 用户；`session.save` 的说明对齐代码（只有 owner）。
- R37：HTML 定义的代码（`kind`、`html`、`on_bind`、`on_init_control`）写入或确认要 `manage`；计划器在定义上写 `trusted { by, rev }`（`BLOCK_DEF_KEYS` 与 `block-def.schema.json` 同步）；Desktop `HtmlBlockHost` 查看模式只自动运行已信任的定义，其余显示快照、点击运行。
- R38：见第 0 步 S3。R44：写进内核文档 07。
- R26：inner-path 发布（`@pub:<实体 ID>`、只走发布路由）、读取计数（`publication_reads`，`ws.list_publications` 带 `reads`）、固定版本只认本发布给出过的版本。

### 3.6 文档

内核文档 02、04、07、08、09、11 与内核 README 的 GAP 已按需求 §10 回写；服务 README（Permissions、方法表、生成的授权表、宏的桥）、宏指南 `aiws/README.md` 已更新。

### 3.7 与许愿格的衔接（2026-11-13，实施后复核）

对照第一期许愿格复核时，用临时服务端测试实测了三处挡住正常使用的问题，owner 定了 Q31–Q33（需求 §7.1），其余直接修：

- **Q31 只有 `run` 的人：** `proc.start` 的 `code` / `run` 先算宏身份，不能 `update` 许愿格、或在输出文件夹 / 已有结果上没有任何写能力时，返回 `PERMISSION_DENIED` / `MACRO_CANNOT_WRITE`（`data { missing, group_id }`），不建运行、不花 AI。Desktop 面板只对有 `update` 的人给分析 / 编码 / 运行，只有 `run` 时显示说明；去掉 `runOnly`（原 D22 的“只有 run 走托管”）。
- **Q32 许愿格的 HTML：** 计划器对来源 `program` 的代码写入不要求 `manage`，宏身份能管理该定义才写 `trusted`，否则去掉；规范化历史带上这个结论（新建随存储的 payload，修改时附带 `trusted` 键），重放不需要判断权限。管理者事后 `entity.set_keys trusted: true` 确认；确认不算对结果的手改（core `freshness::hand_edited`）。设计模式点开未信任的 HTML 控件先提示。
- **Q33 `zone.fetch`：** 维持现状，风险写进内核文档 08、服务 README。
- **读取按宏身份：** `Caller.macro_of`；编码 / 运行阶段的快照和 run token 都带它，`Workspace::access` 按绑定的交集组收紧。交集组读不到的输入，运行以 `DEPENDENCY_UNAVAILABLE`（input … is not readable）失败。分析仍按分析者本人。
- **预览按宏身份：** `plan_for` 的预检用 `prepare_with`（`authorize_as` = 宏身份，分析阶段 = 触发者本人），预览可以应用时应用一定过权限检查；分析的应用也改为按触发者本人。
- **`macro.changed`：** 只在绑定了组的许愿格的 `program` 键被写时记录。
- **托管工具轮数：** 只数模型循环的工具调用，宏的 `sql` / `llm_cache` 不算；计数放在阶段状态里，删掉全局计数表。
- **小项：** 读不到输入的人，主状态为 `no_access`（core `main_status`，WASM 已重建），`needs_coding` 里不出现那张表的字段原因；运行记录里托管配置的 `limits`、配置摘要、派发目标只给管理者。
- **验证中发现的回归（第 1 步带来，基线即失败）：** HTML 定义的 `trusted { by, rev }` 被算进内容根，导入时按导入者重写，`rev` 变了，带 HTML 控件的自带包导入（`ws.import new`、按模板新建、Clone）都以“import verification failed: content root …”失败（Desktop e2e UI16）。改为物化内容时不带 `trusted`：信任是本工作区的决定，包、模板、版本只带代码，引入者重新决定（内核文档 11 “HTML 扩展的信任在引入工作区时决定”）。旧包里带的 `trusted` 仍会导致校验失败（不兼容旧包）。
- **DV（2026-11-13，本机 Zone，release 部署）：** `tests/dv/aiworkspace.macro.spec.ts` 三例通过：`[macro]`（真实模型，浏览器执行器）；`[macro-cli]`（原 `[macro-hosted]`：S6 之后领取令牌只发给配置的 Agent，测试拿不到，改为 CLI 用 `run-start` 作本地执行器，仍验证受限 Deno 跑宏和执行者自己的 AI；托管派发给真实 Agent 未验）；新增 `[macro-permissions]`（真实临时账号：编辑者经 CLI 编码出的页面未信任、设计模式点开先提示、owner 确认后直接运行；只有 run 时 `MACRO_CANNOT_WRITE`、面板显示说明且运行按钮禁用；交集组读不到输入时 owner 的运行失败；`macro.changed` 只在写宏时记录）。
- **Desktop e2e 的 12 例基线失败（2026-11-13 修完）：**
  - 产品问题 4 处：
    - 经入口加入：入口页“以访客继续”直接跳到工作区，工作区应用里也没人调用 `ws.join_entry`，所以新模型下访客经入口进不去。改为跳转时带 `?entry=`，工作区页（`StandalonePage`）用本源的访客令牌先加入再打开。
    - `acl.query { by: all }`：非管理者原来只看到主体正是自己的条目，访客因此看不到 `@guest`、判断不出“已公开”。改为返回对调用者生效的主体（本人、所属组、`@all`、生效时的 `@guest`）的条目；`updated_by` / `updated_at` 只给管理者（读者看不到是谁授权的）。
    - nfs_server 的 zip 类型探测按签名线性扫描本地文件头，不跳过条目数据，演示文稿里不压缩存放的内嵌工作簿会被当成外层条目，图表子文档被认成 xlsx。没装 Calc 时转换失败，图表退回占位框。改为按头里的大小跳过数据（流式写入时再退回签名查找）。
  - 用例过时 8 处：`program-sandbox` 三例改用宏 API v3（`aiws.declare`、通用桥、`ai.map`、新的出网报错文字）；UI14 改用 `acl.query`；UI-SSO1 按入口指向组、经入口加入来写；UI-P16、UI-M04 按访客默认查看模式和“匿名访问 + `@guest`”来写；UI-M01 的桌面上下文显式设为桌面设备（同文件 `test.use(Pixel 7)` 会被新上下文继承）；CN17 等缩放稳定再读（`data-zoom` 在视口停稳后才更新）；OX06 等新组被选中再按取消分组；i18n 等外观设置写回账号再重载（语言跟随账号同步）。另修了偶发的 P3-01b（等新建的演示路径被选中再读）。复跑完整套件：288 通过，剩手写 H03 偶发（单独复跑通过）。
- 测试：服务端 `tests/wish_permissions.rs`（5 例），core `tests/trust.rs`（页面信任与重放一致），`main_status` 用例加 `no_access`。托管工具轮数没有端到端用例：测试里拿不到只发给 Agent 的领取令牌。

## 4. 偏差与待确认

- **D1 不可变发布与 MyContent 登记没做。** ShareContentMgr 目前只接受 DFS 路径作分享来源，也没有登记其他服务的发布、接收访问统计的接口；需求 §8 把这列为第 5 步的前提。需要它提供：`share.create` 接受对象来源（`{ object: { obj_id, name, media_type } }`，跳过快照，直接以对象为内容根并绑定保留）、`share.register_external { service, ref, url, title }`、`share.report_reads { ref, count, last_at }`。AIWS 侧已经有 inner-path 发布的读取计数，接上即可。
- **D2 分享入口改为工作区级。** 入口指向组，组是全工作区的，所以子树管理者不能再建入口（旧的 `entries_on_a_subtree_need_manage_there` 用例删除）；需要“只看某个子树”时，给目标组只在该子树上写条目。
- **D3 Agent 改权限的审批门（R42 中 Agent 的部分）未做**，按需求 §8 随 B-6 做。现在 Agent 按 owner 判定时可以直接改权限。
- **D4 按实体查询的 `expand` 只列成员说明符**，不向 control-panel 展开 BuckyOS 用户组的具体账号。
- **D5 引用登记仍不鉴权**（V3），只限大小、要求工作区 ID、每个发布最多 256 条。
- **D6 `@pub:` 主体用实体 ID** 作发布 ID（一个实体一个 inner-path 发布，名字可有别名）。
- **D7 暂存文件不按输出位置判断**（R22 第三项）：只要求调用者某处有写能力，最终 Commit 按落点与宏身份判断。
- **D8 Desktop 权限界面经映射层工作**（`api/legacyGrants.ts`）：`*`、`group:`、`contact:` 变成单成员逻辑组（`zone`、`sys-…`、`contact-…`），入口预设变成 `share-<caps>` 组；界面本身按需求 §6 后续重做。
- **D9 旧工作区的 HTML 定义没有 `trusted` 标记**，查看模式不再自动运行，需管理者重新 build 或确认（`entity.set_keys` 写 `trusted`）。

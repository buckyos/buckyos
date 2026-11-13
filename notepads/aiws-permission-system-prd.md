# AIWS 权限系统整理需求

状态：草案，待 Review。

日期：2026-11-13。源码基线：buckyos `aee37a21a`（AI Native 第一期已提交）。

依据：源码静态阅读，没有运行测试。标“推断”的条目是读代码得出、没有实测，修之前先写用例复现。

来源：

- [AI Native 内核升级 PRD](aiws-ai-native-kernel-upgrade-prd.md) 提出的权限问题：§6.5 run token、§6.6 宏的 Zone 边界、§7.4 同步程序的授权与数据可见范围、§7.9 授权绑定能力范围、§8.1 对象动作、§10 同步程序的授权主体、§12 实体级可见范围未核对。
- [内核设计 README](../doc/workspace/kernel/README.md) 的 GAP“权限”一节：单个 Block / 数据源的授权，BuckyOS 用户组进入工作区权限组。
- 内核文档 [11 权限、资源与保留](../doc/workspace/kernel/11-权限资源与保留.md)、[08 任务执行与 AI](../doc/workspace/kernel/08-任务执行与AI.md)。
- [AIWS 权限优化 TODO](aiws-permission-optimization-todo.md) 的 A、B、C 三组条目（待立项）。本文吸收全部条目，去向见 §8。

## 0. 结论

1. **模型骨架已经实现，方向不用改。** 一套 ACL、9 项能力，授权绑定实体 ID 和子树，沿祖先取并集，没有 deny，没有行列级授权（[access.rs](../src/frame/aiworkspace/core/src/access.rs)）。Commit 的每个操作都声明了所需能力（[operations.json](../src/frame/aiworkspace/schemas/operations.json)），由规划器统一检查。
2. **有六处可以直接利用的缺陷，建议不等 Review 先修（§3.1）。** 例如：
   - 公开发布路由按内容根能读到服务里任意工作区的对象。
   - 开放给访客的分享入口，对所有知道工作区 ID 的人生效。
   - URL 数据源不限抓取地址。
   - 管理者能覆盖 owner 自己的授权。
3. **服务内部重建身份时，权限会变宽或变窄（§3.2）。** 运行阶段、数据源刷新、会话写入都用 `Caller::user(x)` 重建身份，得到的是“不受限账号、没有组”。受限调用者因此多拿到 `*` 的授权；按组授权的协作者反而读不到。
4. **执行授权与内核文档 08 不一致（§3.3）：**
   - 只有 `run` 的协作者走托管执行（D22），这条路走不通。
   - 子树上的授权不能启动运行。
   - 授权上的委托字段从未生效。
   - 托管配置的 `limits` 没有执行。
5. **写入可执行代码没有单独的门槛（§3.4）。** HTML 控件同源、不设沙盒，以查看者的凭据调用服务；写入 HTML 代码只要普通写权限。2026-11-12 起查看模式下 HTML 控件自动运行。合起来，编辑者可以借管理者打开画布的机会，以管理者身份调用 `ws.grant`。这一项要 owner 裁决（Q1）。
6. **客户端只覆盖了后端能力的一部分，SDK / CLI 没有任何权限命令（§3.7）。**

§4 是需求，§5 是待 owner 裁决的事项，§6 是分期。

## 1. 范围

包括：

- AIWS 服务内的身份、主体、授权、可见范围和方法级检查。
- 分享与公开入口、演示会话。
- 执行授权（运行、宏、托管、AI 归属）和可执行内容的信任。
- 数据源的授权。
- Desktop 的权限界面、SDK / CLI 与 Agent 接口、审计。

不包括：

- verify-hub、SSO、Zone RBAC 本身。AIWS 只使用它们给出的身份、会话上限和用户组。
- Agent 的 appid 统一授权（[Agent＝特殊 App](agent-appid-authorization-implementation-todo.md)）。本文只定 AIWS 一侧怎样认 Agent。
- 资源计量与额度（内核文档 11 第二节）。本文只涉及“由谁承担”。
- 同步程序的执行环境、凭据保管（PRD 第二期）。本文只定它的授权在 ACL 里怎样表达。

不改的原则（内核文档 11）：

- 只有一套 ACL；授权绑定稳定的实体 ID 与子树。
- 只做加法，不引入 deny。§5 Q7 是唯一可能触碰这一条的待决项。
- 不提供行列级授权，用受控提交（提交者字段）代替行级写。
- 由服务端执行，客户端的检查不算数。

## 2. 现状

### 2.1 结构

| 层 | 现在的实现 | 代码 |
| --- | --- | --- |
| 调用者 | 每个请求解析成一个 `Caller`：类别（账号 / External / 访客 / 服务）、principal、`app_id`、成员关系 | [workspace.rs](../src/frame/aiworkspace/store/src/workspace.rs) `Caller`、[auth.rs](../src/frame/aiworkspace/server/src/auth.rs) `RuntimeAuth`、[lib.rs](../src/frame/aiworkspace/server/src/lib.rs) `krpc` |
| 主体 | 授权行的 subject：精确 principal、`public`、`*`、`group:<id>`、`contact:<did>#<tag>` | `Caller::matches` |
| 授权 | `local.sqlite` 的 `grants` 表：`(subject, scope_entity_id)` → 能力位图；scope 为空是工作区级 | `grant_with`、`access_with` |
| 可见范围 | 实体的 `scope`：`user:<p>` 只有本人可见；`presenter` 只有演示者与写权限者可见；`participant:<p>` 另加 `readers` | access.rs `sees_scope` |
| 能力判定 | Commit 操作按 operations.json 的 `capability` 在目标实体上判；kRPC 方法各自在 handler 里判，没有统一的表 | [plan.rs](../src/frame/aiworkspace/core/src/plan.rs) `OPERATIONS`、lib.rs `dispatch` |
| 执行 | 服务不运行程序、不调模型；运行由发起者的浏览器或 owner 的 Agent 执行；应用候选是一个普通 Commit | [wish/mod.rs](../src/frame/aiworkspace/server/src/wish/mod.rs)、[exec.rs](../src/frame/aiworkspace/server/src/wish/exec.rs)、[runs.rs](../src/frame/aiworkspace/store/src/wish/runs.rs) |
| 入口 | 分享入口、公开发布、演示会话链接、放映提词令牌、DID 对象入口、资产与发布的 HTTP 路由 | [entries.rs](../src/frame/aiworkspace/store/src/entries.rs)、[store source.rs](../src/frame/aiworkspace/store/src/source.rs)、[session/mod.rs](../src/frame/aiworkspace/store/src/session/mod.rs)、[obj_provider.rs](../src/frame/aiworkspace/server/src/obj_provider.rs) |
| 客户端 | 按 envelope 的 `capabilities` 决定可用操作；HTML 控件同源运行 | [PermissionsPanel.tsx](../src/frame/desktop/src/app/aiworkspace/ui/sources/PermissionsPanel.tsx)、[htmlRuntime.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/htmlRuntime.ts) |

### 2.2 调用者

| 调用者 | principal | 能匹配的主体 | 限制 |
| --- | --- | --- | --- |
| 账号（不受限） | 用户 ID | 精确、`public`、`*`、`group:`、`contact:` | 无 |
| 账号（受限，Limited） | 用户 ID | 同上，但不匹配 `*` | 不能拥有（创建、导入、Fork、扩展库、`ws.list_subjects`）；`ws.list` 只列直接授予的工作区；拒绝时只给固定文字 |
| External | DID | 精确、`public`、`contact:` | 同受限账号 |
| 访客 | `guest:<26 字符>`，由浏览器自造的 `gt_` 令牌派生；服务端不发放，令牌不过期 | 精确、`public`，加上所有开放给访客的入口 | 不能拥有，不能加入入口，不能本地运行，不能建会话 |
| Agent | Agent 令牌的 `sub` | 精确、`public` | 可以拥有；不匹配 `*` 和 `group:`；可以本地运行（用自己的 AICC）；能加入演示会话，不能加入分享入口 |
| 其他服务 | 令牌的 `sub`，`app_id` 记应用 | 精确、`public` | 可以拥有；control-panel 可以往别人的扩展库安装 |
| run token（`rt_`） | 运行的触发者，类别是服务 | 精确、`public` | 只能调 13 个方法（`RUN_TOKEN_METHODS`）；不检查运行状态，不过期 |
| 会话链接（`dt_` / `pt_`） | `link:<invite_id>`；`pt_` 在会话里代表演示者 | 精确 | 放在 `sys[1]` 时可以调用任何方法，能做什么由授权决定 |

每个调用者的有效授权都包含 `public` 授权和所有开放给访客的入口模板（`access()` 总是传 `public = true`）。

成员关系：

- `group:` 向 control-panel 查用户设置里的组，`contact:` 问 msg-center。
- 每个请求重新查，不缓存。
- 查不到时拒绝（`authorization_unavailable`），除非调用者已经拥有全部能力。
- 代码：lib.rs `prepare_caller`、auth.rs `resolve_subjects`。

### 2.3 能力

| 能力 | 用在哪里 |
| --- | --- |
| `read` | 所有 `doc.*` 读取、发现、SQL。读不到的实体，父实体可读时报 `PERMISSION_DENIED`，否则报 `NOT_FOUND` |
| `append` | `table.insert_records`；只有 `append` 时只能改、删自己提交的那条记录 |
| `update` | 15 个内容操作；本地运行许愿格；URL 数据源的手动刷新 |
| `delete` | 4 个删除与恢复操作 |
| `structure` | 18 个结构操作（创建、移动、改名、字段） |
| `comment` | 标注；自己的标注和便签 Block 可以改、删、移动 |
| `export` | `doc.export`、`doc.checkpoint`、Fork；导出模板（只检查所选画布） |
| `manage` | 授权、分享入口、公开发布、托管执行配置、删除工作区、维护操作、`entity.set_write_policy`、`entity.purge` |
| `run` | 托管运行许愿格。`may_run` = `run` 或任何写能力 |

### 2.4 方法级检查

没有方法到能力的统一表。`METHOD_LEVELS` 只标 stable / experimental / internal，检查散在各 handler。归纳如下：

| 检查 | 方法 |
| --- | --- |
| 可以拥有（不受限账号或服务） | `ws.create` / `ensure` / `import` / `fork`、`ws.list_subjects`、`ext.*` 扩展库、`create_from_template` |
| 工作区级 `manage` | `ws.delete`、`ws.grant` / `revoke`、`ws.publish` / `unpublish`、`ws.set_hosted_exec`、保留与暂存的清理、`diag.*`、覆盖式恢复 |
| 子树 `manage` | 分享入口、`lock.break` |
| 工作区级 `export` | `doc.export`、`doc.checkpoint` |
| 工作区级 `update` 或 `structure` | 上传资产，包括运行阶段为结果暂存文件 |
| 工作区级 `may_run` | 创建运行记录；看到、预览、应用、取消别人的运行 |
| 有任何授权，再逐实体过滤 | `doc.*` 读取、`ws.get_info`、用户状态、`doc.commit` |

Owner 没有特殊路径：

- 建工作区时，写一条带全部能力的工作区级授权，并在 `local_meta` 里记 owner。
- `ws.revoke` 拒绝删除 owner 的工作区级授权。`ws.grant` 是 `INSERT OR REPLACE`，不检查 owner。
- 没有 Zone 管理员越权的路径。

### 2.5 分享与公开

| 入口 | 谁能建 | 带什么 | 怎样收回 |
| --- | --- | --- | --- |
| 授权行 | 工作区级 `manage` | 任意能力（`public` 不能带 `manage`，`*` 可以） | `ws.revoke`；没有审计记录 |
| 分享入口 | 范围上的 `manage` | 能力模板（不能带 `manage`）；进入方式（访客、密码、自注册、DID 登录）；范围；过期时间 | `ws.close_entry` 让访客失去访问；已加入者的授权写进了各自的授权行，要另行 `ws.revoke` |
| 公开发布 | 工作区级 `manage` | `public` 对一个实体的 `read`，外加 `/pub/<ws>/<name>` 免登录路由 | `ws.unpublish` |
| 演示会话链接 | 会话 owner 或演示者 | `at` 观众、`dt` 显示屏、`pt` 提词器（代表演示者）；没有过期 | `session.revoke_invite` 删除它给的授权行 |
| 放映提词令牌 | 放映开始时生成，只在内存 | `show.command` / `watch` / `notes` | 放映结束 |

### 2.6 执行

| 环节 | 现在的实现 |
| --- | --- |
| 启动 | 许愿格上要有 `update`（本地）或 `run`（托管），并且要有工作区级 `may_run`（`require_run`） |
| 执行身份 | 本地：发起者的浏览器，AI 用发起者的会话；托管：owner 的 Agent，AI 记在 owner 名下。External 和访客不能本地运行 |
| 阶段快照与工具 | 以 `Caller { principal: 触发者, ..Caller::user("") }` 打开 |
| run token | 领取时发放；读取按触发者 principal 的精确授权；不检查运行状态 |
| 应用 | 按应用者的授权检查，写入身份记为触发者。`DELEGATION_STALE` 比较许愿格的 `basis_digest`；`OUTPUT_NOT_DECLARED` 校验落点 |
| 宏 | 浏览器：`sandbox="allow-scripts"` 的 iframe，加 CSP 禁网，再进 Worker，令牌不进沙箱。托管：受限的 Deno 子进程。桥上四种操作：`sql`、`ai.complete`、`llm.map`、`zone.fetch` |
| `zone.fetch` | 浏览器带查看者的 cookie（`credentials: 'include'`），托管带 Agent 的 Bearer；不限方法；`declare.zone` 不强制 |
| 托管配置 | `ws.set_hosted_exec`（`manage`）：Agent、触发者、允许的许愿格、`limits`。Agent 必须是 owner 名下的 |

### 2.7 客户端与 SDK

- **授权面板：**
  - 范围只能选“整个工作区”或“某张画布”。画布授权写两行：画布本身和它的内容文件夹。
  - 主体是文本框加下拉，下拉里有 `public`、`*`、Zone 用户和 `group:<id>`（只显示 ID），没有联系人组和 Agent。
  - 能力有四个预设（只读、评论、编辑、管理），也可以逐项勾选。
- **分享对话框：** “公开”开关只作用于整个工作区，固定为 `read` + `comment`。分享入口界面不能设范围和过期时间。
- **能力来源：**
  - 工作区级能力在打开时取一次，在线会话之后不刷新。
  - 实体能力随 envelope 更新。
  - 离线副本把实体能力与工作区级能力取交集。
- **模式：**
  - 没有存过模式时，没有 `update` 的访客默认查看模式，其他人默认编辑模式。
  - 切换模式不受能力限制。
  - 查看模式下 HTML 控件自动运行，访客也一样（2026-11-12 起）。
- **HTML 控件：**
  - 在同源 iframe 里运行，不设 `sandbox`。
  - `window.aiws` 的所有调用都走查看者的会话。
  - 宿主只检查宿主 Block 本身的写权限，原始 `submit` 由服务端检查。
- **SDK / CLI：** `buckyos aiws` 没有授权、分享、发布、入口的命令。Agent 只能用 `info` / `stat` 看自己的能力。
- **DID 对象入口：** 经服务自己的 kRPC 入口转调，带调用者的凭据，只读。

## 3. 问题

编号前缀：

| 前缀 | 类别 |
| --- | --- |
| S | 可以直接利用的缺陷 |
| I | 身份 |
| E | 执行 |
| C | 可执行内容 |
| V | 可见性与泄露 |
| M | 管理 |
| U | 客户端与接口 |

### 3.1 可以直接利用的缺陷

**S1 固定版本的公开读取不检查归属（推断）。**

- 问题：`GET /pub/<ws>/<name>?fixed=<content_root>` 只检查对象在不在 named store 里，不检查它属于这个工作区的某个发布，`name` 也被忽略。对象库是整个服务共用的。这条路由免登录。
- 依据：[server source.rs](../src/frame/aiworkspace/server/src/source.rs) `published`、store source.rs `public_fixed_content`。
- 后果：知道一个内容根，就能读到任何工作区的那个版本。内核文档 11 规定“ObjId、名称、URL 都不是凭据”。

**S2 开放给访客的入口不要求出示入口 ID。**

- 问题：入口的能力模板加给每一个调用者，不需要出示入口 ID。
- 依据：workspace.rs `access_with`、entries.rs `guest_entry_grants`。
- 后果：
  - 工作区 ID 是 `ws_` + 年月 + 4 位 base36，每月 36⁴ 个，可以枚举。[id.rs](../src/frame/aiworkspace/core/src/id.rs) 的注释说“需要保密的链接另带长参数”，入口 ID 就是这个长参数，但它没有起作用。
  - 不受限账号的 `ws.list` 会列出所有带公开授权或开放入口的工作区。

**S3 URL 数据源不限抓取地址。**

- 问题：只校验 http(s)、不带用户名口令，不限主机。
- 依据：core [types.rs](../src/frame/aiworkspace/core/src/types.rs) `check_absolute_url`、server source.rs `http_get`。
- 后果：有 `update` 的人可以让服务 GET 回环、内网或 Zone 内的地址，把响应写进工作区。D24 定的是“只做无凭据的公开 URL 抓取”。

**S4 管理者能覆盖 owner 的授权。**

- 问题：`ws.grant` 不检查 owner，可以把 owner 的工作区级授权覆盖成更少的能力；`*` 也可以带 `manage`。
- 依据：workspace.rs `grant_with`。
- 后果：管理者能架空 owner。`ws.revoke` 有保护，`ws.grant` 没有。

**S5 run token 不过期。**

- 问题：读取时不检查运行状态和尝试编号。
- 依据：lib.rs `run_token_caller`、runs.rs `wish_run_by_token`。
- 后果：运行结束、取消或被新尝试替代以后，旧令牌仍能按触发者的授权读工作区。

**S6 领取令牌外泄，领取不检查领取者。**

- 问题：托管派发失败时，`claim_token` 写进运行详情，能看到这次运行的人都读得到；`proc.claim` 也不检查领取者是不是配置的 Agent。
- 依据：wish/mod.rs `start`、exec.rs `claim`。
- 后果：拿到令牌的人可以领取运行、交回候选，而候选是按执行者的报告被信任的。

### 3.2 服务内部重建身份

| 编号 | 位置 | 现在 | 问题 |
| --- | --- | --- | --- |
| I1 | 运行阶段的快照、阶段工具、宏的 SQL、预检 | `Caller { principal: 触发者, ..Caller::user("") }` | 重建成不受限账号、没有组：受限账号或 External 触发的托管运行多拿到 `*` 的授权；靠 `group:` / `contact:` 授权的协作者在快照里读不到输入 |
| I2 | run token | 类别是服务 | 不匹配 `*` 和 `group:`，与 I1 又是另一套规则 |
| I3 | 数据源定时刷新 | `Caller::user(owner)` | 以 owner 名义写入可以接受，但没有声明这是代行身份 |
| I4 | 数据源手动刷新的写入、会话操作的写入 | `Caller::user(触发者 / 会话 owner)` | 同 I1 |

内核文档 08 要求“缺失所需身份……不回退到系统服务、设备或其他用户的权限”。重建出来的身份既不是原调用者，也不是明确的系统身份。

### 3.3 执行授权

| 编号 | 问题 | 依据 |
| --- | --- | --- |
| E1 | 启动运行要工作区级 `may_run`。只在某张画布上有编辑权、或只在某个许愿格上有 `run` 的协作者，都启动不了。Desktop 授权面板给出的画布授权正是这种 | workspace.rs `require_run` |
| E2 | 只有 `run` 的协作者走托管执行（D22），这条路走不通（推断，没有用例覆盖）。应用按应用者的授权检查，他没有输出位置的写权限；托管编码阶段为结果暂存文件，要工作区级 `update` | runs.rs `wish_apply`、[export.rs](../src/frame/aiworkspace/store/src/export.rs) `stage_upload`。内核文档 08 写的是“托管执行的写权限上限 = 该 Agent 当前权限 ∩ 已确认输出声明” |
| E3 | 授权上的委托字段（`task_digest`、`outputs`）存了但从不读；运行上下文里的 `outputs` 和托管配置摘要都不复核；`limits` 不执行；上下文 24 小时有效期只在领取时检查 | workspace.rs `run_grants`（只有测试调用）、`set_hosted_exec` |
| E4 | `may_run` 让只有 `run` 的人能看到、应用、取消所有人的运行 | runs.rs `wish_run` |
| E5 | 应用者没有记录，Commit 上记的是触发者 | runs.rs `wish_apply` 的 `as_principal` |
| E6 | `doc.commit` 带上任意一个存在的 `run_id`，就成为 `program` 来源，不检查这次运行是否属于调用者 | workspace.rs `commit` |
| E7 | 落点校验的根包括客户端在 `proc.start` 时给的 `location` 和所有输入实体，输入所在的文件夹因此也可写 | runs.rs `check_output_declaration` |
| E8 | 只有 `run` 的触发者能把 `request`、`feedback` 文字送进 owner 的 Agent 的分析和编码提示词。Agent 随后带自己的令牌和 Zone 网络执行宏，这一步没有人确认（推断） | [stages.rs](../src/frame/aiworkspace/server/src/wish/stages.rs) |
| E9 | 宏的 `zone.fetch` 不限方法，带执行者的凭据（浏览器 cookie、Agent Bearer），`declare.zone` 不强制；AI 预算只在执行者一侧检查 | [host.js](../src/frame/aiworkspace/aiws/host.js)、[localExecutor.ts](../src/frame/desktop/src/app/aiworkspace/executor/localExecutor.ts)、websdk `cli/modules/aiws.ts` |
| E10 | `public` 授权和分享入口模板都可以带 `run`；托管配置的触发者为空时等于所有人，所以匿名访客可以用 owner 的 AI 额度跑托管运行（推断） | entries.rs、workspace.rs `hosted_exec_admits` |
| E11 | AI 判断缓存 `llm_cache` 是工作区共用的，任何执行者都能写入；一个协作者可以污染 owner 运行里的判断结果（推断） | stages.rs |

### 3.4 可执行内容

| 编号 | 问题 |
| --- | --- |
| C1 | HTML 控件同源、不设 `sandbox`，`window.aiws` 以查看者的会话调用服务。依据是第二期规划 D16：“Owner 是信任根”。 |
| C2 | 写入可执行代码只要普通写权限：build 扩展要 `structure` 和 `update`；定义里的 HTML 代码可以用普通 `update` Commit 修改（推断）；on_bind 脚本在 Worker 里运行，也不是边界。 |
| C3 | 2026-11-12 起，查看模式下 HTML 控件自动运行，访客也一样（Q27 修订）。 |

C1–C3 合起来，信任根实际上是“任何有写权限的人”，不是 owner：

- 编辑者写一段脚本；管理者或 owner 打开这张画布，脚本就以他们的身份调用 `ws.grant`、`ws.set_hosted_exec`、`ws.publish`。
- 访客自己的身份价值很低，风险主要在已登录的高权限查看者。

不在此列、或没有核实的部分：

- 许愿格产出的 HTML 已在第一期固定为编码时的代码，首次写入经人确认，不在此列。
- 每个工作区有自己的源（`aiws-<id>.<zone>`）。这个源上的会话令牌能否调用其他工作区，没有核实。

### 3.5 可见性与泄露

| 编号 | 问题 | 依据 |
| --- | --- | --- |
| V1 | `ws.get_info` 把托管配置（Agent、触发者）给任何有授权的人 | workspace.rs `info` |
| V2 | 工作区主机上的页面路由按名字解析画布，不检查权限；未登录者可以确认某个画布名存在，并拿到它的 ID | [pages.rs](../src/frame/aiworkspace/server/src/pages.rs) `any`、store source.rs `surface_by_name` |
| V3 | `POST …/referrers` 不鉴权，任何人都可以登记引用记录 | server source.rs |
| V4 | 阶段工具 `path_of` 不检查祖先是否可读，会打印祖先标题；同级重名报 `NAME_CONFLICT`，会泄露看不见的同级实体的名字 | 权限优化 TODO B-3 |
| V5 | Commit 的幂等重放在权限检查之前回答，被撤权的调用者仍能取回自己以前的结果 | workspace.rs `commit` |
| V6 | `entry.describe` 免登录给出 owner 的名字 | lib.rs |

V5、V6 影响很小。列出来是为了在调用者矩阵（R11）里有明确结论。

### 3.6 演示会话与放映

- 会话链接没有过期时间。
- 撤销观众链接只删它给的授权行。已加入的参与者行还在，会话方法按角色合成能力（`Actor::access`），所以撤销后仍能读会话（store session/mod.rs）。
- `session.create` 的 `presenter` 参数不校验。
- `session.save` 的说明是“owner 或源工作区的管理者”，代码只允许 owner。
- `show.start` 时，只要有写权限，哪怕只在一个子树上，就会锁住整个工作区，所有人都不能写（[show.rs](../src/frame/aiworkspace/server/src/show.rs)；README“Known limitations”里“只看工作区级写权限”的说法已过时）。

### 3.7 管理、客户端与接口

| 编号 | 问题 |
| --- | --- |
| M1 | 授权、入口、发布、托管配置的变更都没有审计；`grants` 表没有 `granted_by` / `granted_at`（权限优化 TODO A-3） |
| M2 | 没有工作区权限组（A-2）：BuckyOS 用户组只能以 `group:<id>` 直接写进授权行 |
| M3 | 重新授权是整行替换，并清空 `via_entry`；范围实体删除后，授权行留着 |
| M4 | 没有 owner 专属操作：管理者可以删除工作区、配置托管执行（花 owner 的 Agent 和 AI 额度）、授予别人 `manage` |
| M5 | 没有“私有子树”：工作区级 `read` 能看到全部内容，又没有 deny。所以 PRD §7.4“用 owner 凭据取得的数据不自动向全部成员开放”在现有机制下做不到，除非成员只拿子树授权（权限优化 TODO C-1） |
| U1 | 授权面板的缺口：范围只能选工作区或画布（A-1）；组只显示 ID；没有联系人组、Agent、工作区权限组；不能修改已有授权；不显示 `via_entry`；托管执行的 Agent 要手填 |
| U2 | 分享入口不能设范围和过期时间；公开开关只有工作区级，复制画布链接时实际公开的是整个工作区 |
| U3 | 在线会话的工作区级能力打开后不刷新，授权变化要等被拒绝或重新打开才体现 |
| U4 | 离线副本把实体能力和工作区级能力取交集，只有子树授权的人离线时会失去能力（推断） |
| U5 | 已登录但只读的人默认进入编辑模式：看到一堆不可用的编辑入口，HTML 控件也不自动运行 |
| U6 | SDK / CLI 没有权限命令，Agent 不能查看谁有权限，也不能代 owner 分享。是否应该能，见 Q12 |

## 4. 需求

### 4.1 身份

**R1 调用者表。**

- 把 §2.2 的调用者种类、可匹配的主体和限制写进内核文档 11，作为正式契约。
- 新增调用者种类时，同时更新这张表和 R11 的调用者矩阵。

**R2 代行身份。** 服务代替某人做事时（运行阶段、数据源刷新、会话写入），使用代行身份：

- 带原调用者的类别、是否受限，以及成员关系的来源。
- 成员关系在使用时重新结算；结算不了按 `authorization_unavailable` 处理。
- 不再用 `Caller::user(x)` 重建身份。
- 确实以服务自己名义做的事（如会话时间线、维护清理），使用明确的内部身份，并在 R1 的表里列出它能做什么。

**R3 run token 的有效期。**

- 只在所属尝试仍是当前尝试、运行未结束时有效；按 R2 的代行身份读取。
- 白名单按 owner 2026-11-12 的裁决保持不变（PRD §6.5、权限优化 TODO B-2）。

**R4 Agent。**

- Agent 以自己的 principal 进入，不匹配 `*` 和 `group:`，维持现状。
- 授权面板可以选 Zone 里的 Agent。
- 与 Agent appid 统一授权（owner 权限 ∩ Agent 配置）的对接，等那份 TODO 实施时再做。

### 4.2 Owner 与管理

**R5 owner 专属操作。** 见 Q2。

- 只有 owner 能做：
  - 修改或删除 owner 自己的授权；
  - 授予或收回 `manage`；
  - 删除工作区；
  - 配置托管执行（用的是 owner 的 Agent 和 AI 额度）；
  - 覆盖式恢复。
- 其余 `manage` 操作，管理者可以做：授予非管理能力、分享入口、公开发布、维护操作。

**R6 广泛主体的能力上限。**

- `public`、`*` 和分享入口模板都不能带 `manage`。
- 能否带 `run` 见 Q8。

**R7 审计（A-3）。**

- 以下变更都写一条只追加的记录，内容是谁、何时、对象、改前、改后：
  - 授权与收回；
  - 入口的建立、关闭、加入；
  - 发布与撤销；
  - 托管配置；
  - owner 专属操作；
  - 权限组成员变更。
- 管理者可读。
- 不随导出传播，与 `local.sqlite` 一致。

**R8 授权行的维护。**

- 授权面板可以修改已有授权的能力；替换语义在接口上写明。
- 范围实体被永久删除（purge）时，一并删除它上面的授权行；普通删除时保留，以便恢复。

### 4.3 授权检查

**R9 方法表。**

- 为全部 kRPC 方法建一张声明式的表，写明：
  - 所需能力；
  - 判断位置（工作区、目标实体、owner、可以拥有）；
  - 允许的调用者类别。
- dispatch 按表检查，README 的方法表由它生成。
- 这与 operations.json 对 Commit 操作的做法一致。

**R10 按实体判断。**

- 下列检查从工作区级改为目标实体：
  - 启动运行：许愿格上的 `update` / `run`。
  - 看到和应用运行：许愿格上的 `update` 或 `manage`，或者是自己发起的运行。
  - 为结果暂存文件：输出位置的写权限。
- 放映加锁：只有工作区级写权限才加全局锁，只有子树写权限的人按只读放映处理。
- 仍需要工作区级检查的，在 R9 的表里写明理由。

**R11 调用者矩阵（B-1）。**

- 对 R1 的每种调用者，测试它能看到什么。覆盖的读取入口：
  - 发现（`ls` / `find`）、文本查询、SQL、路径解析、`body`；
  - 对象入口；
  - 资产路由、发布路由。
- 状态要覆盖：撤权之后、成员关系查不到时、入口关闭之后。
- 访客只能读到公开授权和入口给的范围。

**R12 泄露（B-3、B-4、B-5）。**

- 修 §3.5 的 V1–V4。
- V5：幂等重放之前，先确认调用者仍能访问这个工作区。
- 搜索索引不存可见性，查询时过滤（B-4）。
- SQL 的逻辑表范围等于可读的表（B-5）。

### 4.4 主体与权限组

**R13 工作区权限组（A-2）。**

- 工作区内有稳定 ID 的组，主体写作 `wsgroup:<id>`（名字待定）。
- 成员可以是精确 principal、`group:`、`contact:`；不支持组套组，遇到时显式拒绝。
- 改名不改身份。
- 成员变更需要 `manage`，并进入审计。
- 先解析成员关系，再按同一套 ACL 计算，不形成第二个权限引擎。

**R14 主体显示。** 授权和审计界面按名字显示用户、BuckyOS 组、联系人组、Agent 和权限组，不显示原始 ID。

### 4.5 分享与公开

**R15 入口要出示链接（S2）。** 见 Q6。

- 开放给访客的入口，只对出示了入口 ID 的调用者生效。
- 访客第一次经入口进入时，服务把 `guest:<id>` 与入口绑定（写一条带 `via_entry` 的授权），之后凭这条授权访问。
- 入口关闭时，一并收回这些访客授权。
- 工作区 ID 不再被当作秘密。
- 不受限账号的 `ws.list` 只列直接授予的工作区，不列仅因开放入口才能进入的。

**R16 入口的范围与期限。**

- 界面可以设范围（任意实体）和过期时间。
- 列出经入口加入的人。
- 关闭入口时，可以选择同时收回已加入者的授权。

**R17 公开按范围。**

- 公开开关作用于当前范围：工作区、画布或实体。
- 能力固定为 `read`，可选加 `comment`。
- 分享对话框按当前范围显示是否已公开。

**R18 固定版本的公开读取（S1）。**

- `?fixed=<root>` 只返回本工作区已发布实体的历史版本：`entity_versions` 里查得到，并且该实体当前仍发布或曾以该名字发布。其他一律 404。
- 工作区之间的固定版本数据源走同一检查。

**R19 演示会话。**

- 链接可以设过期时间。
- 撤销观众链接时，经它加入的参与者失去会话访问；或者明确定为“只阻止新加入”。见 Q11。
- `presenter` 必须是 Zone 内的 principal。
- `session.save` 的说明与代码对齐。

### 4.6 执行

**R20 只有 `run` 的协作者（D22）。** 见 Q3。按内核文档 08 实现：

- 托管运行的写入上限是“执行身份的当前权限 ∩ 已确认的输出声明”。应用时按这个上限检查，不按点击应用的人。
- 只有 `run` 的人可以触发、预览，并确认应用自己的托管运行。
- owner 配置托管执行时，给 Agent 写一条授权：范围和能力由配置决定，进入审计。Agent 凭它领取运行。

**R21 只有 `run` 的人只能触发“运行”阶段。** 见 Q4。

- 分析和编码会把触发者的文字交给模型，并产生新的宏版本，所以只开放给许愿格上有 `update` 的人。
- 这同时解决 E8。

**R22 委托与托管配置。**

- 删除授权上没有用到的 `task_digest` / `outputs`（允许 breaking change），以运行上下文为准。
- 应用时复核托管配置摘要（配置变了按 `DELEGATION_STALE` 处理）和上下文有效期。
- 执行 `limits`：每日次数、工具轮数。
- `proc.claim` 只接受配置的 Agent（托管）或发起者本人（本地）。
- 领取令牌不出现在运行详情里。

**R23 记录应用者。** Commit 和运行记录同时记触发者、执行者、应用者。

**R24 `run_id` 的使用。** 只有 `proc.apply` 能写带 `run_id` 的 Commit；`doc.commit` 带 `run_id` 时拒绝。

**R25 落点。**

- 落点的根只来自许愿格的输出声明和它自己的结果。
- `location` 只决定放在哪张画布，并且必须在声明的画布内。
- 输入实体不是可写的根，除非同时被声明为输出。

**R26 宏访问 Zone 的身份与方法。** 见 Q5。建议：

- `zone.fetch` 只允许 GET / HEAD。
- 目标必须写在 `declare.zone` 里，由宿主强制。
- 带执行者的凭据，维持现状。

**R27 AI 缓存按归属隔离。** `llm_cache` 的键加上 AI 归属的用户，一个人的执行者不能写入别人的运行会命中的条目。

**R28 `run` 的上限。** 见 Q8。建议 `public`、`*`、入口模板都不能带 `run`。

### 4.7 可执行内容

**R29 引入代码的门槛。** 见 Q1。

**R30 自动运行的范围。** 与 R29 一起定。建议：

- 只有“已信任”的定义在查看模式下自动运行。
- 其余定义显示快照；点击运行时，说明代码来自谁。

### 4.8 数据源

**R31 抓取地址（S3）。**

- URL 数据源拒绝回环、私网、链路本地、组播地址，以及 Zone 自己的主机。
- 按解析后的 IP 判断；重定向的每一跳都要判断。
- Zone 内的数据走工作区数据源，或以后的 Zone 路径来源，带明确的身份。

**R32 同步程序的授权（PRD §7.4、§7.9，权限优化 TODO C-2）。** 在 ACL 里的对应：

| 操作 | 所需权限 |
| --- | --- |
| 安装、升级同步程序，授予能力范围（域名、凭据、预算、写入的数据源、可读的配置输入） | owner 专属 |
| 修改数据源定义 | 数据源上的 `update` |
| 触发同步 | 数据源上的 `run` |
| 查看同步记录 | 数据源上的 `read` |
| 绑定凭据 | 只有 owner |

随 PRD 第二期实施。

**R33 同步数据的可见范围（C-1）。** 见 Q7。

### 4.9 客户端

**R34 授权面板（A-1）。**

- 从数据树、画布、Block 的菜单进入“共享此对象”，范围就是该实体。
- 主体选择器包括 R14 列出的各种主体。
- 能力用预设加自定义；能修改已有授权。
- 显示授权的来源：直接授予、经入口、经组。
- 管理者能查看审计记录。
- 按手机评估，目标用户以手机为主。

**R35 能力实时刷新。** 授权变化进入变更流；客户端收到后刷新工作区级和实体能力，不需要重新打开。

**R36 离线副本的能力。** 按实体能力计算，不与工作区级能力取交集。

**R37 默认模式。** 见 Q9。没有 `update` 的人（不只是访客）默认进入查看模式。

### 4.10 Agent 与接口

**R38 读。** `buckyos aiws` 提供查看授权、入口、发布的命令：管理者看全部，其他人只看与自己有关的，与 `ws.list_grants` 一致。

**R39 写。**

- 授权、入口、发布的写命令提供给 owner 和管理者使用。
- Agent 调用这些写命令，要经过对象动作的审批门（权限优化 TODO B-6），不能自行扩大访问范围。
- 理由：Agent 读到的工作区内容可能带有注入的指令，“把工作区公开”不能由一段内容驱动。

**R40 对象动作（PRD §8.1）。** B-6 定下以后，对象入口声明启动运行、触发同步等动作，检查与对应的 kRPC 方法相同（R9 的表）。

## 5. 待 owner 裁决

**Q1 谁能引入在别人浏览器里运行的代码（C1–C3、R29）。**

- 选项：
  - A：引入和修改 HTML / JS 代码要 `manage`，定义带“已信任”标记。
  - B：HTML 控件改为与宏相同的 sandbox iframe + 桥，即反转 D16。
  - C：维持 D16，但查看模式只自动运行 owner 引入的定义。
- 建议：先做 A，B 作为以后的方向。
  - A 改动小，能堵住编辑者到管理者的路径。
  - 内核文档 11 已经写了“信任在引入工作区时决定”，A 是把这个决定交给 `manage`。

**Q2 owner 专属操作的范围（R5），特别是管理者能否授予 `manage`。**

- 选项：按 R5 的列表；或只保护 owner 自己的授权行。
- 建议：按 R5 的列表。

**Q3 只有 `run` 的协作者，其托管结果由谁、按谁的权限应用（R20）。**

- 选项：
  - A：按内核文档 08，以“执行身份 ∩ 输出声明”为上限。
  - B：维持现状，只有编辑者能应用，然后改文档。
- 建议：A。

**Q4 只有 `run` 的人能否发起分析和编码（R21）。**

- 选项：能；或只能运行。
- 建议：只能运行。

**Q5 宏访问 Zone 内资源的身份和方法（R26）。** PRD §6.6 把这个问题留给了实施 TODO。

- 选项：
  - A：维持现状，用执行者的全部凭据，任意方法。
  - B：只读方法，并强制 `declare.zone`。
  - C：每次运行发一个只读的 Zone 令牌。
- 建议：B。它仍然是“先能力再安全”，只是关掉写的路径。

**Q6 开放给访客的入口是否要求出示入口 ID（R15）。**

- 选项：要求；或维持“知道工作区 ID 即可”。
- 建议：要求。

**Q7 同步数据怎样不向全部成员开放（R33、M5）。**

- 选项：
  - A：子树可以设“不继承工作区级授权”，只由 owner 设置。这等于一种显式的 deny，要改内核文档 11 的原则。
  - B：不改模型。约定共享时给成员子树授权，而不是工作区级授权，由界面引导。
  - C：推迟到第二期再定。
- 建议：C。第二期定的时候在 A、B 之间选；本期先在授权面板上说明“工作区级授权能看到全部内容”。

**Q8 `public`、`*` 和入口模板能否带 `run`（R28）。**

- 选项：能；不能。
- 建议：不能。

**Q9 已登录、只读的人进入时的默认模式（R37）。**

- 选项：查看；编辑。
- 建议：查看。

**Q10 授权上的委托字段（R22）。**

- 选项：删除；接上。
- 建议：删除。

**Q11 撤销观众链接后，已加入者是否离开（R19）。**

- 选项：离开；或只阻止新加入。
- 建议：离开。

**Q12 Agent 能否管理权限（R39）。**

- 选项：经审批门可以；完全不可以。
- 建议：经审批门可以，与 B-6 一起定。

**Q13 本文与权限优化 TODO 的关系。**

- 选项：本文代替它，Review 后按 §6 拆成实施 TODO；或两份并存。
- 建议：代替。

## 6. 分期

| 步骤 | 内容 | 前提 |
| --- | --- | --- |
| 0 | S1–S6、R31：都是小改动，可以不等 Review | S2 的修法依赖 Q6；S4 先只保护 owner 自己的授权行 |
| 1 | 身份与检查：R1–R3、R5–R7、R9–R12、R22–R24 | Q2、Q10 |
| 2 | 执行：R20、R21、R25–R28，与 AI Native 第一期的后续工作一起做 | Q3、Q4、Q5、Q8 |
| 3 | 可执行内容：R29、R30、R37 | Q1、Q9 |
| 4 | 权限组与界面：R8、R13–R17、R19、R34–R36、R38 | 无 |
| 5 | 随 PRD 第二期做 R32、R33；随 B-6 做 R39、R40 | Q7、Q12、B-6 |

第 1 步的方法表（R9）和调用者矩阵（R11）是后面各步验收的基础，建议先做。

## 7. 验收

| 场景 | 通过条件 |
| --- | --- |
| 固定版本 | 未登录请求 `/pub/<ws>/<name>?fixed=<另一个工作区的内容根>`，得到 404 |
| 开放入口 | 访客和账号只凭工作区 ID、不出示入口 ID，进不去；入口关闭后，经它进入的访客失去访问 |
| 抓取地址 | URL 数据源指向 `127.0.0.1`、私网地址、Zone 主机，或重定向到这些地址时，刷新失败，原因可读 |
| owner | 管理者不能修改 owner 的授权、不能授予 `manage`、不能删除工作区、不能配置托管执行 |
| 代行身份 | 受限账号触发的运行读不到只授予 `*` 的内容；靠 `group:` 授权的协作者触发的运行能读到输入 |
| run token | 运行结束、取消或被新尝试替代后，旧令牌的读取被拒绝 |
| 子树授权 | 只在一张画布上有编辑权的协作者，能启动、预览、应用画布内许愿格的本地运行 |
| 只有 `run` | 协作者能触发托管运行、预览并应用，结果只落在输出声明之内；不能发起分析和编码；看不到别人的运行 |
| 可执行内容 | 按 Q1 的结论：编辑者写入的未信任 HTML，不在管理者的查看模式里自动运行 |
| 宏 | `zone.fetch` 的 POST、或访问未声明的 Zone 路径，被拒绝（按 Q5 的结论） |
| 审计 | 每次授权、入口、发布、托管配置的变化，都能查到谁、何时、改前、改后 |
| 调用者矩阵 | R11 的矩阵全部有用例，并进入 CI |
| 界面 | 在手机上能把一张表单独授权给一个 BuckyOS 组；授权变化在已打开的页面上立即体现 |

## 8. 与已有文档的关系

权限优化 TODO 各条目的去向：

| 条目 | 本文 |
| --- | --- |
| A-1 单个 Block / 数据源授权 | R34 |
| A-2 工作区权限组 | R13、R14 |
| A-3 审计与成员缓存 | 审计见 R7。成员关系维持每次请求现查、不加缓存，这与内核文档 11 “权威来源不可用时不得沿用旧权限”一致 |
| B-1 调用者矩阵 | R11 |
| B-2 run token 白名单 | 维持 owner 裁决，不收紧；R3 只加有效期 |
| B-3 泄露 | R12 |
| B-4 索引的可见性、B-5 SQL 的授权边界 | R12 |
| B-6 审批门 | R39、R40、Q12 |
| C-1 同步数据的可见范围 | R33、Q7 |
| C-2 同步程序的授权主体 | R32 |

Review 通过后要回写：

- 内核文档 11：调用者表（R1）、owner 专属操作（R5）、方法表的位置（R9）、入口的语义（R15）、引入代码的门槛（R29）。
- 内核文档 08：只有 `run` 的协作者的应用权限（R20、R21），宏访问 Zone 的方法（R26）。
- 内核设计 README 的 GAP“权限”一节。
- 服务 README：放映加锁的说明（§3.6）。
- PRD：§6.6 的“以什么身份访问 Zone 内资源”（Q5），§12 的“实体级可见范围”（M5）。

## 9. 源码入口

服务端：

- 能力与可见范围：[core/src/access.rs](../src/frame/aiworkspace/core/src/access.rs)
- 调用者、授权、托管配置、Commit 授权：[store/src/workspace.rs](../src/frame/aiworkspace/store/src/workspace.rs)
- 分享入口：[store/src/entries.rs](../src/frame/aiworkspace/store/src/entries.rs)
- 公开发布、数据源刷新：[store/src/source.rs](../src/frame/aiworkspace/store/src/source.rs)、[server/src/source.rs](../src/frame/aiworkspace/server/src/source.rs)
- 身份解析、方法分派、run token：[server/src/lib.rs](../src/frame/aiworkspace/server/src/lib.rs)、[server/src/auth.rs](../src/frame/aiworkspace/server/src/auth.rs)
- 许愿格运行：[server/src/wish/mod.rs](../src/frame/aiworkspace/server/src/wish/mod.rs)、[server/src/wish/exec.rs](../src/frame/aiworkspace/server/src/wish/exec.rs)、[server/src/wish/stages.rs](../src/frame/aiworkspace/server/src/wish/stages.rs)、[store/src/wish/runs.rs](../src/frame/aiworkspace/store/src/wish/runs.rs)
- 演示会话与放映：[store/src/session/mod.rs](../src/frame/aiworkspace/store/src/session/mod.rs)、[server/src/show.rs](../src/frame/aiworkspace/server/src/show.rs)
- 宏的桥：[aiws/host.js](../src/frame/aiworkspace/aiws/host.js)

Desktop：

- 授权与分享：[ui/sources/PermissionsPanel.tsx](../src/frame/desktop/src/app/aiworkspace/ui/sources/PermissionsPanel.tsx)、[ui/shell/ShareDialog.tsx](../src/frame/desktop/src/app/aiworkspace/ui/shell/ShareDialog.tsx)、[ui/shell/ShareEntries.tsx](../src/frame/desktop/src/app/aiworkspace/ui/shell/ShareEntries.tsx)
- 模式：[ui/shell/shellContext.ts](../src/frame/desktop/src/app/aiworkspace/ui/shell/shellContext.ts) `useCanvasMode`
- HTML 控件：[ui/extensions/htmlRuntime.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/htmlRuntime.ts)、[ui/extensions/bridge.ts](../src/frame/desktop/src/app/aiworkspace/ui/extensions/bridge.ts)
- 宏的执行器：[executor/programHost.ts](../src/frame/desktop/src/app/aiworkspace/executor/programHost.ts)、[executor/localExecutor.ts](../src/frame/desktop/src/app/aiworkspace/executor/localExecutor.ts)

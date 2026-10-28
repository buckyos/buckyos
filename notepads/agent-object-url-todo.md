# Agent 对象 URL 与 Object Runtime 接线 TODO

日期：2026-10-27（v0.1，GAP review 后的开工稿）

状态：**§1 URL 设计待拍板（D1–D9）。URL 定了之后 §4 各组件可独立并行实现，§3 Agent 侧接线与 §4 互不阻塞。** 上游设计是 [基于Agent元能力的Agent Runtime设计.md](../doc/opendan/Agent元能力/基于Agent元能力的Agent%20Runtime设计.md) §6（Object Runtime：`read / list / call / subscribe / resolve`，`read(object_id)` 返回给 Agent 的 object document）；协议是 [Agent DID-Object Protocol Spec.md](../doc/opendan/protocol/Agent%20DID-Object%20Protocol%20Spec.md)（v0.3）；URL 规范是 [key url.md](../doc/key%20url.md)。本文不重复论证，只回答：URL 长什么样、谁解析、每个组件要暴露什么、Agent 侧怎么接。

## 0. GAP review 结论（2026-10-27 核对代码）

两头的骨架都在，中间一根线没接；Provider 侧 0/5。

| 层 | 已有 | 缺 |
| --- | --- | --- |
| URL 规范 | [key url.md](../doc/key%20url.md)：`https / file / cyfs / obj / buckyos` 五种 scheme、`/` `//` 缩写、`obj://$service`（config / dfs / taskmgr / kmsg / bucket）、带类型实体 id、RBAC 资源用同一套 url | msg-center / aiworkspace / homestation / opendan 的对象没有 url；`obj://` 到 canonical Object URL 的解析规则没写 |
| 协议 | DID Object Protocol：`{object_url}/did.json`、`/profile.json`、`/props/<n>`、`/methods/<n>`、`/events`；buckyos-base 有 `DIDObjectCard` / `ObjectProfile` / `DIDObjectHttpServer` / `DIDObjectServer` trait / `DIDObjectClient` | buckyos/src 下**没有一个** `impl DIDObjectServer`（只有 buckyos-base 的 camera 测试） |
| Agent 库 | [agent-did-object-lib](../src/frame/agent-did-object-lib/src/)：`read / x_call / subscribe_event`、路由配置、adapters（filesystem / web / did_object / local_http）、五段式 LLM 输出、session 级 unchanged 去重、EventBridge | `agent_runtime` adapter 是空桩；默认路由只有 `file / http(s) / agent://`，**没有 `obj://` 与 `cyfs://`**；没有 resolve 层 |
| 运行时接线 | dev CLI `agent_tool read` / `agent_tool xcall` 走对象库（[agent_tool_cli_dev lib.rs:59-170](../src/frame/agent_tool_cli_dev/src/lib.rs)） | Session 原生工具组 `bash_core` 注册的是 `agent_tool::read_tool::ReadTool`（[xllm.rs:4555](../src/frame/agent_tool/src/xllm.rs)），只认 path / `file://` / `cyfile:` `chunk:` 裸 id，其他 scheme 报 `InvalidArgs`；**两套同名 `read` 行为不一致，LLM 拿到的是窄的那个** |
| Hint → read | Memory `Hint.reference` = `item_x@3` / `<sid>:<seq>`，`sources` 是事件 id；`agent-session recall <tag>`、`agent-memory read`、`agent-session read-session <sid>` | Hint 不携带对象 url；`hint.id → read(object_id)` 在类型层面就不通；Prompt 环境的 known objects 只是 `paths.*` 文件路径 |
| 订阅 | Session 配置 `subscriptions`（source = session / object_event / task / timer / system）+ KEvent / Timer bridge | `<subscribe_event>` 只在 `xml_behavior` 里是合法 action，agent_tool / libopendan 没有实现；对象库的 EventBridge 没接任何服务 |

各组件现状（细节见 §4 各小节）：

| 组件 | 稳定标识 | Agent 可用的读 | LLM 文本 | 事件 | 距离 |
| --- | --- | --- | --- | --- | --- |
| OpenDAN AgentSession | `sid`，无 url | owner 侧 kRPC `session.read / session.worklog`；Agent 自己 `agent-session read-session`；`round_history/` 文件 | 否 | 输入总线 + KEvent / Timer bridge | 中 |
| OpenDAN Workspace | `ws-<uuid>`，`WorkspaceMetadata.url` 可选 | 文件工具；`home.workspaces` | 否 | 无 | 中 |
| msg-center 会话 / 联系人 | `MailboxAddress` = `<owner did>/<sid>`；`obj://msg-center/...` **只是 RBAC key**；群组有 `cyfs://$host/<group_did>/sessions/<sid>` | `msg.list_sessions / list_session / get_message`、`contact.get_contact / list_contacts`、`GET /kapi/msg-center/objects/<id>[/content]` | 否（OpenDAN 自己渲染 XML，只看自己 inbox） | kevent `/msg_center/<owner>/<BOX>/<hex>/changed`、`CONTACTS/changed`、`group/<did>/changed` | 大 |
| AI Workspace | `ws_…` + JSON `Reference{entity_id, selector, mode, version}`（D10 已冻结）；**无 url** | kRPC `doc.read / query / outline / resolve`；`buckyos aiws` 透传 JSON | 仅 wish run 内 `WORKSPACE.md` + `ws_read / ws_query`，要 `proc.claim` | kevent `/aiworkspace/<ws>/head` 仅唤醒；`doc.wait_changes` | 中偏大 |
| HomeStation | **最好**：`cyfs://<zone>/home/<user>[/feed|inbox]`、`.../feed/@/<key>`；版本是 `cyfeed / cyfhead` ObjId | HTTP `/home/<user>/feed`、`/objects/<id>`；kRPC `item.get / published.list` | 否 | **无 kevent**，只有 `?mode=changes&since=` 游标 | 中 |

安全旁注（子代理读代码所得，未运行时验证）：msg-center 的 owner 绑定检查只对 User token 生效，App / Agent token 落到 `_ => Ok(())`（[owner_session.rs:797-814](../src/frame/msg_center/src/owner_session.rs)），RBAC 又给 `agent_runtime` / `agent` 组 `obj://msg-center/*`。Agent 今天只能读自己 mailbox 是 OpenDAN 代码限制，不是服务策略。接通 read 之前要先收口（§5 G1）。

## 1. URL 设计（待拍板）

### 1.1 三层：语义 URI → canonical Object URL → Object DID

```text
语义 URI（Agent 书写、Hint 携带、RBAC 资源、Memory 保存）
  obj://<service>/<path>          zone 内、按服务归属的对象
  cyfs://<zone>/<path>            可跨 zone 寻址（联合）的对象
  obj://<typed_id>                带类型前缀的裸 id 简写
        │ resolve（Gateway 层，§1.3）
        ▼
canonical Object URL（协议 wire，DID Object Protocol）
  https://<host>/<prefix>/<path>  可直接 GET {url}/did.json 等
        │ did.json
        ▼
Object DID（可验证身份，Card.id）
```

规则：
- **Agent 只写语义 URI**。Hint、Memory、Notebook、报告产物、RBAC policy 都用语义 URI；canonical URL 是 resolve 的结果，Agent 不手写。
- 协议 §4.1 已定：`obj://` 不是 wire endpoint，必须先解析成 Object URL。
- `key url.md` 已定的缩写沿用：`//config/x` = `obj://config/x`，`/abs` = `file:///abs`，`cyfs:///p` = `cyfs://<当前 zone>/p`。

### 1.2 待拍板决策

| # | 决策 | 建议 | 备选 / 说明 |
| --- | --- | --- | --- |
| D1 | **`obj://` 与 `cyfs://` 的分工** | `obj://<service>/...` 用于 zone 内、owner 作用域的对象；`cyfs://<zone>/...` 用于本来就要跨 zone 寻址的对象（HomeStation 的 home / feed / entry，msg-center 群组）。**一个对象只有一种语义 URI**，不做双写 | 备选：全部 `obj://`，HomeStation 改名。不建议：HomeStation 的 `cyfs://` 已是联合协议的一部分（`EntryRef::parse`、inbox 投递） |
| D2 | **`obj://` 的 host 段 = 服务名** | 沿用 `key url.md`：`obj://msg-center/...`、`obj://aiworkspace/...`、`obj://homestation/...`（仅 zone 内快捷视图，见 §4.5）、`obj://opendan/<agent_id>/...`、`obj://taskmgr/...`、`obj://bucket/...`。服务名 = `/kapi/<service>` 的 service 名，一张表维护在 `key url.md` | 现有 `obj://msg-center/{inbox,sent,...}/<addr>` 只是 RBAC key，要与实体路径对齐（§4.3） |
| D3 | **canonical Object URL 的形态** | `obj://<service>/<path>` → `https://<zone_host>/kapi/<service>/obj/<path>`；`cyfs://<zone>/<path>` → `https://<zone_host>/<path>`。固定 `/obj/` 子命名空间，不与各服务现有 kRPC POST 根路径、HTTP 路由冲突，网关不需要新规则 | 备选：每个对象单独子域（`https://<objid>.<zone>`），寻址干净但网关证书与路由成本高，不建议 v1 |
| D4 | **Agent 自己的对象怎么命名** | `obj://opendan/<agent_id>/sessions/<sid>`、`/workspaces/<ws-id>`、`/goals/<gid>`、`/artifacts/<aid>`、`/skills/<kind>/<name>`；`agent://self/...` 作为 `obj://opendan/<自己>/...` 的别名，仅在 Agent 自己的 Session 内有效。canonical：`https://<agent_id>.<zone_host>/kapi/opendan/obj/...`（Agent 是 app 实例，已有子域） | 备选：host 段写 `agent` 而不是 `opendan`。按 D2 服务名规则取 `opendan` |
| D5 | **带类型 id 的简写** | `obj://<typed_id>` 按前缀路由：`cyfile: / chunk: / cymsg: / cyfeed: / cyfhead:` → NDM / bucket / msg-center / homestation；`task_` → taskmgr；`ws-` → opendan workspace；`ws_` → aiworkspace。**`ws-` 与 `ws_` 只差一个字符，建议 aiworkspace 或 opendan 改前缀**（aiworkspace 的 `ws_` 面向用户已暴露，建议 opendan 的 workspace 改 `ows-`） | 前缀表与 D2 的服务名表放一起维护 |
| D6 | **对象内的"路径 / 视图 / 版本"怎么写** | 子资源走路径：`.../sessions/<sid>/report`、`.../sessions/<sid>/worklog`；版本与选择器走 query：`?version=<n>`、`?selector=<...>`、`?mode=live|fixed`（直接对应 aiworkspace Reference D10 的字段）；协议视图走协议后缀：`{url}/profile.json`、`{url}/props/<name>` | `read(url)` 默认返回 object document（§2.1），不是 profile 全文 |
| D7 | **Object DID 形态** | zone 内对象：`did:web:<zone_host>:kapi:<service>:obj:<path 段>`（与 D3 一一对应，可由 zone_did_resolver 直接回答）；内容寻址对象：`did:bns:<named_objid>.<zone>`（协议 §4 已列） | v1 只要求 Card 里有 `id` 且与 url 对应；签名验证按协议"当前实现"小节不作为 v1 门槛 |
| D8 | **RBAC 资源 = 语义 URI** | 新对象的 RBAC 资源直接写语义 URI（`key url.md` 已这样做）。各组件的 `profile.json` 里每个 property / action 标 `x-buckyos:rbac`（资源、action），让 Governance 能从 profile 推出"可见不等于可用" | msg-center 现有 RBAC key 要迁到实体路径（§4.3） |
| D9 | **事件名与 kevent 的映射** | profile `events` 里声明的事件名，统一映射到 kevent id `/<service>/obj/<path>/<event>`；各组件在内部把现有 kevent（`/msg_center/.../changed`、`/aiworkspace/<ws>/head`）转发或重命名到这个形态。事件是 best-effort 唤醒，不是 MQ（协议 §10、需求 §2） | HomeStation 当前没有 kevent，要新增 |

### 1.3 Resolve 层（谁解析）

- 位置：`agent-did-object-lib` 之前加一层 **zone resolver**（协议 §1.1 说这是 Gateway 第一层职责，不能下沉到 router / adapter）。实现落在 `buckyos-api`（Rust）与 websdk（TS）各一份纯函数，同一张表：
  - `obj://<service>/<path>` → `https://<zone_host>/kapi/<service>/obj/<path>`（service 是 app 实例时取 `https://<appid>.<zone_host>/kapi/<service>/obj/...`，用现有 `get_zone_service_url` / app url 规则）；
  - `cyfs://<zone>/<path>` → `https://<zone_host>/<path>`，`cyfs:///p` 补当前 zone；
  - `obj://<typed_id>` → 查前缀表展开成 `obj://<service>/...` 再按上条；
  - `did:*` → zone_did_resolver → Card → `alsoKnownAs` 里的 Object URL；
  - `https://` 原样；`file://` 原样。
- 产物：`ObjectRouteConfig` 增加 `match_type = "scheme"` 的 `obj` / `cyfs` 路由，adapter 统一是 `did_object`（因为解析后就是标准 Object URL）；只有当某服务**暂时**不实现 DID Object 端点时，才给它配 `local_http` / 专用 adapter 过渡。
- `key url.md` 新增一节"obj:// 到 Object URL 的解析规则"，服务名表 + 类型前缀表作为唯一来源。

## 2. 组件暴露的最小契约（每个组件都一样）

### 2.1 必须实现

在 `/kapi/<service>/obj/<path>`（或 cyfs 路径）下实现 `DIDObjectServer`，走 buckyos-base 的 `DIDObjectHttpServer`：

| 端点 | 内容 | 最低要求 |
| --- | --- | --- |
| `GET {url}/did.json` | DID Object Card（DID Document） | `id`（D7）、`controller` = owner DID、`alsoKnownAs` 含语义 URI 与 canonical URL、`service` 指向 profile |
| `GET {url}/profile.json` | Object Profile（constrained WoT TD） | `properties`（可读状态）、`actions`（可调方法，先只列**只读 / 幂等**的）、`events`（D9）、`x-buckyos:traits`（集合类对象实现 `IndexTrait`：`query` / `page`） |
| `GET {url}/props/<name>` | 属性值 JSON | 带 `ETag` / 版本（协议 §10.1） |
| `POST {url}/methods/<name>` | kRPC 风格 action | v1 可以只做只读 action；写 action 必须在 profile 标 `x-buckyos:risk` 与 `x-buckyos:rbac`（D8） |
| `GET {url}/events` | 事件声明；订阅走 kevent（D9） | WebSocket lifecycle 不作 v1 门槛（协议"当前实现"小节） |

`read(url)` 的 object document 由 `agent-did-object-lib` 的 `did_object` adapter 从 Card + Profile + 若干关键 props 渲染（五段式），**组件不需要自己产出 LLM 文本**。组件要做的是：Card / Profile 字段写得像给人看的（`title`、`description`、每个 property / action 一句话说明），以及给 profile 加一个 `x-buckyos:summary_props` 列出"read 默认带哪些 props"。

### 2.2 集合与发现

- 每个服务的根 `obj://<service>/` 是一个实现 `IndexTrait` 的对象（Runtime 设计 §6.1 的 Indexer 也是 object）：`query` 列出该服务下调用者可见的顶层对象。
- Agent 的 known objects（Runtime 设计 §6.3）从这里出发：当前 Session、Workspace、Agent root、`obj://msg-center/`、`obj://aiworkspace/`、`cyfs:///home/<owner>`。

## 3. Agent 侧接线（与 §4 并行）

| # | 事项 | 落点 | 说明 |
| --- | --- | --- | --- |
| A1 | **统一 `read`**：`bash_core` 的 `read` 换成 `AgentDIDObjectReadTool`；filesystem adapter 复用 `agent_tool::read_tool` 的目录策略、`range / first_chunk / offset / limit`、`cyfile:` 对象读（需求 §10.1 要求复用，不另造文件安全模型） | [xllm.rs `builtin_bash_group`](../src/frame/agent_tool/src/xllm.rs)、[agent-did-object-lib tool.rs](../src/frame/agent-did-object-lib/src/tool.rs)、[filesystem.rs](../src/frame/agent-did-object-lib/src/adapters/filesystem.rs) | 结果契约仍是 `AgentToolResult`；`read_token_limit` 走 `max_tokens`。手册 [build-in agent-tool手册.md §2](../doc/opendan/build-in%20agent-tool手册.md) 同步改 |
| A2 | **Resolve 层**（§1.3）+ `obj` / `cyfs` 路由 | 新 `buckyos-api::object_url`（Rust）、websdk `object_url.ts`；`ObjectRouteConfig` 默认配置 | 单元测试覆盖 D1–D6 的每条规则 |
| A3 | **`agent_runtime` adapter 落地或退役**：按 D4，Agent 自己的对象也走 `obj://opendan/...` 的 DID Object 端点，`agent://self/...` 只是 resolve 阶段的别名展开 → adapter 可删，减少一套实现 | [agent_runtime.rs](../src/frame/agent-did-object-lib/src/adapters/agent_runtime.rs) | 需要 §4.1 的 opendan 端点先有 |
| A4 | **Hint 带 url**：`Hint` 加 `object_url: Option<String>`；感知记录（`PerceptionRecord`）引用外部对象时保存语义 URI；`agent-session recall` 输出带 url；`<hints>` 渲染里给出"可 read" 的指引 | [memory/hint.rs](../src/frame/lib_opendan/src/memory/hint.rs)、`state/perception.rs`、`runner/observe.rs`、`protocol/perception_record.schema.json` 重生成 | Memory 内容规则不变：hint 不是事实，read 才是 |
| A5 | **known objects 注入**：Prompt 环境新增 `objects.*`（当前 session / workspace / agent 根 / 各服务根的语义 URI），替代只给文件路径 | [Agent Enviroment.md](../doc/opendan/Agent%20Enviroment.md) §3、`runner/assembler.rs` | 只注入 url 与一句话，不注入内容（Runtime 设计 §4.4） |
| A6 | **`subscribe_event` 工具**：实现为 Session 的 `subscriptions` 增加一条 `object_event` 来源，事件 key 按 D9 的 kevent id；对象库 `EventBridgeManager` 接 libopendan 的 KEvent bridge | `agent_tool` 新工具、[bridge/kevent.rs](../src/frame/lib_opendan/src/bridge/kevent.rs)、[runner/inputs.rs](../src/frame/lib_opendan/src/runner/inputs.rs) | 半订阅（每次推理前观察）按 Session 架构 §15.5 的既有差距另行推进，本项只做"按 url 订阅 → 唤醒" |
| A7 | **`xcall` 进运行时**：`x_call` 作为原生工具暴露，Governance 按 profile 的 `x-buckyos:risk / rbac` 做 gate（allow / require_approval / deny） | agent_tool 新工具、libopendan `classify_effect` | 先只放行 profile 标为只读 / 幂等的 action |
| A8 | websdk / `buckyos` CLI 增加 `buckyos obj read <url>`、`buckyos obj resolve <url>`，供 cmd_skills 与 Deno 脚本使用（Agent 工具优先 TS + websdk 的原则） | buckyos-websdk `cli/modules/obj.ts` | 复用 A2 的 TS resolve |

llm_context / agent_tool 层的改动（A1、A6、A7 的工具部分）按惯例单列到 `llm-context-agent-runtime-todo.md` 先行。

## 4. 各组件实施（URL 定稿后可独立开工）

每个组件的任务都是同一模板：① 确认对象路径表 ② `impl DIDObjectServer` 挂到 `/kapi/<service>/obj/` ③ Card / Profile ④ props 映射到现有读接口 ⑤ events 映射到 kevent ⑥ RBAC 对齐 ⑦ 一个 `read(url)` 的 DV 用例。下面只写各自的对象路径建议与特殊点。

### 4.1 OpenDAN（AgentSession / Workspace / Goal / Artifact）

| 对象 | 语义 URI（建议） | props 来源 | 说明 |
| --- | --- | --- | --- |
| Agent 自己 | `obj://opendan/<agent_id>` | `agent.profile`、`agent.info` | 根对象，`IndexTrait` 列 sessions / workspaces / goals |
| Session | `obj://opendan/<agent_id>/sessions/<sid>` | `session.read`（entry / state / statistics / children）；`/report` 子资源 = `report.md`；`/worklog?turn=` = `session.worklog` | 读权限沿用现有 owner / root 规则 + Agent 自己；`agent-session read-session` 改为薄壳 |
| Workspace | `obj://opendan/<agent_id>/workspaces/<ows-id>` | `state/workspaces/<id>.json`（name / description / runtime / url / lease） | D5 建议改前缀 `ows-`；目录内容仍走 `file://` |
| Goal | `obj://opendan/<agent_id>/goals/<gid>` | `state/goals/` | — |
| Artifact | `obj://opendan/<agent_id>/artifacts/<aid>` | `artifacts.head / versions` | 文件本体继续用 `obj://bucket/...` / `cyfile:` |

落点：[opendan service.rs](../src/frame/opendan/src/service.rs) 旁新增 `objects_http.rs`；`--dev` 形态也要有，便于 libopendan 测试。

### 4.2 msg-center 会话 / 消息 / 附件

| 对象 | 语义 URI（建议） | props 来源 |
| --- | --- | --- |
| 某人的会话集合 | `obj://msg-center/<owner_did>/sessions` | `msg.list_sessions`（IndexTrait） |
| 会话 | `obj://msg-center/<owner_did>/sessions/<session_id>` | `msg.get_session_state`；`/messages?before=&limit=` = `msg.list_session` |
| 消息 | `obj://cymsg:<id>`（D5 简写）→ `obj://msg-center/messages/<msg_id>` | `msg.get_message`；现有 `GET /objects/<id>` 可直接挂为 props |
| 附件 | `obj://<cyfile:...>`，读权仍按消息上下文（bucket 设计 §4.4 / Q5） | 现有 `GET /objects/<id>/content` |
| 群组 | 保留 `cyfs://<zone>/<group_did>/...`（D1） | 已有 `/did.json`，补 `profile.json` |

特殊点：现有 RBAC key `obj://msg-center/{inbox,sent,group_inbox,request_box}/<addr>` 改为实体路径（D8）；App / Agent token 的 owner 绑定检查补齐（§5 G1）后，才开放"Agent 读 owner 的其他会话"。

### 4.3 msg-center 联系人（Contact Mgr）

| 对象 | 语义 URI（建议） | props 来源 |
| --- | --- | --- |
| 联系人列表 | `obj://msg-center/<owner_did>/contacts` | `contact.list_contacts`（IndexTrait） |
| 联系人 | `obj://msg-center/<owner_did>/contacts/<contact_did>` | `contact.get_contact`、`contact.get_preferred_binding`；Card 的 `alsoKnownAs` 里放对方 DID，Desktop 的 `ObjectPresentation`（title / icon）也从这里取 |

### 4.4 AI Workspace

| 对象 | 语义 URI（建议） | props 来源 |
| --- | --- | --- |
| 工作区 | `obj://aiworkspace/<ws_id>` | `ws.get_info`；IndexTrait = `doc.outline` |
| 实体（Reference D10 的 URL 投影） | `obj://aiworkspace/<ws_id>/entities/<entity_id>?selector=&mode=live|fixed&version=` | `doc.read`、`doc.relations`、`doc.freshness` |
| 按路径 | `obj://aiworkspace/<ws_id>/path/<...>` → `doc.resolve` 后 302 / 等价于实体 url | — |
| 发布页 | 现有 `https://aiws-<id>.<zone>/<name>` 不变，Card 的 `alsoKnownAs` 互指 | — |

特殊点：wish run 内已有的 `ws_read / ws_profile / ws_outline` 文本就是现成的 `x-buckyos:summary_props` 材料，抽到 run 之外复用，不再要求 `proc.claim`。`Reference` JSON 仍是内核真相，URL 只是投影；两者互转放 `aiworkspace/schemas` 旁的共享 TS/Rust 函数。

### 4.5 HomeStation

| 对象 | 语义 URI | props 来源 |
| --- | --- | --- |
| 主页 | `cyfs://<zone>/home/<user>`（已有） | `/home/<user>/profile`、`published.list` |
| Feed | `cyfs://<zone>/home/<user>/feed` | `/home/<user>/feed?mode=display|changes`（IndexTrait，`page` 用 `since` 游标） |
| 条目 | `cyfs://<zone>/home/<user>/feed/@/<key>`（已有 `EntryRef`） | `item.get`、Head JWT；版本 = `cyfeed` ObjId |
| zone 公共流 | `cyfs://<zone>/home/~zone` | 已有 |

特殊点：canonical URL 就是现有 `https://<zone>/home/...`，只需在这些路径下补 `did.json / profile.json / props`；**新增 kevent**（D9：`/homestation/obj/home/<user>/feed/changed`）；补 websdk 客户端与 `buckyos homestation` CLI（今天 Agent 唯一入口是 libopendan 内部 `ZoneShare`）。

## 5. Governance 前置项

| # | 事项 | 说明 |
| --- | --- | --- |
| G1 | msg-center `owner_session.rs` 对 App / Agent token 做 owner 绑定检查（Agent 的 App token `sub` = owner，只允许读该 owner 作用域） | 接通 §4.2 之前必须做；加 `nfs_server/tests/ndm.rs` 式的真实 token 形态测试 |
| G2 | `profile.json` 的 `x-buckyos:rbac / risk` 字段进协议 Spec（§7.4 扩展命名空间） | A7 的 gate 依赖它 |
| G3 | `read` 的审计：对象库 `RouteTrace` 进 worklog | 低成本，随 A1 做 |

## 6. 顺序建议

1. 拍板 D1–D9，回写 `key url.md`（服务名表 + 类型前缀表 + 解析规则）与协议 Spec §4.1。
2. A2（resolve + 路由）与 A1（统一 read）先做：没有任何 Provider 时，`read obj://bucket/...`、`read file://`、`read https://` 已经能走同一入口，`cyfile:` 不回退。
3. §4.1 OpenDAN 与 §4.5 HomeStation 先做 Provider（标识最稳、改动最小），用它们跑通 `hint → read(url)` 与 `subscribe_event` 的 DV。
4. G1 → §4.2 / §4.3 msg-center；§4.4 AI Workspace 并行。
5. A4 / A5 / A6 / A7 跟随，A8 随第一个 Provider 一起出。

## 7. 验收（全部按真实 Zone DV，全新安装）

- [ ] Jarvis 在聊天里收到一条带 `obj://aiworkspace/...` 的消息，`read` 返回 object document（五段式），再 `read .../props/...` 读到内容。
- [ ] Memory hint 带 `object_url`，Agent 据 hint `read` 到 HomeStation 的某条 entry。
- [ ] `subscribe_event obj://opendan/<agent>/sessions/<sid> finished` 后，子 Session 结束唤醒父 Session。
- [ ] 普通用户的 Agent `read obj://msg-center/<其他用户>/sessions/...` 被拒绝（G1）。
- [ ] `agent_tool read` 与 Session 内 `<read uri=.../>` 对同一 url 输出一致（A1）。

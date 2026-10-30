# Agent 对象 URL 与 Object Runtime 接线 TODO

日期：2026-10-29（v0.4，Q1–Q12 已裁决，全文按裁决定稿）

状态：**§6 第 1、2 段已完成并提交（第 2 段 DV 见其小节）；第 3 段（Agent 工具，A1–A7、A9 读侧）2026-10-29 已实施、本机单 Zone DV 通过，未提交——记录、DV 与遗留见 §6 第 3 段。**§1.5 的裁决列已填，D1–D12 是定稿规则。第 1 段产出：[key url.md](../doc/key%20url.md) 第一部分重写（§1–§10：scheme、`file://<device_id>`、`obj://` host 两类规则与服务名表、`cyfs://` 判据、默认 URL / 跨 Zone https / query 契约、kevent 命名、缩写、非等价转换、RBAC 指引）；[统一url的思考](../doc/opendan/protocol/统一url的思考.md) 删展开与简写写法并指向 key url.md；buckyos-base [基础协议](../../buckyos-base/doc/did-object-protocol.md) §5.2 / §5.4 补 BuckyOS Object DID 与 `obj://` host 约定；[Agent Spec](../doc/opendan/protocol/Agent%20DID-Object%20Protocol%20Spec.md) §4 / §4.1 / §14 对齐 Card 发现与 `obj://` 规则并记录 client 差距；[Agent Session SDK 实现计划](../doc/opendan/Agent%20Session%20SDK%20实现计划.md) A.7 的 `agent://self` 改为删除；[RBAC 权限设计](../doc/arch/verify-hub/BuckyOS%20RBAC权限设计.md)"基于 URL 的资源 key 布局"一节（各服务 key 表、agent 组目标规则、risk↔action 对应表、第 2 段 policy 改动清单）。**运行代码未改。**

v0.3 相对 v0.2 的变化（均为 2026-10-29 对照代码核出）：① HomeStation / msg-center 群组的 `cyfs://` 投递 target 来自 cyfs-ndn 共享的 dispatch 协议，cyfs-gateway 也在消费，改 OBJ 必须先裁决传输层去留（新 Q11）；② msg-center 对 agent 组的 RBAC 是 `all`，G1 扩为 RBAC 收窄；§4.2/4.3 对象路径与现有 RBAC key 布局不同（新 Q12）；③ Q1 被 `key url.md` 已有的 `file://$device_id/path` 收窄；Q4 有 sys_config_service 别名先例；Q5 有 Agent Session SDK 实现计划的 `agent://self` 先例；④ Card 发现漂移在 client 的 `normalize_object_url` 先去尾斜杠处，§2.1 / §7.1 补上；⑤ 工作树里 `doc/key url.md` 有一行未提交改动（`obj://` 行加"最大兼容"），随本 TODO 一起提交。

上游设计是 [基于Agent元能力的Agent Runtime设计.md](../doc/opendan/Agent元能力/基于Agent元能力的Agent%20Runtime设计.md) §6；Agent 侧说明是 [Agent DID-Object Protocol Spec.md](../doc/opendan/protocol/Agent%20DID-Object%20Protocol%20Spec.md)；基础协议是 [buckyos-base DID Object Protocol](../../buckyos-base/doc/did-object-protocol.md)；URL 规范是 [key url.md](../doc/key%20url.md)。这些文档与实现存在差异，本文明确区分当前代码、目标规则和待选方案；本次只更新此 TODO，其他文档与代码的同步属于后续实施。

## 0. GAP review 结论（2026-10-27 核对代码）

以下保留 2026-10-27 的 GAP 快照，不代表所有条目都在 10-29 重跑过验证：两头骨架已有，统一解析和运行时接线缺失；当时核对 Provider 侧为 0/5。本轮补核了路径映射、CYFS/OBJ 语义与 HomeStation 引用链，均为静态核对。

| 层 | 已有 | 缺 |
| --- | --- | --- |
| URL 规范 | [key url.md](../doc/key%20url.md)：`https / file / cyfs / obj / buckyos` 五种 scheme、`/` `//` 缩写、`obj://$service`（config / dfs / taskmgr / kmsg / bucket）、带类型实体 id、RBAC 资源用同一套 url | msg-center / aiworkspace / homestation / opendan 的对象没有 url；`obj://` 到 canonical Object URL 的解析规则没写 |
| 协议 | buckyos-base 有 `DIDObjectCard` / `ObjectProfile` / `DIDObjectHttpServer` / `DIDObjectServer` trait / `DIDObjectClient`，当前框架使用 `did.json / profile.json / props / methods / events` 路由 | buckyos/src 下没有 `impl DIDObjectServer`（10-29 复核，buckyos-base 只有 echo / camera 测试 server）；基础协议的新 Card 发现规则与旧 Agent Spec / client 尚未对齐：[did_obj_client.rs:274](../../buckyos-base/src/name-client/src/did_obj_client.rs) 的 `normalize_object_url` 先去尾斜杠再拼 `/did.json`，client 侧无法区分路径型 / 目录型，见 §2.1 |
| Agent 库 | [agent-did-object-lib](../src/frame/agent-did-object-lib/src/)：`read / x_call / subscribe_event`、路由配置、adapters（filesystem / web / did_object / local_http）、五段式 LLM 输出、session 级 unchanged 去重、EventBridge | `agent_runtime` adapter 是空桩；默认路由只有 `file / http(s) / agent://`，**没有 `obj://` 与 `cyfs://`**；没有 resolve 层（error.rs 只有 `ResolveError` 变体）；`did_object` adapter 把 `object_ref.normalized` 原样交给 `DIDObjectClient::resolve`，给它 `obj://` 等于对 `obj://x/did.json` 发 HTTP，现状只对 https Object URL 可用 |
| 运行时接线 | dev CLI `agent_tool read` / `agent_tool xcall` 走对象库（[agent_tool_cli_dev lib.rs:59-170](../src/frame/agent_tool_cli_dev/src/lib.rs)） | Session 原生工具组 `bash_core` 注册的是 `agent_tool::read_tool::ReadTool`（[xllm.rs:4555](../src/frame/agent_tool/src/xllm.rs)），只认 path / `file://` / `cyfile:` `chunk:` 裸 id，其他 scheme 报 `InvalidArgs`；**两套同名 `read` 行为不一致，LLM 拿到的是窄的那个**。[read_tool.rs:413-416](../src/frame/agent_tool/src/read_tool.rs) 把非空 / 非 localhost 的 file host 当 typo 拒绝，与 `key url.md` 已规定的 `file://$device_id/path` 矛盾。[内置工具手册:195](../doc/opendan/build-in%20agent-tool手册.md) 仍写"仅实现 `file://`"，已落后于代码 |
| Hint → read | Memory `Hint.reference` = `item_x@3` / `<sid>:<seq>`，`sources` 是事件 id；`agent-session recall <tag>`、`agent-memory read`、`agent-session read-session <sid>` | Hint 不携带对象 url；`hint.id → read(object_id)` 在类型层面就不通；Prompt 环境的 known objects 只是 `paths.*` 文件路径 |
| 订阅 | Session 配置 `subscriptions`（source = session / object_event / task / timer / system）+ KEvent / Timer bridge | `<subscribe_event>` 只在 `xml_behavior` 里是合法 action，agent_tool / libopendan 没有实现；对象库的 EventBridge 没接任何服务 |

各组件现状（细节见 §4 各小节）：

| 组件 | 稳定标识 | Agent 可用的读 | LLM 文本 | 事件 | 距离 |
| --- | --- | --- | --- | --- | --- |
| OpenDAN AgentSession | `sid`，无 url | owner 侧 kRPC `session.read / session.worklog`；Agent 自己 `agent-session read-session`；`round_history/` 文件 | 否 | 输入总线 + KEvent / Timer bridge | 中 |
| OpenDAN Workspace | `ws-<uuid>`，`WorkspaceMetadata.url` 可选 | 文件工具；`home.workspaces` | 否 | 无 | 中 |
| msg-center 会话 / 联系人 | `MailboxAddress` = `<owner did>/<sid>`；`obj://msg-center/...` **只是 RBAC key**；群组有 `cyfs://$host/<group_did>/sessions/<sid>` | `msg.list_sessions / list_session / get_message`、`contact.get_contact / list_contacts`、`GET /kapi/msg-center/objects/<id>[/content]` | 否（OpenDAN 自己渲染 XML，只看自己 inbox） | kevent `/msg_center/<owner>/<BOX>/<hex>/changed`、`CONTACTS/changed`、`group/<did>/changed` | 大 |
| AI Workspace | `ws_…` + JSON `Reference{entity_id, selector, mode, version}`（D10 已冻结）；**无 url** | kRPC `doc.read / query / outline / resolve`；`buckyos aiws` 透传 JSON | 仅 wish run 内 `WORKSPACE.md` + `ws_read / ws_query`，要 `proc.claim` | kevent `/aiworkspace/<ws>/head` 仅唤醒；`doc.wait_changes` | 中偏大 |
| HomeStation | 已有 `cyfs://<zone>/home/<user>` 及 feed / inbox / entry 子路径，但不符合本轮确认的文件系统语义，需改为 OBJ；版本是 `cyfeed / cyfhead` ObjId | HTTP `/home/<user>/feed`、`/objects/<id>`；kRPC `item.get / published.list` | 否 | 无 kevent，只有 `?mode=changes&since=` 增量轮询 | 较大：涉及跨 Zone 引用、签名与投递契约 |

安全旁注（10-29 静态复核，未运行时验证）：msg-center 的 owner 绑定检查只对 User token 生效，App / Agent token 落到 `_ => Ok(())`，`caller_identity` 为 `None` 的分支也放行（[owner_session.rs:790-814](../src/frame/msg_center/src/owner_session.rs)）；RBAC 给 `agent_runtime` / `agent` 组的是 `obj://msg-center/*` 的 **`all`**，不是只读（[rbac_config.rs:214-215](../src/kernel/buckyos-api/src/rbac_config.rs)）。Agent 今天只能读自己 mailbox 是 OpenDAN 代码限制，不是服务策略。接通 read 之前要先收口（§5 G1：owner 绑定 + RBAC 收窄 + `None` 分支定性）。

群组一栏里的 `cyfs://$host/<group_did>/sessions/<sid>` 不是 msg-center 自己拼的：[group_sync.rs:37](../src/frame/msg_center/src/group_sync.rs) 拼的是 HTTP path，经 cyfs-ndn 的 `normalize_cyfs_dispatch_target` 才成为 `cyfs://` target。这个 helper 同时被 HomeStation ingress 与 cyfs-gateway 使用，见 §1.5 Q11。

## 1. URL 语义、转换与待决问题

### 1.1 本轮已确认的边界

**按资源语义选择 scheme，本地与远端由 authority 和解析上下文确定。URI 标识资源，不保证当前工具具备访问能力，也不自带授权。**

| 形式 | 语义与约束 |
| --- | --- |
| `https://<host>/<path>` | 通用 Web 引用与访问入口。BuckyOS public 服务通常提供稳定 HTTPS URL；有已知发布映射或可信对象身份绑定时，可以转换为 CYFS / OBJ。不能对任意 URL 只替换 scheme。 |
| 文件 URI：`file://<device_id>/<absolute-path>`（D1–D3） | 标识某设备或执行环境文件系统中的资源，host 段统一是 device id。指定 host 后，工具可以选择 node_executor 等能力访问；scheme 不承诺所有工具均支持远端读取。空 host 的 `file:///path`：有来源 runtime 上下文时属于该 runtime；没有来源上下文（用户输入、持久引用）时按 Zone 的 default device 解释（D2）。 |
| 路径型 `cyfs://<zone>/<path>` | **非 ObjId 路径必须能在 Files / 传统文件系统中定位，并接受传统文件操作直接修改同一资源。** 仅提供只读展示或导出快照、修改仍必须调用业务 API 的对象不符合此定义，应使用 OBJ。 |
| `cyfs://<objid-host>[/<innerpath>]`、`cyfs://<objid-host>.<zone>[/<innerpath>]` | CYFS 内容寻址。ObjId 指向确定内容，编辑产生新 ObjId，不要求对原 ObjId 原地修改。指定 Zone 提供内容来源线索；省略来源时由 resolver 按当前 Zone / 已知内容源定位。 |
| `obj://<service>/<inner-path>` | current-zone 的服务对象引用，host 段直接是服务名（`config / dfs / taskmgr / kmsg / bucket / msg-center / aiworkspace / homestation / opendan`），**不展开成 `obj://config.alice.bns.did/...`**（D4）。authority 是身份，不必对应物理机器；资源能力通过 DID Object 暴露的服务接口访问。DFS 文件也可以有 OBJ 表达，普通业务对象不能因此反向变成文件。 |
| `obj://<entity-id>/<inner-path>` | 实体对象引用，host 段只允许有明确 `did:bns:<id>.<zone>` 的实体 id，如 agent-id、user-id（D5 / D6）；id 自带 Zone，可跨 Zone 解析。不做类型简写，`task_xxx`、`cyfile:...` 之类 id 不进 host 段。 |
| 跨 Zone 的服务对象与投递 target：`https://<zone-host>/<path>` | 跨 Zone 引用服务对象、跨 Zone 投递 NamedObject 时用 https Object URL（D11，基础协议 §5.1 本就要求 Object URL 尽量 HTTPS）。`obj://<service>/` 不带 Zone，只在 current zone 内有意义。 |
| `buckyos://...` | 沿用已有 App / Zone 流程拉起语义，本次不扩展其用途。 |

补充约束：

- **“可以修改”描述资源的文件系统契约，不表示所有调用者都有写权限。** ACL、只读共享或只读挂载不改变文件资源的类别。
- 数据库文件、业务对象、内容快照是不同资源。SQLite 文件可有 CYFS 路径，其中的 Feed / Session / 联系人必须按业务对象判断；“底层落盘”不构成 CYFS 路径成立的理由。
- `cyfs:///p` 表示当前 **Zone** 的路径，不是当前物理节点。`default_device`、当前 Zone、Agent 执行环境是三个独立上下文；default device 是 Zone 级设置，只用于解释没有来源上下文的 `file:///path`（D2）。
- 一个资源可以有多个引用和访问入口。别名、完整身份、传输 endpoint 需分开保存；授权与去重不能随别名拼写改变。对可变路径和其某个 ObjId 快照，只能声明版本关联，不能声明永久等价。
- HomeStation 的 home / feed / inbox / 可变 entry 不满足文件系统契约，**改为 OBJ 已确定，不再以“现有代码已经使用 CYFS”为保留理由**。msg-center 群组、AI Workspace 等其他业务路径也按同一标准审计。
- “Agent 只允许写语义 URI”的限制撤回。Agent 可以使用明确的 HTTPS、文件、CYFS、OBJ 引用，输出时应消除来源上下文歧义。

历史依据：[统一 URL 思考](../doc/opendan/protocol/统一url的思考.md) 已列出服务别名到完整 DID hostname 的展开，以及 OBJ 对 CYFS 的覆盖关系。当前 [HomeStation HomeRef](../src/frame/homestation/src/protocol.rs) 与 [网关 /home 路由](../src/rootfs/etc/boot_gateway.yaml) 将业务 feed 放在 CYFS 同形路径下；修正后，`cyfs:///home/alice/feed` 若存在，只能表示对应文件系统资源。

### 1.2 已知转换与成立条件

下表的“已有”区分规范、局部实现和通用 resolver：**局部接口支持某个写法，不代表统一 read 已支持它。** `Z` 表示确定的 Zone hostname，`Hdfs` 表示该 Zone 已注册的 DFS 服务 DID hostname，`O` 表示 hostname 安全编码的 ObjId；`P/I` 分别表示逻辑路径和对象内路径。

| 输入 | 转换 / 关联结果 | 条件与当前状态 |
| --- | --- | --- |
| `obj://config/P` | 不展开；就是 current zone 的 config 服务对象 `P`（D4） | 对外 / 跨 Zone 引用时改用该服务的 https Object URL（D11）。`obj://config.alice.bns.did/P` 这种展开形式废弃，统一url的思考 / key url.md 中对应段落删除。 |
| `//config/P` | `obj://config/P`，再补完整 authority | `key url.md` 已有 BuckyOS 输入缩写。只在声明采用此语法的输入入口展开，不能改变 shell / UNC 原有解释。 |
| `obj://task_xxxx/` | **不支持**；只接受 `obj://taskmgr/task_xxxx`（D6） | key url.md"带类型的实体 id"一节删除。host 段只允许服务名或带 Zone 的 did:bns 实体 id；底层 ID 不改名。 |
| `cyfs:///P`、`cyfs://_/P` | `cyfs://Z/P` | 当前 Zone 展开已有规范；NFS 目前只接收前两种形式，完整 Zone 形式需外层 resolver 路由，不能直接当作现有 NFS 入参。 |
| `$BUCKYOS_ROOT/data/P` | `cyfs:///P`，对外补为 `cyfs://Z/P` | 本轮采用的目标映射，须确认来源机器和真实挂载关系。当前 NFS 已实现其中已导出目录的访问；未导出表示当前访问能力缺失，不自动否定资源引用。不是对任意容器路径做字符串删前缀。 |
| `cyfs:///P` / `cyfs://Z/P` | `obj://dfs/P` / `obj://Hdfs/P` | 本轮确定的 DFS 文件引用关系；DFS 的 DID Object Provider 与通用转换尚待实现。`obj://dfs/` 是服务根对象，其下文件对象与文件本体需声明关联。 |
| `cyfs://O/I` | `cyfs://O.Z/I` | 已确定来源后可补全；同一 ObjId 的身份不依赖来源 Zone。hostname 模式及 innerpath 路由须由访问端支持，不是给裸 ObjId 直接做 DNS 查询。 |
| `cyfile:<hash>` 等合法 ObjId | `O`，进而构成 `cyfs://O/` | 复用 ObjId 编解码。host 使用小写、无 padding base32；不要构造 `obj://cyfile:<hash>`，其中冒号会与 URL 端口语法冲突。NamedObject 不进 `obj://` host（D6）。 |
| `cyfs:///srv/publish/P` | `https://public.Z/P` | 已有 public 发布映射；保留路径编码，实际可访问性仍受发布配置和权限约束。反向只针对已知发布路由。 |
| `obj://bucket/<bucket_id>/<key>` | `https://Z/bucket/v1/<bucket_id>/<key>` | 已有 Bucket 数据面映射，读当前绑定或指定版本；这不自动证明该 key 是可直接修改的 DFS 文件。token、版本和读权限仍按 Bucket 契约处理。 |
| 已验证的 Object DID / Object URL | 对应完整 OBJ 引用、Card 和已声明 endpoint | 需要可信身份绑定与明确路径编码；DID 的对象 path 不能在转 hostname 时丢掉。通用解析待接线。 |
| 已知 public `https://...` | 对应 CYFS 或 OBJ 引用 | 发布映射或 Card / 注册表确认是同一资源后才能转换。普通网页、业务服务首页、附件正文与业务对象不能互相冒充。 |
| HomeStation `https://Z/home/<user>/...` | 本 Zone 内等价于 `obj://homestation/home/<user>/...`（D10）；跨 Zone 持久引用（entry、Follow、投递 target）就用 https 形式（D11） | 现有 HTTPS 入口保留并升格为跨 Zone 规范引用；业务引用不再反推成 DFS 路径。 |
| 跨 Zone 投递 target（现 `cyfs://<host>/<path>`） | `https://<host>/<path>`（D11） | cyfs-ndn 的 `normalize_cyfs_dispatch_target` 改为产出 https，msg-center 群组、HomeStation ingress、cyfs-gateway 通过同一 helper 跟随；须核实 target 是否进入签名 / 去重键。 |

实现依据：[NFS 导出](../src/frame/nfs_server/src/config.rs)、[当前 Zone URI 解析](../src/frame/nfs_server/src/namespace.rs)、[public 映射](../src/frame/nfs_server/src/share/rpc.rs)、[Bucket URL](../src/kernel/buckyos-api/src/bucket_client.rs)、[ObjId 编码](../../cyfs-ndn/src/ndn-lib/src/object.rs)。当前 NFS 导出的是启动时非点号的一级目录，根层普通文件及 `.fsdb` 不在导出中，新一级目录需重新装载导出配置。这是现有能力与 data→CYFS 目标映射的差距，不能升级成 URI 命名限制，也不能宣称所有 data 路径已经可访问；Files 可见与直接修改的目标契约仍须在实施中满足。

以下不是无条件等价转换：任意 HTTPS 换 scheme；业务 Feed / Session 转 DFS 路径；可变路径转固定 ObjId 后丢失版本语义；原始 `did:...` 或 `cyfile:...` 直接塞进 host；转换时丢弃 selector、version 或改变授权范围。

### 1.3 实践中总结的使用原则

1. **先问资源能否通过传统文件系统直接修改，再选路径型 CYFS 或 OBJ。** 文件操作会修改同一资源的是 CYFS 候选；只能通过业务 action 修改的是 OBJ。不要为了“统一 URL”给数据库对象伪造文件路径。
2. **输入可以推断，输出要补全。** 用户无 scheme 的路径可结合会话设备和工作区推断；只有已经写成 `file:///path` 且没有来源 runtime 的输入才落到 Zone default device（D2）。上下文足够时确定性展开，多个候选会落到不同资源时说明假设或询问。对已经显式指定的 scheme / host 不重新猜测。
3. **裸路径属于产生它的执行环境。** Agent 自身工具产生的路径通常属于 Agent 容器；SSH / node_executor 工具产生的路径属于相应远端环境。不能把它交给接收方按自己的 cwd / default device 重新解释。
4. **相对路径先解析 cwd，绝对路径仍需知道 runtime。** `/tmp/report.md` 在两个容器中是两个资源。发送消息、写 Hint / Memory / Notebook 或跨 Agent 传递前，补齐来源身份。
5. **持久产物优先使用稳定的 DFS / 对象引用。** 经真实挂载确认位于 data 映射目录的 AgentRoot 文件可以转 CYFS；现有接口尚未导出时明确报告访问能力缺失。容器临时文件或镜像层文件不能自动转 CYFS。临时文件 URI 也不能因为加了 scheme 就变成持久引用。
6. **路径映射使用真实挂载与规范路径。** 检查路径段边界、`..`、符号链接和容器→宿主映射，不能只做字符串 starts-with。进入 DFS 命名空间的身份等价不代表绕过原有文件或服务权限。
7. **`obj://<service>/` 与 `cyfs:///P` 只在 current zone 内有意义。** 它们不是简写、没有"完整 authority"展开形式（D4）；持久引用或跨 Zone 输出时改用 https Object URL / `cyfs://<zone>/P`（D11），或显式携带 Zone 上下文。带 Zone 的实体 id（agent-id、user-id）可以直接进 `obj://` host 段跨 Zone 引用（D6）。不要把 bridge 的传输域名当成新的对象身份。
8. **别名、视图和版本要分开。** `obj://dfs/P` 可以描述文件的对象能力，`cyfs:///P` 访问文件数据，read 的输出形式可以不同。业务条目的附件、导出文件、某个版本快照不是业务条目本身，不能随意放进 `alsoKnownAs`。
9. **引用不等于能力。** “URI 可解析”“工具实现了该协议”“调用者获准访问”分别检查；不支持或无权限时返回明确错误，不换一个同形本地路径碰碰运气。
10. **路径型与 ObjId 型写法要可机械识别。** `cyfs:///xxxx` 的 xxxx 是路径；`cyfs://xxxx` 的 xxxx 是 authority。ObjId 使用严格编解码判定，不能靠“看起来像 hash”或 LLM 猜测；文件名的大小写、空格、`#`、`?` 等需正确编码。
11. **默认设备不是 ood1 常量。** default device 是 Zone 级设置、指向 cluster 中某一设备（D2），Desktop 常见值为 ood1 只是现状。多设备、远端会话和 Agent runtime 不能共享一个无来源的"本机"：有来源 runtime 的路径按来源解释，不走 default device。
12. **幂等不等于只读。** OBJ action 即使幂等，也可能修改资源；Profile 和 Governance 要按实际副作用授权，不能因为重试安全就自动放行写操作。

典型 Agent 产物展开（目标规则，不代表 resolver 已实现）：

```text
来源：Agent runtime + cwd；输入：reports/a.md
  → 来源 runtime 内的绝对路径
  → 经挂载表确认：$BUCKYOS_ROOT/data/home/alice/agents/<agent-id>/reports/a.md
  → cyfs://<zone>/home/alice/agents/<agent-id>/reports/a.md

来源：远端 executor；输入：/tmp/a.md
  → file://<executor 的 device_id>/tmp/a.md（D3）
  → 无 DFS 映射，不自动生成 cyfs URI
```

AgentRoot 的正式路径见 [opendan main.rs](../src/frame/opendan/src/main.rs)，容器挂载见 [app_loader.rs](../src/kernel/node_daemon/src/app_loader.rs)。同一规则也适用于工具返回值；不能要求 LLM 仅凭路径字符串猜出挂载关系。

### 1.4 Resolve 层与访问分工（实施建议）

```text
原始输入 + 来源上下文
  → normalize：scheme、别名、cwd、runtime、current-zone、路径编码
  → 明确资源引用（保留原始输入、完整身份、版本 / selector）
  → resolve：身份 / 服务发现、挂载映射、协议能力、访问路由
  → 对应 adapter
      file：filesystem / 已配置的 node_executor
      CYFS 路径：DFS；CYFS ObjId：NDN
      OBJ：DID Object Card → Profile → 声明的属性 / 动作 / 事件
      HTTPS：已知对象 / 发布路由，或普通 web adapter
```

- 入口解析放在对象库之前，Rust 侧落在 buckyos-api、TS 侧落在 websdk；共享规则、注册数据及测试向量。纯语法归一化可以是纯函数，DID 发现、服务查询和能力探测可能需要 I/O，不能把完整 resolver 都定义为纯函数。
- OBJ 是 resolver input，不是 wire endpoint。DID hostname 保留对象身份；真正 HTTP 地址由已知 Provider route 或 Card / Profile 决定，`/kapi/<service>/obj/` 只是可选发布布局。
- 不再把所有 CYFS / OBJ 都先替换成 HTTPS 再交给 `did_object`。路径型 CYFS、ObjId、文件和普通网页有各自访问语义；OBJ 的 DFS / 内容投影由 Provider 声明，尚未具备时返回不支持。
- `ObjectRouteConfig` 增加适当的 obj / cyfs 入口路由。别名归一化先于对象级 RBAC、缓存和事件去重，同时保留目标 Zone / tenant；不同视图、版本、授权上下文不能仅因身份相同就共用内容缓存。
- 当前 Agent `read` 只接收空 host / localhost 的文件 URI；node_executor 已有文件能力但目标通过 `node_did / executor_id / profile_id / workdir` 指定，hostname→executor 仍需接线。见 [read_tool.rs](../src/frame/agent_tool/src/read_tool.rs)、[node_executor_proto.rs](../src/kernel/buckyos-api/src/node_executor_proto.rs)。
- 规范定稿后，在 `key url.md` 维护服务别名、类型简写、转换边界；实现与手册引用同一份规则，不能在 Prompt、SDK、网关和每个工具里各猜一次。

### 1.5 统一裁决清单（Q1–Q12）

**2026-10-29 全部裁决完毕。** "裁决"列是用户原话；表后的 D1–D12 是据此推出的定稿规则，全文其他段落按 D1–D12 改写。"既有规范 / 先例"列保留为改动依据：裁决与之相反的，第 1 段要同步改对应文档。

| # | 要决定的问题 | 建议方案 | 备选及影响 | 既有规范 / 先例 | 裁决 |
| --- | --- | --- | --- | --- | --- |
| Q1 | 文件 URI 省略 host 时的 default-device 规则放在哪里？ | 使用标准 `file://`；`file://<device_id>/path` 按规范表示指定设备；空 host / `localhost` 保留标准本地含义，即产生该路径的 runtime；BuckyOS 用户裸路径入口按 default device 展开为显式 host 后再持久化。 | 让空 host 直接表示集群默认设备（非标准解释），或新增 `files://`；两者都与已有规范冲突，且需所有生产 / 消费工具一致支持。标准依据：[RFC 8089](https://www.rfc-editor.org/rfc/rfc8089.html)。 | [key url.md:18](../doc/key%20url.md) 与 [统一url的思考](../doc/opendan/protocol/统一url的思考.md) 均已规定 `file://$device_id/path`，`/local/path → file:///local/path`。`files://` 备选已被规范排除，本问题收窄为”空 host 的含义与默认值来源”。[read_tool.rs:413](../src/frame/agent_tool/src/read_tool.rs) 当前拒绝非 localhost host，需按规范改。 | 按建议 |
| Q2 | default device 是 Zone、用户还是会话级？ | Zone 提供基础默认值，用户 / 会话显式选择可覆盖；已知来源 runtime 的工具路径不再套 default device。 | 全 Zone 唯一默认值更简单，但不能自然表达多设备用户；必须明确默认值变更不重解释已经持久化的引用。 | 无既有定义；Desktop 常见 default device 为 ood1，只是现状不是规则。 | default device 是已经构造了 `file:///xxxx` 的场景才会用，按规则就是 cluster 里的某个设备为 default device。相对路径推导，或者 `/tmp/xxxxx` 的传统路径推导不在这个分支。 |
| Q3 | 无 DFS 映射的文件，authority 如何标识 Agent / executor 文件环境？ | 注册可解析的执行环境身份，并绑定 node / executor；持久 AgentRoot 仍优先用 CYFS。临时环境引用明确生命周期。 | 用 Agent DID 间接定位”当前运行环境”可跟随调度，但重建后临时文件未必相同；直接用 Docker 容器名不能保证跨节点唯一或长期可解析。 | node_executor 已有 `node_did / executor_id / profile_id / workdir` 定位（[node_executor_proto.rs](../src/kernel/buckyos-api/src/node_executor_proto.rs)），hostname→executor 没有映射。 | 通过 node_daemon 实现的是有稳定的 device id 的，临时通过 buckyos-agent-daemon 添加的也应该鼓励构造稳定的、用户可读的 device id。 |
| Q4 | 服务别名怎样绑定完整 DID hostname？ | 复用服务身份注册数据，固定允许的别名；例如 `config → config.alice.bns.did`。未知短名报错，完整远端身份走 DID resolver。resolver 的别名规则迁移 / 复用 sys_config_service 现有实现，不再造一套。 | 统一约定 `<service>.<zone>` 并强制注册，或者全部由注册表给出；需决定自定义 App 服务、多实例和 owner 作用域的别名范围。 | [sys_config_service main.rs:108-120](../src/kernel/sys_config_service/src/main.rs) 的 `strip_config_key_prefix` / `get_full_res_path` 已实现 `obj://config/` ↔ key 的剥离与补全（key url.md 第 42 行注明）；[bucket_client.rs:309](../src/kernel/buckyos-api/src/bucket_client.rs) 已有 `obj://bucket/` 编解码。两者都是单服务内的局部实现。 | 没必要展开，`config` 就是 current zone 的 config 服务就好了。 |
| Q5 | Agent 自身对象以谁为 authority？ | 以 Agent 自己的 DID hostname 为规范根：`obj://<agent-host>/sessions/<sid>`；OpenDAN 服务路径作为发现入口或别名。 | 以 OpenDAN 服务为根：`obj://opendan/<agent-id>/sessions/<sid>`。两种方案影响对象归属和跨 runtime 稳定性；`agent://self` 是否保留也在此确认。§4.1 暂用服务路径展示对象集合。 | [Agent Session SDK 实现计划.md:2021](../doc/opendan/Agent%20Session%20SDK%20实现计划.md) 已规划 `agent://self/... → {base}/opendan/agents/{aid}/...`，即备选方案（服务为根）。采用建议方案需同步改该计划；`agent://self` 目前除此文档外没有任何实现。 | `obj://<agent-id>/sessions/<sid>` 是可以的。 |
| Q6 | v1 支持哪些 typed-id 简写，内容对象怎样进入 OBJ？ | 保留 `cyfile:...` 等现有工具输入，归一化为 ObjId 型 CYFS；服务对象用明确服务路径。已定义的 `task_...` 可保留简写，不改 `ws-` / `ws_` 底层 ID。 | 若要 `obj://<encoded-objid>/...`，需定义其展开后的完整 DID hostname 与 Provider，不能仅凭类型前缀认为它已有 DID Object Card。跨 Zone 缺少来源的服务 ID 不允许猜归属。 | key url.md 已有 `obj://task_xxxx/ ≡ obj://taskmgr/$taskid` 的类型简写设计；没有任何实现。 | 不要简写，保持 `obj://taskmgr/$taskid` 这种。原有的简写规则应该删除。基本原则：agent-id、user-id 这种有明确的 `did:bns:<xxx-id>.<zoneid>` 的才可以放到 host 段。 |
| Q7 | 对象的版本、selector、视图是否有统一 URI 契约？ | path 定位对象 / 子对象；query 表达 `version / selector / mode` 等已声明选择器，保留 AI Workspace Reference 的全部语义；快照引用和 live 引用分开。`selector` 等 JSON 值进入 query 前做规范化序列化（键排序、无空白），缓存键与签名用规范化后的串。 | 各服务独立定义 query 更灵活，但共享 resolver 只能保留，不能统一解释；需明确哪些 query 参与资源版本、缓存键及签名。 | AI Workspace `Reference.selector` 是 JSON 对象而非字符串，`mode` 为 `live` / `fixed`，`version` 为对象（[aiworkspace_client.d.ts:27-30](../src/rootfs/libexec/buckyos-tool/dist/aiworkspace_client.d.ts)）。没有规范化序列化，同一 Reference 会产生多个 URL。 | 原则上允许多个 URL 指向同一对象，但一个对象最好有一个明确的默认 URL。 |
| Q8 | Object DID 命名采用什么统一约定？ | 对已有 DID 的对象复用身份；新服务子对象从稳定服务身份派生 path DID，不因 HTTP endpoint 调整而改身份。 | 统一从 HTTPS 路由派生 `did:web` 实现简单但耦合发布布局；使用服务 `did:bns` 路径需要相应 resolver 支持。不能无条件把 NamedObject ID 拼成一个尚未注册的 DID。 | 基础协议 §5.2 已给 path DID 示例（`did:web:host:devices:cam01`、`did:bns:zone:...`）；msg-center 群组已有自己的 DID 与 `did.json`（[group_http.rs:204](../src/frame/msg_center/src/group_http.rs)）。 | 该用 PATH 用 PATH。 |
| Q9 | 对象事件是否统一改造底层 kevent 名称？ | Profile 对外声明稳定对象事件，内部 bridge 映射现有事件；key 带目标身份作用域并做合法编码。先打通本 Zone 唤醒，远端事件能力明确报告。 | 全部改成统一 kevent 路径会扩大现有发布者/订阅者改动；若首期要求跨 Zone 订阅，需同时实现相应远端订阅生命周期。 | [bridge/kevent.rs](../src/frame/lib_opendan/src/bridge/kevent.rs) 的 `object_event.object` 直接就是 kevent id pattern，没有对象层。 | 可以统一。 |
| Q10 | HomeStation 的 OBJ 路径和首期动作范围采用什么布局？ | 采用服务 DID 根下 `/home/<user>/feed`、`/inbox`、`/feed/@/<key>`，保留 HTTPS 展示入口；inbox 投递作为 Profile 声明的 action，复用现有内容校验及投递结果。 | 可改为 `/users/<user>/...` 等布局；需一次定稿，因 entry / target 会进入签名和持久数据。若首期只做只读 Provider，跨 Zone 发布/投递改造必须单列后续阶段，不能宣称 HomeStation 已完整接通。 | entry 串已落库（[publish.rs:74](../src/frame/homestation/src/publish.rs) `published.entry`），reader proof `aud` 固定为 `cyfs://<zone>/home`（[auth.rs:187,199](../src/frame/homestation/src/auth.rs)）；[evaluation.rs:1027](../src/frame/homestation/src/evaluation.rs) 接受 cyfs / https / 门户三种入口；Desktop [links.ts](../src/frame/desktop/src/app/homestation/links.ts) 已只产出 https 门户链接。与 Q11 绑定裁决。 | 按建议 |
| Q11 | **跨 Zone NamedObject 投递的 `cyfs://` target 是保留为传输层，还是随 HomeStation 一起改为 OBJ？** | 保留 cyfs-ndn 的 dispatch target 作为传输层契约（网关、群组、HomeStation 共用），HomeStation 的 OBJ 引用只做语义层：Profile 的 inbox action 内部仍调用现有 dispatch，Card / 投递结果里返回 OBJ 引用。本轮不改 cyfs-ndn / cyfs-gateway。 | 把 dispatch target 改成 OBJ：需同步改 cyfs-ndn `normalize_cyfs_dispatch_target`、cyfs-gateway `dispatch.rs` 与 `named_inbox_cache_server.rs`、msg-center 群组五处消费者；跨仓发布顺序与两种 target 并存期都要定义。 | `normalize_cyfs_dispatch_target` 在 [cyfs-ndn cyfs_dispatch.rs:16](../../cyfs-ndn/src/ndn-lib/src/cyfs_dispatch.rs)，固定产出 `cyfs://<host><path>`；消费者：msg-center `group_http / group_auth / group_sync / message_hub / cyfs_dispatch`、HomeStation [ingress.rs:197](../src/frame/homestation/src/ingress.rs)、cyfs-gateway `server/dispatch.rs` 与 `server/named_inbox_cache_server.rs:125`（离线 inbox 缓存按 `target_zone + path` 归一化）。这是 Zone 间投递的通用协议，不是 HomeStation 私有写法。 | 跨 Zone 其实 `https://` 的定义可能更明确。 |
| Q12 | **msg-center 对象路径是否直接对齐现有 RBAC key 布局？** | 对象路径采用 `obj://msg-center/owners/<owner_did>/sessions/...`、`obj://msg-center/owners/<owner_did>/contacts/...`，与现有 `owners/*` RBAC 前缀同形；`inbox / sent / group_inbox / request_box / <addr>` 保持为 RBAC 资源 key，不另作对象路径。§4.2 / §4.3 的路径表按此改。 | 保留 §4.2 / §4.3 原建议 `obj://msg-center/<owner_did>/...`：需在 resolver 与 RBAC 之间加一层对象路径 → 资源 key 的映射，并保证 G4 的”权限不随别名变”。 | 现有 RBAC key：`obj://msg-center/owners/*/contacts`、`inbox/*`、`sent/*`、`group_inbox/*`、`request_box/*`、`objects`、`notify`、`contact_tags/<app>`、`contact_membership`（[rbac_config.rs:170-215](../src/kernel/buckyos-api/src/rbac_config.rs)、[msg_contact.rs:37,53](../src/kernel/buckyos-api/src/msg_contact.rs)）。 | 按建议 |

#### 定稿规则 D1–D12（由裁决推出，全文以此为准）

| # | 规则 | 要改的文档 / 代码 |
| --- | --- | --- |
| D1（Q1） | 文件 URI 用标准 `file://`，host 段是 device id：`file://<device_id>/abs/path`。Agent 工具产出的空 host 路径属于来源 runtime；跨上下文输出、写 Hint / Memory、发消息前补成显式 device id（A9）。 | read_tool 接受 device id host；key url.md 保持 `file://$device_id/path` 并写明空 host 规则 |
| D2（Q2） | default device 是 **Zone 级**设置，指向 cluster 中某一设备。只用于解释没有来源 runtime 上下文、已经写成 `file:///path` 的输入（用户输入、持久引用）。相对路径、`/tmp/x` 等裸路径按 §1.3 第 3 / 4 条由来源 runtime + cwd 推导，不进这个分支。 | key url.md 新增 default device 段；system-config 增加 Zone 级 default device 键（第 1 段 RBAC / 配置设计一并定） |
| D3（Q3） | 执行环境 authority 统一为 device id。node_daemon 管理的设备已有稳定 device id；buckyos-agent-daemon 临时加入的环境也构造稳定、用户可读的 device id 并注册，使 `file://<device_id>/...` 可解析到 node / executor。不用容器名、不用 Agent DID。 | agent-daemon 注册流程；node_executor 的 device id → executor 映射（第 2 段 §4.6） |
| D4（Q4） | 服务别名**不展开**。`obj://config/P` 就是 current zone 的 config 服务对象，host 段直接是服务名；`obj://config.alice.bns.did/P` 形式废弃。跨 Zone 引用服务对象用 https（D11）。sys_config_service 现有 `obj://config/` 剥离逻辑保留；共享模块只做服务名合法性与路径归一化。 | key url.md / 统一url的思考 删除展开段落；§1.1 / §1.2 已改 |
| D5（Q5） | Agent 自身对象 authority 是 agent-id：`obj://<agent-id>/sessions/<sid>`、`/workspaces/<ws-id>`、`/goals/<gid>`、`/artifacts/<aid>`（agent-id 形如 `xiaobai.test.buckyos.io`，有明确 did:bns）。`obj://opendan/` 根对象只做发现（列托管 Agent），不是 Agent 对象的规范路径。`agent://self` 废弃。 | Agent Session SDK 实现计划的 `agent://self` 段改写；§4.1 已改 |
| D6（Q6） | 不做类型简写：`obj://taskmgr/<taskid>` 保持完整服务路径，key url.md"带类型的实体 id"一节删除。`obj://` host 段只允许两类：current-zone 服务名；带 Zone 的 `did:bns:<id>.<zone>` 实体 id（agent-id、user-id）。NamedObject id（`cyfile:` 等）不进 `obj://` host，归一化为 `cyfs://<O>/`。 | key url.md；resolver 校验规则 |
| D7（Q7） | 一个对象可以有多个 URL，但必须有一个明确的默认 URL：Card 的 `id` 对应的 Object URL 为默认，其余在 `alsoKnownAs` 声明。**解读：** 版本 / selector / mode 走 query，`selector` 等 JSON 值规范化序列化后进 query，缓存键与签名用规范化串；快照与 live 引用分开。需确认。 | key url.md query 契约段；AI Workspace 共享编解码（§4.4） |
| D8（Q8） | 子对象用 path DID：从服务 / 实体身份派生（`did:bns:<host>:<path segments>`），已有 DID 的对象复用身份，不因 HTTP endpoint 调整而改身份。 | 基础协议 §5.2 补 BuckyOS 约定；各 Provider Card 的 `id` |
| D9（Q9） | **统一改造底层 kevent 名称**：kevent key 统一为对象事件形式（建议 `/obj/<host>/<path>/<event>`，具体在第 1 段定），现有发布者 / 订阅者一起改；Profile 声明的事件名与 kevent key 一一对应，不再需要 bridge 映射表。首期本 Zone；远端订阅能力明确报告。 | msg-center / aiworkspace / opendan 的 kevent 发布与订阅；[bridge/kevent.rs](../src/frame/lib_opendan/src/bridge/kevent.rs) |
| D10（Q10） | HomeStation 本 Zone 对象路径：`obj://homestation/home/<user>`、`/feed`、`/feed/@/<key>`、`/inbox`、`/home/~zone`；inbox 投递是 Profile 声明的 action；保留 HTTPS 展示入口。 | §4.5 已改 |
| D11（Q11） | **跨 Zone 用 https**。解读：① 跨 Zone 的 Object URL 是 `https://<zone-host>/<path>`（基础协议 §5.1 本就如此）；② cyfs-ndn 的 dispatch target 由 `cyfs://<host><path>` 改为 `https://<host><path>`，msg-center 群组、HomeStation ingress、cyfs-gateway 通过同一 helper 跟随；③ HomeStation 的 entry / Follow / 投递 target 等跨 Zone 持久引用用 https 形式，本 Zone 内 `obj://homestation/...` 是其别名；④ `obj://<service>/` 不带 Zone，只在 current zone 内使用。需确认，尤其 ② 影响 cyfs-ndn / cyfs-gateway 发布顺序。 | cyfs-ndn `cyfs_dispatch.rs`、cyfs-gateway dispatch / named_inbox_cache、msg-center 群组、HomeStation protocol / auth / publish |
| D12（Q12） | msg-center 对象路径直接对齐现有 RBAC key：`obj://msg-center/owners/<owner_did>/sessions/...`、`.../owners/<owner_did>/contacts/...`；`inbox / sent / group_inbox / request_box / <addr>` 仍是 RBAC 资源 key，不另作对象路径。 | §4.2 / §4.3 已改；第 1 段 RBAC 设计 |

不需要重新发明的实施约束：Card 发现按基础协议对齐（§2.1）；权限不能随 URI 别名变化；幂等写 action 仍按写操作治理；默认不增加旧业务 CYFS URI 的兼容层。正式实现前依据定稿结果更新引用和数据生成逻辑，本 TODO 不执行迁移。基础协议 [§5.4](../../buckyos-base/doc/did-object-protocol.md) 与 Agent Spec §4.1 已规定 `obj://` 是 resolver input、不是 wire endpoint，§1.4 与之一致。

## 2. DID Object Provider 的最小契约

### 2.1 必须实现

业务对象实现 `DIDObjectServer`，复用 buckyos-base 的 `DIDObjectHttpServer`，并注册语义引用到对象身份 / Provider 的映射。`/kapi/<service>/obj/<path>` 可以作为新 Provider 的 HTTP 布局，但不是全局转换公式。普通文件、CYFS 内容或 Web 页面不因进入统一 read 就被要求实现这套契约。

**先解决 Card 发现的协议 / 实现漂移：** 基础协议 §5.3 / §6.1 已规定路径型 Object URL 直接取 Card（body 或 `BuckyOS-DID-Document` 头），目录型才追加 `did.json`；旧 Agent Spec 与当前 client / server 仍以追加后缀为主。具体漂移点（10-29 核）：[did_obj_client.rs:274](../../buckyos-base/src/name-client/src/did_obj_client.rs) 的 `normalize_object_url` 先 `trim_end_matches('/')` 再调 [`DIDObjectCard::default_card_url`](../../buckyos-base/src/name-lib/src/did_object_card.rs)（固定拼 `/did.json`），所以 client 在拼 URL 前就丢掉了路径型 / 目录型的区分，与协议 §6.1 "分类在拼 URL 之前完成，不能先去掉尾斜杠"直接冲突；server 侧 `DID_OBJECT_CARD_PATH` / `DID_OBJECT_PROFILE_PATH` 常量同样只认追加后缀。实施时对齐基础协议、Card URL 构造、client、server、SDK 与 Agent Spec，并覆盖 query / 尾斜杠行为。不能把旧 `{url}/did.json` 约定当成新对象的统一硬规则。

| 能力 / 路由 | 内容 | 最低要求 |
| --- | --- | --- |
| 按基础协议发现 Card | DID Object Card（DID Document） | `id` 按 D8（已有 DID 复用，子对象 path DID）；`controller` 指向合法控制者；`alsoKnownAs` 只列同一对象的引用；`service` 声明访问入口与 Profile |
| Card 声明的 Profile URL（当前框架常用 `/profile.json`） | Object Profile（constrained WoT TD） | `properties / actions / events`、`x-buckyos:traits`；集合类对象实现 `IndexTrait` 的 `query / page`；声明实际支持的能力 |
| Profile 声明的 property endpoint（当前常用 `/props/<name>`） | 属性值 JSON | 带 `ETag` / 版本；selector 与版本按 Q7，不由 Agent 猜 endpoint |
| Profile 声明的 action endpoint（当前常用 `/methods/<name>`） | kRPC 风格 action | v1 可先接只读 action；写 action 包括幂等写都必须声明风险与授权资源，走 Governance |
| Profile 声明的 events / subscription binding | 事件与订阅能力 | 事件名与统一后的 kevent key 一一对应（D9）；本 Zone kevent bridge 和远端协议能力分别声明，不把已有 kevent 唤醒等同于远端订阅已实现 |

`read(url)` 的 object document 由对象库的 `did_object` adapter 从 Card + Profile + 关键 props 渲染，组件不需要自己生成 LLM 文本。组件负责清晰的 title / description、每个属性和动作的说明，并声明 `x-buckyos:summary_props`。引用文件正文、附件或其他对象时返回相应资源引用，不能用 `alsoKnownAs` 混淆关联资源。

### 2.2 集合与发现

- 每个服务的根对象实现 `IndexTrait`：`query` 列出调用者可见的顶层对象。`obj://<service>/` 只在 current zone 内有意义（D4）；对外 / 跨 Zone 返回 https Object URL（D11），实体对象返回 `obj://<entity-id>/...`（D6）。
- Agent 的 known objects 从当前 Session、Workspace、Agent 根对象、msg-center / aiworkspace / homestation 服务根出发。`cyfs:///home/<owner>` 是用户文件目录；HomeStation 主页使用 OBJ，两者分开注入。

## 3. Agent 侧接线（§6 第 3 段：在 §4 服务改造之后）

| # | 事项 | 落点 | 说明 |
| --- | --- | --- | --- |
| A1 | **统一 read**：`bash_core` 接入 `AgentDIDObjectReadTool`；filesystem adapter 复用现有目录策略、`range / first_chunk / offset / limit` 和 ObjId 读取 | [xllm.rs](../src/frame/agent_tool/src/xllm.rs)、[tool.rs](../src/frame/agent-did-object-lib/src/tool.rs)、[filesystem.rs](../src/frame/agent-did-object-lib/src/adapters/filesystem.rs) | 保持 `AgentToolResult` 和 token budget，避免另造文件授权模型；同步 [内置工具手册](../doc/opendan/build-in%20agent-tool手册.md) |
| A2 | **Normalize / resolve**（§1.4）及 obj / cyfs / `file://<device_id>` 路由 | `buckyos-api::object_url`（Rust）、websdk `object_url.ts`；`ObjectRouteConfig` | 覆盖 §1.2 转换及失败条件；host 段按 D4 / D6 校验（服务名或带 Zone 的实体 id，其余报错）；两端共用测试向量；区分纯解析和需 I/O 的发现 |
| A3 | **Agent 自身对象接线**：`obj://<agent-id>/...` 走通用 did_object 路径（D5），`agent://` scheme 与 `agent_runtime` 空桩 adapter 删除 | [agent_runtime.rs](../src/frame/agent-did-object-lib/src/adapters/agent_runtime.rs)、dev CLI 默认路由 | §4.1 Provider 接通后退役空桩；`agent://self` 不保留 |
| A4 | **Hint 带 url**：`Hint` 加 `object_url: Option<String>`；感知记录保存可复现的完整引用；recall 和 hints 输出可 read 的引用 | [hint.rs](../src/frame/lib_opendan/src/memory/hint.rs)、state/perception、runner/observe、perception schema | 保留版本 / selector，不保存无来源的裸路径；hint 不是事实，read 才获取当前或指定版本内容 |
| A5 | **known objects 与路径上下文**：新增 `objects.*`（Agent 自己 `obj://<agent-id>`、当前 Session / Workspace、服务根）；保留 shell 所需 `paths.*`；提供 current-zone、来源 runtime 的 device id / cwd 与挂载解析入口 | [Agent Enviroment.md](../doc/opendan/Agent%20Enviroment.md)、runner/assembler | 引用附一句话，不注入对象内容或完整挂载表；由 runtime 确定性展开路径；default device 不注入给 Agent（D2：Agent 路径有来源 runtime） |
| A6 | **subscribe_event**：Session 的 `object_event` 来源接对象库 EventBridgeManager 和 KEvent bridge | agent_tool 新工具、[bridge/kevent.rs](../src/frame/lib_opendan/src/bridge/kevent.rs)、runner/inputs | D9 统一 kevent 命名后，`object_event.object` 直接是对象 URL，bridge 只做 URL → kevent key 的确定性转换；跨 Zone 能力按实际实现报告；半订阅按 Session 架构差距另行推进 |
| A7 | **xcall**：Governance 按 Profile 的 `x-buckyos:risk / rbac` 做 allow / require_approval / deny | agent_tool 新工具、libopendan `classify_effect` | 只读能力先接通；幂等写操作仍做写权限和副作用治理，HomeStation 投递属于写动作 |
| A8 | websdk / CLI 增加 `buckyos obj read <url>`、`buckyos obj resolve <url>` | websdk `cli/modules/obj.ts` | 复用 A2；resolve 输出明确身份、来源上下文和访问路由，不能把“解析成功”写成“有权访问” |
| A9 | **工具输出归一化**：文件结果、产物、消息和持久记忆在跨上下文输出前绑定来源并补完整引用 | 文件工具 / executor 结果、artifact、消息发送边界 | 按 D1–D3：空 host 路径补成 `file://<device_id>/...`，有 DFS 映射的补成 `cyfs://<zone>/...`；验证容器、宿主和远端 executor 的同形路径不串资源 |

llm_context / agent_tool 层的工具改动仍单列到 `llm-context-agent-runtime-todo.md` 跟踪，引用本文件定稿规则，避免复制另一套 URI 语义。

## 4. 各组件实施（§6 第 2 段：第 1 段定义定稿后开工）

每个业务组件的任务：① 确认身份 / 路径表 ② 注册 Provider 并实现 DIDObjectServer ③ Card / Profile ④ props / actions 映射现有接口 ⑤ 声明并映射 events ⑥ RBAC 对齐 ⑦ read 与权限 DV。下面的 `obj://<service>/...` 只在 current zone 内使用（D4），跨 Zone 持久引用用 https Object URL（D11）；路径表已按 D1–D12 定稿。

### 4.1 OpenDAN（AgentSession / Workspace / Goal / Artifact）

| 对象 | 语义 URI（定稿，D5） | props 来源 | 说明 |
| --- | --- | --- | --- |
| OpenDAN 服务根 | `obj://opendan/` | 托管 Agent 列表 | 只做发现：IndexTrait 列本 Zone 托管的 Agent，返回各自的 `obj://<agent-id>` |
| Agent 自己 | `obj://<agent-id>` | `agent.profile / agent.info` | 根对象，IndexTrait 列 sessions / workspaces / goals / artifacts；Card `id` 复用 Agent DID |
| Session | `obj://<agent-id>/sessions/<sid>` | `session.read`；report / worklog 属性或子对象 | 沿用 owner / root / Agent 自己的读权限；`agent-session read-session` 改为薄壳；Card `id` 为 path DID（D8） |
| Workspace | `obj://<agent-id>/workspaces/<ws-id>` | state/workspaces metadata：name / runtime / url / lease 等 | 保留现有 `ws-` ID；目录内容按挂载关系用 CYFS / 文件引用，业务 metadata 走 OBJ |
| Goal | `obj://<agent-id>/goals/<gid>` | state/goals | 有落盘文件不意味着 Goal 对象等价于该文件 |
| Artifact | `obj://<agent-id>/artifacts/<aid>` | `artifacts.head / versions` | 业务对象、Bucket 当前绑定、DFS 文件及 ObjId 版本分别引用 |

agent-id 自带 Zone（`xiaobai.test.buckyos.io`），因此这些引用可直接跨 Zone 使用；HTTP 路由由 Agent 的 Card / Provider route 决定，不从 URL 字符串推。`agent://self` 废弃，Agent Session SDK 实现计划对应段落改写。AgentRoot 目录使用 `cyfs://<zone>/home/<owner>/agents/<agent-id>/`；Agent 对象及 Session 对象不是该目录的别名。`report.md` 作为文件与 Session report 属性可以声明关联。

落点：[opendan service.rs](../src/frame/opendan/src/service.rs) 旁新增对象 Provider；`--dev` 形态也需支持，便于 libopendan 测试。

### 4.2 msg-center 会话 / 消息 / 附件 / 群组

| 对象 | 语义 URI（建议） | props 来源 / 约束 |
| --- | --- | --- |
| 某人的会话集合 | `obj://msg-center/owners/<owner_did>/sessions`（D12） | `msg.list_sessions`（IndexTrait） |
| 会话 | `obj://msg-center/owners/<owner_did>/sessions/<session_id>` | `msg.get_session_state`；messages 查询映射 `msg.list_session` |
| 消息 | `obj://msg-center/messages/<msg_id>` | `msg.get_message`；确定版本的 NamedObject 可另给 ObjId 型 CYFS 引用，不把原始 `cymsg:<id>` 塞进 host |
| 附件 | ObjId 型 `cyfs://<encoded-objid>/`，或已声明的 Bucket / 文件引用 | 现有 `GET /objects/<id>/content`；读权仍按消息上下文（Bucket 设计 §4.4 / 其中的 Q5） |
| 群组 | `obj://msg-center/groups/<group_did>/...` | 复用已有 Card 材料并补 Profile；按文件系统判据修正业务群组的路径型 CYFS 引用 |

现有 RBAC key `obj://msg-center/{inbox,sent,group_inbox,request_box}/<addr>`、`owners/*/contacts`、`objects`、`notify`、`contact_tags/<app>`、`contact_membership` 与对象路径的关系按 D12 定稿：对象路径直接沿用 `owners/<owner_did>/` 前缀，RBAC 不再多一层映射；归一化时保留 owner / Zone。App / Agent token 的 owner 绑定检查补齐且 `agent_runtime` / `agent` 组的 `obj://msg-center/*` `all` 收窄（G1）后，才开放 Agent 读 owner 的其他会话。群组的投递 target 来自 cyfs-ndn dispatch 协议，按 D11 随 helper 改为 https；群组对象跨 Zone 引用用 `obj://<group_did>/...`（群组 DID 自带 Zone，符合 D6）或 https；同步审计群组的发现、投递和引用消费者，不能只改展示字符串。

### 4.3 msg-center 联系人

| 对象 | 语义 URI（建议） | props 来源 |
| --- | --- | --- |
| 联系人列表 | `obj://msg-center/owners/<owner_did>/contacts`（D12，与现有 RBAC key `owners/*/contacts` 同形） | `contact.list_contacts`（IndexTrait） |
| 联系人记录 | `obj://msg-center/owners/<owner_did>/contacts/<contact_did>` | `contact.get_contact / get_preferred_binding`；Desktop title / icon 从此获取 |

对方 DID 是联系人记录关联的身份，不能自动放入该记录的 `alsoKnownAs`：某 owner 的联系人记录与对方本人不是同一个对象。

### 4.4 AI Workspace

| 对象 | 语义 URI（建议） | props 来源 / 约束 |
| --- | --- | --- |
| 工作区 | `obj://aiworkspace/<ws_id>` | `ws.get_info`；IndexTrait = `doc.outline` |
| 实体（AI Workspace Reference D10 的 URL 投影） | 默认 URL `obj://aiworkspace/<ws_id>/entities/<entity_id>`；带选择器的形式 `...?selector=<canonical-json>&mode=<mode>&version=<version>` 是同一对象的附加 URL（D7） | `doc.read / relations / freshness`；mode 为 live 或 fixed；selector 规范化序列化后进 query |
| 按路径发现 | `obj://aiworkspace/<ws_id>/path/<...>` | `doc.resolve` 返回实体引用；路径重绑定与实体身份分开处理 |
| 发布页 | `https://aiws-<id>.<zone>/<name>` | 确认同一对象时才声明别名；发布快照 / 导出页面与 live 实体使用版本或来源关系 |

wish run 内已有 `ws_read / ws_profile / ws_outline` 可作为 summary props 的材料。抽取可独立访问的读能力时，重新明确运行外授权，不能仅删除 `proc.claim` 检查。`Reference` JSON 仍是内核真相，URL 是保留其语义的投影；共享 TS / Rust 编解码与 schemas 联动。发布页 `https://aiws-<id>.<zone>/<name>` 同时是该工作区跨 Zone 的 https Object URL（D11），Card 在 `alsoKnownAs` 中声明两者关系。

### 4.5 HomeStation：业务路径从 CYFS 改为 OBJ

**已定稿（D10 / D11）。** 当前 feed 由业务发布流程维护，并非 Files 中可直接编辑的目录；其底层数据库或 NamedObject 不使 feed 成为 CYFS 路径。两层引用的定稿形式：① 本 Zone 业务引用 `obj://homestation/home/<user>/...`（D10）；② 跨 Zone 持久引用（entry、Follow、Head 内容、投递 target、reader proof audience）统一用 `https://<zone>/home/<user>/...`（D11），它也是 HomeStation 对象的默认 Object URL（D7）。`cyfs://<zone>/home/...` 形式全部废弃。inbox 投递 target 由 cyfs-ndn dispatch helper 产出，随 D11 改为 https 后 HomeStation 跟随，不在 HomeStation 内单独拼。

| 对象 | 本 Zone OBJ 路径（定稿；跨 Zone 默认 URL 为同路径的 `https://<zone>/...`） | 现有能力 |
| --- | --- | --- |
| 主页 | `obj://homestation/home/<user>` | `/home/<user>/profile`、published.list |
| Feed | `obj://homestation/home/<user>/feed` | display 按 cursor 分页，changes 按 since 拉取增量；IndexTrait 区分两类查询及游标 |
| 可变条目 | `obj://homestation/home/<user>/feed/@/<key>` | item.get、Head；某个版本对应 cyfeed / cyfhead ObjId |
| Inbox | `obj://homestation/home/<user>/inbox` | 投递和状态能力通过 DID Object 声明；不是可直接写文件的目录 |
| Zone 公共流 | `obj://homestation/home/~zone` | 现有汇总能力 |
| 内容版本 / 附件 | `cyfs://<encoded-objid>[.<source-zone>]/<innerpath>` | 保留 CYFS 内容寻址；与可变业务对象建立版本 / 内容关系 |

实施清单：

1. **构造与解析**：更新 [HomeRef / EntryRef](../src/frame/homestation/src/protocol.rs) 及 [对象验证](../src/frame/homestation/src/objects.rs)：规范持久形式改为 `https://<zone>/home/<user>/...`（D11），本 Zone 接受 `obj://homestation/home/<user>/...` 作为别名输入并归一化到 https 形式；覆盖 home、stream、inbox、feed / reactions / follows 条目；不再解析 `cyfs://` 形式。
2. **身份与发现**：更新 [directory](../src/frame/homestation/src/directory.rs)、profile、Zone 索引、流和 inbox 的返回引用；[auth](../src/frame/homestation/src/auth.rs) 中 reader proof 的 audience 由 `cyfs://<zone>/home` 改为 `https://<zone>/home`，生成与校验一起改，绑定正确服务和目标 Zone。
3. **投递与状态**：[delivery](../src/frame/homestation/src/delivery.rs)、[ingress](../src/frame/homestation/src/ingress.rs) 的 target、响应校验、重试和状态查询使用相同规范对象引用。`normalize_cyfs_dispatch_target` 在 cyfs-ndn（[cyfs_dispatch.rs:16](../../cyfs-ndn/src/ndn-lib/src/cyfs_dispatch.rs)），ingress 与 cyfs-gateway 都在调用，**不能只把 HomeRef 的字符串前缀改掉**，也不能在 HomeStation 内单方面改 target 格式；按 D11：helper 在 cyfs-ndn 改为产出 `https://<host><path>`，HomeStation ingress / delivery 只消费其结果；Profile 声明的 inbox action 内部调用 dispatch，结果返回 https / obj 引用。复用内容与签名校验；核实 target 是否进入签名或去重键（若进入，改动随 cyfs-ndn 一起发布）。
4. **持久引用与签名**：更新 [publish](../src/frame/homestation/src/publish.rs)、[pull](../src/frame/homestation/src/pull.rs)、sources / follows、Zone feed 索引和资源 origin，entry 一律写 https 形式。entry URI 会进入已签名 Head / Follow 内容，变更会影响签名和 ObjId，不能直接替换数据库字符串后继续认为签名有效。默认按全新安装验证，不自动增加旧引用兼容。
5. **HTTPS 与 UI**：保留已有公开 HTTPS 展示入口的能力。主要改动点是 [evaluation.rs:1027](../src/frame/homestation/src/evaluation.rs) 的入口解析（当前接受 `cyfs://` / `https://` 同形路径 / 门户链接三种），改为 OBJ + 门户链接；[Desktop links](../src/frame/desktop/src/app/homestation/links.ts) 已只产出 `https://<zone>/homestation/...` 门户链接，不含 cyfs，改动量小；mock / fixtures 同步。Card / Profile 按 §2.1 暴露，不再假设追加 `did.json` 就接通。
6. **Provider 与事件**：为主页、Feed、entry、inbox 暴露 Card / Profile / properties / actions；新增对象事件并按 Q9 接 kevent；浏览器展示 URL、对象根和传输 endpoint 分开处理。
7. **消费者与文档**：同步 HomeStation 协议/架构、schema、SDK / CLI、Agent ZoneShare、Desktop 与相关 cmd_skills。补 `buckyos homestation` 和统一 read 的使用路径，实施时核实已有客户端能力，避免重复建设。

### 4.6 DFS / 内容对象与文件访问

- 补 DFS 的 DID Object Provider：根对象列可见导出，子路径指向与 CYFS / Files 相同的资源，读取和修改遵守原有文件契约；对象描述与文件正文分开表达。
- 对 data→CYFS 目标映射与当前 NFS 导出的差异逐项补齐 Files 可见和直接读写能力；当前未导出、动态新增目录等返回明确的未接入状态，不能误判成 URI 格式错误或改指其他文件。
- NFS 完整 Zone authority 的接入由 resolver 做路由并校验目标身份，不能无条件删掉任何远端 host 后访问本地 NFS。
- ObjId 读复用现有 NDN 能力；NamedObject 不进 `obj://` host（D6），统一走 `cyfs://<O>/`，不要求 NamedObject 获得 Card。
- 按 D1–D3 实现 `file://<device_id>/...` 到 node / executor / runtime 的映射：device id 来自 node_daemon 注册或 agent-daemon 构造的稳定 id；工具未支持时明确报不支持，不回退成调用方的本地文件。

## 5. Governance 与一致性前置项

| # | 事项 | 说明 |
| --- | --- | --- |
| G1 | msg-center 对 App / Agent token 补 owner 绑定检查，并收窄 RBAC | 三件事：① `authorize_owner_read_identity` 的 `_ => Ok(())` 分支改为按 token 的 owner / sub 校验；② `None` 分支定性（何种 token 没有 caller identity，是否应放行）；③ [rbac_config.rs:214-215](../src/kernel/buckyos-api/src/rbac_config.rs) 的 `agent_runtime` / `agent` 组 `obj://msg-center/*` `all` 改为与 Message Center.md §5.6 一致的只读观察范围。接通 §4.2 前核实真实 token 结构及 sub / owner 关系，覆盖跨 owner 拒绝；不能仅依赖 OpenDAN 当前只查自己 inbox 的限制。属于第一阶段的 RBAC 设计（§6 第 1 步） |
| G2 | Profile 的 `x-buckyos:rbac / risk` 与协议、Governance 对齐 | 显式区分只读、幂等写、其他写动作；只读与幂等不是同一分类 |
| G3 | read / resolve 的 RouteTrace 进入 worklog | 记录输入、来源上下文、规范身份、路由及失败原因；审计输出不泄露 token 或签名访问凭据 |
| G4 | 别名与跨协议映射的授权一致性 | 同一资源不因换 scheme / host 简写扩大权限；身份归一化保留 Zone / owner，缓存保留视图、版本和授权上下文 |
| G5 | HomeStation 跨 Zone 引用与投递约束 | reader proof audience、对象签名、entry、target 和动作权限一致；引用格式变更与内容验证一起交付 |

## 6. 实施顺序与联动范围

实施分三段，顺序固定，前一段定稿后才开后一段：**先定义，再改服务，最后改 Agent 工具。**

### 第 1 段：系统 URL 定义文档 + 基于 URL 的 RBAC 配置设计（已完成 2026-10-29）

产出是文档与规则，不改运行代码。下列 1–5 均已完成，落点见文件头"第 1 段产出"。第 1 段同时定下的两个具体格式：kevent 对象事件 key `/obj/<host>/<path...>/<event>`（含 `.`、`:` 的段用 `x` 前缀 hex），default device 配置键 `system/default_device`。

1. ~~填完 §1.5 的裁决列~~（已完成）。~~开工前确认 D7、D11 的解读~~（已确认）。
2. 按 D1–D12 回写 [key url.md](../doc/key%20url.md)：`obj://` host 段两类规则与服务名表（D4 / D6，删除"带类型的实体 id"一节和 `config.alice.bns.did` 展开段）、`file://<device_id>` 与空 host / default device 规则（D1–D3）、跨 Zone 用 https 的规则（D11）、默认 URL 与 alsoKnownAs（D7）、version / selector query 契约（D7）、kevent 命名（D9）、§1.2 转换边界与"不是无条件等价"的清单。同步删除 [统一url的思考](../doc/opendan/protocol/统一url的思考.md) 中的展开写法。它是唯一规范源，Prompt / SDK / 网关 / 工具都引用它。
3. 同步 [基础协议](../../buckyos-base/doc/did-object-protocol.md)（Card 发现 §6.1 已是目标规则，不改；补 Object DID 约定 Q8）与 [Agent Spec](../doc/opendan/protocol/Agent%20DID-Object%20Protocol%20Spec.md)（§4 / §14 的 `{url}/did.json` 写法改为与基础协议一致）；Q5 的结果回写 [Agent Session SDK 实现计划](../doc/opendan/Agent%20Session%20SDK%20实现计划.md) 的 `agent://self` 段。
4. 基于 URL 的 RBAC 配置设计：以 [rbac_config.rs](../src/kernel/buckyos-api/src/rbac_config.rs) 现有 `obj://config/*`、`obj://msg-center/...`、`cyfs:///users/{users}/*` 为基线，写出每个服务的资源 key 布局与对象路径的关系（Q12 决定 msg-center；OpenDAN / AI Workspace / HomeStation / DFS 的 key 在此一并定），`agent_runtime` / `agent` 组对各服务的默认权限（G1 ③），以及 G2 的 `x-buckyos:rbac / risk` 与 RBAC 动作（read / write / all / create）的对应表。别名归一化后的资源 key 必须唯一（G4）。
5. §2 的 Provider 最小契约与 §4 路径表已按 D1–D12 定稿；本步只需把它们作为第 2 段各服务的验收基线，不再改动。

### 第 2 段：现有服务按新定义修改（大活）

每个服务按 §4 的七步（身份 / 路径表 → Provider → Card / Profile → props / actions → events → RBAC → DV）推进，可并行，但先做共享层：

1. 共享层：buckyos-base client / server 的 Card 发现对齐 §2.1；buckyos-api 增加 `object_url` 归一化模块（纯函数部分）与 RBAC 配置更新；sys_config_service 的别名实现迁到共享模块或改为调用它（Q4）；websdk 增加同规则的 `object_url.ts` 与共享测试向量（A2 的规则部分，不含 Agent 路由）。
2. D11 投递 target 改 https：cyfs-ndn `cyfs_dispatch.rs` → cyfs-gateway（dispatch / named_inbox_cache）→ msg-center 群组 → HomeStation 的顺序发布；先核实 target 是否进入签名 / 去重键，并在 cyfs-gateway 测试中更新 fixtures。D9 统一 kevent 命名也在本步与共享层一起定格式。
3. G1 → msg-center 会话 / 联系人 / 群组 Provider（§4.2 / §4.3）。
4. OpenDAN Provider（§4.1，按 Q5）；`--dev` 形态同样支持。
5. HomeStation §4.5 引用、发现、鉴权、投递与签名改造（按 Q10 / Q11），按全新安装验证，不加旧引用兼容。
6. AI Workspace Reference 的 URL 投影与运行外授权（§4.4，按 Q7）。
7. DFS Provider 与文件 host → node / executor 映射（§4.6，按 Q1–Q3）。
8. 每个服务完成后跑 §7.1 的解析向量和 §7.3 / §7.2 中属于该服务的项；`buckyos obj resolve / read`（A8 的 CLI 部分）在本段随 websdk 一起给出，用于 DV。

#### 第 2 段实施记录（2026-10-29，未提交）

共享层：
- buckyos-base：`DIDObjectCard::default_card_url` 先分类（路径型原 URL、目录型补 `did.json`）；`DIDObjectClient::resolve_card` 从 `BuckyOS-DID-Document` 头或 body（JSON / JWT）取 Card，两者须一致；`DIDObjectHttpServer` 对对象路径 GET 返回 Card、`?type=profile` 返回 Profile；`DIDObjectRequestContext` 带 `headers` / `query`；`DIDObjectHttpServer::inner()`。测试改为关闭 WebProvider 兼容路由也能解析。
- buckyos-api：`object_url.rs`（严格解析 / 规范化 / host 段两类校验 / `rbac_resource_key` / `object_event_key` / path DID / `expand_input_shorthand` / `file_url_device` / `DEFAULT_DEVICE_KEY`）+ `testdata/object_url_vectors.json`；`did_object_provider.rs`（`KrpcObjectProvider`：按路径模式声明对象种类，属性 / 索引查询 / action 直接调服务已有 kRPC，用调用者自己的 token；`RpcHandlerBackend`；`buffered_request` 供 axum 服务）；`aiws_reference_url` / `parse_aiws_reference_url`；`SystemConfigClient::default_device`；`rbac_literal` 移入。sys_config_service 的前缀剥离改调 `config_key_from_url`。
- websdk：`src/object_url.ts`（同规则，`objectUrl` 命名空间导出）+ `tests/fixtures/object_url_vectors.json`（与 buckyos-api 同一份，测试会比对兄弟仓副本）；`aiwsReferenceUrl` / `parseAiwsReferenceUrl`；CLI `buckyos obj resolve|read`（`cli/modules/obj.ts`）。
- RBAC（`rbac_config.rs` + scheduler）：删 `agent_runtime` / `agent` 对 `obj://msg-center/*` 的 `all`，换成 agent 组对 `owners/*` 只读、objects / group_inbox 读、notify；`obj://task/` → `obj://taskmgr/users/`；aiworkspace / homestation / opendan 组级规则；`system/default_device`（scheduler 在第一个 OOD 写入）；scheduler 为每个 Agent 生成 `obj://<agent-id>[/*]` 对 Agent 自己 `all`、对 owner 只读的精确规则。msg-center RBAC key 不再把 `.` 编成 `%2E`，精确规则经 `rbac_literal` 转义。

D11：cyfs-ndn `normalize_cyfs_dispatch_target` 产出 `https://<host><path>`（CYFS Protocol 文档同步）；cyfs-gateway named_inbox_cache 的 zone 前缀校验跟随；msg-center 群组 / 原生投递 / 本地标签 `https://local/<did>/inbox`；bucket_store `Origin.source` 接受 `https://`。target 进入群组请求签名的 `audience` 与 dispatch 去重键，发送端、网关缓存、接收端用同一 helper，因此一起变；旧 `cyfs://` 缓存项会被网关视为外来项丢弃（全新安装）。

D9：kevent 统一 `/obj/<host>/<path>/<event>`：msg-center 邮箱 `/obj/msg-center/owners/<owner>/sessions[/<sid>]/<box>`、联系人 `.../owners/<owner>/contacts/changed`、群组 `/obj/<group-host>/changed`、投递队列 `/obj/msg-center/delivery/<transport>/changed`；aiworkspace `/obj/aiworkspace/<ws>/{head,locks}`；taskmgr `/obj/taskmgr/<task_id>/{changed,tree}`。订阅端：HomeStation、OpenDAN ui、workflow、websdk / CLI、Desktop MessageHub 与 My Network、lib_opendan kevent bridge（`object_event.object` 现在是 `obj://` URL，确定性转 kevent key / pattern）、agent-did-object-lib EventBridge（同一编码）。

G1：msg-center `authorize_owner_{read,write}_identity` 按 principal 判定：User = 自己 + 可观察的本人 Agent；Agent token（sub = Agent DID）= 自己的 mailbox，只读观察 owner；App token 若是 Agent runtime 同 Agent，否则同 User；System / Device 与进程内调用（`caller_identity` 为 `None` 只在进程内调用出现）放行。

Provider（`/kapi/<host>/obj/...`）：msg-center（`owners/<did>/sessions[/<sid>]`、`owners/<did>/contacts[/<did>]`、`messages/<msg_id>`，事件 inbox / sent / group_inbox / request_box / changed）；OpenDAN（`obj://<agent-id>` 根与 sessions / workspaces / goals / artifacts，挂在 `/kapi/<agent_id>/obj`，`/kapi/opendan/obj`、`/api/opendan/obj` 同一对象，`--dev` 同样挂载）；HomeStation（home / feed / feed/@/<key> / inbox，Card 默认 URL 是同路径 https）；AI Workspace（工作区、`entities/<id>[?mode&selector&version]`、`path/<...>`，读走 kRPC 入口，无授权仍是 NOT_FOUND）；DFS（nfs-server `/kapi/dfs/obj/<path>`，网关新增 `/kapi/dfs/*` → nfs-server，与 NFSP 同一路径 ACL）。

验证（本机，cargo 经 `--config` 指向本地 buckyos-base / cyfs-ndn）：buckyos-base `name-lib` / `name-client` / `buckyos-http-server` 全量；cyfs-ndn `ndn-lib` / `bucket_store` 全量；cyfs-gateway `cyfs-gateway-lib` 的 named_inbox / dispatch；buckyos `buckyos-api`、`system_config`、`task_manager`、`workflow`、`scheduler`、`libopendan`、`agent-did-object-lib`、`agent_tool`、`opendan`、`msg_center`、`homestation`、`aiworkspace`、`nfs_server` 全量（`--test-threads=1`）；websdk jest 全量（593 通过）与 CLI deno 全量（122 通过）；Desktop `tsc -b` 与改动文件 eslint。改动前就失败、与本段无关的测试：scheduler `test_create_pod_instance_no_suitable_node`（另两个 RBAC 测试在 git 版 base 下失败、本地 base 下通过），libopendan `context_limit` 的 3 个（stash 基线同样失败）。未做：真实 Zone DV、cyfs-gateway `test_boot_gatweay` 网关 DV、Desktop e2e。

遗留（没有在本段完成，原因各自写明）：
1. kevent 订阅按对象 key 的 `subscribe` enforce：kevent 服务目前不识别订阅者身份，需先让 kevent 携带调用者 token。
2. msg-center 群组实体对象 `obj://<group-did-host>/...` 的 Provider（群组 HTTP 已有 `did.json`，未接 `KrpcObjectProvider`）。
3. `obj://opendan` 发现根：Zone 内没有 Zone 级 OpenDAN 服务可承接（每个 Agent 一个 loader），候选宿主是 control-panel `agent.list`。
4. HomeStation inbox 投递作为 Profile action（现仍是 `PUT https://<zone>/home/<user>/inbox`，Provider 只给 target）；各 Provider 的写 action 与事件 WebSocket 绑定（只声明了本 Zone kevent key）。
5. agent-daemon 的 device id 仍是注册时的 `executor_id`，"稳定、用户可读"的生成规则属于注册流程，未改。
6. 改动前就失败的测试（见上方验证）未修。

发布顺序：buckyos-base（Card 发现、ctx headers、`inner()`）→ cyfs-ndn（https dispatch target、bucket origin）→ cyfs-gateway（缓存 zone 前缀）→ buckyos + websdk（`Cargo.lock` 里 base / ndn 的提交需随推送更新）。各仓之间是 breaking change，按全新安装验证。

#### 第 2 段 DV（2026-10-29，单 Zone test.buckyos.io，全新安装 + 重建 paios 镜像）

已提交 9589c91f6（buckyos）/ 35bac92（base）/ 20db686（ndn）/ 1aa9edd（gateway）/ cb08cca（websdk）后的构建。脚本 `test/object_url_dv/`：
- `object_url_dv.ts` 7/7：DFS（Card / info / 列表，bob 读 devtest 家目录 403，根 Profile 匿名可读）、msg-center 会话与联系人（bob 403）、msg-center 会话变化的 kevent `/obj/msg-center/owners/<hex>/sessions/<sid>/sent`、taskmgr 任务变化的 `/obj/taskmgr/<id>/changed`、HomeStation（默认 URL `https://<zone>/home/<user>`，feed 行全是 https 入口，inbox target https）、AI Workspace（工作区 / 实体 / 按路径，bob 无授权 404）、Agent `obj://jarvis.test.buckyos.io` 及其 sessions。
- `agent_g1_dv.ts` 4/4：用设备密钥按 node-daemon 方式签 LoginAssertion → verify-hub `login_by_jwt` 拿到 Agent runtime 的 App 会话；读自己邮箱、只读观察 owner、读 bob 被拒、以 owner 身份建会话被拒。
- 既有套件：HomeStation smoke 全过（第 5 步按 10-23 后的 NDM import + `object.register` 流程更新）；msg-center 群组 ALL PASSED、My Network 通过；nfs smoke 5/5、NDM 8/8；网关 `test_boot_gatweay` 33/33（新增 `req_dfs_obj_ok`）；宿主机 `buckyos obj resolve|read` 经节点网关可用。

DV 中顺带修正：msg-center 拒绝信息显示实际身份（Agent DID）而非 token 的 `sub`；CLI `obj resolve` 无会话时给网关相对路径（原先拼出 `local:///...`）。

跨 Zone（ssotest VM 组，10-29 晚）：按用户同意重建了 bob 种子（旧 env 备份在 `~/buckycli-backup-20261029/bob.bns.did`；新种子由 `prepareSeedIdentity` 生成，口令哈希已核对），四个种子包全部通过 `make_sn_config` 校验，四台 VM 全新安装并启动。
- msg-center 原生路由 `test/test_msg_center/test_native_routes_dv.mjs` 通过：bob 查 alice、charlie 可路由，bob → alice 的原生消息经 `did:bns:msg-hub.bob` 投递进 alice 的 REQUEST_BOX。
- `test/object_url_dv/homestation_cross_zone_dv.mjs`：前两步通过——bob 的 profile 给出 `https://bob.web3.devtests.org/home/bob/{feed,inbox}`，feed Card 默认 URL 为 https，发表后的入口是 `https://.../home/bob/feed/@/<key>`；alice `sources.resolve` / `sources.follow` 成功。第 3 步拉取被拒：bob 校验 alice 的 reader proof 时返回 401（`signer_key` 解析远端设备钥未指定 doc type，解析器按 zone 取文档后被绑定校验拒收）。这属于跨 Zone 鉴权，按用户安排留给 `notepads/cross-zone-auth-overview.md`（细节已记在其 §5.1），第 3、4 步（拉取、评论经 https inbox target 投递）待该项修复后重跑本脚本。

DV 发现的既有问题（与本段无关，按用户要求随后修复）：
1. `data/homestation` 等服务库不随 `start.py --all` 清理——这是 RDB `user_data` 分区（`data/<appid>/`）的既定语义，不改清理范围；`start.py` 新增 `--wipe-data`（仅与 `--all` / `--reinstall` 同用），全新安装前删除 `data/` 与 `storage/`，得到空 Zone（AGENTS.md 已写）。
2. OpenDAN 归档旧 AgentRoot：AgentRoot 是容器里的 bind mount，rename 必然 EBUSY（跨挂载为 EXDEV）。`rootfs::archive_root` 在这两种错误下改为把内容复制进归档目录、再清空挂载点本身（不跟随符号链接）。
3. msg-center `ui_session` 状态：调用者不是 System 时，`update/get/list_ui_session_state` 默认作用于调用者自己的 owner 分域（与 `owner_*_ui_session_state` 同一授权），不再落到无 users 组规则的全局 `obj://msg-center/ui-state`；System 调用者保持原路径。
4. `test/kevent_kmsg/task_mgr/task_mgr_kevent_dv.ts` 按 TaskMgr 2.0 重写：口令登录，`raw/v1` + `HumanSet` 建任务，经网关 `/kapi/kevent/stream` 订阅 `/obj/taskmgr/<id>/changed` 与根任务的 `/obj/taskmgr/<root>/tree`（建子任务触发），结束时 `request_control Cancel`。
5. `test/deno.json` 设 `"lock": false`（`deno.lock` 本就 gitignore，本地旧锁文件会锁住旧 websdk dist 哈希）。
6. 跨 Zone DV 中发现：scheduler 只在 Zone 文档有 `sn` 时生成 `zone_tls`（443）与 ACME（DNS-01 经 SN）；10-22 起 `wan_dyn` 不写 Zone SN，按新规则重建的 bob 因此没有 443。`update_node_gateway_config` 改为 Zone SN 缺省时用设备的 `ddns_sn_url`（`node_sn_api_url`，附单测）；VM 更新后 bob 直连 HTTPS 正常（测试 CA 证书）。同类遗留未改：SN AI provider（`derive_sn_ai_provider_endpoints`）也只认 Zone SN，`wan_dyn` Zone 每 5 秒告警一次且不配 SN AI provider，要不要改成取哪台设备的 DDNS SN 需另定。
7. `test/test_msg_center/test_messagehub_sessions.ts` 跟到现行契约：第 5 步不带 owner 的 ui state 即调用者自己的（上面第 3 条）；第 7 步 `/kapi/msg-center/objects/<id>` 自 10-26 起只服务经消息读取的附件与托管群消息，私信本身的 id 返回 404（`doc/message_hub/Message Center.md`），断言改为 401 / 404。现全程 ALL PASSED。

修复后本机复验（`start.py` 更新重启，保留数据）：`object_url_dv.ts` 7/7、`agent_g1_dv.ts` 4/4、`task_mgr_kevent_dv.ts` 通过、`test_messagehub_sessions.ts` ALL PASSED；OpenDAN 归档在真实容器里验证：建临时 Agent `dvarch` 后把其 AgentRoot 的 `.meta/identity.json` 改成别的 Agent，重启容器，旧内容整体进 `.archived/`、挂载点原地清空后重新初始化、Agent 回到 running，随后删除该 Agent。单测：msg_center 207 通过、opendan 全过、start.py 11 通过；scheduler 单线程只剩既有的 `test_create_pod_instance_no_suitable_node`（并行跑时两个 RBAC 测试因共享全局 enforcer 偶发失败，非本次引入）。

### 第 3 段：Agent 工具

Provider 和 RBAC 稳定后再改 Agent 侧，避免工具先于服务定型：

1. A2 的路由部分 + A1 统一 read + A9 输出归一化：`bash_core` 接入对象库，`ObjectRouteConfig` 增加 obj / cyfs / `file://<device_id>` 路由并删除 `agent://` 路由，read_tool 按 D1 接受 device id host。
2. A3 Agent 自身对象、A4 Hint 带 url、A5 known objects。
3. A6 subscribe_event、A7 xcall Governance（G2 / G3）。
4. 同步内置手册与 cmd_skills：`build-in agent-tool手册`（read 一节现已落后）、`buckyos_admin`、`opendan_agent_dev`、`aiworkspace_run` 及受影响的产物 / 分享手册；跑 §7.2。

#### 第 3 段实施方案（2026-10-29 开工时自定，实施中修订）

- **一个 read，两层组合**（A1）：Session 的 `read` 仍是 agent_tool 的 `ReadTool`（文件、`cyfile:` 等 ObjId、range / first_chunk / offset / limit、filesystem policy 都不变）；其余 URL（`obj://`、`cyfs://`、`https://`、`http://`、host 不是本机的 `file://<device_id>/`）交给宿主注入的 `UrlReader`。对象库实现 `UrlReader`，OpenDAN 在 `zone_xllm_deps` 注入；dev CLI `agent_tool read` 用同一组合，所以 CLI 与 Session 行为一致。不让 bash_core 直接依赖对象库（对象库依赖 agent_tool，会成环）。llm_context 层的接口改动记在 `llm-context-agent-runtime-todo.md` §12。
- **路由与端点**（A2）：内置路由表从 dev CLI 移进对象库（`ObjectRouteConfig::builtin()`）：`obj` → did_object；`cyfs` → cyfs adapter；`https` / `http` → did_object（按基础协议 §6.1 发现 Card，响应不是 Card 时退回普通网页内容）；`file` → filesystem（仅自定义配置与 dev 用）。`obj://` 到 Provider 端点的映射进共享模块（buckyos-api `object_endpoint` / websdk `objectEndpoint`，共用向量），CLI 的 `endpointOf` 改用它。容器内的网关源是节点网关（`http://<host-gw>:3180`），Card / Profile 的端点按转发头生成，已实测可直接跟随。
- **凭据**：对象库对当前 Zone 的源（节点网关、Zone 主机名）带进程运行时的 session token；其他主机不带。Card 指向别处时 token 会随同一个 client 发出——Provider 都是本 Zone 服务，列为已知风险。
- **文件归属**（D1–D3、A9 的读侧）：`$BUCKYOS_ROOT/data/<rel>` 是 DFS 文件 `cyfs:///<rel>`，与设备无关；Agent 容器把 owner 家目录与 AgentRoot 同路径挂进来，所以 `cyfs:///home/<owner>/…` 先按本地挂载读，读不到再走 DFS Provider（只有对象信息与列表，DFS 文件正文经 `/nfs/v1/read` 留作后续）。`file://<device_id>/p`：p 在 data 下按 DFS 文件读；否则是该设备上的文件，node executor 路由未接，明确报告不支持。容器私有路径（`/tmp`、镜像层）不转 CYFS。
- **A3**：删除 `agent://` 路由、`agent_runtime` adapter 与 `AdapterType::AgentRuntime`；Agent 自身对象就是 `obj://<agent-id>/…`，走 did_object。

#### 第 3 段实施记录（2026-10-29，未提交）

- **A1 / A2**：agent_tool `read` 增 `UrlReader` 注入点与 `property` 参数（llm_context 层，见 `llm-context-agent-runtime-todo.md` §12）；空字符串参数等同未给、`range` 优先于 `offset` / `limit`（真实 Session 里模型按严格 schema 填满每个字段，旧规则让每次 read 都失败）。buckyos-api `object_endpoint`、`data_path_to_dfs` / `dfs_to_data_path`、`persistent_object_ref`、`readable_object_url`，websdk `objectEndpoint`（CLI `endpointOf` 改用），共用向量新增端点用例。对象库：`ObjectRouteConfig::builtin()`；did_object 经 `zone::ZoneAccess`（节点网关源、Zone 主机名、session token 只发本 Zone 源）定位 `obj://` 端点，本 Zone https 走节点网关，非 Card 的 https 退回网页内容，对象说明含身份 / 声明的属性动作事件（带 risk）/ 摘要属性值，`property` 读单个属性；新增 `cyfs` adapter（本机挂载读 Zone 文件与目录、未挂载走 DFS 对象、`cyfs://<objid>/` 读对象存储、其他设备的 `file://` 与其他 Zone 的 `cyfs://` 明确不支持）；`ObjectUrlReader`。OpenDAN `zone_xllm_deps` 注入；dev CLI `agent_tool read` 改为同一 `ReadTool` + `ObjectUrlReader`（参数与 Session 相同，首个非 `key=value` 参数是 uri），访问对象前按进程环境建 runtime。
- **A3**：删 `agent_runtime` adapter、`AdapterType::AgentRuntime`、`agent://` 路由与 dev CLI 的旧默认路由表。
- **A4**：agent_memory `validate_object_path` 接受规范对象 URL（按段嵌套）；libopendan 记录感知时把 objects / exceptions 与 `source_ref.uri` 转成持久引用（URL 规范化、data 下绝对路径转 `cyfs:///…`、其余原样）；memory `Hint.object_url`、prompt `Hint.object`，`<hint object="…">` 与 background 行 `about <url>`；consolidation 工具对 `objects` 的说明加 URL。
- **A5**：`DefaultAssembler.known_objects`（OpenDAN 打开）→ 系统文本 `## Objects`。
- **A6**：kevent bridge 原先把对象 URL 当 kevent pattern 传给 `create_event_reader`，对象订阅永远收不到事件，已改为传 pattern；msg-center 的会话集合 Profile 声明了 `inbox` 等事件，实际只发在单个会话上，现在 box 变化同时发会话与 owner 会话集合两条（`MailboxAddress::box_event_ids`，订阅 `…/owners/<owner>/**` 的会收到两条唤醒）；bridge 默认摘要只放事件数据（来源与事件名由渲染给出）；`ctl subscribe` 的对象订阅校验并规范化为本 Zone `obj://`（其他 Zone 的 https 对象明确拒绝），订阅 id 按规范形式算。不另做 `subscribe_event` 工具，沿用 `agent-session ctl subscribe`。
- **A7**：`xcall` 走同一内置路由与 Zone 凭据，结果带 Profile 声明的 `risk` 与规范对象 URL；授权仍由 Provider 按调用者身份判（G1 已收窄）。审批门（非 read_only 动作先经用户确认）未做，按"先能力后安全"列为已知风险，手册要求先问用户。
- **顺带修正**：HomeStation Card 的 https 默认 URL 改用 Zone 主机名（`KrpcObjectProvider::with_zone_host`），经节点网关读到的不再是 `https://172.17.0.1:3180/...`。
- **手册**：内置工具手册 `read` 一节重写；AgentTool 开发指南 §5.8；`agent-did-object-lib 需求.md` 去 agent_runtime；`Agent Enviroment.md` §6.1；cmd_skills `ui_core`（read 对象 URL / property / xcall / 对象订阅）、`buckyos_admin`、`opendan_agent_dev`、`aiworkspace_run`。

第 3 段遗留：
1. `xcall` 的审批门：非 `read_only` 动作先经用户确认（现在只有 Provider 的 RBAC 与手册要求先问用户），已知风险。
2. A9 的输出侧：只做了感知记录的对象引用归一化；文件工具 / 产物 / 发消息时的路径还没补成 `file://<device_id>/…` 或 `cyfs:///…`。
3. 读其他设备的 `file://<device_id>/…` 需 node executor 路由（D3）；未挂载的 DFS 文件正文（`/nfs/v1/read`）只给 DFS 对象信息。
4. HomeStation 的 https 路径不带 `BuckyOS-DID-Document`，读 https 链接得到的是网页 JSON，而不是对象说明（`obj://homestation/...` 正常）。
5. OpenDAN 不发 `obj://<agent-id>/sessions/<sid>` 的对象事件（§7.2"订阅 Session 的 finished"仍靠 `semi:session` 与子 Session 汇报）。
6. AI Workspace 无根对象，`## Objects` 不列它；跨 Zone 对象读取与事件归跨 Zone 鉴权 TODO。

#### 第 3 段 DV（2026-10-29，本机单 Zone，重建 paios 镜像）

- 容器内 `agent_tool read`（Agent 自己的 App 会话）：`obj://jarvis.test.buckyos.io`、owner 的 msg-center 会话（只读观察）、`cyfs:///home/devtest/`（本机挂载）、本 Zone https（`/home/devtest/feed`）、`obj://homestation/home/devtest/feed` 都读到；bob 的邮箱与 bob 的文件 `forbidden`。
- 真实 Session：Jarvis 的 UI Session 用原生 `read` 读 `obj://msg-center/owners/did:bns:devtest/sessions`，得到对象说明并答出声明的动作 `page,query`（重建前这一轮因严格 schema 填满参数而每次 read 都失败，见 A1）。
- `## Objects` 出现在该 Session 新一轮的系统文本里（run.json），六条引用与预期一致。
- A6：Jarvis 在 shell 里 `agent-session ctl subscribe semi:object:obj://msg-center/owners/did:web:jarvis.test.buckyos.io/sessions#inbox`，下一条消息进来时该轮 `turn_started.events` 带 `kevent:/obj/msg-center/owners/<hex>/sessions/inbox:…`，background 行出现 `object:obj://… inbox`；测试订阅随后已由 Jarvis `ctl unsubscribe` 撤掉。
- A7：容器内 `agent_tool xcall obj://msg-center/owners/did:bns:devtest/sessions query` 成功、结果 `risk=read_only`；对 bob 的同一调用 `forbidden`。
- 未实测：A4 的 Hint 带 URL 只有单测（真实 Memory 里需要一条 scope 指向对象 URL 的认知）；跨 Zone 对象读取与事件（另一 TODO）；node executor 读其他设备文件。
- 期间发现：宿主根分区 0 字节可用（`/tmp/dev-cache-root/cargo-target` 394 GB），apt 签名校验因此失败；已删其中 `debug/incremental`（97 GB，可重建缓存）。

实施时的联动清单：buckyos-api、对象库 / agent_tool / libopendan、服务 Provider、网关 / node_executor、buckyos-base、**cyfs-ndn、cyfs-gateway（Q11）**、websdk / CLI、Desktop、协议文档、schema / fixtures、内置 cmd_skills。**本次仅修订设计 TODO，运行行为和现有命令未变，因此不修改代码或 cmd_skills；`doc/key url.md` 的一行未提交改动随本 TODO 一起提交。**

## 7. 验收清单（待实施后执行）

### 7.1 语义与解析

- [ ] `obj://<service>/P` 只解析到 current zone 的服务；同一对象的本 Zone `obj://` 形式、跨 Zone https 默认 URL、Card 中声明的其他 URL 定位同一对象（D4 / D7 / D11）；不同 Zone 的同名服务不混淆。
- [ ] `obj://` host 段校验：服务名表之外、且不是带 Zone 的 did:bns 实体 id（agent-id / user-id / group DID）的 host 报错；`obj://task_xxx/`、`obj://cyfile:.../`、`obj://config.alice.bns.did/` 均被拒绝（D6）。
- [ ] Rust / TS 的解析向量一致：相对路径、绝对路径、空 host、`file://<device_id>`、current-zone、base32 ObjId、innerpath、编码字符、尾斜杠、version / selector。
- [ ] `cyfs:///x` 与 `cyfs://x` 不混淆；原始 `cyfile:<hash>` 通过 ObjId 编码进入 host，非法或未知 authority 不猜成本地路径。
- [ ] 没有来源上下文的 `file:///path` 按 Zone default device 解释（D2）；Agent / SSH / executor 产出的路径按产生它的 runtime 解释并在输出时补 device id（D1 / D3）。相同的 `/tmp/a.md` 不跨环境串读。
- [ ] 来源与挂载明确的 data / AgentRoot 文件正确生成完整 CYFS 引用；现阶段未导出目录、根层普通文件等报告访问能力缺失。来源不明、越界或无 DFS 映射的临时文件不伪造等价 CYFS 引用。
- [ ] 对可写的非 ObjId CYFS 测试文件，在 Files / 传统 FS 中修改后，CYFS 与 OBJ-DFS 读取观察到同一资源的新状态；权限拒绝仍生效。
- [ ] ObjId 旧版本内容保持不变；编辑产生新 ObjId；live 路径、业务对象与快照不会被误判为永久别名。
- [ ] 已知 public HTTPS 能按发布映射关联资源；任意 HTTPS、业务首页或内容导出不会自动转为 CYFS / OBJ。
- [ ] Card 的路径型 / 目录型发现及 query 保留符合基础协议；resolver 不把所有对象强制追加 `/did.json`；client 不再在分类前去掉尾斜杠（`https://z/devices/` 取 `.../devices/did.json`，`https://z/devices/cam01` 直取 body / 响应头）。
- [ ] 同一 AI Workspace Reference 经规范化序列化只产生一个 URL；键序或空白不同的 selector 得到相同缓存键（Q7）。
- [ ] msg-center 对象路径与 RBAC 资源 key 的对应关系经 `test_rbac` 验证：agent 组对他人 owner 的会话 / 联系人无读权（Q12 / G1）。

### 7.2 Agent 与 Provider（真实 Zone DV，全新安装）

- [ ] Jarvis 收到 AI Workspace 的完整 OBJ 引用，read 返回 object document，再按声明的 property / selector 获取内容。
- [ ] Memory Hint 保存完整引用及版本信息；跨 Session / Agent 后仍能定位原对象，不依赖接收方 cwd 或当前 Zone 猜测。
- [ ] 订阅 Session 的 finished 事件后，子 Session 结束唤醒父 Session；别名重复订阅按既定规则去重。
- [ ] 普通用户 Agent 读取其他 owner 的 msg-center 对象被拒绝；换成完整 host、别名或其他入口不能绕过。
- [ ] `agent_tool read` 与 Session 内 read 对同一引用行为一致；不支持的协议能力与权限拒绝分别报告。
- [ ] 幂等写 action 仍经过写权限和副作用治理；Profile 未声明的方法不由路径猜测调用。

### 7.3 HomeStation（本 Zone 及跨 Zone）

- [ ] Feed / inbox / 可变 entry 使用 OBJ；`cyfs:///home/alice/feed` 若存在，仅表示文件系统中的资源，不被重定向成发表流。
- [ ] HTTPS 展示链接与 OBJ 引用定位同一业务条目；Desktop、资源 origin、Follow、Zone feed 索引不再生成旧的业务 CYFS 引用。
- [ ] 两个 Zone 完成发现 → 发布 / 订阅 → 投递 → Head / Feed 拉取；签名、ObjId、entry（https 形式）与目标身份一致。
- [ ] reader proof 的 audience 为 `https://<zone>/home` 时通过，错误服务 / Zone audience 或旧的 `cyfs://` audience 被拒绝。
- [ ] 投递成功、拒绝、重试、去重与状态查询使用同一 https target（D11）：cyfs-ndn、cyfs-gateway 离线 inbox 缓存、msg-center 群组、HomeStation 对同一投递得到同一 target，发送与接收不会分别生成 https / cyfs target；网关缓存对旧 `cyfs://` target 不再接受。
- [ ] 内容附件和固定版本仍可按 ObjId 经 CYFS 读取；修改业务 Feed 走声明的 DID Object action。
- [ ] 远端事件（D9 首期只做本 Zone）未交付时明确列为未完成，不能以本 Zone 只读通过替代整体完成。

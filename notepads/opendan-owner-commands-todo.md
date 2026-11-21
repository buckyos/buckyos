# OpenDAN 个人资源设计 TODO · 用 `xcall <对象引用> <动作>` 操作主人的状态

设计稿 · 2026-11-20。核对基线为 BuckyOS `a72c674fc`、buckyos-websdk `2a5789d`。本稿是通用扩展机制的首个业务用例。先实现别名注册、按 Profile 读取和调用本地动作、链接已有对象，再按本文接入主人、日历与待办。[前置 TODO · DID Object 本地扩展与别名](opendan-object-local-extensions-todo.md)

## 1. 要让哪几件事变简单

主人说“把周五的设计评审加到我的日历”，Jarvis 执行一次 **`xcall owner.cal add`**，传入标题和时间，得到人能打开的日程记录。已有 Alice 的个人日历用途对象 URL 时，`xcall "$calendar_ref" add` 使用同一动作和参数。对象决定“对谁的哪一份资源”，动作参数决定“做什么”。日历、待办各自可以有名为 `add` 的动作，参数由该对象的 Profile 定义。

默认把 `owner` 和 `owner.cal` 安装为两个独立的本地别名。`xcall owner notify` 执行提醒脚本，`xcall owner.cal add` 执行用 AIWS 基础 API 写入主人的具名日历的 TypeScript 脚本。人和 Agent 在桌面看到同一条记录。已有提供相同日历动作的对象时，也可以只把 `owner.cal` 链接到它的完整 URL，`xcall owner notify` 继续使用原脚本。已有记录凭创建时的回执定位。

主人说“看看小林周五下午有没有空”，第一次可以查通讯录、消歧并读对方开放的个人对象。下次拿到这个人的稳定忙闲入口和已知动作，就可以直接 `xcall`。小林和主人在同一个 Zone，双方可以在本地找到彼此。能看忙闲、能看标题、能代改日程、能发邀请是四件不同的事。

**首版交付 M0–M3，以及 M4 中的本 Zone 忙闲直达。** 位置、邀请、实时通话和团队分派按各自的 Provider 分项交付。一次调用按模型执行的业务命令计数，运行时照常解析、鉴权和提交。空日历、无权访问和后端失联要给出不同结果。

## 2. 当前能复用什么

| 入口 | 当前行为与实现约束 |
| --- | --- |
| OpenDAN 身份 | Loader 已取得 Agent 的 owner DID。Zone 会话已注入主人的消息会话、HomeStation 和文件。工具使用 Agent App 的会话身份，AIWS 能识别经过绑定的 Agent 及其 owner。当前消息桥接受主人发起的请求，群内还要求主人 @Agent。群成员被提到可以是操作对象，不能据此改写调用者。[Loader](../src/frame/opendan/src/loader.rs#L223) · [会话对象](../src/frame/lib_opendan/src/runner/assembler.rs#L238) · [消息桥](../src/frame/lib_opendan/src/bridge/msg.rs#L197) · [AIWS 鉴权](../src/frame/aiworkspace/server/src/auth.rs#L229) |
| 原生个人日程 | `ScheduleClient.ensure()` 按名取或建 `schedule`，初始化 `calendar`、`todo` 表和日程页。桌面控件和 CLI 读写同一批记录。现有列表也会触发初始化。目前 `buckyos aiws cal` 与 `buckyos aiws todo` 的 handler 已经通过 WebSDK 组合 `ws.ensure`、`doc.query` 和 `doc.commit` 等基础调用，迁移时可以直接复用。OpenDAN 的 `agent-session todo` 管理 Plan 的执行工作。[日程 SDK](../../buckyos-websdk/src/aiws_schedule.ts#L789) · [CLI](../../buckyos-websdk/cli/modules/aiws_schedule.ts#L64) · [人机协作验收](aiworkspace-control-mechanism-todo.md#L424) · [Plan Todo](../src/frame/lib_opendan/src/todos.rs#L1) |
| 默认名字的缺口 | 在临时目录用本地 debug 产物启动独立 AIWS 服务，以一个账号、两个归属该账号的 Agent 测试身份依次执行 `ws.ensure({name:"schedule"})`。前三次均为 `created=true`，返回三个不同 ID，账号再次调用才命中第一个 ID。具名 `ws.create` 和 `ws.ensure` 按 `caller.principal` 查内存索引，建库和重启重建索引按落盘 owner。已有日程集成用例用同一个人类 token 模拟两个客户端。[命名与落盘](../src/frame/aiworkspace/store/src/service.rs#L280) · [现有测试](../src/frame/desktop/tests/aiworkspace/schedule.spec.ts#L23) |
| 对象与 Memory | 现有 `xcall` 能在一次调用内解析 Card/Profile，只调用其中声明的动作。AIWS 通用实体能读 `meta/body/content`，已声明的专用动作是 Wish 阶段和 Source 刷新。新设计里的 `add` 由本地日历扩展或链接目标的 Profile 声明。现有 AIWS 表 URL 只提供其自身 Profile 声明的能力。Hint 可带来源、有效期和 `object_url`。会话 grants 决定谁能召回。[xcall](../src/frame/agent-did-object-lib/src/adapters/did_object.rs#L201) · [AIWS Provider](../src/frame/aiworkspace/server/src/obj_provider.rs#L225) · [Hint](../src/frame/lib_opendan/src/memory/hint.rs#L122) · [可见范围](../src/frame/agent_tool/src/agent_memory/model.rs#L496) |
| 命令入口 | OpenDAN 已有 `agent_tool xcall <对象URL> <动作> --params JSON`，也支持 `key=value`。Rust 多入口程序能按 basename 分派到 `xcall`，标准 baseline 和镜像目前提供 `agent_tool`。WebSDK 的 `buckyos obj` 当前注册 `resolve/read`。下文的裸 `xcall` 是拟交付的同一执行器入口。[argv 解析](../src/frame/agent_tool_cli_dev/src/lib.rs#L738) · [baseline](../src/frame/lib_opendan/src/baseline.rs#L75) · [镜像](../build_aios#L681) · [人类 CLI](../../buckyos-websdk/cli/modules/obj.ts#L25) |
| 联系人与通知 | `contact.lookup` 能从本人的关系、本 Zone 和名字服务找候选。`owners/A/contacts/B` 是 A 保存的 B 的条目，集合支持查询及 `changed` 声明。系统通知已有 `notice` 和可回答的 `input`，非系统调用者只能通知自己的用户。[查人](../src/frame/msg_center/src/contact_lookup.rs#L1) · [通讯录对象](../src/frame/msg_center/src/obj_provider.rs#L200) · [通知类型](../src/kernel/buckyos-api/src/msg_notify.rs#L49) · [收件人校验](../src/frame/msg_center/src/notify.rs#L609) |

## 3. 身份、引用与绑定

**谁的资源，谁在操作。** 本人使用 CLI 时，`owner` 是经 Zone 验证的当前人类用户。Agent 使用时，从已验证的 Agent 或 Agent App 与其 owner 的绑定取得用户 ID 和真实 DID。两者定位同一个人，读写仍保留实际 caller、Agent/App 身份、权限模式及目标服务的 ACL。把根别名链接到一个现有个人对象时，核实该对象代表当前已验证的 owner，再执行面向本人的动作。无 Zone 会话、无法确认绑定、受限账号无权拥有工作区等情形返回相应错误。普通服务身份、设备账号、环境里的用户名、群成员姓名都不足以推断 owner。命令没有 `--as-owner` 或改收件人的捷径。在 `--dev` 仅有本地测试身份时显式标识该模式。[Agent 工具的会话](../src/frame/lib_opendan/src/session_context.rs#L411) · [工作区授权](../src/frame/aiworkspace/README.md#L1519)

同一句“我的日历”在不同人的会话里会指向不同对象。实现和 Memory 需要区分以下引用。

| 引用 | 例子 | 解析和持久化规则 |
| --- | --- | --- |
| 当前上下文的别名 | `owner`、`owner.cal`、`owner.todo` | 按完整名字和可信作用域查各自的绑定。个人资源脚本须确认 Zone、owner 和真实 caller，再定位个人数据。Alice 与她的 Agent 在默认配置下定位同一份日历。另一个运行时可以有不同别名配置。通用 skill 可以使用此拼写。 |
| 确定的人 | 小林的用户 DID，已发布时另有 `person_ref` | 通讯录按姓名返回候选，消歧并核实本 Zone 用户后即能取得 DID。个人对象发布且当前 caller 可见时再提供其对象 URL。通讯录条目表示本人保存的关系。 |
| 已发布的人与用途 | Alice 的日历或小林共享的忙闲入口（`resource_ref`） | URL 固定到人、来源 Zone 与用途，对方按当前用途绑定和权限响应。可写入可见 Hint 供直接 `xcall`。Zone 内的 `obj://` 一起保存来源 Zone，跨 Zone 分享用来源 Zone 正式发布的 HTTPS Object URL。本地脚本未发布此能力时省略该字段。 |
| 固定资源或记录 | 项目日历 URL、创建日程后得到的 `record_ref` | 资源不会随个人绑定移动。回执包含具体 Provider、资源和记录的稳定定位信息，另给 `open_url` 让人打开。变更旧记录时按原目标的当前权限处理。只获准看忙闲的调用者收不到底表或私人记录引用。 |

**两个层次的绑定。** 前置 TODO 管“这个别名由哪个扩展或对象处理”，按完整别名维护绑定修订（`alias_revision`）。本文的用途配置管“这个人的个人日历当前存在哪里”，按 `(zone, owner, slot)` 维护资源绑定修订。`owner.cal` 的默认本地脚本读取用途配置。将该用途从 AIWS 改为外部日历只增加资源绑定修订。将完整别名从本地脚本切到一个固定日历 URL 只改变别名绑定修订，此后按目标 Profile 操作该日历。链接到已发布的个人**用途对象**时，由该对象在服务侧读取其资源绑定。管理界面与回执应标明修订所属层次。

**向他人发布时。** 已有具备所需契约和访问控制的对象可直接用作分享入口。若 BuckyOS 提供统一的 Zone 内个人目录，建议在 Control Panel 的用户模块挂载 `obj://control-panel/people/<user-did>`，由个人对象的 Profile 声明资源属性，读取时给出 `calendar`、`availability` 等当前 caller 可见的用途引用。URL 由服务生成，路径段按 DID 编码。实施此 Provider 时登记 host、挂载 `/kapi/control-panel/obj` 并验证对象路由先于通用 kRPC POST。属性与 ACL 按现有用户类型处理。M1 的提醒与 AIWS 本地脚本可以独立交付。首版没有实施这个用户目录，忙闲入口由 AIWS 提供，即 `obj://aiworkspace/people/<user-did>/availability`（§8），只在本 Zone 内可用。因此 `who` 与 `owner.contacts` 目前不会返回 `person_ref`。owner 2026-11-20 定：本版本不做用户目录，也不做 Zone 外访问。以后对外开放个人用途时遵循 DID Document 的惯例，开通时在 DID Document 里登记一项 `type = DIDObjectService` 的 service，写明 `serviceEndpoint`、`profile` 和 `kind`，调用方从 service 找到入口。[DIDObjectService](../../buckyos-base/doc/did-object-protocol.md#L364)[URL 约定](../doc/key%20url.md#L48) · [URL 构造](../src/kernel/buckyos-api/src/object_url.rs#L401) · [Control Panel 路由](../src/frame/control_panel/src/main.rs#L1440) · [Card 与 Profile](../../buckyos-base/doc/did-object-protocol.md#L137)

**个人用途配置按用户持久化。** 建议新键 `users/<user_id>/resource_bindings/<slot>`，由用户服务提供带版本的窄接口。实现沿用用户级配置的权限约定和 system-config 的条件事务。一个用户所有 Agent 共用配置。可覆盖的首批 slot 是 `calendar`、`todo`。通讯录和系统通知先沿用各自的系统服务。绑定至少记录 schema 版本、`slot`、`mode`、适用的 Provider 引用和资源绑定修订号（本文业务回执中的 `binding_revision`）。凭据由已授权的适配器管理，接口只收可持久引用。[用户级独立配置先例](../src/kernel/buckyos-api/src/rbac_config.rs#L99) · [条件事务](../src/kernel/buckyos-api/src/system_config.rs#L740)

| 配置 / 结果 | 解析语义 |
| --- | --- |
| 从未设置，或显式使用 `default` | `calendar/todo` 使用本人原生 AIWS 日程。纯解析只给出处。首次有权限的写操作才初始化工作区、标准表与人可编辑的页面。尚未初始化的原生日历列表返回空并标记 `initialized=false`。 |
| `object` | 指向配置的、实现相应契约的 DID Object。已有外部产品可通过一个已授权的 Zone 适配器提供此对象。绑定时校验类型和支持的操作。拒绝把用途对象指回自身或组成循环。只读日历可以绑定，新增时明确返回不支持写入。普通网页链接只可用作 `open_url`。 |
| `disabled` | 本人主动停用，返回 `RESOURCE_DISABLED`。显式目标失联、授权过期、类型不符时分别报告。只有本人把配置切回 `default` 才恢复原生后端。 |

绑定管理只向经认证的本人管理会话或明确获得该管理权限的调用者开放。读日历、写日程和改变此后所有写入的目的地分别授权。沿用当前 Agent 权限模式及用户委托。写绑定带 `if_revision`，并发失配返回当前修订。重置也推进修订，做到 A → B → A 能被识别。绑定值只描述资源，禁止把可执行 shell、任意命令模板、token 或带凭据的分享链接作为调度规则。

## 4. 对象参数与动作契约

### 4.1 本用例如何使用通用别名

1. 个人资源包分别注册 `owner`、`owner.cal`、`owner.todo` 等完整名字。点号是名字的一部分。`owner.cal` 即使单独安装也能调用，解析时不读取 `owner` 的属性、不拼接 URL 路径。其余输入、显式文件路径和 URL 的处理统一由前置 TODO 规定。[前置解析与路由](opendan-object-local-extensions-todo.md)
2. 每个别名在当前作用域选择一个实现。本地实现通过安装清单把 Profile 声明的各个动作映射到包内脚本。链接实现保存完整已有 Object URL，沿原适配器读取 Card/Profile 并调用目标声明的动作。链接失联、动作缺失或授权拒绝时即按该错误返回，绝不改走另一份脚本。读对象也使用同一份绑定。
3. `owner` 可为本地脚本、`owner.cal` 可为链接，反过来同样成立。两者分别校验自己的 Profile。链接的根对象可以只提供 `notify`，或仅声明其他动作。使用 `owner.cal add` 不要求它还提供 `resolve`、`cal` 属性或发现接口。人类和 Agent 都通过前置任务交付的 `xcall` 或 `agent_tool xcall` 发出一条业务调用，服务按实际身份鉴权。[现有 URL 动作检查](../src/frame/agent-did-object-lib/src/adapters/did_object.rs#L201) · [动作字段](../../buckyos-base/doc/did-object-protocol.md#L1023)

### 4.2 首批对象

**首批对象交付后，必须退役由这些对象接替的 AIWS 命令行。** 本批的 `buckyos aiws cal` 与 `buckyos aiws todo` 分别由 `xcall owner.cal <动作>` 与 `xcall owner.todo <动作>` 接替。新入口覆盖原命令的全部操作并通过验收后，按 §4.3 迁移现有调用方，移除这两个子命令的注册和帮助条目。

每次调用只有一个对象和一个动作。动词就是相应 Profile 的 action 名。下面同一行列出的各动作分别声明参数、读写性质及返回值。脚本与外部 Provider 都按这份契约验收。对象能提供的属性和事件分别通过 DID Object 的 read 和订阅入口访问。

| 对象 | 动作 | 输入和结果 |
| --- | --- | --- |
| `owner` | `who`、`resolve` | 默认本地 Profile 提供这两个纯读动作。`who` 返回可信 `owner_did`，已发布且可见时另给 `person_ref`。`resolve slot=calendar` 给出本人的用途绑定状态、已知动作和资源绑定修订。存在可见的已发布用途对象时才返回 `resource_ref`。此结果描述个人用途配置。实际调用 `owner.cal` 时以该独立别名的绑定为准。 |
| `owner` | `notify`、`ask`、`notification_status` | 提醒映射到 `notify.send(kind=notice)`，请主人回答映射到 `kind=input`。传消息文本，没带幂等键时宿主按内容 hash 加时间片构造（前置 TODO §5），返回 `notify_id`、状态及已有的 `task_id`/会话链接。按 `notify_id` 取答复。发送成功表示系统已接收，保留 Agent 会话来源供人点击返回。[NotifyHost](../src/frame/lib_opendan/src/notify.rs#L57) · [通知类型](../../buckyos-websdk/src/msg_center_client.ts#L659) |
| `owner.cal`（用途 `calendar`） | `list`、`add`、`update`、`rm`、`skip`、`postpone` | 列表窗口 `[from,to)`，带 offset 的 RFC 3339 时刻，支持 `tz, limit, cursor`，返回 `events[], next_cursor`。新增传 `title` 和 `start/end` 或全天 `start_date/end_date`（结束日含当天）。单条变更传 `record_ref`，更新另传 `patch`。重复事件保留系列、原发生时刻及 `scope=one/following/all`。返回可打开的记录回执。 |
| `owner.todo` | `list`、`add`、`done`、`undone`、`update`、`move`、`rm` | 列表传 `include_done, limit, cursor` 等，返回 `items[], next_cursor`。新增传 `title, due?`，日期型截止日按用户日历解释。单项操作传 `record_ref`。保留重开、排序和显式批量清理语义，返回受影响记录或数量。 |
| `owner.contacts` | `find`、`get`、`update` | 按姓名查询可返回多个候选。get/update 传已消歧的 DID，允许编辑的字段经系统 `contact.*` 写入。返回 `contact_ref`（通讯录关系）及已核实的用户 DID。对方个人对象已发布且可见时另给 `person_ref`。工作区选择联系人时复用该来源。[联系人字段与写入规则](../doc/message_hub/Contact%20Mgr.md#L123) |
| `owner.bindings` | `get`、`set`、`reset` | 传 `slot`，写操作带 `if_revision`。set 另给 `mode` 及对应的 Provider 引用。缺记录的初始版本约定为 0。成功返回新资源绑定修订和可见的已发布用途引用（如有）。本动作只修改个人用途配置。绑定详情只对具备管理权限的 caller 可见。 |

以下是拟交付接口。主人时区已知时可用它解释本地日历日期。涉及“周五”且没有可信时区时，先明确所指日期和时区。

```sh
xcall owner.cal add --params '{"title":"设计评审","start":"2026-11-20T14:00:00+08:00","end":"2026-11-20T14:30:00+08:00"}' --idempotency-key review-1120
xcall owner.cal list --params '{"from":"2026-11-20T00:00:00+08:00","to":"2026-11-21T00:00:00+08:00","tz":"Asia/Shanghai"}'
xcall owner.todo add --params '{"title":"把评审稿发给小林","due":"2026-11-19"}'
xcall owner.contacts find --params '{"name":"小林"}'
xcall owner notify --params '{"text":"评审稿已备好"}'
xcall owner ask --params '{"text":"周五评审是否照常？"}'
```

### 4.3 日历脚本在哪里执行

在前置任务的本地执行器中运行已安装的 owner 扩展。默认 `owner` 提供 `notify`，脚本拿到经验证的 caller 和 owner，复用消息服务投递及回执。`owner.cal`、`owner.todo` 各自提供本地 Profile 和动作映射。日程脚本通过注入的 `ScheduleTransport`，在原 caller 的权限下组合 AIWS 基础 API。构建时把清单、Profile、入口和所需 WebSDK 随扩展打包，验证实际分发目录内的文件。若把 `owner.cal` 改为 link，该别名的直接调用交给目标自己的处理器。本地安装路径和凭据始终由执行宿主掌握。[现有脚本与 transport](../../buckyos-websdk/cli/modules/aiws_schedule.ts#L20) · [SDK 日程实现](../../buckyos-websdk/src/aiws_schedule.ts#L791) · [AIWS 服务原语](../src/frame/aiworkspace/server/src/methods.rs#L97)

| 默认 AIWS 动作 | 脚本调用 |
| --- | --- |
| `owner.cal list` | 只读定位主人的具名 `schedule`（M0），未初始化返回空列表并标记 `initialized=false`。已有标准表则用 `doc.query` 分页读取记录，复用 `calendarOccurrences` 展开、处理例外并裁剪到窗口。[行查询](../../buckyos-websdk/src/aiws_schedule.ts#L851) · [发生项展开](../../buckyos-websdk/src/aiws_schedule.ts#L465) |
| `owner.cal add` | 首次有权写入时 `ws.ensure` 找到/创建工作区，`doc.read` 核对表和页面，缺失时按模板初始化。`eventAddOps` 生成 `table.insert_records`，通过 `doc.commit` 写入日程。[初始化](../../buckyos-websdk/src/aiws_schedule.ts#L799) · [新增事件](../../buckyos-websdk/src/aiws_schedule.ts#L529) |
| `owner.cal update`、`owner.todo done` | 本人的用途脚本从 `record_ref` 定位原 Provider、工作区、表和记录 ID，检查身份、原目标 ACL、版本及原发生项后提交。若当前别名直接链接到另一份固定日历，编辑旧记录时根据旧回执调用原日历或记录 URL 上已声明的动作。只支持本地处理的记录由回执标识的原适配器接续，已卸载或撤权时返回原目标不可用。避免把旧 ID 当成新 Provider 的记录。[重复项修改](../../buckyos-websdk/src/aiws_schedule.ts#L601) · [待办操作](../../buckyos-websdk/src/aiws_schedule.ts#L308) |

把现有 `buckyos aiws cal/todo` 的日程规则从 CLI 参数解析中提取到共享 TS 源码，复用桌面同一套数据模型。新路径通过后同步迁移帮助、Jarvis skill、控件自述模板与生成契约、实际 argv 调用方及 Zone DV，再从 `createAiwsModule()` 移除旧命令注册。原 `--id` 调用方用创建/查询回执提供的 `record_ref`。项目工作区的 `buckyos aiws place calendar` 继续用于放置控件，已有标准表、用户记录和桌面链接保持可用。AIWS 提供 `ws.*`、`doc.*` 等通用 API，领域动作由上面的脚本组合。[当前命令表](../../buckyos-websdk/cli/modules/aiws.ts#L1784) · [日程参数](../../buckyos-websdk/cli/modules/aiws_schedule.ts#L64) · [控件源](../src/frame/aiworkspace/controls/schedule.ts) · [控件文档](../doc/workspace/kernel/05-Block控件与标准库.md#L227) · [桌面调用方](../src/frame/desktop/tests/dv/aiworkspace.controls.spec.ts#L187)

### 4.4 回执、失败与重试

保留 AgentTool 输出格式与退出码。本地和链接实现的成功业务结果放在 `detail.result`，失败原因放在 `detail.error`。示例是尚未发布个人用途对象的 Alice 在本地扩展中创建日程的结果。实际回执填入可解析的具体记录定位信息。用途对象发布并可见时可以增列 `resource_ref`。其中 `binding_revision` 来自个人用途配置，固定日历链接没有使用这份配置时省略此项。[动作回执转换](../src/frame/agent-did-object-lib/src/adapters/did_object.rs#L290)

```json
{
  "owner_did": "did:bns:alice",
  "binding_revision": 0,
  "record_ref": "<此 Provider 中这条日程的稳定引用>",
  "open_url": "<本人可打开的记录链接>",
  "status": "created",
  "idempotency_key": "review-1120"
}
```

**先认领写入，再派发。** 个人用途实现以 `(zone, caller, subject_did, slot, action, idempotency_key)` 认领操作，在首次派发前持久化规范化输入摘要、已选扩展或链接目标及其别名修订、相关的资源绑定修订、实际存储目标与记录 ID。并发只认领一次，同键不同输入报冲突。重启或改绑后沿原认领查回执、按原目标安全续办或报告结果未知。链接到同一已发布用途对象时，直接 URL 调用和通过别名调用要把相同业务键交给同一对象。独立的固定日历有自己的幂等范围。通用层不记录派发（前置 TODO §5）：别名在首次调用与重试之间改绑时，重试按新绑定派发，这是已知风险。别名未改绑时，领域层按原认领和原资源绑定继续。若原目标无法确认结果，返回 `WRITE_OUTCOME_UNKNOWN`，新的资源目标收到零次派发。重放回执仍检查当前权限。

`agent_tool xcall` 解析 `--idempotency-key`。从 buckyos-base `e1ddc24` 起，`DIDObjectClient::invoke_request_from_resolved` 可以传入 `idempotency_key/confirm_token/trace_id`。OpenDAN 的 DID Object 适配器仍自己构造带这三项的请求，再按 Profile 的 form 发送，因为它要按每个 form 的来源和目标选择客户端：Zone 凭据只发给本 Zone 的地址，外部 Profile 不能把调用转到调用者的 Zone，目标不是 HTTPS 时只允许同源或本 Zone 地址。base 的客户端只有一个 HTTP 客户端，做不到这样的区分。[适配器构造请求](../src/frame/agent-did-object-lib/src/adapters/did_object.rs#L176) 领域脚本把原业务键交给 `doc.commit` 并复用记录 ID。模板初始化和业务写入各有稳定的提交键。现有 `ScheduleClient.commitTo` 每次独立提交都会生成新键。业务层和 Provider 须支持以原键取回结果，可用 `doc.get_submission`。Agent 若要安全重试，应在首次发送前保存该键。Provider 无法确认提交时返回 `WRITE_OUTCOME_UNKNOWN`、原键及可见的查询线索，结束自动重试。[CLI 参数](../src/frame/agent_tool_cli_dev/src/lib.rs#L779) · [动作请求](../../buckyos-base/src/name-client/src/did_obj_client.rs#L177) · [协议字段](../../buckyos-base/doc/did-object-protocol.md#L1015) · [SDK 提交](../../buckyos-websdk/src/aiws_schedule.ts#L830) · [AIWS 提交查询](../src/frame/aiworkspace/README.md#L1832)

**让 Agent 能从错误决定下一步。** 保留服务的机器可读错误，补齐 CLI 解析和传输错误在 `detail` 中的原因。区分未知逻辑名、身份缺失、权限拒绝、重名、用途未配置或停用、动作不支持、别名绑定或个人用途配置的版本冲突、本地扩展不可用、Provider 失联和写入结果未知。重名只给可见候选，失联标明能否重试，明确拒绝即终止。只有实际查询成功且无记录才返回空列表。日志保留 trace ID，避免记录凭据和私人日程内容。[CLI 错误映射](../src/frame/agent_tool_cli_dev/src/lib.rs#L2551)

## 5. 联系人直达与后续场景

**Memory 写什么。** 主人在私聊中说“小林以后让我从这里约时间”，把小林 DID、用途和**对该调用者有效的稳定对象 URL**关联起来，并保留来源 Zone、可见范围和观察时间。`owner.cal` 会在另一人的上下文中重新解释，写入 `Hint.object_url` 的应是已经固定到小林的用途对象 URL。沿用该会话的 Memory 可见范围，小林是被描述的人。`scope.subjects` 会影响谁能召回。需要生成可读 Hint 时将 URL 放进对象字段。若采用结构化 relation，先建立两端 MemoryObject。给定有效 Hint 和已知动作，直接调用。绑定变化由用途对象处理。若具体共享被撤回，按原权限处理或回到本人公开的入口重查。用户指名的是固定项目日历时，则保存具体资源引用。[Hint 取 URL](../src/frame/lib_opendan/src/memory/hint.rs#L128) · [会话 grants](../src/frame/lib_opendan/src/memory/mod.rs#L249) · [关系写入](../src/frame/agent_tool/src/agent_memory/state.rs#L1295)

**小林决定向谁分享忙闲。** 建议由小林对自己的共享配置执行 `xcall owner.sharing grant`，参数是 `slot=availability` 和选定的 `to=<本 Zone 用户 DID>`。`revoke` 用相同目标撤权。上线此能力时才登记该逻辑名及动作。初始授权列表为空。在个人对象的用途入口及领域动作两处落实同一授权，使直接知道 URL 的 caller 也经过检查。忙闲 Provider 在服务端依据该条授权生成窄投影，只将 `availability_ref` 给获准的 caller。授权按小林分享给的主体检查。Alice 的 Agent 访问时还须核实它代表 Alice 的有效授权范围。

常用联系人的第二次查询只需一次操作。下面的变量取自可见 Hint 或服务端解析结果。

```sh
xcall "$availability_ref" free_busy --params '{"from":"2026-11-20T13:00:00+08:00","to":"2026-11-20T17:00:00+08:00"}'
```

动作返回 `intervals[], coverage`，只包含请求窗口内获准的忙碌区间和覆盖情况。查到零个忙碌区间且完整覆盖时，才可以回答“这段时间有空”。要约会，单独执行对方开放的 `invite`，回执区分已发、已接受和拒绝。忙闲投影在服务端完成。接受邀请后是否加入日程由接收方决定。[日程共享设计](opendan-d4-timer-reminder-todo.md#L129)

| 能力 | 后续实现边界 |
| --- | --- |
| 位置 | 启用位置源后，可通过对象 `read` 读取 `owner` 的 `location` 属性，返回位置或区域、`observed_at`、精度/来源及新鲜度。无定位源返回未配置，过期观测标记陈旧。用户资料中的文字地点只用于资料展示。源有观测事件且授权允许时再开放订阅，仅向获准会话发出变化提示。[用户资料](../../buckyos-base/src/name-lib/src/user_profile.rs#L404) · [Hints 场景](../doc/opendan/Agent元能力/Agent%20浮现Hints.md#L1517) |
| 通话 | `xcall owner call` 指发起实时通话，仅在真人配置了能返回通话状态的 Provider 时开放。“请主人确认”走 `xcall owner ask`。区分已发起、已接通与结束。要联系其他用户，走接收方开放的消息、邀请或通话渠道。 |
| 团队任务 | owner 2026-11-20 定：按 AIWS 惯例，项目待办的负责人绑定工作区里的「联系人」表，从这张表里选人。联系人表由本 Zone 联系人数据源生成，主键是联系人 `did`，在[联系人数据源 TODO](aiws-zone-contacts-source-todo.md) 中规划。团队分派在它之后实施，待办如何引用联系人行随之设计。todo 当前仅校验 `ext.assignee` 是字符串。被分派者对项目的读取权限另行核实。个人“分配给我”视图按权限引用原记录。若产品需要“接受后加入私人待办”，单独设计请求/接受流程。[assignee 字段](../src/frame/aiworkspace/schemas/standard-tables.md#L25) |
| 事件 | 对象读取与订阅接入逻辑名时复用 §4.1 的 resolver，各按 Profile 检查属性或事件。订阅回执固定订阅时的目标、订阅 ID 和 `alias_revision`，取消及续租均找到原订阅。改写别名后，已存在的订阅继续关联原目标。若原目标是稳定的个人用途 Provider，它可在资源改绑时切换内部来源并发布 `binding_changed`，用资源绑定修订标识变化。断线重连按当前权限重读。撤权即停止投递与补发。绑定变更、日程变化、答复到达等事件在投递验证后逐项发布。[对象事件桥](../src/frame/agent-did-object-lib/src/event_bridge.rs) |

## 6. CodeAgent 的工作顺序

勾选表示首版实现已落地。各环境的验收结果见 §8，收尾任务与待定事项见 §9。

- [x] **M0 · 修正私人日程归属。** 具名创建、ensure、重建索引都使用经验证的 `(owner_principal, name)`，沿用同一并发互斥。命中已有空间也校验真实 caller 对它的访问。为纯查询添加只读的按名查找，缺省时不建空空间。重建索引时识别已有重复项，只让冲突键报错。选定前仍可用原权限和 ID 读写各副本，使用含糊名字的写操作返回冲突。提供本人可执行的选主处理，保留原 ID、内容、历史、ACL 与旧链接，显式解除其他副本的冲突命名。处理中断可重做，禁止按目录遍历顺序选一本。复用名字测试夹具和现有日程页。[服务索引](../src/frame/aiworkspace/store/src/service.rs#L318) · [命名测试](../src/frame/aiworkspace/store/tests/names.rs)
- [x] **M1 · 本地提醒与原生日历。** 在通用扩展机制可用后交付两个独立的本地别名。先在没有个人对象 Provider 的 Zone 用 `xcall owner notify` 生成一张通知卡，用 `xcall owner.cal add` 将一条记录写入本人的 AIWS 日历。验证同键重试、桌面可见性及无可发布用途引用时的回执。再迁移 §4.2 的日历、待办、系统通讯录、`who/resolve` 和询问。分别用人类和 Agent 会话核实实际服务身份、权限和数据归属。
- [x] **M2 · 两种实现与资源换绑。** 接入 `owner.bindings` 的资源修订检查、外部日历和跨进程幂等。用本地用途脚本及第二个测试 Provider 验证同一动作契约，分别覆盖链接固定日历和链接个人用途对象。单独切换 `owner` 和 `owner.cal`。核对新写、旧回执、原键重试的实际目的地，别名改绑后的原键重试只记录、不判失败。外部 HTTPS Provider 按其声明的 endpoint/form 和受众调用，Zone 身份凭据只发给适用的 Zone 服务。记录测试 Provider 的能力和范围。[Rust 对象适配](../src/frame/agent-did-object-lib/src/adapters/did_object.rs) · [Zone 凭据范围](../src/frame/agent-did-object-lib/src/zone.rs#L69)
- [x] **M3 · 让 Jarvis 实际选用它。** 新增 `src/apps/jarvis_agent/agent/cmd_skills/owner/SKILL.md`，声明已安装的 `commands: xcall` 和 `scope: ui, work`，在 `ui_core` 留短入口。让 UI/Work 通过现有机制发现手册。写明何时用逻辑名、何时复用 Hint URL 或记录回执。参数以各对象的 Profile 为准。迁移 `aiworkspace_template_dev`、控件说明、帮助和 DV 中操作个人日程的示例。分别在打包的 Agent 环境和人类登录终端执行示例。[Jarvis 配置](../src/apps/jarvis_agent/agent/agent.toml#L49) · [AIWS 手册](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_template_dev/SKILL.md#L40)
- [ ] **M4 · 联系人协作及其他能力。** 本 Zone 忙闲直达已实现，其余能力按 §5 后续分项交付，次序见 §9。本版本只在本 Zone 内发布用途对象，复用已有 Provider。Zone 外访问按 §3 留到以后。首先交付本 Zone 的忙闲授权、按名字消歧、保存可见 Hint 和一次直调。再按 §5 分项交付邀请、团队分派、位置、实时通话和订阅。复用日程与提醒已有的分工。[提醒实施边界](opendan-d4-timer-reminder-todo.md#L19)

## 7. 验收用例

| 编号 | 操作 | 可判定的结果 |
| --- | --- | --- |
| A1 归属 | Alice 桌面与其两个独立 Agent 身份同时首次添加，一次让 Agent 先建，一次让人先建。重启后再列举。另测 Bob 本人。 | Alice 只有一个具名工作区，三条记录同屏可见，行为保留各自身份。Bob 得到另一份。仅用同一个用户 token 起三个客户端不算通过。 |
| A2 旧数据与只读 | 预置同 owner 同名的两本有数据的日历，重启、选主、重启。另测空账号、只读 Agent 及工作区显式拒绝某个 Agent。 | 冲突可诊断，选主前按名写入被拒，选主后两本原记录按 ID 仍可访问。只读查询不初始化。身份或权限拒绝不伪装成空表，且不泄露候选资源详情。 |
| A3 效率与人可操作 | 在仅安装本地扩展的环境中，主人让新会话创建一条评审日程。人从返回链接改期，Agent 再查询。以 CLI JSON、服务调用记录和 Agent 工具轨迹核对。 | 已知 owner 的常用操作只需一条业务命令，不列全 Zone 工作区、不扫描通讯录、不需模型先 `read` 探索。桌面和 CLI 指向同一记录，跨时区和全天事件日期正确。缺时区时用例覆盖同一绝对时刻在主人和容器分别属于两天的情形。 |
| A4 换源 | 保持本地用途别名不动，将个人资源绑定从 AIWS A 改为 Provider B，A 保持在线可写。再新增。用旧记录回执改期。把 B 设为只读而 A 仍可写，再持原回执改期。另测 B 失联、撤权和过期的并发配置写。 | B 可写时新增只到 B。B 变为只读后原记录仍按 A 当前 ACL 改期，B 没有收到该调用。B 的失败有明确原因，两边均无意外新增。并发配置写只能有一个成功。只增加该用户 calendar 的资源绑定修订。切换后原数据仍留在 A。 |
| A5 重试 | 注入一次“原 Provider 已提交但回包丢失”，随后重启派发进程、切换资源绑定，再以原键重试。再测链接到同一已发布用途对象时改用其完整 URL 重试。别名改绑后的原键重试属已知风险，不在此判定。另测相同键不同输入、撤权后读取回执及外部 Provider 无法确认提交。 | 原请求取回原回执或明确给出结果未知，双方至多出现一次写入。不同输入报冲突，撤权后不披露旧结果。通知重试只生成一张通知卡。 |
| A6 人与权限 | 两个小林同名，选定其一。只有他向 Alice 分享了忙闲。下次用可见 Hint 直调，随后撤销共享。从另一人的会话再试。 | 首次消歧不产生写入。已知目标一次 `xcall` 取得忙闲，无标题或底表 URL。撤权生效。另一个会话既不能据此召回 Alice 的私有 Hint，也不能凭一个 URL 越权。 |
| A7 通讯与团队 | 修改本人通讯录备注。让主人确认一个问题。尝试改收件人发通知。在团队项目分派一项任务，再撤销该成员的读取权。 | 备注由 ContactMgr 返回，询问得到通知/任务引用，答复前保持待处理。越权通知被拒。分派完成后只有一条项目记录，撤权后个人视图不再展示其内容。 |
| A8 可选能力 | 未接位置源、位置已过期、无实时通话 Provider、Provider 缺某项 action、有事件声明但实际断线。 | 未配置、陈旧与不可用分别可辨。不会报出推测位置或“已接通”。重连按最新授权重读。未实现能力不出现在可用动作中。 |
| A9 默认脚本与旧 CLI 退役 | 关闭个人对象 Provider，安装本地 `owner`、`owner.cal` 和 `owner.todo`，运行通知和日程 DV，记录 `list/add/update` 的 AIWS 调用。检查裸入口帮助、个人日历 Profile、工作区命令表和打包手册。退役后在发行版中执行 `buckyos aiws cal list` 和 `buckyos aiws todo list`。 | 提醒只发一张卡。人类和 Agent 的日历脚本都能运行，回执有真实 `record_ref`，没有发布用途 URL 时省略 `resource_ref`。原生路径只调用工作区定位、查询、读取及提交等基础 API，桌面能编辑同一记录。旧日历和待办命令的全部操作均可通过对应的 `xcall` 动作完成。AIWS 命令表和帮助已移除这两个子命令，调用旧入口均返回未知命令。 |
| A10 两种实现 | 同一组 `xcall owner notify` 和 `xcall owner.cal add` 用例先设为“本地提醒 + 链接日历”，再设为“链接个人对象 + 本地 AIWS 日历”。分别改动、移除 `owner` 和 `owner.cal`。另测链接端失联、缺少动作与本地脚本不可用。 | 每次只有选定的处理器执行。只改根别名时日历行为及其别名修订保持原值，移除根后日历仍可调用。反向同理。链接实现及直接访问同一 URL 的结果和权限一致。任一失败都返回明确原因，没有自动尝试另一个实现或二次写入。 |
| A11 作用域与发布 | Alice、Alice 的 Agent、Bob 分别运行本地 `xcall owner who` 和 `xcall owner.cal list`，再用无可信身份的会话尝试。发布忙闲用途对象后，让获准的 Bob 保存并调用 Alice 的 URL，再尝试未获授权的动作。 | Alice 和她的 Agent 定位到相同的 owner DID，调用者 DID 和权限仍可区分。Bob 读到自己的日历，无有效身份时报错。纯本地阶段无公开用途引用，发布后 Hint 的 URL 始终指向 Alice 的获准用途。 |

从 BuckyOS 的 `src/` 运行 `cargo test -p agent_tool_cli_dev -p agent-did-object-lib`、`cargo test -p aiworkspace-store --test names`、`cargo test -p aiworkspace --test objects`。利用通用别名与路由测试夹具覆盖 A10，A1/A2/A11 用真实的多身份路径验证。在 buckyos-websdk 运行 `pnpm run check:cli`、`pnpm run test:cli`，以相同契约验证原生脚本和第二个 Provider。

最后沿用 Desktop 的 Zone DV，检查人类、Agent App 及点击链接后的行为。为每阶段记录实测 caller、Provider、命令、具体资源 ID 及回执，把独立服务、测试 Provider 和真实 Zone 的结果分别写清。[独立服务测试](../src/frame/aiworkspace/server/tests/objects.rs) · [日程 Zone DV](../src/frame/desktop/tests/dv/aiworkspace.controls.spec.ts) · [CLI 验证入口](../../buckyos-websdk/cli/README.md)

## 8. 首版实现与验收记录

首版提交为 BuckyOS `890e925de`、buckyos-websdk `f7e4d7d` 与 `8140857`，2026-11-20 已部署到本机 Zone。默认包安装的六个独立别名为 `owner`、`owner.cal`、`owner.todo`、`owner.contacts`、`owner.bindings`、`owner.sharing`。这些别名随 SDK 分发，执行器先读 `$BUCKYOS_ROOT/libexec/buckyos-tool/dist/owner-extensions`，再叠加 ext-tools 的 `tools/current.json` 快照。前置 TODO §4 规定首版只从已安装的扩展工具包登记，这里与它不同，owner 已接受（§9）。[别名来源](../src/frame/agent-did-object-lib/src/extensions.rs#L71) 每个会话的 `owner.context` 由 Control Panel 验证。改变个人绑定与授予忙闲访问需要本人管理会话。Jarvis 的 UI 和 Work 会话共用同一份 owner 手册。[默认扩展](../../buckyos-websdk/owner-extensions/object_extensions.toml) · [Jarvis 使用手册](../src/apps/jarvis_agent/agent/cmd_skills/owner/SKILL.md)

个人资源 Provider 在 Profile 中声明 `x-buckyos:slot`（`calendar` 或 `todo`），或相应的 `buckyos.calendar`、`buckyos.todo` trait，并将 `list` 声明为只读（`read` 或 `pure`）。派发时重新核实当前 Profile 的类型、动作、effect 和 form。转发到另一项个人用途的对象还要声明 `x-buckyos:resource-binding`，包含 `zone_did`、`owner_did`、`slot`。同 Zone 的环检测、绑定修订及该修订的来源历史在同一事务中提交。外部记录回执保留 Provider 的原始 `provider_record_ref` 和面向人的 `open_url`。后续编辑传入完整 `record_ref`，即使两个 Provider 使用同一个原始 URL 也能定位到创建时的来源。[控制面绑定](../src/frame/control_panel/src/owner.rs) · [原目标回归](../src/frame/control_panel/src/owner_access_tests.rs)

外部写入回包丢失时，运行时在原 Provider 的当前 Profile 上查找只读 `get_submission`，以原调用者身份传入 `action` 和 `idempotency_key`。成功确认的响应在 `result` 中携带 `{"status":"accepted","result":<原业务回执>}`。查无记录、未声明该动作或无法确认，返回 `WRITE_OUTCOME_UNKNOWN` 和原键。撤权后返回权限错误。原生 AIWS 使用自己的 `doc.get_submission`，查询记录操作的回执时复核表权限。[Provider 派发](../src/frame/agent-did-object-lib/src/adapters/did_object.rs) · [AIWS 回执](../src/frame/aiworkspace/store/src/workspace.rs)

| 环境与覆盖 | 实测结果 |
| --- | --- |
| 独立 AIWS 服务，A1、A2、A3、A9、A11 | 四个不同身份运行默认脚本。Alice、`did:bns:agent1`、`did:bns:agent2` 并发新增到 `ws_2611spda`，三条记录为 `r3`、`r4`、`r5`。Bob 首次只读查询得到 `initialized=false`，首次写入得到 `ws_26114a17`。注入回包丢失后按原键 `lost` 重取待办 `r68`。具名索引的两种首建顺序、重启、旧副本选主和 ACL 用服务及存储测试核对。[服务测试](../src/frame/aiworkspace/server/tests/home.rs) · [存储测试](../src/frame/aiworkspace/store/tests/names.rs) |
| Rust CLI、打包的 Deno 脚本、两个 HTTP 日历 Provider，A4、A5、A10 | 控制面使用夹具。以 `add-b` 新增，改绑后同键仅向 B 查询回执。以 `loss-b` 注入提交后断线，B 只写一次。B、C 的列表均返回同一个含 fragment 的记录 URL，使用 C 的回执更新时，仅 C 收到原始 URL。固定 URL 与逻辑别名传递相同业务键。控制面的持久键、条件更新、拒绝跨用户引用及互相改绑成环有独立回归。[双 Provider 联调](../test/object_url_dv/owner_provider_dv.py) · [通用路由测试](../src/frame/agent-did-object-lib/tests/owner_routing.rs) |
| 独立 MessageCenter HTTP、SQLite 和已签名会话，A5、A7 | Agent 的同键通知只生成 `nt-b5dfbe8e9c7c4385a759c17aeb4dfc35` 一张卡，改正文返回 `IDEMPOTENCY_CONFLICT`。询问 `nt-5ced41f687ad41579400f87362dc04ba` 关联测试 TaskManager 的 `t-1`，本人答复后状态从 Open 变为 Answered。其他用户读取与改变通知收件人均被拒绝。已绑定的 Agent DID 和 Agent App 能查询、更新本人允许编辑的联系人。只读、跨人及系统通讯录写入均被拒绝。[通知测试](../src/frame/msg_center/src/test_notify.rs) · [联系人测试](../src/frame/msg_center/src/test_contact_network.rs) |
| 独立 AIWS 服务与发行版投影脚本，A6、A11 | Alice 的 `ws_2611s8qj` 中有私有记录 `r-review`。Bob 和 Bob 的 Agent 直接调用 `obj://aiworkspace/people/did%3Abns%3Aalice/availability`，响应只有裁剪至 06:30–07:00 UTC 的区间和 `coverage=complete`。未共享者和撤权后的 Agent 均被拒绝。另一会话持有同一 URL 也不能召回私聊 Hint。外部日历如未提供本服务可用的忙闲投影，则返回来源不可用。当前用途停用则返回 `RESOURCE_DISABLED`。[共享对象测试](../src/frame/aiworkspace/server/tests/objects.rs) · [Memory 测试](../src/frame/lib_opendan/tests/memory_recall.rs) |
| 发行包首次验证，A3、A9 | 在提取的 SDK 发行包与打包的 Deno 中执行裸 `xcall owner who` 和 `xcall owner notify`，SDK 在临时安装根使用 Alice 的 Ed25519 密钥执行 `login_by_jwt`，登录服务夹具核验签名后签发会话。通过 `who` 核对调用身份，覆盖收件人被篡改时返回错误。发行版执行旧 `buckyos aiws cal list`、`buckyos aiws todo list` 均返回 `UNKNOWN_COMMAND`，退出码为 2。首次尝试真实桌面时停在登录夹具。后续实测见下表 C2–C4。 |

首轮验证通过了 180 项 WebSDK CLI 测试、42 项日程与重复规则测试、60 项 Rust CLI 与对象路由测试。服务和路由测试可沿用 §7 的命令。控制面使用 `cargo test -p control_panel owner::`。将 `BUCKYOS_ROOT` 指向含 Deno 及 SDK `dist/owner-extensions` 的安装根，可另运行 `cargo test -p aiworkspace --test objects installed_availability -- --ignored`，用真实私有日历行验证发行版忙闲投影。

### 收尾复验（2026-11-20，洛杉矶时间）

| 项目和环境 | 调用与回执 |
| --- | --- |
| C2、C3，真实 Zone 的 Desktop、Jarvis 容器、原生 AIWS | 人类终端的 `xcall owner who` 返回 caller/owner `did:bns:devtest`；`buckyos-app-jarvis` 的工具会话 `goal-20261121-12` 返回相同 owner，caller 为 `did:web:jarvis.test.buckyos.io`。同一具名日程 `ws_2611oish` 收到 `owner.todo add` 的 `d2b29c909cfa49eab0c3a31040c2c37d` 和 `owner.cal add` 的 `0c7009807347449db75a9b355d792ec1`，两者状态均为 `created`。`doc.get_changes` 中提交 `cm_5m3dnavzcaf32e7hmu24xnqc4h`、`cm_5i4zsykuhpkvmng4e753ocdlna` 的作者均为该 Agent DID。Chromium 打开日程的 `open_url` 并保存改期，Jarvis 在另一工具会话 `goal-20261121-13` 用 `owner.cal list` 按原 `record_ref` 查到 11 月 21 日 00:00–00:30 UTC。用例结束后只清理本次记录。[桌面 DV](../src/frame/desktop/tests/dv/aiworkspace.controls.spec.ts) |
| C4，真实 Zone 的 MessageHub、Jarvis 与模型 `gpt-6.1-sol:reasoning-medium@codex-proxy` | 首次实测发现 Jarvis 会先读手册，模型执行裸 `xcall` 时也只收到成功摘要。默认对话手册补上日程参数，适配器向模型输出业务 JSON。在最终镜像和手册下，新会话 `ui-20261121-14` 先询问日期和时区，得到“洛杉矶时间、11 月 27 日”后仅执行 `xcall owner.cal add`（键 `ui-20261121-14-design-review-20261127-1400`）。回执为 `created`、`ws_2611oish/4875a4abe24541a4847e923293064001`，回复附原 `open_url`。追问时仅执行 `xcall owner.cal list`，以 `America/Los_Angeles` 当地午夜为窗口边界，返回同一记录的 22:00–22:30 UTC。会话 worklog 中共有这两次 shell 调用；验收后已按回执删除测试记录。[默认对话手册](../src/apps/jarvis_agent/agent/cmd_skills/ui_core/SKILL.md) · [适配器回归](../src/frame/agent-did-object-lib/tests/provider_read.rs) |
| C5，真实 Zone，SDK 0.7.198 | 人类终端的 caller 为 `did:bns:devtest`。Agent 的 `work-20261121-4` 在节点 `did:web:ood1.test.buckyos.io` 经 Node Executor 以 uid 1000 执行；前台与后台任务 `local:shell:call_ZnwMkD0k5xbXXzhchRoNuWIj` 的 `xcall owner who` 均回传 caller `did:web:ownerdv.test.buckyos.io`、owner `did:bns:devtest`。三条路径的 owner.cal 别名修订均为 `3d20f9170b241ba03bfad8be55411beedfde558fa2a41fd679ae3e4df2c63445`。`xcall owner.cal list` 查询历史窗口返回 `initialized=true, events=[], binding_revision=0`。用户覆盖别名 `ownerdv_c5` 的修订为 1；空工具根下调用它返回 `EXTENSION_UNAVAILABLE`。后台操作 `op-18e9fe8a206c21f0111` 完成后回到该会话。 |
| C5，真实 Loader 的 `read` | 旧 Loader 把别名当作文件。修正别名转发后，Loader 从已认证的 AppService runtime 取得网关和凭据。镜像 `0e0d5c36221b` 中，专用 Agent 的 `ui-20261121-9` 分别读取 `owner` 和用户覆盖 `ownerdv_c5`，均读到 Profile `urn:opendan:personal-tools:person:v1` 和 `actions.who`。复验后已恢复测试别名、撤回 host 授权并移除专用 Agent。[Loader 回归](../src/frame/opendan/tests/owner_alias_read.rs) |
| C7，独立 MessageCenter HTTP、SQLite、打包的 owner 扩展与 Control Panel 夹具 | 签名 caller `jarvis.test.buckyos.io@alice` 三次执行 `xcall owner notify 'text=Review ready'`。冻结 CLI 时钟到 00:59:59、01:59:59、02:00:00 UTC，实际卡数依次为 1、1、2。前两次均返回 `nt-c10b2b9d405641ab884c9a64dd97d3be`，第三次返回 `nt-0d2b04475c6a448a827e454113c0d1d0`。默认单测也覆盖该边界。[通知键](../src/frame/agent-did-object-lib/src/owner_host.rs) · [卡片计数测试](../src/frame/msg_center/src/test_notify.rs) |

最终镜像 `local/aios-test:latest-amd64` 的 ID 前缀为 `0e0d5c36221b`。该镜像与更新的宿主 CLI 通过上述桌面用例（24.3 秒）。WebSDK 的 `check:cli` 和 180 项 CLI 测试通过。按仓库约定串行运行 `cargo test -p opendan -p agent-did-object-lib -p agent_tool_cli_dev -- --test-threads=1`，196 项通过，20 项标记为忽略。另跑了 11 项 MessageCenter 通知用例（含冻结时钟的集成测试）、AIWS 的 6 项具名工作区测试和 4 项对象测试（含打包的忙闲投影）。

## 9. 收尾 TODO

M0–M3 与本 Zone 忙闲直达的代码已提交。首版收尾的实测 caller、命令、资源 ID 和回执见 §8。

- [x] **C1 · buckyos-base 的调用元数据。** owner 已推送（buckyos-base `e1ddc24`），`DIDObjectClient::invoke_request_from_resolved` 可以传入三项调用元数据。OpenDAN 适配器不改用它，原因见 §4.4。BuckyOS 没有代码调用这个接口，不需要为它单独执行 `cargo update`。
- [x] **C2 · 在真实 Zone 跑桌面 DV。** 已从日程回执打开页面、改期，并用 `owner.cal list` 复核同一记录。开发 Zone 在 `src/frame/desktop` 下复跑（登录和运行环境见[日程 DV](../src/frame/desktop/tests/dv/aiworkspace.controls.spec.ts)）：

  ```sh
  BUCKYOS_TEST_ADMIN_PASSWORD=bucky2025 BUCKYOS_XCALL=/opt/buckyos/bin/xcall \
    pnpm exec playwright test -c playwright.aiworkspace.zone.config.ts --grep 'private schedule'
  ```

- [x] **C3 · 用真实的 Agent 身份写入。** 同一桌面 DV 在 Jarvis 自己的工具会话内执行 `who` 和两个 `add`，并核对日程页及两条 AIWS 提交的作者。
- [x] **C4 · 用真实模型验收 M3（A3）。** 已在新的 MessageHub 会话核对两轮工具记录、回复链接及洛杉矶当地日期，详见 §8。
- [x] **C5 · 补前置 TODO E4 的运行环境验证。** 在选了 host-runtime、经 Node Executor 执行的 Agent 里，以及在后台任务里，各执行一次 `xcall owner who` 和 `xcall owner.cal list`。确认读到同一份别名快照和用户覆盖、身份是该 Agent。在未安装扩展的 runtime 里执行时应报不可用。[前置 E4](opendan-object-local-extensions-todo.md#L132)
- [x] **C6 · 让文档对齐实现。** §3 补记忙闲入口的实际位置，§4.4 改写适配器传递调用元数据的现状，§8 补提交号和内置别名的来源。
- [x] **C7 · `notify` 的自动键改用两小时的时间片。** 按 Unix 时间每 7200 秒取片。§8 记录了同片重试和跨片发送的通知回执。

**owner 裁决（2026-11-20）**

1. buckyos-base 的改动已推送（C1）。
2. 接受内置别名随 SDK 分发（§8）。
3. 前置 TODO 的 Q1 定为两小时的时间片（C7），Q2 按 review 中的取舍，都已写回前置 TODO §7。
4. 本版本不做用户目录，也不做 Zone 外访问。以后对外开放时遵循 DID Document 的惯例，开通时登记 service（§3）。
5. 团队分派按 AIWS 惯例绑定工作区里的「联系人」表，联系人表在[联系人数据源 TODO](aiws-zone-contacts-source-todo.md) 中规划（§5）。

**首版验收之后 M4 的次序。** 第一步做本 Zone 内的 `invite`：查到忙闲后能直接约上时间，回执区分已发、已接受和拒绝。第二步做团队分派，排在联系人数据源之后。位置、实时通话和订阅依赖的 Provider 目前都不存在，放在最后。Zone 外访问不在本版本。

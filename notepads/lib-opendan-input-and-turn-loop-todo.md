# lib_opendan 输入协议、Turn Loop 与长任务恢复 TODO

日期：2026-10-02

Review 更新：2026-10-03。受控输入收敛为 `on_init / on_input / on_context_switch`；半订阅的渲染材料统一称为 `semi_subscription_snapshot`（半订阅快照），在受控输入使用前装配。以下为目标约定，尚未修改实现。

Review 更新（二）：2026-10-03。对照现有实现（libopendan、llm_context `msg_parser`、旧 OpenDAN 的 msg pump、MsgObject v2、AICC `AiMessage`）定稿两个核心设计：§3.2 Session Input Bus 的数据结构，§3.3 Input 链路（MsgObject → 模板视图 → 模板渲染 → user AiMessage → AICC）。同时把 §4 / §6 中引用长工具原设计的条目同步为已实施的简化版（`task_id`、`RunningTaskResolver`、task）。章节编号未变。

Review 更新（三）：2026-10-03。总线上的消息体直接使用 cyfs-ndn 的 MsgObject（协议级稳定结构），不再定义协议级的 `AgentMessage`；手工投递用一组构造 helper（§3.2.2）。msg bridge 只过滤不转换，MsgObject 到模板视图的转换移到 Session 内（§3.3.2 / §3.3.3）。

Review 更新（四）：2026-10-03。按实施前 review 定稿：协议升级后的旧 Session 迁移前只读；默认按最后一条已消费 input message 或父 Session 的来路回复；未订阅事件直接丢弃；每个 Session 最多 64 条 pending input，满时拒绝 append；预算不足按 AICC 服务不可用处理。长工具宿主接入复用已完成的 llm_context 设计。协议时间统一 UTC，用户时区与 Session 绑定并默认半订阅；附件文本可用 ObjId 或 Agent 可访问的路径保持引用自足。

Review 更新（五）：2026-10-03。模板优先使用 `{{ value | render_format: "格式名" }}`，例如 `{{ session.current_todo | render_format: "todo.summary_xml" }}`。命名格式负责结构排版、转义与附件定位信息；`xml / attr` 等作为底层工具保留，常规模板不必逐字段处理。

状态：**已实施（2026-10-03）**，实施记录与未完成项见 §10。依据本轮对 xAgent 输入设计、串行 / 并行等待和旧 AgentSession 的讨论，以及 `9cd6e8f5` 下的 libopendan 源码核对。

本 TODO 依赖的 llm_context 层修改（工具取消与时限、PendingTool 挂起记录与 `RunningTaskResolver`、`shell` 的执行模式与 task，已实施），以及需要 review 确认的 llm_context 事项，单独放在 [llm_context 长命令 / 长工具 TODO](./llm-context-long-tool-todo.md)。**llm_context 的修改总是先进行**，本 TODO 的宿主接入在其后。

## 1. 目标与范围

通过协议化分层，让应用、bridge、Session、LLM Context、Runtime 和 Agent State 各自承担清楚的职责，并支持独立进程、不同语言和多个 Session 协同推进。旧 OpenDAN 的复杂分支只作为反例和验收场景来源，不移植它的整体实现。

本 TODO 面向 `src/frame/lib_opendan` 的后续改进、xagent 验证和 `doc/opendan/protocol/` 的协议补全。本次只更新 TODO 与 xAgent 目标设计；后续实施也不修改 `src/frame/opendan`。TaskMgr / msg-center 的正式服务桥接继续按 xAgent 计划后移，先用模拟宿主验证协议边界。

Runtime 配置、沙箱和工具执行职责沿用 [AgentRuntime 下移 TODO](./llm-context-agent-runtime-todo.md)。libopendan 保留 Session 绑定、lease、输入提交和 run 生命周期管理，等待任务通过宿主提供的能力查询；不在 Session 内重新实现 TaskMgr、工具执行器或沙箱。

设计说明应先回答“投递什么 JSON，接收方如何处理，崩溃后如何恢复”，再给实现入口。Rust 的 `take_agent_inputs` 等函数只是协议行为的一种实现，不要求其它语言复制 Runner 的内部结构。

协议升级后，旧 Session 必须迁移到新协议才能继续推进；未迁移的一律 readonly，不允许 append input 或 drive。迁移是显式操作，本轮不实现自动迁移或旧格式兼容；读取旧现场用于查看、导出不等于允许新 Runner 推进。正式发布后的低概率旧版本问题按这个边界处理，不为它增加无限期事件版本历史。

## 2. 当前基线与已有能力

| 项目 | 当前 libopendan | 本 TODO 的重点 |
|---|---|---|
| 输入 | `msg / event / change / control / perception`，kmsg headers + 不透明的 JSON payload（msg 只约定 `{text}`） | 收敛为 `msg / event / control` 三种有 schema 的记录（§3.2），定稿跨语言线格式和处理语义 |
| 消息链路 | 批次渲染成一段文本，注入单条纯文本 user 消息；说话人取信封 `from`；没有 MsgObject 入口（旧 OpenDAN 在入口直接把 MsgObject 降成 AiMessage） | 一条链路：MsgObject（原样上总线）→ 模板视图 → 模板 → user AiMessage → AICC（§3.3），附件与消息结构不丢 |
| 提交 | 输入正文与 receipt 同快照；随后提交 state、清门槛、确认输入源 | 保留现有能力，把新的事件与等待分支纳入相同提交纪律 |
| 并发 | Session / run 排他锁与单写者已有实现 | 验证跨 Session、跨进程并行及同一 Session 的独占推进 |
| 长工具 | llm_context 层已实施（挂起记录 `task_id` + `until_ms`、`RunningTaskResolver`、task 工具）；libopendan 仍 `allow_deferred = false`（只对子 context 调用开启），`Pending` 在工具内等待，`waiting_for.refs` 为空 | 宿主侧接入：Session 层等待挂起的 task、结果回填与恢复后再开启 deferred（§6） |
| 输入渲染 | 主循环选择 `on_init / on_wakeup / on_behavior_switch`；检查点另有 `render_observation` 注入路径 | 收敛为三类受控输入，将半订阅快照装配放到受控输入使用前；system 模板独立命名 |
| 半订阅 | 当前有 change / 订阅游标；xAgent 提议持久化 `pending_events` | 渲染为 `semi_subscription_snapshot`，明确保留、版本合并和精确消费，避免误删新事件 |

上述旧 OpenDAN 审查发现不能直接当作 libopendan 的现有缺陷。特别是 receipt 提交顺序已经落地，后续应扩展并验证它。

## 3. P0：输入协议的两个核心设计

本节是定稿内容，不是示意。两个核心设计：

1. **Session Input Bus 里的数据结构**（§3.2）：总线上每条记录的线格式、三种 payload 的字段、校验与拒绝规则，以及它们在 state.json / receipt 中留下的结构。
2. **Input 链路**（§3.3）：`MsgObject →（原样上总线）→ 模板视图 → 模板渲染 → user AiMessage → AICC`，每一段由谁做、输入输出是什么、在哪里持久化、失败怎么办。反向链路（assistant message → MsgObject）较简单，见 §3.4。

§4 之后的各节（路由、受控输入、半订阅、提交与恢复）都以本节的结构为准。

### 3.1 Review：现有实现与目标的差距

依据 `lib_opendan/src/{protocol/input.rs, channel/kmsg.rs, runner/{assembler,live,hook,drive}.rs}`、`llm_context/src/msg_parser.rs`、`opendan/src/{msg_center_pump,prompt_env,session_model}.rs`、`ndn-lib/src/msgobj.rs`（MsgObject v2）、`buckyos-api/src/aicc_client.rs`（AiMessage）。

| # | 现状 | 问题 | 目标（本节） |
|---|---|---|---|
| R1 | `Input.payload` 是不透明的 `Value`；msg 的 payload 只约定 `{text}`，`InputMessage::text()` 取不到 `text` 就把整个 JSON 转成字符串 | 说话人、附件、引用、@、所在会话都到不了模板，也没有可校验的 schema | payload 按 `type` 是三种有 schema 的结构之一（§3.2.2–§3.2.4） |
| R2 | `<msg from=…>` 渲染的是信封头 `from`（投递者） | 经 bridge 投递的消息，说话人会显示成 bridge | 信封 `from` 只用于审计；说话人是 `msg.from`（§3.2.2） |
| R3 | `commit_input_batch` 注入单条 `AiMessage::text(User, text)`；receipt 只有一个 `content: String` 和一个 `message_pos` | 一批无法包含两条消息（半订阅快照 + 受控输入），也无法携带图片 / 文档块 | 一批是有序的 1–2 条 `AiMessage`，receipt 用 `parts[]` 逐条记录（§3.3.5） |
| R4 | MsgObject 到 LLM 有两条路：旧 OpenDAN 在入口用 `msg_parser` 把 MsgObject 直接降成 `AiMessage`（结构信息塞进 `ProviderState{buckyos.msg.metadata}`），存进 `PendingInput`，渲染时再从 AiMessage 反解出 `od.msg/1` JSON；libopendan 的总线则只有 `{text}` | 同一份结构被编码两次；AiMessage 是推理载体，却被当成队列里的消息格式 | 只有一条链路；总线上放 MsgObject 原样，AiMessage 只在渲染之后出现；不使用 `ProviderState` 作旁路（§3.3） |
| R5 | 斜杠命令在 `msg_parser::parse_msg_object_structured` 里识别 | 新链路中 Session 不解析正文 | 由 msg bridge 在入队前识别并投递 `control`；Session 只按 `type` 机械处理（§3.3.2） |
| R6 | 信封头 `intent`、`reply_to` 有定义，libopendan 内没有读取方 | 语义悬空 | 从信封删除；回复关系与机器意图本来就在 MsgObject 里（`thread.reply_to`、`relates_to`、`content.machine`） |
| R7 | kmsg headers 没有 schema 版本；消费侧 `key` 缺省为空串 | 无法拒绝旧格式；空 key 无法去重 | 增加 `schema`；`key` 必填（§3.2.1） |
| R8 | `InputSourceConfig::MsgCenter`（session 直接读 msg-center inbox，未实现） | 与“只有一套 InputBus”冲突，会出现第二种消费进度与确认纪律 | 删除该变体；msg-center 由 msg bridge 消费后投递到总线（§3.3.2） |
| R9 | xAgent §4.2 另定义了一个 `AgentMessage`（`text / attachments: Vec<AttachmentRef> / msg_ref / reply_to / intent`，`Principal`、`AttachmentRef` 未定义） | 群聊、@、消息关系、附件来源没有位置；补全它等于再维护一个与 MsgObject 重叠的协议对象 | 不定义协议级的 `AgentMessage`：消息体直接用 MsgObject（§3.2.2），模板视图是进程内结构（§3.3.3）；实施前反写 xAgent §4.2 / §4.3 / §4.5 |
| R10 | 本文 §4 / §6 仍引用长工具原设计的 `wait.source{kind, id}`、`DeferredResolver`、job | [llm_context TODO](./llm-context-long-tool-todo.md) 已简化并实施：挂起记录只有 `task_id`（+ `until_ms`），接口是 `RunningTaskResolver`，不区分 job 与 run | 本次已同步 §4 / §6 的相关条目 |

### 3.2 核心设计一：Session Input Bus 的数据结构

#### 3.2.1 总线记录（逻辑记录与通道映射）

总线上每条记录是一个 JSON 对象，称为**逻辑记录**。手工投递、跨语言 producer、fixtures 都以它为准：

```jsonc
{
  "schema": "opendan.session_input/3",
  "type": "msg",                      // msg | event | control
  "key": "cymsg:…c3",
  "from": "app:msg-bridge@alice",
  "at_ms": 1790899200000,
  "payload": { /* 由 type 决定：SessionMsg（MsgObject + delivery）| AgentEvent | ControlCommand */ }
}
```

| 字段 | 必需 | 约束 | 含义 |
|---|---|---|---|
| `schema` | ✔ | 必须等于 `opendan.session_input/3` | 线格式版本。其它值一律拒绝，不做旧版兼容 |
| `type` | ✔ | `msg` / `event` / `control` | 决定 `payload` 的结构。`change`、`perception` 不再是合法值 |
| `key` | ✔ | 1–256 字节，不含控制字符 | 逻辑输入的去重键。同一逻辑输入重投沿用同一个 key，不同的输入使用不同的 key（§5）。`type = msg` 时必须是消息的 ObjId（§3.2.2） |
| `from` | ✔ | 1–256 字节 | **投递者**的 principal（自报，只用于审计）。不是说话人，也不是权限依据：能否写入由队列的写权限决定 |
| `at_ms` | ✔ | u64 | 投递时间。只用于展示与诊断，不参与排序 |
| `payload` | ✔ | JSON 对象，序列化后 ≤ 250 KB | 更大的内容放 NamedStore 或 session 目录，只投递引用 |

消费侧由通道补上投递位置，producer 不能提供：

| 字段 | 来源 | 含义 |
|---|---|---|
| `src` | `channels.inputs[].id` | 输入源 |
| `index` | 通道（kmsg index） | 源内单调的投递位置；消费游标与累积确认用它 |

通道映射：

| 通道 | 映射 |
|---|---|
| kmsg | headers `schema` / `type` / `key` / `from` / `at_ms`（全部为字符串）；`Message.payload` 是 `payload` 的 UTF-8 JSON。删除 `intent`、`reply_to` 两个 header |
| 开发文件队列（`DirMsgQueue`） | 它实现的是同一个 kmsg 客户端接口，映射与 kmsg 完全相同 |
| CLI | `xagent post <sid> --json <file \| ->` 接受逻辑记录。`schema`、`from`、`at_ms` 可省略，由 CLI 填当前版本、调用者 principal 和当前时间；`type = msg` 时 `key` 也可省略，由 CLI 按 MsgObject 计算；记录里出现 `src` / `index` 时报错。`--msg "<text>"` 用 §3.2.2 的 helper 构造 MsgObject |

**pending input 容量**：每个 Session 的总线最多保留 64 条尚未提交消费的记录，`msg / event / control` 共用这个上限；满时不能 append，返回可重试的 `input_full`，不覆盖已有记录。容量检查与 append 必须原子执行，CLI、bridge 与跨语言 producer 使用同一限制；bridge 遇到满队列不确认上游，等待容量释放后重投。成功提交消费后释放名额，不把已经消费但等待累积 ack 的位置重复计入。监视任务扫描这最多 64 条 pending 记录查找控制命令，不因前面的普通输入暂存而漏掉已经入队的 stop。该限制约束总线积压；已接收并合并的半订阅状态按 §5 管理。

Rust 类型（`protocol/input.rs`，取代现有 `Input` / `InputKind` / `InputMessage`）：

```rust
pub const SESSION_INPUT_SCHEMA: &str = "opendan.session_input/3";

#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum SessionInput {
    Msg(SessionMsg),
    Event(AgentEvent),
    Control(ControlCommand),
}

/// producer 侧：一条待投递的逻辑记录。
pub struct PostedInput {
    pub schema: String,
    pub key: String,
    pub from: String,
    pub at_ms: u64,
    #[serde(flatten)]
    pub input: SessionInput,
}

/// consumer 侧：从输入源取到的一次投递。
pub struct FetchedInput {
    pub src: String,
    pub index: u64,
    pub key: String,
    pub from: String,
    pub at_ms: u64,
    /// 解析失败时为 Err(原因)，按 §3.2.5 拒绝。
    pub input: Result<SessionInput, RejectReason>,
}
```

`InputRef{src, index, key, kind}` 保留，`kind` 的取值收敛为 `msg | event | control`。

#### 3.2.2 消息：直接使用 MsgObject（`type = "msg"`）

有人对这个 session 说了一段话。消息体**直接使用 cyfs-ndn 的 MsgObject v2**（`CYFS 标准对象` §16，`ndn_lib::MsgObject`），不另外定义协议级的消息对象。MsgObject 本身是协议级的稳定结构，msg-center、MessageHub UI、websdk 都已经使用它；Session 只在它旁边挂一个很小的投递层结构，放 MsgObject 按设计不包含的信息。

```rust
/// `type = "msg"` 的 payload。
pub struct SessionMsg {
    pub msg: MsgObject,            // 原样，不改写、不裁剪
    #[serde(default)]
    pub delivery: MsgDelivery,     // 投递层信息，全部可选
}

pub struct MsgDelivery {
    pub from_name: Option<String>,          // 说话人显示名（msg-center 记录的 from_name 或联系人查询结果）
    pub conversation_name: Option<String>,  // 群名等会话显示名
    pub record_id: Option<String>,          // msg-center 记录 id，审计用
    pub tunnel: Option<String>,             // 入口 tunnel 的 DID
}
```

规则：

| 项 | 规则 |
|---|---|
| 信封 `key` | 必须等于 `msg` 的 ObjId（字符串形式）。消费侧用 `MsgObject::from_json_value_checked` 重新计算并核对，不一致按 `invalid_envelope` 拒绝。同一条消息重投使用同一 key，按 §5 的范围去重，`thread.reply_to` / `relates_to.target` 的 ObjId 直接对得上总线里的 key |
| 合法性 | `MsgObject::validate()` 通过；`content.content`、`content.refs`、`content.machine` 至少一项非空 |
| 说话人 | `msg.from`。总线不携带签名（JWT），可信度来自投递者：msg bridge 只投递 msg-center 已验证的记录；手工投递的消息与投递者同等可信。权限仍只来自 Session 的驱动身份 |
| `to` / `to_session` | Session 不校验、不路由：进了这个队列就是给这个 session 的。它们只用于渲染所在会话和推出回复目标 |
| `kind` | 不限制。哪些 kind 值得投递由 bridge 过滤（§3.3.2）；`chat` / `group_msg` 之外的 kind 渲染时带 `kind` 属性 |
| 附件 | 总线中使用 `content.refs` 的 `DataObj`（必须有 ObjId），本机文件先登记进 NamedStore 再引用。渲染给 Agent 的文本可用 ObjId 或宿主已解析、Agent 可访问的路径定位附件（I4）；路径是本地视图，不另造总线附件格式 |
| 大小 | 整个 payload ≤ 250 KB；`delivery` 各字段 ≤ 256 字节 |
| 未知字段 | `msg` 的未知顶层字段按 MsgObject 的规则落入 `meta`；`delivery` 的未知字段忽略 |

事件不借用 `kind = event` 的 MsgObject：AgentEvent 需要订阅、`seq` 合并和投递策略，这些不是消息语义（§3.2.3）。

**构造 helper**。手工投递和各语言的 producer 不需要手写完整的 MsgObject，libopendan 提供一组很薄的 helper（`protocol/input.rs`），只做构造，不引入新的线格式：

```rust
/// 一条发给 Agent 的文本消息：kind = chat，填 created_at_ms 与 nonce（保证两条相同正文的 ObjId 不同）。
pub fn text_msg(from: &DID, agent: &DID, text: impl Into<String>) -> MsgObject;
/// 追加一个数据对象附件（RefRole::Input）。
pub fn attach(msg: MsgObject, obj_id: ObjId, name: Option<String>) -> MsgObject;
/// 标记为对某条消息的回复。
pub fn reply_to(msg: MsgObject, target: ObjId) -> MsgObject;

impl PostedInput {
    /// 校验 MsgObject，计算 ObjId 作为 key，填 schema / at_ms。
    pub fn msg(poster: &str, msg: MsgObject, delivery: MsgDelivery) -> Result<Self>;
}
```

CLI 的 `xagent post <sid> --msg "<text>" [--from <did>] [--attach <file>]… [--reply-to <obj_id>]` 就是这组 helper 的命令行形式：`--from` 缺省取驱动者的 DID；`--attach` 的本机文件经 `LocalFileResolver`（出口已在用的同一个接口）登记为对象后引用。TS 侧在 websdk 的标准对象上提供同名 helper。

最小的手工投递（一对一、纯文本；ObjId 以省略形式示意）：

```json
{
  "schema": "opendan.session_input/3",
  "type": "msg",
  "key": "cymsg:…m1",
  "from": "did:bns:alice",
  "at_ms": 1790899200000,
  "payload": {
    "msg": {
      "from": "did:bns:alice",
      "to": ["did:bns:jarvis"],
      "kind": "chat",
      "created_at_ms": 1790899200000,
      "nonce": 4211,
      "content": { "content": "查询构建进度" }
    }
  }
}
```

msg bridge 投递的群消息（带附件与回复关系）：

```json
{
  "schema": "opendan.session_input/3",
  "type": "msg",
  "key": "cymsg:…c3",
  "from": "app:msg-bridge@alice",
  "at_ms": 1790899200500,
  "payload": {
    "msg": {
      "from": "did:bns:bob",
      "to": ["did:bns:dev-team"],
      "kind": "group_msg",
      "thread": { "reply_to": "cymsg:…99" },
      "mentions": { "dids": ["did:bns:jarvis"] },
      "created_at_ms": 1790899200000,
      "content": {
        "format": "text/plain",
        "content": "@jarvis 帮我看一下这张截图里的报错，日志在附件里",
        "refs": [
          { "role": "input", "target": { "type": "data_obj", "obj_id": "cyfile:…a1", "uri_hint": "screenshot.png" }, "label": "screenshot.png" },
          { "role": "input", "target": { "type": "data_obj", "obj_id": "cyfile:…b2", "uri_hint": "build.log" }, "label": "build.log" }
        ]
      }
    },
    "delivery": { "from_name": "Bob", "conversation_name": "Dev Team", "record_id": "r-102" }
  }
}
```

#### 3.2.3 AgentEvent（`type = "event"`）

这个 session 关心的某件事发生了。投递者只描述事件，不决定它是 Input 还是 Observe（由接收 Session 按订阅与模板解析，§4.1）。

```rust
pub struct AgentEvent {
    pub subscription_id: Option<String>,  // 显式订阅 id；Session 已登记的隐式订阅可为空
    pub source: EventSource,              // 事件来源
    pub event: String,                    // 事件名：changed / updated / finished / fired …
    pub seq: Option<u64>,                 // 来源内的版本号；Observe 合并用它比较新旧（§5）
    pub summary: String,                  // 给 LLM 的一句话，≤ 1 KB；不用于机械判断
    pub data_ref: Option<String>,         // 详情引用：ObjId 或相对 session 目录的路径
    pub terminal: bool,                   // 该来源的终结事件
}

pub struct EventSource {
    pub kind: String,   // object | session | task | timer | system
    pub id: String,     // object：对象 id；session：sid；task：task_id；timer：定时器名；system：可为空串
}
```

| 字段 | 必需 | 默认 | 约束 |
|---|---|---|---|
| `source.kind` | ✔ | | 未知种类不因格式拒绝；未匹配有效订阅时直接丢弃（§4.1） |
| `source.id` | ✔（`system` 除外） | `""` | ≤ 256 字节 |
| `event` | ✔ | | 1–64 字节 |
| `summary` | ✔ | | ≤ 1 KB；超出拒绝，不截断 |
| `subscription_id` / `seq` / `data_ref` | | 无 | |
| `terminal` | | `false` | |

`source.kind = task` 的 `id` 就是 llm_context 挂起记录与 task 工具使用的 `task_id`（对 Session 之外不透明），挂起调用与事件按它相等匹配（§4.1）。

```json
{
  "schema": "opendan.session_input/3",
  "type": "event",
  "key": "task:t1:revision:7",
  "from": "app:task-bridge@alice",
  "at_ms": 1790899200000,
  "payload": {
    "subscription_id": "watch-t1",
    "source": { "kind": "task", "id": "t1" },
    "event": "updated",
    "seq": 7,
    "summary": "任务 t1 的状态发生变化",
    "terminal": false
  }
}
```

例中的 `watch-t1` 必须对应接收 Session 的有效订阅，并匹配其 source。active 进入受控输入的候选批次，semi 更新待注入的半订阅状态；投递者不能替接收方决定策略。未匹配订阅时直接丢弃并提交消费位置，不进入上下文或 `pending_events`（§4.1）。timer、系统事件也没有无订阅兜底投递；内置 bridge 与用户时区使用 Session 已登记的隐式订阅。

#### 3.2.4 ControlCommand（`type = "control"`）

Session 控制协议。由 Runner 执行，不进入 LLM 上下文，不产生输入 receipt。沿用现有 `ControlCommand`（`#[serde(tag = "command")]`），`perception` 输入并入为 `perceive`：

| `command` | 字段 | 说明 |
|---|---|---|
| `stop` | `reason?` | |
| `decide` | `decision: accept \| discard`、`by`、`note?` | finished 之后唯一仍可接受的输入 |
| `subscribe` | `subscription: {id, mode, source, watch[]}` | |
| `unsubscribe` | `id` | |
| `activity` | `summary?`、`touch[]`、`clear` | |
| `perceive` | `kind`、`summary`、`tags[]`、`objects[]` | 原 `type = perception` 的 payload |

```json
{
  "schema": "opendan.session_input/3",
  "type": "control",
  "key": "ctl:stop:1790899260000",
  "from": "did:bns:alice",
  "at_ms": 1790899260000,
  "payload": { "command": "stop", "reason": "用户取消" }
}
```

应用规则不变，见 [Session Input Protocol](<../doc/opendan/protocol/Session Input Protocol.md>) §4；实施后该文拆成“Agent 输入”与“Session 控制”两篇。

#### 3.2.5 校验与拒绝

投递时（`post`）与消费时（`fetch`）用同一套校验。投递时不合法直接返回错误，不入队；消费时不合法（其它语言或旧版本的 producer 写入）按下表拒绝：标记为已消费、写 `input_rejected{src, index, key, reason}`，不进入上下文，不卡住累积确认。

| `reason` | 条件 |
|---|---|
| `unsupported_schema` | `schema` 缺失或不等于当前版本 |
| `unknown_type` | `type` 不是 `msg / event / control`（含旧的 `change`、`perception`） |
| `invalid_envelope` | `key` / `from` / `at_ms` 缺失或越界；`type = msg` 的 `key` 不等于消息的 ObjId |
| `payload_not_json` | payload 不是 JSON 对象 |
| `payload_too_large` | payload 超过 250 KB（正常情况下在投递时已被拒绝） |
| `invalid_payload` | 必需字段缺失、类型不符、超出 §3.2.2–§3.2.4 的约束；`msg` 未通过 `MsgObject::validate()` |
| `unknown_command` | control 的 `command` 未知 |
| `session_finished` | finished 之后收到 `decide` 以外的输入 |

payload 中的**未知字段忽略**，不拒绝（producer 可以比 consumer 新）；`control` 例外，未知 `command` 拒绝。拒绝是确定性的：同一条记录无论何时、由哪种语言的 Runner 消费，结果相同。

#### 3.2.6 输入在 state.json 与 receipt 中留下的结构

总线记录被消费后，只以三种形式留在 Session 里；它们与线格式一起升版（`state.json`、`session_config`、快照 host meta）。

**待注入的半订阅状态**（`state.pending_events`，Observe 事件的落点；合并与清理规则见 §5）：

```jsonc
"pending_events": [
  {
    "subscription_id": "watch-obj",               // 已登记的隐式订阅可为 null
    "source": { "kind": "object", "id": "…" },
    "latest": {                                    // 该 (subscription_id, source) 的最新版本
      "seq": 8, "key": "obj:…:8", "event": "changed", "summary": "…", "data_ref": null,
      "received_at_ms": 1790899200000,
      "input": { "src": "q", "index": 131 }
    },
    "terminal": null,                              // 终结事件单列，结构同 latest，不被后续版本覆盖
    "superseded": 2                                // 被覆盖的旧版本数
  }
]
```

现有的 `subscription_cursors` 只保留拉模式游标（Session 来源订阅的登记表 rev、`_active_sessions`）；推模式 change 的游标由 `pending_events` 取代。

**输入 receipt**（`state.host.libopendan.input_receipts[]`，与正文同快照）：

```jsonc
{
  "run_id": "…", "input_seq": 2, "turn": 5, "opens_turn": true,
  "hook": "on_input",                              // on_init | on_input | on_context_switch
  "inputs": [ { "src": "q", "index": 121, "key": "cymsg:…c3", "kind": "msg" } ],
  "events": [ { "subscription_id": "watch-obj", "source": { "kind": "object", "id": "…" }, "seq": 8, "key": "obj:…:8" } ],
  "reply": { "route": "message", "to": "did:bns:dev-team", "to_session": null, "kind": "group_msg", "reply_to": "cymsg:…c3", "tunnel": null },
  "parts": [
    { "part": "semi_subscription_snapshot", "pos": { "kind": "accumulated", "index": 6 }, "text": "<semi_subscription_snapshot>…" },
    { "part": "input",                      "pos": { "kind": "accumulated", "index": 7 }, "text": "<session_input hook=\"on_input\" …" }
  ],
  "bootstrap": false, "after_step": 7, "extra": {}, "at_ms": 0
}
```

- `inputs`：本批消费的 msg 与 Input event 的投递位置。
- `events`：本批快照实际注入的半订阅状态版本；提交时按 `(subscription_id, source, key)` 精确清除，存在 seq 时同时核对 seq（§5）。取代 `changes[]` 与 `consumed_only[]`：Observe 事件在合并进 `pending_events` 时已标记消费，不再随 receipt 消费。
- `reply`：本批提交后的默认回复路径，与正文同快照保存。恢复按 receipt 补回 `state.reply`，不从渲染文本反解，也不重新读取可能已清理的总线记录。
- `parts`：本批注入的 1–2 条消息，取代单个 `content` + `message_pos`。`text` 是该消息的文本块；图片 / 文档块不重复保存，按 `pos` 在同一份快照中读取。

**Session 的默认回复路径**（`state.reply`，供 §3.4 使用）：每次提交输入批次，取本批按消费顺序排列的最后一条 input message 的来路；同一 Turn 的后续消息可更新它。event、control、半订阅快照和 context 交接本身不覆盖回复路径；交接批次并入了 message 时仍按该 message 更新。无新 message 时保留上一条路径；没有 message 来路而有 parent session 时，使用创建时记录的父 Session 来路；两者都没有则不产生默认消息回复。

```jsonc
"reply": { "route": "message", "to": "did:bns:dev-team", "to_session": null, "kind": "group_msg", "reply_to": "cymsg:…c3", "tunnel": null }
```

一对一回复取原消息的发送者，群消息回复取原群，保留原路需要的会话与 tunnel 信息，`thread.reply_to` 指向最后一条 input message 的 ObjId。父 Session 来路形如 `{"route":"parent_session","session_id":"…"}`，按父子 Session 的结果 / 消息通道回送。Batch 的“最后一条”按 §3.3.3 的消费顺序确定，不按发送时间或渲染后的顺序确定。

### 3.3 核心设计二：Input 链路

#### 3.3.1 总览与不变量

```text
MsgObject（+ msg-center 记录）           不可变、可寻址、跨 Zone 的消息对象
   │ ① 入队        msg bridge：过滤、分流、补投递层信息；MsgObject 原样，不转换
   ▼
SessionInput{type: msg, payload: {msg, delivery}} ── InputBus      持久、有序、至少一次
   │ ② 选批        Session：校验、去重、排序、input.mode、预算、Turn 归属；MsgObject → 模板视图
   ▼
InputView                                              模板变量 input.*，进程内的纯数据
   │ ③ 渲染        SessionAssembler：behavior 的 on_init / on_input / on_context_switch 模板，纯函数
   ▼
RenderedInput { text, media[] }
   │ ④ 落盘        Session：组成 user AiMessage，与 receipt 写进同一份快照
   ▼
AiMessage{role: user}                                   LLMContext 历史的一部分
   │ ⑤ 推理请求    llm_context 组请求；AICC 解析资源并降格为 provider 原生格式
   ▼
provider 原生消息
```

| 段 | 负责方 | 输入 → 输出 | 持久化 | 失败时 |
|---|---|---|---|---|
| ① 入队 | msg bridge（Session 之外） | msg-center 记录 + MsgObject → 总线记录（MsgObject 原样） | 入队成功即持久 | 不确认 msg-center 记录，稍后重试；重投由 `key` 去重 |
| ② 选批 | Session（lease 持有者） | 已持久的投递 → 本批选中的消息与事件的 `InputView` | 无（未提交前不出队） | 不合法的记录按 §3.2.5 拒绝；未选中的留在队列 |
| ③ 渲染 | SessionAssembler | `InputView` → `RenderedInput` | 无 | 报错并保留现场；不确认输入 |
| ④ 落盘 | Session | `RenderedInput` → 快照中的 `AiMessage` + receipt | 快照 → run.json 门槛 → state.json → 清门槛 → 确认输入源（§5） | 各窗口崩溃按 receipt 恢复，不重新渲染 |
| ⑤ 推理请求 | llm_context、AICC | `AiMessage[]` → provider 请求 | 无新增 | 按 llm_context 的错误策略产生 Outcome |

不变量：

- **I1 结构只编码一次。** 渲染之前，MsgObject 是消息结构的唯一载体，不引入第二个协议级的消息对象；`InputView` 只是它在进程内的视图。总线和 state 中不出现 `AiMessage`；注入的 `AiMessage` 不用 `ProviderState` 携带消息元数据。
- **I2 每段只做一件事。** bridge 不渲染、不决定投递策略；模板不决定路由、出队和 Turn 边界；bridge 不转换消息；llm_context 的推理循环不认识 MsgObject；AICC 不认识 Session。
- **I3 只渲染一次。** ④ 提交之后，上下文里的输入就是快照中的 `AiMessage`。恢复、接手（xllm）、压缩和历史重建都从快照与 worklog 读取，不回到总线重新渲染（那时总线记录可能已被清理，模板变量中的新鲜值也已变化）。
- **I4 文本自足。** 每个附件都在文本块中以 `index`、名称和可读取的来源出现；来源可以是 ObjId，也可以是 Agent 在该 Session runtime 中可访问的路径。去掉图片 / 文档块（纯文本模型、压缩、历史重建）后仍能定位附件。仅有显示名不算可读取路径；使用路径时宿主须保证其在保留的 Session 历史中仍可用，临时缓存路径使用 ObjId 兜底。
- **I5 正文不能伪造结构。** `text` 与所有属性值在渲染时转义；`from`、`key`、`mentioned` 等属性只来自 MsgObject 与 `delivery` 的结构字段。权限来自 Session 的驱动身份，从不来自正文。

#### 3.3.2 ① 入队：msg bridge 过滤后原样投递

位置：`libopendan` 新模块 `bridge/msg.rs`，供 xagent `serve` 与以后的 OpenDAN Supervisor 使用。它是 msg-center inbox 的唯一消费者，Session 不直接读 msg-center（R8）。bridge 不转换消息：MsgObject 原样上总线，bridge 只做过滤、分流和补投递层信息。

```rust
pub enum MsgBridgeOutput {
    Deliver { delivery: MsgDelivery },        // 原样投递 msg，key = ObjId
    Control { key: String, command: ControlCommand },
    Drop { reason: &'static str },            // 确认 msg-center 记录，不投递
}

/// 纯函数：不访问网络；联系人显示名由调用方先查好放进 ctx。
pub fn route_msg_record(record: &MsgRecord, msg: &MsgObject, ctx: &MsgBridgeCtx) -> MsgBridgeOutput;
```

`delivery` 的来源：`from_name` 取记录的 `from_name`，没有则查联系人；`conversation_name` 取群名；`record_id`、`tunnel`（`ingress.transport_did`）取自记录。

过滤与分流（机械规则，按顺序）：

| 情况 | 输出 |
|---|---|
| 记录的发送者与 `msg.from` 不一致 | `Drop`（总线上说话人只认 `msg.from`，不一致的记录不能进入） |
| 群邀请通知（`buckyos.group_invitation`）、Agent 自己的群消息回显 | `Drop`（沿用 `msg_center_pump` 现有规则） |
| 正文、refs、machine 全空 | `Drop` |
| `relates_to.rel = reaction` | `Drop`（首版不投递） |
| 纯文本、无 refs 与 machine、匹配 `^/(<已登记命令>)(\s+<args>)?$`，且发送者是该 session 的驱动者或 Agent owner | 命令在 bridge 的映射表中 → `Control`（如 `/stop` → `stop`，`key` 为 `ctl:<ObjId>`）；已登记但不映射到 Session 控制的命令由 bridge 所在的应用自行处理，不投递 |
| 其它（含 `edit` / `redact`） | `Deliver` |

Session 不解析正文：未被 bridge 识别的斜杠文本就是普通消息，由 LLM 理解（§4.1）。

投递与确认：先 `post` 到目标 session 的队列，成功后再确认（ack）msg-center 记录。两步之间崩溃会重投，由 `key`（ObjId）去重。目标 session 由 bridge 所在的应用按 `to` / `to_session` 路由，不属于本协议。

#### 3.3.3 ② 选批：MsgObject → InputView

选批规则见 §4（路由、`input.mode`、预算、暂存）。选批的输出是纯数据的 `InputView`，是模板能看到的全部输入材料。它是进程内的视图，不是协议对象：不上总线、不落盘，各语言的 Runner 可以有自己的表示，只要模板变量名和 §3.3.4 的渲染结果一致。

```rust
pub struct InputView {
    pub hook: String,                // on_init | on_input | on_context_switch
    pub time: String,                // 本批时间（UTC RFC 3339，Z 后缀）
    pub messages: Vec<MessageView>,  // 本批选中的消息
    pub events: Vec<EventView>,      // 本批选中的 Input event
    pub items: Vec<ItemView>,        // messages 与 events 按消费顺序的混排：{is_msg, is_event, …对应视图的全部字段}
    pub text: String,                // 内建渲染的 <inputs> 块（§3.3.4）；没有输入时为空串
    pub count: usize,                // messages.len() + events.len()
}

pub struct MessageView {
    pub key: String,                        // 信封 key = 消息 ObjId
    pub kind: String,                       // MsgObjKind
    pub from: SpeakerView,                  // {id, did, name}
    pub conversation: Option<ConversationView>, // {kind: direct | group, id, session, name}
    pub is_group: bool,                     // conversation.kind == group；upon 没有比较运算，给布尔字段
    pub text: String,
    pub format: String,
    pub title: Option<String>,
    pub attachments: Vec<AttachmentView>,   // {index, media, name, mime, obj_id, path?, role}
    pub relations: Vec<RelationView>,       // {rel, msg_id, reaction}
    pub mentions: MentionsView,             // {me, all, dids}
    pub machine: Option<Value>,
    pub sent_at: Option<String>,
    pub received_at: String,                // 信封 at_ms
}

pub struct EventView {
    pub key: String,
    pub subscription_id: Option<String>,
    pub source: EventSource,
    pub event: String,
    pub seq: Option<u64>,
    pub summary: String,
    pub data_ref: Option<String>,
    pub terminal: bool,
}
```

`MessageView` 由 Session 从 `SessionMsg` 机械生成（纯函数 `message_view(key, at_ms, &SessionMsg, agent_did)`）：

| MessageView | 来源 | 规则 |
|---|---|---|
| `from.did` / `from.id` | `msg.from` | `id = from.to_raw_host_name()` |
| `from.name` | `delivery.from_name` | 没有则省略 |
| `conversation` | `msg.kind`、`msg.to`、`msg.to_session`、`delivery.conversation_name` | `group_msg` → `{kind: group, id: to[0]}`；其它 kind 只有 `to_session` 非空时给出 `{kind: direct, session}` |
| `text` | `content.content`（trim） | 正文只是 `[attachment]` / `[image]` 等占位词且有 refs 时置空（沿用 `msg_parser` 的规则） |
| `format` / `title` | `content.format` / `content.title` | 非 `text/*` 的 `format` 记为 `text/plain`，只用于推断附件类型 |
| `attachments[]` | `content.refs` 中的 `DataObj`，按数组顺序编号 | `media` / `mime` 用 `msg_parser` 现有的 `attachment_kind` / `attachment_mime` 推断（需从 llm_context 导出，属 llm_context 层的小改动，先行）；`name = label`；`role = ref.role`。宿主已有该 ObjId 到 Session runtime 可读路径的映射时，可在模板使用前补入 `path`；不把 label 或未经验证的 uri_hint 当作可读路径，不在渲染函数里下载文件 |
| `relations[]` | `thread.reply_to` → `reply_to`；`relates_to` → 同名 `rel` | `msg_id` 是目标消息的 ObjId，也就是它在总线上的 key |
| `mentions` | `msg.mentions` | `me = dids 含 Agent DID`（`all` 不置 `me`） |
| `machine` | `content.machine` | |
| `sent_at` | `created_at_ms` | 0 视为缺失 |

`content.refs` 中的 `ServiceDid` 引用首版不进入视图（记入待确认）。

消费顺序：同一输入源内按 `index`；多个输入源按 `channels.inputs` 的声明顺序归并。不按 `at_ms` 或 `created_at_ms` 排序。

模板可用的其它变量（`runtime.*`、`<hints>`、`<active_sessions>`、交接状态、半订阅快照等）沿用 xAgent §6.5 的“每批现算”一栏；`input.*` 是其中唯一来自总线的部分。

所有协议时间与内建渲染中的时间统一用 UTC；RFC 3339 使用 `Z` 后缀，毫秒精度按 fixtures 固定。用户时区是与 Session 绑定的上下文，不取 Runner 所在机器的时区：创建时绑定 `session.timezone`（IANA 时区名），默认建立该用户时区的隐式 semi 订阅。初始值与后续变化进入 `semi_subscription_snapshot`，在下一次受控输入前呈现；时区变化本身不唤醒推理。UTC 时间与已知用户时区共同提供本地时间语境，Runner 换机器不会改变 `input.text`。

#### 3.3.4 ③ 模板渲染：InputView → RenderedInput

渲染用 `llm_context::prompt_engine`（与 behavior 的 system 模板同一个引擎，关闭 `__EXEC__`）。模板来自冻结的 behavior：`prompt.on_init / on_input / on_context_switch`（§4.2），缺省使用内建模板。模板的输出就是 user message 的全文，Session 不再包一层。

内建的 `on_input` 模板：

```text
<session_input hook="{{input.hook}}" time="{{input.time}}">
{{input.text}}
…（hints、活动 session、runtime 状态等每批现算的材料，沿用现有 DefaultAssembler 的顺序）
</session_input>
```

`input.text` 的格式固定（跨语言 Runner 必须逐字节一致，由 fixtures 约束）：

```xml
<inputs>
<msg key="cymsg:…c3" from="Bob" from_id="bob.bns.did" group="Dev Team" mentioned="true" reply_to="cymsg:…99" time="2026-10-02T00:00:00Z">
@jarvis 帮我看一下这张截图里的报错，日志在附件里
<attachment index="0" media="image" name="screenshot.png" mime="image/png" obj_id="cyfile:…a1"/>
<attachment index="1" media="document" name="build.log" mime="text/plain" obj_id="cyfile:…b2"/>
</msg>
<event key="task:t1:revision:7" subscription="watch-t1" source="task:t1" event="updated" seq="7">任务 t1 的状态发生变化</event>
</inputs>
```

| 元素 / 属性 | 来源 | 规则 |
|---|---|---|
| `<msg key>` | 信封 `key`（消息 ObjId） | 总是输出 |
| `kind` | `kind` | 仅 `chat` / `group_msg` 之外时 |
| `from` / `from_id` | `from.name` / `from.id` | 没有 `name` 时 `from` 取 `id`，不输出 `from_id` |
| `group` | `conversation.name`，缺省取 `conversation.id` | 仅 `kind = group` |
| `session` | `conversation.session` | 非空时 |
| `mentioned` | `mentions.me` | 仅为 true 时 |
| `reply_to` / `edit_of` / `redacts` / `thread` | `relations[]` | 值为目标消息的 ObjId，与它的 `<msg key>` 相同 |
| `format` | `format` | 仅非 `text/plain` 时 |
| `title` | `title` | 非空时 |
| `time` | `sent_at`，缺省取 `received_at` | UTC RFC 3339，`Z` 后缀 |
| 元素正文 | `text` | 转义 `& < >`；前后各一个换行 |
| `<attachment index media name mime obj_id path role/>` | `attachments[]` | 每个附件一行，按数组顺序；内建渲染保留 ObjId，`path` 有可读映射时输出，`role` 仅非 `input` 时输出；自定义模板可用其中任一可读来源满足 I4 |
| `<event key subscription source event seq terminal>` | AgentEvent | `source` 为 `kind:id`；正文是转义后的 `summary`；`data_ref` 非空时输出 `data_ref` 属性；`terminal` 仅为 true 时 |

属性值转义 `& < > "`。即使只有一条纯文本消息也使用 `<msg>` 元素，不退化成裸文本：多方对话里说话人与 `key` 始终可见，历史中的形态也保持一致。附件行的写法与 Agent 发送附件时使用的 `<attachment obj_id=… title=…/>` 标签（`msg_parser` 的出口约定）相近但属性不同，入口不解析它，仅供 LLM 阅读。

自定义模板可以只用 `{{ input | render_format: "input.xml" }}`，与 `{{input.text}}` 的结果相同；也可以遍历 `input.messages` / `input.events`，为每个对象选择命名格式。`input.text` 保留为已渲染片段，现有 Jarvis 模板改名后仍可引用。命名格式、底层 filter 和六个模板示例见 §3.3.8。`machine` 不在内建渲染中出现，需要时由模板显式取用。

渲染是纯函数：不读队列、不推进游标、不写 state。已有选中的输入却渲染出空文本时报错并保留现场（§4.2）。

渲染结果：

```rust
pub struct RenderedInput {
    pub text: String,             // 模板输出
    pub media: Vec<AiContent>,    // 随文本一起注入的图片 / 文档块，可为空
}
```

`media` 由 behavior 的 `input.media` 策略机械生成，与模板无关：

| `input.media` | 行为 |
|---|---|
| `reference`（默认） | `media` 为空。附件只以文本中的 `<attachment>` 行出现，Agent 用工具（如 `llm_understand_media`、读文件）按需读取 |
| `inline` | 按消息顺序、附件顺序，为 `media ∈ {image, document}` 的附件生成 `AiContent::Image{source: NamedObject}` / `AiContent::Document{source: NamedObject, title: name}`；每批最多 8 个，超出的只保留文本引用。音频、视频和普通文件始终只有文本引用 |

#### 3.3.5 ④ 落盘：RenderedInput → user AiMessage

```rust
AiMessage { role: AiRole::User, content: [ AiContent::Text{ text: rendered.text } ] ++ rendered.media }
```

一个输入批次由有序的 1–2 条这样的消息组成：有待注入的半订阅状态时，先是半订阅快照消息（纯文本，§4.3），再是受控输入消息。两条消息通过一次 `LLMContext::inject(Injection{messages, host})` 注入，中间不推理、不发布快照。

| loop 模式与时机 | 注入位置（现有 `inject` 语义，不需要改 llm_context） | receipt `parts[].pos` |
|---|---|---|
| function call | 追加到 `state.accumulated`，两条消息相邻 | `accumulated`，index 连续 |
| behavior，首个 Step 之前 | 追加到 `request.input` | `request_input`，index 连续 |
| behavior，已有 Step | 两条消息的内容块依次并入该 Step 的 `next_user_message`（一条 user 消息，块的顺序即注入顺序） | 两个 part 都是 `step{index}` |

receipt 按 §3.2.6 的结构与消息写进同一份快照；提交顺序不变（§5）。worklog 的 `user_message` 只记各 part 的 `text`。恢复时按 receipt 补齐 state，不重新渲染、不重新注入（I3）。

历史与压缩：新 run 重建 session history、机械压缩与上下文改写时，早于当前 Turn 的输入消息只保留文本块，去掉图片 / 文档块（I4 保证引用不丢）。当前 Turn 内的块保持原样，以维持请求前缀稳定。

#### 3.3.6 ⑤ user AiMessage → AICC

这一段没有新增设计，只确认边界：

- llm_context 把 `request.input`、`state.accumulated`、Step 渲染结果组成 `AiMessage[]`，作为 AICC `llm` 请求的输入。llm_context 不改写 user 消息的内容块。
- AICC 的资源层（`aicc/src/resource`）把 `ResourceRef::NamedObject` 解析并验证为 provider 可用的形式；各 provider adapter 把 `AiRole` 与内容块降格成原生格式（`AiMessage` 重构文档 §1.4）。Session 与 bridge 都不做 provider 相关的处理。
- `input.media = inline` 而模型不支持图片 / 文档，或对象不可读时，AICC 返回 `ResourceInvalid` / `UnsupportedOperation`，llm_context 按错误策略给出 `Outcome::Error`。Session 不自动去掉媒体块重试；是否提供机械降级见待确认。

#### 3.3.7 端到端示例

msg-center 收到的 MsgObject（群消息，节选）：

```json
{
  "from": "did:bns:bob",
  "to": ["did:bns:dev-team"],
  "kind": "group_msg",
  "thread": { "reply_to": "cymsg:…99" },
  "mentions": { "dids": ["did:bns:jarvis"] },
  "created_at_ms": 1790899200000,
  "content": {
    "format": "text/plain",
    "content": "@jarvis 帮我看一下这张截图里的报错，日志在附件里",
    "refs": [
      { "role": "input", "target": { "type": "data_obj", "obj_id": "cyfile:…a1", "uri_hint": "screenshot.png" }, "label": "screenshot.png" },
      { "role": "input", "target": { "type": "data_obj", "obj_id": "cyfile:…b2", "uri_hint": "build.log" }, "label": "build.log" }
    ]
  }
}
```

① bridge 补上显示名后原样投递，总线记录就是 §3.2.2 的第二个样例。② 选中它（Batch，本批只有这一条）。③ 用内建模板渲染出：

```xml
<session_input hook="on_input" time="2026-10-02T00:00:01Z">
<inputs>
<msg key="cymsg:…c3" from="Bob" from_id="bob.bns.did" group="Dev Team" mentioned="true" reply_to="cymsg:…99" time="2026-10-02T00:00:00Z">
@jarvis 帮我看一下这张截图里的报错，日志在附件里
<attachment index="0" media="image" name="screenshot.png" mime="image/png" obj_id="cyfile:…a1"/>
<attachment index="1" media="document" name="build.log" mime="text/plain" obj_id="cyfile:…b2"/>
</msg>
</inputs>
</session_input>
```

④ 在 `input.media = inline` 时注入的消息（`reference` 时只有第一个块）：

```json
{
  "role": "user",
  "content": [
    { "type": "text", "text": "<session_input hook=\"on_input\" …>…</session_input>" },
    { "type": "image", "source": { "kind": "named_object", "obj_id": "cyfile:…a1" } },
    { "type": "document", "source": { "kind": "named_object", "obj_id": "cyfile:…b2" }, "title": "build.log" }
  ]
}
```

同一份快照中的 receipt：`hook = on_input`、`inputs = [{src: "q", index: 121, key: "cymsg:…c3", kind: "msg"}]`、`parts = [{part: "input", pos: {kind: "accumulated", index: 7}, text: "<session_input …"}]`，`opens_turn` 取决于提交时是否有打开的 Turn。⑤ 由 AICC 读取两个对象并降格为所选 provider 的格式。

#### 3.3.8 命名渲染格式、内置 filter 与模板示例

模板引擎是 upon（`llm_context::prompt_engine`）。upon 支持在引擎上注册 filter（`Engine::add_filter`，模板里写 `{{ value | name }}` 或 `{{ value | name: arg }}`），现在的 `PromptRenderEngine` 每次渲染都新建一个空的 `Engine`，没有注册任何 filter。以下是待实施的接口与示例。

模板作者优先选择“对象 + 命名格式”，由格式渲染器处理字段、默认值和转义：

```jinja
{{ session.current_todo | render_format: "todo.summary_xml" }}
{{ msg | render_format: "message.xml" }}
{{ msg.attachments | render_format: "attachments.list" }}
```

`render_format(value, format_name)` 是一个通用 filter，注册在 `llm_context::prompt_engine`；具体格式由宿主注册，llm_context 不内置 Session、MsgObject 或 todo 词汇。格式名是稳定标识，不是文件路径；渲染时只查已注册格式，不执行命令、不访问网络、不临时加载文件。`session.current_todo` 等材料由宿主在渲染前装配，filter 本身不读取 todos.json。格式未知或输入形状不匹配时报模板错误，沿用“保留现场、不消费输入”的规则；格式声明支持的空值输出空串。

**命名格式**（由 libopendan 注册；输入格式基于 §3.3.3 的模板视图）：

| 格式名 | 用法 | 结果 |
|---|---|---|
| `input.xml` | `{{ input \| render_format: "input.xml" }}` | 按 `input.items` 顺序渲染 `<inputs>` 块，与 `input.text` 逐字节一致；无输入时为空串 |
| `message.xml` | `{{ msg \| render_format: "message.xml" }}` | 内建 `<msg …>…</msg>` 元素，与 `input.text` 中的同一条逐字节一致；包含说话人、@、关系和附件引用 |
| `message.markdown` | `{{ msg \| render_format: "message.markdown" }}` | `Request from …` 标题、引用正文与附件清单，附件保留 ObjId 或可读路径 |
| `event.xml` | `{{ ev \| render_format: "event.xml" }}` | 内建 `<event …>…</event>` 元素，与 `input.text` 中的同一条逐字节一致 |
| `event.summary_text` | `{{ ev \| render_format: "event.summary_text" }}` | 单行来源、事件名、终态标记与摘要，摘要压缩空白后最多 300 个字符 |
| `attachments.xml` | `{{ msg.attachments \| render_format: "attachments.xml" }}` | 每个附件一行 `<attachment …/>`；没有附件时为空串 |
| `attachments.list` | `{{ msg.attachments \| render_format: "attachments.list" }}` | 一行纯文本，每项带可读取来源：`[0] screenshot.png (image, obj_id=cyfile:…a1), [1] build.log (document, path=/workspace/build.log)`；有可用路径时可用路径，否则用 ObjId |
| `todo.summary_xml` | `{{ session.current_todo \| render_format: "todo.summary_xml" }}` | 把宿主已装配的当前 todo 视图渲染为 XML 摘要，包含其标识、状态和摘要文本；无当前 todo 时为空串 |

XML 格式在内部对正文与属性分别转义，模板直接插入生成的完整片段，不再叠加 `xml` 或 `attr`。格式注册与 golden fixtures 共用一份渲染定义；`input.text` 也调用同一套输入格式渲染器，避免出现两种输出。

**底层通用 filter**（注册在 `llm_context::prompt_engine`，不含 Session 词汇，system 模板同样可用；用于自定义排版或格式实现，常规模板优先使用上面的 `render_format`）：

| filter | 用法 | 结果 |
|---|---|---|
| `xml` | `{{ text \| xml }}` | 转义 `& < >`，用于元素正文 |
| `attr` | `name="{{ name \| attr }}"` | 转义 `& < > "`，用于属性值 |
| `json` | `{{ msg.machine \| json }}` | 任意值的紧凑 JSON |
| `truncate` | `{{ msg.text \| truncate: 200 }}` | 最多 n 个字符，截断时以 `…` 结尾 |
| `oneline` | `{{ msg.text \| oneline }}` | 把连续空白（含换行）压成一个空格 |
| `default` | `{{ msg.from.name \| default: "unknown" }}` | 值为空（None / 空串 / 空列表）时取参数 |
| `join` | `{{ msg.mentions.dids \| join: ", " }}` | 字符串列表拼接 |
| `quote` | `{{ msg.text \| quote }}` | 每行前加 `> `（Markdown 引用） |
| `time` | `{{ msg.sent_at \| time: "%H:%M" }}` | 把 RFC 3339 字符串或毫秒时间戳按 UTC 格式化；值为空时输出空串；用户时区通过 Session 的默认半订阅提供 |

约定：

- filter 都是纯函数，不读队列、不访问网络、不改 state。
- upon 没有比较运算符，条件只判断真假（None、false、0、空串、空列表为假）。所以视图里直接给布尔字段和可判空的字段：`msg.is_group`、`msg.mentions.me`、`msg.attachments`、`msg.relations`、`ev.terminal`、`input.messages`、`input.events`。可能缺失的字段用 `?.` 访问（如 `msg?.title`）。
- upon 不会自动转义。`render_format` 的 XML 格式统一负责转义，使用这些格式的模板满足 I5；直接拼装 XML 的自定义模板仍需用底层 `xml / attr` 正确处理字段。纯文本 / Markdown 格式不提供 XML 结构保证。
- 无论模板怎么排版，每个附件都要以 ObjId 或可读取路径出现在文本里（使用包含附件的 message 格式、`attachments.xml / attachments.list` 或自行遍历），否则破坏 I4。Session 在渲染后逐个检查附件：其 ObjId 与可用路径均未出现时记 warning，不拒绝；显示名不能替代定位信息。内建格式与示例必须满足 I4，自定义模板作者负责响应 warning。

模板示例（behavior cfg 的 `[prompt]` 段；`{{ … }}` 之外的变量名沿用 xAgent §6.5）。

**例 1：内建模板的等价写法**——不写 `on_input` 时就是这个效果：

```toml
on_input = """
<session_input hook="{{ input.hook }}" time="{{ input.time }}">
{{ input | render_format: "input.xml" }}
</session_input>
"""
```

**例 2：一对一 UI 对话**——在输入前加一段背景与当前待办摘要，输入用内建格式（现有 Jarvis `chat_route` 的背景变量继续可用）：

```toml
on_input = """
{% if session.background_hint_changed %}
<background_environment current_clock="{{ runtime.clock_text }}">
{{ session.default_changed_background_hint_text }}
</background_environment>
{% endif %}
{{ session.current_todo | render_format: "todo.summary_xml" }}
{{ input | render_format: "input.xml" }}
"""
```

**例 3：群聊**——按聊天记录的样子排版，突出谁在说话、有没有 @ 自己：

```toml
on_input = """
<group_chat time="{{ input.time }}">
{% for msg in input.messages %}
{{ msg | render_format: "message.xml" }}
{% endfor %}
</group_chat>
Reply only if you were mentioned or the message is clearly addressed to you.
"""
```

**例 4：work session 的启动消息**——把首批消息当作任务说明，用 Markdown 排版，附件列成清单：

```toml
on_init = """
# Task
{{ session.objective }}

{% for msg in input.messages %}
{{ msg | render_format: "message.markdown" }}
{% endfor %}
"""
```

**例 5：事件驱动的 session**（定时自检、任务观察）——消息少、事件多，事件一行一条，终结事件单独标出：

```toml
on_input = """
Wakeup at {{ input.time }}
{% for ev in input.events %}
{{ ev | render_format: "event.summary_text" }}
{% endfor %}
{% for msg in input.messages %}
{{ msg | render_format: "message.xml" }}
{% endfor %}
"""
```

**例 6：混排并保持到达顺序**——消息与事件按消费顺序逐条输出，每条仍用内建格式：

```toml
on_input = """
{% for item in input.items %}
{% if item.is_msg %}{{ item | render_format: "message.xml" }}{% else %}{{ item | render_format: "event.xml" }}{% endif %}
{% endfor %}
"""
```

`on_context_switch` 模板使用同一组 `input.*` 变量和 filter；交接批次没有并入外部输入时 `input.messages` 与 `input.events` 为空。

### 3.4 反向链路：assistant message → MsgObject

```text
assistant AiMessage（function call 的最终回复）/ behavior 结果中的 messages_to_send
   │ Session：取 state.reply 中最后一条 input message / 父 Session 的来路，用 llm_context::ai_message_to_msg_object_with_base_validated_async
   │          转成 MsgObject，写出站记录（与 Turn / run 状态同一次提交）
   ▼
出站记录 { key, msg: MsgObject }
   │ outbound bridge：原样发送
   ▼
msg-center
```

- 目标：`state.reply`（§3.2.6）保存最后一条已消费 input message 的原路回复目标，或父 Session 的来路；message 路径给出 `to`、`to_session`、`kind`、`thread.reply_to` 与 tunnel，`from` 是 Agent DID。不是只绑定开启 Turn 的那条消息；后续输入提交时更新，恢复时按 receipt 还原。`messages_to_send` 自带 target 时以它为准。
- 转换复用 `msg_parser` 现有出口：文本块合并为正文；文本中的 `<attachment path=… | obj_id=…/>` 标签与 `NamedObject` 块转成 `MsgContent.refs`（本地路径经 `LocalFileResolver` 登记为对象，`AttachmentValidator` 做路径与权限检查）；thinking、tool_use、tool_result、provider_state 不出站。
- 幂等：出站记录的 `key = (sid, turn, run_id, 序号)`；bridge 重发同一 key 不产生第二条消息。出入两个方向的线格式都是 MsgObject，Session 是唯一做 MsgObject ↔ AiMessage 转换的地方。
- headless 的子 session 不产生 MsgObject，结果交给父 session（xAgent §4.13）。出站通道（`channels.outbound`）与 UI session 一起后移，这里只约定结构与转换点。

### 3.5 实施清单

- [x] `protocol/input.rs`：`SessionInput / PostedInput / FetchedInput`、`SessionMsg / MsgDelivery`、构造 helper（`text_msg / attach / reply_to / PostedInput::msg`）、`AgentEvent`、`ControlCommand::Perceive`、`RejectReason`；删除 `InputKind::{Change, Perception}`、`Input::{change, perception}`、`HEADER_INTENT`、`HEADER_REPLY_TO`；生成 JSON Schema。
- [x] `channel/kmsg.rs`：`encode_input` / `decode_message` 按 §3.2.1 的 header 映射，增加 `schema` 校验；投递与消费共用 §3.2.5 的校验函数。
- [x] `protocol/config.rs`：删除 `InputSourceConfig::MsgCenter`；behavior 的 `input` 段增加 `media: reference | inline`（与 `input.mode` 并列，随 C7 冻结）。
- [x] `protocol/state.rs`：`pending_events`、`state.reply`；receipt 改为 `events[]` + `parts[]` 并保存提交后的 `reply`，删除 `changes[]`、`consumed_only[]`、`content`、`message_pos`；`OBSERVATION_HOOK` 删除。
- [x] 输入通道：Session 级 pending input 上限 64，原子容量检查与 append、`input_full`、消费后释放容量；旧协议 Session 迁移前只读。
- [x] Session 配置与内置 bridge：绑定用户时区并默认建立隐式 semi 订阅；协议与内建时间渲染统一 UTC。
- [x] `runner/assembler.rs`：`InputMaterial.inputs` 改为 `InputView`；内建 `input.text` 渲染（§3.3.4）；`render_input` 返回 `RenderedInput`。
- [x] `runner/live.rs::commit_input_batch`：注入 1–2 条多块 `AiMessage`，按 `parts[]` 记录位置。
- [x] `runner/history.rs` 与压缩路径：早于当前 Turn 的输入消息去掉媒体块。
- [x] `Cargo.toml`：libopendan 直接依赖 `ndn-lib`（workspace 已有，llm_context 已在用）。
- [x] `runner/assembler.rs`：`message_view`（MsgObject → `MessageView`，§3.3.3）；llm_context 先导出 `attachment_kind` / `attachment_mime`。
- [x] 模板命名格式与 filter（§3.3.8）：llm_context 的 `PromptRenderEngine` 注册 `render_format` 与底层通用 filter，提供宿主注册格式和追加 filter 的入口（llm_context 层，先行）；libopendan 注册 `input.xml`、`message.xml / message.markdown`、`event.xml / event.summary_text`、`attachments.xml / attachments.list`、`todo.summary_xml`。材料在渲染前装配，XML 格式内部统一转义；六个示例模板进 fixtures。
- [x] `bridge/msg.rs`：`route_msg_record`（§3.3.2，只过滤与分流）。
- [ ] （`--attach` 只接受 ObjId，本机文件登记未做，见 §10.3）CLI：`post --json <file | ->` 与 `post --msg … [--from] [--attach] [--reply-to]`，`--no-bridge` 下可单独验证接收方；调用者不需要了解 Rust 类型、闭包或进程内对象。
- [x] 手工投递样例与 fixtures：普通消息、带附件的群消息、active 任务事件、semi 对象变化、停止控制，以及 §3.2.5 每种拒绝原因各一条；每个样例给出所需的 Session 配置与预期行为；`input.text` 的渲染结果进 fixtures。
- [x] 反写：xAgent §4.2 / §4.3 / §4.5 / §6.2 / §9.5 与 C1、C2；Session Input Protocol 拆成“Agent 输入”与“Session 控制”两篇并升到版本 3。

### 3.6 待确认

1. **`input.media` 的默认值**：本文定为 `reference`（快照小、前缀稳定、不依赖模型能力）。旧 OpenDAN 的 UI session 实际是把图片 / 文档块直接交给模型，UI 类 behavior 是否默认 `inline`？
2. **`inline` 失败时是否机械降级**：模型不支持或对象不可读时，本文不自动重试。另一选项是 Session 去掉媒体块重试一次并在文本中注明。
3. **`edit` / `redact` 的投递**：本文作为普通消息原样投递（渲染出 `edit_of` / `redacts` 属性），由 LLM 理解；原消息已进入上下文的不回改历史。是否需要对“原消息尚未被消费”的情况做机械合并或删除？
4. **`reaction` 与 `ServiceDid` 引用**：首版不投递 / 不进入模板视图。是否需要以 Observe 事件或 `relations` 的形式呈现？
5. **说话人与 Agent 的关系**（owner / 联系人 / 陌生人 / 其它 Agent）：模板视图的说话人目前只有 `id / did / name`。是否要由 bridge 在 `delivery` 中给出关系字段供模板与策略使用，还是留给 Agent 用联系人工具查询？
6. **斜杠命令的授权范围**：本文限定为 session 驱动者或 Agent owner 发出的才转成 `control`，其它人发的按普通消息处理。群内其他成员是否可以 `/stop`？
7. **多输入源的归并顺序**：本文按 `channels.inputs` 的声明顺序。目前每个 session 只有一个 kmsg 队列，多源出现前不实现。
8. **prompt 中的消息 key 长度**：`<msg key>` 与 `reply_to` 现在是完整的 ObjId 字符串。是否在渲染时用批内短编号（并在 receipt 中保留对应关系）以节省 token？本文按完整 ObjId，保证跨批次引用稳定。
9. **非人类来源的 `from`**：子 session 发给父的消息、定时任务生成的消息，`from` 用所属 Agent 的 DID，来源 session 放 `to_session` 还是 `meta`？本文倾向 `meta.from_session`，随 Sub Session（C15）实施时定。
10. **自定义模板的转义（已定）**（§3.3.8）：默认推荐 `render_format: "格式名"`，XML 格式渲染器负责转义，模板直接插入完整片段；`xml / attr` 保留给直接拼装 XML 的高级用法。不引入默认二次转义或额外 `raw` 要求，`input.text` 与 system 模板的既有插值方式保持一致。

## 4. P0：统一 Session 的机械判断与输入处理

### 4.1 输入路由与控制

- [x] 用规则表说明 `type + subscription + waiting_for + Session 模板` 的处理结果，覆盖 drive 入口、空闲等待和运行中的检查点；渲染模板只改变正文，不改变输入路由和消费语义。
- [x] kevent 只通知 Runner 队列可能变化，重复通知和漏通知不影响已持久化输入的消费。是否运行 LLM 由可处理的受控输入或可恢复的 run 决定，不设独立的 wakeup 输入入口。
- [x] message 的业务意图交给 LLM；鉴权、去重、排序、暂存和终态拒绝仍按协议机械处理。普通自然语言消息不能被直接解释为停止控制命令：Session 不解析正文，斜杠命令只在 msg bridge 入队前按登记表与发送者身份转成 `control`（§3.3.2）。
- [x] event 按结构化字段机械匹配；`summary` 是给 LLM 的说明，不用于解析任务状态或推导等待条件。
- [x] 挂起调用的结果依赖与普通事件订阅分别登记和匹配。匹配 pending call 的任务通知只触发 resolver 查询，结果回填 ToolResult，不再重复注入普通事件；没有 pending call 时，按有效普通订阅处理，无订阅则丢弃。
- [x] 只接收匹配有效显式 / 隐式订阅的事件：active → Input，semi → Observe。未知、已删除或未匹配的订阅直接丢弃，提交消费位置并确认输入源，不进入上下文或 `pending_events`；timer、系统事件也遵循此规则。Session 模板负责预先登记所需订阅，不再提供“未订阅也投递”的默认策略。
- [x] 同一源的 subscribe / unsubscribe 与 event 按投递 index 生效；unsubscribe 清理该订阅尚未注入的状态，后续迟到事件直接丢弃。不能先应用整批订阅变更再重新解释排在它之前的事件。挂起调用自己的结果依赖不随普通 unsubscribe 删除。
- [x] 挂起工具期间，普通 msg / 无关 Input event 保留待处理，不能填补缺失 ToolResult 或隐式开启替代 run；控制命令继续可处理。stop、审批作废和 task 取消复用 llm_context TODO §3 / §4 / §9 第 7 项的已定规则，不再另设一套停止协议。
- [x] 挂起调用与事件的匹配按 `task_id` 相等：llm_context 的挂起记录只有 `task_id`（对 llm_context 不透明），`source.kind = task` 的 AgentEvent 以 `source.id` 携带同一个 `task_id`（§3.2.3）。事件只用来提前唤醒，结果以 `RunningTaskResolver` 的查询为准。
- [ ] （stop 已实施；心跳未做，见 §10.3）工具执行期间也要能处理 stop 并维持 activity 心跳。当前控制输入只在 `before_inference` 边界读取；Runner 需要一个与工具调用并行的监视任务，读到 stop 后调用 interrupt handle（依赖 llm_context TODO §3 的工具取消）。监视任务只查看控制输入，不确认 / 消费，也不写 state；控制由驱动者在下一个边界执行并提交（§5 单写者纪律）。

| 输入 / 情况 | 机械处理 | 对 LLM / Turn 的影响 |
|---|---|---|
| 可处理的 message | 校验、去重、组批 | 开启新 Turn 或并入未关闭 Turn |
| 匹配普通 active 订阅的 event | 解析 Input 策略、组批 | 在可推理时进入上下文；挂起工具期间按等待规则暂存 |
| semi / Observe event | 合并并持久化半订阅状态 | 在受控输入使用前渲染为半订阅快照；不独立生成输入批次或触发推理 |
| 未匹配有效订阅、也未匹配 pending call 的 event | 丢弃并提交消费位置，再确认输入源 | 不进入上下文，不保存在 `pending_events`，不开启 Turn |
| 匹配 pending call 的任务通知 | 重新查询权威状态、收集结果 | 满足快照所需结果后以 ToolResults 恢复原 run / Turn |
| control | 校验后由 Session 驱动者执行并提交 | 不作为 Agent 输入；停止等命令可改变 Session / Turn 状态 |

### 4.2 三类受控输入与 behavior cfg 模板

这里枚举的是 Session 向 LLM 注入新 user message 的时机；总线上的 Agent 输入类型仍只有 message 和 event。`control` 与“受控输入”没有从属关系。模板能看到的输入材料（`input.*`）、内建渲染格式和渲染结果如何落成 user AiMessage，见 §3.3.3–§3.3.5。

| 入口 / 模板 | 触发条件与渲染材料 | 边界 |
|---|---|---|
| `on_init` | Session 首次启动，根据配置、初始状态和当批可处理输入生成启动消息 | 不要求有外部输入；只在 Session bootstrap 时使用，不因新建或恢复 context 再执行 |
| `on_input` | 将选中的外部 message / Input event 渲染为输入消息 | 消费策略支持单条或组批；收到 control、Observe event 或队列通知本身不构成此入口 |
| `on_context_switch` | 根据交接状态生成目标 context 继续执行所需的消息，包括切换 behavior、进入子 context，以及经交接批次返回的子结果 | 延续当前 Turn；工具触发的子调用返回仍补齐原调用的 ToolResult，不另造 user message |

- [x] behavior cfg 的 system 模板独立命名为 `prompt.system`；`prompt.on_init` 专指启动输入。旧 behavior cfg 的 `prompt.on_init` 是 system 模板，实施时显式改名，不沿用同名异义。
- [x] 输入模板统一为 `prompt.on_init / prompt.on_input / prompt.on_context_switch`；现有 `on_wakeup` 改为 `on_input`，`on_behavior_switch` 改为 `on_context_switch`。普通快照恢复、上下文压缩和 ToolResults 回填不新增受控输入入口。
- [x] 附件是否以图片 / 文档块随文本注入由 behavior 的 `input.media`（`reference | inline`，默认 `reference`）决定，与模板无关（§3.3.4）。
- [x] 单条 / 组批作为消费策略配置，与渲染模板分开；模板接收本次已选中的输入集合。behavior cfg 的 `input.mode` 默认 Batch，延续当前组批方式；Single 从排序后的可处理 message / Input event 中总共选一条，Batch 在批次预算内选取多条，未选输入保留。交接时先完成目标 behavior 的冻结与校验，再读取它的策略与模板。
- [x] 输入装配 / 推理预算不足时，按 AICC 服务不可用的失败与重试路径处理，不另造输入淘汰或预算专用协议。未提交批次不消费、不确认；已提交到快照的批次保留 receipt，恢复时不重复注入。不因暂时无法推理而静默丢输入，也不把本条改成 llm_context 执行预算的重新定义。
- [x] 初始化或交接与外部输入同时可处理时，沿用 `on_init` → `on_context_switch` → `on_input` 的入口选择顺序；允许消费的外部输入并入该批，不重复生成 `on_input`。子 context 不消费调用方队列、未完成工具批次期间暂存输入的规则保持有效。

### 4.3 半订阅快照的装配与提交

统一术语为 **`semi_subscription_snapshot`（半订阅快照）**，模板字段为 `prompt.semi_subscription_snapshot`，装配函数为 `render_semi_subscription_snapshot`。“半订阅”描述变化的接收与使用方式，“快照”描述本次渲染所看到的当前状态。

它是三类受控输入共用的装配材料，不设置 `on_observation`、`observed_state` 或第四类输入 hook。已订阅的 Observe 事件持续合并到持久化的 `pending_events`；本次快照选取其中待注入的状态版本，包含默认半订阅的 Session 用户时区，并按策略包含活动 Session 集合变化，不逐条重放已被覆盖的变化事件。

```text
确定本次受控输入（on_init / on_input / on_context_switch）
  → 选取并渲染待注入的 semi_subscription_snapshot（没有则省略）
  → 注入快照 user message
  → 注入受控输入 user message
  → 将两部分正文与 receipt 作为同一输入批次持久化提交
  → 继续推理
```

- [x] 运行中的检查点可继续接收、合并并保存半订阅状态；仅到达检查点、完成工具调用或收到 Observe event，不独立注入快照。没有受控输入时继续保留状态，空闲时同样保留。
- [x] 将当前 `render_observation` / `hook = observation` 的独立注入路径收敛到上述装配流程；不把旧 `on_behavior_step_ob` 直接作为半订阅输入入口。工具结果的渲染仍属于 LLM Context 的执行协议。
- [x] 渲染与选取无消费副作用，不在 render / drain 阶段清理 `pending_events`。只有受控输入确实提交时，才按 receipt 清理本次实际注入的版本；渲染失败、批次未提交或预算不足而进入 AICC 不可用处理路径时保留状态。
- [x] 一个输入批次可以包含快照消息与受控输入消息，receipt 必须覆盖两部分正文的位置和实际注入的状态版本（`parts[]` 与 `events[]`，结构见 §3.2.6；behavior 模式下两部分并入同一个 Step 的 `next_user_message`，见 §3.3.5）。两部分一起恢复，不允许快照已消费而受控输入丢失；半订阅快照不独立计 Turn，批次是否开启 Turn 仍由受控输入提交时的 Session 状态决定。

## 5. P0：事件身份、合并和消费提交

- [x] 五种身份各司其职（字段定义见 §3.2）：逻辑输入去重 `key`、通道投递位置 `(src, index)`、事件来源 `source`、状态合并键 `(subscription_id, source)` 和来源版本 `seq`。同一逻辑输入重投沿用 key，不同变化使用不同 key。`pending_events` 的存储结构见 §3.2.6。
- [x] pending input 上限固定为 64（§3.2.1），容量满时拒绝 append，不增加无界积压机制。沿用有界 `recent_keys` 处理近期逻辑重投，选批时也按 key 去重；通道消费游标与 receipt 承担同一投递位置和已提交批次的恢复幂等。不承诺跨任意时间、跨协议版本的无限期逻辑去重。
- [x] 同一协议内，Observe 在尚未消费的同一来源状态上按 seq 合并，迟到旧版本不能覆盖仍保留的新版本；没有 seq 时按通道消费顺序更新，并用 key 区分本次状态，不用时间戳冒充来源版本。producer 对同一来源保持 seq 可比较，重启后不能保持时使用新来源身份。提交后不为低概率旧事件追加永久版本水位；状态已清理且超出近期去重窗口的事件不承诺历史版本过滤。协议升级是另一层边界：旧 Session 迁移前 readonly，不能继续消费新旧输入。
- [x] 不对所有 Input event 默认做快照合并；需要保留每次发生的事件按流处理，可覆盖的状态事件由协议声明合并语义。terminal 用于保留和注入优先级，不能代替权威任务状态判断。
- [x] `pending_events` 在空闲时保留；只在消费、取消订阅、明确拒绝或按规则覆盖后清除。它保存已订阅来源的待注入状态，不保存已被覆盖的历史事件。总线容量满按 §3.2.1 拒绝 append；注入预算不足按 AICC 服务不可用处理，保留尚未提交的状态。
- [x] receipt 记录本次实际消费的投递位置及事件版本。清理待观察状态时，按 `(subscription_id, source, key)` 精确匹配，存在 seq 时一并核对；latest / terminal 只清本批实际注入的记录，同一 key 只注入一次。处理 v7 期间收到 v8，提交 v7 不得删除 v8；seq 缺失也不能删除另一 key 的新事件。
- [x] 含半订阅快照的受控输入批次和结果补齐继续遵守已有顺序：正文与 receipt 同快照 → run 门槛 → state 提交 → 清门槛 → 确认输入源。禁止在可恢复提交前出队，也禁止结束时按来源键再次删除本轮期间新到达的事件。
- [x] 对仅更新待观察状态的投递，先提交该状态再确认输入源；区分“已接收保存”与“已注入上下文”，receipt 不声称 LLM 已看到尚未注入的变化。
- [x] 保持 lease 下单写者纪律；bridge 和生产者通过输入通道提交，由驱动者修改 Session state，避免多个异步写入覆盖彼此的磁盘状态。

## 6. P1：串行等待、并行后台任务与可靠恢复

这里的“串行 / 并行”指 Agent 的后续工作是否依赖后台任务。task 的存活与可查询能力由已有实现决定，不统一承诺跨进程存活：外部服务 task 经宿主 resolver 查询；进程内 task 重启后查不到可返回 Unknown，shell 能从执行目录读到 exit 时返回已有结果。同步 `shell` 被打断或崩溃后的结果沿用 llm_context TODO §3.2 的纪律：不追杀进程，恢复时按 runtime 给出“被打断”的结果，不自动重放。

本节依赖 [llm_context TODO](./llm-context-long-tool-todo.md) 的修改（§3 工具取消与时限、§4 挂起记录与 `RunningTaskResolver`、§5 `shell` 的执行模式与 task），它们已于 2026-10-02 实施；以下是宿主侧的接入。原设计的 `wait.source / wait.class`、`DeferredResolver`、job 与 run 的区分均已取消，本节按简化后的词汇书写。

本次 review 不重新设计 task 持久化、resolver 错误分类或 stop 状态机；以该文 §11 的完成记录和现有接口为准。仍需实施的是该文 §11.3 列出的 Session 宿主接入，不能把 llm_context 已完成等同于宿主已经接好。

### 6.1 串行等待：补齐未完成的工具调用

- [x] PendingTool 接入沿用已实现的 `PendingToolCall{call, task_id, until_ms}`，挂起关联与已收集结果以 run 快照为准；`waiting_for.refs` 只从中生成 task_id 列表，用于状态展示和 §4 的事件匹配，不另存一份 call / deadline 协议。
- [x] 删除未使用的 `pending_task_calls: Vec<Value>`（当前只有定义与默认值），不另定 schema；等待关联只认快照中的挂起记录与由它生成的 `waiting_for.refs`。
- [x] 宿主装配现有 `RunningTaskResolver`（`state / wait / cancel / can_resolve / watch / active`），复用组合 resolver 的进程内 task 与可选 buckyos task 路由；Session 自己的子 context、子 session、审批票据按需接入同一接口。不把所有 PendingTool 转换为 TaskMgr 任务，不在 Session 内另建任务执行器。xllm 接手时按当前状态立即回填，不等待。
- [x] 在 drive 恢复入口先对齐快照、提交现场和等待对象，再处理新输入。任务已终态就收集结果，仍在执行才继续等待；通知到达和兜底定时检查使用同一查询路径。
- [x] 收齐快照实际需要的结果后，使用 `ResumeFill::ToolResults` 恢复同一 run 和仍打开的 Turn；轮询未完成任务无需调用 LLM。到达 `until_ms` 而 task 仍未结束时，也按当时的状态回填。
- [x] 等待条件独立于 inbox 是否为空。所有等待出口、CLI 重入和常驻模式都能继续检查任务，不能只有“已经收到部分输入”的分支才做轮询。
- [x] 查询与接手直接复用已实现的判断：`can_resolve(task_id) = false` 时拒绝接手并保留现场；能解析时调用 `state / wait`，得到 `Unknown{reason}` 就用现有 `task_state_observation` 回填续跑，由 LLM 判断，不解析 reason 再派生一种错误协议。进程内 task 换进程后查不到不因此返回 RecoveryBlocked；不静默重建或重放任务。
- [x] 支持结果恢复和崩溃场景后再开启 `allow_deferred`。宿主缺少对应能力时明确拒绝 / 返回 RecoveryBlocked，保留原始现场。这里指已处于挂起态的快照；派发时宿主未开 deferred 而工具返回 Pending 的情况，按 llm_context TODO §4 在工具内等待（最长 30 分钟），不报 Internal 错误。
- [x] stop 复用已实现的平滑结束、必要时打断与配对结果路径；对当前 Turn 的可取消 task 传导取消、审批作废，之前 Turn 的 task 不受影响。宿主只负责控制提交与调用现有能力，不另定义一套 resolver 取消结果或停止竞态协议。

### 6.2 并行等待：工具已经返回，后台任务继续

- [x] 正常 ToolResult 返回任务引用后，Agent 可以继续其它工作；沿用 llm_context 的自动 watch。后续通知只按已登记的 active / semi 订阅处理，不走补齐旧 ToolResult 的路径，未订阅事件按 §4.1 丢弃。
- [x] Agent 在收到变化通知后通过工具重新读取任务状态，决定继续等待、处理结果或改方案。空闲时的定期检查由持久化 timer / 宿主调度提供机会，不能假定 LLM 自己持续轮询。
- [ ] （轮询查询已实施；reconcile 重做结束时不从 call_result 找回 task id，见 §10.3）task 提交与订阅建立之间可能错过终态通知，建立订阅后立即查询一次。恢复使用已持久化 call_result / 挂起记录中的 task_id 查询；`active()` 是当前 resolver 的展示列表，不是重启后的持久任务索引。查不到使用既有 Unknown 状态：有 pending call 时按 §6.1 回填；调用已经返回时只向后续推理呈现状态，不补写旧 ToolResult。不增加另一套恢复状态机。
- [x] 无输入队列时，串行等待由 resolver 恢复；后台 task 的完成检查由宿主调度 / 内置 bridge 提供。没有驱动或调度机会时，不承诺 Session 自行唤醒；已 finished 的 Session 不因 task 完成而重开。
- [x] 后台 task（llm_context TODO §4 / §5：`shell` 的 auto 模式到期转 task，以及其它返回 task 的工具）的 Session 侧处理：
  - run 进行中，task 的状态由 llm_context 的 background env 呈现（`resolver.active()`，半自动订阅），不经过输入队列。
  - run 结束而 Session 尚未 finished 时，对本 context 已 watch、尚未结束且没有 pending call 等待的 task，由 Runner 内置 bridge 接管为隐式 active 订阅；已有显式订阅则沿用其 active / semi 策略。完成时合成 `AgentEvent{source: {kind: task, id: task_id}, terminal: true}`，走同一套已订阅事件处理，不依赖额外输入队列。恢复按上一条从已保存的 task_id 重新查询，不要求进程内 `active()` 自动恢复。被挂起调用等待的 task 只回填 ToolResult，避免重复注入。
  - Session stop 传导给当前 Turn 的 task（可取消的被取消，审批作废）；Session finished / discard 时 task 的清理见 llm_context TODO §9 第 4 项。

### 6.3 Task bridge 与任务创建的恢复关联

- [ ] （只有 `TaskState → AgentEvent` 的映射，未接 TaskMgr，见 §10.3）将 TaskMgr 当前 `phase / outcome` 与结果引用转换成稳定的宿主结果 / AgentEvent；映射在适配层完成，Session 不读取旧 `to_status`，不解析服务专有事件路径来推断工具依赖。
- [x] 通知只加速检查，Task / 状态 API 才是权威来源。原始 kevent 的丢失通过重新查询收敛；需要保留每次发生的事件必须有持久来源或可靠投递，不能仅靠弱通知承诺至少一次。
- [ ] （只落实了在途记录里的幂等键，见 §10.3）创建任务前持久化 dispatch intent；根据 Session / run / call 的稳定身份生成幂等键；创建后持久化 task 绑定。崩溃在创建和登记之间时，按该身份找回同一个任务。这条只针对经 TaskMgr 等外部服务创建的任务；`shell` 转成的进程内 task 不需要推导稳定身份，`task_id` 写在 call_result 里，崩溃后再查即可（llm_context TODO §5）。
- [x] 任务创建、结果收集、ToolResults 恢复及旧等待关联清理之间设定可恢复提交点；链式 PendingTool 不得被上一批的清理操作抹掉。
- [x] 首先用模拟任务服务 / 宿主验证上述协议，正式 TaskMgr bridge 作为后续接入项，不要求修改现有 OpenDAN 来完成验证。

## 7. P1：分层与并行推进的验收

- [x] 多个 Session 可由不同进程并行推进；每个 Session 仍只有一个持 lease 的驱动者、一个 active run。后台任务并行不要求同一 Session 同时启动多个 LLM run。
- [x] bridge 只转换来源和可靠投递；Session 判断消费与等待；LLM Context 推进推理 / 工具调用并给出 Outcome；Runtime 执行工具；Agent State 提供跨 Session 状态。Runner 复用现有组件，不把这些职责收回到一个大循环。
- [x] xagent 与 xllm 交接时遵守 Session / run 锁和提交门槛，不能同时推进同一 run；挂起结果的提供方及能力不足时的行为也需明确：接手方按自己的 resolver 能否解析挂起的 `task_id` 判断，解析不了才拒绝接手；xllm 接手时不等待，按当时的状态回填后续跑（llm_context TODO §4）。
- [x] 跨语言共享 schema、处理规则与 fixtures，包括 §3.2 的三种记录、§3.2.5 的拒绝原因和 §3.3.4 `input.text` 的逐字节渲染结果。先验证独立进程生产 JSON、Rust Runner 消费和恢复；TS Runner 后续用相同 fixture 验证，不要求本轮实现第二套 Runner。
- [x] xAgent 主文以 JSON 样例、规则表和恢复步骤解释设计，详细伪代码作为实现参考；主循环只呈现阶段边界，具体协议约定不隐藏在闭包或模板分支中。

## 8. 验收场景与实施顺序

以下是未来实施必须覆盖的场景，优先扩展现有 `tests/crash.rs`、Runner 测试和协议 fixtures；不是本次已经运行的验证。

| 场景 | 必须观察到的行为 |
|---|---|
| 独立进程手工投递 msg / event / control | schema 与规则决定处理方式，不依赖进程内句柄 |
| 用 helper / `post --msg --attach` 构造的消息，与 msg-center 来的同内容消息 | 都是合法 MsgObject，走同一条渲染路径，输出格式相同 |
| 手写记录的 `key` 与消息 ObjId 不一致，或 MsgObject 不合法 | 按 `invalid_envelope` / `invalid_payload` 拒绝 |
| 带附件、回复关系与 @ 的群消息经 msg bridge 进入（§3.3.7） | 总线记录、渲染文本与注入的 AiMessage 与 fixtures 一致；说话人是 `msg.from`，不是投递者；信封 `key` 等于消息 ObjId |
| 同一批输入分别用 `input.media = reference` 与 `inline` | 文本块相同；`inline` 多出按顺序排列的图片 / 文档块，receipt 的 `parts[].pos` 指向它们所在的消息 |
| 正文包含 `<msg>`、`</inputs>` 等标签文本或以 `/stop` 开头（非授权发送者） | 被转义后作为普通正文进入上下文，不改变结构，不被当作控制命令 |
| msg bridge 在投递成功、确认 msg-center 之前崩溃，在近期去重窗口内重投 | 重投的记录按 `key` 去重，上下文中只出现一次 |
| 输入提交后崩溃，恢复时总线记录已被清理 | 按快照中的 receipt 补齐 state，不重新渲染，消息内容不变 |
| 一个批次多条 message、同一 Turn 后续补充 message，之后崩溃恢复 | `state.reply` 始终指向最后一条已提交 message 的来路；receipt 补回路径，不依赖总线或渲染文本 |
| 只有 event / bootstrap 的批次，或子 Session 没有 message 来路 | 前者保留已有回复路径；后者按 parent session 来路回送；无任何来路时不产生默认消息回复 |
| §3.3.8 的六个示例模板渲染同一批输入 | 输出与 fixtures 一致；`input.xml` 与 `input.text`、`message.xml / event.xml` 与其中的对应元素逐字节相同；XML 格式内部正确转义正文与属性 |
| `session.current_todo` 为空或含标签文本；传入未知格式名或错误的对象形状 | `todo.summary_xml` 对空值输出空串，对文本正确转义；未知格式 / 错误形状报模板错误，输入保持未消费 |
| 附件清单分别使用 ObjId 与宿主已解析的可读路径 | 两者均满足 I4；去掉媒体块后仍能按文本定位，只有显示名时给出 warning |
| 不同时区的 Runner 渲染同一批输入；用户时区随后变化 | 内建时间都是相同 UTC 字符串；Session 绑定的用户时区默认半订阅，变化随下一次受控输入呈现，不独立唤醒 |
| 新 run 重建历史，之前的 Turn 含图片块 | 历史中只保留文本块，`<attachment>` 行仍在 |
| 同一事件投给 active 与 semi Session | 前者形成可推理输入，后者保存变化且空闲时保留 |
| 未订阅、已取消订阅或未知来源的事件到达 | 丢弃并提交消费位置、确认输入源，不进入上下文或 `pending_events`；timer / system 无兜底例外 |
| 同源 event A → unsubscribe → event B | A 按当时有效订阅处理，取消订阅清理未注入状态，B 丢弃；不因 fetch 分批方式改变生效顺序 |
| 64 条 pending input 后继续 append，或多个 producer 争抢最后一个名额 | 最多接受 64 条，第 65 条返回 `input_full`；不覆盖已有记录，bridge 不确认未投递的上游记录 |
| 队列有暂存普通输入，后面已入队 stop | 监视任务能在最多 64 条 pending 记录中发现 stop，按既有取消路径处理 |
| 输入装配 / 推理预算不足 | 走 AICC 服务不可用的失败与重试路径；未提交输入保留，已提交 receipt 的批次不重复注入 |
| 三类受控输入分别携带待注入半订阅状态 | 先出现半订阅快照消息，再出现对应的启动 / 外部输入 / 交接消息；正文与 receipt 同批提交 |
| run 中只有 semi 更新、工具完成或检查点回调 | 更新保存但不独立注入；直到下一次受控输入使用前才渲染快照 |
| on_input 单条与组批策略 | 模板只收到本次选中的输入；未选输入保留；control 与 Observe event 不被当作受控输入消费 |
| 启动 / 交接与外部输入同时到达 | 选择一个受控入口，允许消费的外部输入并入同批，无重复注入 |
| 普通恢复、压缩或工具触发的子 context 返回 | 不重复执行 on_init / on_context_switch；工具返回按 call_id 补齐 ToolResult |
| 非法输入 / 不支持版本（§3.2.5 的每种原因） | 按协议拒绝并记录原因；持久现场不被猜测性改写；累积确认不被卡住 |
| 队列 kevent 重复或丢失 | 已持久化输入仍可被轮询发现并正确消费 |
| 同一协议内 v8 仍在 pending，随后到 v7 | v7 不覆盖 v8；不据此承诺消费清理后的永久版本过滤 |
| 消费 v7 时 v8 到达 | 提交 v7 后，尚未注入的 v8 仍可被消费 |
| 无 seq 的 Observe 更新在前一状态提交前到达 | receipt 按 key 精确清理，前一状态的提交不删除新 key |
| 协议升级后打开旧 Session | 未迁移时 readonly，不能 append / drive；显式迁移成功后才能按新协议推进 |
| 快照 / state / 清门槛 / ack 各窗口崩溃 | 恢复后不漏输入、不重复注入，同一打开的 Turn 延续 |
| 可由 resolver 查询的持久 Task 在 Session 离线期间完成 | 无新通知也能查询终态并恢复；inbox 为空不阻止恢复 |
| 进程内 task 在重启后查不到，或接手方没有对应 resolver 能力 | 前者按 Unknown 回填续跑、不重放；后者按 `can_resolve = false` 拒绝接手并保留现场 |
| TaskMgr 的成功 / 失败 / 取消及重复通知 | 都按当前权威状态映射；结果与 call_id 精确关联 |
| 普通后台任务订阅没有 pending call | 通知正常进入 active / semi 路径，不按任务路径前缀丢弃 |
| 串行等待期间收到消息与停止控制 | 消息保留；控制按定义处理，不破坏挂起调用关联 |
| Task 已创建、绑定未提交时崩溃 | 按稳定幂等身份找回同一 Task，不创建重复任务 |
| ToolResults 恢复后再次 PendingTool | 新等待关联保留，旧结果不会重复补齐 |
| 两个 Session 并行、两个进程争抢同一 Session | 不同 Session 正常推进；同一 Session 仅一个驱动者成功 |
| 长 shell 执行中收到 stop | 工具被取消；快照含配对的 Cancelled 结果，恢复后不重跑 |
| 同步 shell 执行中 Runner 被 kill | 恢复不阻塞、不追杀进程，按 runtime 给出“被打断”的结果，不重放（llm_context TODO §3.2） |
| 串行等待 task 期间 kill -9 | 恢复先查询：Running 继续等待，Finished / Unknown 回填到同一 run / Turn；不承诺进程内 task 总能继续等待 |
| 并行 task：Turn 关闭但 Session 未 finished，task 在 Runner 重启后完成 | 按保存的 task_id 查询，不依赖旧进程的 active 列表；可查询的完成结果按已登记 active / semi 订阅处理；查不到按 Unknown 规则处理 |
| 经外部服务创建任务后、绑定持久化前崩溃 | 按稳定身份找回同一任务，不重复创建 |
| 挂起期间收到 stop | 按定稿规则取消等待并回填，run 以 Stopped 结束 |

建议实施顺序：

0. [llm_context TODO](./llm-context-long-tool-todo.md) 先行：其 §3–§5 已实施。本文新增的 llm_context 层改动有两项，同样先做：从 `msg_parser` 导出 `attachment_kind` / `attachment_mime`（§3.3.3）；`prompt_engine` 注册 `render_format` 与底层通用 filter，开放宿主注册格式和追加 filter 的入口（§3.3.8）。
1. 按 §3.5 实施总线数据结构与 Input 链路（§3.2 / §3.3 已定稿，§3.6 的待确认项不阻塞开工，按本文的选择实现），和 xAgent C1–C3 / C7 对齐；落实三类受控输入、system 模板命名及半订阅快照装配后，反写最终 Spec / Schema / fixtures。
2. 落实未订阅事件丢弃、64 条 pending input、预算不足的 AICC 不可用路径、同协议内的 pending 版本合并和按 key 精确消费；扩展 reply / receipt、UTC 与用户时区订阅、崩溃验证，确认跨进程并行纪律。
3. 在 llm_context TODO §4 / §5 的基础上，实现宿主结果查询、挂起关联和恢复路径；用模拟任务服务验证串行 / 并行两种语义，再开启 deferred。
4. 接入 xagent 手工投递和常驻验证，随后按计划接 TaskMgr 等正式 bridge；更新主文、实现计划与 fixtures。

实现阶段在 `src/` 下运行相应的 `cargo test -p libopendan -- --test-threads=1`；触及共享 waist / 宿主装配时增加对应 crate 的检查。schema 改动按 beta 2.2 breaking change 规则显式升版并拒绝旧版本，不增加旧 OpenDAN 格式的兼容代码。

## 9. 参考与入口

- [xAgent 设计](../doc/opendan/xAgent.md)：§4 输入、§9 Turn Loop、§11 C1–C3 / C8 / C12。
- [Agent Session SDK 实现计划](<../doc/opendan/Agent Session SDK 实现计划.md>)、[协议索引](../doc/opendan/protocol/README.md)、[Session Input Protocol](<../doc/opendan/protocol/Session Input Protocol.md>)。
- [长任务与可靠等待](<../doc/opendan/OpenDAN Long Task & Sub-Agent.md>)、[事件订阅语义](<../doc/opendan/Agent Session的事件订阅.md>)。
- 当前实现入口：`lib_opendan/src/protocol/{input,state}.rs`、`runner/{drive,hook,live,receipts,outcome,assembler}.rs`、`channel/` 与 `tests/`。
- Input 链路两端的现有类型：MsgObject v2（`cyfs-ndn/src/ndn-lib/src/msgobj.rs`，`CYFS 标准对象` §16）；`AiMessage / AiContent / ResourceRef`（`src/kernel/buckyos-api/src/aicc_client.rs`）；MsgObject 与 AiMessage 的现有转换及出口（`src/frame/llm_context/src/msg_parser.rs`）；AICC 资源解析（`src/frame/aicc/src/resource/`）。
- 旧 OpenDAN 的消息入口（只作对照）：`opendan/src/msg_center_pump.rs`（过滤规则、`from_name`、斜杠命令）、`session_model.rs::PendingInput`、`prompt_env.rs` 的 `od.msg/1` 渲染。
- 旧实现审查来源：`opendan/src/agent_session.rs` 的提前出队、按事件来源键清理、Task 事件分流与等待分支；`task_dispatch.rs` 的任务创建关联。只用于设计验证，不纳入改动范围。
- llm_context 层：[llm_context 长命令 / 长工具 TODO](./llm-context-long-tool-todo.md)（先于本 TODO 实施，含对 xAgent.md 的修改清单与待 review 事项）、[AgentRuntime 下移 TODO](./llm-context-agent-runtime-todo.md)。

## 10. 实施记录（2026-10-03）

按 §8 的顺序实施完成。验证：`cargo test -p libopendan -- --test-threads=1`（单元 18 + 集成 108，另有 1 个 `--ignored` 的真实 kmsg 用例未跑）、`cargo test -p llm_context`（209）、`cargo build -p opendan -p agent_tool`。fixtures 已重新生成（13 个场景升到新 schema，新增 `14_input_bus`）。

### 10.1 落点

| 内容 | 位置 |
|---|---|
| llm_context 先行：`attachment_kind / attachment_mime` 导出；`render_format` + 通用 filter（`xml / attr / json / truncate / oneline / default / join / quote / time`）；宿主入口 `EngineConfig.extensions: RenderExtensions`（`with_format` / `with_filter`） | `llm_context/src/{msg_parser,prompt_engine,lib}.rs` |
| 总线数据结构（§3.2）：`SessionInput / PostedInput / FetchedInput / SessionMsg / MsgDelivery / AgentEvent / EventSource / ControlCommand::Perceive / RejectReason`、构造 helper、`parse_record`（投递与消费共用）、`parse_logical_record`、receipt 的 `events[] / parts[] / reply` | `lib_opendan/src/protocol/input.rs` |
| kmsg 映射（`schema` header，删除 `intent / reply_to`）；64 条 pending 上限与 `input_full`、旧 Session 只读（`session_readonly`） | `channel/kmsg.rs`、`state/registry.rs::post_input`（`post.lock`） |
| 配置：`prompt.system`、`on_init / on_input / on_context_switch / semi_subscription_snapshot`、`input{mode, media}`、`session.timezone`、订阅来源 `task / timer / system`、隐式订阅；删除 `InputSourceConfig::MsgCenter` | `protocol/config.rs`（`session_config/4`） |
| state：`pending_events`（`merge_pending_event / clear_pending_events`）、`reply`、`watched_tasks`、`inputs[src].accepted`；删除 `pending_task_calls` | `protocol/state.rs`（`session_state/5`） |
| 路由（§4.1、§5）：按投递顺序处理 control / event / msg，未订阅丢弃，pending call 的 task 通知只唤醒，拉模式 session 订阅合成事件 | `runner/inputs.rs` |
| 模板视图与内建格式（§3.3.3、§3.3.4、§3.3.8）：`InputView / MessageView / EventView`、`message_view`、八个命名格式、`media_blocks`、I4 检查 | `runner/input_view.rs` |
| 模板渲染与三类受控输入（§4.2）、半订阅快照装配（§4.3） | `runner/assembler.rs`、`runner/drive.rs::select_snapshot` |
| 落盘（§3.3.5）：1–2 条消息 + receipt 同快照；检查点不再注入 | `runner/live.rs::commit_input_batch`、`runner/hook.rs`、`runner/receipts.rs`、`runner/flush.rs` |
| 串行等待（§6.1）：`WaitingRun`、`try_fill`、`can_resolve` 拒绝接手、`Unknown` 回填、stop 时取消并回填、`allow_deferred` 开启 | `runner/live.rs`、`runner/drive.rs`、`runner/shared.rs`、`runner/tools.rs` |
| 并行等待（§6.2）：`watched_tasks` 接管与 `poll_watched_tasks` | `runner/outcome.rs`、`runner/drive.rs` |
| stop 监视任务（§4.1 最后一项） | `runner/drive.rs::StopMonitor`、`runner/inputs.rs::stop_queued` |
| bridge：`route_msg_record`、`task_event`、`dispatch_idempotency_key`、`outbound_base / outbound_key` | `lib_opendan/src/bridge/` |
| CLI：`post --json <file \| ->`、`post --msg … [--from] [--attach] [--reply-to]`、`post --stop` | `examples/session.rs` |
| 反写：Session Input Protocol（Agent 输入）、Session Control Protocol（新）、Session Directory Protocol、协议 README、xAgent §4.1–§4.7 / §6.2 / §9.5 / C1–C3 | `doc/opendan/` |

### 10.2 与本文不同或补充的实现决定

1. **`from_id`**：按 §3.3.3 的规则取 `msg.from.to_raw_host_name()`，`did:bns:bob` 得到 `bob.bns.did`（不是示例里原先写的 `bob`）；文中示例已改。
2. **拒绝原因**多一项 `input_policy`（Session 的 `input_policy = none` 时收到 msg / Input event）；`input_rejected` 增加可选的 `detail`（诊断文本，不属于协议判断）。
3. **未订阅 / pending call 事件**写新的 worklog 条目 `event_dropped{input, reason: unsubscribed | pending_call}`，取代 `change_dropped`。
4. **active 事件的接受状态**持久化在 `inputs[src].accepted`：保证 “A → unsubscribe → B” 的结果不随 fetch 分批变化（A 在被接受后即使一时未入批，也不因随后的 unsubscribe 改判）。
5. **拉模式 session 订阅总是 Observe**（与原行为一致）：它没有投递位置，合成事件并入 `pending_events`，`subscription_cursors` 只保留它的 rev 游标。活动 session 集合不再走半订阅游标：每个受控输入消息里现算完整列表。
6. **模板输出的空白**：只含块标签的行不产生输出行，整体去掉首尾空白（否则 `{% for %}` 会留下空行）；规则写进了 Session Input Protocol §6.3 并由 fixtures 固定。
7. **模板变量**：`session.{id, kind, objective, timezone, is_bootstrap, current_todo, background_hint_changed, default_changed_background_hint_text}`、`runtime.{status, clock_text}`、`handover`、内建块文本（`task_text / handover_text / perceptions_text / hints_text / active_sessions_text / runtime_text / builtin`）。libopendan 没有 todo 与背景提示的来源，`current_todo` 为 null、`background_hint_changed` 为 false，由持有这些状态的宿主（OpenDAN）装配。
8. **64 条上限的原子性**：容量检查与 append 在 `<session>/.opendan_agent_session/post.lock` 的临界区内，经 `SessionRegistry::post_input` 投递的 producer 都遵守。直接调用 `kmsg::post_to_queue` 的写入（或读不到 session 目录的跨主机 producer）不受限：kmsg 服务本身没有条件追加。
9. **等待 task 的 drive 返回**：run 挂起在 task 上时，`StopWhen::Idle / Finished` 都在本次 drive 内轮询到 `options.max_wait` 才返回 Idle（`waiting_for.kind = tool`），`MaxOutcomes` 立即返回。原因是进程内 task 随进程结束，立即返回会让它变成 Unknown。
10. **Idle 的含义**：一个 Turn 以等待输入结束时，如果队列里已有候选输入，`StopWhen::Idle` 继续处理而不是返回（原先会返回，留给下一次 drive）。
11. **回填后的输入暂存**：挂起调用刚被回填的 run 先跑完它的工具批次，期间不放入新的输入批次（与工具触发的子 context 返回同一规则）。
12. **session finished 时清空** `pending_events` 与 `watched_tasks`。
13. 机械渲染里 turn 行的 ` changes: …` 改为 ` events: …`（列出的是注入的半订阅状态 key）。

### 10.3 未完成 / 后移

- **stop 监视任务不维持 activity 心跳**：它被限定为只查看、不写 state；心跳仍只在检查点刷新，长工具执行期间不更新。
- **`--attach <本机文件>`**：CLI 只接受 ObjId（`--attach <obj_id>[=<name>]`）；经 `LocalFileResolver` 把本机文件登记进 NamedStore 需要宿主提供 resolver，开发 CLI 没有。
- **出站记录的持久化与发送**（§3.4）：只提供了信封构造与幂等 key（`bridge::outbound_base / outbound_key`），没有接到 Turn 提交与 `channels.outbound`（随 UI session 后移）。
- **msg-center / TaskMgr 的正式 bridge**：只有纯函数映射与模拟宿主验证（测试用假的 `RunningTaskResolver` 与宿主工具），没有接真实服务。外部任务的 dispatch intent 只落实到在途记录的 `idempotency_key`，没有工具使用它。
- **崩溃后重建 `watched_tasks`**：只在正常的 run 结束时从 resolver 的 `active()` 接管；由 reconcile 重做的结束（没有 resolver）不会从快照的 call_result 里找回 task id。
- **显式迁移工具**：旧 Session 只读已落实，迁移本身没有实现（本文 §1 明确本轮不做）。
- **TS 侧**：websdk 的同名构造 helper 与 TS Runner 未做；`14_input_bus` 可直接用于它的验证。
- **`on_context_switch` 携带半订阅快照**没有单独的端到端用例（`on_init`、`on_input` 有）；实现是同一条提交路径。
- §3.6 的待确认项 1–9 均按本文的选择实现，仍待确认。

# agent-did-object-lib 技术需求

Status: Draft v0.1

目标实现目录：`src/frame/agent-did-object-lib`

相关协议：`notepads/Agent元能力/Agent DID-Object Protocol Spec.md`

本文用于指导 code agent 实现 `src/frame/agent-did-object-lib`。这个 lib 是给 Agent Runtime 和独立 Agent 工具进程复用的对象访问骨架，不是 DID Object Protocol 的协议库。本文只描述当前需求和实现边界，不引入向下兼容要求；涉及 DID Object 的部分应遵守相关协议文档。

---

## 1. 背景与目标

Agent Runtime 需要一套统一对象访问骨架，把不同来源的对象都包装成 Agent / LLM 可消费的 `read`、`x-call` 和 subscription 能力。DID Object Protocol 是其中一种重要对象来源，协议文档已经把 DID Object 的核心边界收敛为：

```text
Gateway resolver input
  -> canonical Object URL
  -> DID Object Card
  -> DID Object Profile
  -> declared property / action / event endpoint
```

该协议是 closed-world、caller-neutral、declared capability only。它不定义 Agent 侧开放世界的 `read()`，也不把 `x-call` 当成协议名。

`agent-did-object-lib` 的职责是实现 Agent Runtime 侧的核心对象函数，并提供一个可在多个独立进程中 link 的统一骨架。很多 Agent 工具不是 OpenDAN 主进程的一部分，它们仍应通过同一个 lib 读取路由配置、解析 Object ID、选择 adapter、返回统一的 Agent-facing 结果。

```text
read(input, options)              -> Agent-facing read view
x_call(object, action, params)    -> Agent-facing x-call result
subscribe_event(object, event)    -> local KEvent bridge subscription
unsubscribe_event(subscription)   -> stop / release local bridge subscription
```

核心目标：

1. 实现 `read()`：把文件、本地/远端 Web 资源、Agent Runtime 对象、DID Object 等对象读取并直接返回合法 `AgentToolResult`。
2. 实现 `x_call()`：把 Agent 侧的 x-call 请求路由到对应 adapter，执行 action / method / command，并直接返回合法 `AgentToolResult`。
3. 实现 subscription：把对象事件订阅路由到对应 adapter，并把可唤醒 Agent Session 的事件桥接成本地 KEvent pattern。
4. 建立路由机制和路由配置，让不同 Object ID / Object pattern 能导向不同 adapter；这套配置必须可被 `x-call` 命令行单独使用。
5. 定义统一 `AgentObjectAdapter` trait，让 filesystem、web、agent_runtime、DID Object、本地 HTTP 扩展 adapter 共享同一接口。
6. 支持用户通过本地 HTTP Server（例如 Agent Docker 容器内 TypeScript server）实现进程外 adapter。

---

## 2. 非目标

本库不作为 DID Object Protocol 的基础协议库。协议类型和基础 resolver 应优先复用 `name-lib` / `name-client`，本库只在 Agent Runtime 侧消费这些能力。

本库不实现 DID Object Provider / Host，不负责暴露 `did.json`、`profile.json`、`props/*`、`methods/*` 或 `events` endpoint。

本库不重新定义 DID Object Protocol，不改变 `DIDObjectCard`、`ObjectProfile`、`ActionInvocation`、`ActionResponse`、`EventSubscribeRequest`、`EventFrame` 等协议结构。

本库不负责完整 DID Document 信任链、RBAC、confirm token、quota、审计的最终实现。它必须保留扩展点，并在调用 provider 前做基础 declared capability 检查；provider 仍必须重新校验。

本库不把 Object Event 持久化为真相源。KEvent 是本地 fanout / wakeup 通道，不是远端 event 的 durable delivery 存储。

本库不要求 Event 具备 MQ 语义。第一版所有 object event 都按 best-effort accelerator 处理，用于唤醒 session、提示 cache invalidation 和触发后续 `read()` 刷新；不要求 adapter 实现 ack、backlog、重投、顺序一致性或跨重启补齐。若未来需要可靠事件，应作为单独的 durable event / MQ 能力设计，而不是当前 Event Bridge 的默认语义。

本库第一版不实现新的 Agent Tool CLI。它必须提供可被 `x-call` / `agent_tool` / `opendan` 以及其他独立工具进程调用的 Rust API 和配置加载能力。

---

## 3. 现有依赖与新增依赖约束

当前 workspace 已有可复用依赖：

| 依赖 | 用途 |
|---|---|
| `name-lib` | DID Object Card / Profile / Action / Event 协议类型。 |
| `name-client` | DID Object resolver、property read、action invoke 框架。 |
| `buckyos-api` | Runtime、KEvent client。 |
| `reqwest` | HTTP client，可用于 web adapter 和本地 HTTP adapter。 |
| `url` | URL 解析和 route match。 |
| `serde` / `serde_json` / `toml` | 配置和 JSON payload。 |
| `async-trait` / `tokio` | async trait 和后台 bridge task。 |
| `thiserror` / `anyhow` | 错误类型。 |

当前 workspace 没有明显可直接复用的 WebSocket client crate。实现 event bridge 时：

- MUST 先把 WebSocket transport 抽象成接口。
- 如果实现需要新增 `tokio-tungstenite` 或同类依赖，code agent MUST 先向用户确认，不能静默新增依赖。
- 在依赖未确认前，可以用 mock / fake transport 完成单元测试和桥接状态机测试。

---

## 4. Crate 结构

实现应新增 Rust library crate：

```text
src/frame/agent-did-object-lib/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── error.rs
    ├── types.rs
    ├── config.rs
    ├── router.rs
    ├── runtime.rs
    ├── adapters/
    │   ├── mod.rs
    │   ├── filesystem.rs
    │   ├── web.rs
    │   ├── agent_runtime.rs
    │   ├── did_object.rs
    │   └── local_http.rs
    └── event_bridge.rs
```

`src/Cargo.toml` 需要把 `./frame/agent-did-object-lib` 加入 workspace members。

推荐 package name 为 `agent-did-object-lib`，crate import name 自动为 `agent_did_object_lib`。

---

## 5. 核心 API

库对外提供一个 Runtime 入口：

```rust
pub struct AgentDIDObjectRuntime { ... }

impl AgentDIDObjectRuntime {
    pub fn new(config: ObjectRouteConfig) -> Result<Self, AgentDIDObjectError>;
    pub fn with_kevent_client(self, client: KEventClient) -> Self;

    pub async fn read(&self, input: ReadInput) -> Result<AgentToolResult, AgentDIDObjectError>;
    pub async fn x_call(&self, input: XCallInput) -> Result<AgentToolResult, AgentDIDObjectError>;

    pub async fn subscribe_event(
        &self,
        input: SubscribeEventInput,
    ) -> Result<EventBridgeSubscription, AgentDIDObjectError>;

    pub async fn unsubscribe_event(
        &self,
        input: UnsubscribeEventInput,
    ) -> Result<(), AgentDIDObjectError>;
}
```

API 约束：

- `AgentDIDObjectRuntime` MUST 只依赖传入的配置和可选 runtime client，不在构造时隐式扫描全局目录。
- `read()`、`x_call()`、`subscribe_event()` MUST 先走 router，再调用 adapter。
- `x-call` CLI 未来可以直接加载同一个 `ObjectRouteConfig` 并调用 `AgentDIDObjectRuntime::x_call()`。
- Runtime MUST 支持内存态 adapter registry，方便单元测试注入 fake adapter。

---

## 6. 统一类型

### 6.1 Object ID

Object ID 是路由输入，不等同于协议 wire endpoint。进入 `agent-did-object-lib` 之前，Gateway 必须先把 DID、alias、本地路径等友好表达式解析成 canonical Object URL；本库只消费 URL。

支持输入：

| 输入形态 | 说明 |
|---|---|
| `https://...` / `http://...` | DID Object Protocol 的主 Object URL。 |
| `file://...` / `agent://...` / `obj://...` | BuckyOS 内部 adapter 可消费的 canonical URL。 |

建议类型：

```rust
pub struct ObjectRef {
    pub raw: String,
    pub normalized: String,
}
```

`ObjectRef::parse()` MUST 拒绝非 URL、DID URI、alias 和普通本地路径。这些转换属于 Gateway 第一层职责，不能下沉到 router 或 adapter。

### 6.2 AgentToolResult 返回契约

`read()` 和 `x_call()` 的公开返回值 MUST 是 `doc/agent_tool/agent_tool_result_protocol.md` 定义的 `AgentToolResult`，CLI 不应再对结果做二次拼接。

最低要求：

- `agent_tool_protocol = "1"`。
- `status = success|error|pending`。
- `cmd_name` / `cmd_args` 保留原始调用表达。
- `title` 是一行压缩结果。
- `summary` 是多行压缩结果。
- 主返回体只填 `output` 或 `detail` 之一，除非二者承载不同信息且文档明确说明。

`read()` 是文本型 Agent Tool，默认使用 `output` 放完整文本返回体。所有 Read 的最终主返回体都必须是文本；即使底层 Adapt 读到结构化 JSON，也应由 read 核心流程渲染成文本 output。内部结构化调试信息、route trace、adapt trace、trust trace 可以放在 `detail` 中，但不能和 `output` 重复同一份主内容。第一版建议 `read()` 默认只填 `output`，把结构化信息拼进文本返回体中，避免 CLI 再做渲染。

`x_call()` 是结构化 Agent Tool，默认使用 object `detail` 放 action result / error / meta；如果 action 结果天然是终端文本，也可以使用 `output`。

实现时 SHOULD 复用 `agent_tool` crate 中已有的 `AgentToolResult` 类型；如果依赖方向导致不能直接复用，也必须定义字段兼容的 serializable mirror type，JSON 输出必须满足 `agent_tool_result_protocol.md`。

### 6.3 LLM-friendly 文本渲染

Read 的 `output` 是给 LLM 看的，不是给机器 read-back 的稳定业务 schema。实现时不要为了机器可逆解析，把主输出设计成大段 JSON、XML 或充满转义符的格式。

本库 SHOULD 提供基础渲染工具：

```rust
pub fn render_json_for_llm(value: &serde_json::Value, options: LlmRenderOptions) -> String;
pub fn render_xml_for_llm(input: &str, options: LlmRenderOptions) -> String;
pub fn render_kv_for_llm(items: impl IntoIterator<Item = (String, String)>) -> String;
```

渲染目标：

- 优先生成 Markdown-like 文本：标题、短段落、列表、表格、fenced block。
- JSON object 可以渲染为 section + key/value list；array 可以渲染为 bullet list 或表格。
- XML / HTML 应抽取可读文本和关键属性，不追求保留完整标签结构。
- 不要求机器再 parse 回原结构。
- 避免 JSON-in-string、过度反斜杠转义、为了 XML 合法性引入的大量实体转义。
- 只在必要时使用 fenced code block，例如展示原始代码、日志、配置片段。
- 如果原始内容本身就是 JSON / XML 且用户明确要求原文，才保留原文格式。

这些渲染工具是 read 拼接层和 adapters 的基础库，目的是减少每个 adapter 重复实现“结构化数据转 LLM 友好文本”的逻辑。

### 6.4 Read 输入与内部片段

`read()` 是 Agent-facing rendering contract，不是 DID Object 协议 contract。它的公开返回是 `AgentToolResult`，但内部由多个 Adapt 候选片段拼接得到。

```rust
pub struct ReadInput {
    pub object: String,
    pub purpose: Option<String>,
    pub session_id: Option<String>,
    pub content_only: bool,
    pub range: Option<ReadLineRange>,
    pub max_tokens: Option<usize>,
    pub options: serde_json::Value,
}

pub struct ReadLineRange {
    pub offset: usize,
    pub limit: Option<usize>,
}
```

`offset` 和 `limit` 都是文本行号语义，只作用于最终 content 段，不作用于 meta、prompt guidance、trust guidance 或 error guidance。`offset` 建议使用 1-based 行号；实现时必须在 CLI help 中明确。

Adapt 内部返回建议类型：

```rust
pub struct AdaptReadResponse {
    pub object: String,
    pub object_did: Option<String>,
    pub content: Option<String>,
    pub meta: ReadMeta,
    pub prompt_guidance: Vec<PromptGuidance>,
    pub trust_guidance: Vec<TrustGuidance>,
    pub errors: Vec<ReadAttachedError>,
    pub cache_key: Option<String>,
    pub version: Option<String>,
    pub route: RouteTrace,
    pub adapt_meta: serde_json::Value,
}
```

Adapt SHOULD 尽量无状态。面向 session 的缓存、去重、unchanged 判断、content 大小管理都由 read 核心拼接流程完成，不放在 Adapt 内部。

### 6.5 XCall

```rust
pub struct XCallInput {
    pub object: String,
    pub action: String,
    pub params: serde_json::Value,
    pub session_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub confirm_token: Option<String>,
    pub trace_id: Option<String>,
}
```

`x_call()` 公开返回 `AgentToolResult`。对 DID Object adapter 来说，`detail` 通常来自协议层 `ActionResponse` 映射；对 filesystem / web / agent_runtime / local_http adapter 来说，`detail` 来自各自 adapter 的 action result。provider action error 应映射为 `AgentToolResult.status = "error"` 和结构化 `detail.error`，除非 HTTP / protocol envelope 本身不可解析，才返回 Rust error。

### 6.6 Event

```rust
pub struct SubscribeEventInput {
    pub object: String,
    pub event: String,
    pub filter: serde_json::Value,
    pub session_id: Option<String>,
    pub ttl_ms: Option<u64>,
    pub cursor: Option<String>,
    pub trace_id: Option<String>,
}

pub struct EventBridgeSubscription {
    pub subscription_id: String,
    pub object: String,
    pub object_did: Option<String>,
    pub event: String,
    pub kevent_pattern: String,
    pub expires_at: Option<String>,
    pub cursor: Option<String>,
    pub route: RouteTrace,
}
```

`kevent_pattern` 是返回给 OpenDAN / Agent Session 的本地订阅 pattern。payload 内必须保留原始 `EventFrame`。

---

## 7. 路由配置

路由配置是本库的核心。它必须能独立被 `x-call` 命令行加载，所以不能依赖 OpenDAN session 内部状态。

配置格式使用 TOML。第一版 schema：

```toml
version = 1

[[adapters]]
id = "filesystem"
type = "filesystem"

[[adapters]]
id = "web"
type = "web"

[[adapters]]
id = "agent-runtime"
type = "agent_runtime"

[[adapters]]
id = "local-ts"
type = "local_http"
endpoint = "http://127.0.0.1:8787"
auth_token_env = "AGENT_DID_OBJECT_ADAPTER_TOKEN"

[[adapters]]
id = "did-object"
type = "did_object"

[[routes]]
id = "file-url"
priority = 120
match_type = "scheme"
pattern = "file"
adapter = "filesystem"
methods = ["read"]

[[routes]]
id = "local-obj"
priority = 100
match_type = "scheme"
pattern = "obj"
adapter = "local-ts"

[[routes]]
id = "agent-runtime-objects"
priority = 95
match_type = "scheme"
pattern = "agent"
adapter = "agent-runtime"

[[routes]]
id = "did-object-camera"
priority = 90
match_type = "url_prefix"
pattern = "https://myhome.com/devices/"
adapter = "did-object"

[[routes]]
id = "default-web"
priority = 0
match_type = "scheme"
pattern = "https"
adapter = "web"
```

### 7.1 Adapter 配置

```rust
pub struct AdapterConfig {
    pub id: String,
    pub adapter_type: AdapterType,
    pub endpoint: Option<String>,
    pub auth_token_env: Option<String>,
    pub options: serde_json::Value,
}

pub enum AdapterType {
    Filesystem,
    Web,
    AgentRuntime,
    DidObject,
    LocalHttp,
}
```

不支持未知 adapter type。未来如果要支持新的 adapter type，必须在代码中显式注册，不能靠配置任意执行程序。

### 7.2 Route 配置

```rust
pub struct ObjectRoute {
    pub id: String,
    pub priority: i32,
    pub match_type: RouteMatchType,
    pub pattern: String,
    pub adapter: String,
    pub methods: Vec<RouteMethod>,
    pub options: serde_json::Value,
}
```

`methods` 缺省表示 `read`、`x_call`、`subscribe_event` 都允许。也可以显式限制：

```toml
methods = ["read", "x_call"]
```

支持的 `match_type`：

| match_type | 规则 |
|---|---|
| `exact` | `normalized == pattern`。 |
| `url_prefix` | URL 以 `pattern` 开头。 |
| `scheme` | URL scheme 等于 `pattern`。 |
| `glob` | 仅支持末尾 `*` 的简单前缀 glob，不引入复杂 glob 依赖。 |

路由选择规则：

1. 先解析 `ObjectRef`，得到 canonical URL `normalized`。
2. 过滤不支持当前 method 的 route。
3. 按 `priority` 从高到低排序。
4. 同优先级保持配置文件顺序。
5. 第一个匹配 route 胜出。
6. 找不到 route 时返回 `RouteNotFound`，不隐式访问网络。

### 7.3 配置加载

库提供：

```rust
impl ObjectRouteConfig {
    pub fn from_toml_str(input: &str) -> Result<Self, AgentDIDObjectError>;
    pub async fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, AgentDIDObjectError>;
    pub fn validate(&self) -> Result<(), AgentDIDObjectError>;
}
```

校验项：

- `version == 1`。
- adapter id 唯一。
- route id 唯一。
- route 引用的 adapter 存在。
- `local_http.endpoint` 只允许 `http://127.0.0.1`、`http://localhost` 或配置显式允许的容器内私有 host。
- `priority` 可重复，但同优先级必须按配置顺序稳定。

---

## 8. AgentObjectAdapter Trait

所有 adapter 必须实现统一 trait：

```rust
#[async_trait]
pub trait AgentObjectAdapter: Send + Sync {
    fn id(&self) -> &str;

    async fn read(
        &self,
        req: AdapterReadRequest,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError>;

    async fn x_call(
        &self,
        req: AdapterXCallRequest,
    ) -> Result<AdapterXCallResponse, AgentDIDObjectError>;

    async fn subscribe_event(
        &self,
        req: AdapterSubscribeEventRequest,
    ) -> Result<AdapterEventSubscription, AgentDIDObjectError>;

    async fn unsubscribe_event(
        &self,
        req: AdapterUnsubscribeEventRequest,
    ) -> Result<(), AgentDIDObjectError>;
}
```

Adapter request 必须包含：

- 原始 `ObjectRef`。
- 命中的 `ObjectRoute`。
- session / trace / options。
- 调用 method。

Adapter response 必须包含：

- Agent-facing output 所需字段。
- `adapt_meta`。
- route trace。

Adapter 不应直接读取全局配置。所有配置都通过 `AdapterConfig` 和 request 传入。

---

## 9. Read 核心流程：Adapt 拼接

`read()` 的核心不是单个 adapter 调用，而是把一个或多个 Adapt 的返回拼接成最终 `AgentToolResult`。Adapt 负责尽力读取对象并返回完整 content / meta / guidance 片段；read 核心负责 route、调度、合并、range、session 缓存去重、可信性说明和错误附加。

### 9.1 Adapt 调度策略

路由命中后可以得到一个或多个 Adapt 候选。调度策略由 route options 显式声明：

| strategy | 语义 |
|---|---|
| `first_success` | 谁先返回可用 content 就采用谁；其他未完成 Adapt 可以取消。 |
| `priority_first` | 按 route / adapt priority 顺序尝试，第一个成功结果胜出。 |
| `merge_all` | 等待所有可用 Adapt，合并 content / meta / guidance / error。 |
| `best_effort_merge` | 在超时内收集所有已返回片段，允许带错误返回部分内容。 |

第一版 MUST 支持 `priority_first` 和 `best_effort_merge`。`first_success` 和 `merge_all` 可以作为 route option 预留。

### 9.2 拼接伪代码

```rust
async fn read(input: ReadInput) -> Result<AgentToolResult, AgentDIDObjectError> {
    let cmd_name = "read";
    let cmd_args = render_read_cmd_args(&input);

    let object_ref = ObjectRef::parse(&input.object)?;
    let route_plan = router.plan(RouteMethod::Read, &object_ref, &input)?;
    let session_state = session_cache.lookup(input.session_id.as_deref(), &object_ref);

    let mut adapt_results = Vec::new();
    let mut attached_errors = Vec::new();

    for adapt in route_plan.schedule() {
        match adapt.read(AdapterReadRequest::from(&input, &object_ref, &route_plan)).await {
            Ok(result) => adapt_results.push(result),
            Err(err) if route_plan.best_effort => {
                attached_errors.push(ReadAttachedError::from(err));
            }
            Err(err) => return Ok(agent_tool_error(cmd_name, cmd_args, err)),
        }

        if route_plan.strategy == ReadStrategy::PriorityFirst && has_usable_content(&adapt_results) {
            break;
        }
    }

    if adapt_results.is_empty() && attached_errors.is_empty() {
        return Ok(agent_tool_error(cmd_name, cmd_args, RouteNotFoundOrNoContent));
    }

    let merged = merge_adapt_results(adapt_results, attached_errors, route_plan.merge_policy);
    let unchanged = session_state.is_same_version(&merged.cache_key, &merged.version);

    let mut content = if unchanged {
        None
    } else {
        merged.content
    };

    if let Some(range) = input.range {
        content = content.map(|text| apply_line_range(text, range));
    }

    let content = content.map(|text| enforce_content_budget(text, input.max_tokens));

    let output = if input.content_only {
        render_content_only(content)
    } else {
        render_read_sections(ReadRenderInput {
            content,
            meta: merged.meta,
            prompt_guidance: merged.prompt_guidance,
            unchanged,
            trust_guidance: merged.trust_guidance,
            errors: merged.errors,
        })
    };

    session_cache.update_after_render(input.session_id.as_deref(), &merged);

    Ok(AgentToolResult {
        agent_tool_protocol: "1",
        status: merged.status(),
        cmd_name,
        cmd_args,
        title: render_read_title(&input, &merged),
        summary: render_read_summary(&input, &merged),
        output: Some(output),
        detail: None,
        return_code: Some(if merged.has_fatal_error() { 1 } else { 0 }),
        ..Default::default()
    })
}
```

关键规则：

- Adapt 返回的 `content` 应是完整 content，不按 session 裁剪，也不因为 session 已读而省略。
- `content_only = true` 时，最终 `output` 只包含 content 段；如果 content 因 unchanged 被省略，`output` 可以为空字符串，但 `title/summary/status` 仍按 `AgentToolResult` 填写。
- session 级缓存 / 去重只发生在 read 拼接层。Adapt 不维护面向 session 的 read cache。
- `range.offset` / `range.limit` 只裁剪 content 段，且基于文本行；不会裁剪 meta、guidance、trust 或 error。
- content 大小管理由 read 拼接层根据 session / token budget 决定，不在 Adapt 内部完成。

### 9.3 非 content-only 的五段输出

`content_only = false` 时，`AgentToolResult.output` 按固定顺序拼接文本段。空段可以省略。

#### 第一段：Content

Content 是对象内容本身。读取文本文件时就是文本正文；读取 Web 页面时是抽取后的正文；读取 DID Object 时是对象当前可读状态或 adapter 选择的对象摘要。

如果 session 判断同一对象同一版本已经读过，Content 段 MUST 省略，不能把 Content 替换成“未修改”文字，避免 LLM 把“未修改”误认为对象原文。

#### 第二段：Meta

Meta 是对象元数据。即使读取普通文本文件，也应尽量包含文件大小、mtime / ctime、owner / creator、content type、版本或 hash 等信息。

Meta 是对对象的说明，不是内容正文。Meta 不受 range 影响。

#### 第三段：提示词引导

提示词引导是 adapter 给 LLM 的使用提示，属于可选段。

规则：

- Adapter 可以基于 Object Card / runtime object metadata 提供关键方法指引。
- 默认 read 不强制返回完整 Object Card 或完整 Profile。
- Adapter 可以只描述对象拥有的部分关键方法，尤其是与对象当前状态相关的方法。
- 完整 Profile 属于逐步披露内容。DID Object adapter SHOULD 提示：如果需要完整 Profile，可以通过对象 ID 加 `/profile` 后缀再 read。
- `unchanged` 是提示词引导字段，不属于 Content。它表示 session 里已有相同版本内容，本次省略 Content。

示例段：

```text
Guidance:
- This object supports x-call action `reserve` for tentative booking.
- Full profile is available by reading `<object-id>-profile`.
- unchanged: content omitted because the same version was already read in this session.
```

#### 第四段：可信性说明

可信性说明是渐进披露的短说明，目标是用一两句话描述可信链路，而不是输出完整审计报告。

至少考虑三个信用点：

1. 原始作者：Content 的最初创建者是谁，是否可验证。
2. 发布服务器：Content 由哪个服务器或对象 host 发布，服务器 / owner 的声明是否可信。
3. 传输链路 / Adapt：数据通过什么链路和 Adapt 获取，链路是否可能被篡改，Adapt 的实现者是否可信。

说明维度：

- 链路可信：HTTPS、本地文件、Runtime direct、DID verified、HTTP unverified 等。
- 内容可信：Object ID 是否是 hash，hash 校验是否匹配。
- 声明信息可信：Meta / Owner / author 是否有数字签名或 DID 信任链支持。
- 中转与信源信用：是否经过 local_http adapter、web adapter、agent_runtime adapter 等中转；每一层的作者 / 发布者信用如何。

示例：

```text
Trust:
- Content was read from https://example.com over HTTPS; transport integrity is protected, but author identity is only the website's claim.
- The local HTTP adapter `local-ts` transformed the content; adapter author is configured as local user code and is not independently verified.
```

#### 第五段：错误引导

错误引导体现 Adapt best-effort return 原则。错误不必然使前面内容作废。

场景：

- Adapt 操作本身错误，但其他 Adapt 返回了可用片段。
- 没有 Adapt 能找到完整信息，但有碎片信息可返回。
- read 拼接层发现缓存可能过期，但仍决定返回旧缓存作为参考。
- 用户对实时性要求很高时，错误引导应提示结果可能不可用。

错误信息应附加在结果末尾，并明确它影响的是哪一段、哪个 Adapt 或哪个缓存判断。

示例：

```text
Errors:
- web adapter timed out after 3s; filesystem cache was used instead.
- cached content may be stale because remote ETag check failed.
```

### 9.4 Adapt 无状态原则

Adapt SHOULD 更接近纯函数：

```text
(ObjectRef, ReadInput, RouteContext) -> AdaptReadResponse
```

它可以维护连接池、HTTP client、短期 resolver cache，但不维护面向 session 的“读过什么”“是否 unchanged”“如何裁剪 token”等状态。

Session 级状态统一由 read 拼接层管理：

- 同对象同版本去重。
- content unchanged 判断。
- content range 和 token budget。
- content cache hit / stale / conflict。
- guidance 中的 `unchanged` 指针。

---

## 10. 内置 Adapters

本库第一版至少应提供下列内置 adapters：

| Adapter | 主要用途 | read | x-call | subscription |
|---|---|---:|---:|---:|
| `filesystem` | 读取本地 workspace / file path；当前已有 `read` 能力主要属于这一类。 | MUST | MAY | MAY |
| `web` | 读取传统 Web URL / HTTP 资源，并翻译成 LLM 友好的 read content。 | MUST | MAY | MAY |
| `agent_runtime` | 高效访问 OpenDAN Runtime 内部定义的对象。 | MUST | SHOULD | SHOULD |
| `did_object` | 消费 DID Object Protocol 的 Card / Profile / property / action / event。 | MUST | MUST | SHOULD |
| `local_http` | 用户扩展 adapter，通过本地 HTTP Server 进程外实现。 | MUST | MUST | SHOULD |

### 10.1 Filesystem Adapter

`FilesystemAdapter` 是当前 read 能力的基础 adapter。它负责把本地路径或 `file://` URL 读取成完整 content / meta 片段，交给 read 核心流程拼成 `AgentToolResult`。

支持输入：

- 绝对路径。
- route 允许的相对路径。
- `file://` URL。

要求：

- MUST 支持文本文件读取。
- MUST 对二进制文件返回 metadata / size / content_type 摘要，不应把二进制直接塞进 LLM-facing `summary`。
- MUST 支持 `max_tokens` / size limit，超限时返回摘要和 truncation metadata。
- MUST 在 `adapt_meta` 中标记 resolved path、content type、size、truncated。
- SHOULD 复用现有 `agent_tool` read 的路径权限、root 限制和文本截断策略；不要重新发明一套文件安全模型。
- 第一版 `x_call` 可以返回 `UnsupportedMethod`，除非后续明确需要文件对象 action。

### 10.2 Web Adapter

`WebAdapter` 负责传统 Web / HTTP 资源的 `read()`。它不是 DID Object adapter，也不要求目标 URL 暴露 DID Object Card。

要求：

- MUST 支持 `http://` 和 `https://` URL。
- MUST 使用 HTTP GET 获取内容。
- MUST 根据 content type 把 HTML / text / JSON 翻译成 LLM-friendly content / meta / guidance 片段。
- HTML 应抽取 title、主要文本、链接摘要；不要把原始 HTML 直接作为默认 summary。
- JSON 应通过 `render_json_for_llm()` 转成 Markdown-like content；不要默认把原始 JSON 作为 read `output`。
- 对重定向、状态码、content length、content type 写入 `adapt_meta`。
- `x_call` 第一版可以只支持配置显式声明的 web action；没有声明时返回 `UnsupportedMethod`。

### 10.3 Agent Runtime Adapter

`AgentRuntimeAdapter` 用于高效访问 OpenDAN Runtime 定义的一些对象，避免把本进程内或同 Zone 内已有对象绕到通用 Web / DID Object 路径。

候选对象包括：

- Agent session。
- workspace / worksession。
- Agent Notebook / Memory / Skill / Tool metadata。
- OpenDAN 内部可读状态对象。
- 未来 Runtime 暴露的对象 registry 条目。

要求：

- MUST 通过显式传入的 runtime handle / client 工作，不能在 library 内隐式创建 OpenDAN Runtime。
- MUST 支持 `read()` 返回 Runtime 对象的 Agent-facing 摘要和结构化状态。
- SHOULD 支持 `x_call()` 调用 Runtime 对象声明的轻量 action。
- SHOULD 支持 subscription，把 Runtime 内部事件映射为 KEvent pattern。
- 如果独立工具进程没有 runtime handle，必须返回明确 `UnsupportedObjectRef` 或 `AdapterUnavailable`，不能静默 fallback 到网络。

### 10.4 DID Object Adapter

`DidObjectProtocolAdapter` 是内置 adapter 之一，专门消费 DID Object Protocol。它不是整个 lib 的默认 adapter；默认 route 应由配置决定。

实现要求：

1. 复用 `name-client::DIDObjectClient` 做 Object URL resolve、property read、action invoke。
2. 复用 `name-lib` 中已有 `DIDObjectCard`、`ObjectProfile`、`ActionInvocation`、`ActionResponse`、`EventSubscribeRequest`、`EventFrame`。
3. `read()` 对 Object URL 的基础流程：

```text
resolve(Object URL)
  -> validate card/profile basic structure
  -> collect profile properties/actions/events/traits
  -> read small declared summary properties when options request it
  -> produce AdaptReadResponse
```

4. `x_call()` 流程：

```text
resolve(Object URL)
  -> check action exists in profile
  -> call DIDObjectClient::invoke_action_from_resolved()
  -> validate ActionResponse exactly one of result/error
  -> map ActionResponse meta to AgentToolResult
```

5. `subscribe_event()` 流程：

```text
resolve(Object URL)
  -> check event exists in profile
  -> resolve event endpoint
  -> delegate to ObjectEventBridge
  -> return KEvent pattern
```

当前 `name-client::DIDObjectClient` 主入口是 Object URL。DID / `obj://` 输入在第一版可通过 route 交给 `local_http` / `agent_runtime` adapter，或返回明确错误；不能假装已经完成 DID resolver route。

#### 10.4.1 read() 的最小渲染规则

`read()` 最小输出应包含：

- object URL。
- object DID。
- profile id / kind / traits。
- declared properties/actions/events。
- short summary。

如果 options 中声明读取 properties：

```json
{
  "properties": ["status", "display_name"]
}
```

adapter 应只读取这些 property。第一版不要自动读取所有 property，避免 token 和网络成本失控。

若 property 读取失败，`read()` 不应整体失败；应把失败写入 `AdaptReadResponse.errors` 或 `adapt_meta.property_errors`，并保留已成功读取的内容。

---

### 10.5 Local HTTP Adapter

本地 HTTP Adapter 用于用户扩展。典型场景：用户在 Agent 所在 Docker 容器里启动一个 TypeScript HTTP Server，实现对某类 Object ID 的 read / x-call / event bridge。

安全边界：

- 默认只允许 loopback endpoint。
- 不允许通过 route 配置启动进程。
- 不允许 adapter endpoint 指向公网，除非未来新增显式 policy。
- 如果配置了 `auth_token_env`，请求必须带 `Authorization: Bearer <token>`。

#### 10.5.1 HTTP contract

Adapter server 必须实现 JSON endpoint。这里的 endpoint 是内部 Adapt HTTP contract，不是最终 CLI contract；最终公开 `read()` / `x_call()` 仍由本库包装成 `AgentToolResult`。

```text
POST /adapter/read
POST /adapter/x-call
POST /adapter/events/subscribe
POST /adapter/events/unsubscribe
```

请求通用字段：

```json
{
  "protocol": "agent-did-object-adapter/1",
  "object": "obj://example/item/1",
  "route": {
    "id": "local-obj",
    "adapter": "local-ts"
  },
  "session_id": "optional",
  "trace_id": "optional",
  "options": {}
}
```

`/adapter/x-call` 额外字段：

```json
{
  "action": "reserve",
  "params": {},
  "idempotency_key": "optional",
  "confirm_token": "optional"
}
```

`/adapter/events/subscribe` 额外字段：

```json
{
  "event": "changed",
  "filter": {},
  "ttl_ms": 300000,
  "cursor": null
}
```

`/adapter/read` 响应应使用 `AdaptReadResponse` 的 JSON 形状。最小响应：

```json
{
  "object": "obj://example/item/1",
  "object_did": null,
  "content": "Item full content",
  "meta": {
    "title": "Item title",
    "content_type": "text/plain",
    "updated_at": "optional"
  },
  "prompt_guidance": [],
  "trust_guidance": [],
  "errors": [],
  "cache_key": "obj://example/item/1",
  "version": "optional",
  "route": {
    "id": "local-obj",
    "adapter": "local-ts"
  },
  "adapt_meta": {}
}
```

`/adapter/x-call` 响应可以使用结构化 action response，随后由本库映射为 `AgentToolResult.detail`。如果本地 HTTP server 已经返回合法 `AgentToolResult`，本库 MAY 直接透传，但必须保证 `cmd_name/cmd_args/title/summary/status` 与本次调用一致或由本库覆盖。

本地 HTTP Adapter 的 event subscription 有两种模式：

1. 返回已经桥接到 KEvent 的 `kevent_pattern`，由用户 adapter 自己负责 best-effort 发布 KEvent。
2. 返回 WebSocket event endpoint 信息，由本库 `ObjectEventBridge` 负责创建真实 subscription、维持连接并 best-effort 发布 KEvent。

第一版必须至少支持模式 1；模式 2 可复用 DID Object event bridge。

这两种模式都不要求本地 HTTP adapter 提供 MQ 语义。adapter 不需要维护可靠队列、ack 状态、补投 backlog 或跨进程重放；事件只用于加速 Runtime / Agent Session 重新读取对象状态。

---

## 11. Event 到 KEvent 的桥接

DID Object Protocol 的 event wire binding 是 WebSocket；BuckyOS 内部 Agent Session 使用 KEvent 作为 wakeup / fanout。`agent-did-object-lib` 需要桥接二者。

Event Bridge 的核心目标是在 Runtime 内创建和管理真实 subscription，并把收到的 EventFrame 发布为本地 KEvent。KEvent 只承担本地 wakeup / fanout，不是 MQ；EventFrame 丢失时，恢复路径是根据 `refresh_hints` / `invalidated_objects` 重新 `read()` 对象状态，而不是 replay 事件流。

### 11.0 Delivery 语义

第一版 Event Bridge 只要求 best-effort delivery：

- 不保证每个事件都送达。
- 不保证事件严格有序。
- 不要求 adapter 保存 backlog。
- 不要求 ack / retry-until-delivered。
- 不要求跨 Runtime 重启恢复未送达事件。
- cursor 只作为断线后尽力恢复和丢失检测 hint，不能被上层当作 MQ offset。

因此 adapter 只需要实现事件源或订阅入口，降低实现负担。可靠事件、持久订阅和 MQ-style delivery 如果需要，应作为后续独立能力建模。

### 11.1 启动条件

桥接器必须 lazy start：

- 未订阅对象事件时，不连接远端 WebSocket。
- 第一个订阅到来时，启动 bridge task。
- 多个相同 `(object, event, filter)` 订阅可以复用同一个远端 subscription。
- 最后一个本地订阅取消或过期后，停止远端 subscription 并关闭连接。

### 11.2 Bridge key

```rust
pub struct EventBridgeKey {
    pub adapter_id: String,
    pub object: String,
    pub event: String,
    pub filter_hash: String,
}
```

`filter_hash` 使用稳定 JSON 序列化后 hash。不得用原始 JSON 字符串作为 map key。

### 11.3 WebSocket lifecycle

bridge task 的目标流程：

```text
connect(event_endpoint)
  -> send EventSubscribeRequest { op: "subscribe", ... }
  -> receive EventSubscription
  -> publish local KEvent for each EventFrame
  -> renew before expires_at
  -> send unsubscribe on shutdown
```

必须支持的状态：

```rust
pub enum BridgeState {
    Connecting,
    Subscribing,
    Active,
    Renewing,
    Closing,
    Closed,
    Failed,
}
```

失败处理：

- connect / subscribe 失败：返回 subscribe error，不创建本地 subscription。
- active 后断线：进入重连，指数退避，上限 30s。
- cursor 可用时可以带 cursor 尽力 resume，但不承诺补齐所有断线期间事件。
- cursor 失效或检测到可能丢失事件时发布一次 KEvent，payload 标记 `cursor_expired` 并附带 `refresh_hints`，提示上层重新 `read()`。
- unsubscribe 失败不阻塞本地取消，但必须记录到 error / log。

### 11.4 KEvent event id

不要把 Object URL 原样塞入 KEvent event id。必须编码成合法 global path。

格式：

```text
/obj/<host>/<kind-or-objects>/<safe-id>/<event>
```

示例：

```text
/obj/booking.example.com/stay_offer/abc123/changed
/obj/myhome.com/devices/cam01/low_battery
```

编码规则：

- host 来自 Object URL host，转小写。
- path segment 只保留 `[A-Za-z0-9._-]`，其他字符替换为 `_`。
- 空 segment 跳过。
- event 同样按 segment 规则编码。
- 如果无法从 URL 得到稳定 path，使用 `by_hash/<sha256_short>`。

payload 必须包含原始 frame：

```json
{
  "source": "did-object-event-bridge",
  "object": "https://myhome.com/devices/cam01",
  "object_did": "did:web:myhome.com:devices:cam01",
  "event": "low_battery",
  "frame": {},
  "route": {
    "adapter": "did-object",
    "route_id": "camera-home"
  }
}
```

### 11.5 与 OpenDAN Session Event Pump 的关系

本库只发布 KEvent 并返回 `kevent_pattern`。OpenDAN 的 session event pump 负责订阅 pattern、fanout 到 session、持久化 session subscription intent。

本库不直接写 OpenDAN session meta。即使 OpenDAN 持久化了 session subscription，也只表示 session 重启后可以重新订阅事件源；不表示事件消息被持久化或会被补投。

---

## 12. 错误模型

定义统一错误类型：

```rust
pub enum AgentDIDObjectError {
    InvalidConfig(String),
    RouteNotFound(String),
    AdapterNotFound(String),
    AdapterUnavailable(String),
    UnsupportedObjectRef(String),
    UnsupportedMethod(String),
    ResolveError(String),
    DeclaredCapabilityNotFound(String),
    SchemaError(String),
    HttpError(String),
    ProtocolError(String),
    KEventError(String),
    EventBridgeError(String),
    AdapterError(String),
}
```

错误要求：

- `read()` 的局部 property 失败应进入 `AdaptReadResponse.errors` 或 `adapt_meta.property_errors`，不要升级为整体失败。
- `x_call()` 的 provider action error 应映射为 `AgentToolResult.status = "error"` 和 `detail.error`，不是 Rust error，除非 HTTP / protocol envelope 本身不可解析。
- route / config / adapter missing 必须返回 Rust error。
- event bridge 连接失败必须返回 Rust error；active 后运行时失败通过 KEvent payload 和 log 暴露。

---

## 13. 缓存与 session-aware 规则

第一版可以只实现 read 核心流程内的内存缓存，不实现 durable cache。缓存只属于 read 拼接层，不属于 Adapt。

可缓存内容：

- `ResolvedDIDObject`。
- profile hash。
- route match result。

缓存 key：

```text
normalized object ref + adapter id
```

缓存失效来源：

- action `refresh_hints`。
- event `refresh_hints` / `invalidated_objects`。
- TTL。

`read()` 可以在 `AgentToolResult.summary` / prompt guidance 中提示 unchanged，也可以在内部 `detail` 或调试日志中记录 cache metadata；默认不要求 CLI 解析这些字段。

调试 metadata 示例：

```json
{
  "hit": true,
  "object_version": "optional",
  "profile_hash": "sha256:..."
}
```

不要把 `known_in_session`、`read_after` 反写进核心协议；它们只能是 Agent-facing 表达。

---

## 14. 安全与策略要求

Runtime / adapter 必须：

- 只调用 route 允许的 adapter。
- 对 DID Object adapter，调用前必须确认 property/action/event 在 Profile 中声明。
- 对 `x_call()` 保留 `trace_id`、`idempotency_key`、`confirm_token` 字段。
- 对本地 HTTP adapter，默认只允许 loopback endpoint。
- 不允许 route 配置任意执行外部命令。
- 不在 log 中输出完整 auth token。

Provider 侧策略不由本库替代。即使本库做了 declared capability 检查，Provider 仍必须重新校验 auth、RBAC、object state、quota、params schema、freshness 和 idempotency。

---

## 15. 测试要求

至少新增以下单元测试：

1. `ObjectRouteConfig` TOML 解析、校验、重复 id、缺失 adapter。
2. route priority 和同优先级稳定顺序。
3. route method 限制。
4. ObjectRef normalize：接受 hierarchical URL，拒绝 DID URI、alias、普通本地路径。
5. filesystem adapter read：文本、二进制摘要、size limit / truncation metadata。
6. web adapter read：HTML / text / JSON 到 LLM-friendly content / meta / guidance 片段的翻译。
7. agent_runtime adapter：没有 runtime handle 时返回 `AdapterUnavailable`。
8. DID Object adapter `x_call()`：fake DIDObjectClient 或测试 adapter 返回 success / error mapping。
9. `read()` property 局部失败不导致整体失败。
10. local HTTP adapter request / response JSON contract。
11. event id 编码：URL 特殊字符、空 path、hash fallback。
12. event bridge ref count：第一个订阅启动，最后一个取消关闭。
13. event bridge fake transport：EventFrame -> KEvent publish payload。

验证命令：

```bash
cd src
cargo test -p agent-did-object-lib
```

如果把 crate 加入 workspace 后影响全局构建，还应运行：

```bash
cd src
cargo test
```

---

## 16. 分阶段交付

### Phase 1: Crate 与配置路由

- 创建 `agent-did-object-lib` crate。
- 实现 types / error / config / router。
- 实现 adapter registry 和 fake adapter 测试。
- 完成 route config 单元测试。

### Phase 2: read / x-call 基础能力

- 实现 `AgentDIDObjectRuntime::read()` 和 `x_call()` 主流程。
- 实现 `FilesystemAdapter` 的 read。
- 实现 `WebAdapter` 的 read。
- 实现 `AgentRuntimeAdapter` 的 read 骨架。
- 实现 `DidObjectProtocolAdapter` 的 resolve / read / x_call。
- 实现 `LocalHttpAdapter` 的 read / x_call。
- 完成 action response mapping 测试。

### Phase 3: 本地 HTTP 扩展 adapter

- 固化 `/adapter/read`、`/adapter/x-call`、`/adapter/events/*` JSON contract。
- 完成本地 adapter loopback endpoint 校验。
- 提供测试 HTTP server 或 fake transport。

### Phase 4: Event bridge

- 实现 bridge manager、bridge key、ref count、state machine。
- 实现 `EventTransport` trait。
- 在不新增 WebSocket 依赖的前提下完成 fake transport 单测。
- 若需要真实 WebSocket client，先向用户确认新增依赖。

### Phase 5: OpenDAN / Agent Tool 接入准备

- 输出稳定 Rust API。
- 文档中给出 `x-call` CLI 使用同一 `ObjectRouteConfig` 的入口。
- 不在本阶段修改 OpenDAN session pump，除非后续任务明确要求。

---

## 17. 验收标准

实现完成后应满足：

- `src/frame/agent-did-object-lib` 是可编译 library crate。
- route config 能表达 Object ID / pattern 到具体 adapter 的映射。
- `read()` 必须通过 route 选择 adapter，不允许绕过 route。
- `read()` 和 `x_call()` 公开返回值必须是合法 `AgentToolResult`。
- `read()` 至少支持 filesystem、web、agent_runtime 骨架和 DID Object adapter。
- `x_call()` 能用同一路由配置执行 DID Object action、local HTTP adapter action 或 agent_runtime 声明的轻量 action。
- subscription 能用同一路由配置返回本地 KEvent pattern。
- adapter trait 支持内置和用户扩展实现。
- local HTTP adapter 可由 Agent Docker 容器内 TypeScript server 实现。
- event bridge 在订阅时 lazy start，并能把 EventFrame 发布成合法 KEvent。
- 单元测试覆盖路由、adapter、x-call mapping、event bridge 状态机。
- 未经用户确认，不新增 WebSocket 相关依赖。

---

## 18. 风险与待确认项

1. 真实 WebSocket transport 需要依赖选择；当前 workspace 没有明显现成 crate，新增依赖必须确认。
2. DID 输入的完整 resolver route 依赖 `NameClient` / provider 细节，第一版可以先把 DID route 留给 adapter 或返回明确错误。
3. `read()` 的 Agent-facing 文本输出未来可能需要根据 LLM context compression 再调整；第一版先保持段落结构稳定且可测试。
4. 本地 HTTP adapter 的安全策略第一版只允许 loopback；如果未来允许远端 adapter，需要单独设计 auth / permission。
5. Event bridge 不做 durable delivery。若需要跨重启恢复 subscription intent，应由 OpenDAN session meta 处理；若需要可靠事件或 MQ-style replay，应另起 durable event / MQ schema 任务。

# AICC API 设计

版本：`v0.4-beta2.2-target`
更新基线：`2026-08-31`
配套文档：


- `doc/aicc/aicc 逻辑模型目录.md`
- `doc/aicc/aicc_router.md`
- `doc/aicc/driver_metadata_schema.md`
- `doc/aicc/provider_profile_schema.md`
- `doc/aicc/provider_architecture_durable_data_schema.md`
- `doc/aicc/maintenance/krpc_aicc_calling_guide.md`
- `doc/aicc/maintenance/update_aicc_settings_via_system_config.md`

本文定义 AICC 面向调用方、Provider Adapter、Router、Control Panel 和 Agent Runtime 的标准 API 设计。目标是覆盖 `aicc 逻辑模型目录.md` 中规划的所有已知 AI 调用方法。

> **Beta 2.2 breaking-change 基线**：AICC 只保留控制面、typed inference 数据面和 Helper 三层 API。删除 `AiMethodRequest`、`model.alias`、`must_features`、`requirements.extra.disable_capabilities`、legacy all-in-one method 和管理接口兼容别名。Provider Profile、Protocol Adapter、Model Driver、Provider Rules 和 Pricing 使用独立身份与 schema，不读取旧 `provider_driver`。

---

## 1. 设计原则

### 1.1 核心入口保持 BuckyOS kRPC

AICC 是 BuckyOS 系统服务，核心调用入口保持：

```text
POST /kapi/aicc
```

核心 kRPC 方法：

| method | 语义 |
|---|---|
| `route.resolve` | 控制面路由解析。输入逻辑模型名，输出一次确定的 exact model、Provider 信息、候选顺序和 trace。 |
| `chat.completions.create`、`embeddings.create`、`rerank.create`、`images.*`、`vision.*`、`audio.*`、`videos.*` | typed inference 数据面。只接受 `exact_model`，不接受逻辑模型名，不做逻辑 fallback。 |
| `helper.*` | helper 组合层。接收 `logical_model` 和对应 typed request，语义等价于 `route.resolve` + typed inference。 |
| `cancel` | 请求取消异步 task；返回值必须真实反映是否已触发上游取消或本地中止。 |
| `service.reload_settings` | 从 `services/aicc/settings` 重新加载 Provider 配置。 |
| `quota.query` | 查询调用方在 capability / method 维度的剩余额度和预算状态。 |
| `provider.list` / `provider.health` | 查询 Provider inventory 和健康状态。 |

不为核心调用另建 `/v1/invoke`、`/v1/jobs`、`/v1/objects`。如果未来需要 OpenAI-compatible 或 REST-compatible API，应放在 Gateway Adapter / SDK Facade 层，把请求转换成 AICC kRPC。

### 1.2 `method` 决定 schema，`Capability` 只做粗分组

`method` 是 AICC 的请求 schema discriminator，例如 `chat.completions.create`、`images.generate`、`audio.transcriptions.create`。`api_type` 是路由能力类型，例如 `llm.chat`、`image.txt2img`、`audio.asr`；它不等于 RPC method，也不决定 Provider endpoint。

Provider 不能自定义方法名，只能声明自己支持标准集合中的哪些 method。`Capability` 只表达粗能力和权限边界，不决定 schema，也不随 method 数量膨胀。

每个数据面 method 定义独立 typed request/response。method 自有字段直接位于 `params`，不使用 `AiMethodRequest`、`payload.input_json`、`payload.resources` 或 `payload.options` 等通用容器。
路由规则：

1. Router 按 `api_type` 选择候选；Provider Call Resolver 按 kRPC `method` 和 `api_type` 解析 operation。
2. fallback 不得改变 method，只能在同一 method 的候选模型内切换。
3. `logical_model` 只存在于 `route.resolve` 和 Helper；typed inference 只接受 `exact_model`。
4. `Capability` 可用于 RBAC 边界、UI tab 分组和粗粒度 quota 桶，但不能作为 schema discriminator。
5. RBAC / quota 支持直接挂在 method namespace 上，例如 `audio.*`、`image.*`、`llm.chat`。

标准 capability 粒度：

| capability | 覆盖 method namespace |
|---|---|
| `llm` | `llm.*` |
| `embedding` | `embedding.*` |
| `rerank` | `rerank` |
| `image` | `image.*` |
| `vision` | `vision.*` |
| `audio` | `audio.*` |
| `video` | `video.*` |
| `agent` | `agent.*` |

### 1.3 控制面 / 数据面 / Helper 分层

新调用方应按三层语义接入：

1. `route.resolve` 是控制面，只接受逻辑模型名。它返回 selected ModelUID/exact model、Provider Instance/Profile、Protocol Adapter、Model Driver、origin model、原始 provider model、operation、fallback 和 trace。
2. typed inference 是数据面。请求必须包含 `exact_model`，逻辑模型名会被拒绝；内部关闭逻辑 fallback。调用阶段重新校验 catalog revision 和动态 availability，并生成内部 `ResolvedProviderCall`。
3. `helper.*` 接受 `logical_model`、结构化 requirements/policy 和对应 typed 业务字段，内部先 `route.resolve`，再调用 typed inference。Helper 不接受或构造 `AiMethodRequest`。

`fallback_attempts` 表达路由建议的运行时 failover 候选顺序，不是 lease，也不保证候选在后续时刻仍可用。它不包含 primary；当前受 `runtime_failover` 和系统 `fallback_limit` 限制，排序来自 scheduler 之后的同 method 候选。

`enabled_capabilities` / `disabled_capabilities` 表达本次路由后模型可用能力与请求禁用能力。能力判断使用结构化 `ModelRequirement` / `ModelDisable`，最终能力是 Model Driver、Protocol Adapter 与 Provider discovery 的交集。

### 1.4 数据面复用 BuckyOS ResourceRef / FileObject Meta

AICC 不引入私有 Object Store。非结构化数据通过当前 `ResourceRef` 传递：

```rust
pub enum ResourceRef {
    Url { url: String, mime_hint: Option<String> },
    Base64 { mime: String, data_base64: String },
    NamedObject { obj_id: ObjId },
}
```

`ResourceRef::NamedObject` 的 `obj_id` 可以直接指向 `FileObject`。文件类资源的 media type、大小、digest、宽高、时长、文件名、标签等通用信息应来自 `FileObject.meta`，不再在 AICC 协议中额外定义资源描述结构：

```rust
pub struct FileObjectMeta {
    pub media_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub digest: Option<String>,
    pub attributes: Option<serde_json::Value>,
}
```

Router、policy、日志层只读取 `ObjId` 和 `FileObject.meta`，不读取 bytes。Provider Adapter 只能在最后执行阶段读取 `ResourceRef` 指向的内容。`Url` / `Base64` 只用于临时或小对象输入；需要稳定 metadata、权限和复用的资源应先写成 FileObject，再以 `NamedObject` 引用。

### 1.5 任务生命周期复用 task-manager

AICC 不定义私有 Job API。长任务使用 `task-manager`：

| AICC response | task-manager 是否创建 task | task-manager 状态 | `task_id` 来源 | `event_ref` 是否可订阅 |
|---|---|---|---|---|
| `status=succeeded` | 是 | `Completed` | AICC 外部 task id | 是，用于审计、最终结果和可观察数据。 |
| `status=running` | 是 | `Accepted` / `Running` | TaskMgr 2.0 `task_id` | 是，用于进度、取消和最终结果。调用方直接使用 `get_task({ task_id })`，不扫描任务列表。 |
| `status=failed` | best effort | `Failed` 或未创建 | AICC 诊断 id | 已创建 task 时可订阅；鉴权、反序列化、早期校验失败可直接返回 kRPC error。 |

视频生成、音乐生成、大批量 embedding、长文件转录等默认走异步任务。

---

## 2. 顶层协议

调用方按三层接入（见 §1.3）。本节只定义控制面、数据面和 Helper 三类 canonical 形态。Beta 2.2 不定义 legacy 兼容请求。

### 2.1 `route.resolve`（控制面）

输入逻辑模型名和本次请求约束，输出一次确定的 exact model 选择与解释。`logical_model` 只接受逻辑模型名，传入 `<provider_model_id>@<provider_instance_name>` exact model 会被拒绝。

Request：

```json
{
  "method": "route.resolve",
  "params": {
    "api_type": "llm.chat",
    "logical_model": "llm.plan",
    "requirements": { "tool_call": true, "json_schema": true, "min_context_tokens": 200000 },
    "disable": { "web_search": true },
    "policy": { "profile": "balanced" },
    "estimated_input_tokens": 1200,
    "estimated_output_tokens": 400,
    "session_overlay": {
      "logical_tree": {
        "llm": {
          "children": {
            "plan": {
              "item_overrides": {
                "local": { "weight": 2.0 }
              }
            }
          }
        }
      }
    }
  },
  "sys": [1001, "<session_token>", "trace-aicc-route-001"]
}
```

Response：

```json
{
  "selected_exact_model": "gpt-5.1:reasoning-high@openai_primary",
  "selected_model_uid": "openai:gpt-5.1:responses",
  "provider_instance_name": "openai_primary",
  "provider_profile_id": "openai",
  "protocol_adapter_id": "openai-responses",
  "model_driver_id": "openai",
  "origin_model_id": "gpt-5.1",
  "provider_model_id": "gpt-5.1",
  "operation": "responses.create",
  "enabled_capabilities": ["tool_calling", "json_output"],
  "disabled_capabilities": ["web_search"],
  "fallback_attempts": [
    {
      "exact_model": "claude-sonnet-4-5@anthropic_main",
      "provider_instance_name": "anthropic_main",
      "provider_model_id": "claude-sonnet-4-5"
    }
  ],
  "route_trace": { "...": "见 §3.7" },
  "inventory_revision": "inv-42"
}
```

说明：

1. `selected_exact_model` 是 AICC 语义下的确定物理模型名，形如 `provider_model_id[:variant]@provider_instance_name`。
2. `session_overlay` 是调用方已经合成好的本次请求 route overlay。AICC 不维护应用 session 状态，只把该 overlay 覆盖到系统级 route config 之上。
3. `provider_model_id` 是 Provider discovery 返回并用于 wire request 的原始模型名，不得替换为 `origin_model_id`。
4. `operation` 由 Provider Rules 和 adapter 注册表解析；调用方不能指定任意 operation 或 URL。
5. `enabled_capabilities` / `disabled_capabilities` 表达本次路由后实际启用 / 禁用的能力集合。
6. `fallback_attempts` 是路由建议的运行时候选顺序（不含 primary），供调用方在失败后自行决定是否重试，不是 lease，也不保证后续时刻仍可用。

### 2.2 typed inference（数据面）

数据面只接受 `exact_model`，不接受逻辑模型名，内部默认关闭 `allow_fallback` 和 `runtime_failover`。`chat.completions.create` 示例：

```json
{
  "method": "chat.completions.create",
  "params": {
    "exact_model": "gpt-5.1:reasoning-high@openai_primary",
    "messages": [
      { "role": "developer", "content": [{ "type": "text", "text": "You are a coding assistant." }] },
      { "role": "user", "content": [{ "type": "text", "text": "实现快速排序" }] }
    ],
    "tools": [],
    "response_format": { "type": "json_schema", "json_schema": { "name": "answer", "schema": {} } },
    "idempotency_key": "client-req-001"
  },
  "sys": [1001, "<session_token>", "trace-aicc-chat-001"]
}
```

`images.generate` 示例：

```json
{
  "method": "images.generate",
  "params": {
    "exact_model": "flux-1.1-pro@fal_primary",
    "prompt": "a red fox in snow, cinematic",
    "size": "1024x1024",
    "idempotency_key": "client-img-001"
  }
}
```

数据面字段直接挂在 `params` 上。调用方只能传 canonical 字段；Provider-specific defaults/set/remove 由 Provider Call Resolver 应用。逻辑模型名传入会返回 `invalid_model_name`；primary exact model在推理时 quota exhausted / unavailable 时不 fallback，调用方需重新 `route.resolve` 或使用 `fallback_attempts`。

### 2.3 `helper.*`（组合层）

Helper 使用与对应 typed inference 相同的业务字段，只把 `exact_model` 替换为 `logical_model`，并增加结构化 `requirements`、`disable` 和 `policy`。例如：

```json
{
  "method": "helper.llm_chat",
  "params": {
    "logical_model": "llm.plan",
    "requirements": {
      "tool_call": true,
      "json_schema": false
    },
    "disable": {
      "web_search": true
    },
    "policy": {
      "profile": "balanced"
    },
    "messages": [
      {
        "role": "user",
        "content": [
          { "type": "text", "text": "你好" }
        ]
      }
    ]
  }
}
```

其语义严格等价于 `route.resolve(api_type="llm.chat", logical_model="llm.plan", ...)` 后调用 `chat.completions.create(exact_model=route.selected_exact_model, messages=...)`。Helper 不接收 `model.alias`、`must_features`、`payload` 或 `provider_options`。
### 2.4 typed inference 通用响应约束

每个 typed inference method 使用独立的请求和响应结构，不再通过通用 `AiMethodRequest`、`AiPayload` 或 `AiResponseSummary` 承载业务字段。所有响应共享以下 envelope 字段：

- `task_id`、`status`、`usage`、`cost`、`finish_reason`；
- `provider_task_ref`、`route_trace`、`event_ref`；
- method 自有的 typed 结果，例如 `message`、`artifacts`、`embeddings` 或 `segments`。

同步和异步结果必须使用相同的 typed result。Task Final event 中的业务结果必须精确保存；Base64、ProviderState 和 thinking signature 只能在日志、审计摘要和 provider I/O 观测副本中脱敏，不能改写业务结果。

Provider 原生扩展只允许放入带 Provider namespace 的 `ProviderState` 或明确定义的扩展字段。Provider adapter 必须保持 content block 顺序；无法归一但需要多轮恢复的原生 block 保存为 opaque ProviderState。
### 2.5 流式与进度观察

AICC 不为 streaming 引入独立协议层，也不在 method schema 中定义 `stream: true`、token delta event、image step、video frame 等中间态字段。

成功执行路径只有两类；失败仍使用 `status=failed` 错误态，不引入第三种 `streaming` 状态：

| response status | 语义 |
|---|---|
| `succeeded` | 同步完成，最终 result 直接返回，同时写入 task-manager。 |
| `running` | 异步运行，调用方通过 task-manager 观察进度和最终结果。 |

长任务进度统一由 task-manager 的 task event / task data 承载。task event schema 由 task-manager 定义；AICC method schema 只定义最终 request / result。

调用方需要边推理边展示时，Provider Adapter 可以在执行过程中向 task data 写入实现侧可观察字段，例如：

```json
{
  "aicc": {
    "progress": {
      "partial_text": "已生成的片段",
      "tokens_generated": 128,
      "frames_generated": 12
    }
  }
}
```

这些字段只属于实现侧 task data，不进入 AICC 协议规范。UI 通过订阅 task event 或轮询 task 状态获取。

### 2.6 `idempotency_key`

`idempotency_key` 用于调用方重试去重，避免网络重试或进程恢复导致重复扣费、重复生成或重复写入。

| 维度 | 语义 |
|---|---|
| 幂等窗口 | 默认 24h；实现可以通过配置延长，但不得短于 24h。 |
| 作用域 | `tenant_id + method + idempotency_key`。不同 method 使用相同 key 不互相命中。 |
| 命中运行中任务 | 返回原 `task_id`、原 `status=running` 和原 `event_ref`。 |
| 命中已完成任务 | 返回原 `task_id`、`status=succeeded` 和原 `result`。 |
| 命中已失败任务 | 返回原 `task_id` 和原错误 payload，不自动重试。 |
| 命中已取消任务 | 返回原 `task_id`、`status=failed` 和 `code=cancelled`；调用方要重试必须换新 key。 |

同一作用域内重复 key 但 canonical request body 不一致时，必须返回 `idempotency_conflict`。

### 2.7 通用 batch 策略

AICC v0 不引入通用 batch primitive。批量调用由调用方多次发送标准 method request，AICC 通过 quota、并发限制和 Provider health 控制保护下游 Provider。

单个 method 可以在自身 schema 内定义批量输入，例如 `embedding.text.items`，但这不等同于跨 method 的 batch job API。

### 2.8 `cancel`

Request：

```json
{
  "method": "cancel",
  "params": {
    "task_id": "aicc-002"
  },
  "sys": [1002, "<session_token>", "trace-aicc-cancel"]
}
```

Response：

```json
{
  "task_id": "aicc-002",
  "accepted": true
}
```

语义：

1. `accepted=true` 只能表示已调用 Provider cancel API，或已成功中止本地 polling/执行并安装 late-Final 屏蔽。
2. task binding 不存在、任务已结束、Provider 不支持取消或无法确认中止时返回 `accepted=false`。
3. cancel 成功后到达的 Provider Final 结果不得覆盖 canceled 终态。
4. 已进入 Provider 计费执行阶段时，取消不保证退费；跨 tenant cancel 必须拒绝。

### 2.9 查询类 method 占位

查询类 method 仍走 `/kapi/aicc`，但不属于 AI 推理 method，不参与 fallback。

`quota.query`：

```json
{
  "method": "quota.query",
  "params": {
    "capability": "audio",
    "method": "audio.asr"
  }
}
```

Response：

```json
{
  "quota": {
    "state": "normal",
    "remaining_request_units": 1000,
    "remaining_cost_usd": 12.5,
    "reset_at": "2026-04-26T00:00:00Z"
  }
}
```

`provider.list`：

```json
{
  "method": "provider.list",
  "params": {
    "method": "llm.chat"
  }
}
```

`provider.health`：

```json
{
  "method": "provider.health",
  "params": {
    "exact_model": "gpt-5.5@openai_primary"
  }
}
```

### 2.10 `service.reload_settings`

`service.reload_settings` 用于从 `services/aicc/settings` 重新加载 Provider Instance 配置。`reload_settings`、`reaload_settings` 和 `service.reaload_settings` 均不是有效别名。

语义：

1. 新配置先完整解析和校验，校验成功后再原子替换 Router / Provider registry 的可见快照。
2. 进行中的请求继续使用启动时已经选定的 Provider 和 route decision，不受本次 reload 影响。
3. reload 失败不得污染旧配置；返回失败原因并保持旧配置继续服务。
4. 新请求只能看到旧快照或新快照，不允许看到半更新状态。
5. Provider 连接池、health cache 等运行时状态可以延迟收敛，但不得改变已经启动的 task 归属。

---

## 3. 通用数据结构

### 3.1 ResourceRef

```json
{
  "kind": "named_object",
  "obj_id": "chunk:..."
}
```

Rust enum 到 JSON 的正式映射必须使用：

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceRef {
    Url { url: String, mime_hint: Option<String> },
    Base64 { mime: String, data_base64: String },
    NamedObject { obj_id: ObjId },
}
```

因此 JSON tag 必须是 `url`、`base64`、`named_object`，不得使用 Rust variant 名 `Url`、`Base64`、`NamedObject`。

当 `obj_id` 指向 FileObject 时，AICC 从 FileObject meta 获取资源属性：

| meta 字段 | 说明 |
|---|---|
| `media_type` | MIME 类型，如 `image/png`、`audio/mpeg`、`application/pdf`。 |
| `size_bytes` | 对象字节数。 |
| `digest` | 内容 digest。 |
| `attributes` | 宽高、时长、页数、文件名、标签等扩展 metadata。 |

AICC 不再定义 `modality` 字段。需要区分图片、音频、视频、文档、tensor 等类型时，优先由 method schema 和 `media_type` 推导。

### 3.2 `AiMessage`（content-block 模型）

LLM 消息统一使用 content-block 形态的 `AiMessage`，对齐 Anthropic content block，并能 round-trip OpenAI Responses item 与 Gemini part：

```rust
pub struct AiMessage {
    pub role: AiRole,           // system | user | assistant | tool | developer
    pub content: Vec<AiContent>,
}

#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiContent {
    Text { text: String },
    Image { source: ResourceRef },
    Document { source: ResourceRef, title: Option<String> },
    ToolUse { call_id: String, name: String, args: HashMap<String, Value> },
    ToolResult { call_id: String, content: Vec<AiToolResultContent>, is_error: bool },
    Thinking { summary: Option<String>, text: Option<String>, provider_metadata: Option<Value> },
    ProviderState { provider: String, value: Value },
}
```

`ProviderState.provider` 保存 opaque state 的稳定所有者/消费者 namespace（例如
`openai`、`openrouter`、`anthropic` 或 `google`），由各 adapter 定义其可还原的
namespace；它不保存协议名或原生 item 类型。原生 item 类型继续由 `value` 自描述
（例如 OpenAI Responses 的 `value.type`）。

JSON 形态（注意图片块是 `type:image` + `source`，不再是 `type:resource` + `resource`）：

```json
{
  "role": "user",
  "content": [
    { "type": "text", "text": "请解释这张图。" },
    { "type": "image", "source": { "kind": "named_object", "obj_id": "chunk:image" } }
  ]
}
```

落地约束：

1. `messages[].content` 是 `Vec<AiContent>` content-block 数组。最常见的纯文本消息用单个 `text` block 表达（`AiMessage::text(role, "...")`）。
2. `role` 是 `AiRole` 枚举（snake_case 序列化）。`tool` 是 IR 内部承载 tool result 的角色，`developer` 是 OpenAI Responses 原生角色；Provider Adapter 在 lowering 时改写为各 provider 原生形态。
3. `tool_use` / `tool_result` 用 `call_id` 关联；`tool_result.content` 只允许 `text` / `image` / `document` 三类子块。
4. `thinking` 承载扩展思考；`provider_state` 承载无法跨 provider 抽象、但需要 round-trip 的 provider 原生项（OpenAI reasoning item、Claude server_tool_use 等），lowering 时只有 `provider` 匹配目标的块会被还原，其余丢弃。
5. 多模态内容直接进入 `content` 数组，不引入 `messages_v2` 等并行通道。

### 3.3 Generation Parameters

```json
{
  "temperature": 0.7,
  "top_p": 1.0,
  "max_output_tokens": 2048,
  "seed": 12345,
  "stop": ["</final>"],
  "output": {
    "media_type": "application/json"
  }
}
```

输出格式相关字段统一放在 `output` 子结构：

```json
{
  "output": {
    "media_type": "image/png",
    "size": "1024x1024",
    "sample_rate": 44100,
    "fps": 24
  }
}
```

其中 `media_type` 通用于文本、图片、音频和视频；`size` 仅用于图片；`sample_rate` 仅用于音频；`fps` 仅用于视频。

### 3.4 Usage

```json
{
  "tokens": {
    "input": 1024,
    "output": 512,
    "total": 1536,
    "cached": 300,
    "reasoning": 128
  },
  "media": {
    "audio_seconds": 12.4,
    "video_seconds": 8,
    "image_count": 1
  },
  "request_units": 1,
  "cost": {
    "amount": 0.0123,
    "currency": "USD"
  }
}
```

当前 `AiUsage` 已包含顶层 `request_units`，非 token provider 应至少上报该字段；其它媒体计量字段仍按本节分组结构逐步扩展。

### 3.5 Bounding Box

```json
{
  "format": "xywh",
  "unit": "px",
  "x": 120,
  "y": 80,
  "width": 300,
  "height": 600
}
```

`unit` 可为 `px` 或 `relative`。`relative` 范围为 `[0, 1]`。

### 3.6 Mask

```json
{
  "format": "rle",
  "size": [1024, 1024],
  "counts": "..."
}
```

支持：

| format | 说明 |
|---|---|
| `rle` | COCO-style run length encoding。 |
| `polygon` | 多边形点集。 |
| `bitmap_resource` | 通过 `ResourceRef::NamedObject` 引用 bitmap mask，mask 属性来自 FileObject meta。 |

### 3.7 Route Trace

`AiResponseSummary.extra.route_trace` 和 task data 中的 route trace 必须使用同一结构，便于日志、debug 工具和 UI 解析：

```json
{
  "attempts": [
    {
      "step": 0,
      "exact_model": "gpt-5.5@openai_primary",
      "started_at": "2026-04-25T10:00:00Z",
      "ended_at": "2026-04-25T10:00:01Z",
      "outcome": "failed",
      "error_code": "provider_error",
      "fallback_reason": "provider_5xx"
    },
    {
      "step": 1,
      "exact_model": "claude-sonnet-4.6@anthropic",
      "started_at": "2026-04-25T10:00:01Z",
      "ended_at": "2026-04-25T10:00:03Z",
      "outcome": "succeeded",
      "error_code": null,
      "fallback_reason": "runtime_failover"
    }
  ],
  "final_model": "claude-sonnet-4.6@anthropic"
}
```

`outcome` 可为 `succeeded`、`failed`、`skipped`。`fallback_reason` 可为空；非空时应使用可枚举短码，例如 `provider_5xx`、`timeout`、`health_unavailable`、`policy_retry`。

---

## 4. Method 总览

本表只描述协议规范。当前实现进度不在规范正文维护，应迁出到实现 tracker 或 release note。

| typed method | api_type | 默认逻辑目录 | 默认任务模式 |
|---|---|---|---|
| `chat.completions.create` | `llm.chat` | `llm.chat` / `llm.*` | sync 或 async |
| `embeddings.create` | `embedding.text` / `embedding.multimodal` | `embedding.*` | sync 或 async |
| `rerank.create` | `rerank` | `rerank.general` | sync |
| `images.generate` | `image.txt2img` | `image.txt2img` | sync 或 async |
| `images.edit` | `image.img2img` / `image.inpaint` | `image.img2img` / `image.inpaint` | sync 或 async |
| `images.upscale` / `images.remove_background` | `image.upscale` / `image.bg_remove` | 同 api_type | sync 或 async |
| `vision.ocr` / `vision.caption` / `vision.detect` / `vision.segment` | 同名 api_type | `image.*` | sync 或 async |
| `audio.speech.create` | `audio.tts` | `audio.tts` | sync 或 async |
| `audio.transcriptions.create` | `audio.asr` | `audio.asr` | sync 或 async |
| `audio.music.create` / `audio.enhance` | `audio.music` / `audio.enhance` | 同 api_type | async 或 sync |
| `videos.generate` | `video.txt2video` / `video.img2video` | 对应 `video.*` | async |
| `videos.transform` / `videos.extend` / `videos.upscale` | `video.video2video` / `video.extend` / `video.upscale` | 同 api_type | async |
| `agent.computer_use` | `agent.computer_use` | `agent.computer_use` | session async |

命名规范：

1. 逻辑模型目录使用 `image.txt2img` / `image.img2img`。
2. 标准 method 只使用逻辑模型目录中的 `txt2img` / `img2img`。
3. 当前 Rust 内部已有的 `image.txt2image` / `image.img2image` 应迁移为标准 method 名，不在新协议中保留 alias。

---

## 5. LLM API

### 5.1 `chat.completions.create`

用途：通用对话、Agent plan/code/reason/summary、VQA、多模态聊天、工具调用。

数据面字段直接挂在 `params` 上：`exact_model`、`messages`、`tools`、`response_format`、generation parameters、idempotency 和 task options。Provider-specific 参数由 Provider Rules 生成，不属于调用协议。

```json
{
  "messages": [
    {
      "role": "developer",
      "content": [
        { "type": "text", "text": "You are a coding assistant." }
      ]
    },
    {
      "role": "user",
      "content": [
        { "type": "text", "text": "解释这张图。" },
        { "type": "image", "source": { "kind": "named_object", "obj_id": "chunk:image" } }
      ]
    }
  ],
  "tools": [
    {
      "type": "function",
      "name": "get_weather",
      "description": "Get weather by city.",
      "args_json_schema": {
        "type": "object",
        "properties": {
          "city": { "type": "string" }
        },
        "required": ["city"],
        "additionalProperties": false
      }
    }
  ],
  "response_format": {
    "type": "json_schema",
    "json_schema": {
      "name": "answer",
      "schema": {
        "type": "object",
        "properties": {
          "summary": { "type": "string" }
        },
        "required": ["summary"],
        "additionalProperties": false
      }
    }
  }
}
```

Response mapping：

`chat.completions.create` 返回 `LlmChatInvokeResponse`，assistant 输出使用 content-block `message: AiMessage`。`text`、`tool_use`、`thinking` 和 opaque `ProviderState` 必须保持原始顺序；存在匹配当前 adapter 的 ProviderState 时优先原样 replay，否则从 provider-neutral blocks lowering。foreign ProviderState 被忽略。

Fallback（逻辑路由层语义，由 `route.resolve` / helper / logical definition 承载，数据面 `chat.completions.create` 自身不 fallback）：

1. `llm.plan`、`llm.code`、`llm.summary` 可 parent fallback 到 `llm`。
2. `llm.reason` 默认 disabled 或 strict，避免静默降级到无 reasoning 能力模型。
3. `llm.vision` 必须硬过滤 `vision=true`（在逻辑模型定义 `min_line` 中表达）。

AICC 不定义 `llm.completion`。纯文本 completion 由调用方构造单条 user message 后调用 `chat.completions.create`；服务端不保留旧 Completion schema、method alias 或自动转换入口。

---

## 6. Embedding API

### 6.1 `embedding.text`

用途：文本、代码、文档 chunk embedding。

Request：

```json
{
  "items": [
    { "type": "text", "text": "hello world", "id": "item-1" },
    {
      "type": "resource",
      "id": "doc-1",
      "resource": {
        "kind": "named_object",
        "obj_id": "chunk:doc"
      }
    }
  ],
  "chunking": {
    "strategy": "auto",
    "max_tokens": 800,
    "overlap_tokens": 80
  },
  "embedding_space_id": null,
  "dimensions": 1024,
  "normalize": true,
  "prefer_artifact": "auto"
}
```

`prefer_artifact` 可为 `true`、`false`、`auto`。`auto` 时，`items > 100` 或预估 response body 超过 1MB 必须走 artifact；小于阈值时可直接返回 inline response。

小批量 response：

```json
{
  "data": [
    {
      "index": 0,
      "id": "item-1",
      "embedding": [0.0123, -0.0456],
      "embedding_space_id": "bge-m3:1024:cosine:normalized:v1"
    }
  ]
}
```

大批量 response：

```json
{
  "data_resource": {
    "kind": "named_object",
    "obj_id": "chunk:embeddings"
  }
}
```

Response mapping：

1. 小批量数据放 `AiResponseSummary.extra.embedding`。
2. 大批量数据生成 `AiArtifact`，resource 使用 `NamedObject`。
3. `rows`、`dimensions`、`embedding_space_id` 必须进入结果 FileObject meta。

Fallback：

1. 默认 strict。
2. 如果 request 指定 `embedding_space_id`，fallback 后的模型必须产出相同 space。
3. 已存在向量索引查询时，禁止 fallback 到不同 space。

### 6.2 `embedding.multimodal`

用途：CLIP / SigLIP 类跨模态 embedding。

Request：

```json
{
  "items": [
    {
      "id": "pair-1",
      "text": "a red car",
      "image": {
        "kind": "named_object",
        "obj_id": "chunk:image"
      }
    }
  ],
  "dimensions": 1408,
  "normalize": true
}
```

Response 与 `embedding.text` 相同。fallback 同样必须保证 embedding space 兼容。

---

## 7. Rerank API

### 7.1 `rerank`

用途：Cross-encoder / late interaction 文档重排序。

Request：

```json
{
  "query": "What is the refund policy?",
  "documents": [
    {
      "id": "doc-1",
      "text": "Refunds are available within 30 days.",
      "metadata": { "source": "policy.md" }
    },
    {
      "id": "doc-2",
      "resource": {
        "kind": "named_object",
        "obj_id": "chunk:doc-2"
      }
    }
  ],
  "n": 5,
  "return_documents": false
}
```

Response：

```json
{
  "results": [
    {
      "index": 0,
      "id": "doc-1",
      "score": 0.98
    }
  ]
}
```

Response mapping：结果放 `AiResponseSummary.extra.rerank`。

Fallback：默认 strict。不同 reranker 分数不可直接比较，fallback 只允许在同一任务内重跑，不允许和旧分数混排。

---

## 8. Image API

### 8.1 `image.txt2img`

Request：

```json
{
  "prompt": "A precise product photo of a matte black desk lamp.",
  "negative_prompt": "blurry, low quality",
  "n": 1,
  "aspect_ratio": "1:1",
  "quality": "high",
  "seed": 12345,
  "output": {
    "media_type": "image/png",
    "size": "1024x1024"
  }
}
```

Response：

```json
{
  "images": [
    {
      "kind": "named_object",
      "obj_id": "chunk:generated-image"
    }
  ]
}
```

Response mapping：每张图片生成 `AiArtifact`，宽高、media type 等写入生成图片的 FileObject meta。

OpenAI 渠道的 operation 由最终解析后的 metadata/Provider Rules 决定：

- GPT Image / DALL·E 等图片模型使用 Image API operation；
- 声明 `capabilities.image_generation=true` 的 GPT-5 系列主线模型使用 Responses `image_generation` tool；
- 不得根据模型名、调用方是否传 feature 或 endpoint URL 在 adapter 中临时猜测路径。

Responses 返回的每个 completed `image_generation_call.result` 必须转换为通用图片 artifact，并同时保留对应 OpenAI `ProviderState` 以支持后续多轮编辑。多个结果全部保留；MIME 根据实际输出格式确定。缺失结果、非法 Base64、tool failed/rejected 必须返回明确 Provider error。

### 8.2 `image.img2img`

Request：

```json
{
  "images": [
    {
      "kind": "named_object",
      "obj_id": "chunk:source-image"
    }
  ],
  "prompt": "Change the background to a sunny beach.",
  "strength": 0.6,
  "output": {
    "media_type": "image/png"
  }
}
```

Response：同 `image.txt2img`。

当 Provider Rules 为已选模型解析出 Responses image-generation edit operation 时，输入图片降低为 `input_image`，tool action 使用 `edit`；否则使用图片模型的 Image API edit operation。`image.inpaint` 只有在 mask lowering 已被对应 operation 明确支持时才可声明。

### 8.3 `image.inpaint`

Request：

```json
{
  "image": {
    "kind": "named_object",
    "obj_id": "chunk:image"
  },
  "mask": {
    "kind": "named_object",
    "obj_id": "chunk:mask"
  },
  "prompt": "Add a vase of flowers on the table.",
  "mask_semantics": "white_area_is_edit_area",
  "output": {
    "media_type": "image/png"
  }
}
```

Response：同 `image.txt2img`。

`mask_semantics` 枚举：

| value | 说明 |
|---|---|
| `white_area_is_edit_area` | 白色区域表示编辑区域。 |
| `black_area_is_edit_area` | 黑色区域表示编辑区域。 |
| `alpha_zero_is_edit_area` | alpha 为 0 的透明区域表示编辑区域。 |

### 8.4 `image.upscale`

Request：

```json
{
  "image": {
    "kind": "named_object",
    "obj_id": "chunk:image"
  },
  "scale": 4,
  "target_width": 4096,
  "target_height": 4096,
  "preserve_faces": true,
  "output": {
    "media_type": "image/png"
  }
}
```

Response：

```json
{
  "image": {
    "kind": "named_object",
    "obj_id": "chunk:upscaled"
  }
}
```

### 8.5 `image.bg_remove`

Request：

```json
{
  "image": {
    "kind": "named_object",
    "obj_id": "chunk:image"
  },
  "mode": "rgba_image",
  "output": {
    "media_type": "image/png"
  }
}
```

Response：

```json
{
  "image": {
    "kind": "named_object",
    "obj_id": "chunk:rgba"
  }
}
```

Image fallback：

1. `txt2img` 可 parent fallback，但必须保持 `method=image.txt2img`。
2. `inpaint` fallback 必须保持 mask 语义一致。
3. `upscale` fallback 必须满足目标分辨率和人脸保护等硬约束。

---

## 9. Vision API

Vision API 用于结构化图像理解。自由文本 VQA 使用 `llm.chat`，并在 message content 中传 image resource。

### 9.1 `vision.ocr`

Request：

```json
{
  "document": {
    "kind": "named_object",
    "obj_id": "chunk:page"
  },
  "level": "word",
  "language_hints": ["zh", "en"],
  "return_layout": true,
  "return_artifacts": ["plain_text", "alto_json"]
}
```

Response：

```json
{
  "pages": [
    {
      "page_index": 0,
      "width": 2480,
      "height": 3508,
      "blocks": [
        {
          "type": "text",
          "bbox": {
            "format": "xywh",
            "unit": "px",
            "x": 100,
            "y": 200,
            "width": 500,
            "height": 120
          },
          "lines": [
            {
              "text": "Example text",
              "confidence": 0.98
            }
          ]
        }
      ]
    }
  ],
  "artifacts": {
    "plain_text": {
      "kind": "named_object",
      "obj_id": "chunk:ocr-text"
    }
  }
}
```

Response mapping：

1. 纯文本摘要写 `AiResponseSummary.text`。
2. 结构化 OCR 写 `AiResponseSummary.extra.ocr`。
3. OCR artifact 写 `AiResponseSummary.artifacts`。

### 9.2 `vision.caption`

Request：

```json
{
  "image": {
    "kind": "named_object",
    "obj_id": "chunk:image"
  },
  "style": "short",
  "language": "zh-CN",
  "n": 3
}
```

Response：

```json
{
  "captions": [
    {
      "text": "一盏黑色台灯放在木质桌面上。",
      "confidence": 0.93
    }
  ]
}
```

`captions[0].text` 可同步写入 `AiResponseSummary.text`。

### 9.3 `vision.detect`

Request：

```json
{
  "image": {
    "kind": "named_object",
    "obj_id": "chunk:image"
  },
  "classes": ["person", "car"],
  "score_threshold": 0.3,
  "bbox_spec": {
    "format": "xywh",
    "unit": "px"
  }
}
```

Response：

```json
{
  "detections": [
    {
      "label": "person",
      "class_id": "person",
      "score": 0.97,
      "bbox": {
        "format": "xywh",
        "unit": "px",
        "x": 120,
        "y": 80,
        "width": 300,
        "height": 600
      }
    }
  ]
}
```

Response mapping：写入 `AiResponseSummary.extra.detections`。

### 9.4 `vision.segment`

Request：

```json
{
  "image": {
    "kind": "named_object",
    "obj_id": "chunk:image"
  },
  "prompt": {
    "type": "box",
    "bbox": {
      "format": "xywh",
      "unit": "px",
      "x": 120,
      "y": 80,
      "width": 300,
      "height": 600
    }
  },
  "mask_format": "rle",
  "return_bitmap_mask": false
}
```

Response：

```json
{
  "masks": [
    {
      "id": "mask-1",
      "score": 0.95,
      "bbox": {
        "format": "xywh",
        "unit": "px",
        "x": 120,
        "y": 80,
        "width": 300,
        "height": 600
      },
      "mask": {
        "format": "rle",
        "size": [1024, 1024],
        "counts": "..."
      }
    }
  ]
}
```

Response mapping：结构化 mask 写 `extra.segment`; bitmap mask 写 `artifacts`。

---

## 10. Audio API

### 10.1 `audio.tts`

Request：

```json
{
  "text": "你好，欢迎使用 AICC。",
  "voice": {
    "voice_id": "voice_zh_female_warm_001",
    "language": "zh-CN",
    "gender": "female",
    "style": "warm",
    "speaker_similarity_required": false
  },
  "speed": 1.0,
  "output": {
    "media_type": "audio/mpeg",
    "sample_rate": 44100
  }
}
```

Response：

```json
{
  "audio": {
    "kind": "named_object",
    "obj_id": "chunk:tts-audio"
  }
}
```

Fallback：

1. 如果指定 `voice_id` 且 `speaker_similarity_required=true`，禁止跨 Provider fallback。
2. 如果只指定 language / gender / style，可在满足 voice contract 的 Provider 内 fallback。

### 10.2 `audio.asr`

Request：

```json
{
  "audio": {
    "kind": "named_object",
    "obj_id": "chunk:meeting-audio"
  },
  "language": "zh-CN",
  "timestamps": "segment",
  "diarization": true,
  "output_formats": ["json", "vtt", "srt"]
}
```

Response：

```json
{
  "text": "大家好，欢迎来到今天的会议。",
  "segments": [
    {
      "id": "seg-1",
      "start_seconds": 0.0,
      "end_seconds": 2.4,
      "text": "大家好，欢迎来到今天的会议。",
      "speaker": "SPEAKER_0",
      "confidence": 0.94
    }
  ],
  "artifacts": {
    "vtt": {
      "kind": "named_object",
      "obj_id": "chunk:subtitle-vtt"
    },
    "srt": {
      "kind": "named_object",
      "obj_id": "chunk:subtitle-srt"
    },
    "json": {
      "kind": "named_object",
      "obj_id": "chunk:asr-json"
    }
  }
}
```

Response mapping：

1. transcript 写 `AiResponseSummary.text`。
2. segments 写 `AiResponseSummary.extra.asr.segments`。
3. subtitles 和结构化转录 artifact 写 `AiResponseSummary.artifacts`。

Provider 能够评估语音可靠性时，在 `AiResponseSummary.extra.asr` 中附加 `status`、`speech_detected`、`confidence` 和仅供诊断的 `candidate_text`。`status` 为 `uncertain` 或 `no_speech` 时，候选文字保留在诊断信息中，transcript 正文保持为空。

当 request 指定多个 `output_formats` 时，response `artifacts` 的 key 必须与 format 名一致，例如 `vtt`、`srt`、`json`。

### 10.3 `audio.music`

Request：

```json
{
  "prompt": "A 30-second cheerful acoustic folk song with guitar.",
  "duration_seconds": 30,
  "instrumental": false,
  "lyrics": null,
  "seed": 12345,
  "output": {
    "media_type": "audio/mpeg"
  }
}
```

Response：

```json
{
  "audio": {
    "kind": "named_object",
    "obj_id": "chunk:music"
  },
  "structure": {
    "lyrics": "...",
    "sections": [
      { "name": "intro", "start_seconds": 0, "end_seconds": 8 }
    ]
  }
}
```

默认异步。

### 10.4 `audio.enhance`

Request：

```json
{
  "audio": {
    "kind": "named_object",
    "obj_id": "chunk:noisy-audio"
  },
  "task": "denoise",
  "strength": 0.8,
  "return_stems": false
}
```

Response：

```json
{
  "audio": {
    "kind": "named_object",
    "obj_id": "chunk:enhanced-audio"
  },
  "stems": []
}
```

---

## 11. Video API

视频生成和编辑默认异步，AI method response 返回 task id，最终结果写 task data / event。

### 11.1 `video.txt2video`

Request：

```json
{
  "prompt": "A cinematic tracking shot through a lantern-lit market at dusk.",
  "duration_seconds": 8,
  "aspect_ratio": "16:9",
  "resolution": "720p",
  "generate_audio": true,
  "seed": 12345,
  "output": {
    "media_type": "video/mp4",
    "fps": 24
  }
}
```

Final result：

```json
{
  "video": {
    "kind": "named_object",
    "obj_id": "chunk:generated-video"
  }
}
```

### 11.2 `video.img2video`

Request：

```json
{
  "image": {
    "kind": "named_object",
    "obj_id": "chunk:start-frame"
  },
  "prompt": "Animate the scene with slow camera movement.",
  "duration_seconds": 8,
  "aspect_ratio": "16:9",
  "resolution": "720p"
}
```

Response：同 `video.txt2video`。

### 11.3 `video.video2video`

Request：

```json
{
  "video": {
    "kind": "named_object",
    "obj_id": "chunk:source-video"
  },
  "prompt": "Shift the color palette to teal and warm backlight.",
  "preserve_motion": true,
  "time_range": {
    "start_seconds": 0,
    "end_seconds": 8
  }
}
```

Response：同 `video.txt2video`。

### 11.4 `video.extend`

Request：

```json
{
  "video": {
    "kind": "named_object",
    "obj_id": "chunk:previous-video"
  },
  "prompt": "Continue the shot as the camera rises over the rooftops.",
  "continuation_handle": "runway-gen-abc123",
  "duration_seconds": 7,
  "resolution": "720p"
}
```

Response：同 `video.txt2video`。

### 11.5 `video.upscale`

Request：

```json
{
  "video": {
    "kind": "named_object",
    "obj_id": "chunk:source-video"
  },
  "target_resolution": "4k",
  "denoise": true,
  "sharpen": 0.3,
  "output": {
    "media_type": "video/mp4",
    "fps": 24
  }
}
```

Response：同 `video.txt2video`。

Video fallback：

1. 只能在同 `method` 内 fallback。
2. Provider 一旦返回 Started / Queued，AICC 不再跨 Provider 重试。
3. `extend` 必须保持源视频和 provider operation 的状态一致，默认 strict；如果 Provider 需要上一轮生成状态，必须通过 `continuation_handle` 显式传入。

---

## 12. Agent Runtime API

### 12.1 `agent.computer_use`

`agent.computer_use` 是 `aicc 逻辑模型目录.md` 中的占位方向。它依赖外部环境状态，不建议作为 AICC v0 普通模型调用直接开放。推荐架构：

```text
Agent Runtime / OpenDAN
  -> 管理 environment/session/sandbox
  -> 调用 AICC method agent.computer_use
  -> 执行动作并回传下一帧 observation
```

Request：

```json
{
  "task": "Click the login button and enter the username.",
  "environment": {
    "environment_id": "sandbox-123",
    "session_id": "agent-session-001",
    "screenshot": {
      "kind": "named_object",
      "obj_id": "chunk:screenshot"
    },
    "viewport": {
      "width": 1280,
      "height": 720
    }
  },
  "allowed_actions": [
    "screenshot",
    "left_click",
    "right_click",
    "type",
    "key",
    "scroll",
    "wait"
  ]
}
```

Response：

```json
{
  "actions": [
    {
      "type": "left_click",
      "x": 640,
      "y": 520
    },
    {
      "type": "type",
      "text": "alice@example.com"
    }
  ],
  "requires_next_observation": true
}
```

Fallback：

1. 默认 strict。
2. 不允许静默更换 `environment_id`。
3. 是否可 fallback 由 Agent Runtime 的 session 策略决定，AICC Router 不单独判断 sandbox 可迁移性。

---

## 13. Provider Inventory 要求

每个 Provider instance 必须声明自己支持的 method：

```json
{
  "provider_instance_name": "openai_primary",
  "provider_type": "cloud_api",
  "provider_profile_id": "openai",
  "protocol_adapter_id": "openai-responses",
  "catalog_activation_revision": 42,
  "models": [
    {
      "model_uid": "openai:gpt-5.5:responses",
      "provider_model_id": "gpt-5.5",
      "model_driver_id": "openai",
      "origin_model_id": "gpt-5.5",
      "exact_model": "gpt-5.5@openai_primary",
      "api_types": ["llm.chat"],
      "operations": { "chat.completions.create": "responses.create" },
      "logical_mounts": ["llm.gpt5", "llm.plan", "llm.code", "llm.vision"],
      "capabilities": {
        "tool_call": true,
        "json_schema": true,
        "web_search": true,
        "vision": true,
        "max_context_tokens": 400000
      },
      "attributes": {
        "provider_type": "cloud_api",
        "privacy": "public_cloud",
        "quality_score": 0.95,
        "latency_class": "normal",
        "cost_class": "high"
      },
      "pricing": {},
      "health": {
        "status": "available",
        "quota_state": "normal"
      }
    }
  ]
}
```

约束：

1. `exact_model` 必须是 `<provider_model_id>@<provider_instance_name>`，其中 `provider_model_id` 可携带 `:variant` 后缀（见 §14.3）。
2. `api_types` 和 Model Driver/adapter/discovery 的能力交集是 Router 硬过滤条件。
3. `logical_mounts` 只负责候选展开，不决定 schema。
4. `provider_type` 的可信来源应是 system-config 或 admin override，不能只信 Provider 自声明。
5. `provider_model_id` 不得包含 `@`（可含 `:variant`）；需要表达 HuggingFace revision 等信息时应放入独立字段或 attributes，避免与 `exact_model` 分隔符冲突。
6. `attributes.privacy` 必须使用枚举值：`local`、`private_cloud`、`public_cloud`、`public_cloud_no_log`。

---

## 14. 逻辑目录映射

### 14.1 一级目录

| 一级目录 | 默认 method | fallback |
|---|---|---|
| `llm` | `llm.chat` | parent |
| `embedding` | `embedding.text` / `embedding.multimodal` | strict |
| `rerank` | `rerank` | strict |
| `image` | `image.*` / `vision.*` | same method parent |
| `audio` | `audio.*` | strict 或 voice contract |
| `video` | `video.*` | same method parent |
| `agent` | `agent.*` | strict |

### 14.2 调用方选择模型

普通调用方通过 Helper 使用逻辑路径：

```json
{
  "method": "helper.vision_ocr",
  "params": {
    "logical_model": "image.ocr",
    "image": { "kind": "named_object", "obj_id": "chunk:image" }
  }
}
```

显式两阶段调用先执行 `route.resolve`，再把返回的 exact model 交给 typed inference：

```json
{
  "method": "chat.completions.create",
  "params": {
    "exact_model": "claude-sonnet-4.6@anthropic",
    "messages": []
  }
}
```

解析规则：

1. `route.resolve.logical_model` 必须是不含 `@` 的逻辑路径。
2. typed inference 的 `exact_model` 按 `<provider_model_id>[:variant]@<provider_instance_name>` 解析。
3. exact model 不做 parent fallback 或 runtime failover；Provider 启动失败直接返回失败。
4. logical path fallback 只能改变同一 `api_type` 的候选模型。
### 14.3 Exact model variant 与 Provider Rules lowering

对用户来说，同一 base model 的不同语义档位表现为不同 AICC exact model：

```text
<provider_model_id>[:<variant>]@<provider_instance_name>
```

例如 `gpt-5.1:reasoning-high@openai_primary`。Model Driver variant 定义语义身份；Provider Rules 将该 variant lowering 为当前 Protocol Adapter 的请求参数。Model Driver Metadata 不包含 `provider_options`。

1. `route.resolve` 输出含 variant 的 `selected_exact_model` 和不带 variant 的原始 `provider_model_id`。
2. typed inference 根据 exact model、Provider Rules 和 method 生成内部 `ResolvedProviderCall`；调用方不传 `provider_options`。
3. request rules 的顺序是 Provider defaults、用户 canonical 参数、条件 `set/remove`；用户不能恢复 Provider 明确删除的字段。
4. usage、trace 和 audit 以含 variant 的 exact model 聚合，并记录 Provider Profile、adapter、operation、规则来源和 revision。

---

## 15. 错误模型

标准错误分层：

| 层级 | 表达方式 |
|---|---|
| kRPC transport / auth / parse | `RPCErrors` |
| AICC 启动失败 | `AiMethodResponse.status=failed` + task event |
| Provider 执行失败 | task-manager `Failed` + task event |
| fallback / failover | route trace + task event |

AICC 错误 payload schema：

```json
{
  "code": "provider_error",
  "message": "provider returned rate limit",
  "provider_code": "openai/rate_limit",
  "retriable": true,
  "details": {}
}
```

写入位置：

1. task data 固定写入 `task.data.aicc.error`。
2. task event 的 error data 必须包含同一 payload，便于订阅方即时展示。
3. `AiMethodResponse.status=failed` 且已创建 task 时，调用方应通过 `event_ref` 或 task data 获取该 payload。
4. 早期 kRPC error 不创建 AICC task，使用 kRPC error body 表达。

建议错误码：

| code | 说明 |
|---|---|
| `invalid_request` | 请求格式或字段非法。 |
| `invalid_method` | 未知或不支持的 method。 |
| `schema_validation_failed` | request 未通过 method schema 校验。 |
| `resource_invalid` | 资源不存在、无权限、格式不支持或过大。 |
| `no_provider_available` | 无 Provider 支持指定 method / capability。 |
| `no_candidate_model` | route 过滤后无候选。 |
| `fallback_not_allowed` | fallback 被 policy 或 method 禁止。 |
| `provider_start_failed` | Provider 启动或提交失败。 |
| `provider_error` | Provider 原生错误。 |
| `timeout` | 超时。 |
| `budget_exceeded` | 成本或配额限制。 |
| `policy_denied` | 被 system/user/session policy 拒绝。 |
| `idempotency_conflict` | 同一幂等作用域内重复 key 对应的 canonical request body 不一致。 |
| `cancelled` | 请求或任务已取消。 |
| `internal_error` | 内部错误。 |

---

## 16. 落地顺序

### M0：稳定 kRPC method 入口

1. `/kapi/aicc` 作为稳定入口。
2. AI 调用使用标准 method 名作为 kRPC method。
3. `cancel`、`service.reload_settings` 保持为控制类 method，不定义兼容别名。

### M1：移除独立分类字段

1. 删除 request params 中独立的分类字段。
2. Router 从 kRPC `method` 读取 schema discriminator。
3. Provider Adapter 使用 kRPC `method` 选择 provider-native endpoint。
4. `Capability` 收敛为 namespace 级粗分组，仅用于 RBAC、UI 和 quota。
5. `ProviderInventory.models[].methods` 作为硬过滤。
6. `image.txt2image` / `image.img2image` 等内部旧命名迁移到标准 method 名。

### M2：ResourceRef + FileObject meta

1. `payload` 顶层统一为 `input_json`、`resources`、`options`。
2. `payload.resources` 和各 method schema 中的资源字段统一使用 `ResourceRef`。
3. 文件类资源用 `ResourceRef::NamedObject { obj_id }` 指向 `FileObject`。
4. Router 只读取 `ObjId` 和 `FileObject.meta`。
5. Provider Adapter 只在最后一跳读取资源 bytes。

### M3：逐类实现 schema

优先级建议：

1. `llm.chat` 多模态和 tool schema。
2. `image.txt2img` / `image.img2img`。
3. `audio.asr` / `audio.tts`。
4. `embedding.text` / `rerank`。
5. `vision.ocr` / `vision.caption`。
6. `video.*` 异步任务。
7. `agent.computer_use` 由 Agent Runtime 牵头接入。

每新增一个 method，必须同步：

1. API schema 文档。
2. 标准 method 集合。
3. Provider inventory 声明。
4. Router fallback / policy 规则。
5. Provider Adapter 映射。
6. task-manager 状态和事件。
7. 单元测试或 DV test。

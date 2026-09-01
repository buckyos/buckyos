# AICC 内部模块划分

状态：Beta 2.2 目标规范
范围：第一版内置 Provider

本文定义 AICC 重构后的模块边界。它描述目标方案，不记录当前实现，也不要求兼容旧模块、旧 settings 或旧 metadata。

第一版必须实现以下内置 Provider：

```text
OpenAI / Claude / Gemini / fal / OpenRouter / MiniMax
Kimi / GLM / DeepSeek / 豆包（火山方舟）/ Qwen（阿里云百炼）
```

既有 `sn-openai -> openai-responses` 设计继续作为扩展 Provider 保留，但不计入本次 11 个首版内置 Provider；其动态登录仍只属于 SN dialect/credential runtime，不能进入 OpenAI 基础模块。

相关设计以以下文档为准：

- [aicc_api设计.md](aicc_api设计.md)：控制面、typed inference 与 Helper API；
- [provider_profile_schema.md](provider_profile_schema.md)：Provider Profile、Protocol Adapter、Provider Rules 和价格边界；
- [match_rule.md](match_rule.md)：全局统一匹配规则；
- [aicc_router.md](aicc_router.md)：模型目录、路由、调度和快照；
- [driver_metadata_update_protocol.md](driver_metadata_update_protocol.md)：metadata 更新与 inventory 收敛时序。

## 1. 官方接口调研结论

首版接口选择遵循“官方当前推荐接口优先”。“兼容 OpenAI/Anthropic”只说明可以复用对应 wire codec，不代表认证、模型发现、扩展字段、错误、限制和异步任务完全相同。

| Provider | 首版主接口 | 可复用协议模块 | 必须隔离的厂商差异 |
| --- | --- | --- | --- |
| OpenAI | Responses | `openai/responses` | 官方 endpoint、Bearer key、模型发现；专用 image/audio/video API 独立 operation |
| Claude | Messages | `claude/messages` | `x-api-key`、`anthropic-version`、content block 与 SSE event |
| Gemini | Interactions | `gemini/interactions` | `x-goog-api-key`、interaction/event 结构、Files/Gen Media/Live 等独立接口 |
| fal | Queue API | `fal/queue` | `Authorization: Key`、endpoint 即模型、submit/status/result/cancel/webhook、模型特定输入输出 |
| OpenRouter | Chat Completions | `openai/chat_completions` | 路由参数、归因 header、渠道 metadata、富模型目录与实时价格 |
| MiniMax | Anthropic-compatible Messages | `claude/messages` | `/anthropic` 基址、兼容差异、`base_resp`；speech/image/video/music 为原生接口 |
| Kimi | Chat Completions | `openai/chat_completions` | `partial`、思考内容、缓存 key、图片/视频 content 扩展 |
| GLM | Chat Completions | `openai/chat_completions` | `thinking`、`reasoning_content`、`tool_stream`、JWT 可选鉴权和原生异步 API |
| DeepSeek | Responses | `openai/responses` | thinking/reasoning 约束及兼容差异；官方另有 Chat Completions 和 Anthropic 接口，但首版不因此重复实现 |
| 豆包 | Responses | `openai/responses` | 方舟 `/api/v3` 基址、内置工具、模型/接入点语义及原生媒体任务 |
| Qwen | Responses | `openai/responses` | region/workspace 基址、参数支持子集、session cache header；原生媒体异步任务 |

依据：

- OpenAI 官方示例以 [Responses API](https://platform.openai.com/docs/quickstart/make-your-first-api-request) 为主，并提供独立的 [Models API](https://platform.openai.com/docs/api-reference/models/object)；
- Claude 原生协议是 [Messages API](https://platform.claude.com/docs/en/api/messages)，认证还要求 API key 和 API version header；
- Gemini 已将 [Interactions API](https://ai.google.dev/gemini-api/docs/interactions-overview) 作为新项目默认接口，`generateContent` 保持支持但已属于历史接口；
- fal 官方推荐持久化 [Queue API](https://fal.ai/docs/documentation/model-apis/inference/queue)，完整生命周期包含提交、状态、结果、取消和 webhook；
- OpenRouter 官方入口仍是 [Chat Completions](https://openrouter.ai/docs/quickstart)，其 [Models API](https://openrouter.ai/docs/guides/overview/models) 还返回架构、渠道和价格信息；
- MiniMax 文档推荐 [Anthropic-compatible Messages](https://platform.minimax.io/docs/api-reference/text-chat-anthropic)，媒体能力具有自己的异步接口；
- Kimi 当前主要提供 [Chat Completions](https://platform.kimi.com/docs/api/chat)；
- GLM 的主文本接口是 [Chat Completions](https://docs.bigmodel.cn/api-reference/%E6%A8%A1%E5%9E%8B-api/%E5%AF%B9%E8%AF%9D%E8%A1%A5%E5%85%A8)，并另外提供原生异步调用；
- DeepSeek 官方同时提供 OpenAI/Anthropic 兼容入口，并已声明 [Responses API 能力](https://api-docs.deepseek.com/quick_start/pricing/)；
- 豆包的方舟新能力以 [Responses API](https://www.volcengine.com/docs/82379/1958524) 暴露；
- Qwen 已提供 [OpenAI-compatible Responses API](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-responses)，但明确列出与 OpenAI 的参数和行为差异。

官方接口会继续演进。上表决定首版实现入口，不把模型版本、支持参数和价格硬编码进代码；这些易变事实由 catalog、Provider Rules 和 discovery 管理。

## 2. 划分原则

### 2.1 复用真实协议，不复用厂商品牌

```text
HTTP/SSE/JSON/异步轮询基础设施
  -> 明确 API 代际的 operation codec
    -> 可选的厂商 dialect
      -> Provider Profile 装配
        -> Provider Instance 运行态
```

禁止建立一个按 Provider ID、URL 或模型名分支的“万能 OpenAI-compatible Adapter”。基础 codec 不知道哪些厂商在复用它。

### 2.2 operation 是最小协议复用单位

一个 Adapter 由若干 operation 组合，而不是一个文件包揽厂商全部 API。例如 OpenAI Provider 可以同时绑定：

```text
openai.responses.create
openai.embeddings.create
openai.images.generate / edit
openai.audio.transcribe / speech
openai.videos.create / status / content / cancel
```

只有 request、response、stream 和错误语义相同的 operation 才共享 codec。仅仅都使用 HTTP、JSON 或异步任务，不足以合并成同一个协议实现。

### 2.3 历史接口按真实需求加入一次

首版已经存在真实需求：OpenRouter、Kimi 和 GLM 的官方主入口仍为 Chat Completions，因此实现一份协议族级 `openai-chat-completions`。三家复用这一份 codec，各自只实现差异。

Gemini `generateContent` 等其它历史接口不能为了“兼容完整”预先加入。第一个实际 Provider/operation 确认需要时，新增一份共享历史 Adapter；后续使用同一接口的 Provider 复用它。新旧 Adapter 平级且互不 fallback，只共享更低层基础设施和 canonical IR。

### 2.4 厂商差异优先声明，必要时才写代码

差异分三类：

1. Profile 数据：base URL、固定 header、credential 类型、默认 Adapter；
2. Provider Rules：模型映射、operation、参数映射/删除、能力收窄和价格规则；
3. dialect 代码：基础 schema 无法表达的请求、流事件、错误或任务状态差异。

只存在前两类差异时直接引用基础 Adapter，不创建空壳子类。需要第三类时使用独立 `protocol_adapter_id` 和 `base_adapter_id`，语义上是基础 Adapter 的子类，实现可采用组合或委托。

### 2.5 不用通用扩展 map 掩盖协议差异

Canonical API 不开放任意厂商 JSON。公共字段进入 typed IR；经过评审的少量厂商字段由 Provider Rules 或明确的 dialect 类型承载。不能用 `extra_body: Value` 把协议适配责任推给调用方。

## 3. 顶层模块

```text
aicc
├── service          进程启动、依赖装配、kRPC、优雅退出
├── api              API schema、鉴权、参数校验和 use case
├── error            稳定错误分类及边界转换
├── runtime          不可变 RuntimeSnapshot 与原子发布
├── settings         system-config、CAS revision、候选配置
├── catalog          NDN catalog 解析、索引和 target seq
├── matching         MatchRule schema、编译和唯一执行器
├── model            ModelUID、能力、variant、ModelRegistry
├── provider         Profile、Rules、内置装配、discovery、inventory、生命周期
├── protocol         transport、canonical IR、基础 codec、dialect、原生协议
├── routing          候选、过滤、调度、fallback、trace
├── call             RouteDecision 到 ResolvedProviderCall 的唯一 lowering
├── execution        immediate/stream/task、取消、幂等、TaskMgr bridge
├── resource         ResourceRef 鉴权、限制和最后一跳物化
├── storage          inventory LKGS、usage/audit/task 关联存储接口
└── observability    metrics、trace、审计、诊断和脱敏
```

首轮不新增 workspace crate。先在 AICC service crate 内使用 Rust module 和 `pub(crate)` 控制依赖；只有独立发布或被多个 crate 稳定复用时再评估拆 crate。

## 4. `protocol` 模块

### 4.1 目录

```text
protocol/
├── adapter          ProtocolAdapter、OperationDescriptor、registry
├── transport
│   ├── http         client、timeout、proxy、retry-after、request id
│   ├── sse          framing、断线、终止标记；不解释业务 event
│   ├── json         有界反序列化和未知字段策略
│   ├── multipart    上传与流式读取
│   └── websocket    仅在选中的 realtime operation 需要时实现
├── auth
│   ├── bearer_key
│   ├── named_header_key
│   ├── fal_key
│   └── glm_jwt      可选；从 GLM API key 生成短期 JWT
├── ir               typed request/response/event/error
├── task             polling、deadline、backoff、cancel、webhook 接口
├── openai
│   ├── responses
│   ├── chat_completions
│   ├── embeddings
│   ├── images
│   ├── audio
│   └── videos
├── claude
│   └── messages
├── gemini
│   ├── interactions
│   ├── embeddings
│   ├── files
│   └── gen_media
├── fal
│   └── queue
├── native
│   ├── minimax_media
│   ├── glm_async
│   ├── doubao_media
│   └── dashscope_media
└── dialect
    ├── openrouter_chat
    ├── minimax_messages
    ├── kimi_chat
    ├── glm_chat
    ├── deepseek_responses
    ├── doubao_responses
    └── qwen_responses
```

目录表示职责，不要求每个叶子都立即拆成文件。首版只实现已经映射到 AICC ApiType 且通过合同测试的 operation。

### 4.2 基础 codec 与 dialect

基础 codec 独占 endpoint path、基础 schema、SSE 业务 event、usage/tool/finish reason、协议错误映射和 operation 能力声明。它不管理 credential，不做 discovery，不读 Provider Rules，不按模型名猜操作。

Dialect 是窄委托层：

```text
ResolvedProviderCall
  -> dialect request validation/transform
  -> base operation codec
  -> dialect response/event/error normalization
```

| Dialect | Base | 只负责 |
| --- | --- | --- |
| `openrouter-openai` | `openai-chat-completions` | provider routing、归因/metadata header、渠道结果扩展 |
| `minimax-messages` | `claude-messages` | 兼容差异、`base_resp`、MiniMax content 扩展 |
| `kimi-chat` | `openai-chat-completions` | partial/cache/reasoning 与多模态扩展 |
| `glm-chat` | `openai-chat-completions` | thinking、tool stream、reasoning 与错误扩展 |
| `deepseek-responses` | `openai-responses` | DeepSeek thinking/tool 约束和扩展事件 |
| `doubao-responses` | `openai-responses` | 方舟内置工具、事件与参数差异 |
| `qwen-responses` | `openai-responses` | 支持参数子集、session cache、事件差异 |

每个 dialect 必须声明 `base_adapter_id`、覆盖点和不支持能力，且不能复制基础 request schema、SSE parser 和 contract tests。若官方 wire 行为可完全由 Profile/Rules 表达，应删除该 dialect 并直接绑定 base。

### 4.3 原生异步协议

`protocol::task` 只提供生命周期算法，不假设字段名。每个原生 operation 显式映射厂商状态到 `Submitted | Queued | Running | Succeeded | Failed | Cancelled`。

fal Queue、MiniMax video、GLM async、豆包媒体和 Qwen/DashScope 媒体共享 deadline/backoff/cancel/idempotency 机制，但各自保留 submit/status/result/cancel codec。任务开始后绑定原 Provider runtime 和 Adapter，不跨 Provider 重试。

## 5. `provider` 模块

### 5.1 目录与职责

```text
provider/
├── profile          Provider Profile descriptor
├── rules            Provider Rules 编译结果
├── instance         settings 与运行态组合
├── registry         ExecutableProviderInstance 不可变索引
├── discovery        openai/claude/gemini/openrouter models 与 catalog_only
├── inventory        build、LKGS、seq 收敛、refresh loop
├── lifecycle        start/stop/replace 与迟到写保护
└── builtin
    ├── openai       ├── claude      ├── gemini
    ├── fal          ├── openrouter  ├── minimax
    ├── kimi         ├── glm         ├── deepseek
    ├── doubao       └── qwen
```

`builtin/<provider>` 是装配模块，不是协议实现。它只提供稳定 ID、默认 endpoint 模板、区域/workspace 和 credential schema、discovery、operation/Adapter 默认绑定、catalog 入口及必要 dialect/native module 注册。

### 5.2 首版装配矩阵

| Provider | 默认 Adapter 组合 | Discovery/价格策略 |
| --- | --- | --- |
| OpenAI | Responses + embeddings/images/audio/videos | `/v1/models`；价格由 Provider Rules，动态事实优先 |
| Claude | Messages | Claude Models API；价格由 Provider Rules |
| Gemini | Interactions + embeddings/files/gen-media | Gemini Models API；价格由 Provider Rules |
| fal | Queue | catalog 给出 model endpoint；运行时探测可用性，schema 保持 model-specific |
| OpenRouter | OpenRouter Chat dialect | Models API；使用动态模型、能力和实时价格 |
| MiniMax | MiniMax Messages dialect + native media | Anthropic-compatible Models API；媒体由 catalog/rules 补充 |
| Kimi | Kimi Chat dialect | Kimi Models API；价格由 Provider Rules |
| GLM | GLM Chat dialect + native async | 有官方机器接口时 discovery，否则 catalog；不得爬取文档页 |
| DeepSeek | DeepSeek Responses dialect | 有官方机器接口时 discovery，否则 catalog；动态价格优先、静态规则 fallback |
| 豆包 | Doubao Responses dialect + native media | 可调用模型/接入点由官方 API 或实例配置获得，catalog 补足稳定语义 |
| Qwen | Qwen Responses dialect + native media | region/workspace 参与 endpoint；其余事实由官方 API 或 catalog/rules 给出 |

Discovery 只采信官方机器接口，不能抓网页或读取 SDK 内置列表构造库存。动态 discovery 返回 availability、remote methods、deprecated、health 和实时价格；Model Driver 仍是稳定语义真相源。

### 5.3 鉴权组合

| 认证原语 | 使用者 |
| --- | --- |
| Bearer API key | OpenAI、OpenRouter、Kimi、GLM、DeepSeek、豆包、Qwen |
| named header API key | Claude、Gemini、MiniMax Anthropic-compatible |
| `Authorization: Key` | fal |
| derived short-lived token | GLM JWT，可选；由 credential provider 生成 |

固定 API version、session cache、归因等 header 不是 credential，分别由基础 Adapter、dialect 或 Profile 负责。日志和 trace 只能记录 credential 类型与匿名引用。

## 6. 核心边界

### 6.1 `matching`、`model` 与 `catalog`

全局只存在一份 `MatchRule -> CompiledMatchRule -> match(context)`。普通配置保持 wildcard 字符串，多维条件才用对象。

`model` 只拥有 ModelUID、exact model、capability、variant 和最终模型索引，不知道 endpoint、credential 和 wire schema。`catalog` 拥有 Model Driver、Provider Rules、Known Provider 的 DTO、revision 和解析索引。最终能力为：

```text
Model Driver 静态能力
∩ Adapter operation 能力
∩ Provider discovery/instance 动态能力
```

### 6.2 `call`

`call` 是进入协议层的唯一 lowering：

```text
RouteDecision + canonical request
+ Model Driver variant + Provider Rules
+ selected Adapter operation
= ResolvedProviderCall
```

所有参数优先级、删除规则和资源需求在此确定。Adapter 不再按 Provider/model 猜测 operation。

### 6.3 `runtime`、inventory 与竞争保护

Provider add/reload/metadata refresh 在不可见候选区完成校验、credential、discovery、inventory 和 Router 索引构建，成功后一次性替换完整 `Arc<RuntimeSnapshot>`。请求只捕获一次快照，因此只会看到旧代或新代，不会看到半加入 Provider。

metadata target seq 推进后，推理前或任一 Provider 定时刷新时统一刷新所有 applied seq 落后的库存。实例只有在新 inventory 真正提交后才推进 applied seq；失败时继续使用旧 inventory。

禁用、删除、替换 Provider 或服务退出时，向该实例刷新循环发送幂等 Stop 并等待退出；停止后的迟到写由 generation token 拒绝。

### 6.4 `execution` 与 `resource`

`execution` 统一 immediate、stream 和 task-backed 外部语义，Adapter 只实现原生 wire 生命周期。`resource` 在选定 Provider 后进行最后一跳读取、MIME/大小校验和上传，Router 只能读取资源元数据。

## 7. 关键接口与依赖

接口名为设计示意：

```rust
trait OperationCodec: Send + Sync {
    fn descriptor(&self) -> &OperationDescriptor;
    fn encode(&self, call: &ResolvedProviderCall) -> Result<HttpRequest, AiccError>;
    async fn decode(&self, response: HttpResponse) -> Result<AdapterExecution, AiccError>;
}

trait ProtocolDialect: Send + Sync {
    fn descriptor(&self) -> &DialectDescriptor;
    fn transform_request(&self, call: &mut ResolvedProviderCall) -> Result<(), AiccError>;
    fn normalize_event(&self, event: ProtocolEvent) -> Result<ProtocolEvent, AiccError>;
}

trait ProviderDiscovery: Send + Sync {
    async fn discover(&self, ctx: &DiscoveryContext)
        -> Result<ProviderDiscoverySnapshot, AiccError>;
}
```

约束：codec 不读 inventory/settings；dialect 不重新路由或改变 Adapter 代际；discovery 不编码推理请求；credential provider 不解释业务请求；Provider Rules 不访问网络；handler 不直接调用厂商模块。

```text
api/service -> use cases/runtime
routing     -> model + provider read-only views
call        -> model + provider rules + protocol descriptors/IR
execution   -> protocol + resource + storage
provider    -> catalog + model + matching + storage
dialect     -> declared base codec + protocol primitives
base codec  -> transport + resolved credential + IR
```

禁止 `protocol -> routing`、`model -> provider`、基础 codec 引用 dialect、按 Provider ID 选择分支，或通过全局 `AIComputeCenter` 绕过边界。

## 8. 测试划分

```text
tests/
├── protocol_contract      每个 API 代际/operation 一套 golden + stream contract
├── dialect_contract       基础合同复用 + 仅厂商差异断言
├── provider_builtin       11 家装配、credential、endpoint、discovery fixture
├── provider_inventory     LKGS、seq、refresh、Stop、迟到写
├── routing                exact/logical、能力过滤、fallback、trace
├── runtime_snapshot       add/reload/refresh 与并发请求
├── execution              immediate/stream/task/cancel/idempotency
└── resource_security      鉴权、限制、上传和脱敏
```

OpenRouter/Kimi/GLM 共同运行 Chat Completions 基础合同；DeepSeek/豆包/Qwen 共同运行 Responses 基础合同；MiniMax 运行 Claude Messages 基础合同。每个 dialect 只增加官方差异断言。在线 smoke test 使用独立 credential，不进入默认 `cargo test`。

## 9. 实施顺序

1. canonical IR、error、HTTP/SSE、credential 和 task polling；
2. `openai-responses`、`claude-messages`、`gemini-interactions`；
3. 因 OpenRouter/Kimi/GLM 的真实需求实现一份 `openai-chat-completions`；
4. 11 个轻量 builtin Provider 装配和 discovery；
5. 用合同测试判断七个候选 dialect，能用数据表达的差异不写代码；
6. fal Queue 和首版实际 ApiType 所需的专用/原生 operation；
7. catalog、inventory、RuntimeSnapshot、routing 和 call lowering；
8. execution/resource/storage/observability 与管理 API；
9. metadata seq、定时刷新、Stop 和并发测试；
10. 删除旧 Provider 单体实现和临时兼容入口。

## 10. Review Checklist

- [ ] 复用的是同一 wire operation，还是仅名称相似？
- [ ] 历史 API 是否已有首个真实 Provider 需求？
- [ ] 第二个 Provider 是否复用同一基础 codec 和合同测试？
- [ ] 厂商差异能否由 Profile/Rules 表达，避免空 dialect？
- [ ] dialect 是否声明 base 且只覆盖差异？
- [ ] 基础 codec 是否完全不知道派生 Provider？
- [ ] 是否把任意厂商 JSON 暴露到公共 API？
- [ ] discovery 是否来自官方机器接口而非网页抓取？
- [ ] 异步任务是否显式映射终态、取消和重试？
- [ ] Provider add/reload 是否原子发布一个组合快照？
- [ ] refresh/stop 是否防止迟到写？
- [ ] 是否引入新 crate 或第三方依赖？如有，必须先确认。

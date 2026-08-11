# AICC 新 API 分层设计

本文档描述 AICC breaking change 版本中的 API 分层方向。核心目标是把“逻辑模型路由”和“物理模型推理”拆开，让 AICC 本体接口具有更明确、可解释、行为稳定的控制面。

## 1. 背景问题

现有 AICC 推理接口同时承担两类职责：

- 接收逻辑模型名，执行模型路由。
- 接收请求 payload，启动真实 provider 推理。

这种 all-in-one 接口对调用者很方便，但语义不够清楚：

- 调用者无法明确知道自己是在请求一个逻辑模型，还是指定一个确定物理模型。
- 路由选择、fallback、policy、runtime health、quota、provider lowering 都混在一次推理调用里。
- 想单独检查“这个逻辑模型名当前会路由到哪个物理模型”时，没有稳定控制面接口。
- 不同 API 形态（LLM、文生图、语音、视频等）被包进统一请求结构后，类型约束较弱。

### 1.1 黑盒测试边界问题

all-in-one 接口也让黑盒测试很难写出高信心用例。

从 provider 覆盖测试角度看，测试希望只通过 API 完整跑一遍所有 provider 模型。但当前接口先经过逻辑模型路由，再进入 provider 推理。黑盒测试只能看到一个统一推理入口，无法稳定指定“这一次一定要命中某个 provider 的某个物理模型”，因此很难通过纯 API 覆盖所有 provider model。

从路由测试角度看，测试希望只通过 API 验证路由语义，例如 capability 过滤、成本优先、local only、fallback、session overlay 等。但当前接口会直接启动真实推理，路由结果和 provider 执行结果混在一起。为了构造一个典型路由逻辑，测试环境往往需要准备足够多的 provider、模型能力、quota、health、成本与延迟状态，最终测试会被迫理解并操控组件内部状态。

这导致两类测试都容易退化成白盒测试：

- provider 模型覆盖测试无法稳定控制目标物理模型。
- 路由逻辑测试无法只观察路由控制面结果。

根本问题是 API 边界太难控制。黑盒测试只应该理解接口输入输出，但现有接口没有把“选择模型”和“执行推理”分成两个可观察、可控制的行为。因此测试很难定义出足够可信的用例。

## 2. 设计目标

新设计把 AICC API 拆成三层：

1. Helper 接口：保留现有易用调用体验，但退化为客户端空间的组合 helper。
2. 模型路由接口：给定逻辑模型名和请求约束，返回确定的物理模型选择。
3. 推理接口：只针对确定物理模型执行一次稳定推理，接口按 API 形态拆分并强化类型。

## 3. 总体原则

- AICC 本体推理接口不承载逻辑路由能力。
- 逻辑模型名只出现在模型路由控制面，不直接进入真实推理接口。
- 真实推理接口只接受确定物理模型名。
- 多 Provider 架构天然存在 TOCTOU 问题，AICC 不通过 route lease 承诺路由结果长期有效；调用方可以自行决定失败后是否重新路由并重试一次。
- Helper 接口可以继续提供“传逻辑模型名并得到结果”的体验，但它不属于 AICC 本体核心语义。
- 这是 breaking change，不为旧 all-in-one 语义做长期兼容。

## 4. Helper 接口

现有 AICC 推理接口退化成客户端空间 helper。

Helper 的行为：

```text
logical model request
-> route.resolve
-> exact physical model + provider lowering options
-> typed inference API
-> response
```

也就是说，helper 本身不拥有独立路由逻辑，只是把控制面和数据面串起来。

示例语义：

```text
client.llm_chat({
  model: "llm.chat",
  requirements,
  disable,
  policy,
  messages,
})
```

实际展开为：

```text
route.resolve(api_type="llm.chat", logical_model="llm.chat", requirements, disable, policy)
chat.completions.create(exact_model=route.selected_exact_model, provider_options=route.provider_options, messages)
```

Helper 可以存在于：

- Agent SDK
- Web SDK
- CLI tools
- workflow adapter

但 AICC service 的核心接口不应再把它作为主要协议。

## 5. AICC 本体接口

AICC 本体接口分成两块：

1. 模型路由。
2. 推理接口。

### 5.1 模型路由接口

模型路由接口属于控制面。

它接收一个当前请求的路由相关信息和一个逻辑模型名，返回一个确定的物理模型名，以及解释为什么选择它。

建议接口：

```text
route.resolve
```

输入：

```text
RouteResolveRequest
  request_id
  api_type
  logical_model
  requirements
  disable
  policy
  estimated_input_tokens
  estimated_output_tokens
  session_id
  session_profile
```

输出：

```text
RouteResolveResponse
  selected_exact_model
  provider_instance_name
  provider_driver
  provider_model_id
  provider_options
  enabled_capabilities
  disabled_capabilities
  fallback_attempts
  route_trace
  inventory_revision
  session_config_revision
```

其中：

- `selected_exact_model` 是 AICC 语义下的确定物理模型名，例如 `gpt-5.1@openai-primary`。
- `provider_model_id` 是 provider wire protocol 中真正使用的模型名。
- `provider_options` 是 route / metadata / variant lowering 后得到的 provider 调用参数建议，例如 reasoning effort。
- `fallback_attempts` 是路由器建议的候选顺序，供 helper 或调用方在失败后自行决定是否重试。
- `route_trace` 用于解释候选过滤、policy 命中、session overlay、成本/延迟/health 选择原因。

#### TOCTOU 处理原则

两阶段调用存在 TOCTOU 问题：路由时可用的模型，到推理时可能 quota exhausted 或 health 变化。

这是多 Provider 架构的固有属性，不在 API 层做 lease 承诺。`route.resolve` 只表达“当前观察下的路由选择”，不保证随后推理一定成功。

数据面推理接口只接受 `exact_model`。如果推理失败，调用方可以选择：

- 原地重试同一个 `exact_model`。
- 重新调用 `route.resolve`，拿到新的 `selected_exact_model` 后再推理。
- 使用 `fallback_attempts` 中的候选，自行尝试下一个物理模型。

#### provider options 处理原则

两段式 API 下，路由层和推理层之间不做隐藏状态传递。

逻辑模型名里可以包含一些 option 控制。`route.resolve` 会把这些控制展开成 `provider_options`，作为“路由结果的一部分”返回给调用方。

调用方拿到路由结果后有两种选择：

- 原样把 `provider_options` 透传给第二层推理接口。
- 修改 `provider_options` 或与自己的 request options 合并后，再调用第二层推理接口。

第二层推理接口不关心这些 options 来自哪里。它只按 per request 语义执行：这个 request 给了什么 `exact_model` 和 options，它就用什么 `exact_model` 和 options。第二层接口不感知逻辑模型名，也不感知路由层存在。

因此不存在“provider options patch 与用户 options 冲突时谁优先”的 AICC 本体规则。优先级属于调用方/helper 的合并策略，而不是推理接口协议的一部分。

### 5.2 推理接口

推理接口属于数据面。

它不接收逻辑模型名，不做逻辑路由。它只针对一个确定物理模型执行一次稳定推理。

新的推理接口不再追求 all-in-one，而是按 API 形态拆分，让类型更强。

接口命名尽量贴近行业开创者已经建立的资源语义。AICC 不必完全复制某一家 provider 的 wire protocol，但命名上应优先采用开发者熟悉的形态，例如 `chat.completions.create`、`images.generate`、`embeddings.create`、`audio.transcriptions.create`。

#### LLM 推理接口

示例接口：

```text
chat.completions.create
completions.create
```

输入示例：

```text
LlmChatInvokeRequest
  exact_model
  messages
  tools
  response_format
  temperature
  max_output_tokens
  provider_options
  idempotency_key
  task_options
```

输出示例：

```text
LlmChatInvokeResponse
  task_id
  status
  message
  tool_calls
  usage
  cost
  finish_reason
  provider_task_ref
  route_trace
```

#### 文生图推理接口

示例接口：

```text
images.generate
```

输入示例：

```text
TextToImageInvokeRequest
  exact_model
  prompt
  negative_prompt
  size
  quality
  style
  seed
  output
  provider_options
  idempotency_key
  task_options
```

输出示例：

```text
TextToImageInvokeResponse
  task_id
  status
  artifacts
  usage
  cost
  provider_task_ref
  route_trace
```

#### 其他 API 形态

后续可以继续拆分：

- `images.edit`
- `images.inpaint`
- `images.upscale`
- `vision.ocr`
- `audio.speech.create`
- `audio.transcriptions.create`
- `videos.generate`
- `videos.edit`

每类接口应根据领域输入输出定义强类型结构，而不是统一塞入 `input_json`。

## 6. 逻辑模型名与物理模型名边界

逻辑模型名：

- 只属于 route control plane。
- 表达使用侧需求组合，例如 `llm.chat`、`llm.plan`、`llm.code`。
- 可受 session profile、logical tree、policy、runtime state 影响。

物理模型名：

- 只属于 inference data plane。
- 是一次真实 provider 调用的稳定目标。
- AICC exact model 形式建议继续使用：

```text
provider_model_id@provider_instance_name
```

例如：

```text
gpt-5.1@openai-primary
claude-sonnet-4-5@anthropic-main
```

## 7. Provider Options Lowering

模型路由不仅选物理模型，也可以生成 provider options。

例如 reasoning variant：

```text
AICC exact model: gpt-5.1:reasoning-high@openai-primary
provider_model_id: gpt-5.1
provider_options:
  reasoning:
    effort: high
```

这样 usage / audit 可以按 AICC exact model 聚合，而 provider wire request 仍然使用 provider 原生参数。

`provider_options` 不是 lease，也不是第二层推理接口必须保护的约束。它是 route output 的一部分，调用方可以选择原样使用，也可以修改后再调用物理模型推理接口。

## 8. 迁移策略

Breaking change 版本可以采用以下迁移顺序：

1. 新增 `route.resolve`。
2. 新增按 API 形态拆分的 typed inference interfaces。
3. 将现有 all-in-one API 移到 SDK / CLI helper 层。
4. workflow、Agent tools、UI 逐步改为使用 helper 或显式两阶段调用。
5. AICC service 内部逐步移除推理接口对逻辑模型名的直接支持。

## 9. 第一版最小实现

第一版不需要一次性覆盖所有 modality。

建议先实现：

- `route.resolve`
- `chat.completions.create`
- `images.generate`
- SDK helper:
  - `helper.llm_chat`
  - `helper.text_to_image`

第一版 route response 至少包含：

```text
selected_exact_model
provider_instance_name
provider_model_id
provider_options
fallback_attempts
route_trace
```

第一版推理接口至少支持：

```text
exact_model
payload
provider_options
```

## 10. 待确认问题

- exact model 调用是否完全禁止 fallback。是
- helper 是否保留在 AICC service 内作为 `service.helper.*`，还是完全移到 SDK。
- helper 层是否需要提供一个标准 options merge 策略。
- route.resolve 是否需要支持 dry-run cost estimate 的输入 token 明细。

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
-> exact physical model + complete channel/model identity
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
chat.completions.create(exact_model=route.selected_exact_model, messages)
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
  provider_profile_id
  protocol_adapter_id
  model_driver_id
  origin_model_id
  provider_model_id
  operation
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
- `model_driver_id` / `origin_model_id` 表达模型固有语义；`provider_profile_id` / `protocol_adapter_id` / `operation` 表达实际交付渠道。
- `provider_model_id` 始终保存 Provider discovery 返回并用于真实调用的原始模型名。
- `fallback_attempts` 是路由器建议的候选顺序，供 helper 或调用方在失败后自行决定是否重试。
- `route_trace` 用于解释候选过滤、policy 命中、session overlay、成本/延迟/health 选择原因。

#### TOCTOU 处理原则

两阶段调用存在 TOCTOU 问题：路由时可用的模型，到推理时可能 quota exhausted 或 health 变化。

这是多 Provider 架构的固有属性，不在 API 层做 lease 承诺。`route.resolve` 只表达“当前观察下的路由选择”，不保证随后推理一定成功。

数据面推理接口只接受 `exact_model`。如果推理失败，调用方可以选择：

- 原地重试同一个 `exact_model`。
- 重新调用 `route.resolve`，拿到新的 `selected_exact_model` 后再推理。
- 使用 `fallback_attempts` 中的候选，自行尝试下一个物理模型。

#### Provider call 解析原则

`route.resolve` 不向调用方暴露可任意修改的 Provider options。语义 variant 已编码在 `selected_exact_model` 中；数据面根据 exact model、canonical request、Provider Rules 和当前 catalog revision 生成内部 `ResolvedProviderCall`：

```text
provider_model_id
provider_profile_id
protocol_adapter_id
model_driver_id
origin_model_id
operation
resolved_options
pricing + source
rule/catalog revision
```

operation 按 `method > api_type > adapter default` 解析，并必须存在于 adapter 注册表。请求规则按 Provider defaults、用户 canonical 参数、条件 set/remove 执行。该对象只存在于一次请求生命周期，不是公开配置、inventory 真相源或 route lease。
### 5.2 推理接口

推理接口属于数据面。

它不接收逻辑模型名，不做逻辑路由。它只针对一个确定物理模型执行一次稳定推理。

新的推理接口不再追求 all-in-one，而是按 API 形态拆分，让类型更强。

接口命名尽量贴近行业开创者已经建立的资源语义。AICC 不复制某一家 Provider 的 wire protocol；`chat.completions.create` 是稳定的 provider-neutral method，可以映射到 Responses、Messages、Interactions 或旧兼容 Adapter。实际接口由 `protocol_adapter_id + operation` 决定。

#### LLM 推理接口

示例接口：

```text
chat.completions.create
embeddings.create
rerank.create
images.generate / images.edit / images.upscale / images.remove_background
vision.ocr / vision.caption / vision.detect / vision.segment
audio.speech.create / audio.transcriptions.create / audio.music.create / audio.enhance
videos.generate / videos.transform / videos.extend / videos.upscale
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

## 7. Exact Model Variant Lowering

Model Driver 定义 variant 的语义身份，Provider Rules 定义该身份在当前渠道和 adapter 下的参数 lowering。例如：

```text
AICC exact model: gpt-5.1:reasoning-high@openai-primary
provider_model_id: gpt-5.1
provider_profile_id: openai
protocol_adapter_id: openai-responses
operation: responses.create
resolved_options.reasoning.effort: high
```

调用方只传 exact model 和 canonical request。Provider-specific options 不属于公开数据面协议；usage、audit 和 trace 按含 variant 的 exact model 聚合，并记录规则来源和 revision。
## 8. Beta 2.2 切换策略

Beta 2.2 采用一次性切换，不保留向前兼容：

1. 为全部公开 `api_type` 建立 typed inference request/response。
2. Helper 改为 `logical_model + typed business fields`，内部严格组合 route 和 typed inference。
3. SDK、workflow、Agent tools、UI 和 DV tests 同步迁移。
4. 删除 AICC service 中的 all-in-one methods、`AiMethodRequest`、`model.alias`、`must_features`、`requirements.extra.disable_capabilities` 和 `provider_options` 公共输入。
5. 管理面只保留 `service.reload_settings`，删除所有兼容别名和错误拼写。
6. Provider Profile、Protocol Adapter、Model Driver、Provider Rules、Pricing 和 Known Provider catalog 同时切换到新身份与 schema。

## 9. 验收约束

- route response 和 trace 同时包含 Provider Instance/Profile、Protocol Adapter、Model Driver、origin model、原始 provider model、operation、规则及价格来源。
- exact model 数据面不做逻辑 fallback；TOCTOU 失败由调用方重新 route。
- OpenAI-compatible Provider 调用 Claude 模型时使用 Claude Model Driver 语义，但不得应用 GPT 特判或切换 Anthropic endpoint。
- 同一 origin model 在不同 Provider 可以解析出不同 operation 和价格。
- Adapter 执行层不按模型家族字符串或 base URL 猜测 operation。
- 业务结果精确保存；日志与 provider I/O 摘要独立脱敏。
- 所有 hard constraints 要么正确映射，要么在发起 Provider HTTP 请求前明确失败。

# AICC 新增 Provider 开发指南

> 状态：Beta 2.2 目标规范。本文描述重构后的扩展方式，不沿用 `provider_driver` 或每厂商一套 settings section 的实现模式。

## 1. 先判断需要增加什么

新增渠道或模型时，依次判断：

1. 已有 Provider Profile、Protocol Adapter 和 Model Driver 均适用：只新增 Provider Instance。
2. 渠道的认证声明、默认 `base_url`、模型别名、价格、operation 选择或其它常规差异不同：新增该厂商独立的 `.provider.json` / Provider Rules。
3. 上游 HTTP、SSE、异步任务或错误协议不同：先判断能否增加统一、受限的 `.provider.json` 声明；仍无法安全表达时才实现并注册 Protocol Adapter。
4. 模型语义、variants、能力或参数约束不同：新增或更新 Model Driver catalog。
5. 新增 AICC 业务能力：先扩展 typed API 和 operation registry，再实现 Driver、Adapter 与验收用例。

渠道身份、传输协议和模型语义必须分别建模，不能重新合并成一个 Provider driver。

## 2. 目标组件

### Provider Profile

定义渠道级事实：显示信息、默认 `base_url`、认证方式、默认 Adapter、discovery 与 UI hints。首版内置 Profile 包括 OpenAI、Claude、Gemini、fal、OpenRouter、MiniMax、Kimi、GLM、DeepSeek、豆包和 Qwen，SN 作为扩展 Profile 保留；配置型 Profile 只能引用程序已经注册的 Adapter。

### Provider Rules

负责 `ModelUID + variant + api_type` 到 `provider_model_id + operation + resolved options` 的渠道映射，也承载渠道价格上下文。Rules 生成的参数只进入内部 `ResolvedProviderCall`，不能暴露为公开 `provider_options`。

### Protocol Adapter

只处理协议：请求编码、认证、传输、流式/异步任务、响应解析、错误和取消。Adapter 不负责逻辑模型路由，也不根据模型名猜测能力。

OpenAI、Claude、Gemini 必须分别实现协议族，并按 API 代际注册独立 Adapter。基础协议优先实现 Responses、Messages、Interactions 等官方新接口；Chat Completions、旧 Completions 或 `generateContent` 等历史实现只在首个真实 Provider 需求出现时增加，不为完整覆盖历史而预先实现。新增后它是协议族级共享 Adapter，后续使用同一历史接口的 Provider 直接复用，不能各自实现一遍。

新旧接口 Adapter 平级且互不 fallback，只共享协议中立的 transport、SSE、JSON、错误工具和 normalized IR。Provider 没有渠道差异时直接使用已有 Adapter；渠道差异优先写入独立 `.provider.json`。只有不同 wire envelope、认证算法、流事件、任务状态机或无法声明化的错误语义仍需代码时，内置厂商才注册独立派生 Adapter并声明 `base_adapter_id`。多个派生 Adapter 可以复用同一个基础或历史 Adapter，实现可以继承、组合或委托。基础 Adapter 不允许识别派生 Provider。

SN 的标准示例是 `sn-openai -> openai-responses`：SN 层实现 `api_key` 或 `dynamic_login` 认证，Responses 层只执行基础协议。新增类似内置厂商时必须保持相同的单向依赖和可拆除性。

### Model Driver

定义模型的稳定语义：ModelUID、origin model、variants、结构化能力、上下文限制、参数约束和支持的 AICC api types。Driver 不包含渠道凭据、`base_url` 或厂商请求模板。

## 3. Provider Instance 配置

所有实例使用统一数组：

```json
{
  "providers": [
    {
      "provider_instance_name": "openai-work",
      "provider_type": "cloud_api",
      "provider_profile_id": "openai",
      "protocol_adapter_id": "openai-responses",
      "base_url": "https://api.openai.com/v1",
      "credentials": {
        "api_token": { "locked": "..." }
      },
      "region": "global",
      "enabled": true
    }
  ]
}
```

不使用 Provider family section、`instances[]` 包装、`provider_driver`、settings 中的 `endpoint`、section 级 token、`features` 或字段别名。`base_url` 是 Provider Instance settings 的正式字段；Profile 默认值只用于创建表单，不能覆盖实例显式配置。

用户通过管理 RPC 添加自定义 Provider 时可以只提交协议族、`base_url` 和凭据；registry 解析族默认 Adapter。需要指定已注册的历史/派生协议时可同时提交 `protocol_adapter_id`，但它必须属于所给协议族。例如：

```json
{
  "provider_instance_name": "compat-router",
  "provider_profile_id": "custom",
  "protocol_family_id": "openai",
  "base_url": "https://compat.example/v1"
}
```

这种 `custom` Provider 默认使用空 Provider Rules `{}`。接入测试解析出的 Adapter 只决定调用协议；discovery 返回的每个 `provider_model_id` 保持原样，并在系统当前安装的全部 Model Driver 中唯一匹配原厂 metadata。系统不自动删除 `openai/` 等前缀，也不复用 OpenRouter 等官方 Provider 的 origin mapping；需要非标准命名映射时，必须把该渠道升级为有独立 `.provider.json` 的官方支持 Provider。

`provider.validate` / `provider.add` 通过 registry 校验协议族与 Adapter 的归属关系并保存确定的 `protocol_adapter_id`。当前不会通过发送推理请求盲探多个协议；调用方未显式指定 Adapter 时使用该协议族注册的默认项。该解析只发生在创建或更新阶段，运行时不得再次猜测或 fallback。

## 4. 接入步骤

1. 将渠道加入官方支持范围时，为它创建独立 `.provider.json`，在 Known Provider catalog 定义认证声明、默认 `base_url`、Adapter、`discovery_behavior_id` 和 UI schema；仅在需要时声明 `dynamic_login_behavior_id` / `connection_behavior_id`。用户自行添加的 `custom` Provider 不创建伪官方 catalog，使用标准空规则 `{}`。
2. 如 Provider 需要尚未实现的新协议或历史接口，在 Adapter registry 按需注册固定 `protocol_adapter_id` 和支持的 operations；不要为了覆盖厂商历史而预先实现未被使用的 Adapter。
   若只是兼容旧 API，则新增一份协议族级共享历史 Adapter，不修改官方新接口 Adapter，也不增加运行时协议 fallback；同时把它加入该协议族的接入测试候选顺序。
   若共享 Adapter 已存在且渠道没有差异，直接引用它；有认证、endpoint、参数或能力差异时先写入 `.provider.json`。只有无法安全声明化的执行差异才增加派生 Adapter，声明 `base_adapter_id`，并只实现剩余的最小逻辑差异层。
   如现有 discovery 行为可复用，直接引用已注册的稳定 behavior ID；只有出现新的机器发现 wire 行为时才实现并注册新 behavior。不得在中心代码增加 `provider_profile_id == ...` 或按厂商 ID 的 `match`。
   Adapter 插件在 `src/protocol/plugins/*.rs` 实现 `ProtocolAdapterPlugin`；`build.rs` 按文件名排序自动生成注册表。新增插件文件后不需要再修改 `provider/builtin/registry.rs` 或中心 Adapter 列表。
3. 在 Model Driver catalog 声明 ModelUID、origin model、variants、能力与限制。
4. 在 Provider Rules 中声明 provider model 映射、operation 选择、参数 lowering 和价格解析。
   对拟新增的 dialect 先完成声明化评审；schema 不足时优先扩展统一 schema，禁止在 dialect 代码中直接维护常规模型/参数/operation 表。
5. 让 discovery 只收窄 catalog 声明，不能自行抬高模型能力。
6. 通过 `provider.validate` 校验实例草案，再写入 system-config。
7. 调用 `service.reload_settings`，用 `models.list` 和 `route.resolve` 验证完整身份链。

内置 metadata 文件放入 `driver_metadata/models`、`driver_metadata/providers` 或
`driver_metadata/known-providers` 后由 build script 自动嵌入；不再维护 Rust
`include_bytes!` 清单。聚合 Provider 的 discovery/Rules 必须至少确定
`origin_model_id`；没有直接给出 Model Driver 时，库存构建器在全部 Model Driver metadata
中做唯一匹配并补全 `origin_provider`，多重命中拒绝发布库存。

实例字段 `timeout_ms` 直接控制 HTTP 请求超时；`auto_sync_models=false` 只关闭周期同步，
不跳过启动时的首次发现；`instance_rules` 是强类型对象，目前支持
`exclude_models` 与 `origin_model_overrides`，未知字段会被拒绝。

模型名转换的事实源按以下顺序处理：

- OpenRouter 这类稳定的聚合渠道命名规则写入可更新的 Provider Rules `origin_mappings`；
- 豆包方舟 `ep-*` 是用户实例自己的 endpoint ID，必须在该实例的
  `instance_rules.origin_model_overrides` 中映射到官方模型 ID，不能写成全局映射；
- SN 的 `provider_actual_model_id` 来自网关动态 discovery，继续以动态响应为事实源；
- 恒等命名的 provider 不配置映射。

Provider 不提供价格或价格无法由现有 schema 精确表达时，价格保持未知。禁止为了让
`finance_complete` 变为 true 而填写估算常量。OpenRouter `/models` 与响应 `usage.cost`
当前按其官方约定使用 USD；响应给出的实际金额优先于本地估算。
各内置 Provider 的事实源和静态/动态/unknown 决策见
[`../provider_pricing_sources.md`](../provider_pricing_sources.md)。

原生任务只有在 adapter 明确声明 `cancel_supported` 时才发送远端取消。Gemini/MiniMax
视频当前不支持取消：`task.cancel` 返回 `unsupported_operation`，已有轮询不终止，任务仍会
继续收敛到供应商最终状态。

## 5. 必须验证的行为

- Profile、Adapter、Driver 或 Rules ID 不存在时拒绝加载。
- `route.resolve` 返回 Provider Instance、Profile、Adapter、Driver、ModelUID、origin/provider model 和 operation。
- typed inference 只接受 exact model，且不做隐式 fallback。
- Model Driver variants 在 Provider Rules lowering 后得到正确 operation 和参数。
- 能力结果是 Driver、Adapter 和 discovery 的交集；未知能力不靠模型名猜测。
- 同步、SSE、异步轮询、取消、usage、错误分类和敏感信息脱敏均符合协议。
- OpenRouter 等聚合渠道至少覆盖跨 Model Driver 的映射测试。
- `custom + {}` 必须覆盖标准模型名跨全部 Model Driver 唯一匹配、带厂商前缀不自动改名、未知模型 conservative fallback 和多重命中拒绝。
- 每个实际注册的 API 代际只维护一套共享 Adapter contract test；多个 Provider/派生 Adapter 复用同一历史 Adapter 时不得复制整套 wire protocol 测试。
- 每个派生 Adapter 只测试自身差异和 `base_adapter_id` 委托关系；SN 必须覆盖静态 API Key、动态登录、token 刷新以及删除 SN 不影响 OpenAI 的回归测试。历史 Adapter 删除或失败不能改变官方新接口 Adapter 的行为。自定义 Provider 另需测试新接口优先、历史接口按序匹配、非协议错误停止探测以及 resolved Adapter 持久化。
- GPT-5 `image_generation` 按 metadata 选择 Responses tool；GPT Image/DALL-E 仍走 Image API。

## 6. 文档联动

新增 Profile、Adapter、Driver、Rules、operation 或字段时，同步更新：

- [`../provider_profile_schema.md`](../provider_profile_schema.md)
- [`../driver_metadata_schema.md`](../driver_metadata_schema.md)
- [`../provider_architecture_durable_data_schema.md`](../provider_architecture_durable_data_schema.md)
- [`../aicc_api设计.md`](../aicc_api设计.md)
- [`../aicc-mgr.md`](../aicc-mgr.md)
- acceptance matrix 与对应 Provider 协议用例

# AICC 新增 Provider 开发指南

> 状态：Beta 2.2 目标规范。本文描述重构后的扩展方式，不沿用 `provider_driver` 或每厂商一套 settings section 的实现模式。

## 1. 先判断需要增加什么

新增渠道或模型时，依次判断：

1. 已有 Provider Profile、Protocol Adapter 和 Model Driver 均适用：只新增 Provider Instance。
2. 渠道的认证、默认 endpoint、模型别名、价格或 operation 选择不同：新增 Provider Profile / Provider Rules。
3. 上游 HTTP、SSE、异步任务或错误协议不同：实现并注册 Protocol Adapter。
4. 模型语义、variants、能力或参数约束不同：新增或更新 Model Driver catalog。
5. 新增 AICC 业务能力：先扩展 typed API 和 operation registry，再实现 Driver、Adapter 与验收用例。

渠道身份、传输协议和模型语义必须分别建模，不能重新合并成一个 Provider driver。

## 2. 目标组件

### Provider Profile

定义渠道级事实：显示信息、默认 endpoint、认证方式、默认 Adapter、discovery 与 UI hints。内置 Profile 包括 OpenAI、Claude、Gemini、OpenRouter、SN、MiniMax 和 fal；配置型 Profile 只能引用程序已经注册的 Adapter。

### Provider Rules

负责 `ModelUID + variant + api_type` 到 `provider_model_id + operation + resolved options` 的渠道映射，也承载渠道价格上下文。Rules 生成的参数只进入内部 `ResolvedProviderCall`，不能暴露为公开 `provider_options`。

### Protocol Adapter

只处理协议：请求编码、认证、传输、流式/异步任务、响应解析、错误和取消。Adapter 不负责逻辑模型路由，也不根据模型名猜测能力。

OpenAI、Claude、Gemini 必须分别实现协议族，并按 API 代际注册独立 Adapter。基础协议优先实现 Responses、Messages、Interactions 等官方新接口；Chat Completions、旧 Completions 或 `generateContent` 等历史实现只在接入具体派生 Provider 确有需要时增加，不为完整覆盖历史而预先实现。

新旧接口 Adapter 平级且互不 fallback，只共享协议中立的 transport、SSE、JSON、错误工具和 normalized IR。内置厂商若只扩展某个确定的 Adapter，应注册独立派生 Adapter并声明 `base_adapter_id`；实现可以继承、组合或委托。基础 Adapter 不允许识别派生 Provider。

SN 的标准示例是 `sn-openai -> openai-responses`：SN 层实现 `api_key` 或 `dynamic_login` 认证，Responses 层只执行基础协议。新增类似内置厂商时必须保持相同的单向依赖和可拆除性。

### Model Driver

定义模型的稳定语义：ModelUID、origin model、variants、结构化能力、上下文限制、参数约束和支持的 AICC api types。Driver 不包含渠道凭据、endpoint 或厂商请求模板。

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
      "endpoint": "https://api.openai.com/v1",
      "credentials": {
        "type": "bearer",
        "secret_ref": "system-config://secrets/aicc/openai-work"
      },
      "region": "global",
      "enabled": true
    }
  ]
}
```

不使用 Provider family section、`instances[]` 包装、`provider_driver`、`base_url`、section 级 token、`features` 或字段别名。Profile 默认值只用于创建表单，不能覆盖实例显式配置。

用户添加自定义 Provider 时不填写 `protocol_adapter_id`，只提交协议族、endpoint 和凭据。例如：

```json
{
  "provider_instance_name": "compat-router",
  "provider_profile_id": "custom",
  "protocol_family_id": "openai",
  "endpoint": "https://compat.example/v1"
}
```

`provider.validate` / `provider.add` 先测试该协议族的官方新接口，再按优先级测试运行时已经注册的历史接口。首个成功结果作为内部 `protocol_adapter_id` 保存，例如解析为 `openai-chat-completions`。只有“接口不支持”允许继续下一个候选；认证、网络、限流和服务端错误直接返回。该协商只发生在创建或更新阶段，运行时不得再次试探或 fallback。

## 4. 接入步骤

1. 在 Provider Profile catalog 增加或选择 Profile，并定义认证、endpoint、discovery 和 UI schema。
2. 如具体派生 Provider 需要尚未实现的新协议或历史接口，在 Adapter registry 按需注册固定 `protocol_adapter_id` 和支持的 operations；不要为了覆盖厂商历史而预先实现未被使用的 Adapter。
   若复用基础协议，则同时声明 `base_adapter_id`，并把所有厂商差异留在派生 Adapter。
   若只是兼容旧 API，则新增独立历史接口 Adapter，不修改官方新接口 Adapter，也不增加运行时协议 fallback；同时把它加入该协议族的接入测试候选顺序。
3. 在 Model Driver catalog 声明 ModelUID、origin model、variants、能力与限制。
4. 在 Provider Rules 中声明 provider model 映射、operation 选择、参数 lowering 和价格解析。
5. 让 discovery 只收窄 catalog 声明，不能自行抬高模型能力。
6. 通过 `provider.validate` 校验实例草案，再写入 system-config。
7. 调用 `service.reload_settings`，用 `models.list` 和 `route.resolve` 验证完整身份链。

## 5. 必须验证的行为

- Profile、Adapter、Driver 或 Rules ID 不存在时拒绝加载。
- `route.resolve` 返回 Provider Instance、Profile、Adapter、Driver、ModelUID、origin/provider model 和 operation。
- typed inference 只接受 exact model，且不做隐式 fallback。
- Model Driver variants 在 Provider Rules lowering 后得到正确 operation 和参数。
- 能力结果是 Driver、Adapter 和 discovery 的交集；未知能力不靠模型名猜测。
- 同步、SSE、异步轮询、取消、usage、错误分类和敏感信息脱敏均符合协议。
- OpenRouter 等聚合渠道至少覆盖跨 Model Driver 的映射测试。
- 基础 Adapter 与每个派生 Adapter 分别测试；SN 必须覆盖静态 API Key、动态登录、token 刷新以及删除 SN 不影响 OpenAI 的回归测试。
- 每个实际注册的协议代际分别做 contract test；历史 Adapter 删除或失败不能改变官方新接口 Adapter 的行为。自定义 Provider 另需测试新接口优先、历史接口按序匹配、非协议错误停止探测以及 resolved Adapter 持久化。
- GPT-5 `image_generation` 按 metadata 选择 Responses tool；GPT Image/DALL-E 仍走 Image API。

## 6. 文档联动

新增 Profile、Adapter、Driver、Rules、operation 或字段时，同步更新：

- [`../provider_profile_schema.md`](../provider_profile_schema.md)
- [`../driver_metadata_schema.md`](../driver_metadata_schema.md)
- [`../provider_architecture_durable_data_schema.md`](../provider_architecture_durable_data_schema.md)
- [`../aicc_api设计.md`](../aicc_api设计.md)
- [`../aicc-mgr.md`](../aicc-mgr.md)
- acceptance matrix 与对应 Provider 协议用例

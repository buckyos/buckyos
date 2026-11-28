# AICC Provider UI / Backend Mapping

Provider Wizard 打开时通过 `provider.catalog` 一次性读取已知 Provider profile；保存时通过 `provider.add`、修改时通过 `provider.update` 写入 Provider Instance。UI 不把服务商清单、协议选择或默认 `base_url` 作为真相源。Provider Instance settings、管理 RPC 和 UI DataModel 统一使用 `base_url`，不接收或返回配置字段 `endpoint`。

同一服务商存在多种账号渠道时，各渠道仍使用独立 `provider_profile_id` 和 Provider Rules；Known Provider 的 `ui_hints.setup_group` 只负责把它们合并成一个向导入口，并提供账号类型标签和默认项。选择账号类型后，UI 必须整体切换到该 profile 的 `base_url`、operation base URLs、连接字段和凭据表单，不得混合不同 profile 的默认值。模型库存由所选 profile 的 discovery 与 Provider Rules 决定，UI 不维护账号类型模型白名单。

SN 的默认 `base_url` 为 `https://sn.buckyos.ai/api/v1/ai`；各媒体操作与 Responses 共用该入口，Known Provider 不再提供 operation URL 覆盖。向导直接消费 catalog，因此 SN 不显示 operation URL 输入项。豆包语音等 profile 仍使用该公共字段，前后端保留其通用处理。

`provider_profile_id` 是 catalog 提供的开放字符串，不是前端枚举。前端必须原样保留和展示后端返回的未知新 ID；不得把它归一化为 `custom`、过滤或要求随客户端版本更新白名单。`custom` 只表示用户主动选择的自定义 Provider。

| UI DataModel | Backend field | Durable owner | Notes |
| --- | --- | --- | --- |
| `KnownProviderProfile.provider_profile_id` | `provider_profile_id` | Provider catalog | 渠道规则与展示身份 |
| `KnownProviderProfile.protocol_adapter_id` | `protocol_adapter_id` | catalog + runtime registry | 默认客户端；backend 校验已注册 |
| `KnownProviderProfile.extra_protocol_adapters` | `extra_protocol_adapters` | catalog + runtime registry | 有序的其余客户端 ID；与默认客户端一起构成该 Provider 的客户端集合，规则编辑器的客户端下拉只列这些 |
| `ProtocolAdapter.protocol_family_id` | `protocol_family_id` | runtime registry | OpenAI、Claude、Gemini 协议族；不是可执行 Adapter |
| `ProtocolAdapter.custom_provider_selectable` | custom client visibility | Provider Rules catalog + runtime registry | 由 `*.provider.json` 的 `custom_provider_adapters` 推导；只有值为 `true` 的客户端才出现在自定义 Provider 的客户端列表里，也只有它们会进入自定义实例的客户端集合 |
| custom Provider draft family | `protocol_family_id` | `provider.validate/add/update` request | 所选客户端所属的协议族；与 `protocol_adapter_id` 不一致时拒绝 |
| `ProtocolAdapter.base_adapter_id` | `base_adapter_id` | runtime registry | 只读展示语义子类关系，例如 `legacy-openai-chat-completions -> openai-chat-completions` |
| `KnownProviderProfile.base_url` | `base_url` | Provider catalog default | 仅作表单初值，用户可修正 |
| `ProviderConfig.id` | `provider_instance_name` | system-config | Zone 内唯一实例 ID |
| `ProviderConfig.base_url` | `base_url` | system-config | UI、管理 RPC 和 settings 使用同一字段名 |
| `ProviderConfig.provider_profile_id` | `provider_profile_id` | system-config | 不读取旧 `provider_driver` |
| `ProviderConfig.protocol_adapter_id` | `protocol_adapter_id` | system-config | 自定义 Provider 的默认客户端，用户必须显式选择，后端不探测；Known Provider 可省略，提交了也不读取，客户端集合只来自 catalog |
| `ProviderConfig.auth` | `auth` | system-config typed credential source | SN 显式选择 `api_key` 或 `dynamic_login`；凭据来源使用 `inline_secret|secret_ref|runtime_ref` |
| `ProviderInventory.provider_profile_id` | `provider_profile_id` | inventory / LKGS | discovery 使用的 profile |
| `ProviderInventory.protocol_adapter_id` | `protocol_adapter_id` | inventory / LKGS | 实例默认客户端；每个模型另有自己的默认客户端，`operations` 存 `<客户端>/<operation>` |
| `ModelItem.model_driver` | `model_driver` | Model Driver catalog | 未知值显示 `unknown`，不回退为 profile |
| origin trace | `selected_origin_model_id`, `selected_model_driver` | route trace RDB | 原厂身份 |
| channel trace | `selected_provider_profile_id`, `selected_protocol_adapter_id`, `selected_operation` | route trace RDB | 实际渠道、本次选中的客户端与 operation |

## Loading and errors

- catalog 加载中显示 loading，不渲染本地硬编码列表。
- catalog 中新增且引用现有 Adapter 的 Provider 必须立即作为普通选项展示；图标和本地化缺失时使用通用回退展示。
- 请求失败显示可重试错误；空 catalog 与请求失败分开呈现。
- 保存、连接测试和模型 refresh 失败后保留用户输入。
- SN 表单根据 `auth.mode` 显示 API Key 或动态登录字段，不同时提交两套凭据；动态 token 永不返回 UI。
- 官方 Profile 默认新接口。添加自定义 Provider 时，UI 列出 `custom_provider_selectable=true` 的客户端（当前属于 OpenAI、Claude、Gemini 三个协议族），用户必须显式选一个作为默认客户端，连同所属 `protocol_family_id` 提交；未选时前后端都拒绝。后端只验证所选客户端，不发探测请求、不自动识别协议，也拒绝绕过 UI 提交未开放的内部客户端。实例随后带上同一协议族里其余开放的客户端。推理运行时不重新探测或降级。

## Performance boundary

- 一个 Wizard 打开周期只发起一次 `provider.catalog`，响应携带完整 `providers[]`，没有按 provider 的 N+1 请求。
- catalog 边界转换为 O(n)，Provider inventory 独立加载；catalog 不携带模型列表或凭据。

## Breaking contract

beta 2.2 的 settings 将 `provider_driver` 拆为 `provider_profile_id` 和 `protocol_adapter_id`；模型级 `model_driver_id` 由 catalog/inventory 产生，不写入 Provider Instance settings。`buckyos-api` 和验收报告中已经导出的 `provider_driver` 兼容字段不因 settings 重构而删除。Web UI 无升级兼容压力，`src/frame/desktop/src/api/aicc_mgr.ts` 的 UI DataModel 和管理 RPC 原始 JSON 都直接使用 `base_url`，不保留 `endpoint` 转换层。

## 自定义 Provider 高级接入

向导及实例详情支持 `provider_rules_id`、`instance_rules.model_driver_overrides`、`instance_rules.server_default_routes`；读取 `provider.list` 后原样回填。验证请求也传递实例规则，显示发现、身份识别、不可用预设及聊天候选，配置验证不发推理请求。空渠道规则表示不继承任意厂商规则；更新时空字符串清除，省略保持。设计和验收见 [服务端默认模式](provider_server_defaults.md)。

## 自定义 Provider 的默认操作

所选客户端只决定协议，不证明端点支持它声明的全部能力。库存构建在实例的客户端集合里先按发现结果限制操作候选，再按 binding 的 `default_priority` 选择：跨客户端比较，取最高的正分，同分时优先默认客户端；0 分绑定只有在所有候选都是 0 分时才会落到默认客户端上唯一的那个。显式渠道规则的操作映射优先。调用 lowering 使用同一个解析函数，库存中已选定的客户端和操作仍由路由携带，不在推理时重新探测端点。规则细节见 [Provider schema §5.2](provider_profile_schema.md#52-operation)。

自定义 OpenAI 实例的图片默认操作为 `openai-images` 的文生图 `images.generate`、图生图及局部编辑 `images.edit`；`openai-responses` 的 `responses.create` 保留图片能力，供发现结果或显式规则选择。最高优先级在默认客户端之外仍有多个候选时拒绝歧义，不按名称或注册顺序猜测。每个对 custom 开放的客户端单独使用时都必须对每项能力提供唯一默认操作，单测覆盖该约束。

当前对 custom 开放的客户端中，同一 API 类型在自定义实例里有多个候选的情况如下：

| 协议族 | API 类型 | 候选（客户端/操作 优先级） | 默认操作 |
| --- | --- | --- | --- |
| OpenAI | `llm`、`vision.ocr`、`vision.caption` | `openai-responses/responses.create`、`openai-chat-completions/chat.completions.create`、`legacy-openai-chat-completions/chat.completions.create` 均为 1；`vision.ocr` 另有 `openai-responses/responses.background` 0 | 默认客户端提供的那个；默认客户端不提供时三者并列，报歧义 |
| OpenAI | `image.txt2img` | `openai-images/images.generate` 100，`openai-responses/responses.create` 1，`openai-responses/responses.background` 0 | `images.generate` |
| OpenAI | `image.img2img` | `openai-images/images.edit` 100，`openai-responses/responses.create` 1，`openai-responses/responses.background` 0 | `images.edit` |
| OpenAI | `audio.asr` | `openai-audio/audio.transcriptions` 1，`openai-chat-completions/chat.completions.create` 0 | `audio.transcriptions` |
| Claude | `llm`、`vision.ocr`、`vision.caption` | `claude-messages/messages.create`、`legacy-claude-messages/messages.create` 均为 1 | 默认客户端提供的那个 |

Gemini 族只开放 `gemini-generate-content`，每项 API 类型只有一个 operation。`openai-videos` 的 `video.extend` 是 0 分，只有默认客户端就是 `openai-videos` 或规则显式映射时才可用。`gemini-interactions`、`typesafe-systemone` 不再对 custom 开放。单测覆盖单个客户端内的全部发现候选子集和显式映射、自定义 OpenAI 实例的图片默认，以及跨客户端的选择规则（`protocol/binding.rs`），避免仅检查默认操作是否唯一而漏掉选择错误。

这些默认值不等于渠道能力认证。OpenRouter 等已有专用 profile 的渠道应使用其内置入口，以获得专用协议、发现和操作映射；不能仅凭 OpenAI 兼容端点能连通推断其全部媒体接口可用。接入校验中的 `Inventory` 错误表示模型清单构建失败，应核对模型、适配器及操作映射，不能据此判断网络或 API Key 失败。


## 不支持的能力项

库存构建找不到 `adapter + operation + API 类型` 的 codec 绑定时，只滤除该模型对应的能力项，保留其他能力；适配器未声明的 API 类型也按不支持处理。滤除后不会把该项写入 `api_types`、`operations` 或对应逻辑挂载，不影响同一 Provider 的其他模型。显式操作映射缺失绑定同样过滤；操作选择歧义及其他配置错误仍报错。

`provider.validate.filtered_capabilities`、`provider.list` 的 `inventory.filtered_capabilities` 和库存快照携带过滤明细：`provider_model_id`、`protocol_adapter_id`、`api_type`、可空的 `operation_id`、`reason`（`unregistered_binding` 或 `unsupported_api_type`）。Validation 页面及 Provider 模型清单页提供数量与可展开的完整列表，过滤记录不计入验证错误。发现接口报告的能力限制仍参与原有的能力求交。

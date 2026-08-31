# AICC Provider UI / Backend Mapping

Provider Wizard 打开时通过 `provider.catalog` 一次性读取已知 Provider profile；保存时通过 `provider.add` 写入 Provider Instance。UI 不把服务商清单、协议选择或默认 endpoint 作为真相源。

| UI DataModel | Backend field | Durable owner | Notes |
| --- | --- | --- | --- |
| `KnownProviderProfile.provider_profile_id` | `provider_profile_id` | Provider catalog | 渠道规则与展示身份 |
| `KnownProviderProfile.protocol_adapter_id` | `protocol_adapter_id` | catalog + runtime registry | backend 校验 adapter 已注册 |
| `ProtocolAdapter.base_adapter_id` | `base_adapter_id` | runtime registry | 只读展示语义子类关系；SN 为 `sn-openai -> openai` |
| `KnownProviderProfile.default_endpoint` | `default_endpoint` | Provider catalog default | 仅作表单初值，用户可修正 |
| `ProviderConfig.id` | `provider_instance_name` | system-config | Zone 内唯一实例 ID |
| `ProviderConfig.provider_profile_id` | `provider_profile_id` | system-config | 不读取旧 `provider_driver` |
| `ProviderConfig.protocol_adapter_id` | `protocol_adapter_id` | system-config | 与 profile 分开保存 |
| `ProviderConfig.auth` | `auth` | system-config locked value / credential reference | SN 显式选择 `api_key` 或 `dynamic_login` |
| `ProviderInventory.provider_profile_id` | `provider_profile_id` | inventory / LKGS | discovery 使用的 profile |
| `ProviderInventory.protocol_adapter_id` | `protocol_adapter_id` | inventory / LKGS | 实际 wire adapter |
| `ModelItem.model_driver` | `model_driver` | Model Driver catalog | 未知值显示 `unknown`，不回退为 profile |
| origin trace | `selected_origin_model_id`, `selected_model_driver` | route trace RDB | 原厂身份 |
| channel trace | `selected_provider_profile_id`, `selected_protocol_adapter_id`, `selected_operation` | route trace RDB | 实际渠道与 operation |

## Loading and errors

- catalog 加载中显示 loading，不渲染本地硬编码列表。
- 请求失败显示可重试错误；空 catalog 与请求失败分开呈现。
- 保存、连接测试和模型 refresh 失败后保留用户输入。
- SN 表单根据 `auth.mode` 显示 API Key 或动态登录字段，不同时提交两套凭据；动态 token 永不返回 UI。

## Performance boundary

- 一个 Wizard 打开周期只发起一次 `provider.catalog`，响应携带完整 `providers[]`，没有按 provider 的 N+1 请求。
- catalog 边界转换为 O(n)，Provider inventory 独立加载；catalog 不携带模型列表或凭据。

## Breaking contract

beta 2.2 将 `provider_driver` 拆为 `provider_profile_id` 和 `protocol_adapter_id`。backend 原始 JSON 的校验与 UI DataModel 转换集中在 `src/frame/desktop/src/api/aicc_mgr.ts`。

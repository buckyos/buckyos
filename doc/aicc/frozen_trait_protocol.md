# AICC Trait 与 Protocol 冻结设计

状态：**Frozen / Beta 2.2**

冻结日期：2026-09-14

适用范围：`buckyos-api` 公共 kRPC 契约与 `frame/aicc` 内部端口、Protocol Adapter 契约。

本文冻结当前实现的边界，不复制所有 Rust 字段。公共 DTO、method 常量和 dispatcher 的唯一代码真相源是
`src/kernel/buckyos-api/src/aicc_client.rs`；内部 trait 的真相源是各自所属模块。本文与代码不一致时，必须把差异视为协议变更并同时修正文档、代码和合同测试，不能仅解释为实现细节。

## 1. 冻结结论

1. AICC 控制与推理接口使用 BuckyOS kRPC；Provider URL artifact 另有受鉴权的流式 data endpoint，但不暴露 Provider 厂商 HTTP 协议。
2. `AiccClient`、`AiccHandler`、`AiccServerHandler` 和公共 DTO 属于 SDK 契约；`frame/aicc` 内所有 `pub(crate) trait` 只属于内部替换点。
3. 公共 method 决定请求 schema；`ApiType` 决定模型能力与 Provider operation 绑定；`Capability` 仅为粗粒度分组。三者不能互换。
4. Protocol Adapter 的复用单位是 `(protocol_adapter_id, operation_id, api_type)`，不是 Provider 品牌。
5. Provider Rules 选择 operation 并做声明式 lowering；codec 只负责无法声明化的 wire、认证、流、任务生命周期和错误差异。
6. Beta 2.2 不保留旧 method、旧字段别名或兼容 dispatcher。

## 2. 公共 kRPC 契约

### 2.1 服务与信封

| 项 | 冻结值 |
| --- | --- |
| service unique id | `aicc` |
| service name | `aicc` |
| service port | `4040` |
| NodeGateway 入口 | `POST /kapi/aicc` |
| artifact data endpoint | `POST /kapi/aicc/artifact/open` |
| 请求 | `RPCRequest { method, params, sys }` |
| 响应 | 与请求相同 `seq`、`trace_id` 的 `RPCResponse` |

`params` 必须反序列化为 method 对应的强类型 request。未知字段由带
`deny_unknown_fields` 的 DTO 拒绝；未知 method 返回 `RPCErrors::UnknownMethod`。

### 2.2 双模式客户端与服务端

```text
AiccClient::InProcess(Box<dyn AiccHandler>)
  └─ 直接调用 handler，使用默认 RPCContext

AiccClient::KRPC(Box<kRPC>)
  └─ request -> JSON -> client.call(method, params) -> typed response

AiccServerHandler<T: AiccHandler>
  └─ RPCRequest -> from_json -> handle_* -> RPCResult::Success
```

网络客户端的上下文由 `AiccClient::set_context` 设置。服务端必须用
`RPCContext::from_request` 构造鉴权上下文，不得相信业务 `params` 中自报的用户或租户身份。

### 2.3 Method 分组

| 分组 | method |
| --- | --- |
| 路由/Helper | `route.resolve`, `helper.llm_chat`, `helper.text_to_image` |
| typed inference | `chat.completions.create`, `images.generate`, `embedding.text`, `embedding.multimodal`, `rerank`, `image.img2img`, `image.inpaint`, `image.upscale`, `image.bg_remove`, `vision.ocr`, `vision.caption`, `vision.detect`, `vision.segment`, `audio.tts`, `audio.asr`, `audio.music`, `audio.enhance`, `video.txt2video`, `video.img2video`, `video.video2video`, `video.extend`, `video.upscale`, `agent.computer_use` |
| 任务 | `cancel` |
| 路由/运行管理 | `service.reload_settings`, `routing.get`, `routing.update`, `models.list` |
| Provider 管理 | `provider.catalog`, `protocol_adapter.list`, `provider.validate`, `provider.add`, `provider.list`, `provider.health`, `provider.update`, `provider.delete`, `provider.refresh_models` |
| 用量/诊断 | `quota.query`, `usage.query`, `trace.query` |
| Metadata 更新 | `driver_metadata_update.get`, `driver_metadata_update.set` |

完整 request/response 类型映射由 `AiccHandler` 签名冻结。新增 method 必须同时增加常量、DTO、client、handler、dispatcher 和协议测试。

## 3. 内部 Trait 边界

内部 trait 默认 `Send + Sync`，异步边界使用 `async_trait`。它们不是跨 crate 稳定 ABI。

| 层 | 冻结端口 | 职责 |
| --- | --- | --- |
| service | `ServiceAuthorizer` | 从 `RPCContext` 鉴权并生成可信 caller |
| service | `SettingsStore` | 从 system-config 读取及 CAS 写入 AICC settings |
| service | `PreparedSettingsRuntime`, `ServiceRuntime` | prepare/publish/discard runtime，保证配置发布原子性 |
| service | `InferencePort` | 路由和 canonical call 的服务入口 |
| service | `ProviderValidator`, `QuotaQueryPort`, `UsageQueryPort`, `DriverMetadataPort` | 管理面端口 |
| runtime | `RuntimeBackend`, `RuntimeFactory`, `ModelRegistryAssembler` | 组装 Provider、catalog、模型注册表并发布不可变 snapshot |
| provider | `ProviderDiscovery` | 获取一个 Provider Instance 的动态库存 |
| provider | `CredentialResolver`, `DynamicLoginCredentialResolver` | 将受保护凭据解析到最小使用范围 |
| provider | `ProviderQuotaObserver` | 读取 Provider 配额事实 |
| provider | `ProviderInventoryStore` | 原子保存/读取实例 inventory LKGS |
| protocol | `OperationCodec` | immediate/stream 请求编码和响应解码 |
| protocol | `NativeTaskCodec` | submit/status/result/cancel 原生任务协议 |
| protocol | `ProtocolAdapterPlugin` | 向 `CodecRegistry` 原子注册 descriptor 与 codec |
| protocol | `ArtifactDownloadProtocol` | Adapter 级 URL artifact 下载请求/响应协议；默认实现可复用，特殊协议按 adapter 覆写 |
| execution | `ExecutionStore`, `TaskManagerPort`, `ProviderExecutionPort`, `UsageCompletionPort` | 幂等执行、TaskMgr 桥接、Provider 执行和一次性用量入账 |
| resource | `ResourceAuthorizer`, `ResourceStore`, `UrlResourceFetcher` | `ResourceRef` 鉴权、物化和 artifact 写入 |

禁止创建同时承担路由、发现、wire 编解码和生命周期的“大 Provider trait”。跨层调用必须经过上表端口或不可变数据对象。

## 4. Protocol Adapter 契约

### 4.1 Descriptor

`AdapterDescriptor` 冻结字段为：

- `protocol_family_id`：协议族，例如 OpenAI/Claude/Gemini 语义族；
- `protocol_adapter_id`：具体代际或 dialect 的稳定 ID；
- `interface_generation`：接口代际；
- `base_adapter_id`：可选基础 adapter，只有 descriptor 完全兼容的 operation 才能继承 codec；
- `status`、`probe_priority`、`probe_path`；
- `credential`；
- `operations: BTreeMap<String, OperationDescriptor>`。

`OperationDescriptor` 必须声明稳定的 `operation_id`、一个或多个 `OperationBinding`、请求/响应大小上限，以及 cancel/webhook 能力。每个 binding 固定一个 `ApiType`、由它推导的 `Capability`、支持 feature 与执行模式集合。

注册时必须满足：

- descriptor 的 operation key 与 `operation_id` 相同；
- 同一 operation 内 `ApiType` 不重复；
- codec 声明的执行模式与 descriptor 完全一致；
- `NativeTaskCodec` 完整实现 submit/status/result，声明可取消时还必须实现 cancel；
- 派生 adapter 只能继承 descriptor 与 binding 完全相等的 codec；
- `(adapter, operation, api_type)` 不得重复注册。

### 4.2 调用链

```text
public DTO / AiccCall
  -> route.resolve 选择 exact model 与 operation
  -> ResolvedProviderCall（固定 runtime generation）
  -> Provider Rules + canonical field lowering
  -> CodecRegistry 查找 (adapter, operation, api_type)
  -> OperationCodec 或 NativeTaskCodec
  -> HttpTransport
  -> ProtocolExecution / ProtocolOutput / ProtocolStream
  -> typed public response
```

编码器接收 canonical request、已解析参数、已物化资源、endpoint、credential 和 limits。凭据不得进入 Debug、route trace、task data 或用户响应。

### 4.3 URL artifact 下载

`open_artifact_url_reader(url, artifact_id?)` 的职责链冻结为：service 按精确 URL 查询持久来源并校验 tenant/可选 artifact id，找到原 ProviderInstance，ProviderInstance 解析自己的 credential，再按 `protocol_adapter_id` 调用 `CodecRegistry` 的 artifact 下载协议。默认 `ArtifactDownloadProtocol` 只允许与 Provider base URL 同 origin 的 HTTP(S) URL，使用 Provider credential 发起 GET，并返回有大小上限的异步 byte stream。

Adapter 默认复用该实现。只有认证 header、URL 变换、请求 method 或响应 envelope 确实不同的协议，才注册自定义 `ArtifactDownloadProtocol`；不得为每个 Provider 品牌复制下载函数。未登记 URL 不进入 Adapter，AICC 不按 host 猜测 Provider。

## 5. 执行与任务语义

- `Immediate`、`Stream` 是用户可请求的执行模式；所选 binding 不支持时返回 `unsupported_execution_mode`。
- Provider 原生异步协议是内部 `NativeTask` 模式。AICC 负责将它桥接为统一 task 状态，用户不能通过公共 DTO 直接选择 `NativeTask`。
- 幂等 key 的作用域为租户、method 与 key；同 key 不同请求体返回 `idempotency_conflict`。
- 长任务必须固定 Provider Instance、adapter、operation、模型身份和恢复描述，不能在恢复时按当前 catalog 重新路由。
- 只有 Provider 确认取消才返回 `accepted=true`；终态不可被迟到结果覆盖。

## 6. 错误协议

业务稳定错误使用 `AiccError { code, message, provider_code?, retriable, details? }`。冻结错误码为：

```text
invalid_request, invalid_method, schema_validation_failed, invalid_model_name,
resource_invalid, no_provider_available, no_candidate_model,
fallback_not_allowed, provider_start_failed, provider_error,
unsupported_operation, unsupported_execution_mode, timeout, budget_exceeded,
policy_denied, idempotency_conflict, settings_revision_conflict, cancelled,
internal_error
```

请求/schema/模型名错误映射为 `ParseRequestError`，权限错误映射为 `NoPermission`，其它业务错误映射为 `ReasonError`，其 body 为序列化后的 `AiccError`。调用方应优先用 `AiccError::from_krpc_error` 解码，不能按 message 文本分支。

Protocol 内部错误还保留 request id、retry-after 与是否允许 failover 等信息；对外必须先归一化，不泄露响应正文、header 或凭据。

## 7. 冻结后的变更规则

以下改动均属于合同变更：公共 method/DTO/枚举/error code，adapter/operation ID，trait 职责迁移，执行模式语义，exact model 格式。变更必须：

1. 先修改本文及用户接口文档；
2. 同步 `buckyos-api`、AICC dispatcher 和 TypeScript 产物；
3. 同步 Provider operation bindings、前后端映射和验收矩阵；
4. 增加 request/response round-trip、未知字段、dispatcher、错误映射与 protocol golden tests；
5. Beta 2.2 直接切换，不加兼容 alias。

## 8. 关联文档

- [AICC 用户接口冻结文档](frozen_user_api.md)
- [Model Driver 与逻辑模型 FS 冻结设计](frozen_model_driver_and_logical_model_fs.md)
- [Provider 实现冻结设计](frozen_provider_implementation.md)
- [内部模块架构](internal_module_architecture.md)
- [Provider operation 绑定](provider_operation_bindings.md)

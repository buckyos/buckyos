# AICC Beta 2.2 重新实现路线图

状态：Draft / Gate 0 Frozen

目标版本：Beta 2.2

实现原则：完全抛弃当前 AICC 内部实现，不兼容旧模块、旧 settings 结构和旧 metadata；`buckyos-api::aicc_client` 已导出的 RPC method 和请求/响应类型原则上作为兼容边界优先保持不变，但 reload 接口同步更新为唯一 canonical method `service.reload_settings`，不保留旧 `reload_settings`；复用 BuckyOS 已有的 kRPC、system-config、TaskMgr、Named Object、RDB、HTTP 等平台能力。

## 1. 文档目的

本文把 AICC 目标设计转换为可以由多人并行认领、分阶段集成和验收的工程路线图，回答：

1. 重建哪些模块；
2. 每个模块的输入、输出、依赖和完成标准；
3. 哪些任务可以并行；
4. 哪些契约必须先冻结；
5. 如何从新实现切换并删除旧实现；
6. 每个阶段使用什么测试门禁。

本文是实施 tracker，不替代需求和设计规范。发生冲突时，按下列优先级处理：

1. `doc/aicc/` 根目录中标记为 Beta 2.2 目标规范的文档；
2. 本文 Gate 0 已签字冻结的决策；
3. 当前分支代码，仅用于识别调用面和删除范围，不作为新实现语义依据；
4. `doc/aicc/maintenance/`，仅作维护和验收参考；
5. `doc/aicc/archive/`，不作为实现依据。

主要规范入口：

- [`aicc_requirements.md`](aicc_requirements.md)
- [`aicc_api设计.md`](aicc_api设计.md)
- [`internal_module_architecture.md`](internal_module_architecture.md)
- [`provider_profile_schema.md`](provider_profile_schema.md)
- [`provider_architecture_durable_data_schema.md`](provider_architecture_durable_data_schema.md)
- [`match_rule.md`](match_rule.md)
- [`aicc_router.md`](aicc_router.md)
- [`driver_metadata_update_protocol.md`](driver_metadata_update_protocol.md)
- [`aicc_e2e_test_requirements.md`](aicc_e2e_test_requirements.md)

## 2. 重建范围

### 2.1 必须交付

- `/kapi/aicc` 的控制面、typed inference、Helper 和管理 API；
- 逻辑模型目录、精确模型、variant、路由、调度、fallback 和 trace；
- Provider Profile、Protocol Adapter、Provider Rules、Model Driver 和 Provider Instance 的新身份体系；
- 不可变 RuntimeSnapshot、Provider inventory LKGS 和 metadata seq 全局收敛；
- immediate、Provider streaming 归并、异步任务、取消和幂等；
- ResourceRef 鉴权、限制、最后一跳物化和 artifact 输出；
- usage、cost snapshot、route trace、审计、健康和脱敏诊断；
- Provider 管理、settings CAS、AI Center、Workflow、OpenDAN/Jarvis 和 CLI 联动；
- 11 个首版内置 Provider：OpenAI、Claude、Gemini、fal、OpenRouter、MiniMax、Kimi、GLM、DeepSeek、豆包、Qwen；
- `sn-openai` 扩展 Provider，作为 11 个首版 Provider 之外的既有扩展保留；
- 编码期间的模块单元测试，以及编码完成后的 T1/T1.5/T2/T3 验收闭环。

### 2.2 非目标

- 不读取或迁移旧 Provider family section、settings 中的 `provider_driver`、section token 或旧字段别名；新 `providers[]` 继续使用 `base_url`；
- 不新增旧 method alias、错误拼写或重复入口；管理面只保留 `service.reload_settings`，同步更新 `buckyos-api::aicc_client` 和全部调用方，删除旧 `reload_settings`；其它已导出入口仍作为兼容边界；
- 不为未知 Provider 自动兼容任意 OpenAI-compatible endpoint；
- 不提前实现没有真实 Provider 需求的历史 API 代际；
- 不把任意 Provider JSON 暴露为公共 `extra_body`；
- 不实现本地模型；
- 不把 `agent.computer_use` 作为首版普通模型调用开放；
- 不新增 workspace crate 或第三方依赖，除非单独评审并获得确认；
- 不把 TaskMgr completed task 当作 usage 持久事实源。

## 3. 设计一致性 Review 与 Gate 0

以下问题在复核当前文档、代码和验收清单时发现。Gate 0 已完成并冻结，后续实现统一遵守表中的确定结论；本次冻结同时同步到相关设计文档。

| ID | 问题 | 当前不一致 | 冻结结论 | 影响范围 |
|---|---|---|---|---|
| D-001 | 图像 typed method 名 | 历史材料曾把 typed method 与 api_type 都写成 `image.txt2img` | typed method 保持 `images.generate`，对应 `api_type=image.txt2img`；二者不要求同名 | API、codec、Workflow、CLI、E2E |
| D-002 | LLM canonical method | 历史验收材料曾使用 `llm.chat` | typed method 保持 `chat.completions.create`，对应 `api_type=llm` | API、E2E、Workflow |
| D-003 | method 与 api_type 混用 | 部分历史验收表把 api_type 当 method | method 与 api_type 是不同值域，不要求同名或 1:1；分别冻结值域并维护显式合法关联，preflight 不做名称相等或双射检查 | 全部数据面模块 |
| D-004 | Provider 身份字段与 settings | 新身份设计、旧 family settings、验收报告和公共 RPC 中都出现过 `provider_driver`，新草案又把 `base_url` 改成 `endpoint` | 新 settings 使用统一 `providers[]`，保留 `base_url`，实例保存 `provider_profile_id` 和 `protocol_adapter_id`；模型级 `model_driver_id` 由 catalog/inventory 产生；不兼容读取旧 settings 的 `provider_driver`，但保留 `buckyos-api` 已导出的同名 RPC/报告字段 | Provider、UI、E2E |
| D-005 | Provider 范围 | 内部架构要求 11 家，当前验收基线主要覆盖 7 家和 SN | 首版实现 11 家内置 Provider，SN 单独列为扩展；当前 baseline 缺口不阻塞 Gate 0，在集成测试阶段先补齐 T1/T1.5，再补齐 T2，最后进入 T3 | Provider、集成测试 |
| D-006 | `provider.update` | 管理方法总览包含，后文又写第一版暂不做 | 纳入首版；enable/disable 以及管理 RPC 的 `base_url`、credential、Profile、Adapter、discovery 和实例规则修改统一通过 `provider.update`，并执行 CAS 与实例停止/替换生命周期 | 管理 API、RuntimeSnapshot |
| D-007 | metadata 管理方法名 | 文档同时出现 `driver_metadata_update.*` 和 `provider_catalog_update.*` | 保留已导出的 `driver_metadata_update.get/set`；接口覆盖 Model Driver、Provider Rules 和 Known Provider 三类 metadata catalog 的云更新配置与状态 | API、UI、NDN 集成 |
| D-008 | quota/budget 边界 | 产品 P0 和验收包含 quota/budget，但目标模块树没有独立 owner | 不新增独立顶层 `admission` 模块；模型 admission 归 Model Registry，quota/budget/privacy/trust 在 routing 内部策略层判定，Router 消费判定结果 | 路由、安全、usage |
| D-009 | 测试层级术语 | 历史维护文档使用 L1-L4，当前验收规范使用 T1/T1.5/T2/T3 | 单元测试不再包装成验收层级；集成验收统一使用 T1/T1.5/T2/T3 | CI、报告 |
| D-010 | settings 所有权表述 | 持久化文档称 control-panel 写、AICC 只读；管理 API 又要求 AICC 代理写 system-config | 明确 AICC 是使用调用者 token 的受控写入 facade，system-config 仍是唯一真相源，前端不得直写 | 管理 API、RBAC |

### 3.1 Gate 0 已冻结结论

- [x] D-001 至 D-010 已给出确定结论；
- [x] canonical method 表按 D-001、D-002 冻结；
- [x] api_type、Capability 和 method 映射边界已冻结；
- [x] request/response、ResourceRef、AiMessage、usage、artifact、trace 和错误 schema 以目标规范为准；
- [x] exact model 语法确定为 `<provider_model_id>[:<variant>]@<provider_instance_name>`，`provider_model_id` 和 `provider_instance_name` 均不得包含 `@`；
- [x] settings 统一使用 `providers[]` schema；
- [x] Provider Instance settings、管理 RPC 和 Web UI DataModel 统一使用 `base_url`，不接收或返回配置字段 `endpoint`；Web UI 无升级兼容约束，不保留转换层；
- [x] Provider Instance、Profile、Adapter、Model Driver、ModelUID 身份链已冻结；
- [x] `buckyos-api::aicc_client` 已导出的 RPC 接口原则上作为兼容边界；reload 接口是明确例外，客户端更新为 `service.reload_settings` 并删除旧 `reload_settings`；
- [x] 首版包含 11 家内置 Provider、SN 扩展和目标 operation 覆盖矩阵；
- [x] E2E canonical 表、Provider baseline schema 和 case manifest 必须按冻结契约对齐；
- [x] T1、T1.5、T2、T3 的边界、矩阵维度和完成顺序已冻结；
- [x] API owner、Metadata owner、E2E owner 为对应工作包的固定责任角色；
- [x] Gate 0 结论由本文独立记录，后续变更必须单独评审。

以上勾选表示设计决策已经确定，不表示对应代码、其它设计文档或验收资产已经完成修改；实际对齐工作仍由后续工作包跟踪。

Gate 0 完成标准：任意一个 method、api_type、Provider 身份或 settings 字段都能在规范、Rust 公共类型、TS canonical 表和 case manifest 中得到唯一答案。

## 4. 目标模块与依赖方向

```text
aicc
├── service          进程启动、依赖装配、kRPC、优雅退出
├── api              schema、鉴权、校验和 use case
├── error            稳定错误分类及边界转换
├── runtime          RuntimeSnapshot 与原子发布
├── settings         system-config、CAS、候选配置
├── catalog          三类 catalog DTO、加载和索引
├── matching         MatchRule 编译和唯一执行器
├── model            ModelUID、variant、目录和 registry
├── provider         Profile、Instance、discovery、inventory、生命周期
├── protocol         transport、IR、codec、dialect、native operation
├── routing          内部策略判定、候选、过滤、调度、fallback 和 trace
├── call             RouteDecision 到 ResolvedProviderCall 的唯一 lowering
├── execution        immediate、stream、task、cancel、idempotency
├── resource         ResourceRef 鉴权、限制和最后一跳物化
├── storage          inventory、usage、trace 和 task 关联存储
└── observability    metrics、审计、诊断和脱敏
```

固定依赖方向：

```text
service/api -> use cases/runtime
routing     -> model + provider read-only views + storage policy views
call        -> model + provider rules + protocol descriptors/IR
execution   -> protocol + resource + storage
provider    -> catalog + model + matching + storage
dialect     -> declared base codec + protocol primitives
base codec  -> transport + resolved credential + IR
```

禁止：

- `protocol -> routing`；
- `model -> provider`；
- 基础 codec 引用派生 dialect；
- 按 Provider ID、URL、模型名或模型前缀选择协议分支；
- handler 直接调用厂商模块；
- 通过全局 `AIComputeCenter` 绕过模块边界；
- 在 Router 中读取资源 bytes；
- Provider Rules 访问网络。

## 5. 工作包

### WP-00：工程骨架与所有权

Owner：集成人

依赖：Gate 0

- [x] 建立目标目录和 `pub(crate)` 边界；
- [x] 明确公共 trait、DTO、fixture 和 fake 只在对应工作包确认必要性后引入，WP-00 不预定义推测性接口；
- [x] 为 `lib.rs`、`main.rs`、`Cargo.toml` 指定唯一 owner；
- [x] 建立模块 CODEOWNERS/评审人清单；
- [x] 建立编译顺序和最小 smoke test；
- [x] 新模块不得引用旧 AICC 实现；
- [x] 旧实现已在骨架建立前整体删除，不保留旧模块作为新实现依赖。

完成标准：各小组可以只依赖公共 contract 独立开发，不需要修改同一个巨型文件。

实现记录：Owner 为集成人（`@streetycat`）；关联变更为 WP-00 实现提交/PR；目标验收为 `cargo test -p aicc`、`cargo check -p aicc --all-targets`、AICC clippy 和 workspace test compile；剩余风险是公共 contract 尚未进入代码，必须由各工作包在需求确定后按 owner 边界引入并测试。

所有 WP-01 至 WP-17 共同遵守：编码过程中同步完成本模块单元测试；没有覆盖正常、边界、错误和关键并发语义的工作包不得标为 Done。模块单测是编码完成条件，不用 T1/T1.5/T2/T3 代替模块单测。

### WP-01：公共 API、Canonical IR 与错误

Owner：API 小组

主要路径：`src/kernel/buckyos-api/src/aicc_client.rs`、`src/kernel/buckyos-api/src/aicc_usage_log.rs`

依赖：Gate 0

- [x] 为每个 typed method 定义独立 request/response；
- [x] 定义 `route.resolve`、Helper、cancel 和管理 API；
- [x] 区分 method、api_type 和 Capability；
- [x] 定义 AiMessage content block、tool、thinking 和 ProviderState；
- [x] 定义 ResourceRef、artifact、usage、cost 和 route trace；
- [x] 定义稳定错误码和 kRPC/task error 边界；
- [x] 重写 `AiccClient`、`AiccHandler`、`AiccServerHandler`；
- [x] 删除内部 all-in-one `AiMethodRequest` 路径和临时 method alias；已由 `buckyos-api::aicc_client` 导出的 method/type 保持兼容，但将旧 `reload_settings` 更新为唯一的 `service.reload_settings`；
- [x] 增加 serde round-trip、unknown field 和非法 schema 测试。

完成标准：Rust 服务端、Workflow 和其它 Rust 调用方不再手写协议字段或 method 字符串。

实现记录：公共 API 已改为逐方法强类型 request/response，Canonical IR、稳定错误边界和 client/handler/server dispatch 已落地；Workflow、OpenDAN、Agent Tool、Control Panel 已迁移到类型化调用，管理面 wire method 只保留 `service.reload_settings`。验收覆盖 `buckyos-api`/`aicc` 单测、受影响 crate 全 target check、workspace test compile、协议残留扫描和格式检查；AICC 严格 clippy 被仓库范围外的既有告警基线阻断。

### WP-02：统一 MatchRule

Owner：Catalog/Matching 小组

依赖：Gate 0

- [x] 实现 `MatchRule -> CompiledMatchRule`；
- [x] 支持 exact、`*`、`?` 和转义；
- [x] 支持多维 AND、数组 OR、`not`、`exists` 和允许字段上的 range；
- [x] 为各业务 schema 声明主维度、允许维度和类型；
- [x] catalog/settings 加载时完成校验和编译；
- [x] 推理热路径只执行编译结果；
- [x] trace 记录规则 ID/位置和非敏感参与维度；
- [x] Model Driver、Provider Rules、request/pricing rules、发布 track 共用一套 contract tests；
- [x] 不引入新表达式或 regex 依赖。

完成标准：仓库内不存在第二套 wildcard、predicate 或配置规则 DSL。

实现记录：Owner 为 Catalog/Matching 小组；关联提交为 `a12b3e09`；统一 contract、业务 schema、加载期编译入口、有序 first-match 和脱敏 trace 已落在 `src/frame/aicc/src/matching/mod.rs`。`cargo test -p aicc` 的 10 个测试、`cargo check -p aicc --all-targets`、AICC clippy `-D warnings` 和格式检查均通过，未引入 regex、表达式引擎或新的第三方依赖。剩余集成约束是 WP-03 catalog、后续 settings loader 和推理调用方只能消费本工作包提供的编译接口，不得新增独立 matcher；完整 `buckyos-build.py --skip-web` 仍需具备四个 `BUCKYOS_SDK_TOOL_*` 不可变构建输入后复验。

### WP-03：Catalog 与 Metadata Snapshot

Owner：Metadata 小组

依赖：WP-02

- [x] 实现 Model Driver Catalog DTO；
- [x] 实现 Provider Rules Catalog DTO；
- [x] 实现 Known Provider Catalog DTO；
- [x] 校验 schema version、revision、required features 和引用；
- [x] 构建 exact/pattern 索引和有序 first-match；
- [x] 实现跨 Model Driver 唯一匹配和 conservative fallback；
- [x] 静态能力、渠道规则、动态 discovery 事实保持分离；
- [x] Provider Rules 只能收窄 Model Driver 能力；
- [x] 从 NDN 当前文件集合构建不可变 `CatalogSnapshot`；
- [x] 不重复实现 NDN 下载、验签、activation 或防回退。

完成标准：给定同一 catalog 文件集合，总能生成确定且可校验的 snapshot 和索引。

实现记录：Owner 为 Metadata 小组；三类 catalog DTO、加载期校验与 MatchRule 编译、exact/pattern 索引、跨 Model Driver 唯一匹配、conservative fallback、能力收窄和不可变 `CatalogSnapshot` 已落在 `src/frame/aicc/src/catalog/mod.rs`。输入边界只接收 NDN 已交付的当前文件内容，不实现下载、验签、activation、防回退或持久缓存。`cargo test -p aicc` 的 17 个测试、`cargo check -p aicc --all-targets`、AICC clippy `-D warnings` 和格式检查均通过，未新增依赖；完整系统构建和后续 RuntimeSnapshot 集成由对应工作包继续验证。

### WP-04：Model Registry 与逻辑目录

Owner：Model/Router 小组

依赖：WP-01、WP-02、WP-03

- [x] 实现 ModelUID、exact model、origin model 和 variant；
- [x] 实现完整 Provider/Adapter/Driver/model 身份链；
- [x] 实现 LogicalModelDefinition、mount、min_line、disable_line；
- [x] 实现系统目录、用户 overlay 和 session overlay 合成；
- [x] 实现目录软链接、循环检测和 fallback 深度限制；
- [x] 从 fixture inventory 构建模型索引和逻辑挂载；
- [x] 输出 UI 和 trace 所需的只读模型视图；
- [x] exact model 不依赖字符串猜 Provider 或能力。

完成标准：不连接真实 Provider 也能通过 fixture 完整构建目录和候选集合。

实现记录：Model/Router 小组在 `src/frame/aicc/src/model/mod.rs` 实现 ModelUID、exact/origin/variant 和完整 Provider/Adapter/Driver/model 身份链，以及逻辑目录定义、分层 overlay、软链接、循环与 namespace 校验、最大 5 层 fallback、fixture inventory 索引和只读模型视图。候选展开保留来源路径、权重及 admission/rejection 原因，exact model 仅按显式身份索引，不从名称推断 Provider 或能力。AICC 全量 77 个测试、`cargo check -p aicc --all-targets`、格式与 diff 检查均通过，未新增依赖；真实 inventory 和运行时 Router 的集成分别由 WP-07、WP-10 继续完成。

### WP-05：Protocol 基础设施

Owner：Protocol Infra 小组

依赖：WP-01

- [x] HTTP client、timeout、proxy、Retry-After 和 request ID；
- [x] SSE framing、断线和终止标记，不解释业务事件；
- [x] 有界 JSON、multipart 和必要时的 websocket primitive；
- [x] Bearer、named header、fal key 和可选 GLM JWT；
- [x] OperationDescriptor、codec registry 和 adapter descriptor；
- [x] immediate、stream 和 native task 的统一协议结果；
- [x] polling、deadline、backoff、cancel 和 webhook 基础算法；
- [x] golden request/response/SSE/error contract harness；
- [x] 日志中只记录匿名 credential reference 和类型。

完成标准：基础 transport 完全不知道 Provider、model、routing 和 catalog。

实现记录：Protocol Infra 小组完成 HTTP buffered/streaming transport、有界 JSON/multipart、SSE framing、四类 credential、GLM 短期 JWT、descriptor/codec registry、统一执行结果和原生任务生命周期原语；credential 与 transport 的调试输出只暴露类型、匿名引用和非敏感结构信息。当前没有选中的 realtime operation 需要 websocket，因此按目标规范未预建 websocket primitive。Protocol 模块 33 个正常、边界、错误和并发语义单测及隔离 clippy `-D warnings` 通过，且边界扫描确认不引用 Provider、model、routing 或 catalog。

补充实现记录：根据 WP-06A/B/C/D 的集成反馈，operation descriptor 改为官方 operation ID 下显式声明多个 `(ApiType, Capability, execution modes)` binding，registry 按 `(adapter, operation, ApiType)` 事务注册和分发，并校验 buffered/streaming/native-task codec 集合完整性。新增类型化 `CodecContext`（resolved base URL、脱敏 credential、已物化资源、调用级 timeout/body limits）、增量 `StreamingHttpResponse -> SseFrameStream -> ProtocolStream` 入口、供 codec 映射 Provider 错误的有界非 2xx collector，以及独立于公共 `AiccCall` 的 native-task submit/status/result/cancel contract；SSE 正常终止、EOF、断线、格式错误和全程 body 上限均有单测覆盖。隔离验证 44 个 protocol 测试及 AICC `clippy --no-deps -D warnings` 通过。

Native-task submit 后续补充为必须携带并校验 canonical `CodecInput`，从而让 codec 类型安全读取 typed request 业务字段；status/result/cancel 仍仅依赖 remote task ID、resolved parameters 和调用上下文。同一 `models.predictLongRunning` operation 下四种 video ApiType 的合法提交、缺失 canonical input 和 ApiType/request 错配均有测试覆盖。

派生 Adapter 后续补充 `CodecRegistry::register_derived` 委托机制：派生层只注册真实 override，未覆盖且 descriptor/binding 完全一致的 buffered、streaming 和 native-task codec 从已注册 `base_adapter_id` 继承；不兼容声明必须显式提供 codec，注册失败保持事务性且不影响基础 Adapter。`sn-openai -> openai-responses` 的 encode、buffered decode、streaming decode 委托和不兼容声明拒绝已有单测，当前基线隔离运行 83 个 protocol 测试及 AICC `clippy --no-deps -D warnings` 通过。

### WP-06：基础 Protocol Codec

Owner：四个并行协议小组

依赖：WP-01、WP-05

#### WP-06A OpenAI Responses

- [x] Responses request/response/event/error；
- [x] tool、structured output、reasoning、usage 和 ProviderState；
- [x] OpenAI embeddings/images/audio/videos 独立 operation，按首版矩阵实现；
- [x] 不包含 OpenRouter、DeepSeek、豆包、Qwen 或 SN 分支。

实现记录：在 `src/frame/aicc/src/protocol/openai_responses.rs` 实现独立 `openai-responses` Adapter，覆盖 Responses 请求、即时响应、真正增量的 SSE event、OpenAI 错误体、function tool、JSON schema structured output、reasoning、usage 和可回放 ProviderState；GPT 主线图片能力通过 Responses `image_generation` tool lowering，completed 图片结果生成通用 artifact。另按官方 operation 独立注册 embeddings、Images generate/edit/inpaint、Audio speech/transcriptions 和 Videos submit/status/content/cancel，视频生命周期复用 WP-05 native-task contract。基础 codec 只消费已解析的模型、参数、base URL、Bearer credential 和已物化资源，不包含 OpenRouter、DeepSeek、豆包、Qwen、SN 或 Router/Model 分支。11 个定向合同测试随 AICC 全量 158 个测试通过，all-target check、格式检查及排除 resource 模块既有 `manual_is_multiple_of` lint 后的 clippy `-D warnings` 通过；完整 `buckyos-build.py --skip-web` 仍需提供四个 `BUCKYOS_SDK_TOOL_*` 不可变构建输入后复验。

#### WP-06B Claude Messages

- [x] Messages request/response/content block；
- [x] version header、SSE、tool use、thinking 和 usage；
- [x] 不包含 MiniMax 分支。

实现记录：Claude Messages 小组已在 `src/frame/aicc/src/protocol/claude_messages.rs` 实现独立 `claude-messages / messages.create / llm` binding，覆盖 Messages 请求、响应、content block、`anthropic-version`、named-header credential、tool use/result、thinking/signature、ProviderState、usage、错误映射和真正增量的 SSE 归并；streaming 通过 WP-05 的 `StreamingHttpResponse -> SseFrameStream -> ProtocolStream` 接口逐块消费，不缓冲完整响应，且保留 request ID、Retry-After、断连和有界非 2xx 错误。基础 codec 只包含 Claude Messages wire 语义，不引用 MiniMax 或 Provider/Router/Model 模块。8 个 Claude 合同单测随 AICC 全量 158 个测试通过，all-target check、排除 Resource 模块既有 `manual_is_multiple_of` lint 后的 clippy `-D warnings` 和格式检查通过；完整 `buckyos-build.py --skip-web` 仍需提供四个 `BUCKYOS_SDK_TOOL_*` 不可变构建输入后复验。

#### WP-06C Gemini Interactions

- [x] Interactions request/response/event；
- [x] Gemini auth、files、embeddings 和首版 gen-media；
- [x] 只有实际 Provider 需求确认后才增加 `generateContent` 历史 Adapter。

实现记录：Gemini Interactions 小组已在 `src/frame/aicc/src/protocol/gemini.rs` 实现独立 `gemini-interactions` Adapter，覆盖 `interactions.create` 请求、即时响应、增量 SSE event、错误映射、tool、usage、ProviderState，以及 `x-goog-api-key` named-header credential；另按官方 operation 独立注册 `models.embedContent` 和 `models.predictLongRunning`，覆盖文本/多模态 embeddings、首版图片/语音/音乐生成与视频 native-task 生命周期，并实现 Gemini Files 可恢复上传、查询和删除。基础 codec 只消费 WP-05 提供的类型化上下文、已物化资源和 native-task contract，未加入尚无实际 Provider 需求的 `generateContent` 历史 Adapter。8 个 Gemini 合同单测随 AICC 全量 158 个测试通过，all-target check、排除 Resource 模块既有 `manual_is_multiple_of` lint 后的 clippy `-D warnings`、格式及 diff 检查通过；完整 `buckyos-build.py --skip-web` 仍需提供四个 `BUCKYOS_SDK_TOOL_*` 不可变构建输入后复验。

#### WP-06D OpenAI Chat Completions

- [x] 只实现一份协议族级历史 codec；
- [x] 服务 OpenRouter、Kimi 和 GLM；
- [x] 与 Responses 平级，不在调用失败后互相 fallback；
- [x] 三家派生 Provider 共同运行基础 contract。

完成标准：每个 API 代际独立注册、独立声明 operation，基础合同可被多个派生 Provider 复用。

实现记录：在 `src/frame/aicc/src/protocol/openai_chat_completions.rs` 实现独立 `openai-chat-completions / chat.completions.create / llm` binding 和注册入口，覆盖 canonical messages、图片、function tools、structured output、参数校验、即时响应、usage、错误映射及真正增量的 SSE 文本与 tool-call 参数归并。基础 codec 只消费已解析的模型、参数、URL 和凭据，不包含 OpenRouter、Kimi、GLM 或 Provider/Router/Model 分支，并通过三家消费者共用 contract 验证与 Responses 无失败后 fallback。后续按 WP-08D 的实际复用需求增加窄化的 `pub(crate)` dialect 扩展：派生 Adapter 可转换请求 JSON/header、即时响应和单个 SSE delta，可选择 `max_completion_tokens` 或 `max_tokens`，并可在基础参数未匹配时严格验证和转换 dialect 专属 resolved parameter；基础参数仍由基础 codec 校验，标准 Adapter 继续拒绝未知参数，派生输出不得覆盖基础字段，且未开放任意 `extra_body`。fake derived-adapter 合同覆盖请求、即时响应、SSE、基础 Adapter 声明和专属 resolved parameter 的完整委托链。11 个定向测试及隔离工作树内 AICC 全量 165 个测试通过，all-target check、排除 Resource 模块既有 `manual_is_multiple_of` lint 后的 clippy `-D warnings`、格式和 diff 检查通过；完整 `buckyos-build.py --skip-web` 仍需提供四个 `BUCKYOS_SDK_TOOL_*` 不可变构建输入后复验。

### WP-07：Provider Core、Discovery 与 Inventory

Owner：Provider Runtime 小组

依赖：WP-03、WP-05；可先使用 fake codec

- [x] 实现 Provider Profile、Instance 和 Registry；
- [x] 实现 resolved credential，凭据不进入 snapshot；
- [x] 定义统一 ProviderDiscovery；
- [x] discovery 只使用官方机器接口或 catalog-only，不抓网页；
- [x] 合并 Model Driver、Adapter operation 和 discovery 动态能力的交集；
- [x] inventory schema、fingerprint、revision 和 pricing source；
- [x] 实现 inventory LKGS fallback；
- [x] 实现实例 refresh loop、健康探测和退避；
- [x] 实现幂等 Stop、优雅退出和 generation token 迟到写保护；
- [x] 禁用、删除、替换和服务退出时先停止旧循环。

完成标准：Provider 生命周期可以完全通过 fake discovery 和 fake codec 测试，不依赖 Router。

实现记录：Provider Runtime 小组在 `src/frame/aicc/src/provider/mod.rs` 实现 Provider Profile、Instance、copy-on-write Registry、credential resolver、统一 `ProviderDiscovery`、catalog-only discovery、三方能力交集和带 fingerprint/revision/pricing source 的实例级 inventory；复用 WP-14 RDB LKGS 接口完成有效快照恢复，加入 refresh loop、健康探测、有界指数退避、单实例刷新互斥、幂等 Stop、优雅退出和 generation token 迟到写保护，禁用、删除、替换及服务退出均先停止旧循环。后续补充 SN 可复用的 `api_key`/`dynamic_login` 互斥认证配置、无 token 的动态登录上下文与解析 trait，以及统一的 region/workspace/account schema、默认 base URL 模板和显式 URL 覆盖解析。原始 12 个 fake discovery/fake codec 生命周期单测随 AICC 全量 158 个测试通过；补充契约在隔离工作树中通过全部 15 个 Provider 测试、all-target check、AICC `clippy --no-deps -D warnings`、格式和 diff 检查。主工作区复验仍被并行 WP-08E 的共享 crate 编译错误阻断；完整 `buckyos-build.py --skip-web` 仍需提供四个 `BUCKYOS_SDK_TOOL_*` 不可变构建输入后复验。全局 metadata target/applied 收敛和 RuntimeSnapshot 原子发布仍由 WP-15 负责。

补充实现记录：WP-03 在 `282e815e` 提供 `CatalogSnapshot::resolve_provider_origin`，正式执行 Provider Rules 的 `origin_provider_aliases`、`origin_mappings` 和 `metadata_drivers` 边界；WP-07 在 `b2cfd4be` 将其接入 `InventoryBuilder`，声明 mapping 时先解析唯一 origin model/driver，再以 singleton candidate 调用 Model Driver resolver，并拒绝 discovery 与 mapping 的 origin 冲突。Catalog 测试覆盖同名模型唯一选择、未知 vendor、冲突 mapping 和 metadata 越权，Provider 16 个测试及 all-target check 通过；严格 clippy 仅被 WP-08E 既有 `redundant_closure` 阻断，豁免该单项后通过。

### WP-08：内置 Provider 装配与 Dialect

Owner：七个可并行 Provider 小组

依赖：对应 WP-06、WP-07

| 子组 | Provider | 基础依赖 |
|---|---|---|
| WP-08A（已完成） | OpenAI | OpenAI Responses |
| WP-08B（已完成） | Claude、MiniMax | Claude Messages |
| WP-08C（已完成） | Gemini | Gemini Interactions |
| WP-08D（已完成） | OpenRouter、Kimi、GLM | OpenAI Chat Completions |
| WP-08E（已完成） | DeepSeek、豆包、Qwen | OpenAI Responses |
| WP-08F（已完成） | fal | task polling + fal Queue |
| WP-08G（已完成） | SN | OpenAI Responses + dynamic credential |

每个子组：

- [ ] 提供稳定 Profile ID、显示信息和默认 `base_url`；
- [ ] 提供 region/workspace/account 和 credential schema；
- [ ] 提供 discovery 或 catalog-only inventory；
- [ ] 提供 operation 和 Adapter 默认绑定；
- [ ] 提供 Provider Rules fixture；
- [ ] 复用基础 codec contract；
- [ ] 仅真实 schema/event/error 差异新增 dialect；
- [ ] dialect 声明 `base_adapter_id`、覆盖点和不支持能力；
- [ ] 能由 Profile/Rules 表达的差异不得建立空 dialect；
- [ ] 删除该 Provider 后，基础 codec 不需要修改。

WP-08A 实现记录：OpenAI builtin 装配已落在 `src/frame/aicc/src/provider/builtin/openai.rs`，提供稳定 `openai` Profile、显示信息、默认 `https://api.openai.com/v1`、Bearer credential schema，并明确 region/workspace/account 不支持；通过官方 `/v1/models` 机器接口构建动态 discovery snapshot，保留 ETag revision、健康状态和有界错误处理。内置 Provider Rules 固定 `metadata_drivers: ["openai"]`，将 LLM、embedding、image、audio 和 video API type 显式绑定到 WP-06A 的 `openai-responses` 及专用 operation。OpenAI 没有额外 wire 差异，因此直接复用基础 Adapter，不创建空 dialect，也未修改基础 codec。4 个 builtin 单元测试随 AICC 全量 162 个测试通过，library check、格式与 diff 检查通过；stable clippy 在豁免 Resource 模块既有 `manual_is_multiple_of` lint 后通过，未新增依赖。

WP-08B 实现记录：Claude 与 MiniMax builtin 装配分别落在 `src/frame/aicc/src/provider/builtin/claude.rs` 和 `src/frame/aicc/src/provider/builtin/minimax.rs`，提供稳定 Profile/显示信息、`x-api-key` named-header credential、region/workspace/account schema，以及 Claude 官方地址和 MiniMax global/china 地址解析。两者复用 `anthropic_models.rs` 的分页 Models API discovery，动态库存仅声明 `llm / messages.create`；Provider Rules 分别绑定 `claude`、`minimax` metadata driver 和 WP-06B 的 Messages operation。Claude 直接注册未修改的 `claude-messages` 基础 codec；MiniMax 的 `minimax-messages` 显式声明 `base_adapter_id: claude-messages`、覆盖点和不支持参数，只处理 MiniMax 已确认的参数范围、`base_resp` 错误及 ProviderState namespace，request/response/SSE 其余语义继续委托基础 codec。10 个 WP-08B 定向测试和 31 个 builtin 测试通过；基于当前已提交依赖、只叠加 WP-08B 的隔离基线中，AICC 全量 185 个测试、all-target check 和 stable clippy `--no-deps -D warnings`（豁免 Resource 模块既有新版本 lint）均通过。WP-08D/WP-08F 更新后，共享工作树 AICC 全量 236 个测试及 all-target check 通过；共享严格 clippy 仅被 WP-08E 的既有 `redundant_closure` 告警阻断。格式与 diff 检查通过，未新增依赖，删除 Claude/MiniMax builtin 与 MiniMax dialect 不要求修改基础 codec。

WP-08C 实现记录：Gemini builtin 装配已落在 `src/frame/aicc/src/provider/builtin/gemini.rs`，提供稳定 `gemini` Profile、`Google Gemini` 显示信息、默认 `https://generativelanguage.googleapis.com/v1beta`、`x-goog-api-key` named-header credential schema，并通过 WP-07 的统一 connection contract 明确 region/workspace/account 均不支持。Gemini discovery 使用官方分页 `/v1beta/models` 机器接口，校验模型资源名、合并重复模型的公开 generation methods、拒绝重复 page token，以稳定 hash 生成 inventory revision，并把官方方法映射到 `interactions.create`、`models.embedContent` 和 `models.predictLongRunning`；动态 discovery 不声明价格或扩张 Model Driver 能力。Provider Rules 固定 `metadata_drivers: ["gemini"]`，把首版 LLM、vision、embedding、image、audio 和 video API type 显式绑定到 WP-06C operation。Gemini 无额外渠道级 wire 差异，因此直接复用 `gemini-interactions` 基础 Adapter，未创建空 dialect，也未修改基础 codec。5 个 WP-08C 定向测试和 AICC 全量 234 个测试、all-target check、格式与 diff 检查通过；stable clippy 在豁免 WP-08E 的 `redundant_closure` 和 Resource 模块既有 `manual_is_multiple_of` lint 后以 `-D warnings` 通过，未新增依赖。真实 Gemini API 与 T1/T1.5/T2 验收按 WP-18 集成阶段执行。

WP-08D 实现记录：OpenRouter、Kimi 和 GLM builtin 装配分别落在 `src/frame/aicc/src/provider/builtin/openrouter.rs`、`kimi.rs` 和 `glm.rs`，提供稳定 Profile/显示信息、Bearer credential、统一 region/workspace/account schema、默认 base URL、LLM operation 绑定和 Provider Rules。OpenRouter 与 Kimi 使用官方 Models API discovery，保留 ETag/稳定 revision、公开能力和 OpenRouter 动态价格；OpenRouter Rules 通过 WP-07 的 origin mapping 将 `vendor/model` 显式归属到 Model Driver，即使跨 driver 存在同名模型也不猜测。GLM 提供 global/china 地址解析、Bearer/可选 GLM JWT schema 和显式 catalog-only inventory。`src/frame/aicc/src/protocol/chat_completions_dialects.rs` 注册三个窄 dialect，均声明 `base_adapter_id: openai-chat-completions`、请求/响应覆盖点和不支持项，只处理三家已确认的 routing、thinking/reasoning、cache、partial、tool stream 与 ProviderState 差异；基础请求、响应、SSE 和错误继续委托 WP-06D codec，基础 codec 未加入 Provider 分支。13 个 WP-08D 定向测试随 AICC 全量 236 个测试通过，`cargo check -p aicc --all-targets`、格式与 diff 检查通过；stable clippy 在豁免 WP-08E 的 `redundant_closure` 和 Resource 模块既有 `manual_is_multiple_of` 后以 `-D warnings` 通过，未新增依赖。完整 `buckyos-build.py --skip-web` 仍需提供四个 `BUCKYOS_SDK_TOOL_*` 不可变构建输入后复验。

WP-08E 实现记录：DeepSeek、豆包和 Qwen builtin 装配已落在 `src/frame/aicc/src/provider/builtin/wp08e.rs`，提供稳定 Profile/显示信息、Bearer credential、统一 region/workspace/account schema、默认 base URL 模板、LLM operation 绑定和 Provider Rules；Qwen 按 workspace 与五个明确 region 解析专属域名，豆包使用方舟 `/api/v3`，DeepSeek 使用官方 `/models` 机器接口并保留 ETag revision，豆包/Qwen 使用显式模型 ID 的 catalog-only inventory，不硬编码易变模型或价格。`src/frame/aicc/src/protocol/derived_responses.rs` 注册 `deepseek-responses`、`doubao-responses`、`qwen-responses` 三个窄 dialect，均声明 `base_adapter_id: openai-responses`、覆盖点和不支持项；派生层只处理已确认的参数限制、Qwen session-cache header 和 ProviderState namespace，request/response/SSE/error 继续委托 WP-06A 基础 codec。14 个装配、库存身份链、派生注册、请求约束与委托单测通过，AICC 全量 229 个测试和 `cargo check -p aicc --all-targets` 通过，未新增依赖；豆包/Qwen 原生媒体 operation 按本文建议批次 4 另行实施，不属于本次 Responses 主接口装配。

WP-08F 实现记录（已完成）：fal builtin 装配已落在 `src/frame/aicc/src/provider/builtin/fal.rs`，提供稳定 `fal` Profile、显示信息、默认 `https://queue.fal.run`、`Authorization: Key` credential schema，并明确 region/workspace/account 不支持；使用 catalog-only inventory 提供首版图像放大、背景移除、音频增强和视频放大 endpoint fixture。`src/frame/aicc/src/protocol/fal_queue.rs` 实现独立 `fal-queue / queue.submit` native-task Adapter，覆盖媒体 typed request lowering、submit/status/result/cancel、Submitted/Queued/Running/Succeeded/Failed/Cancelled 映射、官方错误、路径校验、资源归一化及 artifact 输出，复用 WP-05 polling/deadline/backoff/cancel contract，不引入基础 codec 或 Provider 名分支。Provider Rules 将 14 个 image/audio/video API type 显式绑定到 Queue operation；未新增依赖。fal 定向测试 19/19、AICC 全量测试 229/229、all-target check 和格式检查通过；排除 WP-08E `redundant_closure` 与 resource 模块既有 `manual_is_multiple_of` lint 后，clippy `-D warnings` 通过。

WP-08G 实现记录：SN 扩展 Provider 装配已落在 `src/frame/aicc/src/provider/builtin/sn.rs`，提供稳定 `sn` Profile、`SN AI Provider` 显示信息、默认 `https://sn.buckyos.ai/api/v1/ai`、region/workspace/account schema、互斥的 `api_key`/`dynamic_login` credential schema，以及绑定 `openai` Model Driver 和 `responses.create` 的 Provider Rules。SN discovery 使用带 Bearer credential 的 `/models` 机器接口，仅把真实返回的模型作为 LLM inventory。`sn-openai` 声明 `base_adapter_id: openai-responses`，仅覆盖 credential resolution 并通过 WP-05 派生注册直接委托基础 Responses codec；动态登录复用 BuckyOS device-token 登录 API，按 TTL 在内存缓存 token，同实例并发刷新合并，替换/删除时可失效，认证错误及 Debug 输出不暴露 token。7 个 SN 正常、边界、错误、并发、inventory 和基础 codec 隔离测试通过，`cargo check -p aicc --all-targets` 通过，未新增依赖；AICC 全量 218 个测试中 216 个通过，剩余 2 个失败来自并行 WP-08B/WP-08F，严格 clippy 也仅剩对应并行文件的 3 项告警。

建议批次：

1. 所有 Provider 先完成主文本接口和 discovery；
2. fal Queue；
3. OpenAI/Gemini 专用媒体 operation；
4. MiniMax/GLM/豆包/Qwen 原生媒体 operation。

### WP-09：Routing Policy、Quota、Budget、Privacy 与 Trust

Owner：Policy/Security 小组

依赖：WP-01、WP-13 usage query contract；可使用 fake quota source

- [x] 定义 system/user/app/session/request policy 合并顺序；
- [x] 模型能力和逻辑目录 admission 由 Model Registry 负责，本工作包不建立第二套模型 admission；
- [x] 实现 locked policy，低优先级不能放宽；
- [x] 实现 local_only、local_first 和 Provider trust 判定；
- [x] 实现单次 cost ceiling、quota availability 和 budget rejection；
- [x] 定义 `quota.query`；
- [x] 只输出硬约束判定和可解释原因，不负责候选评分；
- [x] fail closed 处理安全真相源读取失败；
- [x] 跨租户、跨应用和 credential scope 测试。

完成标准：实现位于 routing 内部策略层而非独立顶层模块；Router 把策略判定作为确定的 hard filter 输入，不自行读取 quota 或安全配置。

实现记录：Policy/Security 小组在 `src/frame/aicc/src/routing/policy.rs` 实现 system、user、app、session、request 五级字段合并和 locked 冲突拒绝，复用统一 MatchRule 编译 Provider allow/block 规则；策略引擎以只读 trust、credential scope 和 quota source 视图判定 trusted local、隐私、单次成本、请求额度与剩余预算，只返回 hard filter 原因和 local-first 偏好，不实现模型 admission 或候选评分。`quota.query` 通过已冻结公共 DTO 返回调用者作用域视图，安全事实源失败、未知状态和预算存在但成本不可估算均 fail closed。新增 9 个单元测试覆盖合并顺序、locked、local/privacy/trust、quota/budget、共享 matcher、跨租户/应用/credential scope 和查询隔离；基于干净 HEAD 叠加本模块的 AICC 86 个测试全部通过，stable clippy `--no-deps -D warnings` 在豁免仓库既有 Resource 新版本 lint 后通过。真实工作区全量编译暂受并行 Claude codec 与 OperationCodec 接口未同步阻断。

### WP-10：Routing、Scheduler 与 Trace

Owner：Model/Router 小组

依赖：WP-04、WP-09；使用 fixture inventory 可早于真实 Provider 完成

- [x] exact route 和 logical route；
- [x] method/operation/capability/feature 硬过滤；
- [x] privacy/trust/budget/health/allow/block 硬过滤；
- [x] 逻辑目录展开和 strict/parent/target fallback；
- [x] exact model 默认不 fallback；
- [x] balanced、cost、latency、quality、local 和 strict_local profile；
- [x] item weight、exact model weight 和确定性 tie-break；
- [x] session 历史 exact model 软优先，但 AICC 不维护 session cache；
- [x] 输出 RouteDecision、fallback candidates、完整 trace 和用户摘要；
- [x] trace 不记录 prompt、资源内容、credential 或敏感 option。

完成标准：T1 能确定性证明每个候选被选择或排除的原因。

实现记录：Model/Router 小组在 `src/frame/aicc/src/routing/mod.rs` 实现 exact/logical 两类内部路由、method/api_type/capability/operation/feature 与 Provider runtime/policy 硬过滤、过滤后继续 fallback，以及按目录 item 权重、exact model 权重、profile 分数和 exact model 名依次排序的确定性 scheduler。六类 profile、动态成本/延迟/可靠性/质量/本地性评分、调用方传入的 session 历史 exact model 软偏好、完整有序 fallback candidates、结构化 route trace 和固定模板用户摘要均已落地；trace 类型不接收 prompt、资源、credential 或 provider option。新增 12 个单元测试覆盖正常、边界、拒绝、fallback、权重、六类 profile、历史偏好和脱敏；在干净 WP-09 基线叠加本模块后，AICC 98 个测试、all-target check 和 stable clippy `--no-deps -D warnings`（豁免仓库既有 Resource 新版本 lint）通过，当前并行集成工作区的 AICC 157 个测试与 all-target check 也已通过，未新增依赖。真实 Provider read view 到 `CandidateRuntimeState` 的装配由后续 RuntimeSnapshot/API 集成工作包完成。

### WP-11：Call Lowering

Owner：Protocol/Router 联合小组

依赖：WP-01、WP-03、WP-06、WP-10

- [ ] 建立进入协议层的唯一 lowering；
- [ ] 解析 exact variant；
- [ ] 应用 Provider Rules operation 映射；
- [ ] 应用 request defaults、rewrite、delete 和 provider options；
- [ ] 明确全部参数优先级；
- [ ] 校验 Provider variant 完整覆盖 Model Driver variant；
- [ ] 最终确定 operation、credential 和资源需求；
- [ ] Adapter 不根据 Provider/model 名猜 operation；
- [ ] 为每个 Provider/operation 建立 golden lowering fixture。

完成标准：相同 RouteDecision、canonical request 和 catalog snapshot 必须产生相同 ResolvedProviderCall。

### WP-12：Execution、Task、Cancel 与 Idempotency

Owner：Execution 小组

依赖：WP-01、WP-05、WP-11；可先使用 fake Provider

- [ ] 统一 immediate、Provider stream 和 task-backed 外部语义；
- [ ] Provider streaming 中间态写入 TaskMgr event/task data；
- [ ] AICC RPC 不新增独立 streaming transport；
- [ ] 实现启动阶段 failover；
- [ ] Provider task 提交成功后固定 runtime/Adapter，不跨 Provider 重试；
- [ ] 映射 Submitted/Queued/Running/Succeeded/Failed/Cancelled；
- [ ] cancel 返回真实上游取消或本地中止结果；
- [ ] 实现 idempotency scope、canonical body fingerprint 和 conflict；
- [ ] 统一 immediate 与 long task 的 usage completion path；
- [ ] 覆盖重启、并发、迟到 completion 和取消竞态。

完成标准：所有执行形态对调用方暴露一致的 task、event、error、usage 和 cancel 语义。

### WP-13：Resource 与 Artifact

Owner：Resource/Security 小组

依赖：WP-01、平台 Named Store/FileObject API

- [x] URL、Base64、NamedObject 鉴权；
- [x] Router 只读取 ObjId 和 FileObject meta；
- [x] Provider 选定后才读取 bytes；
- [x] MIME、大小、数量和批量限制；
- [x] 压缩包深度、文件数、加密、膨胀比和路径穿越防护；
- [x] 最后一跳上传和 multipart；
- [x] 输出 artifact/FileObject meta；
- [x] embedding 大结果 artifact 的 rows/dimensions/space metadata；
- [x] 普通日志和 trace 不保留原始内容。

完成标准：资源权限、格式或限制失败发生在 Provider 调用前，并返回稳定 `resource_invalid`。

实现记录：Resource/Security 小组已在 `src/frame/aicc/src/resource/mod.rs` 实现分阶段资源预检和最后一跳物化、显式鉴权接口、MIME 与配额限制、ZIP 安全检查、multipart、NamedDataMgr/FileObject artifact 写入及 embedding metadata；所有资源失败统一映射为非重试 `resource_invalid`，调试输出不包含原始内容或 digest。模块 8 个单元测试、`cargo test -p aicc`、all-target check、AICC clippy `-D warnings` 和格式检查均通过。RPC token 验证、RBAC authorizer 装配及真实执行链接入由 WP-16 Service Integration 完成。

### WP-14：Storage、Usage、Trace 与 Observability

Owner：Storage/Observability 小组

依赖：WP-01；可与协议开发并行

- [x] 建立 `aicc_provider_inventory_lkgs`；
- [x] 原子 upsert、schema、SHA-256 和无效行重建；
- [x] inventory 与 metadata applied seq 同事务提交；
- [x] 重写 usage event 写入，保留 RDB 作为唯一持久事实源；
- [x] 每个成功 Provider completion 严格写一条 usage；
- [x] 成功响应缺 usage 视为 Provider protocol error；
- [x] `(tenant,idempotency_key)` 和 `(tenant,task_id)` 去重；
- [x] 实现 usage filter/group/bucket/cursor；
- [x] 实现 route trace/audit 查询和 retention；
- [x] metrics 覆盖 latency、error、queue、health、refresh 和 snapshot generation；
- [x] 建立统一 redaction，并对测试报告做 secret/content 扫描。

完成标准：TaskMgr completed task 删除不影响 usage，所有诊断能用 request/task/route/provider trace ID 关联。

实现记录：Storage/Observability 小组已在 `src/frame/aicc/src/storage/mod.rs` 实现基于平台 RDB instance 的 inventory LKGS、usage、route trace 和 audit 持久化，包含跨 SQLite/Postgres 的原子 inventory/metadata seq upsert、SHA-256 校验与坏行重建、completion usage 强制和双重去重、filter/group/bucket/cursor 查询、四级 trace ID 关联及诊断 retention；`src/frame/aicc/src/observability/mod.rs` 实现 latency/error/queue/health/refresh/snapshot generation metrics、统一递归脱敏和 secret/content 扫描。Storage/Observability 7 个单元测试随 AICC 全量 77 个测试通过，all-target check、AICC `cargo clippy --no-deps -D warnings` 和格式检查通过；完整依赖 clippy 仍受 `buckyos-api` 既有 3 个 `redundant_field_names` 告警阻断。运行时装配及真实 completion/Provider lifecycle 接入由后续对应工作包完成。

### WP-15：RuntimeSnapshot、Settings 与 Metadata 收敛

Owner：Runtime/Consistency 小组

依赖：WP-03、WP-07、WP-14

- [ ] 实现不可变 RuntimeSnapshot；
- [ ] add/reload/metadata refresh 在不可见候选区完成；
- [ ] 校验成功后一次性替换完整 `Arc<RuntimeSnapshot>`；
- [ ] 请求只捕获一次 snapshot；
- [ ] 只解析统一 `providers[]` settings；
- [ ] 实现 target/applied/updating seq；
- [ ] 推理前和任一 Provider refresh 触发同一个全局收敛；
- [ ] 并发收敛合并为单执行者；
- [ ] 单 Provider 失败保留旧 inventory 和旧 applied seq；
- [ ] 刷新过程中目标推进时只提交本轮捕获序列；
- [ ] 列表未变且 seq 相同只做 probe，不重写 inventory；
- [ ] reload/delete/disable/replace/exit 无孤儿任务和迟到写。

完成标准：并发请求只能观察到完整旧代或完整新代，不能看到半加入 Provider 或混合 catalog revision。

### WP-16：Service 与管理 API

Owner：Service Integration 小组

依赖：WP-07、WP-09 至 WP-15

- [ ] 重写进程启动、依赖装配、kRPC dispatch 和优雅退出；
- [ ] 实现 `models.list`、`provider.catalog`、`protocol_adapter.list`；
- [ ] 实现 `provider.validate/add/update/delete/refresh_models/list/health`；
- [ ] 实现 `usage.query`、`trace.query` 和 `quota.query`；
- [ ] 实现 `driver_metadata_update.get/set`；
- [ ] 管理面只实现 `service.reload_settings`，同步更新 `buckyos-api` 和全部调用方，删除旧 `reload_settings` 及错误拼写；
- [ ] 所有写操作使用当前 RPC token 调 system-config；
- [ ] 所有 settings 写使用 `exec_tx` + revision CAS；
- [ ] `provider.validate` 不落盘；
- [ ] 写入后构建候选 snapshot，再原子发布；
- [ ] 管理 API RBAC 和跨租户测试；
- [ ] 所有响应和日志脱敏。

完成标准：前端不直接读写 system-config，管理操作不会产生 settings 已写但 runtime 半生效的状态。

### WP-17：调用方联动

Owner：四个并行集成小组

依赖：WP-01 可开始 mock 对接；最终依赖 WP-16

#### WP-17A Desktop / AI Center

- [ ] 更新 `src/frame/desktop/src/api/aicc_mgr.ts`；
- [ ] Provider DataModel、表单和管理 RPC 请求/响应统一使用 `base_url`，删除 `endpoint` 字段及转换层；
- [ ] Provider Wizard 使用 Profile/Instance/Adapter 新身份；
- [ ] 普通用户不选择 API 代际；
- [ ] 接入 catalog、validate、health、usage、trace 和 settings conflict；
- [ ] 删除旧 control-panel AICC 辅助配置依赖。

#### WP-17B Workflow

- [ ] 更新 `src/kernel/workflow/src/adapters/aicc.rs`；
- [ ] method schema 复用 `buckyos-api` 已导出的公共契约；
- [ ] 删除内部重复 DTO 和 all-in-one payload；除已冻结的 reload method 更新外，不重命名或删除其它已导出的 RPC method/type；
- [ ] 验证 Helper、typed inference、cancel 和 task response。

#### WP-17C OpenDAN/Jarvis/CLI

- [ ] 更新 `src/tools/buckyos-agent/lib/aicc.ts` 和各命令；
- [ ] 更新图像、音频、视频 command-to-method 映射；
- [ ] 更新 Jarvis behavior/include；
- [ ] 保持 task event 进度链路；
- [ ] 删除旧 payload 和 method alias。

#### WP-17D Rootfs / Dev Config / RBAC

- [ ] 把 `src/dev_configs/aicc.json` 切换为统一 `providers[]`；
- [ ] 更新 rootfs service settings、RBAC 和默认配置；
- [ ] 检查空配置启动、安装、reinstall 和 reload；
- [ ] 确认 credential 只使用 locked value/reference。

完成标准：仓库调用方不再引用旧 settings section 或内部重复 DTO；settings 不再使用 `provider_driver`，但公共 RPC/报告兼容字段继续保留。

### WP-18：验收基础、CI 与发布报告

Owner：E2E 小组

依赖：Gate 0 后立即开始，贯穿全部工作包

- [ ] 分别校验 canonical method 值域、api_type 值域和显式合法关联，不检查同名或双射；
- [ ] Provider baseline schema 增加 Profile/Adapter/Model Driver 身份；`provider_driver` 可继续作为与公共 RPC 一致的兼容/报告分组字段，不承担 settings 身份语义；
- [ ] 集成测试阶段把 11 家 Provider 和 SN 加入参数化 baseline：先补齐 T1/T1.5，再补齐 T2，最后进入 T3；
- [ ] 固定 fixture manifest、Mock Provider contract 和 report schema；
- [ ] 汇总 WP-01 至 WP-17 的模块单元测试入口和覆盖范围；
- [ ] AiccClient request/response、序列化和错误映射测试；
- [ ] T1 经 Zone Gateway 执行真实 AICC 路由链路和多 Mock Provider；
- [ ] T1.5 经 Zone Gateway 执行真实 AICC typed/helper method、真实 Adapter 和 Provider 专用高保真 Mock；
- [ ] T1.5 fixture 只依据 Provider 官方 API 文档、官方 schema、官方 SDK 协议定义和官方错误文档；
- [ ] T1.5 覆盖 Provider driver × Adapter/API version × API-Type × operation，以及每个可独立调用的 metadata variant；
- [ ] T2 官方 inventory 双向 diff 和 `ProviderInstance × model × API-Type` 最小真实推理矩阵；
- [ ] T3 message-tunnel/Jarvis 六类消息、多附件、多轮和入口矩阵；
- [ ] 全局/Provider 并发、最小间隔、重试、timeout 和预算门禁；
- [ ] cleanup、settings 字节恢复、Named Object 和消息资源清理；
- [ ] targeted retest command、finance report 和 product defect evidence；
- [ ] 默认 CI 不产生真实模型费用。

完成标准：每个需求、method、Provider、operation 和横切能力都能追踪到模块单测或稳定的 T1/T1.5/T2/T3 case ID，并能明确证明四层之间没有用后一层重复代替前一层。

## 6. 实施波次与并行关系

```text
Gate 0：契约冻结
  │
  ├─ WP-00 工程骨架
  ├─ WP-01 API/IR/Error ────────────────┐
  ├─ WP-02 MatchRule ─→ WP-03 Catalog ─┤
  ├─ WP-05 Protocol Infra ─────────────┤
  ├─ WP-13 Resource ───────────────────┤
  ├─ WP-14 Storage/Observability ──────┤
  └─ WP-18 验收基础 ───────────────────┘
                                         │
      ┌──────────────────────────────────┼────────────────────────┐
      │                                  │                        │
  WP-06 四类基础 codec             WP-04 Model/Directory    Unit/T1/T1.5 fixtures
      │                                  │                        │
  WP-07 Provider Core               WP-09 Admission              │
      │                                  │                        │
  WP-08 11+1 Providers              WP-10 Routing                │
      └──────────────────────────────────┴─→ WP-11 Call Lowering ─┘
                                                │
                         WP-12 Execution ───────┤
                         WP-15 RuntimeSnapshot ─┤
                                                │
                                        WP-16 Service/Admin
                                                │
                                        WP-17 调用方联动
                                                │
                                  单次切换入口并删除旧实现
                                                │
                                       T1 + T1.5 全量
                                                │
                                           T2 + T3
```

### Wave 0：规格冻结与骨架

包含：Gate 0、WP-00、WP-18 初始 skeleton。

出口条件：

- canonical method/api_type 表唯一；
- 公共 DTO 和错误 contract 可编译；
- fixture、case manifest 和报告 schema 已冻结；
- 模块 owner 和集成人已明确。

### Wave 1：公共地基

并行：WP-01、WP-02、WP-05、WP-13、WP-14、WP-18。

出口条件：

- API serde 和 client contract 通过；
- MatchRule contract 通过；
- HTTP/SSE/task fake 可独立运行；
- storage schema 和 mock repository 可用；
- 各工作包只运行与本轮修改对应的模块单元测试；
- 不连接真实 Provider。

### Wave 2：协议、Catalog、模型和路由

并行：WP-03、WP-04、WP-06、WP-07、WP-09、WP-10、WP-18。

出口条件：

- 四类基础 protocol golden 通过；
- fixture inventory 可完整构建 Model Registry；
- route -> lower -> fake execute 最小闭环通过；
- routing、fallback、policy、trace 和 protocol 模块单元测试通过；
- 本阶段不把全量 T1/T1.5 当作编码完成门禁。

### Wave 3：Provider 批量接入

并行：WP-08A 至 WP-08G，同时推进 WP-11、WP-12、WP-13。

出口条件：

- 11 家 Provider 和 SN 都有主接口、Profile、Rules、discovery fixture；
- 每个 dialect 复用对应基础 codec contract；
- 每个 Provider inventory 含完整身份链、operation、capability 和 pricing source；
- 基础 codec 不含 Provider/model 名分支；
- 每个 Provider 和 dialect 的模块单元测试通过。

### Wave 4：运行时一致性和服务管理

包含：WP-14、WP-15、WP-16。

出口条件：

- add/update/reload/refresh 原子发布；
- delete/disable/replace/exit 无孤儿 refresh task；
- metadata seq 并发收敛通过；
- 管理、usage、resource、task、安全和异常路径模块单元测试通过。

### Wave 5：调用方迁移和切换

并行：WP-17A 至 WP-17D。

出口条件：

- Desktop、Workflow、Jarvis 和 CLI 全部只使用新 contract；
- dev/rootfs settings 已切换；
- 仓库内旧 method、旧 settings 字段和旧 Provider 身份引用扫描为零；
- 所有模块单元测试和 build 通过；
- 功能模块编码到此结束；Wave 6 完成旧实现删除、最终单测和编码冻结后，才开始全量 T1/T1.5。

### Wave 6：旧实现删除和编码冻结

- [ ] 把 `lib.rs`、`main.rs` 和 service entry 一次性切到新实现；
- [ ] 删除旧 Provider 单体、旧 router/session、旧 metadata updater 和兼容入口；
- [ ] 删除只服务旧实现的测试、fixture 和配置；
- [ ] 更新根目录设计文档中的实现状态；
- [ ] 执行全部模块单元测试和 build；
- [ ] 冻结进入集成测试的 commit；
- [ ] 未通过模块单测或仍包含旧实现时不得进入 T1/T1.5。

### Wave 7：T1/T1.5 集成验收

- [ ] 先执行 T1 全量，完成路由、fallback、多 instance、policy、task、usage、安全和错误注入验收；
- [ ] 再执行 T1.5 全 Provider 协议契约矩阵；
- [ ] T1.5 覆盖正常响应、streaming、异步状态、cancel、官方错误和 metadata variant lowering；
- [ ] T1/T1.5 必须零真实 Provider 调用、零真实推理费用；
- [ ] 对失败进行批量分类、集中修复、构建部署和同范围复测；
- [ ] 实现发生变化后，重新执行受影响模块单测，再完整重跑受影响 T1/T1.5 范围；
- [ ] 最终必须有一次在两次运行之间没有实现修改的完整通过；
- [ ] T1/T1.5 未全部达到门禁时，不得进入 T2/T3。

### Wave 8：T2/T3 发布验收

- [ ] 获得当次 T2/T3 明确授权；
- [ ] 执行 T2 `ProviderInstance × model × API-Type` 最小真实推理矩阵；
- [ ] T2 不重复 T1 路由组合、T1.5 wire/error 或 metadata variant 覆盖；
- [ ] 执行 T3 各启用消息入口、六类消息、多附件和代表性多轮场景；
- [ ] 处理所有 review、product defect、cleanup 和 baseline mismatch；
- [ ] 生成含调用次数、usage、实际/预计费用、清理结果和 targeted retest command 的发布验收报告。

## 7. 关键路径

总体关键路径：

```text
Gate 0
-> WP-01 API/IR
-> WP-05 Protocol Infra
-> WP-06 Base Codec
-> WP-07 Provider Inventory
-> WP-15 RuntimeSnapshot
-> WP-10 Routing + WP-11 Lowering
-> WP-12 Execution
-> WP-16 Service
-> WP-17 Callers
-> Cutover
-> T1/T1.5
-> T2/T3
```

可脱离关键路径提前完成：

- MatchRule 和 catalog compiler；
- Router 与 Scheduler，使用 fixture inventory；
- Resource security，使用 fake Provider；
- usage/query/storage，使用 synthetic completion；
- UI/Workflow/CLI，使用冻结 client contract 和 mock service；
- T1/T1.5 fixture、manifest、report 和 Mock Provider。

## 8. 多人协作规则

建议长期工作流：

1. API/公共类型；
2. MatchRule/Catalog/Model；
3. Protocol Infra；
4. Protocol Codec；
5. Provider/Inventory/Runtime；
6. Admission/Router/Call；
7. Execution/Resource/Storage/Security；
8. Service/UI/调用方/E2E。

协作约束：

- `aicc_client.rs` 和公共 schema 由 API owner 独占审核；
- `lib.rs`、`main.rs`、`Cargo.toml` 和最终切换由集成人负责；
- Provider 小组只修改自己的 builtin/dialect/native 目录和 fixture；
- Provider PR 不顺手修改 Router 或公共 IR；
- Router PR 不引入 Provider 名或模型名前缀特殊分支；
- 每个 PR 必须包含 contract test、fixture 或 case ID；
- 先合并 trait/DTO/fake，再并行填充实现；
- 一个工作包未满足完成标准时，不把状态标为 Done；
- 新增依赖、crate 或通用组件必须单独评审；
- 现有未提交修改属于工作区所有者，实施时不得覆盖或混入无关改动。

## 9. 测试与交付门禁

### 9.1 固定执行顺序

测试严格按以下顺序推进：

```text
编码阶段
  -> 每个模块同步完成并运行本模块单元测试
  -> 全部模块编码、旧实现删除、build 通过
  -> 冻结集成测试 commit
  -> T1 全量
  -> T1.5 全量
  -> T1/T1.5 Gate 通过
  -> T2 全量获准范围
  -> T3 全量获准范围
  -> 发布报告
```

不得跳过模块单测直接使用 E2E 验证实现，也不得在 T1/T1.5 未完成时开始 T2/T3。

### 9.2 编码阶段模块单元测试

每个工作包的编码提交必须同时包含相应单元测试。最低覆盖要求：

| 模块 | 必测内容 |
|---|---|
| API/Error | serde round-trip、unknown field、非法 schema、method dispatch、错误边界 |
| Matching/Catalog | exact/wildcard/operator、非法规则、revision/reference、确定性索引 |
| Model/Routing | exact/logical、mount、overlay、fallback、hard filter、score、trace |
| Protocol Infra | HTTP/SSE/multipart、timeout、backoff、cancel、redaction |
| Base Codec/Dialect | request/response/event/error golden、基础合同复用、差异点 |
| Provider/Inventory | discovery、能力交集、LKGS、refresh、Stop、迟到写 |
| Admission | quota、budget、privacy、trust、locked policy、fail closed |
| Call Lowering | operation、variant、参数优先级、set/remove、资源要求 |
| Execution | immediate/stream/task、failover、idempotency、取消竞态、usage completion |
| Resource | 权限、MIME、大小、压缩包安全、上传、artifact meta |
| Storage/Observability | 原子写、去重、查询、retention、trace 关联和脱敏 |
| Runtime/Settings | snapshot 原子发布、CAS、seq 收敛、并发触发和失败保留 |
| Service/Callers | handler/client mapping、RBAC、配置转换和错误传播 |

编码内循环只运行本轮修改对应的单元测试，不反复 build、部署或运行全量 T1/T1.5/T2/T3 代替聚焦测试。

目标基础命令：

```bash
cd src
cargo test -p aicc
cargo test -p buckyos-api --test aicc_client_test
uv run buckyos-build.py --skip-web
```

当前仓库尚无 `src/kernel/buckyos-api/tests/aicc_client_test.rs`；该测试目标由 WP-18 创建。在它落地前，相关命令是路线图目标而不是当前可执行门禁。

### 9.3 编码期间的 Push Gate

以下是受影响范围的提交前回归，不代表已经完成编码后的全量集成验收：

- 修改 routing、scheduler、fallback、admission 或 logical model 时，push 前运行受影响 T1 case；
- 修改 Provider protocol、model metadata、Provider metadata 或 Provider 配置时，push 前运行受影响 Provider 的 T1.5；
- 同时修改两类范围时，两项都运行；
- 仅文档或无关模块修改不因此获得 E2E 义务；
- push gate 失败必须修复，但全量 T1/T1.5 仍只在 Wave 7 执行。

### 9.4 集成验收分层

| 层级 | 被测链路 | 核心目标 | 真实调用 | 进入条件 |
|---|---|---|---|---|
| T1 | Zone Gateway -> 真实 AICC -> 多个通用 Mock Provider | 路由、调度、fallback、多 instance、task 和系统行为 | 否 | 编码冻结、全部模块单测和 build 通过 |
| T1.5 | Zone Gateway -> 真实 AICC typed/helper -> 真实 Adapter -> Provider 专用高保真 Mock | 官方 wire request、正常响应、stream、异步状态、错误和 variant lowering | 否 | T1 通过 |
| T2 | Zone Gateway -> 真实 AICC -> 真实 Provider | `ProviderInstance × model × API-Type` 最小真实推理正确性 | 是 | T1/T1.5 Gate 通过且当次获授权 |
| T3 | 消息入口 -> msg-center -> Jarvis -> AICC -> Provider -> 出站 | 六类消息、多附件、多轮和消息投递闭环 | 是 | T1/T1.5 Gate 通过且当次获授权 |

边界约束：

- T1 不使用 Provider 专属 wire 字段作为通过证据；
- T1.5 不重复 T1 路由排列组合，不验证真实推理内容；
- T1.5 的期望协议不能从 AICC 设计、实现、metadata、日志或旧 Mock 反推；
- T2 不重复 T1 的路由组合或 T1.5 的 wire/error/variant 矩阵；
- T3 使用少量代表模型验证消息和 Agent 闭环，不替代 T2 线上模型矩阵。

### 9.5 T1/T1.5 集成阶段

准备和自测试：

```bash
cd test/aicc_test
pnpm run acceptance:preflight
pnpm run acceptance:self-test
cp aicc_acceptance.example.toml aicc_acceptance.local.toml
```

T1：

```bash
pnpm run acceptance:t1 -- \
  --config aicc_acceptance.local.toml \
  --allow-config-mutation
```

T1.5 示例：

```bash
pnpm run acceptance:t1.5-mock-provider -- --port 18081
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- \
  --gateway-url https://test.buckyos.io \
  --mock-base-url http://aicc-reachable-host:18081 \
  --mock-control-url http://127.0.0.1:18081 \
  --provider openai \
  --allow-config-mutation
```

要求：

- T1/T1.5 必须经 Zone Gateway 和真实认证路径调用 AICC，不直连 service port；
- T1 使用 run-scoped 通用 Mock，并在 `finally` 中恢复完整 `services/aicc/settings`；
- T1.5 创建 run-scoped Provider instance，退出时调用 `provider.delete`；
- T1.5 Mock 与 wire fixture 只能依据 Provider 官方 API 文档、官方 schema、官方 SDK 协议定义和官方错误文档；
- 每个可独立调用的 metadata variant 在 T1.5 中作为独立协议单元；
- T1/T1.5 零真实 Provider 调用、零真实推理费用；
- 配置变更必须满足 runner 的显式双重 mutation guard；
- 全量 Gate 要求 P0 100% 通过，非 P0 的 failed/review 必须有明确处置；
- T1/T1.5 Gate 未通过时，T2/T3 保持 Blocked。

### 9.6 T2/T3 集成与发布阶段

T2/T3 执行前必须由用户对本轮范围明确授权。授权至少包含：

- Provider、instance、case 或场景；
- 最大真实调用数和费用预算；
- retry、timeout 和并发上限；
- 是否临时修改 Provider credential 或 AICC settings；
- 是否创建外部消息或 artifact；
- 清理范围。

真实调用必须：

- 经过 Zone Gateway 和真实认证；
- 不直接调用 AICC service port；
- 使用本地 ignored TOML 保存 secret；
- 对命令、日志和报告脱敏；
- 记录实际调用、费用、失败、cleanup 和 targeted retest command。

T2 基本矩阵固定为：

```text
ProviderInstance × active physical model × supported canonical API-Type
```

每个单元默认只执行一次最小正确性请求。逻辑 alias 不重复调用；metadata variant 留在 T1.5；错误注入、参数排列和 wire 分支也留在 T1.5。

T3 按启用消息入口覆盖文本、图片、视频、音频、文档、压缩包的入站和出站，以及多附件、多轮和代表性 Agent 任务。平台不支持的消息类型必须记录 `platform_limitation` 和规定的降级行为。

### 9.7 自动化收敛循环

T1/T1.5/T2/T3 自动化失败按批次处理：

1. 先运行选定范围，收集一批失败和证据；
2. 分类后集中修复实现；
3. 对该批次只构建、部署一次；
4. 重跑受影响批次；
5. 如果本轮改过实现，重新运行相应模块单测，并再次完整运行所选范围；
6. 直到一次完整重跑通过，且它与前一次运行之间没有实现修改。

不采用“一条失败、一次修改、一次构建部署”作为默认循环。确认属于短暂网络、认证或余额等实现外问题时可以停止，但报告必须保留全部 case、Provider、证据和无需继续修改的理由。

### 9.8 发布完成标准

- T1 能确定性证明所有路由分支选择正确模型/instance 或返回正确错误；
- T1.5 能依据 Provider 官方资料独立证明 AICC wire request、正常响应解析和错误映射正确；
- T2 官方 inventory、AICC inventory 和 Adapter 声明双向一致，并完成获准矩阵的最小真实推理；
- 11 家 Provider 和 SN 都有版本化能力基线；
- T3 覆盖启用入口、六类消息、多附件和代表性多轮场景；
- task、resource、usage、quota、安全、配置、metadata 和 observability 均有 case；
- 不匹配的输出种类必然失败；
- 默认测试不产生真实费用；
- 临时 settings、Provider、Named Object、消息和 artifact 全部清理；
- 所有 `review`、baseline mismatch 和阻断 defect 已处理。

## 10. 旧实现删除清单

最终切换时应逐项确认，而不是简单删除目录：

- [ ] 旧 `AIComputeCenter` 全局协调器和绕过边界的 helper；
- [ ] 旧 Provider 单体：OpenAI、Gemini、Claude、MiniMax、fal、SN；
- [ ] 旧 `openai_protocol` / `claude_protocol` 中混合 Provider/model 特例；
- [ ] 旧 ModelRegistry、Router、Scheduler、Session 实现；
- [ ] 旧 metadata resolver/updater 和 AICC 自建下载/activation；
- [ ] 旧 complete request queue 和不符合 TaskMgr 语义的生命周期；
- [ ] 旧 Provider section settings 解析；
- [ ] settings 中的 `provider_driver`、`api_key/apiKey` 等旧兼容字段；保留新 settings 的 `base_url` 及公共 RPC/报告中的 `provider_driver`；
- [ ] `reload_settings`、`reaload_settings`、`service.reaload_settings` 等旧名称、错误拼写和重复入口；`buckyos-api::aicc_client` 同步更新为只调用 `service.reload_settings`；
- [ ] `llm.chat`、`image.txt2image`、`image.img2image` 等旧 method；
- [ ] Desktop、Workflow、Jarvis、CLI 的旧 DTO 和 mapping；
- [ ] dev/rootfs 中旧 AICC 配置；
- [ ] 只验证旧实现行为、与目标规范冲突的测试。

删除后使用 `rg` 扫描旧 module、method、settings key/field 和 Provider 身份残留，并把扫描命令及结果放入 cutover PR。

## 11. 风险登记

| 风险 | 影响 | 应对 |
|---|---|---|
| Gate 0 契约继续变化 | 多条并行线返工 | 公共 contract 变更必须走单独 RFC，并同步 Rust/TS/E2E |
| 11 家 Provider 同时开发导致基础 codec 分叉 | 重复代码和行为不一致 | 基础 codec owner 先交付 contract，派生 Provider 只提交差异 |
| 媒体 operation 工作量过大 | 阻塞全部 Provider 上线 | 先完成 11 家主接口，再分批交付媒体 operation |
| quota/budget owner 不清 | P0 发布后期被阻塞 | Gate 0 明确 routing 内部策略层 owner、事实源和最小 enforcement，不新增独立顶层模块 |
| settings 写入与 runtime reload 非原子 | 半生效和不可恢复状态 | 候选 snapshot 完整构建后发布，失败保留旧 runtime 并返回诊断 |
| refresh/stop 竞态 | 迟到 inventory 覆盖新实例 | Stop + join + generation token + 并发测试 |
| metadata 全局收敛耗时 | 推理入口长时间等待 | 单执行者合并、明确 timeout、旧 LKGS 和失败 Provider 诊断 |
| Provider 官方 API 持续变化 | baseline 漂移和发布失败 | 官方机器接口、版本化 evidence、T2 双向 diff |
| 旧调用方遗漏 | build 后运行期协议错误 | `rg` 残留扫描 + 调用方单测 + T1/T1.5 + rootfs smoke |
| 真实验收费用和 secret 泄露 | 成本或安全事故 | 显式授权、预算预留、ignored TOML、redaction 和 cleanup |

## 12. Tracker

| 工作包 | Owner | 状态 | 前置 | 主要出口 |
|---|---|---|---|---|
| Gate 0 | Architecture/API/Metadata/E2E owners | Done | 无 | 契约冻结 |
| WP-00 | 集成人（`@streetycat`） | Review | Gate 0 | 模块骨架 |
| WP-01 | API 小组 | Done | Gate 0 | API/IR/Error |
| WP-02 | Catalog/Matching 小组 | Done | Gate 0 | MatchRule（`a12b3e09`） |
| WP-03 | Metadata 小组 | Done | WP-02 | CatalogSnapshot |
| WP-04 | TBD | Pending | WP-01/02/03 | Model Registry |
| WP-05 | TBD | Pending | WP-01 | Protocol Infra |
| WP-06 | TBD | Pending | WP-01/05 | Base Codec |
| WP-07 | TBD | Pending | WP-03/05 | Provider Core |
| WP-08 | TBD | Pending | WP-06/07 | 11+1 Providers |
| WP-09 | TBD | Pending | WP-01/14 | Admission |
| WP-10 | TBD | Pending | WP-04/09 | Routing |
| WP-11 | TBD | Pending | WP-03/06/10 | Call Lowering |
| WP-12 | TBD | Pending | WP-05/11 | Execution |
| WP-13 | TBD | Pending | WP-01 | Resource |
| WP-14 | Storage/Observability 小组 | Done | WP-01 | Storage/Observability |
| WP-15 | TBD | Pending | WP-03/07/14 | RuntimeSnapshot |
| WP-16 | TBD | Pending | WP-07/09-15 | Service/Admin |
| WP-17 | TBD | Pending | WP-01/16 | Callers |
| WP-18 | TBD | Pending | Gate 0 | Acceptance |
| T1/T1.5 Gate | TBD | Pending | WP-01 至 WP-18 Done、编码冻结 | 零真实调用的集成验收 |
| T2/T3 Gate | TBD | Pending | T1/T1.5 Gate Done、当次授权 | 真实 Provider 与消息链路发布验收 |

状态只允许：`Pending`、`In Progress`、`Blocked`、`Review`、`Done`。每次更新状态时应同时填写 owner、关联 PR/commit、剩余风险和目标验收 case。

## 13. Definition of Done

AICC 重新实现只有同时满足以下条件才算完成：

- 新模块依赖方向符合本文约束；
- 当前旧内部实现和临时兼容入口已经删除，不只是停止调用；`buckyos-api` 及全部调用方只使用 `service.reload_settings`，其它已导出 RPC 契约保持稳定；
- 没有新增未经确认的 crate 或第三方依赖；
- 公共 API、Rust client、TS canonical 和 E2E manifest 一致；
- 11 家 Provider 和 SN 进入基线并完成对应合同；
- RuntimeSnapshot、inventory LKGS、metadata seq 和 Stop 生命周期通过并发测试；
- Desktop、Workflow、OpenDAN/Jarvis、CLI、rootfs 和 RBAC 已联动；
- WP-01 至 WP-17 均具备覆盖正常、边界、错误和关键并发语义的模块单元测试；
- 全部模块单元测试和 AICC build 通过后才冻结集成测试 commit；
- 编码完成后先完成 T1 和 T1.5 全量验收，且零真实 Provider 调用；
- T1/T1.5 Gate 通过后，经明确授权完成 T2/T3 发布验收；
- 文档、协议、共享类型、数据结构和配置示例已经同步；
- 发布报告没有未解释的阻断失败、review、能力缺口或 cleanup 残留。

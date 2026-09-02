# AICC E2E 测试需求

## 1. 文档目的

本文定义 AICC 端到端测试的完整需求，目标是从模型路由、Provider 官方协议实现、线上模型推理和 Jarvis 消息链路四个层面，证明 AICC 在模型服务稳定、系统配置正确的前提下能够：

1. 按请求约束选择正确的精确模型和 Provider instance。
2. 按 Provider 官方协议发送请求并解析正常响应与错误响应。
3. 通过各 Provider instance 的线上模型完成其支持的 API type 推理。
4. 通过 message-tunnel、MessageHub、msg-center 和 Jarvis 完成真实多模态、多轮消息任务。

本文是测试需求文档，不规定唯一的 runner 实现方式。具体 case manifest、Mock Provider、fixture、报告器和执行脚本可以复用 `doc/aicc/maintenance/acceptance` 与 `test/jarvis_media_dv` 中已有设计和实现。

## 2. 适用范围

### 2.1 被测范围

本文覆盖：

- AICC route control plane、模型目录、metadata、调度和 fallback。
- AICC typed inference、Helper 调用和 Protocol Adapter。
- Provider inventory、Provider instance、模型和能力声明。
- Provider 同步、异步、streaming、资源和错误协议。
- AICC task、usage、quota、配置、安全和可观测性。
- msg-center、MessageHub、message-tunnel、Jarvis/OpenDAN、AICC 和 Provider 组成的完整链路。
- 文本、图片、视频、音频、文档、压缩包及多附件消息。

### 2.2 不属于本方案的目标

- 不以真实模型自然语言全文一致作为通过条件。
- 不对真实模型做压力测试或价格对比。
- 不要求普通 CI 默认调用真实模型。
- 不把 Jarvis 组合工具生成的能力登记成 Provider 模型原生能力。
- 不允许测试绕过 Gateway、认证和真实服务进程后仍称为 E2E/DV 通过。

编码过程中必须同步完成各模块单元测试和模块级协议测试；它们是模块编码完成条件，也可以用于缺陷定位，但不能替代本文要求的真实链路测试。全部模块编码和单元测试完成后才进入集成测试：第一阶段完成 T1/T1.5，第二阶段完成 T2/T3。

## 3. 强制原则

### 3.1 真实链路原则

正式 E2E/DV 请求必须经过真实系统入口：

```text
Test Runner
  -> Zone Gateway
  -> 认证与权限检查
  -> 真实系统服务
  -> 被测下游
  -> Gateway
  -> Test Runner
```

T1 和 T1.5 允许使用 Mock Provider，但 AICC、Gateway、认证、Provider adapter 和任务链路必须是真实实现。T1.5 使用按 Provider 官方文档独立实现的高保真 Mock Provider。T2 必须调用真实 Provider。T3 必须通过选定的真实消息入口进入系统。

### 3.2 确定性与成本原则

- 路由分支由 T1 Mock Provider 覆盖；Provider wire 协议和协议错误分支由 T1.5 高保真 Mock Provider 覆盖，均不产生真实模型费用。
- 真实模型必须显式开启，默认禁止调用。
- 真实模型运行前必须输出计划 case 数、最大调用次数和预计成本，并展示可取消的确认倒计时；用户主动确认或倒计时结束后才可继续。无人值守的自动化测试模式不展示倒计时，但必须通过显式参数或受控配置预先授权。
- 真实模型调用必须同时受最大调用次数和总预算限制。
- 每个 case 使用最小但具有明确判定特征的输入。
- 真实模型的重试次数必须受限，所有 attempt 都要写入报告。
- 测试数据必须预先生成并保存，不依赖测试人员临时输入；无法自动化的外部平台操作除外。

### 3.3 输出类型强约束

每个 case 必须声明期望出站消息种类、附件数量范围和 MIME 类型。实际输出种类不匹配时直接判定失败，不能交给 LLM Judge 放宽。

### 3.4 可重复与可追踪原则

- 每次运行使用独立 `run_id`。
- case 必须可重复执行，不能依赖上次运行残留状态。
- 每个测试产生的配置、会话、任务、对象和临时环境都必须能定位并按范围清理。
- 报告必须能把消息、Jarvis 会话、AICC 路由、任务、Provider 调用和输出对象关联起来。

## 4. 术语与能力事实源

### 4.1 精确模型

精确模型名格式为：

```text
<provider_model_id>[:variant]@<provider_instance_name>
```

同一 Provider driver 可以配置多个 Provider instance。不同 instance 可以提供相同的 `provider_model_id`，但形成不同的精确模型。

### 4.2 逻辑模型

逻辑模型是 AICC 逻辑目录中的路径。逻辑模型请求经过候选展开、硬过滤、权重和调度策略后得到精确模型。

### 4.3 API type、method 与模型能力

- AICC canonical `api_type` 定义 AICC 接受的标准 API 能力分类。
- method 定义具体请求和响应 schema。
- 模型发布方文档定义某个具体模型实际上支持哪些能力。
- Provider adapter 负责把 AICC method 转换为发布方协议。

模型级能力必须满足严格一致：

```text
官方模型能力
<=> AICC 对该模型声明的 canonical api_type
<=> 当前 Provider adapter 对该模型的实际实现
```

发布方通常不使用 AICC 的命名，因此必须维护带证据的映射：

```text
官方 capability/endpoint
  -> AICC canonical api_type
  -> AICC method
  -> Provider adapter 实现入口
```

判定规则：

| 官方声明 | AICC 声明与实现 | 判定 |
|---|---|---|
| 支持 | 声明支持且测试通过 | 通过 |
| 支持 | 未声明、未实现或调用失败 | 缺陷 |
| 不支持 | 未声明且调用被明确拒绝 | 通过 |
| 不支持 | 声明支持或调用被当作受支持能力 | 缺陷 |
| 文档不明确 | 任意 | 阻塞该能力进入发布基线，不能自行推断 |

unknown model 不得通过模型名猜测获得高风险能力。兼容接口偶然接受某参数，也不能替代官方支持声明。

首版 11 家内置 Provider 必须进入参数化能力基线和测试清单：OpenAI、Claude、Google Gemini、Fal、OpenRouter、MiniMax、Kimi、GLM、DeepSeek、豆包和 Qwen；SN AI Provider 作为扩展 Provider 单独纳入。当前基线尚未覆盖的 Provider 在编码完成后的集成测试阶段补齐：先补齐 T1/T1.5 参数化基线和协议合同，再补齐 T2 真实 Provider 矩阵，最后进入 T3。该缺口不阻塞 Gate 0，但未补齐前不得通过对应发布门禁。Provider 的模型、生命周期、能力和协议限制以模型发布方的公开官方文档为事实源；AICC inventory 是被测声明，不能反向作为官方能力依据。

当前 AICC canonical API type 必须按各层职责逐项进入 T1、T1.5 和 T2 覆盖矩阵，不得用 namespace 或“其他 API”概括：

| namespace | canonical api_type | typed method |
|---|---|---|
| LLM | `llm` | `chat.completions.create` |
| Embedding | `embedding.text`、`embedding.multimodal` | 同名 typed method |
| Rerank | `rerank` | `rerank` |
| Image | `image.txt2img`、`image.img2img`、`image.inpaint`、`image.upscale`、`image.bg_remove` | `images.generate`、`image.img2img`、`image.inpaint`、`image.upscale`、`image.bg_remove` |
| Vision | `vision.ocr`、`vision.caption`、`vision.detect`、`vision.segment` | 同名 typed method |
| Audio | `audio.tts`、`audio.asr`、`audio.music`、`audio.enhance` | 同名 typed method |
| Video | `video.txt2video`、`video.img2video`、`video.video2video`、`video.extend`、`video.upscale` | 同名 typed method |
| Agent | `agent.computer_use` | `agent.computer_use` |

method 与 api_type 是不同值域，上表描述显式合法关联，不要求名称相同或 1:1。`route.resolve` 可以接受多个 api_type，同一个 api_type 也可以出现在 route、typed inference 和 Helper 调用形态中。Runner 必须分别从当前协议/schema 枚举 canonical method 和 api_type，并与本清单和 case manifest 校验各自值域及关联；不得检查字符串相等或双射。新增、删除、改名或关联变化如果没有同步更新需求清单、官方能力映射和测试用例，preflight 必须失败。

### 4.4 Provider 官方事实源

T1.5 的 wire 协议契约只能来自 Provider 官方 API 文档、官方 OpenAPI/JSON Schema、官方 SDK 的公开协议定义和官方错误码文档。不得以 AICC 设计文档、AICC metadata、AICC 实现代码、现有请求日志或现有 Mock 行为反向定义期望协议。每个协议 fixture 必须记录官方证据 URL、检查时间、适用 API/version 和证据摘要；官方资料不明确时，该协议点标记为阻塞，不能依据当前实现猜测。

T2 为每个获准测试的 Provider instance 枚举全部 active、已配置且官方声明支持的物理模型，并按每个模型支持的 API type 生成 `ProviderInstance × model × API-Type` 矩阵。模型事实依据来自 Provider 官方模型目录或官方模型文档；AICC `models.list` 仅作为被测声明参与双向一致性检查，不能成为官方能力事实源。报告必须记录模型、官方证据和检查时间。

官方目录可用时，Runner 必须在 preflight 中获取本轮完整官方库存，过滤已移除模型和仅指向其他物理模型的逻辑别名，并与刷新后的 AICC `models.list` 双向对比。双方一致的 active 基础物理模型全部进入 T2；metadata variant 在 T1.5 覆盖，不增加 T2 真实推理单元。任一方向的库存缺失都作为 baseline mismatch 报告，不能通过缩小矩阵隐藏。目录认证默认复用 Provider API token，也允许用独立目录凭据覆盖；凭据不得写入报告。官方目录不可用但存在明确、当前有效的冻结官方模型清单时，可以使用该证据，报告必须标明证据时间和未实时确认的风险。

Fal 这类异构端点市场必须把 endpoint 视为协议与能力边界。每个进入 T1.5/T2 的 endpoint 都必须有独立官方 Schema 或官方文档证据；不得把市场中的其他端点视为同一 adapter 的协议兼容模型。

## 5. 总体分层

| 层级 | 被测主链路 | 模型 | 核心目标 |
|---|---|---|---|
| T1 路由正确性 | Gateway -> AICC -> 多个 Mock Provider | 全部 Mock | 请求选择正确模型，或返回正确错误 |
| T1.5 Provider 协议契约 | Gateway -> AICC -> Provider 专用高保真 Mock | 全部 Mock | AICC 发出的 wire 请求及响应/错误解析符合 Provider 官方协议 |
| T2 线上推理正确性 | Gateway -> AICC -> 真实 Provider | 全部 active 物理模型 | 覆盖 `ProviderInstance × model × API-Type` 并验证真实推理结果 |
| T3 Jarvis 消息链路 | 消息入口 -> msg-center -> Jarvis -> AICC -> Provider -> 出站 | 少量代表模型 | 多模态、多附件、多轮任务完整进出系统 |

四层分别建立覆盖矩阵和通过率。T1 不替代 T1.5 的协议断言，T1.5 不验证真实推理，T2 不重复 T1 的路由排列组合或 T1.5 的 wire 协议与错误注入，T3 的少量消息场景也不能替代 T2 的线上矩阵。

分层边界固定如下：

- 同一 Provider driver 的多 instance 选择、不同模型集合及隔离行为只在 T1 覆盖。T1.5 按 Provider driver、adapter/API version 和 API type 覆盖协议差异。T2 对配置中启用且获准真实调用的每个 Provider instance，覆盖其全部 active 模型支持的每个 API type。T3 可参数化凭据和 instance 标识用于配置与审计，但不要求强制 Jarvis 最终路由到该 instance。
- metadata `variants` 的展开和路由在 T1 覆盖。每个可独立调用的 variant 都在 T1.5 作为独立协议单元验证其 Provider model/options lowering 和响应解析；variant 不作为独立 model 进入 T2。
- `version_rules` 产生的逻辑目录只在 T1 覆盖；除非它改变 wire 协议，否则不进入 T1.5，也不成为 T2/T3 的覆盖维度。
- 跨租户、RBAC 和授权隔离等 BuckyOS 机制只在 T1 覆盖；T1.5/T2/T3 不重复验证。

## 6. 公共测试基础

### 6.1 Case manifest

每个 case 至少声明公共字段，并按所在层级声明适用的路由、协议或内容断言：

- `case_id`、层级、优先级和标签。
- 输入入口、用户、session 和前置状态。
- provider driver、provider instance、模型选择方式。
- api_type、method、required/disabled capabilities。
- 输入消息、附件和 fixture。
- Mock scenario 或真实模型前置条件。
- 期望精确模型、Provider instance、task 状态和错误分类。
- 期望出站消息种类、附件数量、MIME 和语义 rubric。
- timeout、最大 attempt、预计成本和清理要求。

T1.5 case 还必须声明官方协议证据 revision、adapter/API version、预期 wire fixture 和正常/错误响应 fixture；无需声明内容语义 rubric。T2 case 必须声明 `ProviderInstance × model × API-Type` 单元和最小正确性 rubric；无需声明路由组合或 wire fixture。

Manifest 解析失败、case id 重复或必要字段缺失时，runner 必须在执行前失败，不能静默跳过。

### 6.2 Fixture

仓库应维护预生成 fixture，至少包括：

- 具有唯一事实标识的文本。
- PNG、JPEG、透明图、mask、OCR 图片和多图组合。
- 语音、非语音音效、不同编码和采样率音频。
- 短视频、带音轨视频、无音轨视频和字幕文件。
- TXT、Markdown、PDF、DOCX、XLSX、PPTX、HTML、CSV。
- 单文档、多文档、文档与媒体混合的 ZIP。
- 损坏、空、超限和安全边界 fixture。

每个 fixture 必须有固定 ID、路径、MIME、大小、SHA-256、关键事实、适用 case 和生成来源。大文件应优先由确定性脚本生成。

### 6.3 统一结果状态

统一使用：

- `passed`：全部强制断言通过。
- `failed`：协议、路由、结构、语义或清理要求失败。
- `provider_restricted`：请求已到达 Provider，但因账号等级、区域、模型白名单、preview allowlist 或 Provider 服务策略而被拒绝；必须保留脱敏证据，且不能计为通过。
- `skipped`：环境或凭据缺失；发布强覆盖模式下是否允许由门禁决定。
- `not_applicable`：官方明确不支持该组合。
- `review`：结构通过，但仍需人工确认非确定性内容。

`review` 不能在发布强覆盖中自动等价于 `passed`。

## 7. T1：路由正确性测试

### 7.1 目标

在不调用真实模型的情况下，覆盖真实 AICC 路由、metadata、调度、fallback 和路由相关错误处理。通过后，在模型服务稳定且配置正确的前提下，AICC 应按请求需要找到正确的精确模型和 Provider instance。Provider wire 协议的逐字段正确性由 T1.5 负责。

### 7.2 测试架构

```text
T1 Runner
  -> Zone Gateway
  -> 真实 AICC
  -> 真实 route/metadata/scheduler/task/adapter
  -> 多个协议级 Mock Provider
```

Mock Provider 必须位于 Provider HTTP/远端协议边界，不能只替换 AICC 内部 trait。T1 Mock 只需稳定支持路由断言、调用记录和错误触发，不要求完整复刻各 Provider 官方协议；高保真协议模拟属于 T1.5。

### 7.3 模型选择维度

必须覆盖：

- 精确模型。
- 逻辑目录路径。
- metadata `variants` 展开的模型。
- `version_rules`、exact rule、pattern rule、default rule 派生模型。
- 合法但不存在的模型。
- 非法精确模型名、非法逻辑路径和不存在的 Provider instance。
- 已禁用、已下线、未挂载和 metadata 损坏的模型。

`route.resolve` 只用于逻辑模型解析。精确模型应通过要求 exact model 的数据面接口或现有 exact 调用路径测试，主要验证解析、api_type、capability、instance、adapter 和默认不 fallback。不得把 `route.resolve(exact_model)` 当作正式需求。

### 7.4 路由约束维度

核心排列组合至少包含：

1. api_type 和 method。
2. required capabilities 和 disabled capabilities。
3. 任务特征：纯文本、代码、文档 chunk、图片、音频、视频、文本 embedding、多模态 embedding、rerank、JSON schema、tool call、web search、streaming、最小上下文长度和最大输出长度。
4. Provider driver/instance allow 和 deny。
5. `local_only`、隐私和数据边界要求。
6. health、quota、budget 和余额状态。
7. context/output token 上限。
8. 模型和逻辑目录权重。
9. 当前调度 profile：`cost_first`、`latency_first`、`quality_first`、`balanced`、`local_first`、`strict_local`；新增 profile 必须自动进入枚举完整性检查和路由用例。
10. `min_line`、`disable_line`、auto/manual mount。
11. system routing config 和 request/session overlay。
12. locked policy 及调用方不可覆盖项。
13. metadata、pricing、latency 或 quality 数据缺失时的保守行为。

任务内容既要覆盖调用方显式传入 capability 的路径，也要覆盖 helper/Jarvis 根据任务内容构造路由要求的路径。

### 7.5 多 Provider instance

多 Provider instance 只在 T1 的 Mock Provider 场景验证。同一 Provider driver 至少构造：

- 不同凭据的两个 instance。
- 不同 region 或 base URL 的两个 instance。
- 提供相同模型的两个 instance。
- 提供不同模型集合的两个 instance。
- 不同权重、价格、延迟、quota 和 health 的 instance。

必须验证：

- exact model 准确命中指定 instance。
- 逻辑模型在多个同名模型 instance 中按规则选择。
- instance 配置、凭据、inventory、health 和 quota 相互隔离。
- 一个 instance reload、delete 或失败不影响其他 instance。
- usage、cost、trace 和错误归因到实际 instance。

### 7.6 历史路由软优先

历史路由偏好的粒度固定为同一 session 内过去实际选中的精确模型（包含 Provider instance），不得退化为仅按 Provider instance 或模型家族粘滞。后续请求先应用全部硬约束，再在仍合格的候选中软优先选择该 session 过去使用过的精确模型。例如首轮选择 GPT-4，第二轮 GPT-4 与 GPT-5 均合格时应优先 GPT-4。

历史路由只能作为软偏好，不能越过模型存在性、api_type、required capability、disabled capability、privacy、`local_only`、Provider allow/deny、health、quota、budget、context length、output length 和 locked policy 硬约束。

至少覆盖：

- 上一轮 exact model 和 instance 仍适用时优先复用。
- 上一轮模型不支持新 api_type 或 capability 时重新选择。
- 上一轮 instance 不健康、无 quota 或被 policy 拒绝时重新选择。
- 可选择同一 instance 的另一精确模型、同一 Provider driver 的另一 instance，或另一 Provider driver 的合格模型。
- 历史偏好缺失时正常调度。
- 不同 session 的历史偏好互不污染。

测试必须使用真实 session 标识和上一轮实际路由结果建立历史，不得用 request/session overlay 伪造等价的历史路由粘滞。

### 7.7 Fallback 与运行时错误

必须覆盖：

- strict/no fallback。
- parent fallback。
- target logical 和 target exact fallback。
- 精确模型默认不 fallback。
- fallback 不跨 api_type。
- `embedding.text` 和 `embedding.multimodal` 默认 strict；指定 `embedding_space_id` 或已有向量索引时，禁止 fallback 到不同 embedding space、维度、距离度量或归一化约定的模型。
- `rerank` 默认 strict；fallback 重跑不能把不同 reranker 的分数混排。
- fallback loop 和最大深度。
- 首选 Provider 429、5xx、连接失败、timeout 后的切换。
- 400 参数/schema 错误、401 认证错误、403 权限或内容策略拒绝、404 模型不存在、409 幂等冲突及明确标记为不可重试的 Provider 错误必须停止。
- fallback 后 task、usage、trace 和 Provider 归因。
- 多 instance 间 failover。

### 7.8 Mock 行为

Mock Provider 至少支持：

- 同步成功。
- streaming 成功及 chunk 聚合。
- 异步 submit/poll 成功。
- 400、401、403、404、409、429 和 5xx。
- 连接失败、短超时、长超时。
- 可供路由和 task 断言使用的确定性文本、embedding、rerank 和 artifact 结果。
- malformed response、错误 MIME 和缺 usage 等标准化故障触发器；Provider 专属 wire 结构由 T1.5 覆盖。
- Provider task failed/cancelled。
- 同一 scenario 固定 usage、cost、latency 和输出对象。

### 7.9 组合覆盖

- 所有路由硬过滤、fallback 和错误处理分支必须穷举。
- `model selector × api_type × capabilities × task feature × session preference` 使用 covering array 或 pairwise 生成组合。
- 冲突组合、边界值、fallback 链和多 instance 选择单独穷举。
- 报告必须给出需求分支覆盖率和组合覆盖率，不能只统计 case 数量。

### 7.10 T1 断言与通过标准

每个 case 至少断言：

- 选中的 exact model 和 Provider instance。
- provider model ID 和 variant 展开结果；具体 Provider options 的 wire lowering 由 T1.5 断言。
- 候选集合及每个候选保留/过滤原因。
- route trace、fallback attempts 和最终错误分类。
- 只有预期 Mock Provider 收到预期次数的请求。
- task、usage 和错误归因正确。

T1 P0 必须零真实调用、零随机重试、100% 通过，并作为普通 CI 阻断项。

## 8. T1.5：Provider 官方协议契约测试

### 8.1 目标

使用 Provider 专用高保真 Mock Provider，验证 AICC Protocol Adapter 发出的 wire 请求与 Provider 官方要求相同，并验证 AICC 对官方正常响应和各种错误响应的解析与映射。T1.5 不覆盖 T1 的路由排列组合，不调用真实模型，也不判断推理内容是否正确。

### 8.2 独立协议事实源

每个 Provider 的 T1.5 Mock 和断言必须直接依据 4.4 节规定的 Provider 官方资料实现。协议期望数据应作为独立、可审查的 fixture/schema 保存，且必须满足：

- 不导入 AICC Provider adapter 的请求/响应类型来生成期望值。
- 不复制 AICC metadata、设计文档或实现中的 endpoint、字段默认值和错误映射作为期望值。
- 不用 AICC 当前发出的请求录制结果作为 golden fixture。
- 官方协议版本变化时，先更新官方证据和独立 fixture，再用测试发现 AICC 实现差异。
- Mock 未实现官方协议中的相关字段或错误形态时，该矩阵单元不得记为通过。

### 8.3 测试架构

```text
T1.5 Runner
  -> Zone Gateway
  -> 真实 AICC typed/helper method
  -> 真实 Protocol Adapter
  -> Provider 专用高保真 Mock Provider
  -> 独立 wire capture 与官方协议断言
```

测试必须用精确模型或其他确定性方式固定 Provider instance，不经过逻辑路由选择。仅为使请求到达目标 adapter 所需的最小配置不算 T1 路由覆盖。

### 8.4 覆盖矩阵

T1.5 的基本覆盖单元为：

```text
Provider driver
  x Adapter / 官方 API version
  x canonical API-Type
  x 对应 Provider operation/endpoint
```

每个可独立调用的 variant 都增加 T1.5 矩阵单元。除此之外，只有会改变 wire 协议的模型族、区域 endpoint、同步/异步模式或内容传输方式才增加矩阵维度。相同协议的不同 Provider instance、逻辑模型、路由策略和调度组合不重复测试。

### 8.5 正常请求与响应

每个适用矩阵单元至少逐字段验证：

- HTTP method、URL path、query、API version 和 content type。
- 认证 header/query、必需 header、幂等键和签名输入；报告必须脱敏。
- model/endpoint ID、请求 body、默认参数、可选参数、省略规则和枚举序列化。
- 文本、结构化内容、tool/schema、URL、base64、multipart、文件上传及官方支持的资源引用。
- streaming event、异步 submit/poll/cancel、operation ID 和终态协议。
- 官方正常响应到 AICC typed response、task、usage、finish reason、tool call 和 artifact 的映射。

Mock Provider 必须先按官方 schema 校验请求，再返回官方格式响应。宽松接受未知字段、错误字段名、错误路径或错误 content type 的 Mock 不能作为通过证据。

### 8.6 错误覆盖

每个 Provider adapter 必须覆盖官方定义且适用的错误形态，包括：

- 缺失、无效和权限不足的认证。
- 请求 schema/参数错误、模型或 endpoint 不存在、输入超限和不支持的媒体类型。
- rate limit、quota/balance、并发限制、服务不可用和内部错误。
- safety/content policy、region、账号等级和 allowlist 限制。
- streaming 中断、异步 operation failed/cancelled/expired、poll timeout。
- malformed/缺字段响应、错误 content type、不可下载 artifact 和无效 usage。

必须断言 AICC 的 retryability、HTTP/kRPC 错误、稳定错误分类、Provider 原始错误摘要和 task 终态。错误覆盖按 Provider 协议分支完成，不按模型重复。

### 8.7 T1.5 通过标准

- 每个启用 Provider driver 的全部已实现 adapter/API version 和 API type 都有明确结果。
- 每个可独立调用的 metadata variant 都有独立协议结果。
- 所有请求在独立高保真 Mock 中通过官方 schema 和逐字段断言。
- 正常响应、streaming、异步任务及官方错误分支均正确映射。
- 测试证据能追溯到 Provider 官方资料，且未以 AICC 文档或实现生成期望值。
- 零真实 Provider 调用、零真实推理费用；无需验证生成内容语义。

## 9. T2：线上 Provider 推理正确性测试

### 9.1 目标

通过最少的单元内真实调用，证明每个获准测试的 Provider instance 中，每个 active 模型都能对其声明支持的每个 canonical API type 产生正确推理结果。T2 依赖 T1 保证路由、依赖 T1.5 保证 Provider wire 协议，不重复这些层的组合、错误注入和逐字段协议检查。

### 9.2 线上能力基线

T2 baseline 的一条记录对应一个 `ProviderInstance × model × API-Type` 矩阵单元，至少包含：

- release、baseline revision、provider driver 和 provider instance。
- provider model ID、exact model、canonical API type 和 method。
- 模型 active 状态、官方能力证据 URL、检查时间和证据摘要。
- 最小输入 fixture、确定性断言或版本化语义 rubric。
- 最大调用次数、最大重试次数、timeout 和预计成本。
- 对应 case id 和覆盖状态。

每个 active 基础物理模型支持的每个 API type 都必须形成矩阵单元。每个单元默认只执行一个最小正常请求；只有单次调用无法验证该 API type 的核心推理结果时，才能增加最少的补充 case，并必须在 baseline 中说明原因。指向同一物理模型的逻辑别名不重复调用；官方列为独立物理模型的快照作为不同 model 执行；metadata variant 只在 T1.5 执行。输入格式排列、协议参数和错误场景不构成 T2 的独立覆盖维度。

### 9.3 覆盖矩阵

基本覆盖单元为：

```text
Provider instance
  x active physical model
  x 该模型官方支持且 AICC 声明支持的 canonical API-Type
```

Provider driver 是 instance 的归属与报告分组字段，不增加覆盖维度。每个单元使用 baseline 指定的 exact model 发起调用，确保命中目标 instance 和 model。同一个 workflow 可以覆盖多个 API type，但报告必须能反向证明每个矩阵单元都发生了真实调用并完成结果断言。

配置中启用且获得本轮授权的 Provider instance 及其全部 active 物理模型都必须进入矩阵。因 region、账号等级、余额或临时服务状态无法执行时，保留矩阵记录和脱敏证据，不得用同 driver 的其他 instance 或同 instance 的其他模型代替。

### 9.4 最小推理用例

每个 API type 使用能低成本、明确判断推理正确性的最小输入：

- 文本。
- 图片。
- 视频。
- 音频。
- 文档或结构化输入仅在对应 API type 的核心正确性无法由更小输入验证时使用。
- embedding 使用最小 item 数验证维度、有限数值、顺序和 embedding space。
- rerank 使用最小可区分候选集验证排序、document ID、index 和 score。

T2 不覆盖同一 API type 的格式全集、多输入规模边界、参数排列、streaming/非 streaming 双路径、错误响应或资源传输形态；这些属于 T1.5。压缩包和多轮组合任务属于 T3。

各 canonical API type 的推理正确性目标如下：

`chat.completions.create` 是 AICC 的 provider-neutral LLM typed inference 接口，也是 LLM typed inference 的唯一 kRPC method。底层 wire API 的正确性由 T1.5 验证。

- `chat.completions.create`：最短确定性文本任务，验证回答包含指定事实。
- `embedding.text`：最小文本输入到有效向量。
- `embedding.multimodal`：最小文本与图片输入到同一 embedding space 的有效向量。
- `rerank`：query 与内联/resource documents 到有序 document ID、index 和 score。
- `image.txt2img`：文本到图片。
- `image.img2img`：单图加最短文本到图片。
- `image.inpaint`：原图、mask 和文本到图片。
- `image.upscale`：图片到高分辨率图片。
- `image.bg_remove`：图片到透明背景图片或前景 mask，按 method schema 判定。
- `vision.ocr`：图片或文档页到文本及适用时的坐标。
- `vision.caption`：图片到描述文本。
- `vision.detect`：图片到类别、置信度和 bounding box。
- `vision.segment`：图片到 mask、polygon 或 segmentation artifact。
- `audio.tts`：文本到语音音频。
- `audio.asr`：语音音频到文本、时间戳和 speaker 信息，后两项按官方能力判定。
- `audio.music`：文本或参考音频到音乐音频。
- `audio.enhance`：音频到降噪、分离、修复或增强后的音频，具体 operation 按官方能力判定。
- `video.txt2video`：文本到视频。
- `video.img2video`：图片加文本到视频。
- `video.video2video`：视频加文本或控制参数到视频。
- `video.extend`：可续作视频及其 Provider/source operation 状态到延长视频。
- `video.upscale`：视频到高分辨率视频。
- `agent.computer_use`：最小受控环境中的单个可验证动作；只有正式启用时才执行。
- 新增 canonical API 必须定义一个最小正确性用例后再进入 T2 baseline。

真实推理不得只以“请求成功”判定。文本使用固定事实断言；embedding/rerank 使用数值与排序不变量；图片、音频和视频优先使用低分辨率、短时长 fixture，并依次使用结构检查、确定性内容检查和必要的版本化 Judge。除非结果处于 rubric 边界，不重复调用同一单元。

### 9.5 Provider instance 执行断言

每个 `ProviderInstance × model × API-Type` 单元至少验证：

- 请求命中 baseline 指定的 Provider instance 和 exact model。
- Provider 返回真实 operation/request ID，或有其他可审计证据证明发生了线上调用。
- task 达到正确终态，输出 schema、消息类型、artifact MIME 和可读性符合该 API type。
- 推理结果通过 9.4 节规定的确定性断言或语义 rubric。
- usage、cost、trace 和 Provider operation ID 归因到目标 instance、模型和 API type。

T2 不主动制造无效凭据、rate limit、quota exhausted、unsupported parameter、malformed response、timeout 或 cancel 等错误；这些由 T1.5 的官方协议错误覆盖保障。真实环境偶然返回此类错误时，记录为环境/Provider 失败证据，不把它扩展成新的 T2 测试矩阵。

### 9.6 内容正确性判定

按以下顺序判定：

1. 结构判定：状态、schema、消息类型、附件数、MIME、文件头和可读性。
2. 固定事实判定：OCR 编码、音频口令、文档事实、对象清单和结构化字段。
3. LLM Judge：生成内容忠实度、图片/音频/视频语义和质量。
4. 人工复核：Judge 边界分数、复杂媒体或发布强覆盖要求的抽查。

LLM Judge 必须使用版本化 rubric、记录 Judge 模型、Provider、输入摘要、分数和理由。应优先使用不同 Provider 或模型家族，避免被测模型评价自己。Judge 不可用或输出无效时不得自动通过。

### 9.7 T2 通过标准

- 获准执行范围内每个 `ProviderInstance × model × API-Type` 单元都有明确执行结果。
- 每个通过单元都完成了一次可审计的真实推理，并通过结构和内容正确性断言。
- task、usage、artifact、trace 和 Provider operation 归因正确。
- T2 对每个 active 模型执行其支持的 API type，同时不重复 T1 路由组合、T1.5 wire 协议/错误矩阵或单元内的输入与参数排列。
- 发布强覆盖中不允许因缺少必要 key 静默通过；必须 preflight 失败或有明确批准的发布例外。

## 10. T3：message-tunnel/Jarvis 链路测试

### 10.1 目标

验证消息从真实入口进入 msg-center，经 Jarvis/OpenDAN 调用 AICC 和 Provider，再由 msg-center 通过正确出口返回的完整链路。T3 侧重消息与 Agent 任务闭环，不要求覆盖全部模型和路由路径。

### 10.2 入口

runner 根据参数选择：

- Telegram，以及实现并启用时的 Email、Slack、Webhook、移动端/App 内通道、第三方 Agent 平台和 BuckyOS 应用间 message-tunnel；每新增一种 tunnel 都必须自动进入入口覆盖清单。
- MessageHub 公共入口。
- 通过 Gateway 直接调用 msg-center 入站接口。

消息入口、Gateway、登录凭据和期望 Provider 列表必须参数化，不得硬编码到测试脚本。T3 的 Provider instance 参数用于凭据注入和审计，不构成对 Jarvis 最终路由 instance 的强制约束。

每个启用入口必须覆盖：

- 文本、图片、视频、音频、文档、压缩包各至少一次入站。
- 文本、图片、视频、音频、文档、压缩包各至少一次出站。
- 多附件入站和多附件出站。

不要求每个入口执行完整的六类输入乘六类输出笛卡尔积。可以在入口间分配不同经典组合，但不能让任一启用入口完全缺失某一种入站或出站消息类型。

如果外部平台明确不支持某消息类型或多附件，必须记录 `platform_limitation` 证据并验证规定的降级行为，不能静默丢弃。

### 10.3 单轮经典场景

至少覆盖：

- 文本问答、摘要和结构化输出。
- 图片理解、OCR、生成和编辑。
- 音频识别、非语音音频判断、语音合成和增强。
- 视频理解、生成、编辑和长任务回传。
- 文档阅读、问答、数据提取和新文档生成。
- 多文档索引与检索：文档 chunk -> `embedding.text` -> 向量检索 -> `rerank` -> 基于命中文档回答；如 Jarvis 当前未采用该链路，记录为 `not_applicable`，不能伪造内部调用。
- 图文检索：文本和图片 -> `embedding.multimodal` -> 同 space 相似度检索 -> 返回命中图片和解释；只在 Jarvis 配置了该能力时执行。
- 压缩包解包、内容处理、结果文档和重新打包。
- 一次回复同时包含文本和一个或多个附件。

### 10.4 多附件

除非平台明确不支持，入站和出站都必须支持多附件。

入站至少覆盖：

- 多张图片。
- 文本、图片和文档组合。
- 音频和图片组合。
- 视频和字幕文档组合。
- ZIP 和补充说明文档组合。
- 快速连续消息中的多个附件。
- 当前消息附件与历史消息附件同时存在。
- `reply_to` 指向包含多个附件的历史消息。

必须验证附件数量、顺序、MIME、文件名、完整对象 ID、消息归属和 Jarvis 选择结果。

出站至少覆盖：

- 多张图片。
- 文本报告和 PDF/Office 文档。
- 图片和音频讲解。
- 视频、字幕和封面图。
- 结果文档和 ZIP。
- 长任务完成后的多附件主动回传。

平台不支持时，允许按明确策略拆成有序消息、生成 ZIP、返回 Named Store 引用或明确失败。报告必须同时记录标准出站消息和平台实际投递结果。

### 10.5 压缩包

压缩包由 Jarvis 处理，标准链路为：

```text
消息入口
  -> msg-center 保存附件
  -> Jarvis 安全解包
  -> 识别内部文档和媒体
  -> 调用一个或多个 AICC 能力
  -> Jarvis 汇总或生成结果
  -> 可选重新打包
  -> msg-center/message-tunnel 出站
```

至少覆盖：

- 单文档、多文档和文档媒体混合 ZIP。
- 多层目录、中文文件名、同名文件和空目录。
- 空、损坏、加密和不支持格式的压缩包。
- 路径穿越文件名。
- 超大单文件、总解压量、文件数量和嵌套深度限制。
- 输出新 ZIP，并检查内部文件清单、类型、digest 和可读性。

Jarvis 必须限制解压目标路径、文件数、单文件大小、总大小和嵌套深度。

### 10.6 多历史消息任务

每个场景包含前后依赖的多个指令，至少覆盖：

- 外部图片上传 -> 编辑 -> 动画化编辑结果。
- 外部视频上传 -> 分析 -> 二次剪辑或风格化。
- 外部音频上传 -> 识别/分析 -> 生成二次音频。
- 外部文档或 ZIP -> 提取事实 -> 生成新文档或 ZIP。
- Provider 生成图片 -> 同 Provider 再编辑。
- Provider 生成视频 -> 同 Provider continuation/extend。
- 上轮生成内容不重新上传，直接通过历史继续创作。
- `reply_to` 引用较早素材，不能误用最近附件。
- 快速连续发送素材 A、B 后明确选择其中一个。
- 同一 session 历史路由软优先及硬约束导致的重新路由。

“同 Provider 二次创作”必须验证 `provider_task_ref`、source task ID、Provider operation ID、exact model、Provider instance、continuation options 和输入 artifact 引用被保存和恢复；不支持原生续作时必须合理降级并向用户明确说明。

### 10.7 消息与投递语义

必须覆盖：

- 私聊、适用时的群聊。
- `reply_to`、forward 和会话归类。
- 消息顺序和附件绑定。
- 重复入站幂等。
- 重复出站请求不产生重复投递。
- 单附件失败时整体失败或部分成功的明确策略。
- tunnel 可重试失败、最终失败和投递状态回报。
- msg-center/Jarvis 重启后历史、任务和未完成投递恢复。
- 长任务完成后主动回传到原确定性 DID/信道。

### 10.8 Telegram 自动化边界

Telegram Bot API 不能模拟 owner 用户向 bot 发消息。初始方案可以使用真实 owner 的人工入站配合 runner 自动判定出站；如果要求完全自动化，必须另行设计受控 Telegram 用户测试账号、客户端协议、凭据保护和风控方案。引入新依赖前需要单独确认。

### 10.9 T3 通过标准

- 每个启用入口的六类入站和六类出站覆盖齐全，或有平台限制和通过验证的降级策略。
- 多附件不丢失、不乱序、不串到其他消息。
- 多轮任务引用正确的历史输入或生成物。
- 输出消息种类、附件数量和 MIME 严格符合 case 声明。
- 无重复用户可见消息、孤儿附件和无法关联的任务。
- message ID、session、AICC task/trace、Provider operation 和出站消息可关联。
- 结构断言全部通过，语义 Judge 达标或完成人工复核。

## 11. AICC 横切能力

以下能力必须按 T1 路由与系统行为、T1.5 Provider 协议、T2 真实推理的职责分工覆盖，并在 T3 选择代表场景验证链路表现。

### 11.1 Task 生命周期

完整状态机、幂等、取消、恢复和异常分支在 T1 覆盖；Provider 专属同步/streaming/异步 wire 状态映射在 T1.5 覆盖。T2 每个矩阵单元只验证其最小正常推理能够到达正确终态，不重复生命周期排列组合。

- immediate succeeded。
- running -> succeeded、failed、cancelled。
- task 查询、cancel 和 unknown task。
- 无权限查询或取消。
- 相同 idempotency key 的重复请求。
- 相同 key 不同请求体冲突。
- 并发完成、重复终态和迟到 Provider event。
- 服务重启后的异步任务恢复或明确终止语义。

### 11.2 Usage、费用和 quota

quota、budget、幂等计费和 fallback 归因在 T1 覆盖；Provider usage 字段与单位的解析在 T1.5 覆盖。T2 只核对真实调用返回的 usage/cost/trace 已归因到当前矩阵单元，并把实际费用写入报告。

- 成功且 Provider 返回 usage 时写入一次 durable usage。
- 缺 usage 的成功响应按协议要求处理。
- 幂等调用不重复计费。
- fallback 多次 Provider 调用分别记录，最终任务归因明确。
- tenant、Provider instance、model、api_type 和 task 归因。
- quota、budget、余额不足的路由拒绝。
- 估算成本与实际 usage/cost 的报告。

### 11.3 认证、安全和隔离

- 无 token、无效 token、过期 token。
- RBAC 和管理 method 权限。
- 跨用户、跨 tenant 的任务、usage、消息和对象隔离。
- Provider key、session token、私钥和敏感请求不进入日志与报告。
- route trace、Provider error 和 Judge 输入脱敏。
- 外部 URL、文档和压缩包的大小、安全边界与恶意内容处理。
- 文档中的 prompt injection 不得改变系统权限和测试环境边界。

跨用户、跨 tenant、RBAC 和管理 method 授权用例只在 T1 执行。需要第二租户的用例必须始终保留在 manifest；未配置 `other_tenant_session_token` 时明确记为 `skipped`，不得使用同租户凭据伪造通过，也不得阻断其它 T1 用例。

### 11.4 配置和维护

- `service.reload_settings` 成功和失败，并验证 `buckyos-api::aicc_client` 及全部调用方只发送该 method；`reload_settings` 和错误拼写均被拒绝。
- 非法新配置失败后继续使用旧配置。
- Provider validate/add/update/delete/refresh models；`provider.update` 覆盖 enable/disable、base_url/credential/Profile/Adapter/discovery 修改、revision 冲突和实例停止/替换生命周期。
- `driver_metadata_update.get/set` 覆盖三类 metadata catalog 的云更新配置、target/applied seq 状态和 RBAC。
- 多 instance 独立更新与删除。
- Provider 停止、禁用、删除、reload 替换和 AICC 服务退出都必须向对应库存刷新定时任务循环发送幂等 `Stop` 事件并等待优雅退出；验证停止后没有新探测、孤儿定时器或迟到的 inventory/health 写入，重新启用后创建新循环并从持久 seq 继续收敛。
- metadata 云端发布使用严格递增且不可复用的 manifest `revision_seq`，按客户端版本/通道/灰度分组配置兼容目标；验证新旧客户端各自获得兼容版本，低序列回退、同序列不同内容和不兼容版本均被 NDN 更新链路拒绝。
- NDN 替换文件后令 `metadata_target_seq = manifest.revision_seq`；分别验证推理前和 Provider 定时库存刷新触发同一全局收敛，所有落后 Provider 成功后才推进各自 `metadata_applied_seq`。
- 在 NDN 下载、校验、替换和文件就绪确认各阶段注入失败，确认 `metadata_target_seq` 不提前推进；在 Provider inventory 刷新进行中和失败时，确认该 Provider 的 `metadata_applied_seq` 不提前推进且原 inventory 不变，并在 health/trace 中暴露 applied/target seq 落后及失败原因。
- variants、version rules 和 routing config 更新。
- inventory 与发布基线同步。
- metadata 文件更新失败恢复由 NDN 验证；AICC 刷新失败不得推进对应 Provider 的 applied seq。服务重启后 target/applied seq 和配置保持一致。

Gateway、消息入口、登录信息、Provider API token 和选定 instance 必须通过 `aicc_acceptance.toml`、`jarvis_media_dv.toml` 或等价的显式参数配置。测试在授权范围内可以临时新增 Provider instance 或修改 AICC settings，但必须：

- 在 `finally` 中按原始序列化内容恢复 settings。
- 等待 system-config 与 AICC runtime settings 的异步传播收敛后再执行断言或清理校验。
- T1/T1.5/T2 如临时修改 Provider settings，必须验证运行时 Provider inventory 也已恢复，不能只验证配置存储值；T3 只要求原样恢复其临时修改的 settings。
- 确保 API token、登录凭据和 session token 不进入控制台日志、报告或持久化 fixture。

### 11.5 并发与稳定性

- 多 session 并发路由互不污染。
- 不同 session、不同 case 应并发执行，避免无必要的全串行阻塞。
- T1/T1.5/T2 runner 必须同时提供全局并发上限、每个 Provider 的并发上限和每个 Provider 的最小请求间隔。
- T1/T1.5/T2 retry 必须重新经过相同的全局及 Provider 限流，不得绕过并发和请求间隔门禁。
- T3 只限制场景并发；不在 T3 runner 内重复实现全局/Provider 并发和最小请求间隔门禁。
- 多 Provider instance 并发调用只在 T1 Mock 场景验证。
- 并发 idempotency。
- 异步任务并发完成。
- timeout 有上限且输出最后已知状态。
- runner、Mock Provider 或服务异常退出后能够清理测试状态。

## 12. 可观测性与报告

### 12.1 关联字段

在适用场景下，报告应关联：

```text
run_id
  -> case_id
  -> inbound platform/external message ID
  -> BuckyOS message ID/session ID
  -> Jarvis session/round
  -> AICC trace ID/task ID
  -> selected exact model/provider instance
  -> Provider request/operation ID
  -> usage/cost
  -> outbound message ID/delivery record
  -> artifact/named object ID
```

### 12.2 失败分类

至少使用：

- `preflight_failed`
- `baseline_mismatch`
- `routing_failed`
- `provider_protocol_failed`
- `provider_runtime_failed`
- `task_lifecycle_failed`
- `resource_failed`
- `message_transport_failed`
- `attachment_failed`
- `usage_failed`
- `security_failed`
- `judge_failed`
- `assertion_failed`
- `cleanup_failed`
- `platform_limitation`

### 12.3 报告内容

报告至少包含：

- 总体、分层、Provider、instance、model、api_type、入口和消息种类统计。
- baseline revision 和当前版本/commit。
- planned、passed、failed、provider_restricted、skipped、not_applicable、review 数量。
- 每次 attempt、耗时、错误码、failure class 和脱敏诊断。
- T1 路由分支/组合覆盖率。
- T1.5 按 Provider、adapter/API version、API type 的官方协议请求/响应/错误覆盖率及证据 revision。
- T2 `ProviderInstance × model × API-Type` 矩阵结果和推理正确性结论。
- T3 各入口入站、出站和多附件覆盖率。
- 真实调用次数、usage 和预计/实际成本。
- 清理结果和遗留资源。
- 已确认产品缺陷的预期行为、实际行为、复现 case、脱敏诊断和证据路径。
- 可直接复制执行的针对性复测命令；命令必须支持重复传入 `--case <case_id>`，以便修复后只重跑相关 case。

不得在报告中保存 Provider key、session token、用户密码、私钥、完整敏感 prompt 或未经脱敏的 Provider 原始响应。

真实调用的财务报告必须按 case、attempt、Provider driver、Provider instance、精确模型和 API 记录 usage/cost。Judge 调用属于真实模型调用，必须单独归因并计入调用次数和预算。Provider 未返回真实费用时必须标记为未知费用并保留估算敞口，不得按零费用处理。

## 13. 执行模式与门禁

T2/T3 会访问真实 Provider、真实消息入口或修改运行环境。CodeAgent 为检查自身开发结果而运行 T2/T3 前，必须获得当次人工授权；已有测试代码、配置文件、`--yes`、确认倒计时超时或历史授权不能自动视为本次执行许可。授权应明确允许的 Provider、入口、配置变更和费用上限。工程内 CodeAgent 按 `harness/SKILLS/aicc-e2e-test/SKILL.md` 执行该门禁；runner 的确认机制只防止人工误触，不替代 CodeAgent 授权。T1/T1.5 Mock 在不产生真实费用且配置变更已显式允许时，可按普通 CI 门禁执行。

### 13.1 普通 CI

- 执行 T1 全部 P0 Mock 用例。
- 执行 T1.5 全部 P0 协议契约用例。
- 不访问外网，不读取真实 Provider key。
- T1/T1.5 P0 任何失败阻断合入。

### 13.2 Nightly

- 执行 T1 全量。
- 执行 T1.5 全量，并按 Provider 分片。
- 按 `ProviderInstance × model × API-Type` 分片执行 T2。
- 执行 T3 自动化 MessageHub/msg-center 代表场景。
- Telegram owner 入站、需要真实外部账号的 Email/Slack 入站和需要人工视觉、听觉判断的场景可以独立安排。

### 13.3 发布验收

- 重新检查并冻结 Provider 模型能力基线。
- 执行 T1 全量。
- 执行 T1.5 全 Provider 协议契约矩阵。
- 执行 T2 全部获准 Provider instance 的 `ProviderInstance × model × API-Type` 矩阵，每个单元使用一个最小正确性用例。
- 执行 T3 各启用入口、六类消息、多附件和多轮核心场景。
- 所有 `review` 必须处理完毕。
- skipped 和平台限制必须有明确证据和发布批准，不能被计为 passed。

## 14. 发布前能力同步流程

每次 Provider、模型、metadata、路由或 adapter 变更后，至少执行：

1. 更新官方能力证据和 release baseline。
2. 更新 AICC metadata、inventory、api_type 和 adapter。
3. 对比官方能力与 AICC 声明，确保双向一致。
4. 更新受影响的 T1/T1.5/T2 case manifest。
5. 执行受影响 case。
6. 执行该 Provider 的 T1.5 协议回归和受影响的 T2 `ProviderInstance × model × API-Type` 单元。
7. 执行全量发布验收。
8. 验证事实配置、运营策略和路由配置的回滚。

## 15. 建议实施里程碑

| 里程碑 | 范围 | 完成标准 |
|---|---|---|
| M0 规格冻结 | api_type/method、历史路由偏好、消息种类、平台限制 | 所有待定语义有书面结论 |
| M1 公共基础 | fixture、manifest、Mock、报告、Judge | 可运行最小闭环并稳定复现 |
| M2 T1 路由门禁 | 路由、fallback、多 instance、错误注入 | P0 进入 CI 且 100% 通过 |
| M3 T1.5 Provider 协议门禁 | 官方协议证据、高保真 Mock、正常与错误契约 | 全部支持的 Provider adapter/API type 通过独立 wire 断言 |
| M4 T2 线上推理基线 | `ProviderInstance × model × API-Type` 最小真实推理矩阵 | 全部获准矩阵单元有结果且成本受控 |
| M5 T3 Jarvis 链路 | 六类消息、多附件、多轮、三个入口 | 自动及人工链路完成验收 |
| M6 发布门禁 | 全量执行、报告、成本和清理 | 无未解释能力缺口和阻断失败 |

## 16. 最终验收标准

本方案完成必须同时满足：

- T1 能确定性证明所有路由分支会选择正确模型/instance 或返回正确错误。
- T1.5 能依据 Provider 官方资料独立证明 AICC wire 请求、正常响应解析和错误映射正确。
- T2 能以最小真实调用证明每个获准 `ProviderInstance × model × API-Type` 单元的线上推理正确。
- 每次发布有版本化、可追溯的 T1.5 Provider 协议基线和 T2 `ProviderInstance × model × API-Type` 能力基线。
- T3 能证明每个启用入口支持六类消息、多附件和代表性多轮任务，或按平台限制正确降级。
- task、resource、usage、quota、安全、配置和可观测性要求均有对应 case。
- 输出消息种类不匹配必然失败；内容语义通过确定性断言、LLM Judge 和必要人工复核分层判定。
- 测试默认不产生真实模型费用，真实调用成本可计划、可限制、可审计。
- 测试可重复运行，敏感信息不泄漏，临时配置和资源可安全清理。

# AICC Method 验收清单与真实模型判定

定义所有 AICC method 的必测输入、必测输出、异常路径、Agent Runtime 占位语义和真实模型稳定判定规则。

本文档是拆分后的自包含验收任务文档。实现或评审本任务时，以本文档和 README 中列出的依赖文档为准。

## 1. Method 验收清单

本节同时覆盖标准 AI 推理 method、分层 API method 和控制/管理 method。`method` 是 kRPC schema discriminator；`api_type` 只用于 `route.resolve`、Provider inventory 和逻辑目录过滤。

当前实现里的 canonical `ApiType` 序列化值以代码枚举为准：LLM 为 `llm`，不是 `llm.chat`；chat 的真实调用 method 仍是 `llm.chat`。验收用例必须同时验证：

- `route.resolve(api_type="llm")` 可路由到支持 chat 的模型。
- `route.resolve(api_type="llm.chat")` 的行为必须与当前协议约定一致：若实现尚未接受该别名，应返回稳定、可判断的错误，并在报告中标注为命名兼容缺口。
- `embedding.multilingual`、`embedding.code` 当前不是正式 `ApiType` 枚举项；如文档或 inventory 中出现，应作为 capability / logical mount / metadata 标签处理，不能当作已支持的标准 `api_type` 误判为缺测。

### 1.1 LLM

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `route.resolve` | `api_type`、逻辑模型名 `logical_model`、requirements、disable、policy | selected ModelUID/exact model、Provider/Profile/adapter、Model Driver/origin、原始 provider model、operation、fallback、capabilities、trace | 传入 exact model 被拒、无匹配、多 Driver 冲突 |
| `chat.completions.create` | `exact_model`、content-block `messages`、tools、response_format | `message: AiMessage`、`tool_calls`、`finish_reason`、usage、route trace | 传入逻辑模型名被拒、primary quota exhausted 不 fallback、无注册 operation |
| `helper.llm_chat` | 逻辑模型名 + messages | 等价于 `route.resolve` + `chat.completions.create` | 与两阶段行为一致性 |
| `llm.chat`（legacy） | content-block messages、image/document/tool_use block、tools、response_format JSON schema、generation params | `text`/`message`、`tool_calls`、`finish_reason`、usage、route trace | tool schema 非法、JSON schema 不满足、context too long、feature unsupported |

### 1.2 Embedding / Rerank

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `embedding.text` | text items、resource item、chunking、dimensions、normalize、`prefer_artifact` | 小批量 inline embedding、大批量 `data_resource` artifact、embedding meta | embedding space 不匹配、dimensions 不支持、resource invalid |
| `embedding.multimodal` | text + image pair、dimensions、normalize | 与 `embedding.text` 同结构，记录 `embedding_space_id` | fallback 到不同 embedding space 被拒绝 |
| `rerank` | query、text document、resource document、`n`、`return_documents` | `results[index,id,score]` | 文档为空、n 越界、不同 reranker 分数混排禁止 |

### 1.3 Image / Vision

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `images.generate`（typed inference） | `exact_model`、prompt、negative_prompt、size、quality、seed、output | image artifacts，FileObject meta 写 media type / size | 传入逻辑模型名被拒、primary 不 fallback |
| `helper.text_to_image` | 逻辑模型名 + prompt | 等价于 `route.resolve` + `images.generate` | 与两阶段行为一致性 |
| `image.txt2img`（legacy） | prompt、negative_prompt、n、aspect_ratio、quality、seed、output | image artifacts，FileObject meta 写 media type / size | output media type 不支持、预算超限 |
| `image.img2img` | source image、prompt、strength、output | image artifacts | source image invalid、strength 越界 |
| `image.inpaint` | image、mask、prompt、mask_semantics | image artifacts | mask 缺失、mask semantics 不兼容 |
| `image.upscale` | image、scale、target size、preserve_faces | image artifact | 目标分辨率不满足、fallback 不能满足硬约束 |
| `image.bg_remove` | image、mode、output | rgba image artifact | 输入非图片、输出 alpha 缺失 |
| `vision.ocr` | document、level、language_hints、return_layout、return_artifacts | text、`extra.ocr`、OCR artifacts | 不支持语言、文档 meta 缺失 |
| `vision.caption` | image、style、language、n | captions、summary text | n 越界、图片无效 |
| `vision.detect` | image、classes、score_threshold、bbox_spec | detections with bbox | bbox unit 不支持 |
| `vision.segment` | image、prompt、mask_format、return_bitmap_mask | masks、bitmap artifact | mask_format 不支持 |

### 1.4 Audio / Video

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `audio.tts` | text、voice contract、speed、output | audio artifact | voice_id 不可 fallback、sample_rate 不支持 |
| `audio.asr` | audio、language、timestamps、diarization、output_formats | transcript、segments、vtt/srt/json artifacts | output format 不支持、音频 meta 缺失 |
| `audio.music` | prompt、duration、instrumental、lyrics、seed、output | async task、audio artifact、structure | duration 越界、异步任务失败 |
| `audio.enhance` | audio、task、strength、return_stems | enhanced audio artifact、stems | task 不支持 |
| `video.txt2video` | prompt、duration、aspect_ratio、resolution、generate_audio、seed | async task、video artifact | operation timeout、Provider started 后不跨 Provider 重试 |
| `video.img2video` | image、prompt、duration、resolution | async task、video artifact | image invalid |
| `video.video2video` | video、prompt、preserve_motion、time_range | async task、video artifact | time_range 越界 |
| `video.extend` | video、prompt、continuation_handle、duration | async task、video artifact | continuation_handle 缺失或不匹配 |
| `video.upscale` | video、target_resolution、denoise、sharpen、output | async task、video artifact | target_resolution 不支持 |

### 1.5 Agent Runtime Support

`agent.computer_use` 当前作为占位方向，不作为普通 AICC v0 模型调用的强制真实 Provider 验收项。Mock 阶段只验证 schema、路由目录和安全约束：

- screenshot resource。
- viewport。
- allowed actions。
- action array response。
- 不允许无 sandbox / environment 上下文的真实执行。

### 1.6 Control / Management

| Method | 必测输入 | 必测输出 | 异常 |
|---|---|---|---|
| `cancel` | `task_id`、tenant/session 上下文 | accepted / rejected、原 task 状态可观察、task data / event 记录 cancel 语义 | unknown task、跨 tenant cancel、provider 不支持取消、已完成任务重复取消 |
| `service.reload_settings` | 空 params | reload 结果、Provider registry / ModelRegistry 重建摘要；被禁用、移除或替换的旧实例收到库存定时循环 `Stop` 并优雅退出 | settings 非法、凭据缺失、保留上一版可用配置、孤儿定时器或迟到写入 |
| `models.list` | 空 params、可选诊断过滤参数 | Provider inventory、完整模型/渠道身份、operations、逻辑目录、health 摘要 | registry 为空、敏感字段泄露、损坏 catalog 不应导致服务不可诊断 |
| `usage.query` | 时间窗口、provider/model/method/api_type 过滤 | 聚合 usage、明细数量、成本/usage 字段、空结果 | 非法时间窗口、无权限、重复幂等记录不应重复计费 |
| `quota.query` | capability / method、tenant/session 上下文 | 剩余额度、预算状态、限制来源 | 未配置 quota、跨 tenant 查询、非法 method |
| `provider.list` | 可选 instance/profile/adapter 过滤 | Provider 列表、inventory 摘要、health、capability、pricing 脱敏视图 | 无权限、凭据泄露、Provider 状态异常仍可诊断 |
| `provider.health` | Provider Instance | health 状态、最近错误摘要、latency / quota / availability | Provider 不存在、health 过期、敏感错误未脱敏 |
| `provider.validate` | Provider Instance 草案、endpoint、Profile、Adapter、auth | schema 校验结果、可连接性 / mock 可达性、脱敏诊断 | 凭据缺失、endpoint 非法、未知 Profile/Adapter、不得写入 system_config |
| `provider.add` | provider settings、tenant/session 上下文 | system_config 写入、reload 后 `models.list` 可见、审计记录 | 重名冲突、无权限、schema 非法、写入失败回滚 |
| `provider.delete` | provider instance name、tenant/session 上下文 | system_config 删除、库存定时循环收到幂等 `Stop` 并优雅退出、reload 后候选消失、相关 routing 诊断 | 删除不存在、仍被 policy 锁定引用、无权限、孤儿定时器或停止后迟到写入 |
| `provider.refresh_models` | Provider Instance、刷新策略 | model 列表变化时更新库存；target/applied seq 不同时触发所有落后 Provider 收敛；列表未变且 seq 相同时只探测 | Provider 不可达、重建失败不推进 applied seq、目标在刷新中再次变化 |

## 2. 真实模型判定规则

Protocol Adapter 的结构验收同时是发布门禁：OpenAI、Claude、Gemini 必须覆盖官方新接口；历史 API 代际只在首个真实 Provider 需求出现时按需实现，注册后由所有兼容 Provider 共享。每个实际注册的 API 代际使用不同 `protocol_adapter_id`，但只运行一套共享 contract test；第二、第三个 Provider 不得复制历史 wire protocol 实现或整套 contract test。新旧 Adapter 不得相互 fallback，只允许复用协议中立组件。自定义 Provider 必须验证接入测试优先选择新接口、按序尝试已注册历史接口、非协议错误停止探测并持久化 resolved Adapter；用户不选择 API 代际。派生 Adapter 必须暴露独立 ID 和 `base_adapter_id`，并只测试渠道差异及委托关系。SN 必须分别通过 API Key、动态登录、token 过期刷新和认证失败测试，并通过“移除 SN 后 `openai-responses` 测试与行为不变”的删除性测试。

真实模型输出不可完全确定，验收断言必须避开自然语言全文匹配。

| 类型 | 可稳定断言 | 不应断言 |
|---|---|---|
| `llm.chat` | status、非空 text 或 tool_calls、usage、finish_reason、route trace | 回答全文、具体措辞 |
| JSON schema | JSON 可解析、包含 required 字段、字段类型正确 | 字段内容完全一致 |
| tool call | tool name 在允许集合内、args 可解析、required args 存在 | args 的自然语言细节完全一致 |
| image/audio/video artifact | artifact 存在、media type 正确、可读取、size > 0 | 视觉/听觉内容完全一致 |
| async task | running -> succeeded/failed 有闭环、失败有 error code | Provider 完成时间固定 |
| usage/cost | usage 存在且数值非负、真实调用次数受控 | token 数精确一致 |
| trace | final_model、provider、fallback/failover 标志、trace id 存在 | score 细节完全固定 |

真实模型可接受的 `partial`：

- Provider 成功返回，但内容安全策略导致模型拒答，协议链路和错误分类正确。
- Provider 临时不可用，AICC 返回明确 provider error 或 failover trace。
- artifact 生成成功但模型内容不满足人工预期，协议事实正确。

真实模型不可接受的 `partial`，应判为 `failed`：

- task 卡住且没有超时错误。
- usage 缺失但被当作成功。
- artifact 引用不可读取。
- trace 缺失或泄露敏感信息。
- Provider key、token、原始 prompt 出现在报告中。

真实模型重试判定：

1. 重试粒度是单个 `api_type × method × logical_path × Provider × model` 用例，不得扩大到整个 Provider 或整个测试批次。
2. 第 1 次 attempt 失败后，runner 应立即重跑同一用例；第 2 次仍失败时再重跑第 3 次。
3. 任意 attempt 满足通过断言，则该 case 最终状态为 `passed`，并在报告中标注 `passed_after_attempt=N`。
4. 三次 attempt 全部失败时，该 case 最终状态为 `failed`，主失败原因取最后一次 attempt，同时保留全部 attempt 明细。
5. `skipped` 不重试；preflight / 配置错误不重试；明显安全失败不重试。
6. 对已经返回 `running` 或已提交异步任务的 attempt，不允许在同一个 task 上静默重复提交；重试必须创建新的 case attempt id，并在报告中记录可能产生的真实费用。

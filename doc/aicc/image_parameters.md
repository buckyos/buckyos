# 图片参数与错误语义

文生图 `images.generate` 与图生图 `image.img2img` 是不同的请求 schema。定义以 `src/kernel/buckyos-api/src/aicc_client.rs` 为准，TypeScript 消费方为 `buckyos-websdk/src/aicc_client.ts`。

## 请求参数

| 含义 | 文生图 | 图生图 |
| --- | --- | --- |
| 提示词 | `prompt` | `prompt` |
| 参考图 | 无 | `images` |
| 负面提示词 | `negative_prompt` | `negative_prompt` |
| 质量档位 | `quality` | `quality` |
| 宽高比例 | `aspect_ratio` | `aspect_ratio` |
| 尺寸 | `size` | `output.size` |
| 随机种子 | `seed` | `seed` |
| 输出格式 | `output.media_type` | `output.media_type` |
| 变换强度 | 无 | `strength` |

图生图拒绝顶层 `size`；`aspect_ratio` 是两边为 1–100 的 `W:H`。同时给出像素尺寸与比例时，两者必须一致。尺寸的值由渠道解释：Gemini Interactions 使用 `1K`、`2K`、`4K` 档位，不接受精确 `WxH`；不能无条件向不同渠道转发相同值。

## 渠道支持

| 渠道/operation | 图生图映射 | 明确拒绝的选项 |
| --- | --- | --- |
| Gemini Interactions | 比例 → `response_format.aspect_ratio`，尺寸 → `image_size`，种子 → `generation_config.seed` | `negative_prompt`、`quality`、`strength`，非档位尺寸 |
| OpenAI Images / Responses | `output.size`、`quality`、输出格式 | `aspect_ratio`、`negative_prompt`、`seed`、`strength` |
| OpenRouter Image | `aspect_ratio`、`output.size`、`quality`、`seed`、输出格式 | `negative_prompt`、`strength` |
| Seedream | `output.size` 或按既有比例表转换为尺寸；`seed` | `negative_prompt`、`quality`、`strength`，比例表外的比例 |
| Qwen image edit | `output.size` | `aspect_ratio`、`negative_prompt`、`quality`、`seed`、`strength` |
| MiniMax subject reference | `aspect_ratio` | `negative_prompt`、`quality`、`seed`、`strength`、非空 `output` |
| fal Queue | 保留已有负面提示词、种子、强度映射 | `aspect_ratio`、`quality`、非空 `output` 尚无统一映射；文生图也拒绝未映射尺寸和样式 |

新增协议字段不代表任意模型支持。不支持的选项在发送上游请求前失败：精确模型返回 `unsupported_operation` 并指出字段及渠道，逻辑路由在筛选后无候选时返回 `no_candidate_model`，不改写为提示词、静默删除或擅自降级。文生图也遵循此规则；Gemini 的 `quality`、`negative_prompt` 不能当作已支持的能力。

`models.list` 的 `capabilities` 列出当前 operation 已实现的参数，例如 `image.img2img.aspect_ratio: true`、`image.img2img.output.size: true`。这些是适配器声明的参数映射能力，不保证任意参数值都受具体模型支持。

`route.resolve` / Helper 的 `requirements.image_parameters` 是这些能力名的数组。Helper 和 typed 图片调用会自动从显式请求参数生成要求；AIWorkspace 在图生图预路由时也发送这些要求。逻辑路由筛除未声明该能力的候选，精确模型在发送上游请求前报告具体未支持字段。参数值仍由渠道编码和模型校验，不能由能力标记推断任意尺寸/质量值均合法。

Gemini 字段来源：[官方 Interactions API](https://ai.google.dev/api/interactions-api)。协议测试验证请求编码；真实模型接受哪些比例和档位，仍须按模型验收。

## AIWorkspace 对接

宏保持 `aiws.ai.image({ prompt, images?, aspect?, size?, quality?, negative?, seed?, format? })`。host 有参考图时使用图生图，并把尺寸放进 `output.size`，同时保留输出格式。`size` 接受像素形式及 `1K`、`2K`、`4K` 档位，只能使用所选渠道支持的值。

明确请求比例时，host 复核图片实际尺寸；允许 5% 比例误差以容纳模型离散分辨率。尺寸缺失或比例不符返回 `AI_INVALID_ANSWER`，不作为成功结果缓存；缓存命中也复核比例。该检查不做拉伸、裁剪或跨步骤画幅规划，#745 不在此次实现范围。

## 路由错误码

- `no_candidate_model`：逻辑路径没有可达模型，或没有候选满足请求；检查渠道启用、模型发现/排除规则和请求能力。
- `resource_not_permitted`：存在可达候选，但当前身份均无权使用；检查授权。
- `quota_exhausted` / `quota_budget_exceeded`：独立额度错误，模型缺失不能归为额度不足。
- 精确模型对受限身份保留资源不可见规则，不用“不存在”的提示泄露隐藏资源。普通错误不列出不可见模型，管理员通过模型清单、Provider 发现详情和路由预览定位原因。

这是共享 API 与 UI DataModel 的接口变更；已有服务、模块安装、持久表结构和第三方依赖不变。

SDK xllm、宿主机 xllm 和 AIWorkspace 保留 `no_candidate_model` 原因。AIWorkspace 使用 `AI_NO_CANDIDATE_MODEL` 提示检查模型清单，不再引导用户申请模型权限。

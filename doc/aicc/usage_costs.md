# AICenter 用量与成本（#755）

## 结算顺序

有效的响应 `usage.cost`（金额和币种）优先，其次是 `reported_cost` 加 Provider Rules 声明的币种，最后使用调用开始时冻结的价格和实际用量。非有限值、负金额与空币种不能作为有效成本。显式零单价表示免费，缺价格或缺用量表示未知，二者不同。

`usage.cost_source` 区分 `provider_reported` 与 `static_pricing`；`usage.cost_unknown_reason` 给出缺失用量、费率、未核实币种等原因。静态价格是配置报价下的估算，不代表代理服务的实际扣款。异步结果优先采用最终结果用量，缺失字段由任务状态补充，不重复累加；视频时长后备只补服务端没有提供的字段。

SN 静态价格统一采用 OpenRouter USD 报价（2026-11-22），不再混用原厂直连价。`models?output_modalities=all`、embeddings 和 video models 目录覆盖非文本模型；每个 SN 别名对应精确的 OpenRouter slug，不能按相邻型号或相同权重猜价。冻结的上游字段、型号映射和日期保存在 `test/aicc_test/acceptance/sn_openrouter_prices.json`，metadata 的 `source_url` 指向对应 OpenRouter 型号页。输入、输出、缓存读写、上下文阶梯、UTC 时段/星期规则及视频分辨率 SKU 均按该快照配置。Qwen Image 3 Pro 使用同一型号页面明示的 1K $0.04/张、2K $0.075/张及输入参考图 $0.003/张，快照也保存此口径，不要求只返回图片张数的响应虚构图片 token。

54 个 SN 映射中已配置 52 个。`ui-tars-1.5-7b` 为 $0.10/$0.20 每百万输入/输出 token，`qwen3-embedding-8b` 为 $0.01/百万输入 token，二者现在均有 OpenRouter USD 配置。`gpt-4o-transcribe-diarize`、`gpt-4o-mini-tts` 未出现在此次 OpenRouter 全模态目录，不保留原厂价格冒充 OpenRouter 报价；有效响应成本仍优先，未实报时保持未知。基础价格未覆盖的搜索工具调用等额外 SKU 不能视为免费，静态金额仅是已配置维度的估算。

来源：[OpenRouter Models API](https://openrouter.ai/docs/guides/overview/models)、[全模态报价](https://openrouter.ai/api/v1/models?output_modalities=all)、[Embeddings](https://openrouter.ai/api/v1/embeddings/models)、[Video models](https://openrouter.ai/api/v1/videos/models)。OpenRouter 的 Kimi 报价与原厂直连价可能不同；SN 的缺省 USD 声明仍只限于已从现场实报金额核实的 `kimi-k3` / `kimi-k3.1-pro`，不根据静态报价推断其他型号返回的无币种金额。

## 组合计费

复用 `pricing.addons`：既能扩展单位计费，也能扩展 token 计费。所有非零费用项必须有有效用量，缺少一项则整个静态成本未知，不输出部分费用冒充完整费用；明确零数量仍可结算。addons 与基础费率一起冻结，token 阶梯和时段选择保持原有语义。

SN [Voxtral Small](https://openrouter.ai/mistralai/voxtral-small-24b-2507) 使用 OpenRouter 的文本输入 $0.10/百万、输出 $0.30/百万、缓存命中 $0.01/百万，以及音频 $0.0001/秒。配置的 `audio_input_token: 0` 从总输入中排除已上报的音频 token，音频本身由秒数计价，防止重复计费；若响应包含音频 token，必须能与文本部分拆分。OpenAI Chat 用量解析保留 `prompt_tokens_details.audio_seconds` / `usage.audio_seconds`，不会把音频 token 数换算为秒。无音频的已知零秒应显式上报；未上报时保持未知。真实 SN 音频字段仍需实测，不用 mock 代替 Provider 验证。

```json
{
  "currency": "USD",
  "input_token": 0.0000001,
  "output_token": 0.0000003,
  "cache_input_token": 0.00000001,
  "audio_input_token": 0,
  "addons": [{ "unit": "audio_second", "amount": 0.0001 }]
}
```

OpenRouter 的非零每请求附加费也作为 addon 保留，不再因为存在 token 价格而丢弃；历史恢复工具按同一冻结规则求和，缺数量时跳过。

Gemini Omni 1.1 明确区分图片/视频输入 token 与视频输出 token：文本、图片、视频输入 $1.50/百万，文本输出 $9/百万，视频输出 $17.50/百万。2026-12-03 后官方将其重定向至 1.2 并更换视频输出费率，需要届时更新 catalog，不能把未来费率用于历史调用。来源：[Gemini pricing](https://ai.google.dev/gemini-api/docs/pricing)。视频响应缺少模态拆分时保留未知，不能将视频输出全部按文本价格结算。

## 计费用量

`AiUsage` 记录总/输入/输出 token、缓存命中/写入/1h 写入、推理 token、图片/音频/视频输入与输出 token、生成图片和参考图片张数、音频/视频秒数、字符数、请求单位，以及 `compute_seconds`、`megapixels`。

缓存、多模态、推理是总量的子集；1h 写入是缓存写入的子集，不能重复求和。处理耗时不是媒体长度。请求数不是 token。参考图片与生成图片分开统计。

- Gemini 媒体结果按实际成功图片产物填张数，并记录一次成功请求；音乐按请求单价计价。
- OpenAI Responses 图片也统计实际产物数；独立 Images 接口已有计数。转写优先使用 `usage.seconds`，只有缺失时才使用官方顶层 `duration` 后备。
- 同步 TTS 沿用统一字符数补全；豆包使用服务端 `text_words`。Qwen、豆包、MiniMax 原有图片计数和音视频时长逻辑保持。
- fal 的 `metrics.inference_time` 来自任务状态，作为计算秒合并到结果；DeepFilterNet 的 `audio_file.duration` 为音频秒数，不用 `timings.inference` 代替音频长度。来源：[Queue](https://fal.ai/docs/documentation/model-apis/inference/queue)、[DeepFilterNet schema](https://fal.ai/models/fal-ai/deepfilternet3/api)。
- fal 的 `fal-ai/video-upscaler` 按 $0.0008/百万像素收费，但公开 Output Schema 只有文件 URL、类型、名称和字节数，没有帧数、分辨率或完整的计费百万像素量。当前适配器也没有下载视频探测帧数并重建计费用量；尚未用真实任务响应确认是否有额外字段。结算器已支持 `megapixels`，但该响应仍显示 `missing_usage:megapixel`，不通过单帧宽高或请求目标分辨率虚构整段视频的计费像素。来源：[Video upscaler schema](https://fal.ai/models/fal-ai/video-upscaler/api)。

## fal 视频百万像素的核实办法

已确认本 endpoint 的公开单价为 $0.0008/百万像素，公开输出没有计费像素量。旁证如下，不能将其他 endpoint 的精确公式直接套过来：

- [fal 总计费说明](https://fal.ai/docs/documentation/model-apis/pricing) 说明 Model API 按产出计费。
- [fal 自己的模型比较](https://fal.ai/learn/tools/video-to-video-upscalers) 将 RealESRGAN 归为逐帧处理及按百万像素收费。
- [FlashVSR+](https://fal.ai/models/openimaginglab/flashvsr-plus) 明确写输出宽 × 高 × 帧数；[Crystal](https://fal.ai/models/clarityai/crystal-video-upscaler) 则按百万像素 × 秒 × FPS 分档，证明同平台也有不同公式。

优先只读查询已有任务对应的 [Billing Events](https://fal.ai/docs/platform-apis/v1/models/billing-events)（request_id）、[Usage](https://fal.ai/docs/platform-apis/v1/models/usage)（billed quantity、unit price、discount、cost_total），配合 [Pricing API](https://fal.ai/docs/platform-apis/v1/models/pricing) 确认具体 endpoint。Usage 可能是时间窗口内的聚合，需隔离该任务窗口或结合 billing event，不把整个窗口费用当作单次费用。

对已有输出实际探测宽、高及准确解码帧数，比较账单 quantity 与候选输入/输出累计像素量（宽 × 高 × 帧数 / 1e6）；不能只用 duration × fps 代替准确帧数。若历史样本不足，再安排受控的不同缩放倍率、帧数与 FPS 的短视频调用，检查最小计费量及取整，并向 fal 确认。此次没有新增付费调用或读取账号账单；公式仍未达到可用于自动结算的证据强度。

## 查询与 UI

`usage.query` 的 `total`、`grouped` 和 `buckets` 增加：

- `dimensions: { <AiUsage 数字字段>: { amount, reported_events } }`：每个实际上报维度的小计与覆盖记录数；明确上报的 0 保留，完全未上报的维度没有键。
- `unpriced_requests`：没有有效财务快照的记录数。

SQLite/PostgreSQL 在服务器上聚合已有 `usage_json`，无需新增持久列、迁移或为前端逐页取数据。成本按币种分别求和，未知成本不能作为零；空结果 `dimensions={}`、`unpriced_requests=0`。计费 JSON 数字与现有 top-level token 列须保持同源。

Usage 仍使用同一权限边界、时间窗口及筛选条件，通过一次 `summary_and_events` 查询获取完整汇总和一页记录；group_by 同时使用 Provider、模型、App、API 类型。分页游标只限制 events，总计与分组不受游标影响。对分组组合的返回量随实际分组组合数增长，未在本次验证百万级高基数结果。

Usage 消费卡沿用首页卡片样式，显示当前筛选全部记录的 token 和多币种金额。首页和 Usage 页的总计、各分组、列表及近期记录的 token 可展开各单位明细。多币种金额默认展示 USD（没有 USD 时显示稳定排序的首个实际币种），点击展开各币种，绝不隐式换汇。首页分组维度与费用来自同一 30 天/身份范围的服务端汇总，不通过近期五条记录估算。隐藏未上报维度、保留明确零值，覆盖不足时展示上报记录数；不将不同单位合并。SDK `AiUsage` / `UsageAggregate` 同步扩展，Desktop 保留原始 `usage_json`。

## 历史记录

发布新价格不会改变历史冻结快照，也不会自动修改历史成本。提供离线工具 `src/tools/repair_aicc_usage_cost.py`，默认只读 dry-run：

```bash
uv run python src/tools/repair_aicc_usage_cost.py --database /path/to/aicc-usage-log.db
```

在已经核实某个 Provider 的实报币种后，可显式指定（不是按模型金额自动猜测）：

```bash
uv run python src/tools/repair_aicc_usage_cost.py --database /path/to/aicc-usage-log.db \
  --reported-provider sn-ai-provider-default --reported-currency USD
```

确认 dry-run 中的恢复和跳过结果后，加 `--apply --audit /path/to/new-audit.json` 执行。工具先用 SQLite Backup 保存完整备份，审计文件记录每项修改前的用量/成本、恢复依据和结果；只更新仍未计价且与读取时内容相同的 ledger 行，任何并发冲突整批回滚。重复执行不重复计价。不会修改 execution 冻结快照、TaskMgr 已完成结果或原始 Provider 响应。

仅根据可验证的实报金额及币种、历史冻结单价、实际图片产物与已记录用量恢复；不会读取当前价格。缺原始证据、缺费率、模态不完整或复杂阶梯历史快照，均跳过并列原因。该工具未在用户运行中的 Zone 上执行。

## 验证与联动

本地协议/结算测试覆盖图片数、音乐请求、音频秒、计算秒、显式免费、币种优先级、Omni 视频费率；存储测试覆盖跨页相同筛选总计、多币种、0/null 和覆盖数。T1.5 fixture 同步补充 fal duration，且对媒体计费用量增加明确断言；真实 Provider 与 Zone Gateway 的 T1.5/T2 验证须另行安排，不能由本地测试代替。

按用户要求，内置手册 `aigc_image`、`aigc_audio`、`aigc_video`、`buckyos_admin` 不作修改，用量与成本说明集中在本文。没有修改 AgentRoot、Session/Behavior、skill 安装协议、工具挂载或发布命令。没有新增服务、第三方依赖或通用组件，workspace、安装映射和 scheduler 不需新增接入。

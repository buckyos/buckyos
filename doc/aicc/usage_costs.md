# AICenter 用量与成本（#755）

## 结算顺序

有效的响应 `usage.cost`（金额和币种）优先，其次是 `reported_cost` 加 Provider Rules 声明的币种。未返回有效协议费用的 fal 成功任务采用 Billing Events 的实报费用，尚未查到时暂用调用开始时冻结的价格和实际用量估算，并明确标记待账单核对。结算引擎在调用任何 Provider 估算器之前先处理有效协议费用，不能被组合计费或估算覆盖。非有限值、负金额与空币种不能作为有效成本。显式零单价表示免费，缺价格或缺用量表示未知，二者不同。

`usage.cost_source` 使用 `provider_reported`（协议费用）、`provider_billing`（账单费用）、`static_pricing`（静态估算）、`static_pricing_pending_billing`（估算且待查账）、`billing_pending`（未知且待查账）；`usage.cost_unknown_reason` 给出缺失用量、费率、未核实币种等原因。静态价格是配置报价下的估算，不代表代理服务的实际扣款。异步结果优先采用最终结果用量，缺失字段由任务状态补充，不重复累加；视频时长后备只补服务端没有提供的字段。

SN 静态价格优先采用 OpenRouter USD 报价（2026-11-22），仅对其未收录的两个型号按用户要求使用明确标注来源的 OpenAI 官方报价。`models?output_modalities=all`、embeddings 和 video models 目录覆盖非文本模型；每个 SN 别名对应精确的 OpenRouter slug，不能按相邻型号或相同权重猜价。冻结的上游字段、型号映射和日期保存在 `test/aicc_test/acceptance/sn_openrouter_prices.json`，metadata 的 `source_url` 指向对应 OpenRouter 型号页。输入、输出、缓存读写、上下文阶梯、UTC 时段/星期规则及视频分辨率 SKU 均按该快照配置。Qwen Image 3 Pro 使用同一型号页面明示的 1K $0.04/张、2K $0.075/张及输入参考图 $0.003/张，快照也保存此口径，不要求只返回图片张数的响应虚构图片 token。

54 个 SN 映射现在全部有静态配置：52 个采用 OpenRouter，OpenRouter 未收录的 `gpt-4o-transcribe-diarize`、`gpt-4o-mini-tts` 按用户要求采用 OpenAI 官方模型页报价，并在快照 `official_fallbacks` 中单独列明来源。前者模型页当前列示输入 $2.50、输出 $10 每百万 token；后者为文本输入 $0.60、音频输出 $12 每百万 token。未把估计的每分钟价格当成精确单价，也不由字符数推测音频 token；若响应仅有字符/时长、没有这些费率所需 token 用量，仍保留未知。基础价格未覆盖的搜索工具调用等额外 SKU 不能视为免费，静态金额仅是已配置维度的估算。

[OpenAI Diarize 官方模型页](https://developers.openai.com/api/docs/models/gpt-4o-transcribe-diarize)、[OpenAI Mini TTS 官方模型页](https://developers.openai.com/api/docs/models/gpt-4o-mini-tts)（2026-11-22）。

来源：[OpenRouter Models API](https://openrouter.ai/docs/guides/overview/models)、[全模态报价](https://openrouter.ai/api/v1/models?output_modalities=all)、[Embeddings](https://openrouter.ai/api/v1/embeddings/models)、[Video models](https://openrouter.ai/api/v1/videos/models)。OpenRouter 的 Kimi 报价与原厂直连价可能不同；SN 的缺省 USD 声明限于已核实的 `kimi-k3` / `kimi-k3.1-pro` 与 `voxtral-small-24b-2507`。Voxtral 在 [SN 发布配置](https://github.com/buckyos/sn-business/blob/main/CD/cluster_config/release/cluster_config.json) 指向 OpenRouter，SN Chat 入口原样透传响应，结合 [OpenRouter 的 USD 结算说明](https://openrouter.ai/support) 和真实响应确认币种；不根据静态报价为其他型号的无币种金额猜测币种。

## 组合计费

复用 `pricing.addons`：既能扩展单位计费，也能扩展 token 计费。所有非零费用项必须有有效用量，缺少一项则整个静态成本未知，不输出部分费用冒充完整费用；明确零数量仍可结算。addons 与基础费率一起冻结，token 阶梯和时段选择保持原有语义。

SN [Voxtral Small](https://openrouter.ai/mistralai/voxtral-small-24b-2507) 使用 OpenRouter 的文本输入 $0.10/百万、输出 $0.30/百万、缓存命中 $0.01/百万，以及音频 $0.0001/秒。配置的 `audio_input_token: 0` 从总输入中排除已上报的音频 token，音频本身由秒数计价，防止重复计费；若响应包含音频 token，必须能与文本部分拆分。OpenAI Chat 用量解析保留 `prompt_tokens_details.audio_seconds` / `usage.audio_seconds`，不会把音频 token 数换算为秒。真实 SN 响应返回音频 token 与无币种成本，但没有音频秒数。有效协议金额仍优先；需要本地结算时，复用已物化的 ASR 输入媒体信息测量时长，保留协议已有时长。按音频秒组合计价的纯文本/图片 LLM 请求明确记录零音频秒数。测量失败仍为未知，不将音频 token 换算为秒。

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

Gemini 的视觉输入费率显式配置为图片/视频输入 token，音频采用对应原厂费率，分档价格也保留各模态费率。生图按输入 token、文本/思考输出 token 与实际图片输出 token 组合结算，不再仅用按张折算值忽略其他费用。Embedding 2 保留官方 `usageMetadata.promptTokenDetails` 的模态计数；响应完全未返回 token 用量时维持未知。报价来源：[Gemini pricing](https://ai.google.dev/gemini-api/docs/pricing)、[Embeddings API](https://ai.google.dev/api/embeddings)。

Gemini 3.5 Transcribe 仅接收音频，官方输入价为 $2/百万 token、文本输出 $12/百万 token。输入总量与音频分项可能不完全相等；配置通用输入和音频输入使用相同单价，保留协议各项计数，避免剩余输入 token 导致整笔结算未知。来源：[模型输入类型](https://ai.google.dev/gemini-api/docs/models/gemini-3.5-transcribe)、[官方报价](https://ai.google.dev/gemini-api/docs/pricing#gemini-3.5-transcribe)。

## 计费用量

`AiUsage` 记录总/输入/输出 token、缓存命中/写入/1h 写入、推理 token、图片/音频/视频输入与输出 token、生成图片和参考图片张数、音频/视频秒数、字符数、请求单位，以及 `compute_seconds`、`megapixels`。

缓存、多模态、推理是总量的子集；1h 写入是缓存写入的子集，不能重复求和。处理耗时不是媒体长度。请求数不是 token。参考图片与生成图片分开统计。

- Gemini 媒体结果按实际成功图片产物填张数，并记录一次成功请求；音乐按请求单价计价。
- OpenAI Responses 图片也统计实际产物数；独立 Images 接口已有计数。转写优先使用 `usage.seconds`，只有缺失时才使用官方顶层 `duration` 后备。
- 同步 TTS 沿用统一字符数补全；豆包使用服务端 `text_words`。Qwen、豆包、MiniMax 原有图片计数和音视频时长逻辑保持。
- fal 的 `metrics.inference_time` 来自任务状态，作为计算秒合并到结果；DeepFilterNet 的 `audio_file.duration` 为音频秒数，不用 `timings.inference` 代替音频长度。来源：[Queue](https://fal.ai/docs/documentation/model-apis/inference/queue)、[DeepFilterNet schema](https://fal.ai/models/fal-ai/deepfilternet3/api)。
- fal 的 `fal-ai/video-upscaler` 按 $0.0008/百万像素收费，但公开 Output Schema 只有文件 URL、类型、名称和字节数，没有帧数、分辨率或完整的计费百万像素量。响应缺少 `megapixels` 时，先查询账单；权限不足则探测实际生成视频并补全累计百万像素，用冻结单价估算。测量失败保留未知成本并重试，不通过单帧宽高或请求目标分辨率虚构整段视频的计费像素。来源：[Video upscaler schema](https://fal.ai/models/fal-ai/video-upscaler/api)。

Robotics ER 2 Preview 补充当前公布的输入 $1/百万 token、输出 $5/百万 token、缓存命中 $0.10/百万 token。来源：[官方定价](https://ai.google.dev/gemini-api/docs/pricing#gemini-robotics-er-2)。

Gemini API 托管的两款 Gemma 4 按官方定价明确配置免费输入、输出及缓存，避免将免费模型显示为缺价格。

ESRGAN 与 rembg 的静态单价使用 2026-11-22 官方 Pricing API 返回的 $0.000575/计算秒，DeepFilterNet 为 $0.001/音频秒；这些账号查询报价作为默认估算依据，账单或协议有效费用仍优先。来源：[Pricing API](https://fal.ai/docs/platform-apis/v1/models/pricing)。

## fal 视频百万像素的核实办法

已确认本 endpoint 的公开单价为 $0.0008/百万像素，公开输出没有计费像素量。旁证如下，不能将其他 endpoint 的精确公式直接套过来：

- [fal 总计费说明](https://fal.ai/docs/documentation/model-apis/pricing) 说明 Model API 按产出计费。
- [fal 自己的模型比较](https://fal.ai/learn/tools/video-to-video-upscalers) 将 RealESRGAN 归为逐帧处理及按百万像素收费。
- [FlashVSR+](https://fal.ai/models/openimaginglab/flashvsr-plus) 明确写输出宽 × 高 × 帧数；[Crystal](https://fal.ai/models/clarityai/crystal-video-upscaler) 则按百万像素 × 秒 × FPS 分档，证明同平台也有不同公式。

优先只读查询已有任务对应的 [Billing Events](https://fal.ai/docs/platform-apis/v1/models/billing-events)（request_id）、[Usage](https://fal.ai/docs/platform-apis/v1/models/usage)（billed quantity、unit price、discount、cost_total），配合 [Pricing API](https://fal.ai/docs/platform-apis/v1/models/pricing) 确认具体 endpoint。Usage 可能是时间窗口内的聚合，需隔离该任务窗口或结合 billing event，不把整个窗口费用当作单次费用。

对已有输出实际探测宽、高及准确解码帧数，比较账单 quantity 与候选输入/输出累计像素量（宽 × 高 × 帧数 / 1e6）；不能只用 duration × fps 代替准确帧数。若历史样本不足，再安排受控的不同缩放倍率、帧数与 FPS 的短视频调用，检查最小计费量及取整，并向 fal 确认。Billing Events 权限不足时，按用户指定的备选方案，使用输出累计像素量与冻结单价作静态估算；估算来源必须明确，实际扣费与计费取整仍以 fal 账单为准。

## fal 账单费用优先与恢复

所有 fal profile、fal-queue 适配器的成功模型调用均持久化待查项，不限视频放大。已有效返回协议费用的调用直接采用协议费用，后台只关闭待查标记，不请求账单 API。其余调用通过连接配置 `operation_base_urls["billing.events"]` 指定的 Billing Events（fal Known Provider 默认 `https://api.fal.ai/v1`），以原始 request_id 和精确 endpoint_id 查询 `cost_total`（USD，折扣后金额），不以 `cost_subtotal` 或 quantity × 公示价代替实付金额。quantity/unit 可空；实际金额有效时仍能回填，已知的计费用量才补入缺失维度。

后台对网络中断、空账单、429 等持久化退避，尊重 Retry-After，不因进程退出丢失任务，不重新提交生成请求。查询窗口显式覆盖原任务日期，避免服务关机超过 24 小时后落入 API 默认窗口。凭证仅保存引用/指纹，查账时重新解析；模型从库存下线不会阻止已完成任务查账。

账单查询返回 401/403 或未配置账单端点时，持久化切换到本地结算；按视频像素量计费且缺少用量时，先读取制品头并调用现有 ffprobe 获取宽、高和真实帧数，头部不足时下载制品并解码计帧。下载有大小与时间上限，失败时保留待测量状态供重启恢复，不填零费用。测量证据、Usage、费用和执行缓存原子更新，详见 [延迟结算存储](deferred_billing.md)。

队列领取、短租约和状态保存在现有执行 RDB JSON。用 CAS 和事务同时替换同一 event_id 的用量费用、AICC 执行缓存及完成标记；崩溃后重查可幂等应用，不重复增加请求或扣配额。TaskMgr 完成结果不可变，保留完成时标记 pending 的快照；Usage 与 AICC 幂等重放返回更新后的费用。前端当前列表存在 pending 条目时，每 30 秒在页面可见期间刷新费用与汇总。

API 要求 BILLING/FULL 或对应账单读取权限；默认生成 Key 不一定具备这些权限。权限不足时切换到本地估算，测量中断可恢复，不将查询失败当成免费。持久结构、状态及扫描开销见 [延迟费用数据设计](deferred_billing.md)。

## 查询与 UI

`usage.query` 的 `total`、`grouped` 和 `buckets` 增加：

- `dimensions: { <AiUsage 数字字段>: { amount, reported_events } }`：每个实际上报维度的小计与覆盖记录数；明确上报的 0 保留，完全未上报的维度没有键。
- `unpriced_requests`：没有有效财务快照的记录数。

SQLite/PostgreSQL 在服务器上聚合已有 `usage_json`，无需新增持久列、迁移或为前端逐页取数据。成本按币种分别求和，未知成本不能作为零；空结果 `dimensions={}`、`unpriced_requests=0`。计费 JSON 数字与现有 top-level token 列须保持同源。

Usage 列表、筛选总计和分类汇总分别加载，使用相同权限边界、半开时间窗口和筛选条件。列表使用 `output_mode=page` 与 `limit/cursor`，只返回一页事件及匹配记录数，不执行计费维度聚合。筛选总计使用 `summary`，每个分类使用 `groups`，分别按 Provider、模型、App、API 类型聚合；分类通过 `group_limit/group_offset` 按需加载，不生成四维组合。翻页复用当前筛选汇总，筛选变化和刷新才重新查询。Usage 页面无需等待首页概览即可加载列表。

`groups` 不计算总计或时间桶，只返回分组；`summary` 保留完整总计。设置 `group_limit` 时，服务端先按总 token 降序、请求数降序、分组键升序选择分组，再聚合所选分组的所有币种和计费维度，避免在截断分组时丢失币种。`group_limit` 范围为 1–1000，`group_offset` 为跳过的完整分组数。筛选总计始终覆盖全部匹配记录，不受分组分页影响。

Usage 消费卡沿用首页卡片样式，显示当前筛选全部记录的 token 和多币种金额。首页今天、本月、近 30 天分别采用对应时间窗口的完整服务端聚合；首页和 Usage 页的总计、各分组、列表及近期记录的 token 可展开各单位明细。多币种金额默认展示 USD（没有 USD 时显示稳定排序的首个实际币种），点击展开各币种，绝不隐式换汇。首页分组维度与费用来自同一 30 天/身份范围的服务端汇总，不通过近期五条记录估算。账本保留明确零值，界面明细仅展示已上报且大于零的维度；覆盖不足时展示上报记录数，不将不同单位合并。未知费用显示 `--`，已知费用小计及未计价记录数仅在金额的悬停提示中展示。SDK `AiUsage` / `UsageAggregate` 同步扩展，Desktop 保留原始 `usage_json`。

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

首页统计采用渐进响应：近期记录独立分页；今天、本月、趋势、用户及各分类分别展示。模型和 Provider 分组分页不影响完整时间窗口总量，Provider 费用直接取 Provider 分组，不与模型分组重复相加。

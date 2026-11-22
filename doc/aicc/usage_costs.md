# AICenter 用量与成本（#755）

## 结算顺序

有效的响应 `usage.cost`（金额和币种）优先，其次是 `reported_cost` 加 Provider Rules 声明的币种，最后使用调用开始时冻结的价格和实际用量。非有限值、负金额与空币种不能作为有效成本。显式零单价表示免费，缺价格或缺用量表示未知，二者不同。

`usage.cost_source` 区分 `provider_reported` 与 `static_pricing`；`usage.cost_unknown_reason` 给出缺失用量、费率、未核实币种等原因。静态价格是原厂估算，不代表代理服务的实际扣款。异步结果优先采用最终结果用量，缺失字段由任务状态补充，不重复累加；视频时长后备只补服务端没有提供的字段。

SN Provider Rules 增加了 51 个当前映射模型的 USD 原厂价格及来源，沿用现有 catalog 修订发布机制。Kimi USD 依据原厂价格表及现场金额核对；SN 的 `reported_cost.model_ids` 将数值实报成本的默认 USD 声明限定到 `kimi-k3` / `kimi-k3.1-pro`，不外推至其他型号；其他 SN 型号返回无币种金额时回退静态估算，响应明确携带的其他币种优先保留。其他 Provider 不继承该声明。GPT 没有返回金额时使用冻结静态价格；缓存写入费率沿用原厂对应费率，不按输出 token 计费。

本次核验来源（2026-11-21）：

- OpenAI、Anthropic、Gemini、DeepSeek、MiniMax 的已有原厂 USD 配置及其 `source_url`。
- [Kimi USD](https://platform.kimi.ai/docs/pricing/chat)：K3 输入/输出/缓存命中为 $3/$15/$0.30 每百万 token；K3.1 Pro 为 $2/$10/$0.20；缓存写入按其 TTL。
- [Z.AI](https://docs.z.ai/guides/overview/pricing)：GLM-5、5.3、Flash、FlashX 的原厂 USD 价格。
- [Alibaba Cloud](https://www.alibabacloud.com/help/en/model-studio/model-pricing/)：所选公开区域/部署范围的 USD 价格。范围选择固定在配置中，不随 SN 实际后端地区猜测；不同地区和活动价可能不同。
- [xAI](https://docs.x.ai/developers/pricing)：Grok 4.7 短/长上下文费率。

三个 SN 映射尚未配置静态价格，原因不同：`ui-tars-1.5-7b`、`qwen3-embedding-8b` 未核实到与该具体原厂 offering 相对应的公开 USD 单价；不能用其他托管商价格或 `text-embedding-v4` 等不同 API 型号代替。`voxtral-small-24b-2507` 有[官方报价](https://docs.mistral.ai/models/voxtral-small-25-07)：音频 $0.004/分钟，文本输入/输出 $0.10/$0.40 每百万 token（2026-11-22 核对）。后者缺口是音频时长与文本 token 的组合计费，以及 SN 音频用量/型号映射的核实，不是原厂没有报价。当前静态结算 basis 只能选择 token 或单位，不能完整表达这一组合；仅套文本单价会漏算音频。开源权重不意味着托管推理免费，不以汇率换算、第三方价格或相邻型号补缺。上述模型仍可使用有效响应成本，后续有可核实原厂报价时更新 metadata。

Gemini Omni 1.1 明确区分图片/视频输入 token 与视频输出 token：文本、图片、视频输入 $1.50/百万，文本输出 $9/百万，视频输出 $17.50/百万。2026-12-03 后官方将其重定向至 1.2 并更换视频输出费率，需要届时更新 catalog，不能把未来费率用于历史调用。来源：[Gemini pricing](https://ai.google.dev/gemini-api/docs/pricing)。视频响应缺少模态拆分时保留未知，不能将视频输出全部按文本价格结算。

## 计费用量

`AiUsage` 记录总/输入/输出 token、缓存命中/写入/1h 写入、推理 token、图片/音频/视频输入与输出 token、生成图片和参考图片张数、音频/视频秒数、字符数、请求单位，以及 `compute_seconds`、`megapixels`。

缓存、多模态、推理是总量的子集；1h 写入是缓存写入的子集，不能重复求和。处理耗时不是媒体长度。请求数不是 token。参考图片与生成图片分开统计。

- Gemini 媒体结果按实际成功图片产物填张数，并记录一次成功请求；音乐按请求单价计价。
- OpenAI Responses 图片也统计实际产物数；独立 Images 接口已有计数。转写优先使用 `usage.seconds`，只有缺失时才使用官方顶层 `duration` 后备。
- 同步 TTS 沿用统一字符数补全；豆包使用服务端 `text_words`。Qwen、豆包、MiniMax 原有图片计数和音视频时长逻辑保持。
- fal 的 `metrics.inference_time` 来自任务状态，作为计算秒合并到结果；DeepFilterNet 的 `audio_file.duration` 为音频秒数，不用 `timings.inference` 代替音频长度。来源：[Queue](https://fal.ai/docs/documentation/model-apis/inference/queue)、[DeepFilterNet schema](https://fal.ai/models/fal-ai/deepfilternet3/api)。
- fal 的 `fal-ai/video-upscaler` 按 $0.0008/百万像素收费，但公开 Output Schema 只有文件 URL、类型、名称和字节数，没有帧数、分辨率或完整的计费百万像素量。当前适配器也没有下载视频探测帧数并重建计费用量；尚未用真实任务响应确认是否有额外字段。结算器已支持 `megapixels`，但该响应仍显示 `missing_usage:megapixel`，不通过单帧宽高或请求目标分辨率虚构整段视频的计费像素。来源：[Video upscaler schema](https://fal.ai/models/fal-ai/video-upscaler/api)。

## 查询与 UI

`usage.query` 的 `total`、`grouped` 和 `buckets` 增加：

- `dimensions: { <AiUsage 数字字段>: { amount, reported_events } }`：每个实际上报维度的小计与覆盖记录数；明确上报的 0 保留，完全未上报的维度没有键。
- `unpriced_requests`：没有有效财务快照的记录数。

SQLite/PostgreSQL 在服务器上聚合已有 `usage_json`，无需新增持久列、迁移或为前端逐页取数据。成本按币种分别求和，未知成本不能作为零；空结果 `dimensions={}`、`unpriced_requests=0`。计费 JSON 数字与现有 top-level token 列须保持同源。

Usage 仍使用同一权限边界、时间窗口及筛选条件，通过一次 `summary_and_events` 查询获取完整汇总和一页记录；group_by 同时使用 Provider、模型、App、API 类型。分页游标只限制 events，总计与分组不受游标影响。对分组组合的返回量随实际分组组合数增长，未在本次验证百万级高基数结果。

Usage 消费卡沿用首页卡片样式，显示当前筛选全部记录的 token 和多币种金额。总计、各分组和单条记录的 token 可展开各单位明细。隐藏未上报维度、保留明确零值，覆盖不足时展示上报记录数；不将不同单位合并。SDK `AiUsage` / `UsageAggregate` 同步扩展，Desktop 保留原始 `usage_json`。

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

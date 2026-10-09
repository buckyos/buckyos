# Driver metadata 核对记录（更新至 2026-10-08）

本次盘点最初覆盖 `src/frame/aicc/driver_metadata` 的 38 份文件；豆包普通账号与 Agent Plan 拆分后为 39 份：12 份 model-driver、14 份 provider rules、13 份 known-provider。两个豆包 profile 合并在同一份 known-provider catalog 中，账号专属规则仍分别保存。长度能力由 model-driver 声明，并进入各渠道的有效库存；不在每个 Provider 重复填写。

2026-09-26 首轮核对重点是当时目录中 93 个未排除的 LLM 条目。2026-10-08 按各厂商当前官方文档重新复核现有目录中的 117 个未排除 LLM 条目，补齐 Kimi K3、MiniMax M3 和 8 个 Doubao 条目的长度限制，并修正 Doubao Seed 2.1 Turbo 的上下文。当前仍有 12 个条目至少缺少一个长度字段，统一列在下文；这些字段没有逐型号官方依据，继续保持 unknown。

## 取值原则

- 优先采用具体型号的官方规格页、参数上限表和官方模型配置中的明确整数，不能把请求默认值、示例参数、评测生成长度当作硬上限。
- 不统一把所有厂商的 `K` 乘以 1024。例如 Claude 的 128K 输出是 128,000；Gemini 明确写 65,536；GLM 的参数表明确写 131,072。
- 厂商只给标称 K、没有查到精确整数时，使用十进制保守预算，并明确记录这一取值决策。它不是厂商对精确整数边界的承诺。
- `max_output_tokens` 是单次输出预算限制，不能加上上下文长度，也不能额外累加独立的思维链预算。路由仍检查输入加输出是否超过 `max_context_tokens`。
- 保留模型身份、规格、路由权重和规格引用的 `effort`。`default_effort` 表示家族直选的默认档位，与规格引用的预设分开核对。
- 不因为原厂下架模型就删除静态家族；其他 Provider 或本地部署仍可能提供它。Provider 实时可用性继续由 discovery 决定。

## OpenAI

| 模型 | 上下文 | 输出上限 | 官方来源 |
| --- | ---: | ---: | --- |
| gpt-5.4 | 1,050,000 | 128,000 | [模型规格](https://developers.openai.com/api/docs/models/gpt-5.4) |
| gpt-5.5 | 1,050,000 | 128,000 | [模型规格](https://developers.openai.com/api/docs/models/gpt-5.5) |
| gpt-5.4-pro | 1,050,000 | 128,000 | [模型规格](https://developers.openai.com/api/docs/models/gpt-5.4-pro) |
| gpt-5.5-pro | 1,050,000 | 128,000 | [模型规格](https://developers.openai.com/api/docs/models/gpt-5.5-pro) |
| gpt-5.4-mini | 400,000 | 128,000 | [模型规格](https://developers.openai.com/api/docs/models/gpt-5.4-mini) |
| gpt-5.4-nano | 400,000 | 128,000 | [模型规格](https://developers.openai.com/api/docs/models/gpt-5.4-nano) |
| gpt-5.3-codex | 400,000 | 128,000 | [模型规格](https://developers.openai.com/api/docs/models/gpt-5.3-codex) |
| gpt-4o-mini-transcribe（ASR） | 16,000 | 2,000 | [模型规格](https://developers.openai.com/api/docs/models/gpt-4o-mini-transcribe) |

保留 GPT-5.6、5.6 Luna/Terra/Sol、6 Astra 已正确声明的 1,050,000 / 128,000。来源：[5.6](https://developers.openai.com/api/docs/models/gpt-5.6)、[Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna)、[Terra](https://developers.openai.com/api/docs/models/gpt-5.6-terra)、[Sol](https://developers.openai.com/api/docs/models/gpt-5.6-sol)、[Astra](https://developers.openai.com/api/docs/models/gpt-6-astra)。

同时按上述型号页面修正：

- GPT-5.4 Pro 支持 streaming；GPT-5.5 Pro 支持 structured outputs，对应 `json_schema`。
- GPT-5.4 和 5.4 mini 的 `default_effort` 为 `none`；5.4 Pro 为 `medium`。
- GPT-5.6、Luna、Terra、Sol 的 `default_effort` 为 `medium`。不调整它们各自规格引用的 none/low/high 预设。

## Claude

12 个现有模型的输出长度原先都按二进制 K 填写，现按官方十进制限制修正：

| 模型组 | 上下文（保持） | 输出上限 |
| --- | ---: | ---: |
| Fable 5.1 / 5，Opus 5.5 / 5 / 4.8 / 4.7 / 4.6，Sonnet 5 / 4.6 | 1,000,000 | 128,000 |
| Opus 4.5、Sonnet 4.5、Haiku 4.5 的现有日期快照 | 200,000 | 64,000 |

来源：[当前模型表](https://platform.claude.com/docs/en/models/overview)、[Fable 5](https://platform.claude.com/docs/en/models/fable-5/overview)、[Opus 5](https://platform.claude.com/docs/en/models/opus-5/overview)、[Opus 4.8](https://platform.claude.com/docs/en/models/opus-4-8/overview)、[Opus 4.7](https://platform.claude.com/docs/en/models/opus-4-7/overview)、[Opus 4.6](https://platform.claude.com/docs/en/models/opus-4-6/overview)、[Sonnet 4.6](https://platform.claude.com/docs/en/models/sonnet-4-6/overview)、[Opus 4.5](https://platform.claude.com/docs/en/models/opus-4-5/overview)、[Sonnet 4.5](https://platform.claude.com/docs/en/models/sonnet-4-5/overview)、[Haiku 4.5](https://platform.claude.com/docs/en/models/haiku-4-5/overview)。[官方参数建议](https://platform.claude.com/docs/en/about-claude/models/optimizing-for-cost-and-intelligence)明确使用 128,000 这个整数上限。

Opus 5.5 的默认 effort 按模型表改为 `medium`。Batch API 专用的 300K beta 输出不写入普通 LLM 调用能力。

## Gemini

补齐以下模型：上下文 1,048,576、输出 65,536。其他现有 LLM 的这两个字段保持。

- [Gemini 3.7 Flash](https://ai.google.dev/gemini-api/docs/models/gemini-3.7-flash)
- [Gemini 3.5 Flash Lite](https://ai.google.dev/gemini-api/docs/models/gemini-3.5-flash-lite)
- [Gemini 3 Flash Preview](https://ai.google.dev/gemini-api/docs/models/gemini-3-flash-preview)

Google 将第一个数称为 input token limit；AICC 当前用 `max_context_tokens` 对总预算做保守路由检查。本次不改变这层语义。非 LLM 的图片、TTS、embedding 条目不强制添加 LLM 输出限制。

## Qwen

18 个 LLM 条目均补齐输出限制，三个开源型号同时补齐上下文。以具体型号页面为准，不套用同系列的统一默认值。

| 模型 | 上下文 | 输出上限 | 官方来源 |
| --- | ---: | ---: | --- |
| qwen3.8-max / flash | 1,000,000 | 131,072 | [Max](https://help.aliyun.com/zh/model-studio/qwen3-8-max)、[Flash](https://help.aliyun.com/zh/model-studio/qwen3-8-flash) |
| qwen3.7-max / plus / flash | 1,000,000 | 131,072 | [Max](https://help.aliyun.com/zh/model-studio/qwen3-7-max)、[Plus](https://help.aliyun.com/zh/model-studio/qwen3-7-plus)、[Flash](https://help.aliyun.com/zh/model-studio/qwen3-7-flash) |
| qwen3.6-plus / flash | 1,000,000 | 65,536 | [Plus](https://help.aliyun.com/zh/model-studio/qwen3-6-plus)、[Flash](https://help.aliyun.com/zh/model-studio/qwen3-6-flash) |
| qwen3.5-plus / flash | 1,000,000 | 65,536 | [Plus](https://help.aliyun.com/zh/model-studio/qwen3-5-plus)、[Flash](https://help.aliyun.com/zh/model-studio/qwen3-5-flash) |
| qwen3.8-2.4t-a95b / 27b | 1,000,000 | 131,072 | [2.4T](https://help.aliyun.com/zh/model-studio/qwen3-8-2-4t-a95b)、[27B](https://help.aliyun.com/zh/model-studio/qwen3-8-27b) |
| qwen3.6-35b-a3b | 262,144 | 65,536 | [型号规格](https://help.aliyun.com/zh/model-studio/qwen3-6-35b-a3b) |
| qwen3.5-397b-a17b / 122b-a10b / 35b-a3b / 27b | 262,144 | 65,536 | [397B](https://help.aliyun.com/zh/model-studio/qwen3-5-397b-a17b)、[122B](https://help.aliyun.com/zh/model-studio/qwen3-5-122b-a10b)、[35B](https://help.aliyun.com/zh/model-studio/qwen3-5-35b-a3b)、[27B](https://help.aliyun.com/zh/model-studio/qwen3-5-27b) |
| qwen3.6-max-preview | 262,144 | 65,536 | [型号规格](https://help.aliyun.com/zh/model-studio/qwen3-6-max) |
| qwen3-max | 262,144 | 32,768 | [型号规格](https://help.aliyun.com/zh/model-studio/model-qwen3-max) |

Qwen3 Max 的非思考输出上限是 65,536，思考模式是 32,768；当前该条目的 `supported_efforts` 只有 `thinking`，因此使用后者。独立的思维链长度不与输出上限相加。开源型号采用官方托管规格；本地部署仍须按实际服务能力缩窄库存。

## GLM

优先使用[参数说明的逐型号整数表](https://docs.bigmodel.cn/cn/guide/start/concept-param#max_tokens)，并与[模型总表](https://docs.bigmodel.cn/cn/guide/start/model-overview)交叉核对。

| 模型 | 上下文 | 输出上限 | 处理 |
| --- | ---: | ---: | --- |
| glm-5.1 / glm-5 / glm-4.7 / glm-4.6 / glm-4.7-flash | 202,752 | 131,072 | 上下文采用官方模型配置整数，补齐缺失字段 |
| glm-5-turbo / glm-4.7-flashx / glm-5v-turbo | 200,000 | 131,072 | 官方只发布 200K 上下文，采用十进制保守预算 |
| glm-4-long | 1,048,576 | 4,096 | 补输出 |
| glm-4-flashx-250414 | 131,072 | 16,384 | 补输出 |
| glm-4-flash-250414 | 131,072 | 32,768 | 按参数表补输出 |
| glm-4.1v-thinking-flash | 65,536 | 32,768 | 按参数表修正输出 |
| codegeex-4 | 131,072 | 32,768 | 按总表补齐 |
| glm-4-32b-0414-128k | 131,072 | 未确认 | 补齐该扩展上下文型号的上下文，输出留空 |

202,752 的来源是 Z.ai 官方发布的模型配置：[GLM-5.1](https://huggingface.co/zai-org/GLM-5.1/blob/main/config.json)、[GLM-5](https://huggingface.co/zai-org/GLM-5/blob/main/config.json)、[GLM-4.7](https://huggingface.co/zai-org/GLM-4.7/blob/main/config.json)、[GLM-4.6](https://huggingface.co/zai-org/GLM-4.6/blob/main/config.json)、[GLM-4.7-Flash](https://huggingface.co/zai-org/GLM-4.7-Flash/blob/main/config.json)。不能把这些型号的 200K 简单换算成 204,800。

参数表和总表对 `glm-4-flash-250414`、`glm-4.1v-thinking-flash` 的输出存在差异，本次使用参数表的明确最大 `max_tokens=32768`。GLM-4-32B-0414 原生窗口为 32K，官方说明可通过 YaRN 扩展到 128K；当前条目 ID 明确为 `-128k`，仅为该条目登记扩展窗口，见[官方部署说明](https://github.com/zai-org/GLM-4/blob/main/README_zh.md)。

## DeepSeek、Doubao、MiniMax

- DeepSeek V4 Flash / Pro：上下文由 1,000,000 修正为 1,048,576，输出保持 393,216。来源：[Flash 官方配置](https://huggingface.co/deepseek-ai/DeepSeek-V4-Flash/blob/main/config.json)、[Pro 官方配置](https://huggingface.co/deepseek-ai/DeepSeek-V4-Pro/blob/main/config.json)、[官方 Models API](https://api-docs.deepseek.com/api/list-models/)。新 `deepseek-flash` 别名已涉及 V4.1，不借此把现有 V4 家族改成新型号。Vision experimental 条目的现有长度未取得额外证据，保持原值，不外推其他版本的配置。
- Doubao [官方模型表](https://docs.volcengine.com/docs/82379/1747867?lang=zh)现已明确 Seed 2.1、Character、Translation 和 Seed 2.0 Code Preview 的长度：Seed 2.1 Lite/Pro/Evolving 使用 1,048,576 / 262,144；Seed 2.1 Turbo 及 `-260628` 使用 262,144 / 262,144；Character 使用 128,000 / 32,000；Translation 使用 4,000 / 3,000；Seed 2.0 Code Preview 使用 256,000 / 128,000。`doubao-seed-2-1-pro-260628` 只有 256K 上下文，通过 Provider Rules 将通用 Pro 的 1M 能力收窄到 262,144。默认回答 4K 和最大思维链不替代或累加到输出上限。
- MiniMax M2 / M2.1 / M2.5 / M2.7 及现有 highspeed 型号：保留 204,800 / 204,800。[Anthropic 兼容接口](https://platform.minimax.io/docs/api-reference/text-anthropic-api)给出各型号上下文；[OpenAI 兼容接口](https://platform.minimax.io/docs/api-reference/text-chat-openai)说明这些型号最大输出 204,800。65,536 是推荐请求值，不能改成模型硬上限。
- MiniMax M3：补入 1,000,000 上下文和 524,288 最大输出。官方接口文档把 131,072 标为推荐值、524,288 标为 M3 硬上限，不能把推荐值写成 `max_output_tokens`。

## Kimi（2026-10-08 复核）

- Kimi K3：[官方定价页](https://platform.kimi.com/docs/pricing/chat)明确上下文为 1,048,576；[K3 模型页](https://platform.kimi.com/docs/guide/kimi-k3-quickstart)明确 `max_completion_tokens` 默认 131,072、最大 1,048,576。因此模型能力配置为 1,048,576 / 1,048,576，不能把默认值写成硬上限。
- Kimi K2.6、K2.7 Code 和 K2.7 Code Highspeed 的官方页仍只明确 256K 上下文，没有发布逐型号最大 completion 整数，输出上限继续缺省。
- Kimi model-driver 描述 AICC 已知的型号与能力，但 Provider 有效库存只采用当前 API Key 的 `/models` 返回值。公开文档和 model-driver 都不能把 `kimi-k3`、`kimi-k2.7-code-highspeed` 静态注入没有访问权限的账号。

## 仍未确认的字段

| 模型 | 缺失字段 | 原因 |
| --- | --- | --- |
| qwen2-5-72b-20240919、qwen3-0-6b-20250429、qwen3-8b-20250429、qwen3-14b-20250429、qwen3-32b-20250429 | max_context_tokens、max_output_tokens | 火山方舟普通账号 `/api/v3/models` 确认这些聚合渠道型号当前可见，但目录不返回 token 上限；不把其他渠道或相近 Qwen 型号的限制外推到这些精确 ID |
| kimi-k2.6、kimi-k2.7-code、kimi-k2.7-code-highspeed | max_output_tokens | 当前官方模型页只明确 256K 上下文，未给出这些型号的明确 API 输出硬上限 |
| kimi-k2.8-preview | max_context_tokens、max_output_tokens | Agent Plan 清单已列出该预览型号，但当前 Kimi 公共模型文档尚无可核实的硬限制 |
| charglm-4、emohaa | max_context_tokens、max_output_tokens | 当前官方型号表和 API 参数表未给出可确认的型号级限制 |
| glm-4-32b-0414-128k | max_output_tokens | 官方开源卡和部署说明给出上下文，未给出这个托管 ID 的独立输出上限 |

Kimi 来源：[K2.6 快速开始](https://platform.kimi.com/docs/guide/kimi-k2-6-quickstart)、[当前模型文档](https://platform.kimi.com/docs/models)、[当前 API 文档](https://platform.kimi.com/docs/api/chat)。新 K3 的默认值和上限不能回填给 K2.x；Agent Plan 新增型号也不能据名称外推硬限制。

这些未知值继续缺省。有明确输出预算的请求仍会按现有路由规则过滤缺少上限的模型；本次不把未知值解释为无限容量。

本轮还复核了 OpenAI、Claude、Gemini、Qwen、GLM 和 DeepSeek 的现有长度字段。OpenAI 官方模型页仍为当前 GPT 条目所用的 1,050,000/400,000 上下文与 128,000 输出；Claude、Gemini、GLM、DeepSeek 的现值与各自官方模型表和参数表一致。Qwen Cloud 与阿里云百炼个别型号的限制不同，当前 `qwen` Provider 是阿里云百炼渠道，因此继续使用百炼逐型号页面的整数，不用 Qwen Cloud 的跨渠道表覆盖。

## 版本与验证

2026-10-08 修改的 Kimi、MiniMax、Doubao model-driver 分别升为 revision 5、7、10，Doubao Provider Rules 升为 13，内置目录版本升为 14。用户本地、system-config 或云端的整文件覆盖仍遵循既有优先级，不会被这次修改重写。

回归测试覆盖：

- GPT-5.4/5.5、Pro、mini/nano、5.3 Codex 在原厂及 OpenRouter 库存中，请求输出 8,192 和 128,000 时保留 GPT 的高优先级，超过 128,000 时拒绝。
- 输入加输出恰好达到各型号上下文上限时允许，超出一个 token 时拒绝。
- 所有有效 LLM 的长度必须是正整数，输出不能超过上下文；缺失字段集合必须与上表记录一致。

首轮验证结果：`cargo test -p aicc` 当时的 515 个测试全部通过，`cargo check -p aicc --all-targets`、`cargo fmt -p aicc -- --check` 和 `git diff --check` 均通过。这些验证使用本地 fixtures，不产生真实模型调用；首轮未部署或重启运行中的 AICC。后续豆包 T2 补齐另行执行了构建、部署、真实调用与高保真 MockProvider 回归，以对应测试报告为准。

2026-10-08 本轮复核新增了逐型号硬限制断言；`cargo test -p aicc --lib -- --test-threads=1` 的 605 个测试全部通过，`cargo check -p aicc --all-targets`、`cargo fmt -p aicc -- --check` 和 `git diff --check` 均通过。本轮同样未部署、未重启服务，也未发起真实模型调用。

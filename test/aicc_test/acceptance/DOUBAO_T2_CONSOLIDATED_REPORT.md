# 豆包 T2 模型测试整合报告

测试周期：2026-09-25 至 2026-09-30

本文档按 `case_id` 合并 Agent Plan 与标准 ModelArk 的多轮 T2 结果；同一用例重测时以最新结果为准，原始报告保留在执行历史中供审计。

## 状态定义

| 分类 | 含义 |
|---|---|
| 完整通过 | 该物理模型所有已执行 API 单元均通过自动断言。 |
| 部分通过 | Provider 协议、规范化映射、计量和制品验证成功，但媒体/视觉/音频语义仍需 Judge 或人工复核；不属于协议失败。 |
| Provider 受限 | 已发送真实请求，但 Provider 因账号、套餐、开通状态或模型不可用而拒绝。 |
| 失败 | 最新重测后仍有产品、协议或断言失败。 |
| 未启动 | 在产生计费调用前有意排除。 |

## 最终概览

| 账号类型 | 选中单元 | 自动通过 | 协议/制品通过，待语义复核 | Provider 受限 | 失败 |
|---|---:|---:|---:|---:|---:|
| 标准 ModelArk | 47 | 16 | 22 | 9 | 0 |
| Agent Plan | 41 | 14 | 21 | 6 | 0 |
| 合计 | 88 | 30 | 43 | 15 | 0 |

按物理模型统计，标准 ModelArk 为 9 个完整通过、11 个部分通过、7 个 Provider 受限、0 个失败；Agent Plan 为 6 个完整通过、11 个部分通过、2 个 Provider 受限、0 个失败。

## 标准 ModelArk 的 135 个模型库存对账

经过鉴权的 `/api/v3/models` 快照包含 135 个 Provider 模型 ID。它是平台目录结果，不是账号已授权模型数，也不是 T2 测试模型数。

| 分类 | 数量 | 原因 |
|---|---:|---|
| AICC 无法匹配 | 89 | Provider 返回了 ID，但 Provider 覆盖配置和 AICC 模型目录都无法解析出 Model Driver 物理身份，未进入可路由库存。 |
| Provider 规则显式排除 | 5 | 命中非物理模型或当前不支持模态的明确排除规则。 |
| 已匹配但不可路由 | 10 | 已解析物理身份，但 Provider 标记为 `Retiring` 或 `Shutdown`，被规范化为不可用/已弃用。 |
| 可路由但明确未启动 | 4 | 用户在计费调用前明确排除四个 Seedance 2.x 模型族。 |
| 已执行 T2 | 27 | 至少产生一个选中的 T2 API 单元。 |
| **合计** | **135** | `89 + 5 + 10 + 4 + 27` |

Add Provider 向导显示的 41 是内部已匹配库存，即 `10 + 4 + 27`，不是 API Key 已被证明可调用的模型数。真正的可路由库存有 31 个物理模型，其中 27 个已执行、4 个被明确暂缓。

### 89 个无法匹配的目录模型

这些条目的对账原因均为 `no_match`：`InventoryBuilder` 既找不到精确的 `model_driver_overrides` 目标，也找不到有效的 Model Driver 目录身份。通配符操作规则不能创建模型身份。除非另有官方证据证明模型已退役，否则它们属于 AICC 模型目录或渠道覆盖缺口，不能算作账号权限限制。

| 模型族 | 数量 | 无法匹配的具体原因 |
|---|---:|---|
| DeepSeek R1/V3 | 9 | 标准豆包渠道只配置了本轮选中 DeepSeek V4 的映射，缺少这些 R1、蒸馏 R1 和 V3 快照的物理身份/覆盖映射。 |
| Doubao 1.5 | 14 | 旧版文本、思考、视觉、角色及 UI-TARS 快照没有当前 AICC 物理身份。 |
| 旧版豆包文本向量模型 | 4 | 当前只覆盖本轮使用的多模态向量身份，缺少这些文本/大文本向量快照身份。 |
| 旧版豆包 Lite/Pro/Vision | 35 | Seed 系列之前的 Lite、Pro、联网、角色、函数调用和视觉快照没有 Model Driver 身份或渠道覆盖映射。 |
| 豆包 Seaweed/Seed 缺口 | 13 | 当前豆包 Model Driver 未定义或映射这些 Seed 1.6/1.8、Seed 2.0 Pro、旧 Code Preview 和 Seaweed ID。 |
| 豆包图像/视频缺口 | 7 | 旧版 Seedance、Seededit、Seedream 快照未映射到当前物理媒体模型身份。 |
| Kimi | 3 | 标准 Ark 渠道缺少这些 Kimi ID 的物理模型映射；Agent Plan 的其他 Kimi 映射不适用。 |
| Mistral | 1 | 当前 AICC 目录没有匹配的 Mistral 物理身份。 |
| Wan 视频 | 3 | 当前 AICC 目录没有匹配的 Wan 视频 Model Driver 身份。 |
| **合计** | **89** | |

完整 ID 清单：

- **DeepSeek R1/V3（9）：** `deepseek-r1-250120`、`deepseek-r1-250528`、`deepseek-r1-distill-qwen-32b-250120`、`deepseek-r1-distill-qwen-7b-250120`、`deepseek-v3-1-250821`、`deepseek-v3-1-terminus`、`deepseek-v3-2-251201`、`deepseek-v3-241226`、`deepseek-v3-250324`。
- **Doubao 1.5 连字符 ID（11）：** `doubao-1-5-lite-32k-250115`、`doubao-1-5-pro-256k-250115`、`doubao-1-5-pro-32k-250115`、`doubao-1-5-pro-32k-character-250228`、`doubao-1-5-pro-32k-character-250715`、`doubao-1-5-thinking-pro-250415`、`doubao-1-5-thinking-pro-m-250415`、`doubao-1-5-thinking-pro-m-250428`、`doubao-1-5-thinking-vision-pro-250428`、`doubao-1-5-ui-tars-250428`、`doubao-1-5-vision-pro-32k-250115`。
- **Doubao 1.5 点号 ID（3）：** `doubao-1.5-ui-tars-250328`、`doubao-1.5-vision-lite-250315`、`doubao-1.5-vision-pro-250328`。
- **旧版文本向量模型（4）：** `doubao-embedding-large-text-240915`、`doubao-embedding-large-text-250515`、`doubao-embedding-text-240515`、`doubao-embedding-text-240715`。
- **旧版 Lite/Pro/Vision（35）：** `doubao-lite-128k-240428`、`doubao-lite-128k-240828`、`doubao-lite-32k-240428`、`doubao-lite-32k-240628`、`doubao-lite-32k-240828`、`doubao-lite-32k-character-241015`、`doubao-lite-32k-character-250228`、`doubao-lite-4k-240328`、`doubao-lite-4k-character-240515`、`doubao-lite-4k-character-240828`、`doubao-lite-4k-pretrain-character-240516`、`doubao-pro-128k-240515`、`doubao-pro-128k-240628`、`doubao-pro-256k-241115`、`doubao-pro-32k-240615`、`doubao-pro-32k-240828`、`doubao-pro-32k-241215`、`doubao-pro-32k-browsing-240615`、`doubao-pro-32k-browsing-240828`、`doubao-pro-32k-browsing-241115`、`doubao-pro-32k-character-240528`、`doubao-pro-32k-character-240828`、`doubao-pro-32k-character-241215`、`doubao-pro-32k-functioncall-240515`、`doubao-pro-32k-functioncall-240815`、`doubao-pro-32k-functioncall-241028`、`doubao-pro-32k-functioncall-preview`、`doubao-pro-4k-240515`、`doubao-pro-4k-browsing-240524`、`doubao-pro-4k-character-240515`、`doubao-pro-4k-character-240728`、`doubao-pro-4k-functioncall-240515`、`doubao-pro-4k-functioncall-240615`、`doubao-vision-lite-32k-241015`、`doubao-vision-pro-32k-241028`。
- **Seaweed/Seed 缺口（13）：** `doubao-seaweed-241128`、`doubao-seed-1-6-250615`、`doubao-seed-1-6-251015`、`doubao-seed-1-6-flash-250615`、`doubao-seed-1-6-flash-250715`、`doubao-seed-1-6-flash-250828`、`doubao-seed-1-6-lite-251015`、`doubao-seed-1-6-thinking-250615`、`doubao-seed-1-6-thinking-250715`、`doubao-seed-1-6-vision-250815`、`doubao-seed-1-8-251228`、`doubao-seed-2-0-pro-260215`、`doubao-seed-code-preview-251028`。
- **图像/视频缺口（7）：** `doubao-seedance-1-0-lite-i2v-250428`、`doubao-seedance-1-0-lite-t2v-250428`、`doubao-seedance-1-0-pro-250528`、`doubao-seedance-1-5-pro-251215`、`doubao-seededit-3-0-i2i-250628`、`doubao-seedream-3-0-t2i-250415`、`doubao-seedream-4-5-251128`。
- **Kimi（3）：** `kimi-k2-250711`、`kimi-k2-250905`、`kimi-k2-thinking-251104`。
- **Mistral（1）：** `mistral-7b-instruct-v0.2`。
- **Wan 视频（3）：** `wan2-1-14b-flf2v-250417`、`wan2-1-14b-i2v-250225`、`wan2-1-14b-t2v-250225`。

### 5 个被显式排除的目录模型

| Provider 模型 ID | 规则 | 原因 |
|---|---|---|
| `doubao-smart-router-250928` | `doubao-smart-router-*` | Router/选择器，不是稳定物理模型；T2 只覆盖物理模型。 |
| `doubao-seed3d-1-0-250928` | `*3d-*` | 当前 AICC 没有 3D 生成规范 API 类型和 Provider 操作。 |
| `doubao-seed3d-2-0-260328` | `*3d-*` | 同属当前不支持的 3D 模态。 |
| `hitem3d-2-0-251223` | `*3d-*` | 同属当前不支持的 3D 模态。 |
| `hyper3d-gen2-260112` | `*3d-*` | 同属当前不支持的 3D 模态。 |

这些排除由 `doubao.provider.json` 配置驱动，不是运行时代码中的模型名分支。

### 10 个已匹配但不可路由的目录模型

这些 ID 已解析为有效物理身份，但 Provider 目录将其标记为 `Retiring` 或 `Shutdown`。发现流程将两种状态规范化为 `availability=unavailable`、`deprecated=true`，公开库存随后在路由和 T2 规划前过滤它们。

| Provider 模型 ID | 结果 |
|---|---|
| `deepseek-v4-flash-260425` | 已匹配；Provider 生命周期状态不可用/已弃用。 |
| `deepseek-v4-pro-260425` | 已匹配；Provider 生命周期状态不可用/已弃用。 |
| `doubao-embedding-vision-241215` | 已匹配；Provider 生命周期状态不可用/已弃用。 |
| `doubao-embedding-vision-250328` | 已匹配；Provider 生命周期状态不可用/已弃用。 |
| `doubao-embedding-vision-250615` | 已匹配；Provider 生命周期状态不可用/已弃用。 |
| `doubao-seed-2-0-lite-260215` | 已匹配；Provider 生命周期状态不可用/已弃用。 |
| `doubao-seed-2-0-mini-260215` | 已匹配；Provider 生命周期状态不可用/已弃用。 |
| `doubao-seedream-4-0-250828` | 已匹配；Provider 生命周期状态不可用/已弃用。 |
| `doubao-seedream-5-0-260128` | 已匹配；Provider 生命周期状态不可用/已弃用。 |
| `glm-4-7-251222` | 已匹配；Provider 生命周期状态不可用/已弃用。 |

规范化验收快照没有保留每个条目究竟属于 `Retiring` 还是 `Shutdown`，因此不虚构逐 ID 子状态。

### 4 个可路由但明确未启动的模型

| Provider 模型 ID | 原因 |
|---|---|
| `doubao-seedance-2-0-260128` | 对应明确排除的 `doubao-seedance-2.0` 模型族。 |
| `doubao-seedance-2-0-fast-260128` | 对应明确排除的 `doubao-seedance-2.0-fast` 模型族。 |
| `doubao-seedance-2-0-mini-260615` | 对应明确排除的 `doubao-seedance-2.0-mini` 模型族。 |
| `doubao-seedance-2-5-260628` | 对应明确排除的 `doubao-seedance-2.5` 模型族。 |

它们都存在于可路由库存中；未执行属于明确的测试范围排除，不是发现、匹配、协议或 Provider 失败。

### 覆盖结论

标准账号 T2 完整覆盖了选中的 27 个物理模型，但没有覆盖 Provider 返回的全部 135 个 ID。只有 5 个规则排除、10 个生命周期排除和 4 个用户授权排除具备不执行计费 T2 的证据。89 个 `no_match` 是 AICC 覆盖缺口，后续必须二选一处理：

1. 增加 Model Driver 身份和 Provider 映射，再派生并执行对应的 `ProviderInstance x model x API-Type` 单元；
2. 依据官方生命周期、逻辑别名、非物理模型或不支持规范协议的证据，增加显式覆盖/排除规则。

因此，当前结论只能是“已测试标准豆包账号中当前可路由且被选中的全部模型”，不能写成“已测试标准 ModelArk 返回的全部模型”。

## 标准 ModelArk 模型结果

| 物理模型 | 最终分类 | API 结果 |
|---|---|---|
| `deepseek-v4-1-flash-260910` | 完整通过 | `llm`：通过 |
| `deepseek-v4-flash-ga-260731` | 完整通过 | `llm`：通过 |
| `deepseek-v4-pro-ga-260813` | 完整通过 | `llm`：通过 |
| `doubao-embedding-vision-251215` | 完整通过 | `embedding.multimodal`：通过 |
| `doubao-seed-character-251128` | 完整通过 | `llm`：修复标记断言后通过 |
| `doubao-seed-character-260628` | 完整通过 | `llm`：通过 |
| `doubao-seed-translation-250915` | 完整通过 | `llm`：修复语言代码后通过 |
| `glm-5-2-260617` | 完整通过 | `llm`：通过 |
| `glm-5-3-flash-260828` | 完整通过 | `llm`：通过 |
| `doubao-seed-2-0-code-preview-260215` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2-0-lite-260428` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2-0-mini-260428` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2-1-lite-260915` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2-1-pro-260628` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2-1-pro-260915` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2-1-turbo-260628` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seedance-1-0-pro-fast-251015` | 部分通过 | `video.txt2video`、`video.img2video` 已解析官方结果并验证制品，待语义复核 |
| `doubao-seedream-4-0-20260415` | 部分通过 | `image.txt2img`、`image.img2img` 制品验证通过，待语义复核 |
| `doubao-seedream-5-0-flash-260915` | 部分通过 | `image.txt2img`、`image.img2img` 制品验证通过，待语义复核 |
| `doubao-seedream-5-0-pro-260628` | 部分通过 | `image.txt2img`、`image.img2img` 制品验证通过，待语义复核 |
| `doubao-seed-evolving` | Provider 受限 | `llm`、`vision.caption`、`vision.ocr`：`ModelNotOpen` |
| `glm-4-5-air-20250728` | Provider 受限 | `llm`：`InvalidEndpointOrModel.NotFound` |
| `qwen2-5-72b-20240919` | Provider 受限 | `llm`：`InvalidEndpointOrModel.NotFound` |
| `qwen3-0-6b-20250429` | Provider 受限 | `llm`：`InvalidEndpointOrModel.NotFound` |
| `qwen3-14b-20250429` | Provider 受限 | `llm`：`InvalidEndpointOrModel.NotFound` |
| `qwen3-32b-20250429` | Provider 受限 | `llm`：`InvalidEndpointOrModel.NotFound` |
| `qwen3-8b-20250429` | Provider 受限 | `llm`：`InvalidEndpointOrModel.NotFound` |

标准账号没有遗留失败。上述 7 个受限 ID 均由实时 `/api/v3/models` 返回，但该接口没有账号激活状态，因此保留真实调用限制，不通过模型名特判隐藏。

## Agent Plan 模型结果

| 物理模型 | 最终分类 | API 结果 |
|---|---|---|
| `deepseek-v4-flash` | 完整通过 | `llm`：通过 |
| `deepseek-v4-pro` | 完整通过 | `llm`：通过 |
| `doubao-embedding-vision` | 完整通过 | `embedding.multimodal`：通过 |
| `glm-5.3` | 完整通过 | `llm`：通过 |
| `kimi-k2.8-preview` | 完整通过 | `llm`：通过 |
| `minimax-m3` | 完整通过 | `llm`：通过 |
| `deepseek-v4.1-flash` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2.0-mini` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2.1-lite` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2.1-pro` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2.1-turbo` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-evolving` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-tts-2.0` | 部分通过 | `audio.tts` HTTP 输出和制品通过，待音频语义复核 |
| `doubao-seedream-5-0-pro` | 部分通过 | `image.txt2img`、`image.img2img` 制品通过，待语义复核 |
| `doubao-seedream-5.0-lite` | 部分通过 | `image.txt2img`、`image.img2img` 制品通过，待语义复核 |
| `glm-5.3-flash` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `kimi-k2.7-code` | 部分通过 | `llm` 通过；`vision.caption`、`vision.ocr` 协议/输出通过，待语义复核 |
| `doubao-seed-2.0-lite` | Provider 受限 | `llm`、`vision.caption`、`vision.ocr`：`UnsupportedModel` |
| `kimi-k3` | Provider 受限 | `llm`、`vision.caption`、`vision.ocr`：`UnsupportedModel` |

Agent Plan 没有遗留失败。

## 未启动范围

| 账号类型 | 模型族 | 单元数 | 原因 |
|---|---|---:|---|
| 标准 ModelArk 和 Agent Plan | `doubao-seedance-2.5` | 每种账号 4 个 | 用户明确排除 |
| 标准 ModelArk 和 Agent Plan | `doubao-seedance-2.0` | 每种账号 4 个 | 用户明确排除 |
| 标准 ModelArk 和 Agent Plan | `doubao-seedance-2.0-mini` | 每种账号 4 个 | 用户明确排除 |
| 标准 ModelArk 和 Agent Plan | `doubao-seedance-2.0-fast` | 每种账号 4 个 | 用户明确排除 |

每个模型族原本会派生 `video.txt2video`、`video.img2video`、`video.video2video`、`video.extend` 四个单元。Provider 受限模型不属于“未启动”：它们已发送真实请求并被 Provider 拒绝。

语音范围单独处理：Agent Plan 的 `doubao-seed-tts-2.0` 已通过官方 HTTP 协议执行；按要求未实现或测试仅支持 WebSocket 的语音操作；普通 `ark-...` 推理凭证不能证明豆包语音权限，因此标准 ModelArk 不宣告语音操作。

## 已发现并关闭的缺陷

| 缺陷 | 修复前证据 | 修复 | 最终证据 |
|---|---|---|---|
| 翻译目标语言使用展示名称 | Provider 拒绝 `target_language=English` | 配置和 T1.5 请求改用官方代码 `en` | 标准账号翻译 T2 通过 |
| Seedance 结果 URL 响应结构错误 | 真实任务成功，但 AICC 找不到 `video_url` | 解码器和标准/Agent Plan Mock 改用字符串形式 `content.video_url`，并保留 usage | 两个标准账号视频单元生成并验证制品；T1.5 生命周期通过 |
| 角色模型标记断言对排版敏感 | Provider 返回 `BUCKYOS - AICC - 4827` | 通用断言规范化分隔符两侧空白，不添加模型特判 | Character 251128 T2 通过 |

## 执行历史

| 日期 | 执行 | 目的 | 采用结果 |
|---|---|---|---|
| 2026-09-29 | [Agent Plan 完整 T2](../reports/acceptance/aicc-2026-09-29T14-06-14-898Z-c836c51b/summary.md) | 排除 Seedance 后执行 41 个单元 | 14 通过、21 待复核、6 Provider 受限、0 失败 |
| 2026-09-29 | [标准账号初始 T2](../reports/acceptance/aicc-2026-09-29T14-14-36-773Z-10e76d30/summary.md) | 建立 47 单元基线 | 后续重测存在时由新结果覆盖，原结果保留审计 |
| 2026-09-29 | [充值前定向 T1.5](../reports/acceptance/t15-20260929140206-4035410/summary.md) | 翻译和 Seedance 请求协议 | 3 通过、0 失败 |
| 2026-09-30 | [余额恢复后标准账号 T2](../reports/acceptance/aicc-2026-09-30T04-45-42-713Z-8c2a4567/summary.md) | 重试被余额阻塞的 35 个单元 | 暴露 4 个可修复失败，其余结果保留 |
| 2026-09-30 | [修复后 T1.5](../reports/acceptance/t15-20260930053834-640745/summary.md) | 翻译、两类账号视频成功响应及 TaskMgr 制品持久化 | 9 通过、0 失败 |
| 2026-09-30 | [标准账号最终重测](../reports/acceptance/aicc-2026-09-30T06-54-40-209Z-49932e6d/summary.md) | 4 个修复单元和 9 个历史非余额限制单元 | 2 通过、2 待复核、9 Provider 受限、0 失败 |

各轮执行均完成清理：临时 Provider 凭证已恢复，生成的输出对象已删除。

## 验证门禁

- AICC Rust 测试：550 通过、0 失败。
- 验收自测：91 通过、0 失败。
- 验收预检：24 个规范 API、130 个静态 T1 用例、691 个 T1.5 用例。
- 修复后定向 T1.5：9 通过、0 失败。
- `deno check acceptance/*.ts` 和 `git diff --check` 通过。
- 已跟踪差异凭证扫描未发现 API Key。

按时间顺序的实现说明及更早的 T1/T1.5 记录见 [ISSUE_624_TEST_REPORT.md](ISSUE_624_TEST_REPORT.md)。

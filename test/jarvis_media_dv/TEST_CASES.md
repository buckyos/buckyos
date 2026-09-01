# Jarvis 真实环境媒体用例

用例的可执行定义和完整指令位于 [scenarios.ts](./scenarios.ts)。本文用于评审覆盖面与执行顺序。

## 基础闭环（smoke）

| ID | 输入 | 主要验证点 | 自动判定 | 人工判定 |
|---|---|---|---|---|
| `image_describe` | 花朵图片 + 描述要求 | 媒体理解、客观回复 | 有文本终态 | 主体、背景、颜色正确 |
| `image_ocr` | 含 `BUCKYOS-DV-4827` 的图片 | OCR/媒体理解路由 | 回复包含唯一文字 | 没有被额外 OCR 失败覆盖 |
| `image_edit` | 花朵图片 + 添加大黄蜂 | edit_image、附件回传 | 返回 image Named Object | 编辑对象、位置、背景正确 |
| `audio_sfx` | 无人声音效，随后要求转写 | 音频描述与 ASR 可信度 | 有文本且不含已知幻觉句 | 判断为非语音，保持不确定性 |
| `speech_roundtrip` | 清晰语音，随后合成上一句 | ASR → 历史引用 → TTS | 包含编号并返回 audio | 合成语音内容与转写一致 |
| `image_to_video` | 图片 + 4 秒动画要求 | Veo/img2video、长任务、语言 | 返回 video Named Object | 视频选图正确、终态中文 |
| `archive_process_and_repack` | 文档媒体混合 ZIP | 安全解包、事实提取、重新打包 | 固定事实码和 ZIP 附件 | 输出清单与内容可读 |
| `archive_valid_shapes` | 单文档、多文档、嵌套 ZIP | 合法结构、中文路径、空目录和嵌套处理 | 固定事实码或清单 | 文件边界不串包 |
| `archive_*_rejected` | 空、损坏、加密、路径穿越、超文件数、超解压量、超深 ZIP | 解压安全限制和明确拒绝 | 明确诊断且零结果附件 | 无越界写入或继续处理 |

## 跨消息关联（linked）

| ID | 消息链 | 主要验证点 |
|---|---|---|
| `edit_then_animate` | 上传原图 → 无附件编辑 → 无附件动画化编辑结果 | 后续任务引用上一轮生成物，而不是原始图 |
| `rapid_attachment_binding` | 快速发送素材 A → 300ms 后发送素材 B 并生成视频 | 批量进入 Agent 后仍保留消息与附件边界 |
| `correction_without_attachment` | 上传 A → 上传 B → 最近素材生成 → 通过 `reply_to` 引用 A 并无附件纠错 | 最近附件优先和结构化历史引用均能正确工作 |
| `generated_video_extension` | 图片生成视频 → 无附件延长刚生成的视频 | content/source task/Provider continuation 状态持久恢复 |
| `fresh_video_extension` | 上传用户视频 → 要求延长 | 原生续写不可用时采用合理替代并给出用户可读说明 |

## 四类媒体转换矩阵（matrix）

每个格子都是独立 scenario，并从 `/clean` 开始。这里的跨模态转换关注用户语义目标：例如“图片 → 音频”是根据图片生成语音讲解，“音频 → 图片”是根据音频内容生成配图。

| 输入 \ 输出 | 文本 | 图片 | 音频 | 视频 |
|---|---|---|---|---|
| 文本 | `matrix_text_to_text`：摘要 | `matrix_text_to_image`：文生图 | `matrix_text_to_audio`：语音合成 | `matrix_text_to_video`：文生视频 |
| 图片 | `matrix_image_to_text`：视觉描述 | `matrix_image_to_image`：图片编辑 | `matrix_image_to_audio`：语音讲解 | `matrix_image_to_video`：图生视频 |
| 音频 | `matrix_audio_to_text`：语音转写 | `matrix_audio_to_image`：语义配图 | `matrix_audio_to_audio`：音频增强 | `matrix_audio_to_video`：语义视频 |
| 视频 | `matrix_video_to_text`：视频描述 | `matrix_video_to_image`：代表帧 | `matrix_video_to_audio`：语音解说 | `matrix_video_to_video`：视频风格转换 |

矩阵共 16 个真实环境用例。自动检查验证文本终态及目标 MIME 类型 Named Object；内容是否忠于输入、媒体质量和跨模态语义一致性由人工检查。

## 关键异常映射

| 历史异常 | 覆盖用例 |
|---|---|
| 图片编辑只返回说明或本地路径，没有附件 | `image_edit` |
| 图生视频超过 60 秒后重复请求或无终态 | `image_to_video` |
| 后台任务完成后固定回复英文 | `image_to_video` |
| 音效被转写为 “Welcome.” 等句子 | `audio_sfx` |
| 当前消息附件和下一条正文被错误组合 | `rapid_attachment_binding` |
| 纠错时未重新上传，Agent 选错历史图片 | `correction_without_attachment` |
| 编辑后的图片继续生成视频时回退到原图 | `edit_then_animate` |
| Provider 生成视频缺少续写状态映射 | `generated_video_extension` |
| 用户上传视频被错误声称可原生续写 | `fresh_video_extension` |
| 类型化对象 ID 被降成十六进制摘要 | 所有自动附件用例 |

## 消息语义、可靠性与恢复

| ID | 验证点 |
|---|---|
| `outbound_delivery_idempotency` | 同一轮最终只出现一个唯一的出站 `msg_id` / delivery record |
| `group_message_semantics` | 参数化 group DID 内回复保持在原群会话；未配置 group DID 时明确 skipped |
| `forwarded_message_source` | `MsgObject.source` 与当前发送者不混淆 |
| `duplicate_inbound_idempotency` | 同一外部消息重复投递只产生一个存储输入和一个用户可见回复 |
| `delivery_retry_final_state` | 当前环境没有逐消息出站故障注入时明确记录平台限制 |
| `jarvis_restart_recovery` | 未授权重启时明确记录平台限制，不以 reload 冒充 restart |

带语义检查项的实际执行步骤默认交给参数化 LLM Judge；结构、附件数量、MIME、唯一消息和固定事实码继续使用确定性断言。报告保存 Judge task ID、理由及成本，并要求 AICC route trace 使用 `run_id:transport:scenario_id:step_id` 关联 workflow 调用；缺失关联作为产品缺陷失败。

## 建议执行顺序

1. 先运行 `smoke`，确认 Provider、附件回传和长任务链路可用。
2. 运行 `matrix`，逐格确认 16 种输入输出组合及当前 Provider 覆盖能力。
3. 再运行 `linked`，每个 scenario 使用独立会话并从 `/clean` 开始。msg-center 默认并行两个独立会话，同一场景内部步骤保持串行。
4. 默认通过 `msg-center` 收集结构报告；需要验证完整 tunnel 时，在同一入口中显式加入 `telegram`。
5. 失败时保存本次 `summary.md`、链接的场景对话详情和 `summary.json`，同时收集相同时间段的 OpenDAN、AICC 和 msg-center 日志。

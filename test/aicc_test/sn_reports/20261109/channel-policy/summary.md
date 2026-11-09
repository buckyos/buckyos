# SN 渠道禁用与验收结论

日期：2026-11-09（北京时间）。按用户明确决定：Grok 视为成功；服务端仍无法解决的 Qwen Image/Wan 按已有 exclude/remove_api_types 机制处理。本轮没有新的真实模型调用，费用 $0。

## 当前配置

| SN 模型 | 处理 | 涉及 API | 依据 |
|---|---|---|---|
| qwen-image-3-pro | `exclude: true` | image.txt2img、image.img2img | 当前 SN 暴露的两个 API 均无 T2 通过项；生成的上游入口冲突仍未解决，编辑被前置生成阻断 |
| wan-3.0 | `exclude: true` | video.generate、video.edit、video.extend | 三个 API 均无 T2 通过项；生成出站 multipart 与 OpenRouter JSON 协议不匹配，编辑/延长被前置阻断 |
| grok-4.7 | 保持开放 | llm、vision.ocr、vision.caption | LLM 按用户验收决定接受为通过；OCR/caption 既有通过 |
| gpt-6.1-terra | 保持开放 | 既有全部 API | 四个历史失败单元已于 2026-11-09 独立 T2 通过 |
| qwen3.7-plus | 保持开放 | llm、vision.ocr、vision.caption | OCR 已成功返回内容；既有识别质量结果保留 |

两个模型在 SN 所有当前 API 都没有成功项，因此采用整模型 `exclude`，无需逐 API 重复移除。其他 Provider 及原厂 `qwen.model.json` 的图像/视频能力不受本轮禁用影响。既有 Realtime 排除、UI-TARS computer-use 移除等策略保持。

依据：[SN #49](https://github.com/buckyos/sn-business/issues/49)、[补充复现](https://github.com/buckyos/sn-business/issues/49#issuecomment-6527012935)、[Grok 上游原始响应核查](https://github.com/buckyos/sn-business/issues/54#issuecomment-6569762068)。#54 确认该次拒答已存在于 OpenRouter 返回、上游标识为 xAI，未发现 SN/LiteLLM 注入或改写。具体模型侧/平台侧策略来源仍不作超出证据的判断。

## Grok 验收口径

`grok-4.7.llm` 当前记为 **passed（用户接受）**。这是用户对成功调用及正确透传上游拒答的验收决定，不表示本轮重新调用得到了 `BUCKYOS-AICC-4827`。2026-10-30 原始请求、拒答和自动失败报告保留；通用标记断言、提示词和 Mock 协议没有为 Grok 放宽。issue 正文和评论没有改写。

## 汇总范围

| 范围 | 单元数 | 通过 | 失败 | 待复核 |
|---|---:|---:|---:|---:|
| 历史已测全部（含现已禁用） | 172 | 114 | 56 | 2 |
| 已禁用的历史单元 | 5 | 0 | 5 | 0 |
| 扣除已禁用后的历史启用范围 | 167 | 114 | 51 | 2 |

通过数包含 1 个 Grok 用户接受单元；5 个被禁用的历史失败没有改成通过。启用范围仍有 27 个未重新执行的旧 NDM Proxy 读取失败记录、24 项内容验收失败和 2 项待复核，不能据此宣布全量 T2 通过。该范围是已有历史矩阵，不是本轮重新抓取全部账号目录后生成的新全量矩阵。

## 验证与报告

- SN Rust 单测 **12/12**：完整库存排除两个模型、保留原厂能力与可用 API；同步 metadata revision 断言。
- acceptance 自测 **152/152**、Deno 类型检查通过；包含渠道过滤、原厂能力保留、禁用 API 不进入矩阵及正常图片模型覆盖。
- T1.5 **81 passed、54 skipped、0 failed**，共 135 个定向单元。skipped 是显式禁用策略产生，不计通过；这些单元没有发送推理请求。Grok、Terra、Qwen3.7 Plus 的基础 API 和相关预设通过。
- [本轮 T1.5 全部用例](runs/t15-20261109081136-2942440/summary.md)、[零真实调用账本](runs/t15-20261109081136-2942440/finance.md)。
- [真实 Gateway 下的临时 SN 库存核验](runtime-inventory/summary.md)。
- SN JSON 本地 override 已原子替换并由 AICC reload 生效；有独立备份，无 release 构建或 Zone 重启。
- 临时 Provider/Mock 设置 cleanup passed；原系统库存恢复；Gateway 登录和 models.list 收尾检查通过。没有新制品需要清理。
- `git diff --check`、报告链接及提交白名单检查通过。未运行完整 workspace 测试、完整构建或新的 T2。

变更仅为 SN Provider 配置、测试基线/路由暴露、既有测试断言和报告。没有修改原厂模型参数、共享协议、公共运行逻辑、依赖或其他 Provider。检查了 `aigc_image`、`aigc_video`、`aigc_audio`：手册按运行库存发现可用模型，没有依赖这两个 SN 模型，不需要改命令或示例。

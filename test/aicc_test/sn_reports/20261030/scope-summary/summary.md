最新状态：[公共修复已部署及定向复测结果](../local-fix-retest/summary.md)。以下保留诊断时证据和原轮次结果；当前跨轮矩阵为 100 通过、64 失败、2 review。

# 禁用项与新增模型结果

依据：Git HEAD 的 SN provider 配置、当前 `sn.provider.json`、各轮 T1.5/T2 报告及最终内容复核。这里的“通过”仅指本次测试覆盖的 API；没有将 T1.5 Mock 通过视作真实模型通过。

后续只读诊断已通过配置式原生 NDN 入口下载 34 个既有对象，证明 Proxy 405 不等于模型生成失败或 NDN 无下载能力；下表仍保留原 T2 验收状态。公共存储/驱动后续修复未部署，详见[逐项诊断](../followup-diagnosis/summary.md)。

## 原禁用项

| 原条目 | 本次结果 | 当前配置 |
|---|---|---|
| gpt-5.4-mini / vision.ocr | T1.5 通过；T2 返回文本，但完整 OCR 标记错误 | 已解除 remove_api_types，仍开放 |
| gpt-5.4-nano / vision.ocr | T1.5 通过；T2 返回文本，但完整 OCR 标记错误 | 已解除 remove_api_types，仍开放 |
| gpt-image-2 / image.inpaint | T1.5 通过；T2 生成成功，制品读取遇到 NDM Proxy 405 | 已解除 remove_api_types，仍开放 |
| qwen3.7-plus / vision.ocr | T1.5 通过；T2 超时问题已排除，但返回文本的完整标记错误 | 已解除 remove_api_types，仍开放 |
| qwen-image-3-pro / image.txt2img、image.img2img | T1.5 通过；T2 SN/LiteLLM 上游使用 chat/completions，被 OpenRouter 404 拒绝；img2img 被前置生成失败阻断 | 两项均解除 remove_api_types，仍开放 |
| wan-3.0 / video.generate、video.edit、video.extend | T1.5 通过；T2 上游 multipart 与 OpenRouter JSON 要求不匹配，400；edit/extend 被前置生成失败阻断 | 三项均解除 remove_api_types，仍开放 |
| gpt-realtime-2.1 | 缺少对应 WebSocket 传输，未恢复可调用测试 | 保持 exclude |
| ui-tars-1.5-7b / agent.computer_use | 原厂输出文本动作，不是本适配器要求的原生 computer tool 协议 | 保持 remove_api_types |
| sora-2、sora-2-pro | 当前 SN 发布列表中已经删除，不再作为当前可调用模型 | 从 SN 显式条目删除；最终通配规则 exclude |

共恢复 9 个 API 单元，T1.5 全部通过，最终 T2 全部失败；其中包含模型输出质量失败及资源/上游失败。失败后没有重新写回 exclude/remove_api_types。这是当前配置的实际状态，不能把这些单元描述为已完整验收。

生产协议修复为 SN 音频 ASR reasoning 参数透传、SN 视频上传编辑请求补齐 model；相应单测与 T1.5 通过。前者的 Gemini ASR 真实测试通过；后者的 Wan 真实编辑仍被前置视频生成失败阻断，未实测成功。

## 新增 SN 条目（25 个）

### 本次 T2 所选 API 全部通过（10 个）

| 模型 | 本次通过范围 |
|---|---|
| claude-opus-5.5、claude-sonnet-5.5 | llm、vision.ocr、vision.caption |
| deepseek-v4-pro-0813 | llm |
| deepseek-v4.1-flash-1017 | llm、vision.ocr、vision.caption |
| gemini-3.8-flash、gemini-4-argon | llm、vision.ocr、vision.caption、audio.asr |
| glm-5.3-flash | 两个 SN 实例的 llm、vision.ocr、vision.caption |
| qwen3.8-27b、qwen3.8-2.4t-a95b | llm |
| qwen3.8-omni-flash | llm、vision.caption |

### 部分通过（7 个）

| 模型 | 通过 | 未通过 |
|---|---|---|
| claude-haiku-5.5 | llm、OCR | caption 把五个背景椭圆描述为四个 |
| qwen3.8-flash | llm、caption | OCR 完整标记错误 |
| kimi-k3.1-pro | 两个实例的 llm、caption | 两个实例的 OCR 完整标记错误 |
| minimax-m3.1-flash | llm、caption | OCR 完整标记错误 |
| grok-4.7 | OCR、caption | llm 拒答，未输出验收标记 |
| gpt-6.1-sol | llm、caption、computer-use | OCR 完整标记错误；生图下载及 img2img 前置资源读取受 NDM Proxy 405 阻断 |
| gpt-6.1-terra | llm、caption | computer-use 没有原生动作；OCR、生图及 img2img 前置生成未确认 background 执行 |

### 尚未完成端到端适配验收（2 个）

`gpt-image-2.5-flare`、`gpt-image-2.5-sunburst`：T1.5 通过；T2 txt2img/inpaint 已产生结果，但 NDM Proxy 405 导致制品无法下载；img2img 的前置资源读取同样失败，目标调用没有执行。因此不能标为 T2 成功，也不能据此判定上游模型不会生成。

### 无独立 T2 结论（5 个别名/预设）

| 条目 | T2 代表模型 |
|---|---|
| gpt-6.1-sol-pro | gpt-6.1-sol |
| gpt-6.1-terra-pro | gpt-6.1-terra |
| glm-5.3-flashx | glm-5.3-flash |
| glm-5.3-prime | glm-5.3 |
| qwen3.8-max-prime | qwen3.8-max-0902 |

这些条目的参数/映射已通过独立 T1.5。按本次物理模型去重规则没有另发真实推理，不能称为各自独立通过 T2。

### 保持禁用（1 个）

`gpt-realtime-2.1-mini`：新加入配置，但与已有 Realtime 相同，当前缺少 WebSocket 传输支持，保持 exclude，没有可调用 T2 结论。

## 失败归类

最终 67 个矩阵失败按互斥主因归为：NDM Proxy 405（33）、Qwen Image 上游路由（2）、Wan 上游 Content-Type（3）、Pro OCR 超时（2）、GPT-6.1-Terra 协议响应（4）、制品归属写入（1）、Grok 拒答（1）、内容复核（21，其中 OCR 19、caption 2）。详见[失败证据与边界](../external-validation/summary.md)及[内容复核逐项结果](../manual-review/summary.md)。

另有收尾失败不计入 67：33 个测试对象未清理，以及两个 Pro OCR 任务在 AICC 与 TaskMgr 间终态不一致。状态为测试完成时的记录，本次整理没有重新执行模型测试，也没有改变模型禁用状态。

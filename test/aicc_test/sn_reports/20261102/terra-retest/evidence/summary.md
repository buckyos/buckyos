# Terra 失败重测的任务与 trace

原始任务及错误经只读持久化记录核验。所有任务均为 Terminal/Failed；下表的图编辑前置是文生图任务，不是已执行的 img2img 目标。

| 单元 | Task | Trace | 错误 |
|---|---|---|---|
| vision.ocr | `t-41cff213c93f463aafd2d2072a1d4369` | `aicc-idem-7d88e489ac7db764b0f4e677708e7535` | Provider did not accept Responses background execution |
| image.txt2img | `t-279962a9e25e4e84a0b5adb7c1dded07` | `aicc-idem-84e046506bb3640d44a58c8ca7172fa3` | Provider did not accept Responses background execution |
| img2img 前置 txt2img | `t-e0ea0f3648d148d2ae2f0c795a7f956e` | `aicc-idem-de0c8319ca79fd106f0c1e4b4640fbb3` | Provider did not accept Responses background execution |
| agent.computer_use | `t-800a4bcb20b74b8aad83ee7032819924` | `aicc-idem-2df5592e7b5261a9a5504b0fc047bb6b` | OpenAI response has no computer action |

这些错误发生在 Provider 响应的协议验收阶段；没有形成可继续验收的 image/computer/OCR 结果，不是 Named Data 下载错误。仅有当前错误摘要，不能进一步断定 background 字段是缺失、false 还是 null，也不能把“没有 computer action”直接解释成模型不支持或上游拒答。

没有修改 AICC 实现、原厂参数、SN 渠道操作或提示词；background 用例保持原异步操作，没有同步回退。没有新增付费直连探测，没有追加 issue 评论。

# TODO：完善 llm_understand_media 的输入决策、转换与共享缓存

需求来源：设计文档自身加上开发者对 Agent runtime baseline 应携带哪些工具的思考。

2026-11-05：本 TODO 的内容已并入设计文档 [LLM Understand Media](../../doc/llm_context/LLM%20Understand%20Media.md)，按"基础工具、有则用、最小内核可直传"的原则重新组织：

- 尽力原则（有则用、最小内核 = 直传）→ §1.4
- 发送前三问、决策阶梯、三类产物、目录结构、预算、决策记录、格式支持目标 → §4.6
- 目录理解与检索（默认宿主循环）→ §4.7
- 复用 preview 共享表示缓存：沿用什么、要扩展什么、内容身份、边界 → §4.8
- 验收 → §8 第 15–20 项；待定项 → §9；实施清单 T1–T6 → §10

实现进度在设计文档 §10 勾选，本文件不再维护。

2026-11-05（晚）：§4.5 / §4.8 的落地改为 Workflow Service 的系统内置 pipeline，见 [llm-media-pipeline-workflow-todo.md](../llm-media-pipeline-workflow-todo.md)；设计文档相应章节待按该 TODO 的 T0 回写。

# SN Qwen OCR background 诊断（输出上限 1024）

日期：2026-10-23（Asia/Shanghai）。Ubuntu；经过 Zone Gateway、AICC 和激活时的 SN 地址。使用普通 SN Key 的独立临时实例，未调用激活实例进行推理。本轮为诊断，不将质量失败改记为通过。

| 用例 | 参数与输入 | 结果 | 耗时 |
|---|---|---|---|
| qwen3.7-plus / vision.ocr / responses.background | background=true、stream=false、max_output_tokens=1024；原 image_ocr.png，base64 输入 | 失败：`max_output_tokens`；未取得上游任务 ID | 首次 AICC 响应 36612 ms；终态 36745 ms |

- 本地任务 ID：`t-9bb66799e2af49d9bbdf3c234a4175b0`；不能将此 TaskMgr ID 当作上游后台任务句柄。
- 图片大小：2189 bytes；SHA256：`eaa912debbdd30ccbbebbdae7ac4950b321afaaf740dd98ad76af3d7cfdc19c9`，与原失败用例相同。
- 未额外关闭 thinking，未添加 store 参数；沿用原模型行为。1024/4096 是本轮诊断引入的输出上限。
- 使用已有 SN NativeTask codec；只有取得已接受的上游后台任务 ID 才能进入轮询。本轮无远端句柄，未执行上游状态查询，也没有自动重发推理。
- 1024-token 上限提前终止请求，未覆盖原约 120 秒失败；不能据此判断长任务异步能力。
- 针对性单测 `background_responses_cover_submit_poll_result_cancel_and_failure` 1/1 通过。没有新增产品代码或替代真实上游的 Mock 结论。
- 清理：settings、metadata 已恢复，临时实例已删除，激活 SN 的 device_jwt 动态登录保留。本地网关未修改。

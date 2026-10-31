# SN issue 最终待确认正文

状态：已按用户确认提交并回读核验。A：[回复 #49](https://github.com/buckyos/sn-business/issues/49#issuecomment-6527012935)；B：[Terra issue #53](https://github.com/buckyos/sn-business/issues/53)。labels、assignees、milestone、project placement 均为 none。

## A

仓库：`buckyos/sn-business`

目标：https://github.com/buckyos/sn-business/issues/49

2026-10-30 补充复现：本 issue 的 Qwen Image 和 Wan 协议问题仍存在。

SN 源码/release 配置核对至 `c23a55b4`，入口 `https://sn.buckyos.ai/api/v1/ai`，使用 GLOBAL_OPEN；线上 LiteLLM 实际版本未核验，未修改或部署 SN。本文不含 API Key。

复现 run：`aicc-2026-10-30T09-21-18-381Z-cb64369b`。

**Qwen Image**：BuckyOS 选择 SN `/images/generations`，模型 `qwen-image-3-pro`，prompt 为 `A blue square containing 4827`，n=1。SN/LiteLLM 的上游返回 404：

```text
qwen/qwen-image-3-pro is an image generation model and cannot be used with the chat/completions endpoint. Use the /api/v1/images endpoint instead.
```

直接 txt2img 任务：`t-6ea197a1fc3f43719254ed48027e22ab`；img2img 前置生成：`t-323972e8476247eeb7299288dcb89b52`。img2img 目标请求未发起，因此不是已独立复现编辑接口拒绝。

**Wan**：最小生成为 2 秒、854×480。直接生成任务 `t-d3cf3e26610f460e817ca165bfbc32d4`、edit/extend 共用前置任务 `t-e73949b85d414829a26a83cddbbcc692`，均返回 400：

```text
litellm.BadRequestError: OpenAIException - Content-Type must be application/json
```

当前 release 仍以 `openai/alibaba/wan-3.0` 对接 OpenRouter api_base。[LiteLLM OpenAI 视频编码器](https://github.com/BerriAI/litellm/blob/main/litellm/llms/openai/videos/transformation.py)使用 multipart，[OpenRouter 视频入口](https://openrouter.ai/docs/api/api-reference/video-generation/create-videos)要求 JSON。只改请求头或 AICC→SN 编码不能消除 LiteLLM 的重新编码。edit/extend 目标请求未发起。

请补齐 SN→上游的独立图片接口适配、视频 JSON 字段映射和生命周期，并用严格 Mock 检查最终出站路径、Content-Type、字段及响应。修复后分别验证生成、编辑、延长，不以共用前置失败代替目标接口验证。

本轮 Qwen3.7 OCR 已返回文本；输入图与原 issue 相同，但提示词/执行条件不同，不据此宣布原长任务问题修复。本地 NDM Proxy、SQLite/异步驱动问题和内容质量问题没有归入本回复的 SN 缺陷。

## B

仓库：`buckyos/sn-business`

标题：[sn-aicc] GPT-6.1-Terra 的 OpenRouter 路由未完成 background/computer 协议闭环

2026-10-30 经 SN US_FULL 调用 `gpt-6.1-terra`：LLM、caption 成功，但 Responses background/computer 协议闭环失败。

环境：SN 源码/release 配置 `c23a55b4`；入口 `https://sn.buckyos.ai/api/v1/ai`；release 映射为 `openrouter/openai/gpt-6.1-terra`。线上 LiteLLM 实际版本未核验，未修改或部署 SN，本文不含 API Key。

Run：`aicc-2026-10-30T09-01-10-151Z-69abbe3b`。

- OCR：BuckyOS 提交 background=true、stream=false；响应未确认 background=true。任务 `t-a6d0d88596674680838c66b49242dce3`。
- txt2img：同样未确认 background=true。任务 `t-867ab7447b3a400bab2d172cb9b30351`。
- img2img：前置生成同样失败，目标请求未发起。前置任务 `t-828ed7d8ee3a4bff9300d5521d53794b`。
- computer-use：请求包含 tools=[{type:computer}]、tool_choice=required，并要求在截图按钮内点击一次；响应 output 没有可解析的 computer_call。任务 `t-d33f9afcfed74ec5b4e4fb7c62ba9148`。

错误分别为 `Provider did not accept Responses background execution` 和 `OpenAI response has no computer action`。这是提交响应校验失败，不是 15 分钟轮询超时。

当前结论仅为该实际路由无法满足 BuckyOS 所选 operation，不能据此认定原厂模型不支持。原始上游响应体没有保留，尚不能区分 background 字段缺失/null/false，也不能确定 computer 响应是拒答、文本动作或其他输出。

请关联以上任务核对脱敏出站请求、真实上游、状态码及响应摘要，明确该路由是否支持 [Responses background](https://openrouter.ai/docs/api/api-reference/responses/create-responses)和原生 computer tool。支持则修复转发/响应并验证提交、轮询、结果、取消及 computer_call；不支持则明确渠道限制或更换上游，BuckyOS 据此更新 SN provider 覆盖，不修改原厂模型参数。请补齐 SN→模拟上游的严格正反协议用例；img2img 需要实际到达目标接口后验收。

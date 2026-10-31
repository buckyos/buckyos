最新状态：[公共修复已部署及定向复测结果](../local-fix-retest/summary.md)。以下保留诊断时证据和原轮次结果；当前跨轮矩阵为 100 通过、64 失败、2 review。

# SN T2 未通过项与测试边界

历史矩阵合并结果为 166 项：99 通过、67 失败。后续诊断见[逐项结论](../followup-diagnosis/summary.md)：34 个既有制品已使用配置式原生 NDN 入口读回，公共 SQLite/执行器修复仅完成单测，未部署。自动轮次结果、内容复核与基础设施收尾分别保存；不会把 Provider 返回成功等同于制品已可下载交付。

| 分类 | 最终矩阵失败数 | 依据与边界 |
|---|---:|---|
| NDM Proxy 制品读取/后续调用 | 33 | Gateway 将 `/ndm/*` 转发给 Control Panel；[当前 handler](../../../../../src/frame/control_panel/src/main.rs)不提供 SDK 使用的 `/ndm/proxy/v1/read/*`、`/rpc/*`，返回 405。NDM Proxy 协议本身不能用配置补齐；原生 NDN GET 是另一个入口，已通过临时配置证明可下载，并未修改 NDN 逻辑。生成成功但下载失败仍为失败；前置资源不可读时没有发起目标 img2img 调用。 |
| Qwen Image | 2 | AICC 已提交 SN `/images/generations`；SN/LiteLLM 返回 OpenRouter 404：模型不能用于 chat/completions，应使用 `/api/v1/images`。错误发生在 SN 向其上游的适配；BuckyOS 未改用不符合 SN 入口契约的路径，也未改写 SN 仓库。 |
| Wan | 3 | 返回 OpenRouter 400：`Content-Type must be application/json`。SN release 使用 `openai/alibaba/wan-3.0` 对接 OpenRouter；[LiteLLM OpenAI video 实现](https://github.com/BerriAI/litellm/blob/main/litellm/llms/openai/videos/transformation.py)强制 multipart，而 [OpenRouter 视频入口](https://openrouter.ai/docs/api/api-reference/video-generation/create-videos)要求 JSON。edit/extend 被前置生成失败阻断，未声称已经调用目标编辑/扩展接口。 |
| GPT Pro OCR | 2 | runner 等待 TaskMgr 终态超过 900000ms；两任务已有 remote ID。本地进度持久化/驱动退出问题已复现，不能认定为上游慢。上游结果与费用仍未知。 |
| GPT-6.1-Terra | 4 | computer-use 未返回 computer action；OCR、生成及 img2img 前置资源的响应未确认 `background=true`。根据 [AICC operation 契约](../../../../../doc/aicc/provider_operation_bindings.md)保持 fail-closed，未改为同步降级。 |
| 本地制品归属写入 | 1 | GPT-5.5 img2img 的前置生成任务返回 `inline artifact ownership write failed`。图片内容已存储并读回，归属记录缺失。同期 INSERT 等待 5.008 秒；本地 DELETE 模式读写冲突可复现 SQLITE_BUSY。原错误代码被丢弃，历史锁持有者不可还原；公共存储/驱动修复仅通过单测。 |
| Grok 指令输出 | 1 | 原任务返回 `I can't do that.`，未包含要求的验收标记；保留失败。 |
| 人工内容复核 | 21 | 19 项完整 OCR 标记不符、2 项 caption 数量描述错误。详见 [内容复核](../manual-review/summary.md)；原始自动状态为 review，没有伪装为自动通过。 |

上述模型/协议错误均保留其官方约束。T1.5 验证 BuckyOS 到 SN 的公开协议，不将 SN 内部 LiteLLM 到第三方上游的错误模拟为正常响应。当前失败没有通过排除模型或删除 API-Type 消除。

## 收尾与未验证项

- 所有真实测试轮次均恢复临时 SN Provider 凭据；Gateway 临时 response timeout 已恢复到原配置（默认 60000ms），reload 成功。更新后的 AICC 二进制与本地 metadata 部署保留。
- US_FULL 的清理用例单独失败：33 个测试生成 Named Data 对象无法通过缺失的 NDM Proxy 删除接口清理。它不属于上述 166 个模型/API 单元，不通过直接删除持久存储或修改数据库掩盖失败。
- 两个超时任务 `t-3a45f634551e4e139b2abf2202718b21`、`t-8c95965769204262b859cd8a34ebc9fa` 已经通过 Gateway 提交 TaskMgr Cancel；受理不等于取消完成。AICC 本地执行记录现为 `cancelled`；TaskMgr 仍为 Running，AICC cancel 的 TaskMgr 收尾返回 `control_already_pending`，跨服务终态尚未一致。未绕过鉴权或直接改写任务数据库；上游执行和计费结果未知。
- 未使用原厂 Provider 账户执行额外真实推理；这些渠道只有 T1.5 与 T2 矩阵配置覆盖。
- `aigc_image`、`aigc_audio`、`aigc_video`、`buckyos_admin` 手册已检查。CLI、canonical API 和 Agent 路径未变化，手册通过运行时发现模型，不包含本次模型列表的固定副本，因此无需修改。真实 Provider/平台覆盖以各轮报告为准。
- 没有新增第三方依赖或通用组件。后续本地修复涉及公共 storage/execution 和错误日志，必要性及未部署状态见逐项诊断；SN 私有协议修复仍在 sn_openai.rs。未创建 Git commit 或 PR。

# Grok issue 最终提交内容

状态：已按用户确认提交为 [#54](https://github.com/buckyos/sn-business/issues/54)；标题、完整正文和元数据回读一致。

仓库：`buckyos/sn-business`

标题：[sn-aicc][待诊断] Grok 4.7 对无害标记回显请求返回拒答，策略来源未明

元数据：labels、assignees、milestone、project placement 均为 none。

当前验收：2026-11-09 用户接受 Grok 为通过；[服务端归因与渠道策略](../../20261109/channel-policy/summary.md)。以下是当时按确认提交的历史正文，保持原文。

## 正文

通过 SN US_FULL 调用 `grok-4.7`，无害的固定标记回显请求返回 `I can't do that.`，未满足验收要求。此 issue 请求协助定位 SN、LiteLLM、OpenRouter 或模型侧的策略来源，暂不认定为已确认的 SN 转发缺陷。

### 复现请求与结果

- SN 入口：`https://sn.buckyos.ai/api/v1/ai`。
- AICC 方法：`chat.completions.create`；精确模型：`grok-4.7@sn-us-full-20261030-t2`。
- AICC 持久化请求仅有一条 user 消息：`Reply with the exact marker BUCKYOS-AICC-4827.`。
- `max_output_tokens = 1024`；持久化请求未配置 system 消息、tools、guidelines 或 reasoning effort。
- 预期：最终文本为 `BUCKYOS-AICC-4827`。
- 实际：最终文本为 `I can't do that.`，`finish_reason = stop`；TaskMgr 为 `Terminal/Succeeded`。
- 测试保留为内容验收失败；不是已观测到的 HTTP、JSON 解析或任务执行失败，也没有证据表明由大陆用户限制引起。

### 关联证据

- 原始执行日期：2026-10-30；2026-11-02 重新核对持久化请求和结果，本次提报没有新增付费调用，尚未验证当前线上是否仍复现。
- Run：`aicc-2026-10-30T09-01-10-151Z-69abbe3b`。
- Case：`t2.sn-ai-provider.sn-us-full-20261030-t2.grok-4.7.llm`。
- Task：`t-16d4d953decb4554be257a21261789b0`。
- Trace：`aicc-idem-9d8704d8c8ff4fdb81991cb26bb22a97`。
- Provider 返回的推理摘要将请求视作潜在 jailbreak；这里只记录该分类现象，不公开完整推理内容。
- SN 源码基准 `c23a55b4` 的 release 配置将 `grok-4.7` 转发到 `openrouter/x-ai/grok-4.7`：[配置位置](https://github.com/buckyos/sn-business/blob/c23a55b4/CD/cluster_config/release/cluster_config.json#L558)。线上实际配置和 LiteLLM 版本尚未核验。

### 需要协助核查

1. 关联上述 trace/task，确认实际出站模型、上游 provider、请求和响应摘要，核对是否注入 system/guidelines 或其他安全策略。
2. 在相同提示、参数和路由条件下比较 SN 与 OpenRouter/原厂结果，确定是代理层处理问题还是上游策略误判；目前没有付费直连对照证据。
3. 明确该拒答是否为预期策略；若是误判或配置问题，修复后保留同一无害提示验证，不通过更换提示词或放宽验收条件掩盖原问题。

本文不包含 API Key、登录 token 或完整原始响应。其他模型的 OCR/caption 识别错误、图片 inpaint 背景保留问题及本地 NDN 问题不包含在本 issue 中。

# OpenRouter AICC 测试报告

代码基线：`42cefa00f`（第二轮）、`38e75598b`（第一轮），均为 PR #671 的评审响应提交。

## 最终汇总

两轮验收互补：T1 / T1.5 每轮都是全量，T2 分两批覆盖不同的 `<provider, model, api-type>` 矩阵，合起来才是完整的 T2 覆盖。

| 层级 | 轮次 | 代码基线 | 范围 | 结果 | 真实调用 | 实际费用 |
| --- | --- | --- | --- | --- | ---: | ---: |
| T1 | 第一轮 | `38e75598b` | 全量 192 用例 | 192 passed，0 failed | 0 | $0 |
| T1.5 | 第一轮 | `38e75598b` | 全量，15 个 Provider，2371 用例 | 2371 passed，0 failed | 0 | $0 |
| T2 | 第一轮 | `38e75598b` | 14 个组合 | 14 项调用成功、产物校验通过，0 failed | 14 | $0.9772828 |
| T1 | 第二轮 | `42cefa00f` | 全量 192 用例 | 192 passed，0 failed | 0 | $0 |
| T1.5 | 第二轮 | `42cefa00f` | 全量，15 个 Provider，2371 用例 | 2371 passed，0 failed | 0 | $0 |
| T2 | 第二轮 | `42cefa00f` | 6 个组合 | 6 项调用成功，0 failed | 6 | $0.0018546 |

`review` 表示真实调用成功、产物校验通过，仅语义质量待人工复核（T2 未启用 Judge）。

### T2 覆盖矩阵

第一轮（`38e75598b`）——视频、语音合成、Claude 5.5 三件改动：

- `openrouter` / `minimax-hailuo-2.3` / `video.txt2video`、`video.img2video`
- `openrouter` / `google-gemini-3.8-flash-tts`、`google-gemini-3.1-flash-tts-preview`、`google-gemini-3.8-flash-lite-tts`、`minimax-speech-2.8-hd`、`minimax-speech-2.8-turbo` / `audio.tts`
- `claude` / `claude-sonnet-5-5`、`claude-haiku-5-5` / `llm`、`vision.ocr`、`vision.caption`
- `claude` / `claude-opus-5-5` / `llm`

第二轮（`42cefa00f`）——Claude 5.5 采样约束改为 Provider 规则后，同时覆盖直连与聚合两条渠道：

- `claude` / `claude-sonnet-5-5`、`claude-haiku-5-5`、`claude-opus-5-5` / `llm`
- `openrouter` / `anthropic/claude-sonnet-5.5`、`anthropic/claude-haiku-5.5`、`anthropic/claude-opus-5.5` / `llm`

`fal` 的改动只在测试代码（`fal-ai/imageutils/rembg` 嵌套模型 lifecycle 断言），不产生 T2 组合。

## 结论要点

- **视频费用**：两个用例均成功生成，并按 OpenRouter 结算费用入账（`video.txt2video` 实际 USD 0.485298，`cost_status=actual`），不再退化为按次计费的估算值。
- **视频参数**：`aspect_ratio` 已透传到 OpenRouter 请求。
- **语音合成**：5 个模型全部成功；`google/gemini-*-tts` 按调用方 `VoiceSpec` 的 style 选择官方 voice，全部被接受。
- **Claude 5.5 采样约束**：Anthropic 官方直连确认 `claude-sonnet-5-5` / `claude-haiku-5-5` / `claude-opus-5-5` 对 `temperature`、`top_p`、`top_k` 分别返回 `invalid_request_error: <field> is deprecated for this model`，三个都可单独触发，不带时返回 200。AICC 侧改为在 `claude.provider.json` / `openrouter.provider.json` 用 `request_rules.remove` 剔除这三个字段，第二轮 6 个组合在两条渠道上全部调用成功。
- **价格维度**：OpenRouter 实时目录中 `pricing.request=0` 与 token 费率并存时不再切换为按次计费，非零 `request` 费用不再导致整个渠道目录刷新失败；T1 与 T1.5 全量覆盖该 Provider 发现路径。

## 已知覆盖缺口

验收脚手架中所有 `llm` 用例（T1 / T1.5 / T2）都不发送 `temperature` / `top_p` / `top_k`，因此「客户端显式传采样参数」这一场景没有端到端用例。该约束目前的证据是：单元测试锁定配置命中与剥离机制、厂商 API 基线确认约束真实存在、真实调用确认链路无回归。

补齐需要在 T1.5 契约里增加一条带采样参数的 Claude 5.5 用例，并让 MockProvider 对该模型模拟厂商的拒绝行为，使该问题在 T1.5 即可暴露。

## 详细报告

### T1

- [第一轮 T1 全量：192/192](test/aicc_test/reports/acceptance/aicc-t1-2026-10-19T14-58-09-105Z-4672337f/summary.md)
- [第二轮 T1 全量：192/192](test/aicc_test/reports/acceptance/aicc-t1-2026-10-19T16-55-41-594Z-3aa37962/summary.md)

### T1.5

- [第一轮 T1.5 全量：2371/2371，15 个 Provider](test/aicc_test/reports/acceptance/t15-20261019151333-1799507/summary.md)
- [第二轮 T1.5 全量：2371/2371，15 个 Provider](test/aicc_test/reports/acceptance/t15-20261019170828-1914521/summary.md)

### T2

- [第一轮 T2 定向矩阵：14 个组合，14 次真实调用](test/aicc_test/reports/acceptance/aicc-2026-10-19T15-56-44-409Z-69d8904f/summary.md)
- [第一轮 T2 费用明细](test/aicc_test/reports/acceptance/aicc-2026-10-19T15-56-44-409Z-69d8904f/finance.md)
- [第二轮 T2 定向矩阵：6 个组合，6 次真实调用](test/aicc_test/reports/acceptance/aicc-2026-10-19T17-49-34-392Z-9a83c9f3/summary.md)
- [第二轮 T2 费用明细](test/aicc_test/reports/acceptance/aicc-2026-10-19T17-49-34-392Z-9a83c9f3/finance.md)

## 补充验证

第二轮（`42cefa00f`）：

- `cargo test -p aicc`：665 passed，0 failed。
- `cargo fmt -p aicc`、`cargo check -p aicc --all-targets`：通过。
- `cargo build -p aicc --release --target x86_64-unknown-linux-musl`：通过。

第一轮（`38e75598b`），本轮未改动验收脚手架，结果沿用：

- `pnpm run acceptance:self-test`：123 passed，0 failed。
- `pnpm run acceptance:preflight`：通过。
- `deno check acceptance/run_gateway.ts acceptance/provider_credentials.ts`：通过。

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
| T1.5 | 第三轮 | `3ecf640ae` | 采样参数定向：`claude` / `openrouter` 各 2 条（剥离 + 对照） | 4 passed，0 failed | 0 | $0 |
| T1.5 | 第三轮 | `3ecf640ae` | 全量，15 个 Provider，2375 用例 | 2375 passed，0 failed | 0 | $0 |
| T1 | 第三轮 | `3ecf640ae` | 全量 192 用例 | 192 passed，0 failed | 0 | $0 |

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
- **采样参数剥离**：`claude` 直连与 `openrouter` 聚合两条渠道各有一对用例。剥离用例在 `claude-sonnet-5-5` / `anthropic/claude-sonnet-5.5` 上显式传 `temperature=0.4`、`top_p=0.9`，`aicc.llm.input` 打印的 lowering 后 canonical 请求里这两个键都不存在；对照用例走同渠道不受约束的 `claude-sonnet-5` / `anthropic/claude-sonnet-5`，两个值原样出现在 wire 请求里，说明断言不是「脚手架没发参数」造成的假通过。MockProvider 同时按厂商行为对这两个模型回 400，配置一旦回退，T1.5 立即失败。
- **价格维度**：OpenRouter 实时目录中 `pricing.request=0` 与 token 费率并存时不再切换为按次计费，非零 `request` 费用不再导致整个渠道目录刷新失败；T1 与 T1.5 全量覆盖该 Provider 发现路径。

## 已知覆盖缺口

采样参数的端到端用例已补齐（第三轮）。剩余缺口是 OpenRouter 视频契约缺少 `aspect_ratio` 的固化断言：
T2 已确认真实请求里带上了该字段，但 T1.5 的 `openrouter.videos.v1` 契约没有对应的取值断言，回归时只能靠 T2 发现。

第三轮只改验收脚手架与契约断言，没有改 Provider 配置，也没有新增 `<provider, model, api-type>` 组合，因此不跑 T2。

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

### 第三轮（采样参数）

- [T1.5 定向 4 条：4/4](test/aicc_test/reports/acceptance/t15-20261020034609-1992371/summary.md)
- [T1.5 全量：2375/2375，15 个 Provider](test/aicc_test/reports/acceptance/t15-20261020035602-2008042/summary.md)
- [T1 全量：192/192](test/aicc_test/reports/acceptance/aicc-t1-2026-10-20T04-24-01-234Z-d0f3492e/summary.md)

## 补充验证

第三轮（`3ecf640ae`），只改验收脚手架：

- `node --experimental-strip-types --test acceptance/acceptance.test.ts`：123 passed，0 failed。
- 契约校验直接对四个 `<渠道, 模型>` 组合验证模型级禁字段：`claude-sonnet-5-5` 与 `anthropic/claude-sonnet-5.5` 收到 `temperature` / `top_p` 时报 `is forbidden`，`claude-sonnet-5` 与 `anthropic/claude-sonnet-5` 不受影响。

第二轮（`42cefa00f`）：

- `cargo test -p aicc`：665 passed，0 failed。
- `cargo fmt -p aicc`、`cargo check -p aicc --all-targets`：通过。
- `cargo build -p aicc --release --target x86_64-unknown-linux-musl`：通过。

第一轮（`38e75598b`），本轮未改动验收脚手架，结果沿用：

- `pnpm run acceptance:self-test`：123 passed，0 failed。
- `pnpm run acceptance:preflight`：通过。
- `deno check acceptance/run_gateway.ts acceptance/provider_credentials.ts`：通过。

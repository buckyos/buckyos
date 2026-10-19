# OpenRouter AICC 测试报告

日期：2026-10-19
代码基线：`38e75598b`（PR #671 评审修改）

## 最终汇总

| 层级 | 范围 | 结果 | 真实调用 | 实际费用 |
| --- | --- | --- | ---: | ---: |
| T1 | 全量 | 192 passed，0 failed | 0 | $0 |
| T1.5 | 全量，15 个 Provider | 2371 passed，0 failed | 0 | $0 |
| T2 | 被修改的 `<provider, model, api-type>` 矩阵，14 个组合 | 14 项调用成功、产物校验通过、0 failed，待人工语义复核 | 14 | $0.9772828 |

本轮改动涉及 `openrouter`、`claude`、`fal`。`fal` 的改动只在测试代码（`fal-ai/imageutils/rembg` 嵌套模型 lifecycle 断言），不产生 T2 组合。T2 执行的组合为：

- `openrouter` / `minimax-hailuo-2.3` / `video.txt2video`、`video.img2video`
- `openrouter` / `google-gemini-3.8-flash-tts`、`google-gemini-3.1-flash-tts-preview`、`google-gemini-3.8-flash-lite-tts`、`minimax-speech-2.8-hd`、`minimax-speech-2.8-turbo` / `audio.tts`
- `claude` / `claude-sonnet-5-5`、`claude-haiku-5-5` / `llm`、`vision.ocr`、`vision.caption`
- `claude` / `claude-opus-5-5` / `llm`（同家族对照，确认其未声明 `fixed_sampling` 与实际行为一致）

T2 未启用 Judge，`review` 表示真实调用成功、产物校验通过，仅语义质量待人工复核。本轮矩阵内没有免费或带免费额度的模型，按预估成本从低到高执行：Claude → 语音合成 → 视频生成。

结论要点：

- 视频：两个用例均成功生成，并按 OpenRouter 结算费用入账（`video.txt2video` 实际 USD 0.485298，`cost_status=actual`），不再退化为按次计费的估算值。
- 语音合成：5 个模型全部成功；`google/gemini-*-tts` 按调用方 `VoiceSpec` 的 style 选择官方 voice，全部被接受。
- Claude 5.5 采样：Anthropic 官方直连确认 `temperature` 对 `claude-sonnet-5-5` 返回 `invalid_request_error: temperature is deprecated for this model`；同一模型不带 `temperature` 返回 200。AICC 侧 6 个 5.5 用例全部成功且未出现该错误。
- 价格维度：OpenRouter 实时目录中 `pricing.request=0` 与 token 费率并存时不再切换为按次计费，非零 `request` 费用不再导致整个渠道目录刷新失败；T1 与 T1.5 全量覆盖该 Provider 发现路径。

## 详细报告

### T1

- [T1 全量：192/192](test/aicc_test/reports/acceptance/aicc-t1-2026-10-19T14-58-09-105Z-4672337f/summary.md)

### T1.5

- [T1.5 全量：2371/2371，15 个 Provider](test/aicc_test/reports/acceptance/t15-20261019151333-1799507/summary.md)

### T2

- [T2 定向矩阵：14 个组合，14 次真实调用，0 failed](test/aicc_test/reports/acceptance/aicc-2026-10-19T15-56-44-409Z-69d8904f/summary.md)
- [T2 费用明细](test/aicc_test/reports/acceptance/aicc-2026-10-19T15-56-44-409Z-69d8904f/finance.md)

## 补充验证

- `cargo test -p aicc`：664 passed，0 failed。
- `cargo test -p aicc openrouter`：26 passed；`claude`：24 passed；`fal_queue`：8 passed。
- `pnpm run acceptance:self-test`：123 passed，0 failed。
- `pnpm run acceptance:preflight`：通过。
- `deno check acceptance/run_gateway.ts acceptance/provider_credentials.ts`：通过。
- `cargo fmt -p aicc -- --check`、`cargo check -p aicc --all-targets`：通过。

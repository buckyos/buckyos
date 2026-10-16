# OpenRouter AICC 测试报告

日期：2026-10-16

## 最终汇总

- OpenRouter 模型范围取其官方实时模型目录与 AICC 当前内置原厂模型目录的交集，共纳入 110 个物理模型；OpenRouter 可配置路由的逻辑 ID、别名和变体 ID 不进入物理模型目录。
- 模型 ID、输入/输出模态、operations、variants 和生命周期由官方目录及原厂能力约束共同确定；OpenRouter 动态目录与 Provider 规则只能和原厂 `*.model.json` 取交集或继续收窄，不能新增原厂未声明的 api-type。价格从 OpenRouter 官方模型发现结果动态装载，实际调用费用以 OpenRouter 返回的 `reported_cost` 入账。
- OpenRouter 已覆盖 Chat Completions、Responses、Embedding、Rerank、图片、视频、音乐、TTS 与 ASR 所需的协议映射；TTS 使用模型对应的官方 voice 和输出格式。
- Gemini 图像模型已按原厂模型卡重新校准基础能力：`gemini-3.1-flash-image` 与 `gemini-nano-banana-2.1` 均为 131072 输入、32768 输出并支持 reasoning/web_search；`gemini-3-pro-image` 为 65536/32768 并支持 reasoning/web_search；`gemini-3.1-flash-lite-image` 为 65536/4096 且仅支持 reasoning；`gemini-2.5-flash-image` 为 65536/32768，不声明 reasoning/web_search。五个模型均只配置 `image.txt2img`、`image.img2img`，OpenRouter 规则仅可继续收窄。
- T1 与 T1.5 全量通过，因此按顺序进入 T2。T2 先执行免费模型，再执行新增 TTS 定向验证，最后将全量矩阵分为三片完成真实调用。
- Gemini medium 变体线上回归中，`google/gemini-3.8-flash` 与 `google/gemini-2.5-flash` 均接受 `reasoning.effort=medium`，OpenRouter 渠道保留真实 `medium` 映射。OpenRouter 当前实时目录不提供物理模型 `google/lyria-3.5`，因此没有为该 `<model, api-type>` 发起 T2，也未用 `google/lyria-3-pro-preview` 或 `google/lyria-3-clip-preview` 替代。

| 层级 | 范围 | 结果 | 真实调用 | 实际费用 |
| --- | --- | --- | ---: | ---: |
| T1 | 全量 | 189 passed，0 failed | 0 | $0 |
| T1.5 | 全量 | 2549 passed，0 failed | 0 | $0 |
| T2 免费优先 | Cohere Rerank | 1 passed，0 failed | 1 | $0 |
| T2 新增 TTS 定向 | 3 个模型 | 3 review，0 failed | 3 | $0 |
| T2 历史全量（三片合计） | 修正前 262 个用例，含后来排除的 33 个越权组合 | 93 passed，154 review，15 failed | 267 | $1.6070398512928002 |
| T2 历史失败用例重试 | 修正前 15 个 failed 用例 | 4 review，11 failed | 15 | $0.055082543175 |
| T2 最终有效矩阵 | 110 个物理模型、229 个用例，覆盖 229/229 | 90 passed，134 review，5 failed | 由各轮报告合并去重 | 见各轮报告 |
| 收窄后 T1 | 全量 | 130 passed，0 failed | 0 | $0 |
| 收窄后 T1.5 | Gemini、Qwen、OpenRouter | 1271 passed，0 failed | 0 | $0 |
| Gemini 元数据校准后 T1 | 全量；Manifest 130/130 | 189 passed，0 failed | 0 | $0 |
| Gemini 元数据校准后 T1.5 | 全量；Manifest 2492/2492 | 2492 passed，0 failed | 0 | $0 |
| 最新 T1 | 全量 | 189 passed，0 failed | 0 | $0 |
| 最新 T1.5 | 全量；Manifest 2510/2510 | 2510 passed，0 failed | 0 | $0 |

T2 全量轮次中的 `review` 因使用 `--no-judge`，调用与基础产物校验已成功，但未进行 Judge 语义判定。Gemini 图片模型的 LLM/视觉理解组合及 Qwen 文本模型的视觉理解组合不属于原厂能力，最终矩阵已排除这 33 个组合；此前针对这些组合的 OpenRouter 调用/Judge 结果不作为能力支持证据，也无需继续重试。最终 229 个 `<model, api-type>` 均已有非 skipped 的 T2 结果。

## 详细报告

### T1

- [T1 全量：189/189](test/aicc_test/reports/acceptance/aicc-t1-2026-10-15T21-24-24-918Z-7e42a653/summary.md)
- [能力收窄后 T1 全量：130/130](test/aicc_test/reports/acceptance/aicc-t1-2026-10-16T04-46-28-394Z-46c298bb/summary.md)
- [Gemini 元数据校准后 T1 全量：189 passed，Manifest 130/130](test/aicc_test/reports/acceptance/aicc-t1-2026-10-16T05-21-20-370Z-48a6e256/summary.md)
- [最新 T1 全量：189/189](test/aicc_test/reports/acceptance/aicc-t1-2026-10-16T07-42-46-007Z-845ce456/summary.md)
- [能力基线修正后 T1 全量：189/189](test/aicc_test/reports/acceptance/aicc-t1-2026-10-16T08-43-15-887Z-8631ec17/summary.md)

### T1.5

- [T1.5 全量：2549/2549](test/aicc_test/reports/acceptance/t15-20261015212854-3254221/summary.md)
- [能力收窄后 Gemini/Qwen/OpenRouter：1271/1271](test/aicc_test/reports/acceptance/t15-20261016045122-3847997/summary.md)
- [Gemini 元数据校准后 T1.5 全量：2492/2492](test/aicc_test/reports/acceptance/t15-20261016052615-3901972/summary.md)
- [最新 T1.5 全量：2510/2510](test/aicc_test/reports/acceptance/t15-20261016074720-4116059/summary.md)
- [能力基线修正后 T1.5 全量：2510/2510](test/aicc_test/reports/acceptance/t15-20261016084809-10062/summary.md)

### T2

- [免费模型优先验证：Cohere Rerank](test/aicc_test/reports/acceptance/aicc-2026-10-15T22-05-07-884Z-424c7ddf/summary.md)
- [新增 TTS 定向验证](test/aicc_test/reports/acceptance/aicc-2026-10-15T22-05-24-363Z-0080672a/summary.md)
- [全量分片 1：88 个用例](test/aicc_test/reports/acceptance/aicc-2026-10-15T22-05-45-955Z-8721c006/summary.md)
- [全量分片 2：87 个用例](test/aicc_test/reports/acceptance/aicc-2026-10-15T22-16-03-727Z-334f089f/summary.md)
- [全量分片 3：87 个用例](test/aicc_test/reports/acceptance/aicc-2026-10-15T22-29-10-627Z-17f97a06/summary.md)
- [失败用例重试：15 个用例](test/aicc_test/reports/acceptance/aicc-2026-10-16T04-12-33-252Z-53597aa6/summary.md)
- [Gemini/Qwen 原厂能力预检：不支持的组合已排除](test/aicc_test/reports/acceptance/aicc-2026-10-16T04-25-25-123Z-882e0084/summary.md)
- [Gemini medium 变体线上回归：2 个指定模型均接受真实 medium](test/aicc_test/reports/acceptance/openrouter-medium-regression-20261016/summary.md)
- [最终有效矩阵预检：229 个组合，0 baseline mismatch](test/aicc_test/reports/acceptance/aicc-2026-10-16T09-31-52-487Z-b8a2c618/summary.md)

## 补充验证

- `cargo test -p aicc -- --test-threads=1`：649 passed，0 failed。
- `cargo test -p aicc openrouter -- --test-threads=1`：20 passed，0 failed（含聚合平台不得扩展原厂 api-type 的防回归用例）。
- `cargo test -p aicc gemini -- --test-threads=1`：31 passed，0 failed（含 Gemini 图像模型原厂能力矩阵及 Thinking/Search 请求编码用例）。
- `pnpm acceptance:self-test`：121 passed，0 failed。
- `pnpm acceptance:preflight`：通过，T1.5 静态矩阵 878 个用例。
- `deno check acceptance/*.ts`：通过。
- `cargo fmt -p aicc -- --check`：通过。
- `git diff --check`：通过。

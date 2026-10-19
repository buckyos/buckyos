# AICC E2E Acceptance Report

## 人工复核与执行口径（2026-10-19）

下方自动结果原样保留。80 个模型/API 单元的原始结果是 10 passed、48 failed、22 review；另有 usage 审计和 artifact discovery 两项基础设施失败，自动总数因此为 82。

- 42 个矩阵单元因 Gateway 登录过期返回 `ExpiredSignature`，未形成对应的 AICC 推理任务；这些是测试客户端失败，不是 42 个 SN 模型失败。该轮 `84` 的计数是 RPC 推理尝试（含前置生成），其中 AICC 实际接受 42 个任务，不能把全部 84 次解释成 Provider 推理。
- 另有 5 个 HTTP 500 单元和 1 个 computer-use 失败。500 单元为 `gpt-5.4-pro/image.txt2img`、`gpt-5.4-pro/vision.ocr`、`gpt-5.4/image.txt2img`、`gpt-5.4/image.img2img`、`gpt-5.5-pro/vision.ocr`。这些请求断开时任务仍未终止；不能由此断言模型已完成生成，也不能认定为下载鉴权错误。
- `gpt-5.6-luna/agent.computer_use` 返回调用方未允许的动作，任务进入 Failed，AICC 未执行该动作。
- 该轮 computer-use 输入为 1×1 图片，任务要求报告标题，而断言要求固定坐标点击，属于无效语义用例；不由该结果推断模型不支持 computer-use。有效验收见后续真实尺寸按钮截图用例。
- 使用新登录只读恢复审计，32 个 passed/review 单元均有且仅有 1 条匹配 exact model 和 ProviderInstance 的 durable usage 与 route trace，32/32 通过。原 usage 审计失败由登录过期引起。
- 5 个残留非终态任务已按本轮 run ID 取消，均核验为 `Terminal/Canceled`。
- SN 未返回可识别的实际 USD 费用。`$0.84` 是对 84 次 RPC 尝试的保守估算，不是实付账单；以 AICC 接受的 42 项任务计算，同一 `$0.01/项` 估算为 `$0.42`，真实费用仍未知。

22 个 review 单元人工复核：19 passed、2 failed、1 review。复核不改变下方原始 runner 状态，也不将后续补测替换为本轮通过。

| 模型 | API | 人工结论 | 内容证据 |
|---|---|---|---|
| gpt-4o-mini-transcribe | audio.asr | passed | 转写为“今天的测试编号是四八二七。”，符合语音 fixture |
| gpt-4o-transcribe-diarize | audio.asr | passed | 转写保留“今天的测试”“编号”“4827” |
| gpt-4o-mini-tts | audio.tts | review | MP3 文件非空且可识别；未完成听审 |
| gpt-5.3-codex | vision.caption | passed | 正确描述粉色花朵、黄色花心、绿茎叶及棕色椭圆背景 |
| gpt-5.3-codex | vision.ocr | passed | `6ICKYOS-OV-4827` 含 rubric 要求的可见数字 4827；不代表全文 OCR 无误 |
| gpt-5.4-mini | vision.caption | passed | 正确描述 fixture 的粉色花朵、绿茎叶及背景 |
| gpt-5.4-mini | vision.ocr | passed | `BUCKYNS-0V-4827` 含 4827；其余字符有误识别 |
| gpt-5.4-mini | image.txt2img | passed | 下载图片具有蓝色方块及清晰可读的 4827；有光晕，但不违反该最小 rubric |
| gpt-5.4-nano | vision.caption | passed | 正确描述粉色花朵及绿叶、黄色中心、棕色椭圆背景 |
| gpt-5.4-nano | vision.ocr | failed | 输出 `E U C K Y ' U S — D ' U I — 4 B U I F ?`，缺少可见标记 4827 |
| gpt-5.4-nano | image.txt2img | failed | 图片为蓝色方形画布，未出现数字 4827 |
| gpt-5.4-pro | vision.caption | passed | 正确描述花朵、茎叶及背景 |
| gpt-5.4 | vision.caption | passed | 正确描述花朵、茎叶及背景 |
| gpt-5.4 | vision.ocr | passed | `Visible text: 6ICKYOS-0V-4827` 含 4827；其余字符不完全准确 |
| gpt-5.5-pro | vision.caption | passed | 正确描述粉色花朵、绿叶、黄色花心与椭圆背景 |
| gpt-5.5-pro | image.txt2img | passed | 蓝色方块内清晰显示 4827 |
| gpt-5.5 | vision.caption | passed | 正确描述花朵及上下椭圆背景 |
| gpt-5.5 | vision.ocr | passed | `BUCKY05-0V-4827` 含 4827；其余字符不完全准确 |
| gpt-5.5 | image.txt2img | passed | 蓝色方块内清晰显示 4827 |
| gpt-5.6-luna | vision.caption | passed | 正确描述中央花朵、茎叶与五个背景椭圆 |
| gpt-5.6-luna | vision.ocr | passed | `BLOCKYS-04-4827` 含 4827；其余字符不完全准确 |
| gpt-5.6-luna | image.txt2img | passed | 蓝色正方形画布上清晰显示 4827，符合最小内容要求 |

生成图片在清理前经 Gateway NDN 接口下载；文件 SHA-256 与该轮本地 case artifact audit 对应。复核未增加模型调用。后续登录过期单元的补测报告另行链接。

- Run: `aicc-2026-10-19T07-28-29-275Z-eb259b9a`
- Commit: `2dd069965d82127790c093dc5fc04ec331b9efa9`
- Capability baseline: `2026-10-19.3`
- Real model calls: 84/80
- Planned maximum cost: $0.800000
- Known actual / unknown estimated exposure: $0.000000 / $0.840000
- Total exposure / budget: $0.840000 / $1.000000
- Unknown-cost calls: 84; budget exceeded: false
- Results: passed=10, failed=50, provider_restricted=0, skipped=0, not_applicable=0, review=22

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config test/aicc_test/aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.image.txt2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.vision.ocr --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.txt2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5-pro.vision.ocr --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.txt2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.llm --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.caption --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.ocr --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.txt2img --case t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.llm
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-ai-provider-default.glm-5.3.llm | T2 | sn-ai-provider/sn-ai-provider-default | glm-5.3@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.glm-5.llm | T2 | sn-ai-provider/sn-ai-provider-default | glm-5@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-4o-mini-transcribe.audio.asr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-4o-mini-transcribe@sn-ai-provider-default | audio.asr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-4o-mini-tts.audio.tts | T2 | sn-ai-provider/sn-ai-provider-default | gpt-4o-mini-tts@sn-ai-provider-default | audio.tts | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-4o-transcribe-diarize.audio.asr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-4o-transcribe-diarize@sn-ai-provider-default | audio.asr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.3-codex.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.3-codex@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.3-codex.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.3-codex@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.3-codex.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.3-codex@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro@sn-ai-provider-default | vision.ocr | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.4@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5-pro.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5-pro@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5-pro.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5-pro@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5-pro.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5-pro@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5-pro.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5-pro@sn-ai-provider-default | vision.ocr | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.5@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna@sn-ai-provider-default | image.txt2img | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna@sn-ai-provider-default | llm | passed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna@sn-ai-provider-default | vision.caption | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna@sn-ai-provider-default | vision.ocr | review |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | llm | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | vision.caption | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol@sn-ai-provider-default | vision.ocr | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | llm | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | vision.caption | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra@sn-ai-provider-default | vision.ocr | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | llm | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | vision.caption | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-astra@sn-ai-provider-default | vision.ocr | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | llm | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | vision.caption | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-luna@sn-ai-provider-default | vision.ocr | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | agent.computer_use | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.llm | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | llm | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | vision.caption | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | gpt-6-sol@sn-ai-provider-default | vision.ocr | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.img2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-image-2@sn-ai-provider-default | image.img2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.inpaint | T2 | sn-ai-provider/sn-ai-provider-default | gpt-image-2@sn-ai-provider-default | image.inpaint | failed |
| t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.txt2img | T2 | sn-ai-provider/sn-ai-provider-default | gpt-image-2@sn-ai-provider-default | image.txt2img | failed |
| t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.llm | T2 | sn-ai-provider/sn-ai-provider-default | kimi-k3@sn-ai-provider-default | llm | failed |
| t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.caption | T2 | sn-ai-provider/sn-ai-provider-default | kimi-k3@sn-ai-provider-default | vision.caption | failed |
| t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.ocr | T2 | sn-ai-provider/sn-ai-provider-default | kimi-k3@sn-ai-provider-default | vision.ocr | failed |
| t2.sn-ai-provider.sn-ai-provider-default.text-embedding-3-large.embedding.text | T2 | sn-ai-provider/sn-ai-provider-default | text-embedding-3-large@sn-ai-provider-default | embedding.text | failed |
| t2.sn-ai-provider.sn-ai-provider-default.text-embedding-3-small.embedding.text | T2 | sn-ai-provider/sn-ai-provider-default | text-embedding-3-small@sn-ai-provider-default | embedding.text | failed |
| t2.usage.durable_audit | T2 | -/- | - | usage.query | failed |
| t2.cleanup.artifact_discovery | T2 | -/- | - | task.get | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-ai-provider-default | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-4o-mini-transcribe | gpt-4o-mini-transcribe | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-4o-mini-tts | gpt-4o-mini-tts | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-4o-transcribe-diarize | gpt-4o-transcribe-diarize | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.3-codex | gpt-5.3-codex | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4 | gpt-5.4 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-realtime-2.1 | gpt-realtime-2.1 | filtered | unsupported_canonical_protocol | - |
| sn-ai-provider/sn-ai-provider-default | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | sora-2-pro | sora-2-pro | filtered | unsupported_canonical_protocol | - |
| sn-ai-provider/sn-ai-provider-default | sora-2 | sora-2 | filtered | unsupported_canonical_protocol | - |
| sn-ai-provider/sn-ai-provider-default | text-embedding-3-large | text-embedding-3-large | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | text-embedding-3-small | text-embedding-3-small | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | glm-5.3 | glm-5.3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | glm-5 | glm-5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-4o-mini-transcribe | gpt-4o-mini-transcribe | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-4o-mini-tts | gpt-4o-mini-tts | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-4o-transcribe-diarize | gpt-4o-transcribe-diarize | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.3-codex | gpt-5.3-codex | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-mini | gpt-5.4-mini | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-nano | gpt-5.4-nano | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4-pro | gpt-5.4-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.4 | gpt-5.4 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5-pro | gpt-5.5-pro | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.5 | gpt-5.5 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-luna | gpt-5.6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-sol | gpt-5.6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-5.6-terra | gpt-5.6-terra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-astra | gpt-6-astra | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-luna | gpt-6-luna | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | kimi-k3 | kimi-k3 | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | text-embedding-3-large | text-embedding-3-large | included | physical model | - |
| sn-ai-provider/sn-ai-provider-default | text-embedding-3-small | text-embedding-3-small | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-mini.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-nano.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4-pro.vision.ocr.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.4.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5-pro.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5-pro.vision.ocr.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.5.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-luna.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.agent.computer_use.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.llm.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.caption** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.caption.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-sol.vision.ocr.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.agent.computer_use.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.llm.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.caption** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.caption.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-5.6-terra.vision.ocr.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.agent.computer_use.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.llm.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.vision.caption** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.vision.caption.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-astra.vision.ocr.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.agent.computer_use.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.llm.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.vision.caption** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.vision.caption.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-luna.vision.ocr.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.agent.computer_use.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.llm.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.vision.caption** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.vision.caption.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-6-sol.vision.ocr.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.inpaint** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.inpaint.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.gpt-image-2.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.llm.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.caption** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.caption.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.kimi-k3.vision.ocr.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.text-embedding-3-large.embedding.text** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.text-embedding-3-large.embedding.text.json
- **defect.aicc.t2.sn-ai-provider.sn-ai-provider-default.text-embedding-3-small.embedding.text** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: Invalid token: JWT decode error: Invalid token: JWT decode error:ExpiredSignature. Evidence: cases/t2.sn-ai-provider.sn-ai-provider-default.text-embedding-3-small.embedding.text.json

## Cleanup

Status: failed

- removed 0 uploaded fixture object(s)
- removed 10 generated output object(s)

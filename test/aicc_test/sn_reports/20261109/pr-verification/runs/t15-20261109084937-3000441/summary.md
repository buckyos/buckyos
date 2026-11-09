# AICC E2E Acceptance Report

- Run: `t15-20261109084937-3000441`
- Commit: `8326b53f2eed96f223451ca987d9ddfb9c9324c4`
- Capability baseline: `2026-11-02.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=1268, failed=5, provider_restricted=0, skipped=70, not_applicable=0, review=0
- Manifest coverage: 1342/2146 (62.53%); passed=1268, failed=4

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
AICC_T15_ALLOW_CONFIG_MUTATION=true pnpm run acceptance:t1.5 -- --gateway-url "http://127.0.0.1:3180" --mock-base-url "http://127.0.0.1:18081" --mock-control-url "http://127.0.0.1:18081" --allow-config-mutation --case t1.5.openai.openai.videos.v1.video.generate.error.invalid_request --case t1.5.openai.openai.videos.v1.video.generate.error.not_found --case t1.5.openai.openai.videos.edits.v1.video.edit.error.invalid_request --case t1.5.openai.openai.videos.edits.v1.video.edit.error.not_found
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.server-default-route | T1.5 | sn-ai-provider/t15-20261109084937-3000441-server-default | qwen3.7-plus@t15-20261109084937-3000441-server-default | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-6-astra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-6-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.6-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.6-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.5-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.4.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.4-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.4-mini.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.4-nano.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-5.3-codex.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.glm-5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.glm-5.3.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.kimi-k3.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.qwen3.6-plus.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.6-plus@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.qwen3.7-plus.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.7-plus@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.qwen3.8-max-0902.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-0902@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.ui-tars-1.5-7b.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | ui-tars-1.5-7b@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.voxtral-small-24b-2507.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-6.1-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-6.1-terra-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-6.1-sol-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.claude-opus-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.claude-sonnet-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.claude-haiku-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gemini-4-argon.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gemini-3.8-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.grok-4.7.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.qwen3.8-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.qwen3.8-omni-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.qwen3.8-max-prime.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.qwen3.8-27b.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-27b@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.qwen3.8-2.4t-a95b.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-2.4t-a95b@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.kimi-k3.1-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.glm-5.3-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.glm-5.3-flashx.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.glm-5.3-prime.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-prime@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.minimax-m3.1-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.deepseek-v4.1-flash-1017.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.deepseek-v4-pro-0813.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4-pro-0813@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.inline-artifact-provider-state | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.stream | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.stream-interrupted | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.qwen3.7-plus.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.7-plus@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.ui-tars-1.5-7b.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | ui-tars-1.5-7b@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.claude-opus-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.claude-sonnet-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.claude-haiku-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.gemini-4-argon.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.gemini-3.8-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.grok-4.7.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.qwen3.8-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.kimi-k3.1-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.glm-5.3-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.glm-5.3-flashx.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.minimax-m3.1-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.deepseek-v4.1-flash-1017.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-6-astra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-6-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.6-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.6-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.5-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.4.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.4-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.4-mini.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.4-nano.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-5.3-codex.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.kimi-k3.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.qwen3.6-plus.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.6-plus@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.qwen3.7-plus.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.7-plus@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.qwen3.8-max-0902.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-0902@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.ui-tars-1.5-7b.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | ui-tars-1.5-7b@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-6.1-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-6.1-terra-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-6.1-sol-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.claude-opus-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.claude-sonnet-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.claude-haiku-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gemini-4-argon.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gemini-3.8-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.grok-4.7.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.qwen3.8-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.qwen3.8-omni-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.qwen3.8-max-prime.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.kimi-k3.1-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.glm-5.3-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.glm-5.3-flashx.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.minimax-m3.1-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.deepseek-v4.1-flash-1017.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-6-astra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.6-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.6-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.5-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.4.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.4-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.4-mini.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.4-nano.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.3-codex.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-6.1-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-6.1-terra-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-6.1-sol-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.background-ignored | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async-failed | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async_poll_timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async_artifact_unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async-cancel | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6-astra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6-astra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6-luna.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.6-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.6-sol.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.6-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.6-terra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.6-luna.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.5-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.5-pro.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.5.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4-pro.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4-mini.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4-mini.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4-nano.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4-nano.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-sol.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-terra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-terra-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-terra-pro.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-sol-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-sol-pro.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6-astra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6-astra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6-luna.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.6-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.6-sol.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.6-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.6-terra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.6-luna.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.5.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.5.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.4.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.4.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.4-mini.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.4-mini.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.4-nano.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.4-nano.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-sol.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-terra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-terra-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-terra-pro.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-sol-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-sol-pro.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-6-astra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-6-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-5.6-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-5.6-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-5.6-luna.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-6.1-sol.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-6.1-terra-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-6.1-sol-pro.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.model.text-embedding-3-large.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-large@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.model.text-embedding-3-small.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.model.qwen3-embedding-8b.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3-embedding-8b@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | text-embedding-3-small@t15-20261109084937-3000441-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.5-flare.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-flare@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-flare@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.5-sunburst.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.5-flare.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-flare@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-flare@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.5-sunburst.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.5-flare.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-flare@t15-20261109084937-3000441-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-flare@t15-20261109084937-3000441-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.5-sunburst.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261109084937-3000441-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261109084937-3000441-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-image-2@t15-20261109084937-3000441-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.model.gpt-4o-mini-tts.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.model.gpt-4o-mini-tts.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-speech.v1.audio.tts.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-tts@t15-20261109084937-3000441-sn-ai-provider | audio.tts | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.model.gpt-4o-mini-transcribe.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-mini-transcribe@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.model.gpt-4o-transcribe-diarize.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-audio-transcriptions.v1.audio.asr.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-4o-transcribe-diarize@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.model.gemini-4-argon.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.model.gemini-3.8-flash.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | voxtral-small-24b-2507@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.model.wan-3.0.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.model.wan-3.0.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.video-keyframe | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async-failed | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async_poll_timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.async_artifact_unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.error.upstream_json_required | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.generated-artifact-id.same-provider | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async-failed | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async_poll_timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async_artifact_unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.upstream_json_required | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.generated-artifact-id.same-provider | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async-failed | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async_poll_timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async_artifact_unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.upstream_json_required | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.success | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.authentication | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.unavailable | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.timeout | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-haiku-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-haiku-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-haiku-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-haiku-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-haiku-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-haiku-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-haiku-5.5-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-haiku-5.5-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-haiku-5.5-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-haiku-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-haiku-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-haiku-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-haiku-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-haiku-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-haiku-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-haiku-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-opus-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-opus-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-opus-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-opus-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-opus-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-opus-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-opus-5.5-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-opus-5.5-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-opus-5.5-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-opus-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-opus-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-opus-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-opus-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-opus-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-opus-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-opus-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-sonnet-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-sonnet-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-sonnet-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-sonnet-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-sonnet-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-sonnet-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-sonnet-5.5-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-sonnet-5.5-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-sonnet-5.5-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-sonnet-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-sonnet-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-sonnet-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.claude-sonnet-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.claude-sonnet-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.claude-sonnet-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | claude-sonnet-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.deepseek-v4-pro-0813-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4-pro-0813:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.deepseek-v4-pro-0813-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4-pro-0813:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.deepseek-v4-pro-0813-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4-pro-0813:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.deepseek-v4-pro-0813-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4-pro-0813:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.deepseek-v4.1-flash-1017-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.deepseek-v4.1-flash-1017-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.deepseek-v4.1-flash-1017-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.deepseek-v4.1-flash-1017-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.deepseek-v4.1-flash-1017-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.deepseek-v4.1-flash-1017-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.deepseek-v4.1-flash-1017-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.deepseek-v4.1-flash-1017-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.deepseek-v4.1-flash-1017-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.deepseek-v4.1-flash-1017-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.deepseek-v4.1-flash-1017-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.deepseek-v4.1-flash-1017-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | deepseek-v4.1-flash-1017:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.variant.gemini-3.8-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gemini-3.8-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gemini-3.8-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.gemini-3.8-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.variant.gemini-3.8-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gemini-3.8-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gemini-3.8-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.gemini-3.8-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.variant.gemini-3.8-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gemini-3.8-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gemini-3.8-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.gemini-3.8-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-3.8-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.variant.gemini-4-argon-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gemini-4-argon-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gemini-4-argon-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.gemini-4-argon-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.variant.gemini-4-argon-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gemini-4-argon-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gemini-4-argon-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.gemini-4-argon-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.variant.gemini-4-argon-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | audio.asr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gemini-4-argon-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gemini-4-argon-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.gemini-4-argon-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gemini-4-argon:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flash-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flash-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flash-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flash:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flashx-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flashx-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flashx-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flashx-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flashx-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flashx-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flashx-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flashx-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flashx-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-flashx:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-prime-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-prime:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-prime-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-prime:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-prime-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3-prime:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5.3:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5-reasoning-thinking | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | glm-5:reasoning-thinking@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.3-codex-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.3-codex-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.3-codex-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.3-codex-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.3-codex:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-mini-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-mini-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-mini-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-mini-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-mini-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-mini:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-nano-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-nano-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-nano-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-nano-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-nano-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-nano:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.4-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.4-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.4-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.4-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.4-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.4:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.5-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.5-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.5-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.5-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.5-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.5-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.5-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.5-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.5-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.5-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.5-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.5-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.5-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.5-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.5-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.5-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.5-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.5-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.5:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-luna-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-sol-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-5.6-terra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-5.6-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-astra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-astra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-astra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-astra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-astra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-astra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-astra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-astra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-astra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-astra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-astra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-astra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-astra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-astra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-astra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-astra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-astra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-astra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-astra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-astra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-astra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-astra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-astra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-luna-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-luna-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-luna-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-luna-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-luna-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-luna-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-luna-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-luna-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-luna:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-sol-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6-sol-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6-sol-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-pro-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-pro-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-sol-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-sol:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-pro-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-pro-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-pro-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-pro-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-pro-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-pro-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-pro-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-pro-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra-pro:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | grok-4.7:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.kimi-k3.1-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.kimi-k3.1-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.kimi-k3.1-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.kimi-k3.1-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.kimi-k3.1-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.kimi-k3.1-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.kimi-k3.1-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.kimi-k3.1-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.kimi-k3.1-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3.1-pro:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.kimi-k3-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.kimi-k3-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.kimi-k3-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.kimi-k3-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.kimi-k3-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.kimi-k3-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.kimi-k3-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.kimi-k3-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.kimi-k3-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | kimi-k3:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.minimax-m3.1-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.minimax-m3.1-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.minimax-m3.1-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-high@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.minimax-m3.1-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.minimax-m3.1-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.minimax-m3.1-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.minimax-m3.1-flash-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.minimax-m3.1-flash-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.minimax-m3.1-flash-reasoning-max | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-max@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.minimax-m3.1-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.minimax-m3.1-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.minimax-m3.1-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.minimax-m3.1-flash-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.minimax-m3.1-flash-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.minimax-m3.1-flash-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | minimax-m3.1-flash:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.6-plus-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.6-plus:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.6-plus-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.6-plus:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.6-plus-reasoning-thinking | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.6-plus:reasoning-thinking@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.6-plus-reasoning-thinking | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.6-plus:reasoning-thinking@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.7-plus-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.7-plus:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.7-plus-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.7-plus:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.qwen3.7-plus-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.7-plus:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.7-plus-reasoning-thinking | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.7-plus:reasoning-thinking@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.7-plus-reasoning-thinking | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.7-plus:reasoning-thinking@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.qwen3.7-plus-reasoning-thinking | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.7-plus:reasoning-thinking@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-2.4t-a95b-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-2.4t-a95b:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-2.4t-a95b-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-2.4t-a95b:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-2.4t-a95b-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-2.4t-a95b:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-27b-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-27b:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-27b-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-27b:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-27b-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-27b:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-27b-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-27b:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.qwen3.8-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.qwen3.8-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-flash-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-flash-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.qwen3.8-flash-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-flash-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-flash-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.qwen3.8-flash-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-flash:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-max-0902-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-0902:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-max-0902-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-0902:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-max-0902-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-0902:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-max-0902-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-0902:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-max-0902-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-0902:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-max-0902-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-0902:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-max-prime-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-max-prime-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-max-prime-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-max-prime-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-max-prime-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-max-prime-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-max-prime-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-max-prime-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-max-prime:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-omni-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-omni-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash:reasoning-low@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-omni-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-omni-flash-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash:reasoning-medium@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-omni-flash-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-omni-flash-reasoning-none | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash:reasoning-none@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.8-omni-flash-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.8-omni-flash-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109084937-3000441-sn-ai-provider | qwen3.8-omni-flash:reasoning-xhigh@t15-20261109084937-3000441-sn-ai-provider | vision.caption | passed |
| t1.5.openai.openai.responses.v1.llm.success | T1.5 | openai/t15-20261109084937-3000441-openai | gpt-5.6@t15-20261109084937-3000441-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.inline-artifact-provider-state | T1.5 | openai/t15-20261109084937-3000441-openai | gpt-5.6@t15-20261109084937-3000441-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.stream | T1.5 | openai/t15-20261109084937-3000441-openai | gpt-5.6@t15-20261109084937-3000441-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.stream-interrupted | T1.5 | openai/t15-20261109084937-3000441-openai | gpt-5.6@t15-20261109084937-3000441-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.invalid_request | T1.5 | openai/t15-20261109084937-3000441-openai | gpt-5.6@t15-20261109084937-3000441-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.authentication | T1.5 | openai/t15-20261109084937-3000441-openai | gpt-5.6@t15-20261109084937-3000441-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.permission | T1.5 | openai/t15-20261109084937-3000441-openai | gpt-5.6@t15-20261109084937-3000441-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.not_found | T1.5 | openai/t15-20261109084937-3000441-openai | gpt-5.6@t15-20261109084937-3000441-openai | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.rate_limit | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.error.server_error | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.response.malformed_response | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.response.wrong_content_type | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.llm.response.missing_required_response_field | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | llm | passed |
| t1.5.openai.openai.responses.v1.vision.ocr.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | vision.ocr | passed |
| t1.5.openai.openai.responses.v1.vision.caption.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | vision.caption | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.invalid_request | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.authentication | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.permission | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.not_found | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-1 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-1 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.rate_limit | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.error.server_error | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.response.malformed_response | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.response.wrong_content_type | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.responses.computer.v1.agent.computer_use.response.missing_required_response_field | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | gpt-5.6@t15-20261109084937-3000441-openai-isolation-2 | agent.computer_use | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.invalid_request | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.authentication | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.permission | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.not_found | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-2 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-2 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.rate_limit | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.error.server_error | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.response.malformed_response | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.response.wrong_content_type | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.embeddings.v1.embedding.text.response.missing_required_response_field | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | text-embedding-3-small@t15-20261109084937-3000441-openai-isolation-3 | embedding.text | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.model.gpt-image-2.5-flare.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2.5-flare@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2.5-flare@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.model.gpt-image-2.5-sunburst.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2.5-sunburst@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2.5-sunburst@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.invalid_request | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.authentication | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.permission | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.not_found | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-3 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-3 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.rate_limit | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.error.server_error | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.response.malformed_response | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.response.wrong_content_type | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.response.missing_required_response_field | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.txt2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.model.gpt-image-2.5-flare.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2.5-flare@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2.5-flare@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.model.gpt-image-2.5-sunburst.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2.5-sunburst@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2.5-sunburst@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.invalid_request | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.authentication | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.permission | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.not_found | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-4 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-4 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.rate_limit | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.error.server_error | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.response.malformed_response | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.response.wrong_content_type | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.response.missing_required_response_field | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-5 | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-5 | image.inpaint | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.model.gpt-image-2.5-flare.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2.5-flare@t15-20261109084937-3000441-openai-isolation-5 | image.inpaint | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2.5-flare@t15-20261109084937-3000441-openai-isolation-5 | image.inpaint | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.model.gpt-image-2.5-sunburst.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2.5-sunburst@t15-20261109084937-3000441-openai-isolation-5 | image.inpaint | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2.5-sunburst@t15-20261109084937-3000441-openai-isolation-5 | image.inpaint | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-image-2@t15-20261109084937-3000441-openai-isolation-5 | image.inpaint | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.invalid_request | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.authentication | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.permission | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.not_found | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-5 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-5 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.rate_limit | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.error.server_error | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.response.malformed_response | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.response.wrong_content_type | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.speech.v1.audio.tts.response.missing_required_response_field | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-4o-mini-tts@t15-20261109084937-3000441-openai-isolation-6 | audio.tts | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.invalid_request | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.authentication | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.permission | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.not_found | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-6 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-6 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.rate_limit | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.error.server_error | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.response.malformed_response | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.response.wrong_content_type | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.audio.transcriptions.v1.audio.asr.response.missing_required_response_field | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | gpt-transcribe@t15-20261109084937-3000441-openai-isolation-7 | audio.asr | passed |
| t1.5.openai.openai.videos.v1.video.generate.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.video-keyframe | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.async | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.async-failed | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.async_poll_timeout | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.async_artifact_unavailable | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.error.invalid_request | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | failed |
| t1.5.openai.openai.videos.v1.video.generate.error.authentication | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.error.permission | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.error.not_found | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-7 | sora-2@t15-20261109084937-3000441-openai-isolation-7 | video.generate | failed |
| t1.5.openai.openai.videos.v1.video.generate.error.rate_limit | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.error.server_error | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.response.malformed_response | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.response.wrong_content_type | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.generate | passed |
| t1.5.openai.openai.videos.v1.video.generate.response.missing_required_response_field | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.generate | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.success | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.task-result-artifact | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.async | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.async-failed | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.async_poll_timeout | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.async_artifact_unavailable | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.error.invalid_request | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | failed |
| t1.5.openai.openai.videos.edits.v1.video.edit.error.authentication | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.error.permission | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.error.not_found | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-8 | sora-2@t15-20261109084937-3000441-openai-isolation-8 | video.edit | failed |
| t1.5.openai.openai.videos.edits.v1.video.edit.error.rate_limit | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-9 | sora-2@t15-20261109084937-3000441-openai-isolation-9 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.error.server_error | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-9 | sora-2@t15-20261109084937-3000441-openai-isolation-9 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.response.malformed_response | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-9 | sora-2@t15-20261109084937-3000441-openai-isolation-9 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.response.wrong_content_type | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-9 | sora-2@t15-20261109084937-3000441-openai-isolation-9 | video.edit | passed |
| t1.5.openai.openai.videos.edits.v1.video.edit.response.missing_required_response_field | T1.5 | openai/t15-20261109084937-3000441-openai-isolation-9 | sora-2@t15-20261109084937-3000441-openai-isolation-9 | video.edit | passed |
| t1.5.runner | T1.5 | -/- | - | provider.add/models.list | failed |

## Unexecuted manifest cases

- t1.5.artifact-id.sn-ai-provider.sn.openai-video-edits.v1.video.edit.to.openai.openai.videos.edits.v1.video.edit.cross-provider
- t1.5.custom.openai.openai.audio.speech.v1.audio.tts.success
- t1.5.custom.openai.openai.audio.transcriptions.v1.audio.asr.success
- t1.5.custom.openai.openai.embeddings.v1.embedding.text.success
- t1.5.custom.openai.openai.images.edits.v1.image.img2img.success
- t1.5.custom.openai.openai.images.edits.v1.image.inpaint.success
- t1.5.custom.openai.openai.images.generations.v1.image.txt2img.success
- t1.5.custom.openai.openai.responses.computer.v1.agent.computer_use.success
- t1.5.custom.openai.openai.responses.v1.llm.success
- t1.5.custom.openai.openai.responses.v1.vision.caption.success
- t1.5.custom.openai.openai.responses.v1.vision.ocr.success
- t1.5.custom.openai.openai.videos.edits.v1.video.edit.success
- t1.5.custom.openai.openai.videos.v1.video.generate.success
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.authentication
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.insufficient_balance
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.invalid_parameters
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.overloaded
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.rate_limit
- t1.5.glm.glm.audio.speech.v4.audio.tts.error.server_error
- t1.5.glm.glm.audio.speech.v4.audio.tts.response.malformed_response
- t1.5.glm.glm.audio.speech.v4.audio.tts.response.missing_required_response_field
- t1.5.glm.glm.audio.speech.v4.audio.tts.response.wrong_content_type
- t1.5.glm.glm.audio.speech.v4.audio.tts.success
- t1.5.glm.glm.audio.speech.v4.audio.tts.task-result-artifact
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.authentication
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.insufficient_balance
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.invalid_parameters
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.overloaded
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.rate_limit
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.error.server_error
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.response.malformed_response
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.response.missing_required_response_field
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.response.wrong_content_type
- t1.5.glm.glm.audio.transcriptions.v4.audio.asr.success
- t1.5.glm.glm.chat-completions.v4.llm.error.authentication
- t1.5.glm.glm.chat-completions.v4.llm.error.insufficient_balance
- t1.5.glm.glm.chat-completions.v4.llm.error.invalid_parameters
- t1.5.glm.glm.chat-completions.v4.llm.error.overloaded
- t1.5.glm.glm.chat-completions.v4.llm.error.rate_limit
- t1.5.glm.glm.chat-completions.v4.llm.error.server_error
- t1.5.glm.glm.chat-completions.v4.llm.model.glm-5.3-flash.success
- t1.5.glm.glm.chat-completions.v4.llm.model.glm-5.3-flashx.success
- t1.5.glm.glm.chat-completions.v4.llm.response.malformed_response
- t1.5.glm.glm.chat-completions.v4.llm.response.missing_required_response_field
- t1.5.glm.glm.chat-completions.v4.llm.response.wrong_content_type
- t1.5.glm.glm.chat-completions.v4.llm.stream
- t1.5.glm.glm.chat-completions.v4.llm.stream-interrupted
- t1.5.glm.glm.chat-completions.v4.llm.success
- t1.5.glm.glm.chat-completions.v4.vision.caption.model.glm-5.3-flash.success
- t1.5.glm.glm.chat-completions.v4.vision.caption.model.glm-5.3-flashx.success
- t1.5.glm.glm.chat-completions.v4.vision.caption.success
- t1.5.glm.glm.chat-completions.v4.vision.ocr.model.glm-5.3-flash.success
- t1.5.glm.glm.chat-completions.v4.vision.ocr.model.glm-5.3-flashx.success
- t1.5.glm.glm.chat-completions.v4.vision.ocr.success
- t1.5.glm.glm.embeddings.v4.embedding.text.error.authentication
- t1.5.glm.glm.embeddings.v4.embedding.text.error.insufficient_balance
- t1.5.glm.glm.embeddings.v4.embedding.text.error.invalid_parameters
- t1.5.glm.glm.embeddings.v4.embedding.text.error.overloaded
- t1.5.glm.glm.embeddings.v4.embedding.text.error.rate_limit
- t1.5.glm.glm.embeddings.v4.embedding.text.error.server_error
- t1.5.glm.glm.embeddings.v4.embedding.text.response.malformed_response
- t1.5.glm.glm.embeddings.v4.embedding.text.response.missing_required_response_field
- t1.5.glm.glm.embeddings.v4.embedding.text.response.wrong_content_type
- t1.5.glm.glm.embeddings.v4.embedding.text.success
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.authentication
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.insufficient_balance
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.invalid_parameters
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.overloaded
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.rate_limit
- t1.5.glm.glm.images.generations.v4.image.txt2img.error.server_error
- t1.5.glm.glm.images.generations.v4.image.txt2img.response.malformed_response
- t1.5.glm.glm.images.generations.v4.image.txt2img.response.missing_required_response_field
- t1.5.glm.glm.images.generations.v4.image.txt2img.response.wrong_content_type
- t1.5.glm.glm.images.generations.v4.image.txt2img.success
- t1.5.glm.glm.images.generations.v4.image.txt2img.task-result-artifact
- t1.5.glm.glm.videos.generations.v4.video.generate.async
- t1.5.glm.glm.videos.generations.v4.video.generate.async-failed
- t1.5.glm.glm.videos.generations.v4.video.generate.async_poll_timeout
- t1.5.glm.glm.videos.generations.v4.video.generate.error.authentication
- t1.5.glm.glm.videos.generations.v4.video.generate.error.insufficient_balance
- t1.5.glm.glm.videos.generations.v4.video.generate.error.invalid_parameters
- t1.5.glm.glm.videos.generations.v4.video.generate.error.overloaded
- t1.5.glm.glm.videos.generations.v4.video.generate.error.rate_limit
- t1.5.glm.glm.videos.generations.v4.video.generate.error.server_error
- t1.5.glm.glm.videos.generations.v4.video.generate.response.malformed_response
- t1.5.glm.glm.videos.generations.v4.video.generate.response.missing_required_response_field
- t1.5.glm.glm.videos.generations.v4.video.generate.response.wrong_content_type
- t1.5.glm.glm.videos.generations.v4.video.generate.success
- t1.5.glm.glm.videos.generations.v4.video.generate.task-result-artifact
- t1.5.glm.glm.videos.generations.v4.video.generate.video-keyframe
- t1.5.kimi.kimi.chat-completions.v1.llm.error.authentication
- t1.5.kimi.kimi.chat-completions.v1.llm.error.invalid_request
- t1.5.kimi.kimi.chat-completions.v1.llm.error.not_found
- t1.5.kimi.kimi.chat-completions.v1.llm.error.permission
- t1.5.kimi.kimi.chat-completions.v1.llm.error.rate_limit
- t1.5.kimi.kimi.chat-completions.v1.llm.error.server_error
- t1.5.kimi.kimi.chat-completions.v1.llm.model.kimi-k3.1-pro.success
- t1.5.kimi.kimi.chat-completions.v1.llm.response.malformed_response
- t1.5.kimi.kimi.chat-completions.v1.llm.response.missing_required_response_field
- t1.5.kimi.kimi.chat-completions.v1.llm.response.wrong_content_type
- t1.5.kimi.kimi.chat-completions.v1.llm.stream
- t1.5.kimi.kimi.chat-completions.v1.llm.stream-interrupted
- t1.5.kimi.kimi.chat-completions.v1.llm.success
- t1.5.kimi.kimi.chat-completions.v1.vision.caption.model.kimi-k3.1-pro.success
- t1.5.kimi.kimi.chat-completions.v1.vision.caption.success
- t1.5.kimi.kimi.chat-completions.v1.vision.ocr.model.kimi-k3.1-pro.success
- t1.5.kimi.kimi.chat-completions.v1.vision.ocr.success
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-high
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-low
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-max
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-medium
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-none
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-luna-reasoning-xhigh
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-high
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-low
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-max
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-medium
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-none
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-reasoning-xhigh
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-high
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-low
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-max
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-medium
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-none
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-sol-reasoning-xhigh
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-high
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-low
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-max
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-medium
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-none
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-5.6-terra-reasoning-xhigh
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-high
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-low
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-max
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-medium
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-sol-reasoning-xhigh
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-high
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-low
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-max
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-medium
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-none
- t1.5.openai.openai.responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-none
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-mini-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-none
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-nano-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-none
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.4-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-none
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.5-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-max
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-none
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-luna-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-max
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-none
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-max
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-none
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-sol-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-max
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-none
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-5.6-terra-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-max
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-sol-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-high
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-low
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-max
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-none
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.img2img.variant.gpt-6.1-terra-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-none
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-mini-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-none
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-nano-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-pro-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-none
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.4-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-pro-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-none
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.5-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-max
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-none
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-luna-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-max
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-none
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-max
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-none
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-sol-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-max
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-none
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-5.6-terra-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-max
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-sol-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-high
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-high.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-low
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-low.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-max
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-max.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-medium.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-none
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-none.task-result-artifact
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-xhigh.task-result-artifact
- t1.5.openai.openai.responses.v1.llm.cloud-update
- t1.5.openai.openai.responses.v1.llm.history
- t1.5.openai.openai.responses.v1.llm.native-history
- t1.5.openai.openai.responses.v1.llm.tool-history
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.3-codex-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.3-codex-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.3-codex-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.3-codex-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-none
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-mini-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-none
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-nano-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-pro-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-pro-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-pro-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-none
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.4-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-pro-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-pro-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-pro-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-none
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.5-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-max
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-none
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-luna-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-max
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-none
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-max
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-none
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-max
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-none
- t1.5.openai.openai.responses.v1.llm.variant.gpt-5.6-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-sol-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-sol-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-sol-reasoning-max
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-terra-reasoning-high
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-terra-reasoning-low
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-terra-reasoning-max
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-terra-reasoning-none
- t1.5.openai.openai.responses.v1.llm.variant.gpt-6.1-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.3-codex-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-none
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-mini-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-none
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-nano-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-pro-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-pro-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-pro-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-none
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.4-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-pro-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-pro-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-pro-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-none
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.5-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-max
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-none
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-luna-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-max
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-none
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-max
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-none
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-max
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-none
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-5.6-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-max
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-high
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-low
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-max
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-none
- t1.5.openai.openai.responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.3-codex-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-none
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-mini-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-none
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-nano-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-pro-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-pro-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-pro-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-none
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.4-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-pro-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-pro-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-pro-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-none
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.5-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-max
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-none
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-luna-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-max
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-none
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-max
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-none
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-max
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-none
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-5.6-terra-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-max
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-sol-reasoning-xhigh
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-high
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-low
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-max
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-medium
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-none
- t1.5.openai.openai.responses.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-xhigh
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.error.authentication
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.error.invalid_request
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.error.permission
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.error.provider_error
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.error.rate_limit
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.google-gemini-3.1-flash-tts-preview.success
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.google-gemini-3.1-flash-tts-preview.task-result-artifact
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.google-gemini-3.8-flash-lite-tts.success
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.google-gemini-3.8-flash-lite-tts.task-result-artifact
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.google-gemini-3.8-flash-tts.success
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.google-gemini-3.8-flash-tts.task-result-artifact
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.minimax-speech-2.8-hd.success
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.minimax-speech-2.8-hd.task-result-artifact
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.minimax-speech-2.8-turbo.success
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.model.minimax-speech-2.8-turbo.task-result-artifact
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.response.malformed_response
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.response.missing_required_response_field
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.response.wrong_content_type
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.success
- t1.5.openrouter.openrouter.audio.speech.v1.audio.tts.task-result-artifact
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.error.authentication
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.error.invalid_request
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.error.permission
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.error.provider_error
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.error.rate_limit
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.model.google-gemini-3.5-transcribe.success
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.response.malformed_response
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.response.missing_required_response_field
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.response.wrong_content_type
- t1.5.openrouter.openrouter.audio.transcriptions.v1.audio.asr.success
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.error.authentication
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.error.invalid_request
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.error.permission
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.error.provider_error
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.error.rate_limit
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.response.malformed_response
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.response.missing_required_response_field
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.response.wrong_content_type
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.success
- t1.5.openrouter.openrouter.chat.audio.v1.audio.music.task-result-artifact
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.authentication
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.invalid_request
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.permission
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.provider_error
- t1.5.openrouter.openrouter.decisions.alpha.decision.error.rate_limit
- t1.5.openrouter.openrouter.decisions.alpha.decision.response.malformed_response
- t1.5.openrouter.openrouter.decisions.alpha.decision.response.missing_required_response_field
- t1.5.openrouter.openrouter.decisions.alpha.decision.response.wrong_content_type
- t1.5.openrouter.openrouter.decisions.alpha.decision.success
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.authentication
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.invalid_request
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.permission
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.provider_error
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.error.rate_limit
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.response.malformed_response
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.response.missing_required_response_field
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.response.wrong_content_type
- t1.5.openrouter.openrouter.embeddings.v1.embedding.text.success
- t1.5.openrouter.openrouter.images.v1.image.img2img.success
- t1.5.openrouter.openrouter.images.v1.image.img2img.task-result-artifact
- t1.5.openrouter.openrouter.images.v1.image.txt2img.error.authentication
- t1.5.openrouter.openrouter.images.v1.image.txt2img.error.invalid_request
- t1.5.openrouter.openrouter.images.v1.image.txt2img.error.permission
- t1.5.openrouter.openrouter.images.v1.image.txt2img.error.provider_error
- t1.5.openrouter.openrouter.images.v1.image.txt2img.error.rate_limit
- t1.5.openrouter.openrouter.images.v1.image.txt2img.response.malformed_response
- t1.5.openrouter.openrouter.images.v1.image.txt2img.response.missing_required_response_field
- t1.5.openrouter.openrouter.images.v1.image.txt2img.response.wrong_content_type
- t1.5.openrouter.openrouter.images.v1.image.txt2img.success
- t1.5.openrouter.openrouter.images.v1.image.txt2img.task-result-artifact
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.authentication
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.invalid_request
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.permission
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.provider_error
- t1.5.openrouter.openrouter.rerank.v1.rerank.error.rate_limit
- t1.5.openrouter.openrouter.rerank.v1.rerank.response.malformed_response
- t1.5.openrouter.openrouter.rerank.v1.rerank.response.missing_required_response_field
- t1.5.openrouter.openrouter.rerank.v1.rerank.response.wrong_content_type
- t1.5.openrouter.openrouter.rerank.v1.rerank.success
- t1.5.openrouter.openrouter.responses.v1.llm.error.authentication
- t1.5.openrouter.openrouter.responses.v1.llm.error.invalid_request
- t1.5.openrouter.openrouter.responses.v1.llm.error.permission
- t1.5.openrouter.openrouter.responses.v1.llm.error.provider_error
- t1.5.openrouter.openrouter.responses.v1.llm.error.rate_limit
- t1.5.openrouter.openrouter.responses.v1.llm.history
- t1.5.openrouter.openrouter.responses.v1.llm.model.moonshotai-kimi-k3.1-pro.success
- t1.5.openrouter.openrouter.responses.v1.llm.model.x-ai-grok-4.7.success
- t1.5.openrouter.openrouter.responses.v1.llm.model.z-ai-glm-5.3-flash.success
- t1.5.openrouter.openrouter.responses.v1.llm.model.z-ai-glm-5.3-flashx.success
- t1.5.openrouter.openrouter.responses.v1.llm.native-history
- t1.5.openrouter.openrouter.responses.v1.llm.response.malformed_response
- t1.5.openrouter.openrouter.responses.v1.llm.response.missing_required_response_field
- t1.5.openrouter.openrouter.responses.v1.llm.response.wrong_content_type
- t1.5.openrouter.openrouter.responses.v1.llm.sampling-forwarded
- t1.5.openrouter.openrouter.responses.v1.llm.sampling-omitted
- t1.5.openrouter.openrouter.responses.v1.llm.stream
- t1.5.openrouter.openrouter.responses.v1.llm.stream-interrupted
- t1.5.openrouter.openrouter.responses.v1.llm.success
- t1.5.openrouter.openrouter.responses.v1.llm.tool-history
- t1.5.openrouter.openrouter.responses.v1.vision.caption.model.moonshotai-kimi-k3.1-pro.success
- t1.5.openrouter.openrouter.responses.v1.vision.caption.model.x-ai-grok-4.7.success
- t1.5.openrouter.openrouter.responses.v1.vision.caption.model.z-ai-glm-5.3-flash.success
- t1.5.openrouter.openrouter.responses.v1.vision.caption.model.z-ai-glm-5.3-flashx.success
- t1.5.openrouter.openrouter.responses.v1.vision.caption.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.moonshotai-kimi-k3.1-pro.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.x-ai-grok-4.7.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.z-ai-glm-5.3-flash.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.z-ai-glm-5.3-flashx.success
- t1.5.openrouter.openrouter.responses.v1.vision.ocr.success
- t1.5.openrouter.openrouter.videos.v1.video.generate.async
- t1.5.openrouter.openrouter.videos.v1.video.generate.async-failed
- t1.5.openrouter.openrouter.videos.v1.video.generate.async_artifact_unavailable
- t1.5.openrouter.openrouter.videos.v1.video.generate.async_poll_timeout
- t1.5.openrouter.openrouter.videos.v1.video.generate.error.authentication
- t1.5.openrouter.openrouter.videos.v1.video.generate.error.invalid_request
- t1.5.openrouter.openrouter.videos.v1.video.generate.error.permission
- t1.5.openrouter.openrouter.videos.v1.video.generate.error.provider_error
- t1.5.openrouter.openrouter.videos.v1.video.generate.error.rate_limit
- t1.5.openrouter.openrouter.videos.v1.video.generate.response.malformed_response
- t1.5.openrouter.openrouter.videos.v1.video.generate.response.missing_required_response_field
- t1.5.openrouter.openrouter.videos.v1.video.generate.response.wrong_content_type
- t1.5.openrouter.openrouter.videos.v1.video.generate.success
- t1.5.openrouter.openrouter.videos.v1.video.generate.task-result-artifact
- t1.5.openrouter.openrouter.videos.v1.video.generate.video-keyframe
- t1.5.openrouter.openrouter.videos.v1.video.generate.video-keyframes
- t1.5.openrouter.openrouter.videos.v1.video.generate.video-reference-images
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.authentication
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.invalid_request
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.not_found
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.permission
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.rate_limit
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.error.server_error
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.malformed_response
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.missing_required_response_field
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.response.wrong_content_type
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.stream-interrupted
- t1.5.qwen.qwen.chat-completions.compatible-v1.llm.success
- t1.5.qwen.qwen.chat-completions.compatible-v1.vision.caption.success
- t1.5.qwen.qwen.chat-completions.compatible-v1.vision.ocr.success
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.authentication
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.invalid_request
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.not_found
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.permission
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.rate_limit
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.error.server_error
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.malformed_response
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.success
- t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.task-result-artifact
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.img2img.success
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.img2img.task-result-artifact
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.async
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.async-failed
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.async_artifact_unavailable
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.async_poll_timeout
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.authentication
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.invalid_request
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.not_found
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.permission
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.rate_limit
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.error.server_error
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.response.malformed_response
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.success
- t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.task-result-artifact
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async-failed
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async_artifact_unavailable
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.async_poll_timeout
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.authentication
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.invalid_request
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.not_found
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.permission
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.rate_limit
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.error.server_error
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.malformed_response
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.success
- t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.task-result-artifact
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.async
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.async-failed
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.async_artifact_unavailable
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.async_poll_timeout
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.error.authentication
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.error.invalid_request
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.error.not_found
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.error.permission
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.error.rate_limit
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.error.server_error
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.response.malformed_response
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.success
- t1.5.qwen.qwen.dashscope.video_synthesis.v1.video.generate.task-result-artifact
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.async
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.async-failed
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.async_artifact_unavailable
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.async_poll_timeout
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.error.authentication
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.error.invalid_request
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.error.not_found
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.error.permission
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.error.rate_limit
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.error.server_error
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.response.malformed_response
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.response.missing_required_response_field
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.response.wrong_content_type
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.success
- t1.5.qwen.qwen.dashscope.wan3.video_synthesis.v1.video.generate.task-result-artifact
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.authentication
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.invalid_request
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.not_found
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.permission
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.rate_limit
- t1.5.qwen.qwen.responses.compatible-v1.llm.error.server_error
- t1.5.qwen.qwen.responses.compatible-v1.llm.inline-artifact-provider-state
- t1.5.qwen.qwen.responses.compatible-v1.llm.response.malformed_response
- t1.5.qwen.qwen.responses.compatible-v1.llm.response.missing_required_response_field
- t1.5.qwen.qwen.responses.compatible-v1.llm.response.wrong_content_type
- t1.5.qwen.qwen.responses.compatible-v1.llm.stream
- t1.5.qwen.qwen.responses.compatible-v1.llm.stream-interrupted
- t1.5.qwen.qwen.responses.compatible-v1.llm.success
- t1.5.qwen.qwen.responses.compatible-v1.llm.video-input
- t1.5.qwen.qwen.responses.compatible-v1.vision.caption.success
- t1.5.qwen.qwen.responses.compatible-v1.vision.ocr.success
- t1.5.switch.openai.openai.responses.v1.llm.to.openrouter.openrouter.responses.v1.llm
- t1.5.switch.openrouter.openrouter.responses.v1.llm.to.openai.openai.responses.v1.llm

## Confirmed product defects

- **defect.aicc.t1.5.openai.openai.videos.v1.video.generate.error.invalid_request** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=openai.videos.v1; scenario=invalid_request; captured_requests=1; missing stable AICC error code provider_error. Evidence: cases/t1.5.openai.openai.videos.v1.video.generate.error.invalid_request.json
- **defect.aicc.t1.5.openai.openai.videos.v1.video.generate.error.not_found** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=openai.videos.v1; scenario=not_found; captured_requests=1; missing stable AICC error code provider_error. Evidence: cases/t1.5.openai.openai.videos.v1.video.generate.error.not_found.json
- **defect.aicc.t1.5.openai.openai.videos.edits.v1.video.edit.error.invalid_request** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=openai.videos.edits.v1; scenario=invalid_request; captured_requests=1; missing stable AICC error code provider_error. Evidence: cases/t1.5.openai.openai.videos.edits.v1.video.edit.error.invalid_request.json
- **defect.aicc.t1.5.openai.openai.videos.edits.v1.video.edit.error.not_found** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed contract=openai.videos.edits.v1; scenario=not_found; captured_requests=1; missing stable AICC error code provider_error. Evidence: cases/t1.5.openai.openai.videos.edits.v1.video.edit.error.not_found.json
- **defect.aicc.t1.5.runner** (AICC, provider_protocol_failed): expected AICC maps the official Provider protocol fixture through its real Adapter; observed captured_requests=0; provider:openai:cases: NdmProxyApiError: Method not allowed. Evidence: cases/t1.5.runner.json

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

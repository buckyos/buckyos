# AICC E2E Acceptance Report

- Run: `t15-20261030155506-1454156`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=30, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 30/30 (100.00%); passed=30, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.5-pro.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.5-pro@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.5.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.5@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-5.4-pro.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.4-pro@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.background-ignored | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async-failed | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async_poll_timeout | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async_artifact_unavailable | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.async-cancel | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.authentication | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.unavailable | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.error.timeout | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.response.malformed_response | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.5-pro.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.5-pro@t15-20261030155506-1454156-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.5-pro.task-result-artifact | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.5-pro@t15-20261030155506-1454156-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.5.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.5@t15-20261030155506-1454156-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.5.task-result-artifact | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.5@t15-20261030155506-1454156-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4-pro.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.4-pro@t15-20261030155506-1454156-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-5.4-pro.task-result-artifact | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.4-pro@t15-20261030155506-1454156-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.5.success | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.5@t15-20261030155506-1454156-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-5.5.task-result-artifact | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-5.5@t15-20261030155506-1454156-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261030155506-1454156-sn-ai-provider | gpt-6-sol@t15-20261030155506-1454156-sn-ai-provider | image.img2img | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

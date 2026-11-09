# AICC E2E Acceptance Report

- Run: `t15-20261109081136-2942440`
- Commit: `7c9472fda19935f8070a63943361063b8ad2e376`
- Capability baseline: `2026-11-02.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=81, failed=0, provider_restricted=0, skipped=54, not_applicable=0, review=0
- Manifest coverage: 135/135 (100.00%); passed=81, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.server-default-route | T1.5 | sn-ai-provider/t15-20261109081136-2942440-server-default | qwen3.7-plus@t15-20261109081136-2942440-server-default | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.qwen3.7-plus.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | qwen3.7-plus@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.grok-4.7.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.qwen3.7-plus.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | qwen3.7-plus@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.grok-4.7.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.qwen3.7-plus.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | qwen3.7-plus@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.grok-4.7.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-terra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-terra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra@t15-20261109081136-2942440-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.model.wan-3.0.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.generate.model.wan-3.0.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.generate | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.generated-artifact-id.same-provider | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async-failed | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async_poll_timeout | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.async_artifact_unavailable | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.authentication | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.unavailable | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.timeout | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.error.upstream_json_required | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.edit.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.edit | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.generated-artifact-id.same-provider | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async-failed | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async_poll_timeout | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.async_artifact_unavailable | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.authentication | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.unavailable | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.timeout | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.error.upstream_json_required | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | video.extend | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.authentication | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.unavailable | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.error.timeout | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.txt2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.success | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.authentication | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.insufficient_balance | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.unavailable | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.error.timeout | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.response.malformed_response | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.response.wrong_content_type | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.response.missing_required_response_field | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | - | image.img2img | skipped |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109081136-2942440-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-high | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-high | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-high | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-high@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-low | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-low | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-low | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-low@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-medium@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | grok-4.7:reasoning-xhigh@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.7-plus-reasoning-none | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | qwen3.7-plus:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.7-plus-reasoning-none | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | qwen3.7-plus:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.qwen3.7-plus-reasoning-none | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | qwen3.7-plus:reasoning-none@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.qwen3.7-plus-reasoning-thinking | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | qwen3.7-plus:reasoning-thinking@t15-20261109081136-2942440-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.qwen3.7-plus-reasoning-thinking | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | qwen3.7-plus:reasoning-thinking@t15-20261109081136-2942440-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.qwen3.7-plus-reasoning-thinking | T1.5 | sn-ai-provider/t15-20261109081136-2942440-sn-ai-provider | qwen3.7-plus:reasoning-thinking@t15-20261109081136-2942440-sn-ai-provider | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

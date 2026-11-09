# AICC E2E Acceptance Report

- Run: `t15-20261102034947-888943`
- Commit: `7c9472fda19935f8070a63943361063b8ad2e376`
- Capability baseline: `2026-11-02.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=63, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 63/63 (100.00%); passed=63, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.grok-4.7.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.kimi-k3.1-pro.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.glm-5.3-flash.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.glm-5.3-flashx.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.grok-4.7.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.kimi-k3.1-pro.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.glm-5.3-flash.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.glm-5.3-flashx.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.grok-4.7.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.kimi-k3.1-pro.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.glm-5.3-flash.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.glm-5.3-flashx.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.5-flare.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-flare@t15-20261102034947-888943-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-flare@t15-20261102034947-888943-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.5-sunburst.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261102034947-888943-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-generations.v1.image.txt2img.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261102034947-888943-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.5-flare.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-flare@t15-20261102034947-888943-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-flare@t15-20261102034947-888943-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.5-sunburst.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261102034947-888943-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261102034947-888943-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.5-flare.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-flare@t15-20261102034947-888943-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-flare@t15-20261102034947-888943-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.5-sunburst.success | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261102034947-888943-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | gpt-image-2.5-sunburst@t15-20261102034947-888943-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash:reasoning-high@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash:reasoning-high@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flash-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash:reasoning-high@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash:reasoning-low@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash:reasoning-low@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flash-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash:reasoning-low@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flash-reasoning-max | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash:reasoning-max@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flash-reasoning-max | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash:reasoning-max@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flash-reasoning-max | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flash:reasoning-max@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flashx-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx:reasoning-high@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flashx-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx:reasoning-high@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flashx-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx:reasoning-high@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flashx-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx:reasoning-low@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flashx-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx:reasoning-low@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flashx-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx:reasoning-low@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flashx-reasoning-max | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx:reasoning-max@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flashx-reasoning-max | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx:reasoning-max@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flashx-reasoning-max | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | glm-5.3-flashx:reasoning-max@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-high@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-high@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-high@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-low@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-low@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-low@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-medium | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-medium@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-medium | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-medium@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-medium | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-medium@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.grok-4.7-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-xhigh@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.grok-4.7-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-xhigh@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.grok-4.7-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | grok-4.7:reasoning-xhigh@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.kimi-k3.1-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro:reasoning-high@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.kimi-k3.1-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro:reasoning-high@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.kimi-k3.1-pro-reasoning-high | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro:reasoning-high@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.kimi-k3.1-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro:reasoning-low@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.kimi-k3.1-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro:reasoning-low@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.kimi-k3.1-pro-reasoning-low | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro:reasoning-low@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.kimi-k3.1-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro:reasoning-max@t15-20261102034947-888943-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.kimi-k3.1-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro:reasoning-max@t15-20261102034947-888943-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.kimi-k3.1-pro-reasoning-max | T1.5 | sn-ai-provider/t15-20261102034947-888943-sn-ai-provider | kimi-k3.1-pro:reasoning-max@t15-20261102034947-888943-sn-ai-provider | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

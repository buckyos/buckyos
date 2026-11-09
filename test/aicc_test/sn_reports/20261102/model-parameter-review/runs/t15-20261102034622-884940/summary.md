# AICC E2E Acceptance Report

- Run: `t15-20261102034622-884940`
- Commit: `7c9472fda19935f8070a63943361063b8ad2e376`
- Capability baseline: `2026-11-02.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=51, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 51/51 (100.00%); passed=51, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.openrouter.openrouter.responses.v1.llm.model.z-ai-glm-5.3-flash.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.model.z-ai-glm-5.3-flashx.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.model.x-ai-grok-4.7.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.model.moonshotai-kimi-k3.1-pro.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.z-ai-glm-5.3-flash.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.z-ai-glm-5.3-flashx.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.x-ai-grok-4.7.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.model.moonshotai-kimi-k3.1-pro.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.model.z-ai-glm-5.3-flash.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.model.z-ai-glm-5.3-flashx.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.model.x-ai-grok-4.7.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.model.moonshotai-kimi-k3.1-pro.success | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.moonshotai-kimi-k3.1-pro-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro:reasoning-high@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.moonshotai-kimi-k3.1-pro-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro:reasoning-high@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.moonshotai-kimi-k3.1-pro-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro:reasoning-high@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.moonshotai-kimi-k3.1-pro-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro:reasoning-low@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.moonshotai-kimi-k3.1-pro-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro:reasoning-low@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.moonshotai-kimi-k3.1-pro-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro:reasoning-low@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.moonshotai-kimi-k3.1-pro-reasoning-max | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro:reasoning-max@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.moonshotai-kimi-k3.1-pro-reasoning-max | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro:reasoning-max@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.moonshotai-kimi-k3.1-pro-reasoning-max | T1.5 | openrouter/t15-20261102034622-884940-openrouter | moonshotai/kimi-k3.1-pro:reasoning-max@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.x-ai-grok-4.7-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-high@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.x-ai-grok-4.7-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-high@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.x-ai-grok-4.7-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-high@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.x-ai-grok-4.7-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-low@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.x-ai-grok-4.7-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-low@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.x-ai-grok-4.7-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-low@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.x-ai-grok-4.7-reasoning-medium | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-medium@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.x-ai-grok-4.7-reasoning-medium | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-medium@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.x-ai-grok-4.7-reasoning-medium | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-medium@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.x-ai-grok-4.7-reasoning-xhigh | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-xhigh@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.x-ai-grok-4.7-reasoning-xhigh | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-xhigh@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.x-ai-grok-4.7-reasoning-xhigh | T1.5 | openrouter/t15-20261102034622-884940-openrouter | x-ai/grok-4.7:reasoning-xhigh@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.z-ai-glm-5.3-flash-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash:reasoning-high@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.z-ai-glm-5.3-flash-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash:reasoning-high@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.z-ai-glm-5.3-flash-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash:reasoning-high@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.z-ai-glm-5.3-flash-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash:reasoning-low@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.z-ai-glm-5.3-flash-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash:reasoning-low@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.z-ai-glm-5.3-flash-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash:reasoning-low@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.z-ai-glm-5.3-flash-reasoning-max | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash:reasoning-max@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.z-ai-glm-5.3-flash-reasoning-max | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash:reasoning-max@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.z-ai-glm-5.3-flash-reasoning-max | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flash:reasoning-max@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.z-ai-glm-5.3-flashx-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx:reasoning-high@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.z-ai-glm-5.3-flashx-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx:reasoning-high@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.z-ai-glm-5.3-flashx-reasoning-high | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx:reasoning-high@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.z-ai-glm-5.3-flashx-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx:reasoning-low@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.z-ai-glm-5.3-flashx-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx:reasoning-low@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.z-ai-glm-5.3-flashx-reasoning-low | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx:reasoning-low@t15-20261102034622-884940-openrouter | vision.ocr | passed |
| t1.5.openrouter.openrouter.responses.v1.llm.variant.z-ai-glm-5.3-flashx-reasoning-max | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx:reasoning-max@t15-20261102034622-884940-openrouter | llm | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.caption.variant.z-ai-glm-5.3-flashx-reasoning-max | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx:reasoning-max@t15-20261102034622-884940-openrouter | vision.caption | passed |
| t1.5.openrouter.openrouter.responses.v1.vision.ocr.variant.z-ai-glm-5.3-flashx-reasoning-max | T1.5 | openrouter/t15-20261102034622-884940-openrouter | z-ai/glm-5.3-flashx:reasoning-max@t15-20261102034622-884940-openrouter | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

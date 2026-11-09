# AICC E2E Acceptance Report

- Run: `t15-20261102042624-928527`
- Commit: `7c9472fda19935f8070a63943361063b8ad2e376`
- Capability baseline: `2026-11-02.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=12, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 12/12 (100.00%); passed=12, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.glm-5.3-flashx.success | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx@t15-20261102042624-928527-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.model.glm-5.3-flashx.success | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx@t15-20261102042624-928527-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.glm-5.3-flashx.success | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx@t15-20261102042624-928527-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flashx-reasoning-high | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx:reasoning-high@t15-20261102042624-928527-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flashx-reasoning-high | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx:reasoning-high@t15-20261102042624-928527-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flashx-reasoning-high | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx:reasoning-high@t15-20261102042624-928527-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flashx-reasoning-low | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx:reasoning-low@t15-20261102042624-928527-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flashx-reasoning-low | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx:reasoning-low@t15-20261102042624-928527-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flashx-reasoning-low | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx:reasoning-low@t15-20261102042624-928527-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.glm-5.3-flashx-reasoning-max | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx:reasoning-max@t15-20261102042624-928527-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.glm-5.3-flashx-reasoning-max | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx:reasoning-max@t15-20261102042624-928527-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.ocr.variant.glm-5.3-flashx-reasoning-max | T1.5 | sn-ai-provider/t15-20261102042624-928527-sn-ai-provider | glm-5.3-flashx:reasoning-max@t15-20261102042624-928527-sn-ai-provider | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

# AICC E2E Acceptance Report

- Run: `t15-20261030083704-1003203`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=24, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 24/24 (100.00%); passed=24, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.glm.glm.chat-completions.v4.llm.model.glm-5.3-flash.success | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash@t15-20261030083704-1003203-glm | llm | passed |
| t1.5.glm.glm.chat-completions.v4.llm.model.glm-5.3-flashx.success | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx@t15-20261030083704-1003203-glm | llm | passed |
| t1.5.glm.glm.chat-completions.v4.vision.ocr.model.glm-5.3-flash.success | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash@t15-20261030083704-1003203-glm | vision.ocr | passed |
| t1.5.glm.glm.chat-completions.v4.vision.ocr.model.glm-5.3-flashx.success | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx@t15-20261030083704-1003203-glm | vision.ocr | passed |
| t1.5.glm.glm.chat-completions.v4.vision.caption.model.glm-5.3-flash.success | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash@t15-20261030083704-1003203-glm | vision.caption | passed |
| t1.5.glm.glm.chat-completions.v4.vision.caption.model.glm-5.3-flashx.success | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx@t15-20261030083704-1003203-glm | vision.caption | passed |
| t1.5.glm.glm.chat-completions.v4.llm.variant.glm-5.3-flash-reasoning-high | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash:reasoning-high@t15-20261030083704-1003203-glm | llm | passed |
| t1.5.glm.glm.chat-completions.v4.vision.caption.variant.glm-5.3-flash-reasoning-high | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash:reasoning-high@t15-20261030083704-1003203-glm | vision.caption | passed |
| t1.5.glm.glm.chat-completions.v4.vision.ocr.variant.glm-5.3-flash-reasoning-high | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash:reasoning-high@t15-20261030083704-1003203-glm | vision.ocr | passed |
| t1.5.glm.glm.chat-completions.v4.llm.variant.glm-5.3-flash-reasoning-low | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash:reasoning-low@t15-20261030083704-1003203-glm | llm | passed |
| t1.5.glm.glm.chat-completions.v4.vision.caption.variant.glm-5.3-flash-reasoning-low | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash:reasoning-low@t15-20261030083704-1003203-glm | vision.caption | passed |
| t1.5.glm.glm.chat-completions.v4.vision.ocr.variant.glm-5.3-flash-reasoning-low | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash:reasoning-low@t15-20261030083704-1003203-glm | vision.ocr | passed |
| t1.5.glm.glm.chat-completions.v4.llm.variant.glm-5.3-flash-reasoning-max | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash:reasoning-max@t15-20261030083704-1003203-glm | llm | passed |
| t1.5.glm.glm.chat-completions.v4.vision.caption.variant.glm-5.3-flash-reasoning-max | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash:reasoning-max@t15-20261030083704-1003203-glm | vision.caption | passed |
| t1.5.glm.glm.chat-completions.v4.vision.ocr.variant.glm-5.3-flash-reasoning-max | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flash:reasoning-max@t15-20261030083704-1003203-glm | vision.ocr | passed |
| t1.5.glm.glm.chat-completions.v4.llm.variant.glm-5.3-flashx-reasoning-high | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx:reasoning-high@t15-20261030083704-1003203-glm | llm | passed |
| t1.5.glm.glm.chat-completions.v4.vision.caption.variant.glm-5.3-flashx-reasoning-high | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx:reasoning-high@t15-20261030083704-1003203-glm | vision.caption | passed |
| t1.5.glm.glm.chat-completions.v4.vision.ocr.variant.glm-5.3-flashx-reasoning-high | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx:reasoning-high@t15-20261030083704-1003203-glm | vision.ocr | passed |
| t1.5.glm.glm.chat-completions.v4.llm.variant.glm-5.3-flashx-reasoning-low | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx:reasoning-low@t15-20261030083704-1003203-glm | llm | passed |
| t1.5.glm.glm.chat-completions.v4.vision.caption.variant.glm-5.3-flashx-reasoning-low | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx:reasoning-low@t15-20261030083704-1003203-glm | vision.caption | passed |
| t1.5.glm.glm.chat-completions.v4.vision.ocr.variant.glm-5.3-flashx-reasoning-low | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx:reasoning-low@t15-20261030083704-1003203-glm | vision.ocr | passed |
| t1.5.glm.glm.chat-completions.v4.llm.variant.glm-5.3-flashx-reasoning-max | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx:reasoning-max@t15-20261030083704-1003203-glm | llm | passed |
| t1.5.glm.glm.chat-completions.v4.vision.caption.variant.glm-5.3-flashx-reasoning-max | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx:reasoning-max@t15-20261030083704-1003203-glm | vision.caption | passed |
| t1.5.glm.glm.chat-completions.v4.vision.ocr.variant.glm-5.3-flashx-reasoning-max | T1.5 | glm/t15-20261030083704-1003203-glm | glm-5.3-flashx:reasoning-max@t15-20261030083704-1003203-glm | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

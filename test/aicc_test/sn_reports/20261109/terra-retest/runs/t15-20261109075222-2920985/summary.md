# AICC E2E Acceptance Report

- Run: `t15-20261109075222-2920985`
- Commit: `7c9472fda19935f8070a63943361063b8ad2e376`
- Capability baseline: `2026-11-02.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=56, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 56/56 (100.00%); passed=56, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra@t15-20261109075222-2920985-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra@t15-20261109075222-2920985-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra@t15-20261109075222-2920985-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.model.gpt-6.1-terra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.model.gpt-6.1-terra.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.model.gpt-6.1-terra.success | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra@t15-20261109075222-2920985-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109075222-2920985-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-high.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109075222-2920985-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109075222-2920985-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-high | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-high@t15-20261109075222-2920985-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109075222-2920985-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-low.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109075222-2920985-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109075222-2920985-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-low | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-low@t15-20261109075222-2920985-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109075222-2920985-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-max.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109075222-2920985-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109075222-2920985-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-max | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-max@t15-20261109075222-2920985-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109075222-2920985-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-medium.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109075222-2920985-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109075222-2920985-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-medium | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-medium@t15-20261109075222-2920985-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109075222-2920985-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-none.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109075222-2920985-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109075222-2920985-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-none | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-none@t15-20261109075222-2920985-sn-ai-provider | vision.ocr | passed |
| t1.5.sn-ai-provider.sn.openai-responses.computer.v1.agent.computer_use.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109075222-2920985-sn-ai-provider | agent.computer_use | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.variant.gpt-6.1-terra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109075222-2920985-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.variant.gpt-6.1-terra-reasoning-xhigh.task-result-artifact | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109075222-2920985-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109075222-2920985-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.openai-responses.v1.vision.caption.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109075222-2920985-sn-ai-provider | vision.caption | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.vision.ocr.variant.gpt-6.1-terra-reasoning-xhigh | T1.5 | sn-ai-provider/t15-20261109075222-2920985-sn-ai-provider | gpt-6.1-terra:reasoning-xhigh@t15-20261109075222-2920985-sn-ai-provider | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

# AICC E2E Acceptance Report

- Run: `t15-20261102034842-887535`
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
| t1.5.kimi.kimi.chat-completions.v1.llm.model.kimi-k3.1-pro.success | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro@t15-20261102034842-887535-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.model.kimi-k3.1-pro.success | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro@t15-20261102034842-887535-kimi | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.model.kimi-k3.1-pro.success | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro@t15-20261102034842-887535-kimi | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k3.1-pro-reasoning-high | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro:reasoning-high@t15-20261102034842-887535-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k3.1-pro-reasoning-high | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro:reasoning-high@t15-20261102034842-887535-kimi | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k3.1-pro-reasoning-high | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro:reasoning-high@t15-20261102034842-887535-kimi | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k3.1-pro-reasoning-low | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro:reasoning-low@t15-20261102034842-887535-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k3.1-pro-reasoning-low | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro:reasoning-low@t15-20261102034842-887535-kimi | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k3.1-pro-reasoning-low | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro:reasoning-low@t15-20261102034842-887535-kimi | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k3.1-pro-reasoning-max | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro:reasoning-max@t15-20261102034842-887535-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k3.1-pro-reasoning-max | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro:reasoning-max@t15-20261102034842-887535-kimi | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k3.1-pro-reasoning-max | T1.5 | kimi/t15-20261102034842-887535-kimi | kimi-k3.1-pro:reasoning-max@t15-20261102034842-887535-kimi | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

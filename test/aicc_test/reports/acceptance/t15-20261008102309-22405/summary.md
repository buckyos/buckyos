# AICC E2E Acceptance Report

- Run: `t15-20261008102309-22405`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `official-provider-protocols-2026-10-08.1`
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
| t1.5.deepseek.deepseek.responses.v1.llm.success | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.stream | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.stream-interrupted | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.error.invalid_format | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.error.authentication | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.error.insufficient_balance | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.error.invalid_parameters | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.error.rate_limit | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.error.overloaded | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.response.malformed_response | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.response.wrong_content_type | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.response.missing_required_response_field | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.ocr.success | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | vision.ocr | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.caption.success | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash@t15-20261008102309-22405-deepseek | vision.caption | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.variant.deepseek-flash-reasoning-high | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-high@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.caption.variant.deepseek-flash-reasoning-high | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-high@t15-20261008102309-22405-deepseek | vision.caption | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.ocr.variant.deepseek-flash-reasoning-high | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-high@t15-20261008102309-22405-deepseek | vision.ocr | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.variant.deepseek-flash-reasoning-low | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-low@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.caption.variant.deepseek-flash-reasoning-low | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-low@t15-20261008102309-22405-deepseek | vision.caption | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.ocr.variant.deepseek-flash-reasoning-low | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-low@t15-20261008102309-22405-deepseek | vision.ocr | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.variant.deepseek-flash-reasoning-max | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-max@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.caption.variant.deepseek-flash-reasoning-max | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-max@t15-20261008102309-22405-deepseek | vision.caption | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.ocr.variant.deepseek-flash-reasoning-max | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-max@t15-20261008102309-22405-deepseek | vision.ocr | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.variant.deepseek-flash-reasoning-none | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-none@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.caption.variant.deepseek-flash-reasoning-none | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-none@t15-20261008102309-22405-deepseek | vision.caption | passed |
| t1.5.deepseek.deepseek.responses.v1.vision.ocr.variant.deepseek-flash-reasoning-none | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-flash:reasoning-none@t15-20261008102309-22405-deepseek | vision.ocr | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.variant.deepseek-v4-pro-reasoning-high | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-v4-pro:reasoning-high@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.variant.deepseek-v4-pro-reasoning-low | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-v4-pro:reasoning-low@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.variant.deepseek-v4-pro-reasoning-max | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-v4-pro:reasoning-max@t15-20261008102309-22405-deepseek | llm | passed |
| t1.5.deepseek.deepseek.responses.v1.llm.variant.deepseek-v4-pro-reasoning-none | T1.5 | deepseek/t15-20261008102309-22405-deepseek | deepseek-v4-pro:reasoning-none@t15-20261008102309-22405-deepseek | llm | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

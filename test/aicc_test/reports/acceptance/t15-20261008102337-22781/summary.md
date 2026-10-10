# AICC E2E Acceptance Report

- Run: `t15-20261008102337-22781`
- Commit: `7612d141be9938f5d1c4a89b41f20ae80459c1c1`
- Capability baseline: `official-provider-protocols-2026-10-08.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=26, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 26/26 (100.00%); passed=26, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.kimi.kimi.chat-completions.v1.llm.success | T1.5 | kimi/t15-20261008102337-22781-kimi | kimi-k3@t15-20261008102337-22781-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.stream | T1.5 | kimi/t15-20261008102337-22781-kimi | kimi-k3@t15-20261008102337-22781-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.stream-interrupted | T1.5 | kimi/t15-20261008102337-22781-kimi | kimi-k3@t15-20261008102337-22781-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.invalid_request | T1.5 | kimi/t15-20261008102337-22781-kimi | kimi-k3@t15-20261008102337-22781-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.authentication | T1.5 | kimi/t15-20261008102337-22781-kimi | kimi-k3@t15-20261008102337-22781-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.permission | T1.5 | kimi/t15-20261008102337-22781-kimi | kimi-k3@t15-20261008102337-22781-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.not_found | T1.5 | kimi/t15-20261008102337-22781-kimi | kimi-k3@t15-20261008102337-22781-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.rate_limit | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3@t15-20261008102337-22781-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.server_error | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3@t15-20261008102337-22781-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.response.malformed_response | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3@t15-20261008102337-22781-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.response.wrong_content_type | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3@t15-20261008102337-22781-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.response.missing_required_response_field | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3@t15-20261008102337-22781-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.success | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3@t15-20261008102337-22781-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.success | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3@t15-20261008102337-22781-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k2.7-code-highspeed-reasoning-medium | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k2.7-code-highspeed:reasoning-medium@t15-20261008102337-22781-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k2.7-code-highspeed-reasoning-medium | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k2.7-code-highspeed:reasoning-medium@t15-20261008102337-22781-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k2.7-code-highspeed-reasoning-medium | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k2.7-code-highspeed:reasoning-medium@t15-20261008102337-22781-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k3-reasoning-high | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3:reasoning-high@t15-20261008102337-22781-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k3-reasoning-high | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3:reasoning-high@t15-20261008102337-22781-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k3-reasoning-high | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3:reasoning-high@t15-20261008102337-22781-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k3-reasoning-low | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3:reasoning-low@t15-20261008102337-22781-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k3-reasoning-low | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3:reasoning-low@t15-20261008102337-22781-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k3-reasoning-low | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3:reasoning-low@t15-20261008102337-22781-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k3-reasoning-max | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3:reasoning-max@t15-20261008102337-22781-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k3-reasoning-max | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3:reasoning-max@t15-20261008102337-22781-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k3-reasoning-max | T1.5 | kimi/t15-20261008102337-22781-kimi-isolation-1 | kimi-k3:reasoning-max@t15-20261008102337-22781-kimi-isolation-1 | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

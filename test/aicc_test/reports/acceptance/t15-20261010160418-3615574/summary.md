# AICC E2E Acceptance Report

- Run: `t15-20261010160418-3615574`
- Commit: `a8f620c679b59a93a96d98ebb22f8a202ef0d504`
- Capability baseline: `official-provider-protocols-2026-10-10.2`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=35, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 35/35 (100.00%); passed=35, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.kimi.kimi.chat-completions.v1.llm.success | T1.5 | kimi/t15-20261010160418-3615574-kimi | kimi-k3@t15-20261010160418-3615574-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.stream | T1.5 | kimi/t15-20261010160418-3615574-kimi | kimi-k3@t15-20261010160418-3615574-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.stream-interrupted | T1.5 | kimi/t15-20261010160418-3615574-kimi | kimi-k3@t15-20261010160418-3615574-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.invalid_request | T1.5 | kimi/t15-20261010160418-3615574-kimi | kimi-k3@t15-20261010160418-3615574-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.authentication | T1.5 | kimi/t15-20261010160418-3615574-kimi | kimi-k3@t15-20261010160418-3615574-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.permission | T1.5 | kimi/t15-20261010160418-3615574-kimi | kimi-k3@t15-20261010160418-3615574-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.not_found | T1.5 | kimi/t15-20261010160418-3615574-kimi | kimi-k3@t15-20261010160418-3615574-kimi | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.rate_limit | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.error.server_error | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.response.malformed_response | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.response.wrong_content_type | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.response.missing_required_response_field | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.success | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3@t15-20261010160418-3615574-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.success | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3@t15-20261010160418-3615574-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k2.6-reasoning-medium | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.6:reasoning-medium@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k2.6-reasoning-medium | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.6:reasoning-medium@t15-20261010160418-3615574-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k2.6-reasoning-medium | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.6:reasoning-medium@t15-20261010160418-3615574-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k2.6-reasoning-none | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.6:reasoning-none@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k2.6-reasoning-none | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.6:reasoning-none@t15-20261010160418-3615574-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k2.6-reasoning-none | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.6:reasoning-none@t15-20261010160418-3615574-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k2.7-code-highspeed-reasoning-medium | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.7-code-highspeed:reasoning-medium@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k2.7-code-highspeed-reasoning-medium | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.7-code-highspeed:reasoning-medium@t15-20261010160418-3615574-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k2.7-code-highspeed-reasoning-medium | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.7-code-highspeed:reasoning-medium@t15-20261010160418-3615574-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k2.7-code-reasoning-medium | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.7-code:reasoning-medium@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k2.7-code-reasoning-medium | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.7-code:reasoning-medium@t15-20261010160418-3615574-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k2.7-code-reasoning-medium | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k2.7-code:reasoning-medium@t15-20261010160418-3615574-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k3-reasoning-high | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3:reasoning-high@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k3-reasoning-high | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3:reasoning-high@t15-20261010160418-3615574-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k3-reasoning-high | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3:reasoning-high@t15-20261010160418-3615574-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k3-reasoning-low | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3:reasoning-low@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k3-reasoning-low | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3:reasoning-low@t15-20261010160418-3615574-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k3-reasoning-low | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3:reasoning-low@t15-20261010160418-3615574-kimi-isolation-1 | vision.ocr | passed |
| t1.5.kimi.kimi.chat-completions.v1.llm.variant.kimi-k3-reasoning-max | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3:reasoning-max@t15-20261010160418-3615574-kimi-isolation-1 | llm | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.caption.variant.kimi-k3-reasoning-max | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3:reasoning-max@t15-20261010160418-3615574-kimi-isolation-1 | vision.caption | passed |
| t1.5.kimi.kimi.chat-completions.v1.vision.ocr.variant.kimi-k3-reasoning-max | T1.5 | kimi/t15-20261010160418-3615574-kimi-isolation-1 | kimi-k3:reasoning-max@t15-20261010160418-3615574-kimi-isolation-1 | vision.ocr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

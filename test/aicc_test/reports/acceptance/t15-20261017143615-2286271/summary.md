# AICC E2E Acceptance Report

- Run: `t15-20261017143615-2286271`
- Commit: `b06b8044b7bf3d5731e14d855ba33c70a5673e27`
- Capability baseline: `official-provider-protocols-2026-10-14.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=115, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 115/115 (100.00%); passed=115, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.claude.anthropic.messages.2023-06-01.llm.success | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.stream | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.stream-interrupted | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.invalid_request | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.authentication | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.permission | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.rate_limit | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.error.overloaded | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.response.malformed_response | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.response.wrong_content_type | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.response.missing_required_response_field | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.success | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.success | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.history | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.tool-history | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.structured-output | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-high@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-high@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-high@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-low@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-low@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-low@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-max@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-max@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-max@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-medium@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-medium@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-medium@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-1-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-xhigh@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-1-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-1-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5-1:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-high@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-high@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-high@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-low@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-low@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-low@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-max@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-max@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-max@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-medium@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-fable-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-xhigh@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-fable-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-fable-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-fable-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-haiku-4-5-20251001-reasoning-none | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-4-5-20251001:reasoning-none@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-haiku-4-5-20251001-reasoning-none | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-4-5-20251001:reasoning-none@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-haiku-4-5-20251001-reasoning-none | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-4-5-20251001:reasoning-none@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-haiku-4-5-20251001-reasoning-thinking | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-4-5-20251001:reasoning-thinking@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-haiku-4-5-20251001-reasoning-thinking | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-4-5-20251001:reasoning-thinking@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-haiku-4-5-20251001-reasoning-thinking | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-4-5-20251001:reasoning-thinking@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-haiku-5-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-high@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-haiku-5-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-high@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-haiku-5-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-high@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-haiku-5-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-low@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-haiku-5-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-low@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-haiku-5-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-low@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-haiku-5-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-max@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-haiku-5-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-max@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-haiku-5-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-max@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-haiku-5-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-medium@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-haiku-5-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-haiku-5-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-haiku-5-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-xhigh@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-haiku-5-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-haiku-5-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-haiku-5-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-high@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-high@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-high@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-low@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-low@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-low@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-max@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-max@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-max@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-medium@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-opus-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-xhigh@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-opus-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-opus-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-opus-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-high@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-high@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-high@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-low@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-low@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-low@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-max@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-max@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-max@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-medium@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-xhigh@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-high@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-high@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-high | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-high@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-low@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-low@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-low | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-low@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-max@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-max@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-max | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-max@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-medium@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-medium | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-medium@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.claude.anthropic.messages.2023-06-01.llm.variant.claude-sonnet-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-xhigh@t15-20261017143615-2286271-claude | llm | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.caption.variant.claude-sonnet-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.caption | passed |
| t1.5.claude.anthropic.messages.2023-06-01.vision.ocr.variant.claude-sonnet-5-reasoning-xhigh | T1.5 | claude/t15-20261017143615-2286271-claude | claude-sonnet-5:reasoning-xhigh@t15-20261017143615-2286271-claude | vision.ocr | passed |
| t1.5.custom.claude.anthropic.messages.2023-06-01.llm.success | T1.5 | claude/t15-20261017143615-2286271-custom-claude | claude-sonnet-5@t15-20261017143615-2286271-custom-claude | llm | passed |
| t1.5.custom.claude.anthropic.messages.2023-06-01.vision.ocr.success | T1.5 | claude/t15-20261017143615-2286271-custom-claude | claude-sonnet-5@t15-20261017143615-2286271-custom-claude | vision.ocr | passed |
| t1.5.custom.claude.anthropic.messages.2023-06-01.vision.caption.success | T1.5 | claude/t15-20261017143615-2286271-custom-claude | claude-sonnet-5@t15-20261017143615-2286271-custom-claude | vision.caption | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

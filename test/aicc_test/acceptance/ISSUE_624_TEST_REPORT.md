# Issue #624 AICC test report

Test date: 2026-09-25

The acceptance runner recorded `ed84d1c4ffb464d35a6bbcf8e6c469c7706fd307` as HEAD while the Issue #624 fixes were present in the working tree. The reviewed implementation was subsequently committed as `e8cd06075e1175ec81bda02672888e671ad8b143`.

## Summary

| Layer | Run | Scope | Result |
|---|---|---|---|
| T1 | `aicc-t1-2026-09-25T00-06-25-529Z-46ef5454` | DV routing, provider management, task lifecycle, usage, security, configuration, observability | 129 passed, 0 failed |
| T1.5 | `t15-20260925073100-3060149` | High-fidelity MockProvider protocol contracts | 188 passed, 0 failed; 188/188 manifest coverage |
| T2 | `aicc-2026-09-25T00-25-49-574Z-33cdf124` | Doubao real-provider inventory and protocol calls | 37 executed: 14 passed, 6 provider-restricted, 16 require semantic review, 1 failed |

All three runs completed cleanup successfully.

## T1

- Duration: 2026-09-25 00:06:25Z to 00:09:16Z.
- Cases: 129.
- Result: 129 passed, 0 failed.
- Covered logical and exact-model routing, metadata variants, policy and scheduler branches, health and runtime failover, task lifecycle and recovery, usage attribution, tenant/RBAC boundaries, configuration transactions, and trace correlation/redaction.
- Real provider calls: none.

## T1.5

- Duration: 2026-09-25 07:31:00Z to 07:33:33Z.
- Cases: 188.
- Result: 188 passed, 0 failed.
- Manifest coverage: 188/188 (100%).
- Covered successful and streaming responses, interrupted streams, provider error classes, malformed responses, wrong content types, missing required fields, media operations, asynchronous tasks, variants, and provider health recovery.
- Real provider calls: none.

## T2: Doubao

- Duration: 2026-09-25 00:25:49Z to 00:38:34Z.
- Real provider calls: 37/37 planned calls executed.
- Result: 14 passed, 6 provider-restricted, 16 require semantic review, 1 failed.
- Cleanup: passed; temporary Provider credentials were restored.
- Financial guard: total recorded/estimated exposure was USD 0.3601023 against a USD 0.76 budget; the budget was not exceeded.

### Passed

The 14 automatically passed cases covered:

- LLM calls through DeepSeek V4 Flash/Pro/V4.1 Flash, Doubao Seed 2.0 Mini, Seed 2.1 Lite/Pro/Turbo, Seed Evolving, GLM 5.3/5.3 Flash, Kimi K2.7 Code/K2.8 Preview, and MiniMax M3.
- Multimodal embedding through `doubao-embedding-vision`.

### Provider-restricted

Six cases were classified as `provider_restricted`, not excluded from model discovery:

- `doubao-seed-2.0-lite`: LLM, vision caption, and vision OCR.
- `kimi-k3`: LLM, vision caption, and vision OCR.

The provider returned `UnsupportedModel` for the tested Agent Plan account. This is an account/plan restriction rather than proof that AICC cannot support those models.

### Semantic review

Sixteen successful provider responses require a Judge or manual semantic review:

- Vision caption/OCR responses from DeepSeek V4.1 Flash, Doubao Seed 2.0 Mini, Seed 2.1 Lite/Pro, Seed 2.1 Turbo, Seed Evolving, GLM 5.3 Flash, and Kimi K2.7 Code.
- TTS output from `doubao-seed-tts-2.0`.

These cases reached the provider and returned outputs; the runner did not automatically declare their content semantically correct.

### Failed

One case failed:

- `t2.doubao.doubao-main.doubao-seed-2.1-turbo.vision.ocr`
- Classification: `provider_protocol_failed`.
- Diagnostic: `HTTP request failed`.

The failure is retained in this report and must not be represented as a passing or provider-restricted case.

## Additional verification

- `cargo test -p aicc`: 488 passed.
- `cargo test -p buckyos-api`: 213 unit tests and 4 integration tests passed.
- Desktop TypeScript check, targeted ESLint, production build, and BuckyOS desktop module build passed.
- The updated Playwright case could not run because the local Playwright Chromium executable was not installed.


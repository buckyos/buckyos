# AICC E2E Acceptance Report

- Run: `aicc-2026-10-19T03-35-35-021Z-750b79f0`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `2026-10-19.1`
- Real model calls: 0/73
- Planned maximum cost: $0.730000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $1.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=73, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.qwen-t2.cmaas-qwen3.8-flash.llm | T2 | qwen/qwen-t2 | cmaas-qwen3.8-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.deepseek-v4.1-flash.vision.caption | T2 | qwen/qwen-t2 | deepseek-v4.1-flash@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.deepseek-v4.1-flash.vision.ocr | T2 | qwen/qwen-t2 | deepseek-v4.1-flash@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.kimi-k2.6.vision.caption | T2 | qwen/qwen-t2 | kimi-k2.6@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.kimi-k2.6.vision.ocr | T2 | qwen/qwen-t2 | kimi-k2.6@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.kimi-k2.7-code.vision.caption | T2 | qwen/qwen-t2 | kimi-k2.7-code@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.kimi-k2.7-code.vision.ocr | T2 | qwen/qwen-t2 | kimi-k2.7-code@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.5-122b-a10b.llm | T2 | qwen/qwen-t2 | qwen3.5-122b-a10b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-122b-a10b.vision.caption | T2 | qwen/qwen-t2 | qwen3.5-122b-a10b@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.5-122b-a10b.vision.ocr | T2 | qwen/qwen-t2 | qwen3.5-122b-a10b@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.5-27b.llm | T2 | qwen/qwen-t2 | qwen3.5-27b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-27b.vision.caption | T2 | qwen/qwen-t2 | qwen3.5-27b@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.5-27b.vision.ocr | T2 | qwen/qwen-t2 | qwen3.5-27b@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.5-35b-a3b.llm | T2 | qwen/qwen-t2 | qwen3.5-35b-a3b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-35b-a3b.vision.caption | T2 | qwen/qwen-t2 | qwen3.5-35b-a3b@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.5-35b-a3b.vision.ocr | T2 | qwen/qwen-t2 | qwen3.5-35b-a3b@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.5-397b-a17b.llm | T2 | qwen/qwen-t2 | qwen3.5-397b-a17b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-397b-a17b.vision.caption | T2 | qwen/qwen-t2 | qwen3.5-397b-a17b@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.5-397b-a17b.vision.ocr | T2 | qwen/qwen-t2 | qwen3.5-397b-a17b@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.5-flash-2026-02-23.llm | T2 | qwen/qwen-t2 | qwen3.5-flash-2026-02-23@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-flash-2026-02-23.vision.caption | T2 | qwen/qwen-t2 | qwen3.5-flash-2026-02-23@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.5-flash-2026-02-23.vision.ocr | T2 | qwen/qwen-t2 | qwen3.5-flash-2026-02-23@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.5-flash.llm | T2 | qwen/qwen-t2 | qwen3.5-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-flash.vision.caption | T2 | qwen/qwen-t2 | qwen3.5-flash@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.5-flash.vision.ocr | T2 | qwen/qwen-t2 | qwen3.5-flash@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus-2026-02-15.llm | T2 | qwen/qwen-t2 | qwen3.5-plus-2026-02-15@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus-2026-02-15.vision.caption | T2 | qwen/qwen-t2 | qwen3.5-plus-2026-02-15@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus-2026-02-15.vision.ocr | T2 | qwen/qwen-t2 | qwen3.5-plus-2026-02-15@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus-2026-04-20.llm | T2 | qwen/qwen-t2 | qwen3.5-plus-2026-04-20@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus-2026-04-20.vision.caption | T2 | qwen/qwen-t2 | qwen3.5-plus-2026-04-20@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus-2026-04-20.vision.ocr | T2 | qwen/qwen-t2 | qwen3.5-plus-2026-04-20@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus.llm | T2 | qwen/qwen-t2 | qwen3.5-plus@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus.vision.caption | T2 | qwen/qwen-t2 | qwen3.5-plus@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.5-plus.vision.ocr | T2 | qwen/qwen-t2 | qwen3.5-plus@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.6-35b-a3b.llm | T2 | qwen/qwen-t2 | qwen3.6-35b-a3b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-35b-a3b.vision.caption | T2 | qwen/qwen-t2 | qwen3.6-35b-a3b@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.6-35b-a3b.vision.ocr | T2 | qwen/qwen-t2 | qwen3.6-35b-a3b@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.6-flash-2026-04-16.llm | T2 | qwen/qwen-t2 | qwen3.6-flash-2026-04-16@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-flash-2026-04-16.vision.caption | T2 | qwen/qwen-t2 | qwen3.6-flash-2026-04-16@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.6-flash-2026-04-16.vision.ocr | T2 | qwen/qwen-t2 | qwen3.6-flash-2026-04-16@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.6-flash.llm | T2 | qwen/qwen-t2 | qwen3.6-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-flash.vision.caption | T2 | qwen/qwen-t2 | qwen3.6-flash@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.6-flash.vision.ocr | T2 | qwen/qwen-t2 | qwen3.6-flash@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus-2026-04-02.llm | T2 | qwen/qwen-t2 | qwen3.6-plus-2026-04-02@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus-2026-04-02.vision.caption | T2 | qwen/qwen-t2 | qwen3.6-plus-2026-04-02@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus-2026-04-02.vision.ocr | T2 | qwen/qwen-t2 | qwen3.6-plus-2026-04-02@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.7-flash-2026-07-15.llm | T2 | qwen/qwen-t2 | qwen3.7-flash-2026-07-15@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-flash-2026-07-15.vision.caption | T2 | qwen/qwen-t2 | qwen3.7-flash-2026-07-15@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.7-flash-2026-07-15.vision.ocr | T2 | qwen/qwen-t2 | qwen3.7-flash-2026-07-15@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.7-flash.llm | T2 | qwen/qwen-t2 | qwen3.7-flash@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-flash.vision.caption | T2 | qwen/qwen-t2 | qwen3.7-flash@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.7-flash.vision.ocr | T2 | qwen/qwen-t2 | qwen3.7-flash@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-05-17.llm | T2 | qwen/qwen-t2 | qwen3.7-max-2026-05-17@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-05-17.vision.caption | T2 | qwen/qwen-t2 | qwen3.7-max-2026-05-17@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-05-17.vision.ocr | T2 | qwen/qwen-t2 | qwen3.7-max-2026-05-17@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-05-20.llm | T2 | qwen/qwen-t2 | qwen3.7-max-2026-05-20@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-05-20.vision.caption | T2 | qwen/qwen-t2 | qwen3.7-max-2026-05-20@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-05-20.vision.ocr | T2 | qwen/qwen-t2 | qwen3.7-max-2026-05-20@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-06-08.llm | T2 | qwen/qwen-t2 | qwen3.7-max-2026-06-08@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-06-08.vision.caption | T2 | qwen/qwen-t2 | qwen3.7-max-2026-06-08@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.7-max-2026-06-08.vision.ocr | T2 | qwen/qwen-t2 | qwen3.7-max-2026-06-08@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.7-max.llm | T2 | qwen/qwen-t2 | qwen3.7-max@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-max.vision.caption | T2 | qwen/qwen-t2 | qwen3.7-max@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.7-max.vision.ocr | T2 | qwen/qwen-t2 | qwen3.7-max@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.7-plus-2026-05-26.llm | T2 | qwen/qwen-t2 | qwen3.7-plus-2026-05-26@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.7-plus-2026-05-26.vision.caption | T2 | qwen/qwen-t2 | qwen3.7-plus-2026-05-26@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.7-plus-2026-05-26.vision.ocr | T2 | qwen/qwen-t2 | qwen3.7-plus-2026-05-26@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.8-2.4t-a95b.llm | T2 | qwen/qwen-t2 | qwen3.8-2.4t-a95b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-2.4t-a95b.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-2.4t-a95b@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.8-2.4t-a95b.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-2.4t-a95b@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.8-27b.llm | T2 | qwen/qwen-t2 | qwen3.8-27b@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-27b.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-27b@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.8-27b.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-27b@qwen-t2 | vision.ocr | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | cmaas-qwen3.8-flash | cmaas-qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4.1-flash | deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-122b-a10b | qwen3.5-122b-a10b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-27b | qwen3.5-27b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-35b-a3b | qwen3.5-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-397b-a17b | qwen3.5-397b-a17b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash-2026-02-23 | qwen3.5-flash-2026-02-23 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash | qwen3.5-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-02-15 | qwen3.5-plus-2026-02-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-04-20 | qwen3.5-plus-2026-04-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus | qwen3.5-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-35b-a3b | qwen3.6-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash-2026-04-16 | qwen3.6-flash-2026-04-16 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash | qwen3.6-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus-2026-04-02 | qwen3.6-plus-2026-04-02 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash-2026-07-15 | qwen3.7-flash-2026-07-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash | qwen3.7-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-17 | qwen3.7-max-2026-05-17 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-20 | qwen3.7-max-2026-05-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-06-08 | qwen3.7-max-2026-06-08 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max | qwen3.7-max | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus-2026-05-26 | qwen3.7-plus-2026-05-26 | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-2.4t-a95b | qwen3.8-2.4t-a95b | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-27b | qwen3.8-27b | included | physical model | - |
| qwen/qwen-t2 | cmaas-qwen3.8-flash | cmaas-qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | deepseek-v4.1-flash | deepseek-v4.1-flash | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| qwen/qwen-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-122b-a10b | qwen3.5-122b-a10b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-27b | qwen3.5-27b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-35b-a3b | qwen3.5-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-397b-a17b | qwen3.5-397b-a17b | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash-2026-02-23 | qwen3.5-flash-2026-02-23 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-flash | qwen3.5-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-02-15 | qwen3.5-plus-2026-02-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus-2026-04-20 | qwen3.5-plus-2026-04-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.5-plus | qwen3.5-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-35b-a3b | qwen3.6-35b-a3b | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash-2026-04-16 | qwen3.6-flash-2026-04-16 | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-flash | qwen3.6-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus-2026-04-02 | qwen3.6-plus-2026-04-02 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash-2026-07-15 | qwen3.7-flash-2026-07-15 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-flash | qwen3.7-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-17 | qwen3.7-max-2026-05-17 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-05-20 | qwen3.7-max-2026-05-20 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max-2026-06-08 | qwen3.7-max-2026-06-08 | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-max | qwen3.7-max | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus-2026-05-26 | qwen3.7-plus-2026-05-26 | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-2.4t-a95b | qwen3.8-2.4t-a95b | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-27b | qwen3.8-27b | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)

# AICC E2E Acceptance Report

- Run: `aicc-2026-10-10T15-29-03-475Z-f7764b07`
- Commit: `c3e1327677f9697ecdc26e51714621308d8ec86f`
- Capability baseline: `2026-10-10.2`
- Real model calls: 0/20
- Planned maximum cost: $0.200000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.200000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=12, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.kimi.kimi-t2.kimi-k2.6.llm | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | llm | skipped |
| t2.kimi.kimi-t2.kimi-k2.6.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | vision.caption | skipped |
| t2.kimi.kimi-t2.kimi-k2.6.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | vision.ocr | skipped |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.llm | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | llm | skipped |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.caption | skipped |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.ocr | skipped |
| t2.kimi.kimi-t2.kimi-k2.7-code.llm | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | llm | skipped |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | vision.caption | skipped |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | vision.ocr | skipped |
| t2.kimi.kimi-t2.kimi-k3.llm | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | llm | skipped |
| t2.kimi.kimi-t2.kimi-k3.vision.caption | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.caption | skipped |
| t2.kimi.kimi-t2.kimi-k3.vision.ocr | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.ocr | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| kimi/kimi-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code-highspeed | kimi-k2.7-code-highspeed | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| kimi/kimi-t2 | kimi-k3 | kimi-k3 | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.6 | kimi-k2.6 | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code-highspeed | kimi-k2.7-code-highspeed | included | physical model | - |
| kimi/kimi-t2 | kimi-k2.7-code | kimi-k2.7-code | included | physical model | - |
| kimi/kimi-t2 | kimi-k3 | kimi-k3 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (kimi)

# AICC E2E Acceptance Report

- Run: `aicc-2026-10-10T15-29-19-561Z-710c1dec`
- Commit: `c3e1327677f9697ecdc26e51714621308d8ec86f`
- Capability baseline: `2026-10-10.2`
- Real model calls: 20/20
- Planned maximum cost: $0.200000
- Known actual / unknown estimated exposure: $0.000000 / $0.200000
- Total exposure / budget: $0.200000 / $0.200000
- Unknown-cost calls: 20; budget exceeded: false
- Results: passed=3, failed=9, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider kimi --case t2.kimi.kimi-t2.kimi-k2.6.vision.caption --case t2.kimi.kimi-t2.kimi-k2.6.vision.ocr --case t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption --case t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.ocr --case t2.kimi.kimi-t2.kimi-k2.7-code.vision.caption --case t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr --case t2.kimi.kimi-t2.kimi-k3.llm --case t2.kimi.kimi-t2.kimi-k3.vision.caption --case t2.kimi.kimi-t2.kimi-k3.vision.ocr
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.kimi.kimi-t2.kimi-k2.6.llm | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | llm | passed |
| t2.kimi.kimi-t2.kimi-k2.6.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | vision.caption | failed |
| t2.kimi.kimi-t2.kimi-k2.6.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.6@kimi-t2 | vision.ocr | failed |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.llm | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | llm | passed |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.caption | failed |
| t2.kimi.kimi-t2.kimi-k2.7-code-highspeed.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.7-code-highspeed@kimi-t2 | vision.ocr | failed |
| t2.kimi.kimi-t2.kimi-k2.7-code.llm | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | llm | passed |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.caption | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | vision.caption | failed |
| t2.kimi.kimi-t2.kimi-k2.7-code.vision.ocr | T2 | kimi/kimi-t2 | kimi-k2.7-code@kimi-t2 | vision.ocr | failed |
| t2.kimi.kimi-t2.kimi-k3.llm | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | llm | failed |
| t2.kimi.kimi-t2.kimi-k3.vision.caption | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.caption | failed |
| t2.kimi.kimi-t2.kimi-k3.vision.ocr | T2 | kimi/kimi-t2 | kimi-k3@kimi-t2 | vision.ocr | failed |

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

- **defect.aicc.t2.kimi.kimi-t2.kimi-k3.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: LLM output omitted the requested acceptance marker. Evidence: cases/t2.kimi.kimi-t2.kimi-k3.llm.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (kimi)

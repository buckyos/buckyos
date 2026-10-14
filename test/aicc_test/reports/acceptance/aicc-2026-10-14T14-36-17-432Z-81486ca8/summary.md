# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T14-36-17-432Z-81486ca8`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 1/1
- Planned maximum cost: $0.010000
- Known actual / unknown estimated exposure: $0.000000 / $0.010000
- Total exposure / budget: $0.010000 / $0.030000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 600000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.segment
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.segment | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools | gemini-3.1-pro-preview-customtools | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools | gemini-3.1-pro-preview-customtools | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

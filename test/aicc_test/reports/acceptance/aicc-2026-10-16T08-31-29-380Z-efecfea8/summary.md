# AICC E2E Acceptance Report

- Run: `aicc-2026-10-16T08-31-29-380Z-efecfea8`
- Commit: `e897c75d6ec5aaa78f72325a27fca221bbd9409e`
- Capability baseline: `2026-10-16.1`
- Real model calls: 1/1
- Planned maximum cost: $0.010000
- Known actual / unknown estimated exposure: $0.000000 / $0.010000
- Total exposure / budget: $0.010000 / $0.100000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.lyria-3.5.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3.5@google-gemini-t2 | audio.music | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | lyria-3.5 | lyria-3.5 | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3.5 | lyria-3.5 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 1 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

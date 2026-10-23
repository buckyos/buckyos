# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T08-36-33-851Z-2281b6ee`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-23.1`
- Real model calls: 2/2
- Planned maximum cost: $0.020000
- Known actual / unknown estimated exposure: $0.000000 / $0.020000
- Total exposure / budget: $0.020000 / $30.000000
- Unknown-cost calls: 2; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-recheck-new-gpt6sol.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-recheck-new-gpt6sol-t2.gpt-6-sol.image.img2img | T2 | sn-ai-provider/sn-recheck-new-gpt6sol-t2 | gpt-6-sol@sn-recheck-new-gpt6sol-t2 | image.img2img | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-recheck-new-gpt6sol-t2 | gpt-6-sol | gpt-6-sol | included | physical model | - |
| sn-ai-provider/sn-recheck-new-gpt6sol-t2 | gpt-6-sol | gpt-6-sol | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 2 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)

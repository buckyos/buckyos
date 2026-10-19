# AICC E2E Acceptance Report

- Run: `aicc-2026-10-17T08-53-57-613Z-e0db2642`
- Commit: `bc5dfa2615c43d0fda68af307d0002a62e6cfd30`
- Capability baseline: `2026-10-10.2`
- Real model calls: 1/1
- Planned maximum cost: $0.010000
- Known actual / unknown estimated exposure: $0.000000 / $0.010000
- Total exposure / budget: $0.010000 / $0.010000
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
| t2.fal.fal-t2.fal-ai-imageutils-rembg.image.bg_remove | T2 | fal/fal-t2 | fal-ai/imageutils/rembg@fal-t2 | image.bg_remove | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| fal/fal-t2 | fal-ai/imageutils/rembg | fal-ai/imageutils/rembg | included | physical model | - |
| fal/fal-t2 | fal-ai/imageutils/rembg | fal-ai/imageutils/rembg | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 26 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (fal)

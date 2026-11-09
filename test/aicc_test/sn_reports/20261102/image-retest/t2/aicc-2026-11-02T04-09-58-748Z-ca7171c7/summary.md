# AICC E2E Acceptance Report

- Run: `aicc-2026-11-02T04-09-58-748Z-ca7171c7`
- Commit: `7c9472fda19935f8070a63943361063b8ad2e376`
- Capability baseline: `2026-11-02.2`
- Real model calls: 7/8
- Planned maximum cost: $0.160000
- Known actual / unknown estimated exposure: $0.000000 / $0.140000
- Total exposure / budget: $0.140000 / $4.000000
- Unknown-cost calls: 7; budget exceeded: false
- Results: passed=0, failed=4, provider_restricted=0, skipped=0, not_applicable=0, review=3

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-image-retest-20261102.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.img2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.img2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.inpaint | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare@sn-us-full-20261030-t2 | image.inpaint | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.inpaint | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst@sn-us-full-20261030-t2 | image.inpaint | review |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst@sn-us-full-20261030-t2 | image.txt2img | review |
| t2.cleanup.named_data | T2 | -/- | - | ndm.remove | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare | gpt-image-2.5-flare | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst | gpt-image-2.5-sunburst | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2 | gpt-image-2 | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-flare | gpt-image-2.5-flare | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2.5-sunburst | gpt-image-2.5-sunburst | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-image-2 | gpt-image-2 | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: NDN fixture read failed with HTTP 403. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.img2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.txt2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: NDN fixture read failed with HTTP 403. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-flare.image.txt2img.json
- **defect.aicc.t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: NDN fixture read failed with HTTP 403. Evidence: cases/t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-image-2.5-sunburst.image.img2img.json

## Cleanup

Status: failed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)

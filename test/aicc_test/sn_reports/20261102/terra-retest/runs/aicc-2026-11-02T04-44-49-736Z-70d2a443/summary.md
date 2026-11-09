# AICC E2E Acceptance Report

- Run: `aicc-2026-11-02T04-44-49-736Z-70d2a443`
- Commit: `7c9472fda19935f8070a63943361063b8ad2e376`
- Capability baseline: `2026-11-02.3`
- Real model calls: 4/6
- Planned maximum cost: $0.120000
- Known actual / unknown estimated exposure: $0.000000 / $0.100000
- Total exposure / budget: $0.100000 / $2.000000
- Unknown-cost calls: 4; budget exceeded: false
- Results: passed=0, failed=4, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.sn-terra-retest-20261102.local.toml --allow-credential-mutation --timeout-ms 900000 --provider sn-ai-provider --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.agent.computer_use --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.image.img2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.image.txt2img --case t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.vision.ocr
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.agent.computer_use | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | agent.computer_use | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.image.img2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | image.img2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.image.txt2img | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | image.txt2img | failed |
| t2.sn-ai-provider.sn-us-full-20261030-t2.gpt-6.1-terra.vision.ocr | T2 | sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra@sn-us-full-20261030-t2 | vision.ocr | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra | gpt-6.1-terra | included | physical model | - |
| sn-ai-provider/sn-us-full-20261030-t2 | gpt-6.1-terra | gpt-6.1-terra | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (sn-ai-provider)

# AICC E2E Acceptance Report

- Run: `aicc-2026-10-18T07-27-08-006Z-88c38b0e`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `2026-10-18.1`
- Real model calls: 2/1
- Planned maximum cost: $0.010000
- Known actual / unknown estimated exposure: $0.000000 / $0.020000
- Total exposure / budget: $0.020000 / $1.000000
- Unknown-cost calls: 2; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider qwen --case t2.qwen.qwen-t2.qwen-image-3.0-pro.image.img2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.qwen-t2.qwen-image-3.0-pro.image.img2img | T2 | qwen/qwen-t2 | qwen-image-3.0-pro@qwen-t2 | image.img2img | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.qwen.qwen-t2.qwen-image-3.0-pro.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.qwen.qwen-t2.qwen-image-3.0-pro.image.img2img.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)

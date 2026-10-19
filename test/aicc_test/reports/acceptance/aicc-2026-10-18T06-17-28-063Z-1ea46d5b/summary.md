# AICC E2E Acceptance Report

- Run: `aicc-2026-10-18T06-17-28-063Z-1ea46d5b`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `2026-10-18.1`
- Real model calls: 3/5
- Planned maximum cost: $0.050000
- Known actual / unknown estimated exposure: $0.000000 / $0.030000
- Total exposure / budget: $0.030000 / $5.000000
- Unknown-cost calls: 3; budget exceeded: false
- Results: passed=1, failed=2, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider qwen --case t2.qwen.qwen-t2.qwen-image-3.0-pro.image.img2img --case t2.qwen.qwen-t2.qwen-image-3.0-pro.image.txt2img
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.glm.glm-t2.glm-5.3.llm | T2 | glm/glm-t2 | glm-5.3@glm-t2 | llm | passed |
| t2.qwen.qwen-t2.qwen-image-3.0-pro.image.img2img | T2 | qwen/qwen-t2 | qwen-image-3.0-pro@qwen-t2 | image.img2img | failed |
| t2.qwen.qwen-t2.qwen-image-3.0-pro.image.txt2img | T2 | qwen/qwen-t2 | qwen-image-3.0-pro@qwen-t2 | image.txt2img | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| glm/glm-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| glm/glm-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |
| glm/glm-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| glm/glm-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (glm, qwen)

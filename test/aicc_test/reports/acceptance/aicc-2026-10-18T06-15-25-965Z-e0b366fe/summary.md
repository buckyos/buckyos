# AICC E2E Acceptance Report

- Run: `aicc-2026-10-18T06-15-25-965Z-e0b366fe`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `2026-10-18.1`
- Real model calls: 0/10
- Planned maximum cost: $0.100000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $5.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=10, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.doubao-agent-plan.doubao-agent-plan-t2.glm-5.3.llm | T2 | doubao-agent-plan/doubao-agent-plan-t2 | glm-5.3@doubao-agent-plan-t2 | llm | skipped |
| t2.glm.glm-t2.glm-5.3.llm | T2 | glm/glm-t2 | glm-5.3@glm-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen-image-3.0-pro.image.img2img | T2 | qwen/qwen-t2 | qwen-image-3.0-pro@qwen-t2 | image.img2img | skipped |
| t2.qwen.qwen-t2.qwen-image-3.0-pro.image.txt2img | T2 | qwen/qwen-t2 | qwen-image-3.0-pro@qwen-t2 | image.txt2img | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus.llm | T2 | qwen/qwen-t2 | qwen3.6-plus@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus.vision.caption | T2 | qwen/qwen-t2 | qwen3.6-plus@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.6-plus.vision.ocr | T2 | qwen/qwen-t2 | qwen3.6-plus@qwen-t2 | vision.ocr | skipped |
| t2.qwen.qwen-t2.qwen3.8-max-0902.llm | T2 | qwen/qwen-t2 | qwen3.8-max-0902@qwen-t2 | llm | skipped |
| t2.qwen.qwen-t2.qwen3.8-max-0902.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-max-0902@qwen-t2 | vision.caption | skipped |
| t2.qwen.qwen-t2.qwen3.8-max-0902.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-max-0902@qwen-t2 | vision.ocr | skipped |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| doubao-agent-plan/doubao-agent-plan-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| glm/glm-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| glm/glm-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |
| doubao-agent-plan/doubao-agent-plan-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| glm/glm-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| glm/glm-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | glm-5.3 | glm-5.3 | included | physical model | - |
| qwen/qwen-t2 | glm-5 | glm-5 | included | physical model | - |
| qwen/qwen-t2 | qwen-image-3.0-pro | qwen-image-3.0-pro | included | physical model | - |
| qwen/qwen-t2 | qwen3.6-plus | qwen3.6-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-max-0902 | qwen3.8-max-0902 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (doubao-agent-plan, glm, qwen)

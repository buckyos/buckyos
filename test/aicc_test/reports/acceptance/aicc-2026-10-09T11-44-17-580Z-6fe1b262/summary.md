# AICC E2E Acceptance Report

- Run: `aicc-2026-10-09T11-44-17-580Z-6fe1b262`
- Commit: `0c32470f0ebaa0f14a1a262bc2bcb0eea553d8fc`
- Capability baseline: `2026-10-09.4`
- Real model calls: 8/8
- Planned maximum cost: $0.080000
- Known actual / unknown estimated exposure: $0.000000 / $0.080000
- Total exposure / budget: $0.080000 / $0.080000
- Unknown-cost calls: 8; budget exceeded: false
- Results: passed=4, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.qwen.qwen-t2.qwen3.7-plus.vision.caption | T2 | qwen/qwen-t2 | qwen3.7-plus@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.qwen3.7-plus.vision.ocr | T2 | qwen/qwen-t2 | qwen3.7-plus@qwen-t2 | vision.ocr | passed |
| t2.qwen.qwen-t2.qwen3.8-flash.vision.caption | T2 | qwen/qwen-t2 | qwen3.8-flash@qwen-t2 | vision.caption | passed |
| t2.qwen.qwen-t2.qwen3.8-flash.vision.ocr | T2 | qwen/qwen-t2 | qwen3.8-flash@qwen-t2 | vision.ocr | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| qwen/qwen-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |
| qwen/qwen-t2 | qwen3.7-plus | qwen3.7-plus | included | physical model | - |
| qwen/qwen-t2 | qwen3.8-flash | qwen3.8-flash | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (qwen)

# AICC E2E Acceptance Report

- Run: `t15-20261014103839-301910`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `official-provider-protocols-2026-10-13.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=4, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 4/4 (100.00%); passed=4, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.success | T1.5 | fal/t15-20261014103839-301910-fal | fal-ai/imageutils/rembg@t15-20261014103839-301910-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.ta[REDACTED] | T1.5 | fal/t15-20261014103839-301910-fal | fal-ai/imageutils/rembg@t15-20261014103839-301910-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async | T1.5 | fal/t15-20261014103839-301910-fal | fal-ai/imageutils/rembg@t15-20261014103839-301910-fal | image.bg_remove | passed |
| t1.5.fal.fal.imageutils-rembg.queue-v1.image.bg_remove.async-failed | T1.5 | fal/t15-20261014103839-301910-fal | fal-ai/imageutils/rembg@t15-20261014103839-301910-fal | image.bg_remove | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

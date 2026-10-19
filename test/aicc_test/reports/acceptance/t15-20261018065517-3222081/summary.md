# AICC E2E Acceptance Report

- Run: `t15-20261018065517-3222081`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `official-provider-protocols-2026-10-18.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=3, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 3/3 (100.00%); passed=3, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.qwen.qwen.dashscope.image_synthesis.v1.image.txt2img.success | T1.5 | qwen/t15-20261018065517-3222081-qwen | wanx2.1-t2i-turbo@t15-20261018065517-3222081-qwen | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_generation.v1.image.txt2img.success | T1.5 | qwen/t15-20261018065517-3222081-qwen | qwen-image-3.0-pro@t15-20261018065517-3222081-qwen | image.txt2img | passed |
| t1.5.qwen.qwen.dashscope.image_edit.v1.image.img2img.success | T1.5 | qwen/t15-20261018065517-3222081-qwen | qwen-image-3.0-pro@t15-20261018065517-3222081-qwen | image.img2img | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

# AICC E2E Acceptance Report

- Run: `t15-20261023083355-3941542`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `official-provider-protocols-2026-10-23.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=10, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 10/10 (100.00%); passed=10, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.txt2img.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | gpt-6-sol@t15-20261023083355-3941542-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.openai-responses.background.v1.image.img2img.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | gpt-6-sol@t15-20261023083355-3941542-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.img2img.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | gpt-image-2@t15-20261023083355-3941542-sn-ai-provider | image.img2img | passed |
| t1.5.sn-ai-provider.sn.openai-images-edits.v1.image.inpaint.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | gpt-image-2@t15-20261023083355-3941542-sn-ai-provider | image.inpaint | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.txt2video.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | wan-3.0@t15-20261023083355-3941542-sn-ai-provider | video.txt2video | passed |
| t1.5.sn-ai-provider.sn.openai-videos.v1.video.img2video.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | wan-3.0@t15-20261023083355-3941542-sn-ai-provider | video.img2video | passed |
| t1.5.sn-ai-provider.sn.openai-video-edits.v1.video.video2video.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | wan-3.0@t15-20261023083355-3941542-sn-ai-provider | video.video2video | passed |
| t1.5.sn-ai-provider.sn.openai-video-extensions.v1.video.extend.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | wan-3.0@t15-20261023083355-3941542-sn-ai-provider | video.extend | passed |
| t1.5.sn-ai-provider.sn.qwen-images-generations.v1.image.txt2img.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | qwen-image-3-pro@t15-20261023083355-3941542-sn-ai-provider | image.txt2img | passed |
| t1.5.sn-ai-provider.sn.qwen-images-edits.v1.image.img2img.ta[REDACTED] | T1.5 | sn-ai-provider/t15-20261023083355-3941542-sn-ai-provider | qwen-image-3-pro@t15-20261023083355-3941542-sn-ai-provider | image.img2img | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

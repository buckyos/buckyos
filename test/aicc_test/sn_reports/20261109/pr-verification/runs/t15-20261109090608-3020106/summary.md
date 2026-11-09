# AICC E2E Acceptance Report

- Run: `t15-20261109090608-3020106`
- Commit: `8326b53f2eed96f223451ca987d9ddfb9c9324c4`
- Capability baseline: `2026-11-02.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=12, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 12/12 (100.00%); passed=12, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.openai.openai.images.generations.v1.image.txt2img.model.gpt-image-2.5-flare.success | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-flare@t15-20261109090608-3020106-openai | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-flare@t15-20261109090608-3020106-openai | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.model.gpt-image-2.5-sunburst.success | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-sunburst@t15-20261109090608-3020106-openai | image.txt2img | passed |
| t1.5.openai.openai.images.generations.v1.image.txt2img.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-sunburst@t15-20261109090608-3020106-openai | image.txt2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.model.gpt-image-2.5-flare.success | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-flare@t15-20261109090608-3020106-openai | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-flare@t15-20261109090608-3020106-openai | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.model.gpt-image-2.5-sunburst.success | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-sunburst@t15-20261109090608-3020106-openai | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.img2img.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-sunburst@t15-20261109090608-3020106-openai | image.img2img | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.model.gpt-image-2.5-flare.success | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-flare@t15-20261109090608-3020106-openai | image.inpaint | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.model.gpt-image-2.5-flare.task-result-artifact | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-flare@t15-20261109090608-3020106-openai | image.inpaint | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.model.gpt-image-2.5-sunburst.success | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-sunburst@t15-20261109090608-3020106-openai | image.inpaint | passed |
| t1.5.openai.openai.images.edits.v1.image.inpaint.model.gpt-image-2.5-sunburst.task-result-artifact | T1.5 | openai/t15-20261109090608-3020106-openai | gpt-image-2.5-sunburst@t15-20261109090608-3020106-openai | image.inpaint | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

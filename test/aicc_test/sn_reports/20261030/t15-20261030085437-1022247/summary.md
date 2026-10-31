# AICC E2E Acceptance Report

- Run: `t15-20261030085437-1022247`
- Commit: `d40291a9a9085d1cec7ccb37d519d9115baf534c`
- Capability baseline: `2026-10-30.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=2, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 2/2 (100.00%); passed=2, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.sn-ai-provider.sn.openai-responses.v1.llm.model.voxtral-small-24b-2507.success | T1.5 | sn-ai-provider/t15-20261030085437-1022247-sn-ai-provider | voxtral-small-24b-2507@t15-20261030085437-1022247-sn-ai-provider | llm | passed |
| t1.5.sn-ai-provider.sn.audio-chat.v1.audio.asr.success | T1.5 | sn-ai-provider/t15-20261030085437-1022247-sn-ai-provider | voxtral-small-24b-2507@t15-20261030085437-1022247-sn-ai-provider | audio.asr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

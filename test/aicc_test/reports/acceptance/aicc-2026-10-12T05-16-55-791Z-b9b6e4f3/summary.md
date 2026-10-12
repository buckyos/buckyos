# AICC E2E Acceptance Report

- Run: `aicc-2026-10-12T05-16-55-791Z-b9b6e4f3`
- Commit: `6a2894a88d79780ce0c3a601ff4b07f91ca135c2`
- Capability baseline: `2026-10-10.2`
- Real model calls: 2/2
- Planned maximum cost: $0.020000
- Known actual / unknown estimated exposure: $0.000000 / $0.020000
- Total exposure / budget: $0.020000 / $0.020000
- Unknown-cost calls: 2; budget exceeded: false
- Results: passed=1, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 120000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.doubao-agent-plan.doubao-agent-plan-t2.kimi-k2.8-preview.llm | T2 | doubao-agent-plan/doubao-agent-plan-t2 | kimi-k2.8-preview@doubao-agent-plan-t2 | llm | passed |
| t2.doubao-speech.doubao-speech-t2.doubao-seed-tts-2.0.audio.tts | T2 | doubao-speech/doubao-speech-t2 | doubao-seed-tts-2.0@doubao-speech-t2 | audio.tts | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| doubao-agent-plan/doubao-agent-plan-t2 | kimi-k2.8-preview | kimi-k2.8-preview | included | physical model | - |
| doubao-speech/doubao-speech-t2 | doubao-seed-tts-2.0 | doubao-seed-tts-2.0 | included | physical model | - |
| doubao-agent-plan/doubao-agent-plan-t2 | kimi-k2.8-preview | kimi-k2.8-preview | included | physical model | - |
| doubao-speech/doubao-speech-t2 | doubao-seed-tts-2.0 | doubao-seed-tts-2.0 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 1 generated output object(s)
- temporary Provider credentials restored after testing (doubao-agent-plan, doubao-speech)

# AICC E2E Acceptance Report

- Run: `aicc-2026-10-23T07-47-52-194Z-8515748e`
- Commit: `53b4431e2d46ec731598ed0343d4a6596bee8d15`
- Capability baseline: `2026-10-23.1`
- Real model calls: 1/1
- Planned maximum cost: $0.010000
- Known actual / unknown estimated exposure: $0.000000 / $0.010000
- Total exposure / budget: $0.010000 / $30.000000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.native.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.doubao-speech.native-doubao-speech-t2.doubao-seed-asr-2.0.audio.asr | T2 | doubao-speech/native-doubao-speech-t2 | doubao-seed-asr-2.0@native-doubao-speech-t2 | audio.asr | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| doubao-speech/native-doubao-speech-t2 | doubao-seed-asr-2.0 | doubao-seed-asr-2.0 | included | physical model | - |
| doubao-speech/native-doubao-speech-t2 | doubao-seed-asr-2.0 | doubao-seed-asr-2.0 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 26 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (doubao-speech)

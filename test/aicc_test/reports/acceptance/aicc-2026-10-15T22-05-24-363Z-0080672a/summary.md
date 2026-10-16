# AICC E2E Acceptance Report

- Run: `aicc-2026-10-15T22-05-24-363Z-0080672a`
- Commit: `e897c75d6ec5aaa78f72325a27fca221bbd9409e`
- Capability baseline: `2026-10-15.1`
- Real model calls: 3/3
- Planned maximum cost: $0.030000
- Known actual / unknown estimated exposure: $0.000000 / $0.030000
- Total exposure / budget: $0.030000 / $0.100000
- Unknown-cost calls: 3; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=3

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.openrouter.openrouter-t2.google-gemini-3.1-flash-tts-preview.audio.tts | T2 | openrouter/openrouter-t2 | google/gemini-3.1-flash-tts-preview@openrouter-t2 | audio.tts | review |
| t2.openrouter.openrouter-t2.google-gemini-3.8-flash-tts.audio.tts | T2 | openrouter/openrouter-t2 | google/gemini-3.8-flash-tts@openrouter-t2 | audio.tts | review |
| t2.openrouter.openrouter-t2.minimax-speech-2.8-turbo.audio.tts | T2 | openrouter/openrouter-t2 | minimax/speech-2.8-turbo@openrouter-t2 | audio.tts | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| openrouter/openrouter-t2 | google/gemini-3.1-flash-tts-preview | google/gemini-3.1-flash-tts-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-tts | google/gemini-3.8-flash-tts | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-turbo | minimax/speech-2.8-turbo | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.1-flash-tts-preview | google/gemini-3.1-flash-tts-preview | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.8-flash-tts | google/gemini-3.8-flash-tts | included | physical model | - |
| openrouter/openrouter-t2 | minimax/speech-2.8-turbo | minimax/speech-2.8-turbo | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 3 generated output object(s)
- temporary Provider credentials restored after testing (openrouter)

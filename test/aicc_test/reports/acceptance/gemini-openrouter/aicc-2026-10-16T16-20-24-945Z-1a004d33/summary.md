# AICC E2E Acceptance Report

- Run: `aicc-2026-10-16T16-20-24-945Z-1a004d33`
- Commit: `6d025991be1a359de271a110191c273a17c0fcfe`
- Capability baseline: `2026-10-16.2`
- Real model calls: 4/4
- Planned maximum cost: $0.040000
- Known actual / unknown estimated exposure: $0.010181 / $0.010000
- Total exposure / budget: $0.020181 / $0.100000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=4

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.openrouter.openrouter-t2.google-gemini-3.5-transcribe.audio.asr | T2 | openrouter/openrouter-t2 | google/gemini-3.5-transcribe@openrouter-t2 | audio.asr | review |
| t2.openrouter.openrouter-t2.google-gemini-4-argon.llm | T2 | openrouter/openrouter-t2 | google/gemini-4-argon@openrouter-t2 | llm | review |
| t2.openrouter.openrouter-t2.google-gemini-4-argon.vision.caption | T2 | openrouter/openrouter-t2 | google/gemini-4-argon@openrouter-t2 | vision.caption | review |
| t2.openrouter.openrouter-t2.google-gemini-4-argon.vision.ocr | T2 | openrouter/openrouter-t2 | google/gemini-4-argon@openrouter-t2 | vision.ocr | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| openrouter/openrouter-t2 | google/gemini-3.5-transcribe | google/gemini-3.5-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-4-argon | google/gemini-4-argon | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-3.5-transcribe | google/gemini-3.5-transcribe | included | physical model | - |
| openrouter/openrouter-t2 | google/gemini-4-argon | google/gemini-4-argon | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (openrouter)

# AICC E2E Acceptance Report

- Run: `aicc-2026-10-16T16-19-26-107Z-793e8527`
- Commit: `6d025991be1a359de271a110191c273a17c0fcfe`
- Capability baseline: `2026-10-16.2`
- Real model calls: 7/7
- Planned maximum cost: $0.070000
- Known actual / unknown estimated exposure: $0.000246 / $0.060000
- Total exposure / budget: $0.060246 / $0.100000
- Unknown-cost calls: 6; budget exceeded: false
- Results: passed=0, failed=5, provider_restricted=0, skipped=0, not_applicable=0, review=2

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-4-argon.audio.asr --case t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.caption --case t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.detect --case t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.ocr --case t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.segment
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-3.5-transcribe.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-transcribe@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | audio.asr | failed |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.llm | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | llm | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.caption | failed |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.detect | failed |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.ocr | failed |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.segment | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-3.5-transcribe | gemini-3.5-transcribe | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-transcribe | gemini-3.5-transcribe | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

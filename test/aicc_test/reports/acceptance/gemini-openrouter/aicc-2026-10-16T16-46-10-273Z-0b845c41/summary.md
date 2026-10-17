# AICC E2E Acceptance Report

- Run: `aicc-2026-10-16T16-46-10-273Z-0b845c41`
- Commit: `6d025991be1a359de271a110191c273a17c0fcfe`
- Capability baseline: `2026-10-16.2`
- Real model calls: 6/6
- Planned maximum cost: $0.060000
- Known actual / unknown estimated exposure: $0.000246 / $0.050000
- Total exposure / budget: $0.050246 / $0.200000
- Unknown-cost calls: 5; budget exceeded: false
- Results: passed=0, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=6

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-4-argon.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.llm | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | llm | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.segment | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

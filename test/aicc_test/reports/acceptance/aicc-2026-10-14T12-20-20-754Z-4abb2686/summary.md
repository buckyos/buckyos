# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T12-20-20-754Z-4abb2686`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 3/3
- Planned maximum cost: $0.030000
- Known actual / unknown estimated exposure: $0.000814 / $0.020000
- Total exposure / budget: $0.020814 / $1.000000
- Unknown-cost calls: 2; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=2

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-preview-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts@google-gemini-t2 | audio.tts | review |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.segment | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts | gemini-2.5-flash-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts | gemini-2.5-flash-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 1 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

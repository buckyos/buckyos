# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T12-09-04-912Z-90defb00`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 16/16
- Planned maximum cost: $0.160000
- Known actual / unknown estimated exposure: $0.004536 / $0.140000
- Total exposure / budget: $0.144536 / $1.000000
- Unknown-cost calls: 14; budget exceeded: false
- Results: passed=0, failed=3, provider_restricted=6, skipped=0, not_applicable=0, review=7

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash-preview-tts.audio.tts --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | audio.asr | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.llm | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | llm | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.caption | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.detect | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.ocr | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.segment | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-preview-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts@google-gemini-t2 | audio.tts | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.detect | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.segment | failed |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.segment | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-2.5-flash-lite | gemini-2.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts | gemini-2.5-flash-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-flash-preview | gemini-3-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash-lite | gemini-3.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-lite | gemini-2.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts | gemini-2.5-flash-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-flash-preview | gemini-3-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash-lite | gemini-3.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

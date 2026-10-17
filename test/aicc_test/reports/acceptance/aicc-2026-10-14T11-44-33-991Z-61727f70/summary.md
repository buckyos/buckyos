# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T11-44-33-991Z-61727f70`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 41/41
- Planned maximum cost: $0.410000
- Known actual / unknown estimated exposure: $0.005171 / $0.300000
- Total exposure / budget: $0.305171 / $1.000000
- Unknown-cost calls: 30; budget exceeded: false
- Results: passed=6, failed=16, provider_restricted=0, skipped=0, not_applicable=0, review=19

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.audio.asr --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.llm --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.caption --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.detect --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.ocr --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.segment --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash-preview-tts.audio.tts --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.ocr --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment --case t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.detect --case t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.segment --case t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.detect --case t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.segment --case t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.detect --case t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.segment
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | audio.asr | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.llm | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | llm | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.caption | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.detect | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.ocr | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.segment | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-preview-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts@google-gemini-t2 | audio.tts | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.detect | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.ocr | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.segment | failed |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.llm | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.detect | failed |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.segment | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-tts-preview.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-tts-preview@google-gemini-t2 | audio.tts | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.detect | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.segment | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.5-transcribe.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-transcribe@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash-lite-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash-lite-tts@google-gemini-t2 | audio.tts | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash-tts@google-gemini-t2 | audio.tts | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.detect | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.segment | failed |
| t2.google-gemini.google-gemini-t2.gemma-4-26b-a4b-it.llm | T2 | google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemma-4-26b-a4b-it.vision.caption | T2 | google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemma-4-26b-a4b-it.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemma-4-31b-it.llm | T2 | google-gemini/google-gemini-t2 | gemma-4-31b-it@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemma-4-31b-it.vision.caption | T2 | google-gemini/google-gemini-t2 | gemma-4-31b-it@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemma-4-31b-it.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemma-4-31b-it@google-gemini-t2 | vision.ocr | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-2.5-flash-lite | gemini-2.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts | gemini-2.5-flash-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-flash-preview | gemini-3-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-tts-preview | gemini-3.1-flash-tts-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash-lite | gemini-3.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-transcribe | gemini-3.5-transcribe | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash-lite-tts | gemini-3.8-flash-lite-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash-tts | gemini-3.8-flash-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it | gemma-4-26b-a4b-it | included | physical model | - |
| google-gemini/google-gemini-t2 | gemma-4-31b-it | gemma-4-31b-it | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-lite | gemini-2.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-preview-tts | gemini-2.5-flash-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-flash-preview | gemini-3-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-tts-preview | gemini-3.1-flash-tts-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash-lite | gemini-3.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-transcribe | gemini-3.5-transcribe | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash-lite-tts | gemini-3.8-flash-lite-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash-tts | gemini-3.8-flash-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemma-4-26b-a4b-it | gemma-4-26b-a4b-it | included | physical model | - |
| google-gemini/google-gemini-t2 | gemma-4-31b-it | gemma-4-31b-it | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: expected structured vision output. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.ocr** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.ocr.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.detect** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: expected structured vision output. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.detect.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.segment** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: expected structured vision output. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.segment.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.detect** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: expected structured vision output. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.detect.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.segment** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: expected structured vision output. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.segment.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.detect** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: expected structured vision output. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.detect.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.segment** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: expected structured vision output. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.segment.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 3 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

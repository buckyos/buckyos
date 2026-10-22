# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T12-51-12-164Z-d93a924c`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 30/30
- Planned maximum cost: $0.300000
- Known actual / unknown estimated exposure: $0.003578 / $0.250000
- Total exposure / budget: $0.253578 / $1.000000
- Unknown-cost calls: 25; budget exceeded: false
- Results: passed=5, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=24

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.segment
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.segment | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.llm | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.segment | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview | gemini-3.1-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash | gemini-3.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.6-flash | gemini-3.6-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.7-flash | gemini-3.7-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview | gemini-3.1-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash | gemini-3.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.6-flash | gemini-3.6-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.7-flash | gemini-3.7-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.segment** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.segment.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T12-45-35-924Z-5bc023c3`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 31/27
- Planned maximum cost: $0.270000
- Known actual / unknown estimated exposure: $0.002406 / $0.250000
- Total exposure / budget: $0.252406 / $1.000000
- Unknown-cost calls: 25; budget exceeded: false
- Results: passed=1, failed=6, provider_restricted=7, skipped=0, not_applicable=0, review=13

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-2.5-flash-image.image.img2img --case t2.google-gemini.google-gemini-t2.gemini-3-pro-image.image.img2img --case t2.google-gemini.google-gemini-t2.gemini-3.1-flash-image.image.img2img --case t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite-image.image.img2img --case t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.llm --case t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.detect
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-image.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-image@google-gemini-t2 | image.img2img | failed |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-image.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-image@google-gemini-t2 | image.txt2img | review |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro-preview-tts.audio.tts | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro-preview-tts@google-gemini-t2 | audio.tts | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | audio.asr | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.llm | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | llm | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.caption | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.detect | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.ocr | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.segment | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-3-pro-image.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-3-pro-image@google-gemini-t2 | image.img2img | failed |
| t2.google-gemini.google-gemini-t2.gemini-3-pro-image.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-3-pro-image@google-gemini-t2 | image.txt2img | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-image.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-image@google-gemini-t2 | image.img2img | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-image.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-image@google-gemini-t2 | image.txt2img | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite-image.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-image@google-gemini-t2 | image.img2img | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite-image.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-image@google-gemini-t2 | image.txt2img | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.audio.asr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | audio.asr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.llm | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | llm | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.detect | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.segment | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-2.5-flash-image | gemini-2.5-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro-preview-tts | gemini-2.5-pro-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro | gemini-2.5-pro | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-pro-image | gemini-3-pro-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-image | gemini-3.1-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-image | gemini-3.1-flash-lite-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite | gemini-3.1-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools | gemini-3.1-pro-preview-customtools | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-image | gemini-2.5-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro-preview-tts | gemini-2.5-pro-preview-tts | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro | gemini-2.5-pro | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-pro-image | gemini-3-pro-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-image | gemini-3.1-flash-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite-image | gemini-3.1-flash-lite-image | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite | gemini-3.1-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools | gemini-3.1-pro-preview-customtools | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.llm** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.llm.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.detect** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.detect.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 8 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

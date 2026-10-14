# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T13-00-40-541Z-0d886257`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 17/28
- Planned maximum cost: $0.280000
- Known actual / unknown estimated exposure: $1.334906 / $0.140000
- Total exposure / budget: $1.474906 / $1.000000
- Unknown-cost calls: 14; budget exceeded: true
- Results: passed=2, failed=15, provider_restricted=0, skipped=0, not_applicable=0, review=8

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-embedding-001.embedding.text --case t2.google-gemini.google-gemini-t2.gemini-embedding-2.embedding.text --case t2.google-gemini.google-gemini-t2.gemini-nano-banana-2.1.image.img2img --case t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.extend --case t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.img2video --case t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.txt2video --case t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.video2video --case t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.extend --case t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.img2video --case t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.txt2video --case t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.extend --case t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.img2video --case t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.txt2video --case t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.img2video --case t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.txt2video
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-embedding-001.embedding.text | T2 | google-gemini/google-gemini-t2 | gemini-embedding-001@google-gemini-t2 | embedding.text | failed |
| t2.google-gemini.google-gemini-t2.gemini-embedding-2.embedding.multimodal | T2 | google-gemini/google-gemini-t2 | gemini-embedding-2@google-gemini-t2 | embedding.multimodal | passed |
| t2.google-gemini.google-gemini-t2.gemini-embedding-2.embedding.text | T2 | google-gemini/google-gemini-t2 | gemini-embedding-2@google-gemini-t2 | embedding.text | failed |
| t2.google-gemini.google-gemini-t2.gemini-nano-banana-2.1.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-nano-banana-2.1@google-gemini-t2 | image.img2img | failed |
| t2.google-gemini.google-gemini-t2.gemini-nano-banana-2.1.image.txt2img | T2 | google-gemini/google-gemini-t2 | gemini-nano-banana-2.1@google-gemini-t2 | image.txt2img | review |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.extend | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.extend | failed |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.img2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.img2video | failed |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.txt2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.txt2video | failed |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.video2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.video2video | failed |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.llm | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | llm | passed |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.caption | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.caption | review |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.ocr | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.ocr | review |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.lyria-3-clip-preview.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3-clip-preview@google-gemini-t2 | audio.music | review |
| t2.google-gemini.google-gemini-t2.lyria-3-pro-preview.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3-pro-preview@google-gemini-t2 | audio.music | review |
| t2.google-gemini.google-gemini-t2.lyria-3.5.audio.music | T2 | google-gemini/google-gemini-t2 | lyria-3.5@google-gemini-t2 | audio.music | review |
| t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.extend | T2 | google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview@google-gemini-t2 | video.extend | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview@google-gemini-t2 | video.img2video | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview@google-gemini-t2 | video.txt2video | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.extend | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.extend | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.img2video | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.txt2video | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview@google-gemini-t2 | video.img2video | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview@google-gemini-t2 | video.txt2video | failed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-embedding-001 | gemini-embedding-001 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2 | gemini-embedding-2 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-nano-banana-2.1 | gemini-nano-banana-2.1 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-1.1-flash | gemini-omni-1.1-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview | gemini-robotics-er-2-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-clip-preview | lyria-3-clip-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-pro-preview | lyria-3-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3.5 | lyria-3.5 | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview | veo-3.1-fast-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-generate-preview | veo-3.1-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview | veo-3.1-lite-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-001 | gemini-embedding-001 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2 | gemini-embedding-2 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-nano-banana-2.1 | gemini-nano-banana-2.1 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-1.1-flash | gemini-omni-1.1-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview | gemini-robotics-er-2-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-clip-preview | lyria-3-clip-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3-pro-preview | lyria-3-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | lyria-3.5 | lyria-3.5 | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview | veo-3.1-fast-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-generate-preview | veo-3.1-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview | veo-3.1-lite-generate-preview | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-nano-banana-2.1.image.img2img** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-nano-banana-2.1.image.img2img.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.extend** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.extend.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.video2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.video2video.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.extend** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.extend.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.img2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.img2video.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.txt2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.txt2video.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.extend** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.extend.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.img2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.img2video.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.txt2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.txt2video.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.img2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.img2video.json
- **defect.aicc.t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.txt2video** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed Error: financial budget exhausted: exposure $1.474906 + reservation $0.010000 > $1.000000. Evidence: cases/t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.txt2video.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 6 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

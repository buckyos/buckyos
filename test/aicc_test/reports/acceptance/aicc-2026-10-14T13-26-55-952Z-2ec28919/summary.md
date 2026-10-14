# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T13-26-55-952Z-2ec28919`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 17/16
- Planned maximum cost: $0.160000
- Known actual / unknown estimated exposure: $9.154284 / $0.070000
- Total exposure / budget: $9.224284 / $15.000000
- Unknown-cost calls: 7; budget exceeded: false
- Results: passed=0, failed=5, provider_restricted=0, skipped=0, not_applicable=0, review=8

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-nano-banana-2.1.image.img2img --case t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.extend --case t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.video2video --case t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.extend --case t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.extend
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-nano-banana-2.1.image.img2img | T2 | google-gemini/google-gemini-t2 | gemini-nano-banana-2.1@google-gemini-t2 | image.img2img | failed |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.extend | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.extend | failed |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.img2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.img2video | review |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.txt2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.txt2video | review |
| t2.google-gemini.google-gemini-t2.gemini-omni-1.1-flash.video.video2video | T2 | google-gemini/google-gemini-t2 | gemini-omni-1.1-flash@google-gemini-t2 | video.video2video | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.extend | T2 | google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview@google-gemini-t2 | video.extend | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview@google-gemini-t2 | video.img2video | review |
| t2.google-gemini.google-gemini-t2.veo-3.1-fast-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview@google-gemini-t2 | video.txt2video | review |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.extend | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.extend | failed |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.img2video | review |
| t2.google-gemini.google-gemini-t2.veo-3.1-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-generate-preview@google-gemini-t2 | video.txt2video | review |
| t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.img2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview@google-gemini-t2 | video.img2video | review |
| t2.google-gemini.google-gemini-t2.veo-3.1-lite-generate-preview.video.txt2video | T2 | google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview@google-gemini-t2 | video.txt2video | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-nano-banana-2.1 | gemini-nano-banana-2.1 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-1.1-flash | gemini-omni-1.1-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview | veo-3.1-fast-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-generate-preview | veo-3.1-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview | veo-3.1-lite-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-nano-banana-2.1 | gemini-nano-banana-2.1 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-omni-1.1-flash | gemini-omni-1.1-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-fast-generate-preview | veo-3.1-fast-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-generate-preview | veo-3.1-generate-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | veo-3.1-lite-generate-preview | veo-3.1-lite-generate-preview | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 4 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

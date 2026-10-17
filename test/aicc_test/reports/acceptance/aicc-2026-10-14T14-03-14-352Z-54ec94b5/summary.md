# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T14-03-14-352Z-54ec94b5`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 28/28
- Planned maximum cost: $0.280000
- Known actual / unknown estimated exposure: $0.005862 / $0.240000
- Total exposure / budget: $0.245862 / $0.300000
- Unknown-cost calls: 24; budget exceeded: false
- Results: passed=0, failed=1, provider_restricted=4, skipped=0, not_applicable=0, review=23

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --provider google-gemini --case t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.segment
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.detect | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash-lite@google-gemini-t2 | vision.segment | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-2.5-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-flash@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.detect | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-2.5-pro.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-2.5-pro@google-gemini-t2 | vision.segment | provider_restricted |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3-flash-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3-flash-preview@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-flash-lite@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools@google-gemini-t2 | vision.segment | failed |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.1-pro-preview@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash-lite.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash-lite@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.5-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.5-flash@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.6-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.6-flash@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.7-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.7-flash@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-3.8-flash.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-3.8-flash@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-4-argon.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-4-argon@google-gemini-t2 | vision.segment | review |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.detect | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.detect | review |
| t2.google-gemini.google-gemini-t2.gemini-robotics-er-2-preview.vision.segment | T2 | google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview@google-gemini-t2 | vision.segment | review |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-2.5-flash-lite | gemini-2.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro | gemini-2.5-pro | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-flash-preview | gemini-3-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite | gemini-3.1-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools | gemini-3.1-pro-preview-customtools | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview | gemini-3.1-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash-lite | gemini-3.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash | gemini-3.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.6-flash | gemini-3.6-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.7-flash | gemini-3.7-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview | gemini-robotics-er-2-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash-lite | gemini-2.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-flash | gemini-2.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-2.5-pro | gemini-2.5-pro | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3-flash-preview | gemini-3-flash-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-flash-lite | gemini-3.1-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview-customtools | gemini-3.1-pro-preview-customtools | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.1-pro-preview | gemini-3.1-pro-preview | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash-lite | gemini-3.5-flash-lite | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.5-flash | gemini-3.5-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.6-flash | gemini-3.6-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.7-flash | gemini-3.7-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-3.8-flash | gemini-3.8-flash | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-4-argon | gemini-4-argon | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-robotics-er-2-preview | gemini-robotics-er-2-preview | included | physical model | - |

## Confirmed product defects

- **defect.aicc.t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.segment** (AICC, assertion_failed): expected the selected exact model satisfies the case routing, capability, resource, and security assertions; observed RPCError: RPC call error: 500. Evidence: cases/t2.google-gemini.google-gemini-t2.gemini-3.1-pro-preview-customtools.vision.segment.json

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

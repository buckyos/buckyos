# AICC E2E Acceptance Report

- Run: `t15-20261014144406-655193`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `official-provider-protocols-2026-10-14.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=2, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 2/2 (100.00%); passed=2, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.success | T1.5 | google-gemini/t15-20261014144406-655193-google-gemini | gemini-3.5-flash@t15-20261014144406-655193-google-gemini | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.success | T1.5 | google-gemini/t15-20261014144406-655193-google-gemini | gemini-3.5-flash@t15-20261014144406-655193-google-gemini | vision.segment | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

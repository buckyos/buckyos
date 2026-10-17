# AICC E2E Acceptance Report

- Run: `t15-20261016164540-509753`
- Commit: `6d025991be1a359de271a110191c273a17c0fcfe`
- Capability baseline: `official-provider-protocols-2026-10-15.1`
- Real model calls: 0/0
- Planned maximum cost: $0.000000
- Known actual / unknown estimated exposure: $0.000000 / $0.000000
- Total exposure / budget: $0.000000 / $0.000000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=6, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0
- Manifest coverage: 6/6 (100.00%); passed=6, failed=0

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t1.5.google-gemini.gemini.interactions.v1beta.llm.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016164540-509753-google-gemini | gemini-4-argon@t15-20261016164540-509753-google-gemini | llm | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.ocr.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016164540-509753-google-gemini | gemini-4-argon@t15-20261016164540-509753-google-gemini | vision.ocr | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.caption.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016164540-509753-google-gemini | gemini-4-argon@t15-20261016164540-509753-google-gemini | vision.caption | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.detect.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016164540-509753-google-gemini | gemini-4-argon@t15-20261016164540-509753-google-gemini | vision.detect | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.vision.segment.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016164540-509753-google-gemini | gemini-4-argon@t15-20261016164540-509753-google-gemini | vision.segment | passed |
| t1.5.google-gemini.gemini.interactions.v1beta.audio.asr.model.gemini-4-argon.success | T1.5 | google-gemini/t15-20261016164540-509753-google-gemini | gemini-4-argon@t15-20261016164540-509753-google-gemini | audio.asr | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T13-22-38-279Z-4109a0b6`
- Commit: `e742bc1cf5be6b716ef849e624d5b381a495adb6`
- Capability baseline: `2026-10-14.1`
- Real model calls: 2/2
- Planned maximum cost: $0.020000
- Known actual / unknown estimated exposure: $0.000003 / $0.010000
- Total exposure / budget: $0.010003 / $0.050000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=2, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.google-gemini.google-gemini-t2.gemini-embedding-001.embedding.text | T2 | google-gemini/google-gemini-t2 | gemini-embedding-001@google-gemini-t2 | embedding.text | passed |
| t2.google-gemini.google-gemini-t2.gemini-embedding-2.embedding.text | T2 | google-gemini/google-gemini-t2 | gemini-embedding-2@google-gemini-t2 | embedding.text | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| google-gemini/google-gemini-t2 | gemini-embedding-001 | gemini-embedding-001 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2 | gemini-embedding-2 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-001 | gemini-embedding-001 | included | physical model | - |
| google-gemini/google-gemini-t2 | gemini-embedding-2 | gemini-embedding-2 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (google-gemini)

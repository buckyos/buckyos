# AICC E2E Acceptance Report

- Run: `aicc-2026-10-17T14-38-16-038Z-f693e93a`
- Commit: `b06b8044b7bf3d5731e14d855ba33c70a5673e27`
- Capability baseline: `2026-10-14.1`
- Real model calls: 5/5
- Planned maximum cost: $0.050000
- Known actual / unknown estimated exposure: $0.000838 / $0.000000
- Total exposure / budget: $0.000838 / $0.100000
- Unknown-cost calls: 0; budget exceeded: false
- Results: passed=3, failed=0, provider_restricted=0, skipped=0, not_applicable=0, review=0

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config /home/zhangzhen/buckyos/test/aicc_test/aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 900000 --no-real-model-calls
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.claude.claude-t2.claude-haiku-5-5.llm | T2 | claude/claude-t2 | claude-haiku-5-5@claude-t2 | llm | passed |
| t2.claude.claude-t2.claude-haiku-5-5.vision.caption | T2 | claude/claude-t2 | claude-haiku-5-5@claude-t2 | vision.caption | passed |
| t2.claude.claude-t2.claude-haiku-5-5.vision.ocr | T2 | claude/claude-t2 | claude-haiku-5-5@claude-t2 | vision.ocr | passed |

## Physical model coverage

| Provider | Inventory model | Physical model | Decision | Reason | Retained representative |
|---|---|---|---|---|---|
| claude/claude-t2 | claude-haiku-5-5 | claude-haiku-5-5 | included | physical model | - |
| claude/claude-t2 | claude-haiku-5-5 | claude-haiku-5-5 | included | physical model | - |

## Confirmed product defects

None.

## Cleanup

Status: passed

- removed 0 uploaded fixture object(s)
- removed 0 generated output object(s)
- temporary Provider credentials restored after testing (claude)

# AICC E2E Acceptance Report

- Run: `aicc-2026-10-14T16-11-01-145Z-b852b041`
- Commit: `5f803fc558207eae84a289904e3ee3bfd2342e93`
- Capability baseline: `2026-10-14.1`
- Real model calls: 3/3
- Planned maximum cost: $0.030000
- Known actual / unknown estimated exposure: $0.000219 / $0.010000
- Total exposure / budget: $0.010219 / $1.000000
- Unknown-cost calls: 1; budget exceeded: false
- Results: passed=1, failed=1, provider_restricted=0, skipped=0, not_applicable=0, review=1

## Targeted retest

Run after fixing the reported defect; repeat `--case` to select additional cases:

```bash
pnpm run acceptance:gateway -- --config aicc_acceptance.local.toml --allow-credential-mutation --timeout-ms 600000 --provider claude --case t2.claude.claude-t2.claude-haiku-5-5.vision.ocr
```

## Cases

| Case | Layer | Provider | Model | API | Status |
|---|---|---|---|---|---|
| t2.claude.claude-t2.claude-haiku-5-5.llm | T2 | claude/claude-t2 | claude-haiku-5-5@claude-t2 | llm | passed |
| t2.claude.claude-t2.claude-haiku-5-5.vision.caption | T2 | claude/claude-t2 | claude-haiku-5-5@claude-t2 | vision.caption | review |
| t2.claude.claude-t2.claude-haiku-5-5.vision.ocr | T2 | claude/claude-t2 | claude-haiku-5-5@claude-t2 | vision.ocr | failed |

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

# AICC E2E Acceptance Report

- Run: `t15-20261018054009-3100276`
- Commit: `1e8f93081e1a1b9561e77a78e3f7cb67f9922330`
- Capability baseline: `official-provider-protocols-2026-10-18.1`
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
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.success | T1.5 | sn-ai-provider/t15-20261018054009-3100276-sn-ai-provider | qwen3-embedding-8b@t15-20261018054009-3100276-sn-ai-provider | embedding.text | passed |
| t1.5.sn-ai-provider.sn.openai-embeddings.v1.embedding.text.error.invalid_argument | T1.5 | sn-ai-provider/t15-20261018054009-3100276-sn-ai-provider | qwen3-embedding-8b@t15-20261018054009-3100276-sn-ai-provider | embedding.text | passed |

## Confirmed product defects

None.

## Cleanup

Status: passed

- temporary Provider instances were removed and the Mock selection was reset

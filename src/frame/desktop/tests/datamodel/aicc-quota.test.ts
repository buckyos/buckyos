import {
  filterRules,
  formatTokens,
  fromGroupDraft,
  modelQuotaStatus,
  parseTokens,
  quotaRefusalOf,
  quotaWindow,
  sortSubjects,
  toGroupDraft,
  usageRatio,
  type QuotaRuleView,
  type QuotaUsageState,
  type VisibilityEntry,
} from '../../src/app/ai-center/datamodel/quota.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

const HOUR = 3_600_000
const DAY = 24 * HOUR

function state(overrides: Partial<QuotaUsageState> = {}): QuotaUsageState {
  return {
    status: 'normal', settled_tokens: 0, reserved_tokens: 0, pending_tokens: 0,
    limit_tokens: 1000, available_tokens: 1000, debt_tokens: 0, reset_at_ms: null, updated_at_ms: 0,
    ...overrides,
  }
}

function view(rule: Partial<QuotaRuleView['rule']>, usage: Partial<QuotaUsageState> = {}): QuotaRuleView {
  return {
    rule: {
      rule_id: 'qr1', source: 'admin', object: { type: 'user', user_id: 'bob' }, resource: { type: 'all' },
      kind: 'periodic', period: '24h', limit_tokens: 1000, enabled: true, version: 1,
      created_by: 'root', created_at_ms: 0, updated_by: 'root', updated_at_ms: 0,
      ...rule,
    } as QuotaRuleView['rule'],
    state: state(usage),
    editable: false,
  }
}

Deno.test('quota windows follow the fixed UTC boundaries of the service', () => {
  const at = Date.UTC(2026, 9, 22, 13, 30)
  const four = quotaWindow('4h', at)
  assert(four.startMs === Date.UTC(2026, 9, 22, 12) && four.endMs === Date.UTC(2026, 9, 22, 16), '4h windows start at multiples of 4 UTC hours')
  const day = quotaWindow('24h', at)
  assert(day.startMs === Date.UTC(2026, 9, 22) && day.endMs - day.startMs === DAY, '24h windows start at UTC midnight')
  const week = quotaWindow('week', at)
  assert(new Date(week.startMs).getUTCDay() === 1 && week.startMs <= at && at < week.endMs, 'weeks start on Monday 00:00 UTC')
  const base = Date.UTC(1970, 0, 5)
  const threeDays = quotaWindow('72h', at)
  assert((threeDays.startMs - base) % (3 * DAY) === 0 && threeDays.endMs - threeDays.startMs === 3 * DAY, '72h windows count from 1970-01-05')
  const before = quotaWindow('72h', base - 1)
  assert(before.startMs === base - 3 * DAY && before.endMs === base, 'windows before the base use floor division')
})

Deno.test('token amounts accept K/M shorthand and reject anything else', () => {
  assert(parseTokens('50K') === 50_000, '50K')
  assert(parseTokens('1.5m') === 1_500_000, '1.5m')
  assert(parseTokens('2,000,000') === 2_000_000, 'thousands separators')
  assert(parseTokens('0') === 0, 'zero is a valid limit (forbidden)')
  for (const bad of ['', 'abc', '-5', '1e3', '5KK']) assert(parseTokens(bad) === null, `rejects ${JSON.stringify(bad)}`)
  assert(formatTokens(172_000) === '172K' && formatTokens(1_500_000) === '1.5M' && formatTokens(999) === '999', 'abbreviation')
})

Deno.test('usage ratio counts reserved tokens and treats a zero limit as exhausted', () => {
  assert(usageRatio(state({ settled_tokens: 600, reserved_tokens: 200 })) === 0.8, 'settled + reserved over the limit')
  assert(usageRatio(state({ limit_tokens: 0, status: 'forbidden' })) === 1, 'zero limit reads as full')
  assert(usageRatio(state({ limit_tokens: undefined, granted_tokens: 100, settled_tokens: 150 })) === 1.5, 'a debt exceeds 1')
})

Deno.test('rule filters combine kind, source, state and search', () => {
  const rules = [
    view({ rule_id: 'a', object: { type: 'user', user_id: 'bob' } }, { status: 'near_limit' }),
    view({ rule_id: 'b', kind: 'total', period: undefined, object: { type: 'app', app_id: 'aiworkspace' } }),
    view({ rule_id: 'c', source: 'user', enabled: false, resource: { type: 'model', exact_model: 'gpt-5@openai' } }),
  ]
  const base = { query: '', kind: 'all', source: 'all', status: 'all' } as const
  assert(filterRules(rules, { ...base, kind: 'total' }).map((r) => r.rule.rule_id).join() === 'b', 'kind')
  assert(filterRules(rules, { ...base, status: 'limited' }).map((r) => r.rule.rule_id).join() === 'a', 'near or at the limit')
  assert(filterRules(rules, { ...base, status: 'disabled' }).map((r) => r.rule.rule_id).join() === 'c', 'disabled')
  assert(filterRules(rules, { ...base, query: 'AIWORK' }).map((r) => r.rule.rule_id).join() === 'b', 'search is case-insensitive over the App')
  assert(filterRules(rules, { ...base, query: 'gpt-5' }).map((r) => r.rule.rule_id).join() === 'c', 'search covers exact models')
})

Deno.test('model marks come from enabled user-wide limits covering the model', () => {
  const rules = [
    view({ rule_id: 'p', resource: { type: 'provider', provider_instance_name: 'openai' } }, { status: 'near_limit' }),
    view({ rule_id: 'm', resource: { type: 'model', exact_model: 'claude@anthropic' } }, { status: 'exhausted' }),
    view({ rule_id: 'app', object: { type: 'user_app', user_id: 'bob', app_id: 'x' } }, { status: 'forbidden' }),
    view({ rule_id: 'off', enabled: false }, { status: 'exhausted' }),
  ]
  assert(modelQuotaStatus(rules, ['gpt-5@openai']) === 'near_limit', 'provider rule covers its models')
  assert(modelQuotaStatus(rules, ['claude@anthropic', 'gpt-5@openai']) === 'exhausted', 'the worst state wins')
  assert(modelQuotaStatus(rules, ['qwen@dashscope']) === undefined, 'App-specific and disabled rules do not mark models')
})

Deno.test('group visibility drafts round-trip the stored allow items', () => {
  const entries: VisibilityEntry[] = [
    { type: 'provider_all_models', provider_instance_name: 'openai' },
    { type: 'provider_models', provider_instance_name: 'anthropic', exact_models: ['b@anthropic', 'a@anthropic'] },
  ]
  const draft = toGroupDraft(entries)
  assert(draft.providers.openai.mode === 'all_models' && draft.providers.anthropic.mode === 'models', 'per-Provider choices')
  const back = fromGroupDraft({ ...draft, providers: { ...draft.providers, closed: { mode: 'closed' } } })
  assert(JSON.stringify(back) === JSON.stringify([
    { type: 'provider_models', provider_instance_name: 'anthropic', exact_models: ['a@anthropic', 'b@anthropic'] },
    { type: 'provider_all_models', provider_instance_name: 'openai' },
  ]), 'closed Providers produce nothing; output is sorted')
  assert(JSON.stringify(fromGroupDraft(toGroupDraft([{ type: 'all_providers' }]))) === '[{"type":"all_providers"}]', 'all Providers')
  assert(sortSubjects(['group:b', 'type:limited', 'group:a', 'type:admin', 'type:user']).join() === 'type:admin,type:user,type:limited,group:a,group:b', 'UserType groups first')
})

Deno.test('refusals are recognised by their wire code, never their text', () => {
  const thrown = { message: 'anything', code: 'quota_exhausted', data: { code: 'quota_exhausted', details: { reset_at_ms: 5 } } }
  assert(quotaRefusalOf(thrown)?.details?.reset_at_ms === 5, 'thrown kRPC error carries details in data')
  const failedTask = { code: 'quota_budget_exceeded', message: 'x', details: { max_allowed_tokens: 10 } }
  assert(quotaRefusalOf(failedTask)?.details?.max_allowed_tokens === 10, 'a failed task carries details directly')
  assert(quotaRefusalOf({ message: 'quota_exhausted' }) === undefined, 'text never decides')
  assert(quotaRefusalOf({ code: 'provider_unavailable' }) === undefined, 'other codes are not refusals')
})

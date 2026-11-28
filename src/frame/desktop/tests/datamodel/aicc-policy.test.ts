/** AI Center user policy and AI Credit helpers (src/app/ai-center/datamodel/policy.ts) and the mock policy world
 * the e2e tests run against (src/app/ai-center/mock/policy.ts).
 *   deno test --no-check -A --config tests/datamodel/messagehub.deno.json tests/datamodel/aicc-policy.test.ts */
import {
  addCredit,
  creditMicro,
  formatCredit,
  formatCreditMicro,
  formatTokens,
  limitsValid,
  modelLimitStatus,
  normalizeBody,
  parseLimitAmount,
  parsePositiveCredit,
  parseSignedCredit,
  parseSubject,
  policyRefusalOf,
  isUnpricedRefusal,
  priceForm,
  priceSummary,
  pricingOfForm,
  restrictBody,
  sameBody,
  sortSubjects,
  startsCharging,
  type EffectiveLimit,
} from '../../src/app/ai-center/datamodel/policy.ts'
import { MockPolicyWorld, type MockAccount, type MockInventoryModel } from '../../src/app/ai-center/mock/policy.ts'

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

Deno.test('Credit amounts are exact decimals with six places, shown with two', () => {
  assert(creditMicro('12.5') === 12_500_000n && creditMicro('-2.5') === -2_500_000n && creditMicro('.5') === 500_000n, 'parse like the service')
  for (const bad of ['', '-', '1.', '1.0000001', '1e3', '1,5', 'abc']) assert(creditMicro(bad) === null, `rejects ${JSON.stringify(bad)}`)
  assert(formatCreditMicro(12_500_000n) === '12.5' && formatCreditMicro(-1n) === '-0.000001' && formatCreditMicro(0n) === '0', 'wire form')
  assert(addCredit('0.1', '0.2') === '0.3' && addCredit('3.25', '-5.75') === '-2.5', 'exact sums (no float drift)')
  assert(formatCredit('12.5') === '12.50' && formatCredit('-2.5') === '-2.50', 'two decimals, negative kept')
  assert(formatCredit('0.004') === '< 0.01' && formatCredit('-0.004') === '> -0.01' && formatCredit('0') === '0.00', 'tiny amounts never read 0.00')
  assert(parsePositiveCredit('1,000.5') === '1000.5' && parsePositiveCredit('0') === null && parsePositiveCredit('-1') === null, 'a recharge is positive')
  assert(parseSignedCredit('-2.5') === '-2.5' && parseSignedCredit('0') === null, 'an adjustment is signed and not zero')
  assert(formatTokens(172_000) === '172K' && formatTokens(1_500_000) === '1.5M', 'token abbreviations')
})

Deno.test('limits: amounts in their unit, one per window and unit', () => {
  assert(parseLimitAmount('credit', '10.5') === '10.5' && parseLimitAmount('credit', '0') === '0', 'Credit decimals; 0 forbids use')
  assert(parseLimitAmount('token', '200K') === '200000' && parseLimitAmount('token', '1.5') === '2', 'tokens are whole numbers, K/M accepted')
  assert(parseLimitAmount('credit', '-1') === null && parseLimitAmount('token', 'x') === null, 'invalid amounts')
  assert(limitsValid([{ period: '5h', unit: 'credit', amount: '10' }, { period: '5h', unit: 'token', amount: '100' }, { period: '7d', unit: 'credit', amount: '50' }]), 'distinct window and unit')
  assert(!limitsValid([{ period: '24h', unit: 'credit', amount: '1' }, { period: '24h', unit: 'credit', amount: '2' }]), 'duplicate window and unit')
  assert(!limitsValid([{ period: '24h', unit: 'credit', amount: '1.0000001' }]), 'more than six decimals')
})

Deno.test('subjects parse and sort user side layers before App side ones', () => {
  assert(parseSubject('user_app:bob/writer')?.appId === 'writer' && parseSubject('user_app:bob/writer')?.userId === 'bob', 'user + App')
  for (const bad of ['type:root', 'group:', 'user:a/b', 'user_app:bob', 'team:x']) assert(parseSubject(bad) === null, `rejects ${bad}`)
  assert(sortSubjects(['app:x', 'user:bob', 'group:b', 'type:limited', 'user_app:bob/x', 'group:a', 'type:admin', 'type:user']).join() ===
    'type:admin,type:user,type:limited,group:a,group:b,user:bob,app:x,user_app:bob/x', 'types, groups, users, Apps, users’ Apps')
})

Deno.test('policy bodies: no-opinion entries drop out, App side drafts lose what they may not say', () => {
  const body = { all_providers: false, providers: { a: {}, b: { visible: 'hidden' as const, limits: [] } }, models: { 'm@a': { limits: [] } }, limits: [] }
  assert(JSON.stringify(normalizeBody(body)) === '{"providers":{"b":{"visible":"hidden"}}}', 'empty entries, false and empty lists are no opinion')
  assert(sameBody({ providers: { x: { price_coefficient: 0 } } }, { providers: { x: { price_coefficient: 0, limits: [] } }, models: {} }), 'same meaning compares equal')
  const own = restrictBody({ all_providers: true, price_coefficient: 1, providers: { p: { visible: 'all_models', price_coefficient: 2 }, q: { visible: 'selected_models' } }, models: { 'm@q': { visible: true }, 'n@p': { visible: true } } }, 'own_app')
  assert(JSON.stringify(own) === '{"providers":{"q":{"visible":"selected_models"}},"models":{"m@q":{"visible":true}}}', 'only narrowing visibility and limits remain')
})

Deno.test('a free layer or Provider that becomes charged is detected (unpriced models stop working)', () => {
  assert(startsCharging({ all_providers: true, price_coefficient: 0 }, { all_providers: true, price_coefficient: 1 }), 'layer default 0 → 1')
  assert(startsCharging({ providers: { local: { price_coefficient: 0 } } }, { providers: { local: { price_coefficient: 0.5 } } }), 'Provider 0 → 0.5')
  assert(!startsCharging({ price_coefficient: 1 }, { price_coefficient: 2 }), 'already charging')
  assert(!startsCharging(undefined, { price_coefficient: 1 }), 'a new policy has nothing to compare with')
})

Deno.test('prices: simple forms round-trip, anything richer is edited as JSON', () => {
  const tokens = priceForm({ currency: 'CREDIT', input_token: 10, output_token: 40, rules: [] })
  assert(tokens?.mode === 'tokens' && tokens.input === '10' && tokens.output === '40', 'token rates per million')
  assert(JSON.stringify(pricingOfForm({ mode: 'tokens', input: '1.5', output: '', cacheInput: '' })) === '{"currency":"CREDIT","input_token":1.5}', 'empty rates are left out')
  assert(pricingOfForm({ mode: 'tokens', input: '', output: '', cacheInput: '' }) === null && pricingOfForm({ mode: 'tokens', input: 'x', output: '1', cacheInput: '' }) === null, 'needs a valid rate')
  assert(JSON.stringify(pricingOfForm({ mode: 'unit', unit: 'image', amount: '0.5' })) === '{"currency":"CREDIT","unit":"image","amount":0.5}', 'per unit')
  assert(priceForm({ currency: 'CREDIT', tiers: { steps: [] } }) === null && priceForm({ currency: 'CREDIT', unit: 'image', amount: 1, addons: [{ unit: 'second', amount: 1 }] }) === null, 'tiers and add-ons are JSON')
  const summary = priceSummary({ state: 'priced', source: 'model', pricing: { currency: 'CREDIT', input_token: 10, output_token: 40 }, provider_coefficient: 1.5, model_coefficient: 0.5 })
  assert(summary.input === 7.5 && summary.output === 30, 'coefficients multiply the listed price')
})

Deno.test('limit marks and refusals come from structured fields, never text', () => {
  const limit = (resource: EffectiveLimit['resource'], status: EffectiveLimit['status']): EffectiveLimit => ({
    policy_id: 'p', source: 'admin', subject: 'user:bob', counter: { type: 'user', user_id: 'bob' }, resource, period: '24h', unit: 'credit', amount: '10', used: '9', status,
  })
  const limits = [limit({ type: 'provider', provider_instance_name: 'openai' }, 'near_limit'), limit({ type: 'model', exact_model: 'c@anthropic' }, 'exhausted'), limit({ type: 'all' }, 'normal')]
  assert(modelLimitStatus(limits, 'gpt@openai') === 'near_limit' && modelLimitStatus(limits, 'c@anthropic') === 'exhausted' && modelLimitStatus(limits, 'q@local') === undefined, 'worst covering limit')
  assert(policyRefusalOf({ code: 'credit_insufficient', data: { details: { balance: '-2.5', provider_instance_name: 'p', exact_model: 'm@p' } } })?.code === 'credit_insufficient', 'thrown kRPC error')
  assert(isUnpricedRefusal(policyRefusalOf({ code: 'resource_not_permitted', details: { reason: 'unpriced' } })), 'unpriced is a reason of resource_not_permitted')
  assert(policyRefusalOf({ message: 'credit_insufficient' }) === undefined && policyRefusalOf({ code: 'quota_budget_exceeded' }) === undefined, 'text and removed codes are not refusals')
})

function world(viewer: MockAccount) {
  const inventory: MockInventoryModel[] = [
    { exact_model: 'gpt@openai', provider_instance_name: 'openai', provider_model_id: 'gpt', model_driver_id: 'openai', origin_model_id: 'gpt', token_metered: true },
    { exact_model: 'gpt@router', provider_instance_name: 'router', provider_model_id: 'gpt', model_driver_id: 'openai', origin_model_id: 'gpt', token_metered: true },
    { exact_model: 'mini@openai', provider_instance_name: 'openai', provider_model_id: 'mini', model_driver_id: 'openai', origin_model_id: 'mini', token_metered: true },
    { exact_model: 'raw@router', provider_instance_name: 'router', provider_model_id: 'raw', model_driver_id: '', origin_model_id: '', token_metered: true, provider_cost_currency: 'USD' },
  ]
  const accounts: MockAccount[] = [
    { user_id: 'alice', user_type: 'admin', groups: [] },
    { user_id: 'bob', user_type: 'user', groups: ['students'] },
    { user_id: 'carol', user_type: 'user', groups: ['students', 'staff'] },
  ]
  let current = viewer
  const mock = new MockPolicyWorld(() => inventory, () => current, () => accounts, () => ['students', 'staff'], () => ['aiworkspace', 'writer'])
  return { mock, as: (user: MockAccount) => { current = user } }
}

Deno.test('mock world: layers override, groups favour the user, App side only narrows', () => {
  const alice: MockAccount = { user_id: 'alice', user_type: 'admin', groups: [] }
  const { mock } = world(alice)
  mock.listPolicies('manage')
  const view = (subject: string) => mock.listPolicies('manage').find((item) => item.policy.subject === subject && item.policy.source === 'admin')
  mock.upsertPolicy({ source: 'admin', subject: 'type:user', body: { all_providers: true, price_coefficient: 1 } }, view('type:user')?.policy.version)
  mock.upsertPolicy({ source: 'admin', subject: 'group:students', body: { providers: { router: { visible: 'hidden', price_coefficient: 0.8 } } } })
  mock.upsertPolicy({ source: 'admin', subject: 'group:staff', body: { providers: { router: { visible: 'all_models', price_coefficient: 1.2 } } } })
  const bobView = mock.effective({ user_id: 'bob' })
  assert(!bobView.models.some((model) => model.provider_instance_name === 'router'), 'a group hides a Provider the user type opens (C02)')
  assert(bobView.models.find((model) => model.exact_model === 'gpt@openai')?.visible_from.join() === 'type:user', 'visibility source')
  const carolView = mock.effective({ user_id: 'carol' })
  const routed = carolView.models.find((model) => model.exact_model === 'gpt@router')
  assert(routed && routed.price.provider_coefficient === 0.8, 'among groups the visible one wins and the smaller coefficient (C03)')
  mock.upsertPolicy({ source: 'admin', subject: 'app:writer', body: { providers: { openai: { visible: 'hidden' } } } })
  const through = mock.effective({ user_id: 'carol', app_id: 'writer' })
  assert(!through.models.some((model) => model.provider_instance_name === 'openai') && through.models.some((model) => model.provider_instance_name === 'router'), 'an App policy narrows calls through it (C05)')
  assert(mock.effective({ user_id: 'carol' }).models.some((model) => model.provider_instance_name === 'openai'), 'direct calls are not narrowed')
})

Deno.test('mock world: prices by identity, exact exceptions, unpriced models and explicit free', () => {
  const alice: MockAccount = { user_id: 'alice', user_type: 'admin', groups: [] }
  const { mock } = world(alice)
  const prices = mock.getPrices()
  mock.updatePrices({ expected_revision: prices.revision, upserts: [{ entry_key: 'model:openai:gpt', pricing: { input_token: 10, output_token: 40 } }] })
  const policies = mock.listPolicies('manage')
  const user = policies.find((item) => item.policy.subject === 'type:user')!
  mock.upsertPolicy({ source: 'admin', subject: 'type:user', body: { all_providers: true, price_coefficient: 1, providers: { router: { price_coefficient: 0 } } } }, user.policy.version)
  const bob = mock.effective({ user_id: 'bob' })
  const state = (exact: string) => bob.models.find((model) => model.exact_model === exact)?.price.state
  assert(state('gpt@openai') === 'priced' && state('mini@openai') === 'unpriced', 'the identity price covers its exact models; others are unpriced (C11, C25)')
  assert(state('gpt@router') === 'free' && state('raw@router') === 'free', 'a Provider coefficient 0 is free')
  assert(bob.unpriced_models?.join() === 'mini@openai', 'unpriced visible models are listed')
  assert(mock.effective({}).models.every((model) => model.price.state === 'free'), 'administrators stay free by default (Q17)')
  const conflict = (() => { try { mock.updatePrices({ expected_revision: prices.revision, deletes: ['model:openai:gpt'] }); return false } catch (error) { return (error as { code?: string }).code === 'settings_revision_conflict' } })()
  assert(conflict, 'a stale revision is refused')
  const draft = mock.effective({ user_id: 'bob', draft: { source: 'admin', subject: 'type:user', body: { all_providers: true, price_coefficient: 1 } } })
  assert(draft.removed?.includes('gpt@router') === false && draft.unpriced_models?.includes('raw@router') === true, 'a draft shows what becomes unpriced')
})

Deno.test('mock world: recharges are idempotent and balances may go negative', () => {
  const alice: MockAccount = { user_id: 'alice', user_type: 'admin', groups: [] }
  const { mock, as } = world(alice)
  const first = mock.recharge({ user_id: 'carol', recharge_id: 'rc-1', amount: '10' })
  const again = mock.recharge({ user_id: 'carol', recharge_id: 'rc-1', amount: '10' })
  assert(!first.replayed && again.replayed && again.account.balance === '10', 'the same id credits once (C17)')
  const conflict = (() => { try { mock.recharge({ user_id: 'carol', recharge_id: 'rc-1', amount: '11' }); return false } catch (error) { return (error as { code?: string }).code === 'idempotency_conflict' } })()
  assert(conflict, 'the same id with other parameters is refused')
  assert(mock.adjust({ user_id: 'carol', adjust_id: 'adj-1', amount: '-12.5', note: 'refund' }).account.balance === '-2.5', 'an adjustment may go below zero')
  assert(mock.ledger({ user_id: 'carol' }).items[0].type === 'entry', 'the ledger lists entries, newest first')
  as({ user_id: 'bob', user_type: 'user', groups: ['students'] })
  const denied = (() => { try { mock.recharge({ user_id: 'bob', recharge_id: 'rc-2', amount: '5' }); return false } catch (error) { return (error as { code?: string }).code === 'policy_denied' } })()
  assert(denied, 'a user cannot recharge (C18)')
  const own = (() => { try { mock.upsertPolicy({ subject: 'user_app:bob/writer', body: { price_coefficient: 0 } }); return false } catch { return true } })()
  assert(own, 'a user’s own App policy carries no coefficient (C06)')
  assert(mock.upsertPolicy({ subject: 'user_app:bob/writer', body: { limits: [{ period: '5h', unit: 'credit', amount: '10' }] } }).policy.source === 'user', 'a user limits their own App')
})

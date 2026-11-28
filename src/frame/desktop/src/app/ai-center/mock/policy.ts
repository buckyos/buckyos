// In-memory user policies, price list and Credit accounts of the mock runtime. It follows the service's
// evaluation (frame/aicc/src/policy) where the UI can observe it: user side layers where the more specific
// one decides (the group layer picks what favours the user), App side policies that only narrow, prices
// from `exact:` then `model:` entries, explicit free (zero price or coefficient), unpriced models that a
// paying user cannot use, limits with their window state, idempotent recharges and adjustments, versions
// and conflicts. Consumption and balances are seeded, since nothing calls models here.
import type {
  AiccPolicy,
  AiccPolicyView,
  CreditAccount,
  CreditCharge,
  CreditEntry,
  CreditEntryResponse,
  CreditLedgerItem,
  CreditLedgerListResponse,
  EffectiveLimit,
  EffectiveModel,
  LimitCounter,
  LimitPeriod,
  LimitResource,
  LimitStatus,
  LimitUnit,
  ModelCreditPrice,
  PolicyAuditRecord,
  PolicyBody,
  PolicyDraft,
  PolicyEffectiveResponse,
  PolicyGetResponse,
  PolicyLimit,
  PolicySource,
  PolicyStatusResponse,
  PriceEntry,
  PriceGetResponse,
  PriceUpdateRequest,
} from '../datamodel/policy'
import { creditMicro, formatCreditMicro, isAppSide, isZeroPricing, parseLimitAmount, parsePriceKey, parseSubject } from '../datamodel/policy'

export interface MockInventoryModel {
  exact_model: string
  provider_instance_name: string
  provider_model_id: string
  variant?: string
  /** Model Driver identity; empty when the model is not mapped to one. */
  model_driver_id: string
  origin_model_id: string
  token_metered: boolean
  /** The Provider's own cost currency, when the inventory knows its price. */
  provider_cost_currency?: string
}

export interface MockAccount { user_id: string; user_type: 'admin' | 'user' | 'limited'; groups: string[] }

export class MockPolicyError extends Error {
  readonly code: string

  constructor(message: string, code = 'invalid_request') {
    super(message)
    this.code = code
  }
}

const HOUR = 3_600_000
const PERIOD_MS: Record<LimitPeriod, number> = { '5h': 5 * HOUR, '24h': 24 * HOUR, '7d': 7 * 24 * HOUR }
const NEAR_LIMIT_PERCENT = 80n

interface Counter { start: number; end: number; credit: bigint; tokens: number }

interface AppliedLimit {
  policy: AiccPolicy
  counter: LimitCounter
  resource: LimitResource
  limit: PolicyLimit
  amount: bigint
}

interface Decision { visible: boolean; visibleFrom: string[]; providerCoefficient: number; modelCoefficient: number; coefficientFrom: string[] }

function layerVisibility(body: PolicyBody, provider: string, exactModel?: string): boolean | undefined {
  const model = exactModel ? body.models?.[exactModel]?.visible : undefined
  if (model !== undefined) return model
  const visibility = body.providers?.[provider]?.visible
  if (visibility) return visibility === 'all_models' ? true : visibility === 'hidden' ? false : exactModel === undefined
  return body.all_providers === true ? true : undefined
}

function counterKey(counter: LimitCounter): string {
  switch (counter.type) {
    case 'user': return `user:${counter.user_id}`
    case 'app': return `app:${counter.app_id}`
    case 'user_app': return `user_app:${counter.user_id}/${counter.app_id}`
  }
}

function resourceKey(resource: LimitResource): string {
  switch (resource.type) {
    case 'all': return 'all'
    case 'provider': return `provider:${resource.provider_instance_name}`
    case 'model': return `model:${resource.exact_model}`
    case 'hidden': return 'hidden'
  }
}

function scopeKey(counter: LimitCounter, resource: LimitResource, period: LimitPeriod): string {
  return `${counterKey(counter)}|${resourceKey(resource)}|${period}`
}

function limitValue(limit: PolicyLimit): bigint | null {
  const text = parseLimitAmount(limit.unit, limit.amount)
  if (text === null) return null
  return limit.unit === 'token' ? BigInt(text) : creditMicro(text)
}

function formatUsed(unit: LimitUnit, used: bigint): string {
  return unit === 'token' ? used.toString() : formatCreditMicro(used)
}

function limitStatus(amount: bigint, used: bigint): LimitStatus {
  if (amount === 0n) return 'forbidden'
  if (used >= amount) return 'exhausted'
  return used * 100n >= amount * NEAR_LIMIT_PERCENT ? 'near_limit' : 'normal'
}

/** The policies that apply to one user (through one App), by layer. */
class Evaluator {
  private readonly layers: AiccPolicy[][]
  private readonly appSide: AiccPolicy[]
  private readonly account: MockAccount
  private readonly appId?: string

  constructor(policies: AiccPolicy[], account: MockAccount, appId?: string) {
    this.account = account
    this.appId = appId || undefined
    this.layers = [[], [], []]
    this.appSide = []
    for (const policy of policies.filter((item) => item.enabled)) {
      const subject = policy.subject
      if (subject === `type:${account.user_type}`) this.layers[0].push(policy)
      else if (subject.startsWith('group:') && account.groups.includes(subject.slice(6))) this.layers[1].push(policy)
      else if (subject === `user:${account.user_id}`) this.layers[2].push(policy)
      else if (this.appId && subject === `app:${this.appId}`) this.appSide.push(policy)
      else if (this.appId && subject === `user_app:${account.user_id}/${this.appId}`) this.appSide.push(policy)
    }
    const rank = (policy: AiccPolicy) => policy.subject.startsWith('app:') ? 0 : policy.source === 'admin' ? 1 : 2
    this.appSide.sort((a, b) => rank(a) - rank(b))
  }

  private userVisibility(provider: string, exactModel?: string): [boolean, string[]] {
    for (const layer of [...this.layers].reverse()) {
      const opinions = layer.flatMap((policy) => {
        const visible = layerVisibility(policy.body, provider, exactModel)
        return visible === undefined ? [] : [[visible, policy.subject] as const]
      })
      if (opinions.length === 0) continue
      const allowing = opinions.filter(([visible]) => visible).map(([, subject]) => subject)
      return allowing.length ? [true, allowing] : [false, opinions.map(([, subject]) => subject)]
    }
    return [false, []]
  }

  private appHides(provider: string, exactModel?: string): string | undefined {
    return this.appSide.find((policy) => layerVisibility(policy.body, provider, exactModel) === false)?.subject
  }

  providerVisible(provider: string): boolean {
    return this.userVisibility(provider)[0] && this.appHides(provider) === undefined
  }

  private coefficient(value: (body: PolicyBody) => number | undefined): [number, string | undefined] {
    for (const layer of [...this.layers].reverse()) {
      let best: [number, string] | undefined
      for (const policy of layer) {
        const coefficient = value(policy.body)
        if (coefficient !== undefined && (!best || coefficient < best[0])) best = [coefficient, policy.subject]
      }
      if (best) return best
    }
    return [1, undefined]
  }

  model(provider: string, exactModel: string): Decision {
    let [visible, visibleFrom] = this.userVisibility(provider, exactModel)
    if (visible) {
      const hiding = this.appHides(provider, exactModel)
      if (hiding) { visible = false; visibleFrom = [hiding] }
    }
    const [providerCoefficient, providerFrom] = this.coefficient((body) => body.providers?.[provider]?.price_coefficient ?? body.price_coefficient)
    const [modelCoefficient, modelFrom] = this.coefficient((body) => body.models?.[exactModel]?.price_coefficient)
    return { visible, visibleFrom, providerCoefficient, modelCoefficient, coefficientFrom: [providerFrom, modelFrom].filter((item): item is string => !!item) }
  }

  private static placed(body: PolicyBody, provider: string, exactModel: string): [LimitResource, PolicyLimit][] {
    return [
      ...(body.limits ?? []).map((limit) => [{ type: 'all' }, limit] as [LimitResource, PolicyLimit]),
      ...(body.providers?.[provider]?.limits ?? []).map((limit) => [{ type: 'provider', provider_instance_name: provider }, limit] as [LimitResource, PolicyLimit]),
      ...(body.models?.[exactModel]?.limits ?? []).map((limit) => [{ type: 'model', exact_model: exactModel }, limit] as [LimitResource, PolicyLimit]),
    ]
  }

  limits(provider: string, exactModel: string): AppliedLimit[] {
    const userCounter: LimitCounter = { type: 'user', user_id: this.account.user_id }
    const chosen = new Map<string, AppliedLimit>()
    for (const layer of this.layers) {
      const best = new Map<string, AppliedLimit>()
      for (const policy of layer) {
        for (const [resource, limit] of Evaluator.placed(policy.body, provider, exactModel)) {
          const amount = limitValue(limit)
          if (amount === null) continue
          const key = `${resourceKey(resource)}|${limit.period}|${limit.unit}`
          const current = best.get(key)
          if (current && current.amount >= amount) continue
          best.set(key, { policy, counter: userCounter, resource, limit, amount })
        }
      }
      best.forEach((value, key) => chosen.set(key, value))
    }
    const limits = [...chosen.values()]
    for (const policy of this.appSide) {
      const counter: LimitCounter = policy.subject.startsWith('app:')
        ? { type: 'app', app_id: this.appId ?? '' }
        : { type: 'user_app', user_id: this.account.user_id, app_id: this.appId ?? '' }
      for (const [resource, limit] of Evaluator.placed(policy.body, provider, exactModel)) {
        const amount = limitValue(limit)
        if (amount !== null) limits.push({ policy, counter, resource, limit, amount })
      }
    }
    return limits
  }

  allLimits(inventory: MockInventoryModel[]): AppliedLimit[] {
    const seen = new Set<string>()
    const out: AppliedLimit[] = []
    const push = (limit: AppliedLimit) => {
      const key = `${limit.policy.policy_id}|${counterKey(limit.counter)}|${resourceKey(limit.resource)}|${limit.limit.period}|${limit.limit.unit}`
      if (seen.has(key)) return
      seen.add(key)
      out.push(limit)
    }
    for (const model of inventory) this.limits(model.provider_instance_name, model.exact_model).forEach(push)
    this.limits('', '').forEach(push)
    for (const policy of [...this.layers.flat(), ...this.appSide]) {
      for (const provider of Object.keys(policy.body.providers ?? {})) this.limits(provider, '').forEach(push)
      for (const model of Object.keys(policy.body.models ?? {})) this.limits(model.slice(model.lastIndexOf('@') + 1), model).forEach(push)
    }
    return out
  }
}

export class MockPolicyWorld {
  private policies: AiccPolicy[] = []
  private audit: PolicyAuditRecord[] = []
  private priceRevision = 1
  private prices = new Map<string, PriceEntry>()
  private creditRates: Record<string, number> = {}
  private balances = new Map<string, { micro: bigint; updated: number }>()
  private entries: CreditEntry[] = []
  private charges: CreditCharge[] = []
  private counters = new Map<string, Counter>()
  private sequence = 0
  private seeded = false

  private readonly inventory: () => MockInventoryModel[]
  private readonly viewer: () => MockAccount
  private readonly accounts: () => MockAccount[]
  private readonly groupIds: () => string[]
  private readonly appsOf: (userId: string) => string[]

  constructor(
    inventory: () => MockInventoryModel[],
    viewer: () => MockAccount,
    accounts: () => MockAccount[],
    groupIds: () => string[],
    appsOf: (userId: string) => string[],
  ) {
    this.inventory = inventory
    this.viewer = viewer
    this.accounts = accounts
    this.groupIds = groupIds
    this.appsOf = appsOf
  }

  private id(prefix: string): string {
    this.sequence += 1
    return `${prefix}${this.sequence.toString(16).padStart(6, '0')}`
  }

  private manages(): boolean {
    return this.viewer().user_type === 'admin'
  }

  private requireAdmin() {
    if (!this.manages()) throw new MockPolicyError('this operation requires an administrator', 'policy_denied')
  }

  private seed() {
    if (this.seeded) return
    this.seeded = true
    const now = Date.now()
    const add = (source: PolicySource, subject: string, body: PolicyBody, by = 'system') => {
      const policy: AiccPolicy = { policy_id: this.id('pol_'), source, subject, enabled: true, body, version: 1, created_by: by, created_at_ms: now - 30 * 24 * HOUR, updated_by: by, updated_at_ms: now - 2 * 24 * HOUR }
      this.policies.push(policy)
      return policy
    }
    const sn = this.inventory().find((model) => model.provider_instance_name.startsWith('sn-'))?.provider_instance_name
    add('admin', 'type:admin', { all_providers: true, price_coefficient: 0 })
    add('admin', 'type:user', { all_providers: true, price_coefficient: 1, ...(sn ? { providers: { [sn]: { price_coefficient: 0 } } } : {}) }, 'alice')
    add('admin', 'user:bob', { limits: [{ period: '24h', unit: 'credit', amount: '20' }] }, 'alice')
    add('admin', 'app:aiworkspace', { limits: [{ period: '7d', unit: 'credit', amount: '500' }] }, 'alice')
    add('user', 'user_app:alice/aiworkspace', { limits: [{ period: '5h', unit: 'credit', amount: '5' }] }, 'alice')
    const window = (period: LimitPeriod, startAgo: number) => ({ start: now - startAgo, end: now - startAgo + PERIOD_MS[period] })
    this.counters.set('user:bob|all|24h', { ...window('24h', 6 * HOUR), credit: 17_000_000n, tokens: 410_000 })
    this.counters.set('app:aiworkspace|all|7d', { ...window('7d', 50 * HOUR), credit: 120_000_000n, tokens: 2_900_000 })
    this.counters.set('user_app:alice/aiworkspace|all|5h', { ...window('5h', 3 * HOUR), credit: 5_200_000n, tokens: 140_000 })

    const price = (key: string, pricing: Record<string, unknown>, note?: string) => this.prices.set(key, { entry_key: key, pricing: { currency: 'CREDIT', ...pricing }, note, updated_by: 'alice', updated_at_ms: now - 3 * 24 * HOUR })
    price('model:openai:gpt-5.1', { input_token: 10, output_token: 40, cache_input_token: 2.5 })
    price('model:claude:claude-opus-4.7', { input_token: 15, output_token: 75 })
    price('model:claude:claude-sonnet-4.5', { input_token: 3, output_token: 15 })
    const image = this.inventory().find((model) => !model.token_metered && model.provider_instance_name === sn)
    if (image) price(`exact:${image.exact_model}`, { unit: 'image', amount: 0.5 }, 'SN images')

    const entry = (user: string, kind: CreditEntry['kind'], amount: bigint, at: number, note?: string) => {
      const balance = (this.balances.get(user)?.micro ?? 0n) + amount
      this.balances.set(user, { micro: balance, updated: at })
      this.entries.push({ entry_id: this.id('ce_'), user_id: user, kind, amount: formatCreditMicro(amount), balance_after: formatCreditMicro(balance), recharge_id: kind === 'recharge' ? `rc-seed-${this.sequence}` : undefined, operator: 'alice', note, created_at_ms: at })
    }
    const charge = (user: string, app: string, exact: string, tokens: number, amount: bigint, at: number, status: CreditCharge['status'] = 'charged') => {
      const balance = this.balances.get(user)?.micro ?? 0n
      this.balances.set(user, { micro: balance - amount, updated: at })
      this.charges.push({ attempt_id: this.id('att_'), task_id: this.id('task_'), user_id: user, app_id: app, provider_instance_name: exact.slice(exact.lastIndexOf('@') + 1), exact_model: exact, admitted_at_ms: at - 2000, tokens, amount: formatCreditMicro(amount), status, settled_at_ms: at })
    }
    entry('alice', 'recharge', 150_000_000n, now - 20 * 24 * HOUR, 'Initial')
    // Administrators are free (type:admin coefficient 0); bob pays.
    charge('alice', 'aiworkspace', 'gpt-5.1@openai-main', 182_000, 0n, now - 3 * HOUR, 'free')
    charge('alice', 'jarvis', 'claude-opus-4.7@anthropic-work', 640_000, 0n, now - 30 * HOUR, 'free')
    entry('bob', 'recharge', 30_000_000n, now - 10 * 24 * HOUR, 'Monthly')
    charge('bob', 'studio', 'claude-sonnet-4.5@anthropic-work', 1_200_000, 17_000_000n, now - 5 * HOUR)
    charge('bob', 'messagehub', 'gpt-5.1@openai-main', 220_000, 9_750_000n, now - 26 * HOUR)
    charge('bob', 'messagehub', 'sn-router-balanced@sn-router', 50_000, 0n, now - 27 * HOUR, 'free')
    entry('dave', 'recharge', 5_000_000n, now - 8 * 24 * HOUR)
    charge('dave', 'codeassistant', 'claude-opus-4.7@anthropic-work', 160_000, 7_500_000n, now - 7 * 24 * HOUR)
  }

  // ---------- Evaluation ----------

  private account(userId: string): MockAccount {
    const viewer = this.viewer()
    if (userId === viewer.user_id) return viewer
    const account = this.accounts().find((item) => item.user_id === userId)
    if (!account) throw new MockPolicyError('the user is not an account of this Zone')
    return account
  }

  private target(userId?: string): MockAccount {
    if (!userId || userId === this.viewer().user_id) return this.viewer()
    this.requireAdmin()
    return this.account(userId)
  }

  private entryFor(model: MockInventoryModel): [ModelCreditPrice['source'], PriceEntry] | undefined {
    const exact = this.prices.get(`exact:${model.exact_model}`) ??
      (model.variant ? this.prices.get(`exact:${model.provider_model_id}@${model.provider_instance_name}`) : undefined)
    if (exact) return ['exact', exact]
    if (!model.model_driver_id || !model.origin_model_id) return undefined
    const entry = this.prices.get(`model:${model.model_driver_id}:${model.origin_model_id}`)
    return entry ? ['model', entry] : undefined
  }

  private rate(currency?: string): number | undefined {
    return currency ? this.creditRates[currency.toUpperCase()] : undefined
  }

  private describe(model: MockInventoryModel, providerCoefficient: number, modelCoefficient: number): ModelCreditPrice {
    const coefficient = providerCoefficient * modelCoefficient
    const entry = this.entryFor(model)
    const rate = entry ? undefined : this.rate(model.provider_cost_currency)
    const state: ModelCreditPrice['state'] = coefficient === 0 ? 'free'
      : entry ? (isZeroPricing(entry[1].pricing) ? 'free' : 'priced')
      : rate !== undefined ? (rate === 0 ? 'free' : 'priced')
      : 'unpriced'
    const price: ModelCreditPrice = { state, provider_coefficient: providerCoefficient, model_coefficient: modelCoefficient }
    if (entry) Object.assign(price, { source: entry[0], entry_key: entry[1].entry_key, pricing: entry[1].pricing })
    else if (rate !== undefined) Object.assign(price, { source: 'provider_cost', provider_cost_currency: model.provider_cost_currency, credit_rate: rate })
    return price
  }

  private usable(policies: AiccPolicy[], account: MockAccount, appId?: string): Set<string> {
    const evaluator = new Evaluator(policies, account, appId)
    const out = new Set<string>()
    for (const model of this.inventory()) {
      const decision = evaluator.model(model.provider_instance_name, model.exact_model)
      if (decision.visible && this.describe(model, decision.providerCoefficient, decision.modelCoefficient).state !== 'unpriced') out.add(model.exact_model)
    }
    return out
  }

  /** What the viewer may see, for hiding resources in views of other people's policies. */
  private viewerVisible(): { providers: Set<string>; models: Set<string> } {
    const evaluator = new Evaluator(this.policies, this.viewer())
    const models = new Set<string>()
    const providers = new Set<string>()
    for (const model of this.inventory()) {
      if (evaluator.model(model.provider_instance_name, model.exact_model).visible) {
        models.add(model.exact_model)
        providers.add(model.provider_instance_name)
      }
    }
    for (const provider of new Set(this.inventory().map((model) => model.provider_instance_name))) {
      if (evaluator.providerVisible(provider)) providers.add(provider)
    }
    return { providers, models }
  }

  private effectiveLimit(limit: AppliedLimit, now: number, hideFor?: { providers: Set<string>; models: Set<string> }): EffectiveLimit {
    const counter = this.counters.get(scopeKey(limit.counter, limit.resource, limit.limit.period))
    const active = counter && counter.end > now ? counter : undefined
    const used = active ? (limit.limit.unit === 'token' ? BigInt(active.tokens) : active.credit) : 0n
    const hidden = hideFor && ((limit.resource.type === 'provider' && !hideFor.providers.has(limit.resource.provider_instance_name)) ||
      (limit.resource.type === 'model' && !hideFor.models.has(limit.resource.exact_model)))
    return {
      policy_id: limit.policy.policy_id,
      source: limit.policy.source,
      subject: limit.policy.subject,
      counter: limit.counter,
      resource: hidden ? { type: 'hidden' } : limit.resource,
      period: limit.limit.period,
      unit: limit.limit.unit,
      amount: limit.limit.amount,
      used: formatUsed(limit.limit.unit, used),
      ...(active ? { window_start_ms: active.start, reset_at_ms: active.end } : {}),
      status: limitStatus(limit.amount, used),
    }
  }

  private balance(userId: string): string {
    return formatCreditMicro(this.balances.get(userId)?.micro ?? 0n)
  }

  // ---------- Policies ----------

  private concerns(policy: AiccPolicy): boolean {
    const viewer = this.viewer()
    const subject = parseSubject(policy.subject)
    if (!subject) return false
    switch (subject.kind) {
      case 'type': return policy.subject === `type:${viewer.user_type}`
      case 'group': return viewer.groups.includes(subject.id)
      case 'user':
      case 'user_app': return subject.userId === viewer.user_id
      case 'app': return true
    }
  }

  private editable(policy: AiccPolicy): boolean {
    return policy.source === 'admin' ? this.manages() : parseSubject(policy.subject)?.userId === this.viewer().user_id
  }

  private view(policy: AiccPolicy): AiccPolicyView {
    const inventory = this.inventory()
    const providers = new Set(inventory.map((model) => model.provider_instance_name))
    const models = new Set(inventory.map((model) => model.exact_model))
    const unavailable = [
      ...Object.keys(policy.body.providers ?? {}).filter((provider) => !providers.has(provider)),
      ...Object.keys(policy.body.models ?? {}).filter((model) => !models.has(model)),
    ]
    let body = structuredClone(policy.body)
    if (!this.manages()) {
      const visible = this.viewerVisible()
      body = {
        ...body,
        providers: Object.fromEntries(Object.entries(body.providers ?? {}).filter(([name]) => visible.providers.has(name))),
        models: Object.fromEntries(Object.entries(body.models ?? {}).filter(([name]) => visible.models.has(name))),
      }
    }
    return { policy: { ...policy, body }, editable: this.editable(policy), ...(this.manages() && unavailable.length ? { unavailable_resources: unavailable } : {}) }
  }

  listPolicies(scope: 'mine' | 'manage'): AiccPolicyView[] {
    this.seed()
    if (scope === 'manage') {
      this.requireAdmin()
      return this.policies.map((policy) => this.view(policy))
    }
    return this.policies
      .filter((policy) => this.concerns(policy) && (!policy.subject.startsWith('app:') || this.manages()))
      .map((policy) => this.view(policy))
  }

  private policyOf(policyId: string): AiccPolicy {
    const policy = this.policies.find((item) => item.policy_id === policyId)
    if (!policy || (!this.manages() && !this.concerns(policy))) throw new MockPolicyError('the policy was not found')
    return policy
  }

  getPolicy(policyId: string): PolicyGetResponse {
    this.seed()
    const policy = this.policyOf(policyId)
    const audit = this.manages() || policy.source === 'user' ? this.audit.filter((item) => item.target_id === policyId) : []
    return { policy: this.view(policy), audit }
  }

  private validate(draft: PolicyDraft): PolicySource {
    const source = draft.source ?? (this.manages() ? 'admin' : 'user')
    const subject = parseSubject(draft.subject)
    if (!subject) throw new MockPolicyError(`\`${draft.subject}\` is not a policy subject`)
    const body = draft.body ?? {}
    const coefficientOk = (value?: number) => value === undefined || (Number.isFinite(value) && value >= 0 && value <= 1_000_000)
    const limitsOk = (limits?: PolicyLimit[]) => {
      const keys = new Set<string>()
      return (limits ?? []).every((limit) => {
        const key = `${limit.period}/${limit.unit}`
        if (keys.has(key) || limitValue(limit) === null) return false
        keys.add(key)
        return true
      })
    }
    const entries = [...Object.values(body.providers ?? {}), ...Object.values(body.models ?? {})]
    if (!coefficientOk(body.price_coefficient) || !limitsOk(body.limits) || !entries.every((entry) => coefficientOk(entry.price_coefficient) && limitsOk(entry.limits))) {
      throw new MockPolicyError('a price coefficient must be a non-negative number and each limit a non-negative amount, one per period and unit')
    }
    const hasCoefficient = body.price_coefficient !== undefined || entries.some((entry) => entry.price_coefficient !== undefined)
    if (isAppSide(draft.subject) && hasCoefficient) throw new MockPolicyError('an App policy cannot set a price coefficient')
    if (source === 'admin') {
      this.requireAdmin()
      if (subject.kind === 'group' && !this.groupIds().includes(subject.id)) throw new MockPolicyError('the policy names an undefined user group')
      if (subject.userId) this.account(subject.userId)
      return source
    }
    if (subject.kind !== 'user_app' || subject.userId !== this.viewer().user_id) throw new MockPolicyError('a personal policy applies to one of your own Apps', 'policy_denied')
    const widens = body.all_providers !== undefined ||
      Object.values(body.providers ?? {}).some((entry) => entry.visible === 'all_models') ||
      Object.entries(body.models ?? {}).some(([model, entry]) => entry.visible === true && body.providers?.[model.slice(model.lastIndexOf('@') + 1)]?.visible !== 'selected_models')
    if (widens) throw new MockPolicyError('a personal policy can only hide resources and add limits')
    if (!this.appsOf(this.viewer().user_id).includes(subject.appId ?? '')) throw new MockPolicyError('the App is not one of yours', 'policy_denied')
    return source
  }

  upsertPolicy(draft: PolicyDraft, expectedVersion?: number): AiccPolicyView {
    this.seed()
    const source = this.validate(draft)
    const existing = this.policies.find((policy) => policy.source === source && policy.subject === draft.subject)
    if ((existing?.version ?? undefined) !== expectedVersion) throw new MockPolicyError('settings revision conflict', 'settings_revision_conflict')
    const now = Date.now()
    const operator = this.viewer().user_id
    const before = existing ? structuredClone(existing) : undefined
    const policy: AiccPolicy = existing
      ? { ...existing, enabled: draft.enabled ?? true, body: structuredClone(draft.body ?? {}), version: existing.version + 1, updated_by: operator, updated_at_ms: now }
      : { policy_id: this.id('pol_'), source, subject: draft.subject, enabled: draft.enabled ?? true, body: structuredClone(draft.body ?? {}), version: 1, created_by: operator, created_at_ms: now, updated_by: operator, updated_at_ms: now }
    this.policies = existing ? this.policies.map((item) => item === existing ? policy : item) : [...this.policies, policy]
    this.audit.unshift({ audit_id: this.id('aud_'), target_type: 'policy', target_id: policy.policy_id, action: existing ? 'update' : 'create', operator, before: before as never, after: structuredClone(policy) as never, created_at_ms: now })
    return this.view(policy)
  }

  deletePolicy(policyId: string, expectedVersion: number) {
    this.seed()
    const policy = this.policyOf(policyId)
    if (!this.editable(policy)) throw new MockPolicyError('the policy cannot be changed by the caller', 'policy_denied')
    if (policy.version !== expectedVersion) throw new MockPolicyError('settings revision conflict', 'settings_revision_conflict')
    this.policies = this.policies.filter((item) => item !== policy)
    this.audit.unshift({ audit_id: this.id('aud_'), target_type: 'policy', target_id: policyId, action: 'delete', operator: this.viewer().user_id, before: structuredClone(policy) as never, created_at_ms: Date.now() })
  }

  effective(request: { user_id?: string; app_id?: string; draft?: PolicyDraft }): PolicyEffectiveResponse {
    this.seed()
    const account = this.target(request.user_id)
    const appId = request.app_id || undefined
    let drafted = this.policies
    if (request.draft) {
      const draft = request.draft
      const source = this.validate(draft)
      const existing = this.policies.find((policy) => policy.source === source && policy.subject === draft.subject)
      const policy: AiccPolicy = { ...(existing ?? { policy_id: 'draft', created_by: '', created_at_ms: 0, updated_by: '', updated_at_ms: 0 }), source, subject: draft.subject, enabled: draft.enabled ?? true, body: draft.body ?? {}, version: 0 }
      drafted = existing ? this.policies.map((item) => item === existing ? policy : item) : [...this.policies, policy]
    }
    const evaluator = new Evaluator(drafted, account, appId)
    const models: EffectiveModel[] = []
    const unpriced: string[] = []
    for (const model of this.inventory()) {
      const decision = evaluator.model(model.provider_instance_name, model.exact_model)
      if (!decision.visible) continue
      const price = this.describe(model, decision.providerCoefficient, decision.modelCoefficient)
      const usable = price.state !== 'unpriced'
      if (!usable) unpriced.push(model.exact_model)
      models.push({ exact_model: model.exact_model, provider_instance_name: model.provider_instance_name, visible_from: decision.visibleFrom, ...(decision.coefficientFrom.length ? { coefficient_from: decision.coefficientFrom } : {}), price, usable })
    }
    const now = Date.now()
    const hideFor = this.manages() ? undefined : this.viewerVisible()
    const response: PolicyEffectiveResponse = {
      user_id: account.user_id,
      ...(appId ? { app_id: appId } : {}),
      balance: this.balance(account.user_id),
      models,
      limits: evaluator.allLimits(this.inventory()).map((limit) => this.effectiveLimit(limit, now, hideFor)),
      ...(unpriced.length ? { unpriced_models: unpriced } : {}),
    }
    if (request.draft) {
      const draft = request.draft
      const before = this.usable(this.policies, account, appId)
      const after = this.usable(drafted, account, appId)
      response.added = [...after].filter((model) => !before.has(model))
      response.removed = [...before].filter((model) => !after.has(model))
      const subject = parseSubject(draft.subject)
      response.affected_users = this.accounts().filter((item) => {
        switch (subject?.kind) {
          case 'type': return draft.subject === `type:${item.user_type}`
          case 'group': return item.groups.includes(subject.id)
          case 'user':
          case 'user_app': return item.user_id === subject.userId
          case 'app': return true
          default: return false
        }
      }).map((item) => item.user_id)
      const callerApp = subject?.appId
      const mineBefore = this.usable(this.policies, this.viewer(), callerApp)
      const mineAfter = this.usable(drafted, this.viewer(), callerApp)
      response.caller_loses = [...mineBefore].filter((model) => !mineAfter.has(model))
    }
    return response
  }

  status(appId?: string): PolicyStatusResponse {
    this.seed()
    const viewer = this.viewer()
    const evaluator = new Evaluator(this.policies, viewer, appId)
    const now = Date.now()
    const hideFor = this.viewerVisible()
    const limits = evaluator.allLimits(this.inventory())
      .filter((limit) => appId !== undefined || limit.counter.type === 'user')
      .map((limit) => this.effectiveLimit(limit, now, hideFor))
    return {
      user_id: viewer.user_id,
      balance: this.balance(viewer.user_id),
      limits,
      near_limit_count: limits.filter((limit) => limit.status === 'near_limit').length,
      exhausted_count: limits.filter((limit) => limit.status === 'exhausted' || limit.status === 'forbidden').length,
    }
  }

  // ---------- Prices ----------

  getPrices(): PriceGetResponse {
    this.seed()
    if (this.manages()) {
      const unpriced = this.inventory().filter((model) => !this.entryFor(model) && this.rate(model.provider_cost_currency) === undefined).map((model) => model.exact_model)
      return {
        revision: this.priceRevision,
        entries: [...this.prices.values()].map((entry) => structuredClone(entry)).sort((a, b) => a.entry_key.localeCompare(b.entry_key)),
        ...(Object.keys(this.creditRates).length ? { credit_rates: { ...this.creditRates } } : {}),
        ...(unpriced.length ? { unpriced_models: unpriced } : {}),
      }
    }
    const visible = this.viewerVisible()
    const keys = new Set(this.inventory().filter((model) => visible.models.has(model.exact_model)).flatMap((model) => this.entryFor(model)?.[1].entry_key ?? []))
    return { revision: this.priceRevision, entries: [...keys].map((key) => structuredClone(this.prices.get(key)!)) }
  }

  updatePrices(request: PriceUpdateRequest): PriceGetResponse {
    this.seed()
    this.requireAdmin()
    if (request.expected_revision !== this.priceRevision) throw new MockPolicyError('settings revision conflict', 'settings_revision_conflict')
    const seen = new Set<string>()
    for (const input of request.upserts ?? []) {
      const key = parsePriceKey(input.entry_key)
      if (!key) throw new MockPolicyError('a price key starts with model: or exact:')
      if (seen.has(input.entry_key) || request.deletes?.includes(input.entry_key)) throw new MockPolicyError('a price key appears more than once')
      seen.add(input.entry_key)
      const pricing = input.pricing as Record<string, unknown> | null
      if (!pricing || typeof pricing !== 'object' || ((pricing.currency as string | undefined) ?? 'CREDIT') !== 'CREDIT') throw new MockPolicyError(`${input.entry_key}: a price list entry is priced in CREDIT`)
      const hasRate = ['input_token', 'output_token', 'cache_input_token', 'amount'].some((field) => typeof pricing[field] === 'number') || Array.isArray(pricing.tiers) || typeof pricing.tiers === 'object'
      if (!hasRate) throw new MockPolicyError(`${input.entry_key}: a price list entry needs token rates or a unit price`)
      if (key.kind === 'exact') {
        const provider = key.exactModel.slice(key.exactModel.lastIndexOf('@') + 1)
        const models = this.inventory().filter((model) => model.provider_instance_name === provider || model.exact_model.endsWith(`@${provider}`))
        if (models.length && !models.some((model) => model.exact_model === key.exactModel || `${model.provider_model_id}@${provider}` === key.exactModel)) {
          throw new MockPolicyError(`${input.entry_key} is not a model or variant of ${provider}`)
        }
      }
    }
    const now = Date.now()
    for (const input of request.upserts ?? []) {
      this.prices.set(input.entry_key, { entry_key: input.entry_key, pricing: { currency: 'CREDIT', ...(input.pricing as Record<string, unknown>) }, note: input.note, updated_by: this.viewer().user_id, updated_at_ms: now })
    }
    for (const key of request.deletes ?? []) this.prices.delete(key)
    if (request.credit_rates) {
      const rates: Record<string, number> = {}
      for (const [currency, rate] of Object.entries(request.credit_rates)) {
        const code = currency.trim().toUpperCase()
        if (!code || code === 'CREDIT' || !Number.isFinite(rate) || rate < 0) throw new MockPolicyError('a Credit rate needs a currency and a non-negative number')
        rates[code] = rate
      }
      this.creditRates = rates
    }
    this.priceRevision += 1
    return this.getPrices()
  }

  // ---------- Credit ----------

  private accountView(userId: string): CreditAccount {
    const balance = this.balances.get(userId)
    return { user_id: userId, balance: formatCreditMicro(balance?.micro ?? 0n), updated_at_ms: balance?.updated ?? 0 }
  }

  accountGet(userId?: string): CreditAccount {
    this.seed()
    return this.accountView(this.target(userId).user_id)
  }

  accountList(group?: string): CreditAccount[] {
    this.seed()
    this.requireAdmin()
    return this.accounts().filter((account) => !group || account.groups.includes(group)).map((account) => this.accountView(account.user_id))
  }

  ledger(request: { user_id?: string; cursor?: string; limit?: number }): CreditLedgerListResponse {
    this.seed()
    const userId = this.target(request.user_id).user_id
    const items: CreditLedgerItem[] = [
      ...this.entries.filter((entry) => entry.user_id === userId).map((entry) => ({ type: 'entry' as const, ...entry })),
      ...this.charges.filter((charge) => charge.user_id === userId).map((charge) => ({ type: 'charge' as const, ...charge })),
    ].sort((a, b) => time(b) - time(a))
    const offset = Number(request.cursor ?? 0) || 0
    const limit = Math.min(500, Math.max(1, request.limit ?? 50))
    const page = items.slice(offset, offset + limit)
    return { user_id: userId, balance: this.balance(userId), items: page, ...(offset + limit < items.length ? { next_cursor: String(offset + limit) } : {}) }
  }

  private apply(userId: string, requestId: string, kind: CreditEntry['kind'], amount: bigint, note?: string): CreditEntryResponse {
    this.requireAdmin()
    if (!requestId.trim() || requestId.length > 128) throw new MockPolicyError('the id is invalid')
    this.account(userId)
    const previous = this.entries.find((entry) => entry.recharge_id === requestId)
    if (previous) {
      if (previous.user_id !== userId || previous.kind !== kind || creditMicro(previous.amount) !== amount || (previous.note ?? undefined) !== (note ?? undefined)) {
        throw new MockPolicyError('the id was already used with different parameters', 'idempotency_conflict')
      }
      return { entry: { ...previous }, account: this.accountView(userId), replayed: true }
    }
    const now = Date.now()
    const balance = (this.balances.get(userId)?.micro ?? 0n) + amount
    this.balances.set(userId, { micro: balance, updated: now })
    const entry: CreditEntry = { entry_id: this.id('ce_'), user_id: userId, kind, amount: formatCreditMicro(amount), balance_after: formatCreditMicro(balance), recharge_id: requestId, operator: this.viewer().user_id, ...(note ? { note } : {}), created_at_ms: now }
    this.entries.push(entry)
    return { entry: { ...entry }, account: this.accountView(userId), replayed: false }
  }

  recharge(request: { user_id: string; recharge_id: string; amount: string; note?: string }): CreditEntryResponse {
    this.seed()
    const amount = creditMicro(request.amount)
    if (amount === null || amount <= 0n) throw new MockPolicyError('a recharge adds a positive amount (up to six decimals)')
    return this.apply(request.user_id, request.recharge_id, 'recharge', amount, request.note)
  }

  adjust(request: { user_id: string; adjust_id: string; amount: string; note: string }): CreditEntryResponse {
    this.seed()
    const amount = creditMicro(request.amount)
    if (amount === null || amount === 0n) throw new MockPolicyError('an adjustment is a non-zero signed amount (up to six decimals)')
    if (!request.note.trim()) throw new MockPolicyError('an adjustment needs a note')
    return this.apply(request.user_id, request.adjust_id, 'adjust', amount, request.note)
  }

  rechargeGet(rechargeId: string): { entry?: CreditEntry; account?: CreditAccount } {
    this.seed()
    this.requireAdmin()
    const entry = this.entries.find((item) => item.recharge_id === rechargeId)
    return entry ? { entry: { ...entry }, account: this.accountView(entry.user_id) } : {}
  }
}

function time(item: CreditLedgerItem): number {
  return item.type === 'entry' ? item.created_at_ms : item.settled_at_ms
}

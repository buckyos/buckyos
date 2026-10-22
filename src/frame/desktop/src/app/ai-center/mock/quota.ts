// In-memory token quota and visibility of the mock runtime. It keeps the rules the service
// enforces where the UI can observe them: rule identity and conflicts, versions, admin-only
// operations, total accounts restored on rebuild, idempotent recharges, visibility revisions
// and the grants a preview lists. Consumption is seeded, since nothing calls models here.
import {
  quotaWindow,
  type QuotaAccountView,
  type QuotaGrantRecord,
  type QuotaObject,
  type QuotaQueryResponse,
  type QuotaRechargeRequest,
  type QuotaRechargeResponse,
  type QuotaResource,
  type QuotaRule,
  type QuotaRuleDetail,
  type QuotaRuleDraft,
  type QuotaRulePreview,
  type QuotaRuleSource,
  type QuotaRuleUpdate,
  type QuotaRuleView,
  type QuotaUsageState,
  type VisibilityEntry,
  type VisibilityGrant,
  type VisibilityPreview,
  type VisibilityState,
  type VisibilitySubjectPolicy,
} from '../datamodel/quota'

export interface MockInventory { provider_instance_name: string; enabled: boolean; exact_models: string[] }
export interface MockAccount { user_id: string; user_type: 'admin' | 'user' | 'limited'; groups: string[] }

export class MockQuotaError extends Error {
  readonly code: string

  constructor(message: string, code = 'invalid_request') {
    super(message)
    this.code = code
  }
}

const NOW = () => Date.now()

function scopeKey(object: QuotaObject, resource: QuotaResource): string {
  return JSON.stringify([object, resource])
}

function overlapsObject(a: QuotaObject, b: QuotaObject): boolean {
  const user = (o: QuotaObject) => (o.type === 'app' ? undefined : o.user_id)
  const app = (o: QuotaObject) => (o.type === 'user' ? undefined : o.app_id)
  if (a.type === 'user' && b.type === 'app') return false
  if (a.type === 'app' && b.type === 'user') return false
  return (user(a) === undefined || user(b) === undefined || user(a) === user(b)) &&
    (app(a) === undefined || app(b) === undefined || app(a) === app(b))
}

function overlapsResource(a: QuotaResource, b: QuotaResource): boolean {
  if (a.type === 'all' || b.type === 'all') return true
  const provider = (r: QuotaResource) => (r.type === 'provider' ? r.provider_instance_name : r.type === 'model' ? r.exact_model.split('@').pop() : undefined)
  if (a.type === 'model' && b.type === 'model') return a.exact_model === b.exact_model
  return provider(a) === provider(b)
}

export class MockQuotaWorld {
  private rules: QuotaRule[] = []
  private consumed = new Map<string, number>()
  private accounts = new Map<string, QuotaAccountView>()
  private grants: QuotaGrantRecord[] = []
  private audit: QuotaRuleDetail['audit'] = []
  private visibility: VisibilitySubjectPolicy[] = []
  private visibilityRevision = 1
  private sequence = 0
  private seeded = false

  private inventory: () => MockInventory[]
  private viewer: () => MockAccount
  private accountsOf: () => MockAccount[]
  private groupIds: () => string[]

  constructor(
    inventory: () => MockInventory[],
    viewer: () => MockAccount,
    accountsOf: () => MockAccount[],
    groupIds: () => string[],
  ) {
    this.inventory = inventory
    this.viewer = viewer
    this.accountsOf = accountsOf
    this.groupIds = groupIds
  }

  private id(prefix: string): string {
    this.sequence += 1
    return `${prefix}${this.sequence.toString(16).padStart(6, '0')}`
  }

  private seed() {
    if (this.seeded) return
    this.seeded = true
    this.visibility = [
      { subject: 'type:admin', entries: [{ type: 'all_providers' }], updated_by: 'system', updated_at_ms: 0 },
      { subject: 'type:user', entries: [{ type: 'all_providers' }], updated_by: 'system', updated_at_ms: 0 },
    ]
    const first = this.inventory().find((provider) => provider.exact_models.length > 0)
    const add = (source: QuotaRuleSource, object: QuotaObject, resource: QuotaResource, kind: QuotaRule['kind'], extra: Partial<QuotaRule>, consumed: number) => {
      const rule = this.insert(source, object, resource, kind, extra, 'alice', extra.limit_tokens ?? 0)
      this.consumed.set(rule.rule_id, consumed)
      return rule
    }
    add('admin', { type: 'user', user_id: 'bob' }, first ? { type: 'provider', provider_instance_name: first.provider_instance_name } : { type: 'all' }, 'periodic', { period: '24h', limit_tokens: 200_000 }, 172_000)
    add('admin', { type: 'app', app_id: 'aiworkspace' }, { type: 'all' }, 'periodic', { period: 'week', limit_tokens: 2_000_000 }, 420_000)
    add('user', { type: 'user_app', user_id: 'alice', app_id: 'aiworkspace' }, { type: 'all' }, 'periodic', { period: '4h', limit_tokens: 30_000 }, 30_000)
    const total = this.insert('admin', { type: 'user', user_id: 'carol' }, { type: 'all' }, 'total', {}, 'alice', 100_000)
    const account = this.accounts.get(total.account_id ?? '')
    if (account) {
      account.settled_tokens = 60_000
      account.reserved_tokens = 10_000
      this.refreshAccount(account)
    }
  }

  private insert(source: QuotaRuleSource, object: QuotaObject, resource: QuotaResource, kind: QuotaRule['kind'], extra: Partial<QuotaRule>, operator: string, initial: number): QuotaRule {
    const now = NOW()
    let accountId: string | undefined
    if (kind === 'total') {
      const key = scopeKey(object, resource)
      let account = [...this.accounts.values()].find((item) => scopeKey(item.object, item.resource) === key)
      if (!account) {
        account = {
          account_id: this.id('qa_'), object, resource, granted_tokens: initial, settled_tokens: 0, reserved_tokens: 0,
          pending_tokens: 0, available_tokens: initial, debt_tokens: 0, start_at_ms: now, version: 1, updated_at_ms: now, enforced: true,
        }
        this.accounts.set(account.account_id, account)
        this.grants.push({ grant_id: `initial:${account.account_id}`, account_id: account.account_id, kind: 'initial', amount_tokens: initial, operator, created_at_ms: now })
      }
      accountId = account.account_id
    }
    const rule: QuotaRule = {
      rule_id: this.id('qr_'), source, object, resource, kind, account_id: accountId, enabled: true, version: 1,
      created_by: operator, created_at_ms: now, updated_by: operator, updated_at_ms: now, ...extra,
    }
    this.rules.push(rule)
    this.audit?.unshift({ audit_id: this.id('qx_'), target_type: 'rule', target_id: rule.rule_id, action: 'create', operator, after: rule, created_at_ms: now })
    return rule
  }

  private refreshAccount(account: QuotaAccountView) {
    const occupied = account.settled_tokens + account.reserved_tokens
    account.available_tokens = Math.max(0, account.granted_tokens - occupied)
    account.debt_tokens = Math.max(0, occupied - account.granted_tokens)
  }

  private state(rule: QuotaRule): QuotaUsageState {
    const now = NOW()
    const status = (cap: number, occupied: number, forbiddenAtZero: boolean): QuotaUsageState['status'] => {
      if (!rule.enabled) return 'disabled'
      if (cap === 0 && forbiddenAtZero) return 'forbidden'
      if (cap - occupied <= 0) return 'exhausted'
      return occupied * 100 >= cap * 80 ? 'near_limit' : 'normal'
    }
    if (rule.kind === 'total') {
      const account = this.accounts.get(rule.account_id ?? '')
      const granted = account?.granted_tokens ?? 0
      const settled = account?.settled_tokens ?? 0
      const reserved = account?.reserved_tokens ?? 0
      return {
        status: status(granted, settled + reserved, false), reset_at_ms: null, granted_tokens: granted,
        account_start_ms: account?.start_at_ms, settled_tokens: settled, reserved_tokens: reserved, pending_tokens: account?.pending_tokens ?? 0,
        available_tokens: Math.max(0, granted - settled - reserved), debt_tokens: Math.max(0, settled + reserved - granted), updated_at_ms: now,
      }
    }
    const limit = rule.limit_tokens ?? 0
    const settled = this.consumed.get(rule.rule_id) ?? 0
    const window = quotaWindow(rule.period ?? '24h', now)
    return {
      status: status(limit, settled, true), window_start_ms: window.startMs, window_end_ms: window.endMs, reset_at_ms: window.endMs,
      limit_tokens: limit, settled_tokens: settled, reserved_tokens: 0, pending_tokens: 0,
      available_tokens: Math.max(0, limit - settled), debt_tokens: Math.max(0, settled - limit), updated_at_ms: now,
    }
  }

  private manages(): boolean {
    return this.viewer().user_type === 'admin'
  }

  private concerns(rule: QuotaRule): boolean {
    const me = this.viewer().user_id
    if (rule.object.type === 'app') return rule.source === 'admin'
    return rule.object.user_id === me
  }

  private hiddenFor(resource: QuotaResource): boolean {
    if (this.manages()) return false
    const access = this.accessOf(this.viewer())
    if (resource.type === 'provider') return !access.has(resource.provider_instance_name)
    if (resource.type === 'model') return ![...access.values()].some((models) => models.has(resource.exact_model))
    return false
  }

  private view(rule: QuotaRule): QuotaRuleView {
    const editable = !rule.deleted && (rule.source === 'admin' ? this.manages() : rule.object.type !== 'app' && rule.object.user_id === this.viewer().user_id)
    const state = this.state(rule)
    const inInventory = rule.resource.type === 'all' ||
      (rule.resource.type === 'provider' && this.inventory().some((p) => p.provider_instance_name === (rule.resource as { provider_instance_name: string }).provider_instance_name)) ||
      (rule.resource.type === 'model' && this.inventory().some((p) => p.exact_models.includes((rule.resource as { exact_model: string }).exact_model)))
    if (!inInventory && state.status !== 'disabled') state.status = 'resource_unavailable'
    return { rule: this.hiddenFor(rule.resource) ? { ...rule, resource: { type: 'hidden' } } : rule, state, editable }
  }

  query(appId?: string): QuotaQueryResponse {
    this.seed()
    const rules = this.rules
      .filter((rule) => !rule.deleted && this.concerns(rule))
      .filter((rule) => !appId || rule.object.type === 'user' || rule.object.app_id === appId)
      .map((rule) => this.view(rule))
    return {
      user_id: this.viewer().user_id, rules,
      near_limit_count: rules.filter((view) => view.rule.enabled && view.state.status === 'near_limit').length,
      exhausted_count: rules.filter((view) => view.rule.enabled && ['exhausted', 'forbidden'].includes(view.state.status)).length,
    }
  }

  listRules(scope: 'mine' | 'manage'): QuotaRuleView[] {
    this.seed()
    if (scope === 'manage' && !this.manages()) throw new MockQuotaError('this operation requires an administrator', 'policy_denied')
    return this.rules.filter((rule) => !rule.deleted && (scope === 'manage' || this.concerns(rule))).map((rule) => this.view(rule))
  }

  private ruleOf(ruleId: string): QuotaRule {
    const rule = this.rules.find((item) => item.rule_id === ruleId && !item.deleted)
    if (!rule || (!this.manages() && !this.concerns(rule))) throw new MockQuotaError('the quota rule was not found')
    return rule
  }

  private constraints(object: QuotaObject, resource: QuotaResource, excluding?: string): QuotaRuleView[] {
    return this.rules
      .filter((rule) => !rule.deleted && rule.enabled && rule.rule_id !== excluding)
      .filter((rule) => overlapsObject(rule.object, object) && overlapsResource(rule.resource, resource))
      .filter((rule) => this.manages() || this.concerns(rule))
      .map((rule) => this.view(rule))
  }

  getRule(ruleId: string): QuotaRuleDetail {
    this.seed()
    const rule = this.ruleOf(ruleId)
    const view = this.view(rule)
    const constraints = [...(rule.enabled ? [view] : []), ...this.constraints(rule.object, rule.resource, rule.rule_id)]
    const account = rule.account_id ? this.accounts.get(rule.account_id) : undefined
    const own = rule.object.type !== 'app' && rule.object.user_id === this.viewer().user_id
    return {
      rule: view,
      constraints,
      blocking_rule_ids: constraints.filter((item) => item.state.available_tokens === 0).map((item) => item.rule.rule_id),
      account: account && (this.manages() || own) ? { ...account, enforced: rule.enabled } : undefined,
      grants: account && (this.manages() || own) ? this.grants.filter((grant) => grant.account_id === account.account_id).reverse() : [],
      audit: this.manages() || rule.source === 'user' ? (this.audit ?? []).filter((item) => item.target_id === rule.rule_id || item.target_id === rule.account_id) : [],
    }
  }

  private source(draft: QuotaRuleDraft): QuotaRuleSource {
    const source = draft.source ?? (this.manages() ? 'admin' : 'user')
    if (source === 'admin' && !this.manages()) throw new MockQuotaError('this operation requires an administrator', 'policy_denied')
    if (source === 'user') {
      if (draft.object.type !== 'user_app' || draft.object.user_id !== this.viewer().user_id) throw new MockQuotaError('a personal limit applies to one of your own Apps', 'policy_denied')
      if (draft.kind !== 'periodic') throw new MockQuotaError('only an administrator manages total quota', 'policy_denied')
    }
    if (draft.kind === 'periodic' && (!draft.period || draft.limit_tokens === undefined)) throw new MockQuotaError('a periodic rule needs a period and a token limit')
    return source
  }

  private duplicate(draft: QuotaRuleDraft, source: QuotaRuleSource): QuotaRule | undefined {
    const key = scopeKey(draft.object, draft.resource)
    return this.rules.find((rule) => !rule.deleted && rule.source === source && rule.kind === draft.kind &&
      scopeKey(rule.object, rule.resource) === key && (draft.kind === 'total' || rule.period === draft.period || rule.pending_period?.period === draft.period))
  }

  preview(draft: QuotaRuleDraft): QuotaRulePreview {
    this.seed()
    const source = this.source(draft)
    const existing = draft.kind === 'total'
      ? [...this.accounts.values()].find((account) => scopeKey(account.object, account.resource) === scopeKey(draft.object, draft.resource))
      : undefined
    const now = NOW()
    const probe: QuotaRule = {
      rule_id: '', source, object: draft.object, resource: draft.resource, kind: draft.kind, period: draft.period,
      limit_tokens: draft.limit_tokens, account_id: existing?.account_id, enabled: true, version: 0,
      created_by: '', created_at_ms: now, updated_by: '', updated_at_ms: now,
    }
    const state = draft.kind === 'total' && !existing
      ? { ...this.state(probe), granted_tokens: draft.initial_tokens ?? 0, available_tokens: draft.initial_tokens ?? 0, status: (draft.initial_tokens ?? 0) > 0 ? 'normal' as const : 'exhausted' as const }
      : this.state(probe)
    return {
      state,
      existing_account: existing,
      conflict_rule_id: this.duplicate(draft, source)?.rule_id,
      other_constraints: this.constraints(draft.object, draft.resource),
    }
  }

  create(draft: QuotaRuleDraft): QuotaRuleView {
    this.seed()
    const source = this.source(draft)
    const existing = this.duplicate(draft, source)
    if (existing) throw new MockQuotaError('a rule with the same source, object, resource and period already exists')
    const rule = this.insert(source, draft.object, draft.resource, draft.kind, { period: draft.period, limit_tokens: draft.limit_tokens }, this.viewer().user_id, draft.initial_tokens ?? 0)
    return this.view(rule)
  }

  update(update: QuotaRuleUpdate): QuotaRuleView {
    this.seed()
    const rule = this.ruleOf(update.rule_id)
    if (!this.view(rule).editable) throw new MockQuotaError('the quota rule cannot be changed by the caller', 'policy_denied')
    if (rule.version !== update.expected_version) throw new MockQuotaError('settings revision conflict', 'settings_revision_conflict')
    const before = { ...rule }
    if (update.limit_tokens !== undefined) rule.limit_tokens = update.limit_tokens
    if (update.enabled !== undefined) rule.enabled = update.enabled
    if (update.cancel_pending_period) rule.pending_period = undefined
    if (update.period && update.period !== rule.period) {
      const clash = this.duplicate({ object: rule.object, resource: rule.resource, kind: 'periodic', period: update.period }, rule.source)
      if (clash && clash.rule_id !== rule.rule_id) throw new MockQuotaError('a rule with the same source, object, resource and period already exists')
      rule.pending_period = { period: update.period, effective_at_ms: quotaWindow(rule.period ?? update.period, NOW()).endMs }
    }
    rule.version += 1
    rule.updated_by = this.viewer().user_id
    rule.updated_at_ms = NOW()
    this.audit?.unshift({ audit_id: this.id('qx_'), target_type: 'rule', target_id: rule.rule_id, action: 'update', operator: rule.updated_by, before, after: { ...rule }, created_at_ms: rule.updated_at_ms })
    return this.view(rule)
  }

  delete(ruleId: string, expectedVersion: number) {
    this.seed()
    const rule = this.ruleOf(ruleId)
    if (!this.view(rule).editable) throw new MockQuotaError('the quota rule cannot be changed by the caller', 'policy_denied')
    if (rule.version !== expectedVersion) throw new MockQuotaError('settings revision conflict', 'settings_revision_conflict')
    rule.deleted = true
    rule.enabled = false
    rule.version += 1
  }

  recharge(request: QuotaRechargeRequest): QuotaRechargeResponse {
    this.seed()
    if (!this.manages()) throw new MockQuotaError('this operation requires an administrator', 'policy_denied')
    if (!Number.isInteger(request.amount_tokens) || request.amount_tokens <= 0) throw new MockQuotaError('a recharge adds a positive number of tokens')
    const account = this.accounts.get(request.account_id)
    if (!account) throw new MockQuotaError('the quota account was not found')
    const previous = this.grants.find((grant) => grant.grant_id === request.recharge_id)
    if (previous) {
      if (previous.account_id !== request.account_id || previous.amount_tokens !== request.amount_tokens || (previous.note ?? undefined) !== (request.note ?? undefined)) {
        throw new MockQuotaError('the recharge id was already used with different parameters', 'idempotency_conflict')
      }
      return { recharge: previous, account: { ...account }, replayed: true }
    }
    const recharge: QuotaGrantRecord = {
      grant_id: request.recharge_id, account_id: account.account_id, kind: 'recharge', amount_tokens: request.amount_tokens,
      operator: this.viewer().user_id, note: request.note, created_at_ms: NOW(),
    }
    this.grants.push(recharge)
    account.granted_tokens += request.amount_tokens
    account.version += 1
    account.updated_at_ms = recharge.created_at_ms
    this.refreshAccount(account)
    this.audit?.unshift({ audit_id: this.id('qx_'), target_type: 'account', target_id: account.account_id, action: 'recharge', operator: recharge.operator, after: { recharge_id: recharge.grant_id, amount_tokens: recharge.amount_tokens }, created_at_ms: recharge.created_at_ms })
    return { recharge, account: { ...account }, replayed: false }
  }

  rechargeOf(rechargeId: string): { recharge?: QuotaGrantRecord; account?: QuotaAccountView } {
    const recharge = this.grants.find((grant) => grant.grant_id === rechargeId)
    return { recharge, account: recharge ? this.accounts.get(recharge.account_id) : undefined }
  }

  private grantsOf(policy: VisibilitySubjectPolicy[], subjects: string[]): Map<string, Set<string>> {
    const out = new Map<string, Set<string>>()
    for (const item of policy.filter((p) => subjects.includes(p.subject))) {
      for (const entry of item.entries) {
        for (const provider of this.inventory()) {
          const models = entry.type === 'all_providers' ? provider.exact_models
            : entry.provider_instance_name !== provider.provider_instance_name ? null
            : entry.type === 'provider_all_models' ? provider.exact_models
            : provider.exact_models.filter((model) => entry.exact_models.includes(model))
          if (!models) continue
          const set = out.get(provider.provider_instance_name) ?? new Set<string>()
          models.forEach((model) => set.add(`${model}\u0000${item.subject}`))
          set.add(`\u0000${item.subject}`)
          out.set(provider.provider_instance_name, set)
        }
      }
    }
    return out
  }

  private subjectsOf(account: MockAccount): string[] {
    return [`type:${account.user_type}`, ...account.groups.map((group) => `group:${group}`)]
  }

  private accessOf(account: MockAccount): Map<string, Set<string>> {
    const raw = this.grantsOf(this.visibility, this.subjectsOf(account))
    const out = new Map<string, Set<string>>()
    raw.forEach((set, provider) => out.set(provider, new Set([...set].map((item) => item.split('\u0000')[0]).filter(Boolean))))
    return out
  }

  visibilityState(): VisibilityState {
    this.seed()
    if (!this.manages()) throw new MockQuotaError('this operation requires an administrator', 'policy_denied')
    const exposed = new Set<string>()
    this.grantsOf(this.visibility, this.visibility.map((item) => item.subject)).forEach((set) => set.forEach((item) => {
      const model = item.split('\u0000')[0]
      if (model) exposed.add(model)
    }))
    return {
      revision: this.visibilityRevision,
      subjects: this.visibility.map((item) => ({ ...item, entries: [...item.entries] })),
      providers: this.inventory().map((provider) => ({ ...provider, exact_models: [...provider.exact_models] })),
      unexposed_models: this.inventory().flatMap((provider) => provider.exact_models).filter((model) => !exposed.has(model)),
    }
  }

  private checkSubject(subject: string) {
    const valid = ['type:admin', 'type:user', 'type:limited'].includes(subject) ||
      (subject.startsWith('group:') && this.groupIds().includes(subject.slice('group:'.length)))
    if (!valid) throw new MockQuotaError('visibility subject names an undefined user group')
  }

  updateVisibility(expectedRevision: number, subject: string, entries: VisibilityEntry[]): VisibilityState {
    this.seed()
    if (!this.manages()) throw new MockQuotaError('this operation requires an administrator', 'policy_denied')
    this.checkSubject(subject)
    if (expectedRevision !== this.visibilityRevision) throw new MockQuotaError('settings revision conflict', 'settings_revision_conflict')
    this.visibility = this.visibility.filter((item) => item.subject !== subject)
    if (entries.length > 0) this.visibility.push({ subject, entries, updated_by: this.viewer().user_id, updated_at_ms: NOW() })
    this.visibilityRevision += 1
    return this.visibilityState()
  }

  previewVisibility(userId: string | undefined, draft?: { subject: string; entries: VisibilityEntry[] }): VisibilityPreview {
    this.seed()
    if (!this.manages()) throw new MockQuotaError('this operation requires an administrator', 'policy_denied')
    const target = this.accountsOf().find((account) => account.user_id === (userId ?? this.viewer().user_id))
    if (!target) throw new MockQuotaError('the user is not an account of this Zone')
    const effective = draft
      ? [...this.visibility.filter((item) => item.subject !== draft.subject), ...(draft.entries.length > 0 ? [{ subject: draft.subject, entries: draft.entries, updated_by: '', updated_at_ms: 0 }] : [])]
      : this.visibility
    const grantsMap = this.grantsOf(effective, this.subjectsOf(target))
    const grants: VisibilityGrant[] = []
    grantsMap.forEach((set, provider) => {
      const byModel = new Map<string, string[]>()
      set.forEach((item) => {
        const [model, subject] = item.split('\u0000')
        byModel.set(model, [...(byModel.get(model) ?? []), subject])
      })
      const models = [...byModel.keys()].filter(Boolean).sort()
      if (models.length === 0) grants.push({ provider_instance_name: provider, sources: byModel.get('') ?? [] })
      models.forEach((model) => grants.push({ provider_instance_name: provider, exact_model: model, sources: [...new Set(byModel.get(model))].sort() }))
    })
    const response: VisibilityPreview = { user_id: target.user_id, grants }
    if (draft) {
      const models = (policy: VisibilitySubjectPolicy[], subjects: string[]) => {
        const out = new Set<string>()
        this.grantsOf(policy, subjects).forEach((set) => set.forEach((item) => { const model = item.split('\u0000')[0]; if (model) out.add(model) }))
        return out
      }
      const before = models(this.visibility, [draft.subject])
      const after = models(effective, [draft.subject])
      response.added = [...after].filter((model) => !before.has(model))
      response.removed = [...before].filter((model) => !after.has(model))
      response.affected_users = this.accountsOf().filter((account) => this.subjectsOf(account).includes(draft.subject)).map((account) => account.user_id)
      const mine = this.subjectsOf(this.viewer())
      const mineAfter = models(effective, mine)
      response.caller_loses = [...models(this.visibility, mine)].filter((model) => !mineAfter.has(model))
    }
    return response
  }
}

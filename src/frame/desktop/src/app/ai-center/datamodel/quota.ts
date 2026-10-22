// Token quota and resource visibility of AICC (product/ai_center/AI_Center_Quota_PRD.md).
// Wire types follow buckyos-api `aicc_quota.rs`; the helpers only format and preview, the
// service computes every state and decides every call.

export type QuotaRuleSource = 'admin' | 'user'
export type QuotaObject =
  | { type: 'user'; user_id: string }
  | { type: 'app'; app_id: string }
  | { type: 'user_app'; user_id: string; app_id: string }
export type QuotaResource =
  | { type: 'all' }
  | { type: 'provider'; provider_instance_name: string }
  | { type: 'model'; exact_model: string }
  | { type: 'hidden' }
export type QuotaPeriod = '4h' | '24h' | '72h' | 'week'
export type QuotaKind = 'periodic' | 'total'
export type QuotaStatus = 'normal' | 'near_limit' | 'exhausted' | 'forbidden' | 'disabled' | 'resource_unavailable'

export interface QuotaPendingPeriod { period: QuotaPeriod; effective_at_ms: number; conflict?: boolean }

export interface QuotaRule {
  rule_id: string
  source: QuotaRuleSource
  object: QuotaObject
  resource: QuotaResource
  kind: QuotaKind
  period?: QuotaPeriod
  limit_tokens?: number
  account_id?: string
  enabled: boolean
  deleted?: boolean
  pending_period?: QuotaPendingPeriod
  version: number
  created_by: string
  created_at_ms: number
  updated_by: string
  updated_at_ms: number
}

export interface QuotaUsageState {
  status: QuotaStatus
  window_start_ms?: number
  window_end_ms?: number
  reset_at_ms: number | null
  limit_tokens?: number
  granted_tokens?: number
  account_start_ms?: number
  settled_tokens: number
  reserved_tokens: number
  pending_tokens: number
  available_tokens: number
  debt_tokens: number
  updated_at_ms: number
}

export interface QuotaRuleView { rule: QuotaRule; state: QuotaUsageState; editable: boolean }

export interface QuotaQueryResponse { user_id: string; rules: QuotaRuleView[]; near_limit_count: number; exhausted_count: number }

export interface QuotaAuditRecord {
  audit_id: string
  target_type: string
  target_id: string
  action: string
  operator: string
  before?: unknown
  after?: unknown
  created_at_ms: number
}

export interface QuotaGrantRecord {
  grant_id: string
  account_id: string
  kind: 'initial' | 'recharge'
  amount_tokens: number
  operator: string
  note?: string
  created_at_ms: number
}

export interface QuotaAccountView {
  account_id: string
  object: QuotaObject
  resource: QuotaResource
  granted_tokens: number
  settled_tokens: number
  reserved_tokens: number
  pending_tokens: number
  available_tokens: number
  debt_tokens: number
  start_at_ms: number
  version: number
  updated_at_ms: number
  enforced: boolean
}

export interface QuotaRuleDetail {
  rule: QuotaRuleView
  constraints: QuotaRuleView[]
  blocking_rule_ids: string[]
  account?: QuotaAccountView
  grants?: QuotaGrantRecord[]
  audit?: QuotaAuditRecord[]
}

export interface QuotaRuleDraft {
  source?: QuotaRuleSource
  object: QuotaObject
  resource: QuotaResource
  kind: QuotaKind
  period?: QuotaPeriod
  limit_tokens?: number
  initial_tokens?: number
}

export interface QuotaRulePreview {
  state: QuotaUsageState
  existing_account?: QuotaAccountView
  conflict_rule_id?: string
  other_constraints: QuotaRuleView[]
}

export interface QuotaRuleUpdate {
  rule_id: string
  expected_version: number
  limit_tokens?: number
  period?: QuotaPeriod
  enabled?: boolean
  cancel_pending_period?: boolean
}

export interface QuotaRechargeRequest { account_id: string; recharge_id: string; amount_tokens: number; note?: string }
export interface QuotaRechargeResponse { recharge: QuotaGrantRecord; account: QuotaAccountView; replayed: boolean }

export type VisibilityEntry =
  | { type: 'all_providers' }
  | { type: 'provider_all_models'; provider_instance_name: string }
  | { type: 'provider_models'; provider_instance_name: string; exact_models: string[] }

export interface VisibilitySubjectPolicy { subject: string; entries: VisibilityEntry[]; updated_by: string; updated_at_ms: number }

export interface VisibilityState {
  revision: number
  subjects: VisibilitySubjectPolicy[]
  providers: { provider_instance_name: string; enabled: boolean; exact_models: string[] }[]
  unexposed_models: string[]
}

export interface VisibilityGrant { provider_instance_name: string; exact_model?: string; sources: string[] }

export interface VisibilityPreview {
  user_id: string
  grants: VisibilityGrant[]
  added?: string[]
  removed?: string[]
  affected_users?: string[]
  caller_loses?: string[]
}

/** Per-Provider choice of a group (§9.5): not open, every model following the inventory, or chosen models. */
export type ProviderChoice =
  | { mode: 'closed' }
  | { mode: 'all_models' }
  | { mode: 'models'; exact_models: string[] }

export interface GroupVisibilityDraft { allProviders: boolean; providers: Record<string, ProviderChoice> }

export const QUOTA_PERIODS: QuotaPeriod[] = ['4h', '24h', '72h', 'week']

const HOUR_MS = 60 * 60 * 1000
/** `1970-01-05 00:00 UTC`, a Monday: base of the 72-hour and weekly windows. */
const PERIOD_EPOCH_MS = 4 * 24 * HOUR_MS

export function periodLengthMs(period: QuotaPeriod): number {
  switch (period) {
    case '4h': return 4 * HOUR_MS
    case '24h': return 24 * HOUR_MS
    case '72h': return 72 * HOUR_MS
    case 'week': return 7 * 24 * HOUR_MS
  }
}

/** The fixed UTC window `[start, end)` containing `atMs` (§5.1), as the service computes it. */
export function quotaWindow(period: QuotaPeriod, atMs: number): { startMs: number; endMs: number } {
  const length = periodLengthMs(period)
  const base = period === '72h' || period === 'week' ? PERIOD_EPOCH_MS : 0
  const startMs = base + Math.floor((atMs - base) / length) * length
  return { startMs, endMs: startMs + length }
}

/** `1.2K`, `3.4M`; exact integers below 1,000. */
export function formatTokens(value: number): string {
  const abs = Math.abs(value)
  const scale = (n: number, suffix: string) => {
    const scaled = value / n
    return `${Number.isInteger(scaled) ? scaled : scaled.toFixed(scaled >= 100 ? 0 : 1).replace(/\.0$/, '')}${suffix}`
  }
  if (abs >= 1_000_000_000) return scale(1_000_000_000, 'B')
  if (abs >= 1_000_000) return scale(1_000_000, 'M')
  if (abs >= 1_000) return scale(1_000, 'K')
  return String(value)
}

/** Exact integer with thousands separators (shown next to every abbreviated value). */
export function formatTokensExact(value: number): string {
  return Math.round(value).toLocaleString('en-US')
}

/** `50k`, `1.5M`, `2,000,000` → a non-negative integer; `null` when not a token amount. */
export function parseTokens(text: string): number | null {
  const match = text.trim().replace(/[,_\s]/g, '').match(/^(\d+(?:\.\d+)?)([kKmMbB]?)$/)
  if (!match) return null
  const factor = { '': 1, k: 1_000, m: 1_000_000, b: 1_000_000_000 }[match[2].toLowerCase() as '' | 'k' | 'm' | 'b']
  const value = Number(match[1]) * factor
  if (!Number.isFinite(value) || value > Number.MAX_SAFE_INTEGER) return null
  return Number.isInteger(value) ? value : Math.round(value)
}

/** Occupied share of the limit or grant (can exceed 1 with a debt). */
export function usageRatio(state: QuotaUsageState): number {
  const cap = state.limit_tokens ?? state.granted_tokens ?? 0
  const occupied = state.settled_tokens + state.reserved_tokens
  if (cap <= 0) return occupied > 0 || state.status === 'exhausted' || state.status === 'forbidden' ? 1 : 0
  return occupied / cap
}

export function quotaTone(status: QuotaStatus): 'ok' | 'warning' | 'error' | 'unknown' {
  switch (status) {
    case 'normal': return 'ok'
    case 'near_limit': return 'warning'
    case 'exhausted':
    case 'forbidden': return 'error'
    case 'disabled':
    case 'resource_unavailable': return 'unknown'
  }
}

export function objectUserId(object: QuotaObject): string | undefined {
  return object.type === 'app' ? undefined : object.user_id
}

export function objectAppId(object: QuotaObject): string | undefined {
  return object.type === 'user' ? undefined : object.app_id
}

export function resourceProvider(resource: QuotaResource): string | undefined {
  if (resource.type === 'provider') return resource.provider_instance_name
  if (resource.type === 'model') return resource.exact_model.split('@').pop()
  return undefined
}

/** `model:variant@provider` → `{ model, variant, provider }` for showing the full identity. */
/** Worst warning state of the viewer's own user-wide limits over any of these exact models (§9.5 marks). */
export function modelQuotaStatus(rules: QuotaRuleView[], exactModels: string[]): 'near_limit' | 'exhausted' | 'forbidden' | undefined {
  const rank = { near_limit: 1, exhausted: 2, forbidden: 3 } as const
  let worst: keyof typeof rank | undefined
  for (const { rule, state } of rules) {
    if (!rule.enabled || rule.object.type !== 'user' || !(state.status in rank)) continue
    const resource = rule.resource
    const covers = exactModels.some((exact) =>
      resource.type === 'all' ||
      (resource.type === 'provider' && resourceProvider({ type: 'model', exact_model: exact }) === resource.provider_instance_name) ||
      (resource.type === 'model' && resource.exact_model === exact))
    const status = state.status as keyof typeof rank
    if (covers && (!worst || rank[status] > rank[worst])) worst = status
  }
  return worst
}

export function splitExactModel(exactModel: string): { model: string; variant?: string; provider: string } {
  const at = exactModel.lastIndexOf('@')
  const head = at >= 0 ? exactModel.slice(0, at) : exactModel
  const provider = at >= 0 ? exactModel.slice(at + 1) : ''
  const colon = head.indexOf(':')
  return colon >= 0 ? { model: head.slice(0, colon), variant: head.slice(colon + 1), provider } : { model: head, provider }
}

export interface QuotaFilter {
  query: string
  kind: 'all' | QuotaKind
  source: 'all' | QuotaRuleSource
  status: 'all' | 'limited' | 'disabled'
}

export const EMPTY_QUOTA_FILTER: QuotaFilter = { query: '', kind: 'all', source: 'all', status: 'all' }

export function filterRules(rules: QuotaRuleView[], filter: QuotaFilter): QuotaRuleView[] {
  const query = filter.query.trim().toLowerCase()
  return rules.filter(({ rule, state }) => {
    if (filter.kind !== 'all' && rule.kind !== filter.kind) return false
    if (filter.source !== 'all' && rule.source !== filter.source) return false
    if (filter.status === 'disabled' && rule.enabled) return false
    if (filter.status === 'limited' && !['near_limit', 'exhausted', 'forbidden'].includes(state.status)) return false
    if (!query) return true
    const haystack = [
      objectUserId(rule.object), objectAppId(rule.object),
      rule.resource.type === 'provider' ? rule.resource.provider_instance_name : undefined,
      rule.resource.type === 'model' ? rule.resource.exact_model : undefined,
      rule.rule_id,
    ].filter(Boolean).join(' ').toLowerCase()
    return haystack.includes(query)
  })
}

/** The stored allow items of a subject as the per-Provider choices of the editor. */
export function toGroupDraft(entries: VisibilityEntry[]): GroupVisibilityDraft {
  const draft: GroupVisibilityDraft = { allProviders: false, providers: {} }
  for (const entry of entries) {
    if (entry.type === 'all_providers') draft.allProviders = true
    else if (entry.type === 'provider_all_models') draft.providers[entry.provider_instance_name] = { mode: 'all_models' }
    else draft.providers[entry.provider_instance_name] = { mode: 'models', exact_models: [...entry.exact_models] }
  }
  return draft
}

/** The editor's choices back as allow items (closed Providers produce nothing). */
export function fromGroupDraft(draft: GroupVisibilityDraft): VisibilityEntry[] {
  const entries: VisibilityEntry[] = draft.allProviders ? [{ type: 'all_providers' }] : []
  for (const [provider, choice] of Object.entries(draft.providers).sort(([a], [b]) => a.localeCompare(b))) {
    if (choice.mode === 'all_models') entries.push({ type: 'provider_all_models', provider_instance_name: provider })
    if (choice.mode === 'models') entries.push({ type: 'provider_models', provider_instance_name: provider, exact_models: [...choice.exact_models].sort() })
  }
  return entries
}

/** UserType groups first (admin, user, limited), then system groups by id. */
export function sortSubjects(subjects: string[]): string[] {
  const typeOrder = ['type:admin', 'type:user', 'type:limited']
  return [...subjects].sort((a, b) => {
    const ia = typeOrder.indexOf(a)
    const ib = typeOrder.indexOf(b)
    if (ia >= 0 || ib >= 0) return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib)
    return a.localeCompare(b)
  })
}

/** AICC refusal codes (§7.4) the UI explains. */
export const QUOTA_REFUSAL_CODES = ['quota_exhausted', 'quota_budget_exceeded', 'quota_unavailable', 'resource_not_permitted', 'attribution_unavailable'] as const
export type QuotaRefusalCode = (typeof QUOTA_REFUSAL_CODES)[number]

export interface QuotaDenialDetails {
  required_tokens: number
  max_allowed_tokens: number
  provider_instance_name: string
  exact_model: string
  blocking: { rule_id: string; source: QuotaRuleSource; object: QuotaObject; resource: QuotaResource; kind: QuotaKind; period?: QuotaPeriod; state: QuotaUsageState; recoverable: boolean }[]
  reset_at_ms: number | null
}

/** The refusal of an AICC call: its wire `code` and details, never its text. */
export function quotaRefusalOf(error: unknown): { code: QuotaRefusalCode; details?: QuotaDenialDetails } | undefined {
  // Thrown kRPC errors carry the `AiccError` in `data`; a failed task carries it directly.
  const e = error as { code?: unknown; data?: unknown; details?: unknown } | null
  if (typeof e?.code !== 'string' || !(QUOTA_REFUSAL_CODES as readonly string[]).includes(e.code)) return undefined
  const details = (e.data as { details?: QuotaDenialDetails } | undefined)?.details ?? (e.details as QuotaDenialDetails | undefined)
  return { code: e.code as QuotaRefusalCode, details }
}

// AICC user policies, the Credit price list and Credit accounts (buckyos doc/aicc/aicc_user_policy.md,
// notepads/aicc-user-policy-credit-todo.md). Wire types are the websdk mirror of buckyos-api
// `aicc_quota.rs`; the helpers here only format, edit and compare drafts: the service evaluates every
// policy and decides every call.
import type {
  AiccPolicy,
  AiccPolicyView,
  CreditInsufficientDetails,
  EffectiveLimit,
  LimitPeriod,
  LimitStatus,
  LimitUnit,
  ModelCreditPrice,
  ModelPolicy,
  PolicyBody,
  PolicyLimit,
  ProviderPolicy,
  ProviderVisibility,
  QuotaDenialDetails,
} from 'buckyos'

export type {
  AiccPolicy,
  AiccPolicyView,
  CreditAccount,
  CreditCharge,
  CreditChargeStatus,
  CreditEntry,
  CreditEntryResponse,
  CreditInsufficientDetails,
  CreditLedgerItem,
  CreditLedgerListResponse,
  CreditPriceState,
  EffectiveLimit,
  EffectiveModel,
  LimitCounter,
  LimitPeriod,
  LimitResource,
  LimitStatus,
  LimitUnit,
  ModelCreditPrice,
  ModelPolicy,
  PolicyAuditRecord,
  PolicyBody,
  PolicyDraft,
  PolicyEffectiveResponse,
  PolicyGetResponse,
  PolicyLimit,
  PolicySource,
  PolicyStatusResponse,
  PriceEntry,
  PriceEntryInput,
  PriceGetResponse,
  PriceUpdateRequest,
  ProviderPolicy,
  ProviderVisibility,
  QuotaBlockingLimit,
  QuotaDenialDetails,
} from 'buckyos'

/** `details` of `resource_not_permitted` for a visible model without a price (buckyos-api `ResourceUnpricedDetails`). */
export interface ResourceUnpricedDetails { reason: 'unpriced'; provider_instance_name: string; exact_model: string }

export const LIMIT_PERIODS: LimitPeriod[] = ['5h', '24h', '7d']
export const LIMIT_UNITS: LimitUnit[] = ['credit', 'token']
export const USER_TYPE_SUBJECTS = ['type:admin', 'type:user', 'type:limited'] as const

// ---------- Credit amounts (decimal strings, six decimals, may be negative) ----------

const MICRO = 1_000_000n

/** `12.5` → 12_500_000n; at most six decimals and an optional sign, `null` for anything else (as the service parses it). */
export function creditMicro(value: string): bigint | null {
  const text = value.trim()
  const match = text.match(/^([+-]?)(\d*)(?:\.(\d*))?$/)
  if (!match) return null
  const [, sign, whole, fraction] = match
  if ((!whole && !fraction) || (fraction !== undefined && fraction.length === 0) || (fraction?.length ?? 0) > 6) return null
  const micro = BigInt(whole || '0') * MICRO + BigInt((fraction ?? '').padEnd(6, '0') || '0')
  if (micro > 9_223_372_036_854_775_807n) return null
  return sign === '-' ? -micro : micro
}

/** 12_500_000n → `"12.5"`; whole numbers have no decimal point. */
export function formatCreditMicro(micro: bigint): string {
  const negative = micro < 0n
  const abs = negative ? -micro : micro
  const whole = abs / MICRO
  const fraction = (abs % MICRO).toString().padStart(6, '0').replace(/0+$/, '')
  return `${negative ? '-' : ''}${whole}${fraction ? `.${fraction}` : ''}`
}

/** Sum of two Credit amounts, exactly. */
export function addCredit(left: string, right: string): string | null {
  const a = creditMicro(left)
  const b = creditMicro(right)
  return a === null || b === null ? null : formatCreditMicro(a + b)
}

/** Two decimals for people; a tiny non-zero amount reads `< 0.01` (or `> -0.01`), never `0.00`. */
export function formatCredit(value: string | undefined | null, locale = 'en'): string {
  const micro = creditMicro(value ?? '0') ?? 0n
  const format = (amount: number) => amount.toLocaleString(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 })
  if (micro > 0n && micro < 10_000n) return `< ${format(0.01)}`
  if (micro < 0n && micro > -10_000n) return `> ${format(-0.01)}`
  return format(Number(micro) / 1_000_000)
}

/** The exact decimal shown next to a rounded amount. */
export function exactCredit(value: string | undefined | null): string {
  const micro = creditMicro(value ?? '0')
  return micro === null ? String(value ?? '') : formatCreditMicro(micro)
}

export function isNegativeCredit(value: string | undefined | null): boolean {
  return (creditMicro(value ?? '0') ?? 0n) < 0n
}

export function isPositiveCredit(value: string | undefined | null): boolean {
  return (creditMicro(value ?? '0') ?? 0n) > 0n
}

/** A positive amount typed by a person (recharge); `null` when not one. */
export function parsePositiveCredit(text: string): string | null {
  const micro = creditMicro(text.replace(/[,_\s]/g, ''))
  return micro !== null && micro > 0n ? formatCreditMicro(micro) : null
}

/** A signed, non-zero amount (adjustment); `null` when not one. */
export function parseSignedCredit(text: string): string | null {
  const micro = creditMicro(text.replace(/[,_\s]/g, ''))
  return micro !== null && micro !== 0n ? formatCreditMicro(micro) : null
}

// ---------- Token amounts ----------

/** `1.2K`, `3.4M`; exact integers below 1,000. */
export function formatTokens(value: number, locale = 'en'): string {
  if (locale !== 'en' && locale !== 'en-US') {
    return value.toLocaleString(locale, { notation: 'compact', maximumFractionDigits: 1 })
  }
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
export function formatTokensExact(value: number, locale = 'en'): string {
  return Math.round(value).toLocaleString(locale)
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

// ---------- Limits ----------

/** The amount typed for a limit in its unit's wire form (`'0'` forbids use); `null` when invalid. */
export function parseLimitAmount(unit: LimitUnit, text: string): string | null {
  if (unit === 'token') {
    const tokens = parseTokens(text)
    return tokens === null ? null : String(tokens)
  }
  const micro = creditMicro(text.replace(/[,_\s]/g, ''))
  return micro !== null && micro >= 0n ? formatCreditMicro(micro) : null
}

/** Limits of one place of a policy are unique per period and unit, with valid amounts. */
export function limitsValid(limits: PolicyLimit[] | undefined): boolean {
  const keys = new Set<string>()
  return (limits ?? []).every((limit) => {
    const key = `${limit.period}/${limit.unit}`
    if (keys.has(key)) return false
    keys.add(key)
    return parseLimitAmount(limit.unit, limit.amount) === limit.amount
  })
}

/** Share of the limit used (above 1 when a large call went over). */
export function limitRatio(limit: Pick<EffectiveLimit, 'unit' | 'used' | 'amount'>): number {
  const value = (text: string) => limit.unit === 'token' ? Number(text) : Number(creditMicro(text) ?? 0n)
  const amount = value(limit.amount)
  const used = value(limit.used)
  if (!(amount > 0)) return 1
  return used / amount
}

export function limitTone(status: LimitStatus): 'ok' | 'warning' | 'error' {
  return status === 'normal' ? 'ok' : status === 'near_limit' ? 'warning' : 'error'
}

const STATUS_RANK: Record<LimitStatus, number> = { normal: 0, near_limit: 1, exhausted: 2, forbidden: 3 }

/** Worst state of the limits that apply to calls of this exact model (resource all, its Provider or itself). */
export function modelLimitStatus(limits: EffectiveLimit[], exactModel: string): LimitStatus | undefined {
  const provider = splitExactModel(exactModel).provider
  let worst: LimitStatus | undefined
  for (const limit of limits) {
    const resource = limit.resource
    const covers = resource.type === 'all' ||
      (resource.type === 'provider' && resource.provider_instance_name === provider) ||
      (resource.type === 'model' && resource.exact_model === exactModel)
    if (covers && limit.status !== 'normal' && (!worst || STATUS_RANK[limit.status] > STATUS_RANK[worst])) worst = limit.status
  }
  return worst
}

// ---------- Subjects ----------

export type SubjectKind = 'type' | 'group' | 'user' | 'app' | 'user_app'

export interface SubjectParts { kind: SubjectKind; id: string; userId?: string; appId?: string }

export function parseSubject(subject: string): SubjectParts | null {
  const colon = subject.indexOf(':')
  if (colon <= 0) return null
  const kind = subject.slice(0, colon)
  const rest = subject.slice(colon + 1)
  if (!rest) return null
  switch (kind) {
    case 'type': return (USER_TYPE_SUBJECTS as readonly string[]).includes(subject) ? { kind, id: rest } : null
    case 'group': return { kind, id: rest }
    case 'user': return rest.includes('/') ? null : { kind, id: rest, userId: rest }
    case 'app': return { kind, id: rest, appId: rest }
    case 'user_app': {
      const slash = rest.indexOf('/')
      if (slash <= 0 || slash === rest.length - 1) return null
      return { kind, id: rest, userId: rest.slice(0, slash), appId: rest.slice(slash + 1) }
    }
    default: return null
  }
}

/** App side subjects only narrow and carry no coefficient. */
export function isAppSide(subject: string): boolean {
  return subject.startsWith('app:') || subject.startsWith('user_app:')
}

const KIND_ORDER: Record<SubjectKind, number> = { type: 0, group: 1, user: 2, app: 3, user_app: 4 }

/** UserType groups first (admin, user, limited), then system groups, users, Apps and users' Apps. */
export function sortSubjects(subjects: string[]): string[] {
  const typeOrder = USER_TYPE_SUBJECTS as readonly string[]
  return [...subjects].sort((a, b) => {
    const ka = parseSubject(a)?.kind
    const kb = parseSubject(b)?.kind
    const oa = ka ? KIND_ORDER[ka] : 9
    const ob = kb ? KIND_ORDER[kb] : 9
    if (oa !== ob) return oa - ob
    if (ka === 'type') return typeOrder.indexOf(a) - typeOrder.indexOf(b)
    return a.localeCompare(b)
  })
}

// ---------- Policy bodies ----------

function compactLimits(limits: PolicyLimit[] | undefined): PolicyLimit[] | undefined {
  if (!limits?.length) return undefined
  return [...limits].sort((a, b) => LIMIT_PERIODS.indexOf(a.period) - LIMIT_PERIODS.indexOf(b.period) || a.unit.localeCompare(b.unit))
}

function compactEntry<T extends ProviderPolicy | ModelPolicy>(entry: T): T | undefined {
  const out = {} as T
  if (entry.visible !== undefined) out.visible = entry.visible
  if (entry.price_coefficient !== undefined) out.price_coefficient = entry.price_coefficient
  const limits = compactLimits(entry.limits)
  if (limits) out.limits = limits
  return Object.keys(out).length ? out : undefined
}

/** Drop what has no opinion (empty Provider / model entries, empty limit lists, `all_providers: false`)
 * and sort keys, so two bodies that mean the same compare equal and the service stores no noise. */
export function normalizeBody(body: PolicyBody): PolicyBody {
  const out: PolicyBody = {}
  if (body.all_providers === true) out.all_providers = true
  if (body.price_coefficient !== undefined) out.price_coefficient = body.price_coefficient
  const limits = compactLimits(body.limits)
  if (limits) out.limits = limits
  const providers = Object.entries(body.providers ?? {})
    .sort(([a], [b]) => a.localeCompare(b))
    .flatMap(([name, entry]) => {
      const compact = compactEntry(entry)
      return compact ? [[name, compact] as const] : []
    })
  if (providers.length) out.providers = Object.fromEntries(providers)
  const models = Object.entries(body.models ?? {})
    .sort(([a], [b]) => a.localeCompare(b))
    .flatMap(([name, entry]) => {
      const compact = compactEntry(entry)
      return compact ? [[name, compact] as const] : []
    })
  if (models.length) out.models = Object.fromEntries(models)
  return out
}

export function sameBody(a: PolicyBody, b: PolicyBody): boolean {
  return JSON.stringify(normalizeBody(a)) === JSON.stringify(normalizeBody(b))
}

/** Set (or clear with `undefined`) one field of a Provider entry. */
export function withProvider(body: PolicyBody, provider: string, patch: Partial<ProviderPolicy>): PolicyBody {
  const current = { ...(body.providers?.[provider] ?? {}) }
  for (const [key, value] of Object.entries(patch) as [keyof ProviderPolicy, unknown][]) {
    if (value === undefined) delete current[key]
    else (current as Record<string, unknown>)[key] = value
  }
  return { ...body, providers: { ...(body.providers ?? {}), [provider]: current } }
}

/** Set (or clear with `undefined`) one field of a model entry. */
export function withModel(body: PolicyBody, exactModel: string, patch: Partial<ModelPolicy>): PolicyBody {
  const current = { ...(body.models?.[exactModel] ?? {}) }
  for (const [key, value] of Object.entries(patch) as [keyof ModelPolicy, unknown][]) {
    if (value === undefined) delete current[key]
    else (current as Record<string, unknown>)[key] = value
  }
  return { ...body, models: { ...(body.models ?? {}), [exactModel]: current } }
}

/** Every place of a body that sets a coefficient, `''` being the layer default. */
function coefficients(body: PolicyBody): Map<string, number> {
  const out = new Map<string, number>()
  if (body.price_coefficient !== undefined) out.set('', body.price_coefficient)
  for (const [provider, entry] of Object.entries(body.providers ?? {})) {
    if (entry.price_coefficient !== undefined) out.set(provider, entry.price_coefficient)
  }
  return out
}

/** The change turns a free layer or Provider (coefficient 0) into a charged one: unpriced models of its users
 * become unusable (§3.3, Q6), so saving lists them first. */
export function startsCharging(before: PolicyBody | undefined, after: PolicyBody): boolean {
  const old = coefficients(before ?? {})
  for (const [place, value] of coefficients(after)) {
    if (value > 0 && old.get(place) === 0) return true
  }
  return false
}

/** Every field a person may not write on this kind of policy, cleared (App side: no coefficients and no
 * widening; a user's own App policy: only hiding and limits). */
export function restrictBody(body: PolicyBody, mode: EditorMode): PolicyBody {
  if (mode === 'user_side') return body
  const out: PolicyBody = { limits: body.limits }
  out.providers = Object.fromEntries(Object.entries(body.providers ?? {}).map(([name, entry]) => [name, {
    visible: entry.visible === 'all_models' ? undefined : entry.visible,
    limits: entry.limits,
  }]))
  out.models = Object.fromEntries(Object.entries(body.models ?? {}).map(([name, entry]) => {
    const provider = splitExactModel(name).provider
    const selects = out.providers?.[provider]?.visible === 'selected_models'
    return [name, { visible: entry.visible === true && !selects ? undefined : entry.visible, limits: entry.limits }]
  }))
  return normalizeBody(out)
}

/** `user_side`: UserType groups, system groups and users; `app_side`: an administrator's App or user + App
 * policy; `own_app`: a user's own policy on one of their Apps. */
export type EditorMode = 'user_side' | 'app_side' | 'own_app'

export function editorModeOf(subject: string, source: 'admin' | 'user'): EditorMode {
  if (source === 'user') return 'own_app'
  return isAppSide(subject) ? 'app_side' : 'user_side'
}

/** Visibility choices a policy of this mode may store for a Provider (`undefined` = no opinion). */
export function providerVisibilityChoices(mode: EditorMode): (ProviderVisibility | undefined)[] {
  return mode === 'user_side' ? [undefined, 'all_models', 'selected_models', 'hidden'] : [undefined, 'selected_models', 'hidden']
}

export interface BodySummary { allProviders: boolean; coefficient?: number; providers: number; hiddenProviders: number; models: number; limits: number }

export function summarizeBody(body: PolicyBody): BodySummary {
  const providers = Object.values(body.providers ?? {})
  const limits = (body.limits?.length ?? 0) +
    providers.reduce((sum, entry) => sum + (entry.limits?.length ?? 0), 0) +
    Object.values(body.models ?? {}).reduce((sum, entry) => sum + (entry.limits?.length ?? 0), 0)
  return {
    allProviders: body.all_providers === true,
    coefficient: body.price_coefficient,
    providers: providers.filter((entry) => entry.visible === 'all_models' || entry.visible === 'selected_models').length,
    hiddenProviders: providers.filter((entry) => entry.visible === 'hidden').length,
    models: Object.keys(body.models ?? {}).length,
    limits,
  }
}

/** The stored policy of one source and subject (`undefined` when there is none). */
export function policyOf(policies: AiccPolicyView[], subject: string, source: 'admin' | 'user'): AiccPolicyView | undefined {
  return policies.find((view) => view.policy.subject === subject && view.policy.source === source)
}

export function emptyPolicy(subject: string, source: 'admin' | 'user'): AiccPolicy {
  return { policy_id: '', source, subject, enabled: true, body: {}, version: 0, created_by: '', created_at_ms: 0, updated_by: '', updated_at_ms: 0 }
}

// ---------- Exact models ----------

/** `model:variant@provider` → `{ model, variant, provider }` for showing the full identity. */
export function splitExactModel(exactModel: string): { model: string; variant?: string; provider: string } {
  const at = exactModel.lastIndexOf('@')
  const head = at >= 0 ? exactModel.slice(0, at) : exactModel
  const provider = at >= 0 ? exactModel.slice(at + 1) : ''
  const colon = head.indexOf(':')
  return colon >= 0 ? { model: head.slice(0, colon), variant: head.slice(colon + 1), provider } : { model: head, provider }
}

// ---------- Prices ----------

export const PRICE_UNITS = ['request', 'image', 'second', 'video_second', 'audio_second', 'character', 'megapixel', 'reference_image'] as const
export type PriceUnit = (typeof PRICE_UNITS)[number]

export function modelPriceKey(modelDriverId: string, originModelId: string): string {
  return `model:${modelDriverId}:${originModelId}`
}

export function exactPriceKey(exactModel: string): string {
  return `exact:${exactModel}`
}

export function parsePriceKey(key: string): { kind: 'model'; driver: string; origin: string } | { kind: 'exact'; exactModel: string } | null {
  if (key.startsWith('model:')) {
    const rest = key.slice('model:'.length)
    const colon = rest.indexOf(':')
    if (colon <= 0 || colon === rest.length - 1) return null
    return { kind: 'model', driver: rest.slice(0, colon), origin: rest.slice(colon + 1) }
  }
  if (key.startsWith('exact:')) {
    const exactModel = key.slice('exact:'.length)
    return exactModel.includes('@') ? { kind: 'exact', exactModel } : null
  }
  return null
}

/** What the simple price form edits: Credit per million tokens, or Credit per unit. */
export type PriceForm =
  | { mode: 'tokens'; input: string; output: string; cacheInput: string }
  | { mode: 'unit'; unit: PriceUnit; amount: string }

const SIMPLE_TOKEN_KEYS = new Set(['currency', 'input_token', 'output_token', 'cache_input_token', 'rules'])
const SIMPLE_UNIT_KEYS = new Set(['currency', 'unit', 'amount', 'rules'])

function isEmptyValue(value: unknown): boolean {
  return value === undefined || value === null || (Array.isArray(value) && value.length === 0)
}

/** The simple form of a stored price, or `null` when it uses tiers, rules or other dimensions (edited as JSON). */
export function priceForm(pricing: unknown): PriceForm | null {
  if (!pricing || typeof pricing !== 'object' || Array.isArray(pricing)) return null
  const record = pricing as Record<string, unknown>
  const keys = Object.keys(record).filter((key) => !isEmptyValue(record[key]))
  const rate = (key: string) => typeof record[key] === 'number' ? String(record[key]) : ''
  if (typeof record.unit === 'string') {
    if (!keys.every((key) => SIMPLE_UNIT_KEYS.has(key)) || !(PRICE_UNITS as readonly string[]).includes(record.unit)) return null
    return { mode: 'unit', unit: record.unit as PriceUnit, amount: rate('amount') }
  }
  if (!keys.every((key) => SIMPLE_TOKEN_KEYS.has(key))) return null
  return { mode: 'tokens', input: rate('input_token'), output: rate('output_token'), cacheInput: rate('cache_input_token') }
}

function rateValue(text: string): number | null | undefined {
  const trimmed = text.trim()
  if (!trimmed) return undefined
  if (!/^\d+(\.\d+)?$/.test(trimmed)) return null
  const value = Number(trimmed)
  return Number.isFinite(value) ? value : null
}

/** The wire price of a filled form (token rates per million tokens); `null` while a field is invalid or empty. */
export function pricingOfForm(form: PriceForm): Record<string, unknown> | null {
  if (form.mode === 'unit') {
    const amount = rateValue(form.amount)
    return typeof amount === 'number' ? { currency: 'CREDIT', unit: form.unit, amount } : null
  }
  const input = rateValue(form.input)
  const output = rateValue(form.output)
  const cache = rateValue(form.cacheInput)
  if (input === null || output === null || cache === null || (input === undefined && output === undefined)) return null
  return {
    currency: 'CREDIT',
    ...(input !== undefined ? { input_token: input } : {}),
    ...(output !== undefined ? { output_token: output } : {}),
    ...(cache !== undefined ? { cache_input_token: cache } : {}),
  }
}

export function isZeroPricing(pricing: unknown): boolean {
  if (!pricing || typeof pricing !== 'object') return false
  let sawRate = false
  const zero = (value: unknown): boolean => {
    if (Array.isArray(value)) return value.every(zero)
    if (value && typeof value === 'object') {
      return Object.entries(value as Record<string, unknown>).every(([key, item]) => key === 'when' || zero(item))
    }
    if (typeof value === 'number') { sawRate = true; return value === 0 }
    return true
  }
  return zero(pricing) && sawRate
}

/** The rates a person pays for a model, coefficients applied (token rates per million tokens). */
export interface PriceSummary {
  state: ModelCreditPrice['state']
  input?: number
  output?: number
  unit?: string
  amount?: number
  /** Provider cost × a Credit rate: the amount follows the Provider's own price. */
  currency?: string
  rate?: number
  coefficient: number
  /** The price has more than the summary shows (tiers, rules, other dimensions). */
  complex: boolean
}

export function priceSummary(price: ModelCreditPrice): PriceSummary {
  const coefficient = price.provider_coefficient * price.model_coefficient
  const summary: PriceSummary = { state: price.state, coefficient, complex: false }
  if (price.source === 'provider_cost') {
    summary.currency = price.provider_cost_currency
    summary.rate = price.credit_rate === undefined ? undefined : price.credit_rate * coefficient
    return summary
  }
  const form = priceForm(price.pricing)
  const pricing = (price.pricing ?? {}) as Record<string, unknown>
  if (!form) {
    summary.complex = price.pricing !== undefined
    if (typeof pricing.input_token === 'number') summary.input = pricing.input_token * coefficient
    if (typeof pricing.output_token === 'number') summary.output = pricing.output_token * coefficient
    return summary
  }
  if (form.mode === 'unit') {
    summary.unit = form.unit
    summary.amount = Number(form.amount || 0) * coefficient
  } else {
    if (form.input) summary.input = Number(form.input) * coefficient
    if (form.output) summary.output = Number(form.output) * coefficient
  }
  return summary
}

export function formatRate(value: number, locale = 'en'): string {
  return value.toLocaleString(locale, { maximumFractionDigits: value !== 0 && Math.abs(value) < 0.01 ? 6 : 4 })
}

// ---------- Refusals ----------

/** AICC refusal codes (§3.7) the UI explains. */
export const POLICY_REFUSAL_CODES = ['quota_exhausted', 'credit_insufficient', 'quota_unavailable', 'resource_not_permitted', 'attribution_unavailable'] as const
export type PolicyRefusalCode = (typeof POLICY_REFUSAL_CODES)[number]

export type PolicyRefusal =
  | { code: 'quota_exhausted'; details?: QuotaDenialDetails }
  | { code: 'credit_insufficient'; details?: CreditInsufficientDetails }
  | { code: 'resource_not_permitted'; details?: ResourceUnpricedDetails | Record<string, unknown> }
  | { code: 'quota_unavailable' | 'attribution_unavailable'; details?: Record<string, unknown> }

/** The refusal of an AICC call: its wire `code` and details, never its text. */
export function policyRefusalOf(error: unknown): PolicyRefusal | undefined {
  // Thrown kRPC errors carry the `AiccError` in `data`; a failed task carries it directly.
  const e = error as { code?: unknown; data?: unknown; details?: unknown } | null
  if (typeof e?.code !== 'string' || !(POLICY_REFUSAL_CODES as readonly string[]).includes(e.code)) return undefined
  const details = (e.data as { details?: unknown } | undefined)?.details ?? e.details
  return { code: e.code, details: details && typeof details === 'object' ? details : undefined } as PolicyRefusal
}

export function isUnpricedRefusal(refusal: PolicyRefusal | undefined): boolean {
  return refusal?.code === 'resource_not_permitted' && (refusal.details as { reason?: unknown } | undefined)?.reason === 'unpriced'
}

// ---------- Ids ----------

/** An idempotency key generated once per dialog: retries and repeated clicks apply once (§3.4). */
export function newEntryId(prefix: 'rc' | 'adj'): string {
  const random = typeof crypto !== 'undefined' && 'randomUUID' in crypto ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(16).slice(2)}`
  return `${prefix}-${random}`
}

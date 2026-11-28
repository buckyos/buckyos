// The usecase table of the mock runtime (doc/aicc/aicc_router.md §11–§12): the Zone default, each user's layer,
// routing groups and App bindings, evaluated the way the service does where the UI can observe it — the highest
// layer that sets a cell decides unless the Zone default locked it, a layer model that is missing or unsuitable is
// skipped, LLM usecases without a model fall back to the fallback LLM when it is on, and the "when unavailable"
// prediction routes again with the model's Provider treated as disabled. Default models follow the preference of the
// mock routing world for the usecases it simulates, otherwise the first fitting model in Provider order.
import {
  FALLBACK_LLM,
  LLM_USECASES,
  USECASES,
  modelFits,
  type LockedValue,
  type OnUnavailable,
  type RoutingBinding,
  type RoutingGroup,
  type RoutingUsecasesResponse,
  type UsecaseCoverage,
  type UsecaseCoverageChange,
  type UsecaseLayerValue,
  type UsecaseModelState,
  type UsecaseRequirement,
  type UsecaseRoute,
  type UsecaseRoutes,
  type UsecaseRow,
  type UsecaseSetRequest,
} from '../datamodel/usecases'
import type { ModelMetadata, StoreSnapshot } from './types'

export class MockRoutingError extends Error {
  readonly code: string

  constructor(message: string, code = 'invalid_request') {
    super(message)
    this.code = code
  }
}

export interface MockRoutingViewer { user_id: string; admin: boolean }

type Recorder = (level: 'info' | 'warning', kind: string, message: string, details: Record<string, unknown>) => void

/** Requirements of the mock catalog's usecases; no mock model has the context `llm.vision` asks for. */
export const MOCK_USECASE_REQUIREMENTS: Record<string, UsecaseRequirement> = {
  'llm.vision': { vision: true, min_context_tokens: 300000 },
}

/** The default preference of the usecases the mock routing world simulates (its TASKS: levels of provider model ids,
 * the cheaper cost class first within a level). Kept here so the usecase mock never calls the catalog mock, which
 * e2e tests stub with call counting. */
const PREFERENCE: Record<string, string[][]> = {
  'llm.chat': [['gpt-5.1', 'gpt-5.6', 'claude-sonnet-4.5'], ['gpt-5.1-mini'], ['qwen3.5-27b']],
  'llm.code': [['claude-sonnet-4.5'], ['gpt-5.1', 'gpt-5.6'], ['qwen2.5-coder-32b']],
  'llm.plan': [['claude-opus-4.7'], ['gpt-5.1', 'gpt-5.6'], ['qwen2.5-coder-32b']],
}
const COST_RANK: Record<string, number> = { low: 0, medium: 1, high: 2 }

export function usecaseApiType(usecase: string): string {
  if (usecase === 'agent_runtime.computer_use') return 'agent.computer_use'
  return usecase.startsWith('llm.') ? 'llm' : usecase
}

interface Layer { name: string; routes: UsecaseRoutes }
interface Entry { model: ModelMetadata; provider: string }
interface Context {
  snapshot: StoreSnapshot
  layers: Layer[]
  excludedProviders: Set<string>
  excludedModels: Set<string>
}
interface Resolved { model?: string; source?: string; coverage: UsecaseCoverage; reason?: string }

const isUsecase = (usecase: string) => usecase === FALLBACK_LLM || (USECASES as readonly string[]).includes(usecase)

function locked<T>(value: T | LockedValue<T> | undefined): LockedValue<T> | undefined {
  if (value === undefined) return undefined
  return typeof value === 'object' && value !== null && 'value' in value ? { value: value.value, ...(value.locked ? { locked: true } : {}) } : { value: value as T }
}

function applySet(routes: UsecaseRoutes, request: UsecaseSetRequest): UsecaseRoutes {
  const next = structuredClone(routes)
  const route: UsecaseRoute = next[request.usecase] ?? {}
  for (const field of request.clear ?? []) delete route[field]
  const model = locked(request.model)
  const policy = locked(request.on_unavailable)
  if (model) route.model = model
  if (policy) route.on_unavailable = policy
  if (request.enabled !== undefined) route.enabled = request.enabled
  if (!route.model && !route.on_unavailable && route.enabled === undefined) delete next[request.usecase]
  else next[request.usecase] = route
  return next
}

export class MockUsecaseWorld {
  private recorder?: Recorder
  private zone: UsecaseRoutes
  private zoneRevision = 1
  private readonly users = new Map<string, { routes: UsecaseRoutes; version: number }>()
  private readonly groups = new Map<string, { owner: string; group: RoutingGroup }>()
  /** `<owner>\u0000<app_id>` → group id. */
  private readonly bindings = new Map<string, string>()
  private nextGroup = 1

  constructor(populated: boolean) {
    this.zone = populated
      ? {
        'llm.plan': { model: { value: 'gpt-5.1@openai-main', locked: true } },
        'llm.code': { on_unavailable: { value: 'next_model' } },
      }
      : {}
  }

  connect(recorder: Recorder) {
    this.recorder = recorder
  }

  table(snapshot: StoreSnapshot, viewer: MockRoutingViewer, request: { view?: string; without_provider?: string }): RoutingUsecasesResponse {
    const view = request.view ?? 'self'
    const layers: Layer[] = [{ name: 'zone', routes: this.zone }]
    let revision = 0
    let canEdit = true
    if (view === 'zone') {
      if (!viewer.admin) throw new MockRoutingError('only an administrator may view the zone layer', 'no_permission')
      revision = this.zoneRevision
    } else {
      const user = this.users.get(viewer.user_id)
      layers.push({ name: 'user', routes: user?.routes ?? {} })
      if (view === 'self') {
        revision = user?.version ?? 0
      } else {
        let groupId: string | undefined
        if (view.startsWith('group:')) groupId = view.slice('group:'.length)
        else if (view.startsWith('app:')) {
          groupId = this.bindings.get(`${viewer.user_id}\u0000${view.slice('app:'.length)}`)
          canEdit = false
        } else throw new MockRoutingError('view must be self, zone, group:<id> or app:<app_id>')
        if (groupId) {
          const group = this.ownGroup(viewer, groupId)
          revision = group.version
          layers.push({ name: `group:${groupId}`, routes: group.usecases ?? {} })
        }
      }
    }
    const excludedProviders = new Set<string>()
    if (request.without_provider) {
      if (!viewer.admin) throw new MockRoutingError('only an administrator may preview removing a Provider', 'no_permission')
      excludedProviders.add(request.without_provider)
    }
    const context: Context = { snapshot, layers, excludedProviders, excludedModels: new Set() }
    return {
      view,
      revision,
      settings_revision: this.zoneRevision,
      fallback_llm_enabled: this.fallbackEnabled(context),
      can_edit: canEdit,
      rows: [...USECASES, FALLBACK_LLM].map((usecase) => this.row(context, usecase, viewer.admin)),
    }
  }

  set(snapshot: StoreSnapshot, viewer: MockRoutingViewer, request: UsecaseSetRequest): { ok: boolean; revision: number } {
    const zone = request.target === 'zone'
    this.validate(snapshot, zone, request.usecase, { model: locked(request.model), on_unavailable: locked(request.on_unavailable), enabled: request.enabled })
    if (zone) {
      if (!viewer.admin) throw new MockRoutingError('only an administrator may change the Zone default', 'no_permission')
      if (request.revision !== this.zoneRevision) throw new MockRoutingError('settings revision conflict', 'settings_revision_conflict')
      this.zone = applySet(this.zone, request)
      this.zoneRevision += 1
      this.recorder?.('info', 'usecase_updated', `usecase ${request.usecase} changed in the Zone default`, { usecase: request.usecase, settings_revision: this.zoneRevision })
      return { ok: true, revision: this.zoneRevision }
    }
    if (request.target === 'user') {
      const current = this.users.get(viewer.user_id) ?? { routes: {}, version: 0 }
      if (request.revision !== current.version) throw new MockRoutingError('settings revision conflict', 'settings_revision_conflict')
      const next = { routes: applySet(current.routes, request), version: current.version + 1 }
      this.users.set(viewer.user_id, next)
      return { ok: true, revision: next.version }
    }
    if (request.target.startsWith('group:')) {
      const group = this.ownGroup(viewer, request.target.slice('group:'.length))
      if (request.revision !== group.version) throw new MockRoutingError('settings revision conflict', 'settings_revision_conflict')
      group.usecases = applySet(group.usecases ?? {}, request)
      group.version += 1
      group.updated_at_ms = Date.now()
      return { ok: true, revision: group.version }
    }
    throw new MockRoutingError('target must be zone, user or group:<id>')
  }

  listGroups(viewer: MockRoutingViewer): RoutingGroup[] {
    return [...this.groups.values()].filter((entry) => entry.owner === viewer.user_id).map((entry) => this.withApps(entry.owner, entry.group))
  }

  createGroup(snapshot: StoreSnapshot, viewer: MockRoutingViewer, request: { name: string; usecases?: UsecaseRoutes }): RoutingGroup {
    const name = groupName(request.name)
    for (const [usecase, route] of Object.entries(request.usecases ?? {})) this.validate(snapshot, false, usecase, route)
    const group: RoutingGroup = { group_id: `rg${this.nextGroup++}`, name, usecases: request.usecases ?? {}, version: 1, updated_at_ms: Date.now() }
    this.groups.set(group.group_id, { owner: viewer.user_id, group })
    return this.withApps(viewer.user_id, group)
  }

  updateGroup(snapshot: StoreSnapshot, viewer: MockRoutingViewer, request: { group_id: string; version: number; name?: string; usecases?: UsecaseRoutes }): RoutingGroup {
    const group = this.ownGroup(viewer, request.group_id)
    if (request.version !== group.version) throw new MockRoutingError('settings revision conflict', 'settings_revision_conflict')
    if (request.name !== undefined) group.name = groupName(request.name)
    if (request.usecases) {
      for (const [usecase, route] of Object.entries(request.usecases)) this.validate(snapshot, false, usecase, route)
      group.usecases = request.usecases
    }
    group.version += 1
    group.updated_at_ms = Date.now()
    return this.withApps(viewer.user_id, group)
  }

  deleteGroup(viewer: MockRoutingViewer, request: { group_id: string; version: number }): { ok: boolean } {
    const group = this.ownGroup(viewer, request.group_id)
    if (request.version !== group.version) throw new MockRoutingError('settings revision conflict', 'settings_revision_conflict')
    this.groups.delete(group.group_id)
    for (const [key, groupId] of this.bindings) if (groupId === group.group_id) this.bindings.delete(key)
    return { ok: true }
  }

  listBindings(viewer: MockRoutingViewer): RoutingBinding[] {
    return [...this.bindings.entries()]
      .filter(([key]) => key.startsWith(`${viewer.user_id}\u0000`))
      .map(([key, group_id]) => ({ app_id: key.slice(viewer.user_id.length + 1), group_id }))
  }

  setBinding(viewer: MockRoutingViewer, request: RoutingBinding): { ok: boolean } {
    if (!request.app_id.trim()) throw new MockRoutingError('app_id must not be empty')
    this.ownGroup(viewer, request.group_id)
    this.bindings.set(`${viewer.user_id}\u0000${request.app_id}`, request.group_id)
    return { ok: true }
  }

  removeBinding(viewer: MockRoutingViewer, request: { app_id: string }): { ok: boolean } {
    return { ok: this.bindings.delete(`${viewer.user_id}\u0000${request.app_id}`) }
  }

  /** Every usecase of the Zone before and after a Provider is added (`provider.validate`). */
  coverage(before: StoreSnapshot, after: StoreSnapshot): UsecaseCoverageChange[] {
    const layers = [{ name: 'zone', routes: this.zone }]
    const context = (snapshot: StoreSnapshot): Context => ({ snapshot, layers, excludedProviders: new Set(), excludedModels: new Set() })
    return USECASES.map((usecase) => {
      const was = this.resolve(context(before), usecase)
      const will = this.resolve(context(after), usecase)
      return { usecase, before: was.model, before_coverage: was.coverage, after: will.model, after_coverage: will.coverage }
    })
  }

  private ownGroup(viewer: MockRoutingViewer, groupId: string): RoutingGroup {
    const entry = this.groups.get(groupId)
    if (!entry || entry.owner !== viewer.user_id) throw new MockRoutingError('the routing group was not found')
    return entry.group
  }

  private withApps(owner: string, group: RoutingGroup): RoutingGroup {
    const app_ids = [...this.bindings.entries()].filter(([key, id]) => id === group.group_id && key.startsWith(`${owner}\u0000`)).map(([key]) => key.slice(owner.length + 1))
    return { ...group, app_ids }
  }

  private validate(snapshot: StoreSnapshot, zone: boolean, usecase: string, route: UsecaseRoute) {
    if (!isUsecase(usecase)) throw new MockRoutingError(`unknown usecase \`${usecase}\``)
    if ((route.model?.locked || route.on_unavailable?.locked) && !zone) throw new MockRoutingError('only the administrator layer may lock a usecase')
    if (route.enabled !== undefined && (!zone || usecase !== FALLBACK_LLM)) throw new MockRoutingError('enabled only switches the fallback LLM in the administrator layer')
    const exact = route.model?.value
    if (!exact) return
    const entry = this.inventory({ snapshot, layers: [], excludedProviders: new Set(), excludedModels: new Set() }).find((item) => item.model.exact_model === exact)
    if (!entry) throw new MockRoutingError(`exact model \`${exact}\` is not in any inventory`)
    if (!(entry.model.api_types as string[]).includes(usecaseApiType(usecase))) throw new MockRoutingError(`\`${exact}\` does not serve ${usecase}`)
    if (usecase !== FALLBACK_LLM && !modelFits(entry.model, usecaseApiType(usecase), snapshot.usecaseRequirements[usecase])) {
      throw new MockRoutingError(`\`${exact}\` does not meet ${usecase}`)
    }
  }

  private inventory(context: Context): Entry[] {
    const entries: Entry[] = []
    for (const provider of context.snapshot.providers) {
      const name = provider.config.provider_instance_name
      if (!provider.config.enabled || context.excludedProviders.has(name)) continue
      for (const model of provider.inventory.models) entries.push({ model, provider: name })
    }
    if (!context.excludedProviders.has('local')) for (const model of context.snapshot.localModels) entries.push({ model, provider: 'local' })
    return entries
  }

  private fits(context: Context, usecase: string, model: ModelMetadata): boolean {
    return modelFits(model, usecaseApiType(usecase), usecase === FALLBACK_LLM ? undefined : context.snapshot.usecaseRequirements[usecase])
  }

  private modelState(context: Context, usecase: string, exact: string): UsecaseModelState {
    const entry = this.inventory(context).find((item) => item.model.exact_model === exact)
    if (!entry) return 'missing'
    return this.fits(context, usecase, entry.model) ? 'available' : 'unsuitable'
  }

  private fallbackEnabled(context: Context): boolean {
    return context.layers[0]?.routes[FALLBACK_LLM]?.enabled === true
  }

  private defaultModel(context: Context, usecase: string): string | undefined {
    const entries = this.inventory(context).filter((entry) => !context.excludedModels.has(entry.model.exact_model) && this.fits(context, usecase, entry.model))
    if (entries.length === 0) return undefined
    if (usecase === FALLBACK_LLM) return (entries.find((entry) => entry.model.logical_mounts.includes(FALLBACK_LLM)) ?? entries[0]).model.exact_model
    for (const level of PREFERENCE[usecase] ?? []) {
      const pool = entries
        .filter((entry) => level.includes(entry.model.provider_model_id))
        .sort((left, right) => (COST_RANK[left.model.attributes.cost_class] ?? 3) - (COST_RANK[right.model.attributes.cost_class] ?? 3))
      if (pool[0]) return pool[0].model.exact_model
    }
    return entries[0].model.exact_model
  }

  private cells(context: Context, usecase: string) {
    const zone = context.layers[0]?.routes[usecase]
    const descending = [...context.layers].reverse()
    let model: string | undefined
    let modelSource: string | undefined
    for (const layer of zone?.model?.locked ? context.layers.slice(0, 1) : descending) {
      const value = layer.routes[usecase]?.model?.value
      if (!value || context.excludedModels.has(value) || this.modelState(context, usecase, value) !== 'available') continue
      model = value
      modelSource = layer.name
      break
    }
    let policy: OnUnavailable = 'none'
    let policySource = 'default'
    for (const layer of zone?.on_unavailable?.locked ? context.layers.slice(0, 1) : descending) {
      const value = layer.routes[usecase]?.on_unavailable?.value
      if (!value) continue
      policy = value
      policySource = layer.name
      break
    }
    return { model, modelSource, policy, policySource, modelLocked: zone?.model?.locked === true, policyLocked: zone?.on_unavailable?.locked === true }
  }

  private resolve(context: Context, usecase: string): Resolved {
    const cells = this.cells(context, usecase)
    if (cells.model) return { model: cells.model, source: cells.modelSource, coverage: 'covered' }
    const model = this.defaultModel(context, usecase)
    if (model) return { model, source: 'default', coverage: 'covered' }
    if (LLM_USECASES.includes(usecase) && this.fallbackEnabled(context)) {
      const fallback = this.resolve(context, FALLBACK_LLM)
      if (fallback.model) return { model: fallback.model, source: 'fallback_llm', coverage: 'fallback_llm' }
    }
    const apiType = usecaseApiType(usecase)
    const any = this.inventory(context).some((entry) => (entry.model.api_types as string[]).includes(apiType))
    return { coverage: 'uncovered', reason: any ? 'requirements_unmet' : 'no_model_for_api_type' }
  }

  private backup(context: Context, usecase: string, resolved: Resolved, policy: OnUnavailable): { to?: string; note?: string } {
    if (policy === 'none' || !resolved.model || resolved.coverage !== 'covered') return {}
    const current = this.inventory(context).find((entry) => entry.model.exact_model === resolved.model)
    if (!current) return {}
    const without: Context = { ...context, excludedProviders: new Set([...context.excludedProviders, current.provider]), excludedModels: new Set(context.excludedModels) }
    if (policy === 'same_model') {
      const weights = context.snapshot.routingView.provider_weights
      const same = this.inventory(without)
        .filter((entry) => entry.model.provider_model_id === current.model.provider_model_id && this.fits(context, usecase, entry.model))
        .sort((left, right) => (weights[right.provider] ?? 1) - (weights[left.provider] ?? 1))
      return same[0] ? { to: same[0].model.exact_model } : { note: 'no_other_model' }
    }
    if (policy === 'next_model') {
      const next = this.resolve(without, usecase)
      return next.coverage === 'covered' && next.model ? { to: next.model } : { note: 'no_other_model' }
    }
    const price = current.model.pricing
    for (let attempt = 0; attempt < 64; attempt += 1) {
      const next = this.resolve(without, usecase)
      if (next.coverage !== 'covered' || !next.model) break
      const candidate = this.inventory(without).find((entry) => entry.model.exact_model === next.model)?.model.pricing
      if (candidate && notPricier(candidate, price)) return { to: next.model }
      without.excludedModels.add(next.model)
    }
    return { note: 'no_known_cheaper_model' }
  }

  private row(context: Context, usecase: string, admin: boolean): UsecaseRow {
    const cells = this.cells(context, usecase)
    const resolved = this.resolve(context, usecase)
    const layers: UsecaseLayerValue[] = context.layers.flatMap((layer) => {
      const route = layer.routes[usecase]
      if (!route) return []
      return [{
        layer: layer.name,
        ...(route.model ? { model: route.model.value, model_state: this.modelState(context, usecase, route.model.value) } : {}),
        ...(route.model?.locked ? { model_locked: true } : {}),
        ...(route.on_unavailable ? { on_unavailable: route.on_unavailable.value } : {}),
        ...(route.on_unavailable?.locked ? { on_unavailable_locked: true } : {}),
        ...(route.enabled !== undefined ? { enabled: route.enabled } : {}),
      }]
    })
    const backup = this.backup(context, usecase, resolved, cells.policy)
    return {
      usecase,
      api_type: usecaseApiType(usecase),
      model: resolved.model,
      model_source: resolved.source,
      on_unavailable: cells.policy,
      on_unavailable_source: cells.policySource,
      ...(cells.modelLocked ? { model_locked: true } : {}),
      ...(cells.policyLocked ? { on_unavailable_locked: true } : {}),
      layers,
      fallback_to: backup.to,
      fallback_note: backup.note,
      coverage: resolved.coverage,
      coverage_reason: resolved.reason,
      detail: admin && resolved.coverage === 'uncovered' ? `no candidate for ${usecase}` : undefined,
      enabled: usecase === FALLBACK_LLM ? this.fallbackEnabled(context) : undefined,
    }
  }
}

function notPricier(candidate: ModelMetadata['pricing'], current: ModelMetadata['pricing']): boolean {
  const keys = ['input_token_usd', 'output_token_usd'] as const
  return keys.every((key) => candidate[key] !== undefined && current[key] !== undefined && candidate[key]! <= current[key]!)
}

function groupName(name: string): string {
  const trimmed = name.trim()
  if (!trimmed || [...trimmed].length > 64) throw new MockRoutingError('a group name has 1..64 characters')
  return trimmed
}

let world: MockUsecaseWorld | null = null

/** One usecase world per page, shared by the AICC mock (Provider validation) and every routing API instance. */
export function mockUsecaseWorld(): MockUsecaseWorld {
  const params = new URLSearchParams(globalThis.location?.search ?? '')
  world ??= new MockUsecaseWorld((params.get('aiccScenario') ?? params.get('scenario')) === 'populated')
  return world
}

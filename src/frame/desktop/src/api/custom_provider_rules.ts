import type { ModelCatalog } from '../app/ai-center/datamodel/model-catalog'
import { parseProviderAdvancedRules } from './provider_model_selection'

export type RuleObject = Record<string, unknown>
export interface CustomRulesDraft {
  provider_rules_id: string
  instance_rules: RuleObject
  modelVariants: Record<string, unknown>
}
export interface ModelRuleDraft {
  alias: string
  routes: string
  operations: string
  variants: string
  pricing: string
  request_rules: string
  disabled_api_types: string
  scoped_request_rules: Record<string, string>
  effort_options: Record<string, string>
}
export type RuleCategory = 'aliases' | 'routes' | 'operations' | 'variants' | 'pricing'

export const ruleObject = (value: unknown): RuleObject =>
  value && typeof value === 'object' && !Array.isArray(value) ? value as RuleObject : {}
export const ruleArray = (value: unknown): RuleObject[] =>
  Array.isArray(value) ? value.map(ruleObject) : []
export const rulesJson = (value: unknown) => JSON.stringify(value, null, 2)
const emptyCatalog = () => ({
  format: 'buckyos.aicc.provider-rules-catalog', schema_version: 1, schema_revision: 1,
  revision_seq: 1, provider_profile_id: 'custom', models: [], patterns: [{ match: '*' }],
  variants: [], model_pricing: [], model_driver_overrides: {},
})

export function createCustomRulesDraft(config: {
  provider_rules_id?: string
  instance_rules?: RuleObject
  rules_catalog?: RuleObject
  resolved_model_rules?: Record<string, RuleObject>
}): CustomRulesDraft {
  const instance = structuredClone(config.instance_rules ?? {})
  const boundCatalog = config.provider_rules_id ? config.rules_catalog : undefined
  instance.catalog = structuredClone(instance.catalog ?? boundCatalog ?? emptyCatalog())
  instance.model_driver_overrides = {
    ...ruleObject(ruleObject(instance.catalog).model_driver_overrides),
    ...ruleObject(instance.model_driver_overrides),
  }
  instance.server_default_routes = {
    ...ruleObject(ruleObject(instance.catalog).server_default_routes),
    ...ruleObject(instance.server_default_routes),
  }
  instance.exclude_models ??= []
  const modelVariants = ruleObject(ruleObject(instance.catalog).model_variants)
  delete ruleObject(instance.catalog).model_variants
  return {
    modelVariants,
    provider_rules_id: config.provider_rules_id ?? '', instance_rules: instance,
  }
}

function variantsForModel(draft: CustomRulesDraft, id: string): unknown {
  const catalog = ruleObject(draft.instance_rules.catalog)
  const overrides = draft.modelVariants
  if (id in overrides) return overrides[id]
  const variants = ruleArray(catalog.variants).filter((variant) => ruleObject(variant.match).provider_model_id === id)
  return variants.length ? variants : undefined
}

function narrowVariantMatch(rule: RuleObject, ids: Set<string>): RuleObject | null {
  const match: RuleObject = typeof rule.match === 'string' ? { provider_model_id: rule.match } : { ...ruleObject(rule.match) }
  const condition = match.provider_model_id
  if (condition === undefined || condition === '*' || ruleObject(condition).exists === true) {
    match.provider_model_id = { not: [...ids] }
  } else if (typeof condition === 'string' || Array.isArray(condition)) {
    const values = typeof condition === 'string' ? [condition] : condition
    if (values.some((value) => typeof value !== 'string' || /[*?[\]\\]/.test(value))) {
      throw new Error('This variant has a conditional model pattern. Narrow its provider_model_id match in the provider JSON before editing individual efforts.')
    }
    const remaining = values.filter((value) => !ids.has(String(value)))
    if (!remaining.length) return null
    match.provider_model_id = typeof condition === 'string' ? remaining[0] : remaining
  } else if ('not' in ruleObject(condition)) {
    const excluded = ruleObject(condition).not
    match.provider_model_id = { not: [...new Set([...(Array.isArray(excluded) ? excluded : [excluded]), ...ids])] }
  } else if (ruleObject(condition).exists !== false) {
    throw new Error('Edit this variant model condition in the provider JSON before changing individual efforts.')
  }
  return { ...rule, match }
}

export function providerRulesFile(draft: CustomRulesDraft): RuleObject {
  const instance = draft.instance_rules
  const catalog = structuredClone(ruleObject(instance.catalog))
  const aliases = { ...ruleObject(catalog.model_driver_overrides), ...ruleObject(instance.model_driver_overrides) }
  if (catalog.model_driver_overrides !== undefined || Object.keys(aliases).length) catalog.model_driver_overrides = aliases
  const routes = ruleObject(instance.server_default_routes ?? catalog.server_default_routes)
  if (catalog.server_default_routes !== undefined || Object.keys(routes).length) catalog.server_default_routes = structuredClone(routes)
  const models = ruleArray(catalog.models)
  for (const id of (instance.exclude_models ?? []) as string[]) {
    const rule = models.find((rule) => rule.id === id)
    if (rule) rule.exclude = true
    else models.push({ id, exclude: true })
  }
  if (catalog.models !== undefined || models.length) catalog.models = models
  const overrides = draft.modelVariants
  const ids = new Set(Object.keys(overrides))
  if (ids.size) {
    catalog.variants = [
      ...ruleArray(catalog.variants).map((rule) => narrowVariantMatch(rule, ids)).filter((rule) => rule !== null),
      ...Object.entries(overrides).flatMap(([id, variants]) => ruleArray(variants).map((variant) => ({
        ...variant, match: { ...ruleObject(variant.match), provider_model_id: id },
      }))),
    ]
  }
  delete catalog.model_variants
  return catalog
}

export function parseProviderRulesFile(source: string, base?: CustomRulesDraft): CustomRulesDraft {
  const catalog = ruleObject(JSON.parse(source))
  const fields = ['format', 'schema_version', 'schema_revision', 'revision_seq', 'provider_profile_id',
    'reported_cost', 'custom_provider_adapters', 'static_inventory_models', 'model_driver_overrides',
    'server_default_routes', 'supplemental_inventory_api_types', 'models', 'patterns', 'access_rules',
    'model_pricing', 'variants']
  if (catalog.format !== 'buckyos.aicc.provider-rules-catalog' || catalog.schema_version !== 1
    || !Number.isInteger(catalog.schema_revision) || ![0, 1].includes(Number(catalog.schema_revision))
    || !Number.isSafeInteger(catalog.revision_seq) || Number(catalog.revision_seq) < 0
    || typeof catalog.provider_profile_id !== 'string' || !catalog.provider_profile_id.trim()
    || Object.keys(catalog).some((key) => !fields.includes(key))) {
    throw new Error('Invalid .provider.json rules catalog')
  }
  for (const key of ['models', 'patterns', 'variants', 'model_pricing', 'access_rules', 'custom_provider_adapters', 'static_inventory_models', 'supplemental_inventory_api_types']) {
    if (catalog[key] !== undefined && !Array.isArray(catalog[key])) throw new Error(`${key} must be an array`)
  }
  for (const key of ['model_driver_overrides', 'server_default_routes']) {
    const value = catalog[key]
    if (value !== undefined && (!value || typeof value !== 'object' || Array.isArray(value))) throw new Error(`${key} must be an object`)
  }
  parseProviderAdvancedRules({
    model_driver_overrides_json: rulesJson(catalog.model_driver_overrides ?? {}),
    server_default_routes_json: rulesJson(catalog.server_default_routes ?? {}),
  })
  return createCustomRulesDraft({
    provider_rules_id: base?.provider_rules_id,
    instance_rules: {
      ...structuredClone(base?.instance_rules ?? {}), catalog,
      model_driver_overrides: structuredClone(catalog.model_driver_overrides ?? {}),
      server_default_routes: structuredClone(catalog.server_default_routes ?? {}), exclude_models: [],
    },
  })
}

export function modelRuleDraft(file: CustomRulesDraft, id: string, resolved: RuleObject = {}): ModelRuleDraft {
  const instance = file.instance_rules
  const catalog = ruleObject(instance.catalog)
  const exact = ruleArray(catalog.models).find((rule) => rule.id === id)
  const price = ruleArray(catalog.model_pricing).find((rule) => rule.id === id)
  return {
    alias: String(ruleObject(instance.model_driver_overrides)[id] ?? ruleObject(catalog.model_driver_overrides)[id] ?? ''),
    routes: ((ruleObject(instance.server_default_routes)[id] ?? []) as string[]).join(', '),
    operations: rulesJson(exact?.operations ?? ruleObject(resolved.rule).operations ?? {}),
    request_rules: rulesJson(exact?.request_rules ?? ruleObject(resolved.rule).request_rules ?? []),
    disabled_api_types: rulesJson(exact?.remove_api_types ?? ruleObject(resolved.rule).remove_api_types ?? []),
    variants: rulesJson(id in file.modelVariants ? file.modelVariants[id] : resolved.variants ?? variantsForModel(file, id) ?? []),
    pricing: rulesJson(price?.pricing ?? resolved.pricing ?? null),
    scoped_request_rules: {},
    effort_options: {},
  }
}

export function applyModelRuleEdits(file: CustomRulesDraft, edits: Record<string, Partial<ModelRuleDraft>>, resolved: Record<string, RuleObject>): CustomRulesDraft {
  const next = structuredClone(file)
  const instance = next.instance_rules
  const catalog = ruleObject(instance.catalog)
  const aliases = { ...ruleObject(instance.model_driver_overrides) }
  const catalogAliases = { ...ruleObject(catalog.model_driver_overrides) }
  const routes = { ...ruleObject(instance.server_default_routes) }
  const catalogRoutes = { ...ruleObject(catalog.server_default_routes) }
  let models = ruleArray(catalog.models)
  let prices = ruleArray(catalog.model_pricing)
  const variants = { ...next.modelVariants }
  for (const [id, changes] of Object.entries(edits)) {
    try {
      if (changes.alias !== undefined) {
        delete aliases[id]
        delete catalogAliases[id]
        if (changes.alias.trim()) aliases[id] = changes.alias.trim()
      }
      if (changes.routes !== undefined) {
        const tasks = changes.routes.split(',').map((task) => task.trim()).filter(Boolean)
        delete routes[id]
        delete catalogRoutes[id]
        if (tasks.length) routes[id] = [...new Set(tasks)]
      }
      if (changes.operations !== undefined || changes.request_rules !== undefined || changes.disabled_api_types !== undefined || changes.scoped_request_rules !== undefined) {
        const old = models.find((rule) => rule.id === id) ?? ruleObject(resolved[id]?.rule)
        const rule = { ...old, id }
        if (changes.operations !== undefined) {
          const operations: unknown = JSON.parse(changes.operations)
          if (!operations || typeof operations !== 'object' || Array.isArray(operations)
            || Object.values(operations).some((value) => typeof value !== 'string')) throw new Error('Invalid operation mapping')
          Object.assign(rule, { operations })
        }
        if (changes.request_rules !== undefined) {
          const request_rules: unknown = JSON.parse(changes.request_rules)
          if (!Array.isArray(request_rules)) throw new Error('Request rules must be an array')
          Object.assign(rule, { request_rules })
        }
        if (changes.disabled_api_types !== undefined) {
          const remove_api_types: unknown = JSON.parse(changes.disabled_api_types)
          if (!Array.isArray(remove_api_types) || remove_api_types.some((type) => typeof type !== 'string')) throw new Error('Disabled API types must be a string array')
          Object.assign(rule, { remove_api_types })
        }
        if (changes.scoped_request_rules) {
          let source = rulesJson(ruleObject(rule).request_rules ?? [])
          for (const [apiType, text] of Object.entries(changes.scoped_request_rules)) {
            const scoped: unknown = JSON.parse(text)
            if (!Array.isArray(scoped) || scoped.some((item) => !item || typeof item !== 'object' || Array.isArray(item))) throw new Error('Request rules must be an object array')
            source = replaceScopedRequestRules(source, apiType, scoped)
          }
          Object.assign(rule, { request_rules: JSON.parse(source) })
        }
        models = models.filter((rule) => rule.id !== id)
        models.push(rule)
      }
      if (changes.variants !== undefined || changes.effort_options !== undefined) {
        const value: unknown = JSON.parse(changes.variants ?? modelRuleDraft(next, id, resolved[id]).variants)
        if (!Array.isArray(value)) throw new Error('Variants must be an array')
        variants[id] = value.map((item) => {
          const variant = ruleObject(item)
          const effort = String(variant.variant).replace(/^reasoning-/, '')
          const text = changes.effort_options?.[effort]
          if (text === undefined) return variant
          const options: unknown = JSON.parse(text)
          if (!options || typeof options !== 'object' || Array.isArray(options)) throw new Error('provider_options must be an object')
          return { ...variant, provider_options: options }
        })
      }
      if (changes.pricing !== undefined) {
        const value: unknown = JSON.parse(changes.pricing)
        if (value !== null && (typeof value !== 'object' || Array.isArray(value))) throw new Error('Pricing must be an object or null')
        prices = prices.filter((rule) => rule.id !== id)
        if (value !== null) prices.push({ id, pricing: value })
      }
    } catch (error) {
      throw new Error(`${id}: ${error instanceof Error ? error.message : String(error)}`)
    }
  }
  instance.model_driver_overrides = aliases
  instance.server_default_routes = routes
  if (catalog.model_driver_overrides !== undefined || Object.keys(catalogAliases).length) catalog.model_driver_overrides = catalogAliases
  if (catalog.server_default_routes !== undefined || Object.keys(catalogRoutes).length) catalog.server_default_routes = catalogRoutes
  if (catalog.models !== undefined || models.length) catalog.models = models
  next.modelVariants = variants
  if (catalog.model_pricing !== undefined || prices.length) catalog.model_pricing = prices
  return next
}

export function previewRules(file: CustomRulesDraft, category: RuleCategory): unknown {
  const catalog = ruleObject(file.instance_rules.catalog)
  switch (category) {
    case 'aliases': return { ...ruleObject(catalog.model_driver_overrides), ...ruleObject(file.instance_rules.model_driver_overrides) }
    case 'routes': return file.instance_rules.server_default_routes ?? {}
    case 'operations': return {
      models: ruleArray(catalog.models).map(({ id, operations }) => ({ id, operations: operations ?? {} })),
      patterns: ruleArray(catalog.patterns).map((rule) => ({ match: rule.match, operations: rule.operations ?? {} })),
    }
    case 'variants': return providerRulesFile(file).variants ?? []
    case 'pricing': return catalog.model_pricing ?? []
  }
}

export function paginateRuleModels(ids: string[], excluded: string[], query: string, page: number, size: number) {
  const disabled = new Set(excluded)
  const needle = query.trim().toLocaleLowerCase()
  const matches = [...new Set(ids)].filter((id) => id.toLocaleLowerCase().includes(needle))
    .sort((a, b) => Number(disabled.has(a)) - Number(disabled.has(b)) || a.localeCompare(b))
  const pages = Math.max(1, Math.ceil(matches.length / size))
  const current = Math.max(0, Math.min(page, pages - 1))
  return { total: matches.length, pages, page: current, models: matches.slice(current * size, (current + 1) * size) }
}

export function disabledRuleModels(file: CustomRulesDraft, resolved: Record<string, RuleObject>): string[] {
  const exact = ruleArray(ruleObject(file.instance_rules.catalog).models)
  const explicitlyEnabled = new Set(exact.filter((rule) => rule.exclude === false).map((rule) => rule.id))
  return [...new Set([
    ...((file.instance_rules.exclude_models ?? []) as string[]),
    ...exact.filter((rule) => rule.exclude === true).map((rule) => String(rule.id)),
    ...Object.entries(resolved).filter(([id, value]) => !explicitlyEnabled.has(id) && ruleObject(value.rule).exclude === true).map(([id]) => id),
  ])]
}

export function setRuleModelEnabled(file: CustomRulesDraft, id: string, enabled: boolean, resolved: Record<string, RuleObject>): CustomRulesDraft {
  const next = structuredClone(file)
  const excluded = new Set((next.instance_rules.exclude_models ?? []) as string[])
  if (enabled) excluded.delete(id); else excluded.add(id)
  next.instance_rules.exclude_models = [...excluded]
  if (enabled) {
    const catalog = ruleObject(next.instance_rules.catalog)
    const models = ruleArray(catalog.models)
    const current = models.find((rule) => rule.id === id)
    if (current) current.exclude = false
    else if (ruleObject(resolved[id]?.rule).exclude === true) models.push({ ...ruleObject(resolved[id]?.rule), id, exclude: false })
    catalog.models = models
  }
  return next
}

export interface RuleModelDefinition {
  target: string
  driver: string
  id: string
  apiTypes: string[]
  efforts: string[]
  directOnly: boolean
}
export interface RuleAdapterDefinition {
  id: string
  operations: { id: string; apiTypes: string[] }[]
  defaults: Record<string, string>
}
export interface ProviderRuleOptions {
  models: RuleModelDefinition[]
  tasks: string[]
  adapters: RuleAdapterDefinition[]
}

export function toProviderRuleOptions(catalog: ModelCatalog, tasks: unknown[], adapters: unknown[]): ProviderRuleOptions {
  return {
    models: catalog.vendors.flatMap((vendor) => vendor.models.map((model) => ({
      target: `${vendor.id}/${model.id}`, driver: vendor.id, id: model.id,
      apiTypes: model.metadata.api_types ?? [],
      efforts: model.metadata.llm?.supported_efforts ?? [],
      directOnly: vendor.specs.some((spec) => spec.direct_only && (model.metadata.llm?.spec === spec.id
        || model.metadata.llm?.additional_specs?.some((binding) => binding.spec === spec.id))),
    }))),
    tasks: [...new Set(tasks.map(ruleObject).filter((task) => task.kind === 'task' && (task.api_type ?? task.apiType) === 'llm').map((task) => String(task.path)).filter((path) => /^llm\.[a-z0-9_.-]+$/.test(path)))].sort(),
    adapters: adapters.map(ruleObject).map((adapter) => ({
      id: String(adapter.protocol_adapter_id),
      defaults: ruleObject(adapter.default_operations) as Record<string, string>,
      operations: ruleArray(adapter.operations).map((operation) => ({
        id: String(operation.operation_id), apiTypes: (operation.api_types ?? []) as string[],
      })),
    })),
  }
}

export function ruleModelDefinition(id: string, row: ModelRuleDraft, resolved: RuleObject, options: ProviderRuleOptions): RuleModelDefinition | undefined {
  const target = row.alias.trim() || (resolved.model_driver && resolved.origin_model_id ? `${resolved.model_driver}/${resolved.origin_model_id}` : '')
  const exact = options.models.find((model) => model.target === target || (!target && model.id === id))
  if (exact) return exact
  const actualTarget = `${resolved.model_driver}/${resolved.origin_model_id}`
  if (target !== actualTarget || !resolved.model_metadata) return undefined
  const metadata = ruleObject(resolved.model_metadata)
  const llm = ruleObject(metadata.llm)
  return {
    target, driver: String(resolved.model_driver), id: String(resolved.origin_model_id),
    apiTypes: (metadata.api_types ?? []) as string[], efforts: (llm.supported_efforts ?? []) as string[],
    directOnly: metadata.direct_only === true,
  }
}

export function scopedRequestRules(source: string, apiType: string): RuleObject[] {
  return ruleArray(JSON.parse(source)).filter((rule) => {
    const when = ruleObject(rule.when)
    return when.api_type === apiType || Array.isArray(when.api_type) && when.api_type.includes(apiType)
  })
}

export function replaceScopedRequestRules(source: string, apiType: string, rules: RuleObject[]): string {
  const remaining = ruleArray(JSON.parse(source)).flatMap((rule) => {
    const when = ruleObject(rule.when)
    if (when.api_type === apiType) return []
    if (Array.isArray(when.api_type) && when.api_type.includes(apiType)) {
      const api_types = when.api_type.filter((type) => type !== apiType)
      return api_types.length ? [{ ...rule, when: { ...when, api_type: api_types } }] : []
    }
    return [rule]
  })
  return rulesJson([...remaining, ...rules.map((rule) => ({ ...rule, when: { ...ruleObject(rule.when), api_type: [apiType] } }))])
}

export function validateRuleEditor(file: CustomRulesDraft, edits: Record<string, Partial<ModelRuleDraft>>, resolved: Record<string, RuleObject>, options: ProviderRuleOptions, adapterId: string): void {
  const adapter = options.adapters.find((adapter) => adapter.id === adapterId)
  for (const id of Object.keys(edits)) {
    const row = modelRuleDraft(file, id, resolved[id])
    const metadata = ruleModelDefinition(id, row, resolved[id] ?? {}, options)
    const operations = ruleObject(JSON.parse(row.operations))
    const variants = ruleArray(JSON.parse(row.variants))
    const routes = row.routes.split(',').map((route) => route.trim()).filter(Boolean)
    if (routes.some((route) => !options.tasks.includes(route))) throw new Error(`${id}: unknown LLM task`)
    if (row.alias && !metadata) throw new Error(`${id}: select a recognized built-in model`)
    if ((!metadata || metadata.directOnly || !metadata.apiTypes.includes('llm')) && routes.length) throw new Error(`${id}: this model cannot use server-default tasks`)
    for (const [apiType, operation] of Object.entries(operations)) {
      if (!metadata?.apiTypes.includes(apiType) || !adapter?.operations.some((item) => item.id === operation && item.apiTypes.includes(apiType))) {
        throw new Error(`${id}: unsupported protocol mapping for ${apiType}`)
      }
    }
    if (new Set(variants.map((variant) => variant.variant)).size !== variants.length) throw new Error(`${id}: duplicate thinking effort mapping`)
    for (const variant of variants) {
      const effort = String(variant.variant).replace(/^reasoning-/, '')
      if (!metadata?.efforts.includes(effort) || effort === 'native' || variant.model_driver !== metadata.driver) throw new Error(`${id}: unsupported thinking effort ${effort}`)
      if (!variant.provider_options || typeof variant.provider_options !== 'object' || Array.isArray(variant.provider_options)) throw new Error(`${id}: provider_options must be an object`)
    }
  }
}

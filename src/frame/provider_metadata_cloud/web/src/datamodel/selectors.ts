import type {
  DictionaryItem,
  LogicalDirectoryRecord,
  MatchType,
  MetadataVariantRecord,
  MetadataVersionRuleRecord,
  ModelParamRuleRecord,
  ProviderCloudSeed,
  ProviderRecord,
  DriverMetadataDocument,
  DriverMetadataRule,
  PublishPreview,
  WarningRecord,
  OpsOverlayRecord,
  OpsBulkOperationPreview,
  OpsBulkPreviewRow,
} from './types'
import type { OpsBulkOperationInput } from './schemas'

export function getProviderModelCount(seed: ProviderCloudSeed, providerKey: string) {
  const ownRuleKeys = new Set(seed.model_param_rules
    .filter((rule) => rule.enabled && (rule.provider_key === providerKey || (rule.provider_key === null && rule.original_provider === providerKey)))
    .map((rule) => rule.rule_key))
  seed.provider_model_rules
    .filter((rule) => rule.enabled && rule.provider_key === providerKey)
    .forEach((selection) => {
      seed.model_param_rules
        .filter((rule) => {
          if (!rule.enabled || !rule.model_id_selector) {
            return false
          }
          if (selection.rule_type === 'include_origin') {
            return rule.original_provider === selection.selector
          }
          if (selection.rule_type === 'include_pattern') {
            return wildcardMatch(selection.selector, rule.model_id_selector)
          }
          return false
        })
        .forEach((rule) => ownRuleKeys.add(rule.rule_key))
    })
  return ownRuleKeys.size
}

export function getProviderWarnings(seed: ProviderCloudSeed, providerKey: string) {
  return seed.warnings.filter((warning) => warning.target_key === providerKey)
}

export function getOpsOverlay(
  seed: ProviderCloudSeed,
  targetType: OpsOverlayRecord['target_type'],
  targetKey: string,
) {
  return seed.ops_overlays.find((overlay) => overlay.target_type === targetType && overlay.target_key === targetKey) ?? null
}

export function getOpsPatchValue<T>(overlay: OpsOverlayRecord | null, key: string, fallback: T): T {
  const value = overlay?.ops_patch[key]
  return value === undefined ? fallback : value as T
}

export function buildOpsProviderPreview(seed: ProviderCloudSeed, provider: ProviderRecord) {
  const overlay = getOpsOverlay(seed, 'provider', provider.provider_key)
  return {
    provider_key: provider.provider_key,
    name: provider.name,
    visible: provider.enabled && !overlay?.disabled,
    recommendation_level: getOpsPatchValue(overlay, 'recommendation_level', 'standard'),
    display_priority: getOpsPatchValue(overlay, 'display_priority', 100),
    routing_policy_tag: getOpsPatchValue(overlay, 'routing_policy_tag', ''),
  }
}

export function buildOpsModelPreview(seed: ProviderCloudSeed, rule: ModelParamRuleRecord) {
  const overlay = getOpsOverlay(seed, 'model_param_rule', rule.rule_key)
  return {
    rule_key: rule.rule_key,
    model_id_selector: rule.model_id_selector,
    visible: rule.enabled,
    routing_weight: getOpsPatchValue(overlay, 'routing_weight', 50),
    recommendation_level: getOpsPatchValue(overlay, 'recommendation_level', 'standard'),
    display_priority: getOpsPatchValue(overlay, 'display_priority', 100),
    pricing_override: getOpsPatchValue(overlay, 'pricing_override', null),
  }
}

function getBasePricing(rule: ModelParamRuleRecord) {
  const estimatedCost = typeof rule.pricing?.estimated_cost_usd === 'number' ? rule.pricing.estimated_cost_usd : null
  if (estimatedCost === null) {
    return null
  }
  return {
    input: estimatedCost,
    output: estimatedCost * 2,
  }
}

function getEffectivePricing(seed: ProviderCloudSeed, rule: ModelParamRuleRecord) {
  const overlay = getOpsOverlay(seed, 'model_param_rule', rule.rule_key)
  return getOpsPatchValue<{ input: number; output: number } | null>(overlay, 'pricing_override', null) ?? getBasePricing(rule)
}

function getEffectiveRoutingWeight(seed: ProviderCloudSeed, rule: ModelParamRuleRecord) {
  const overlay = getOpsOverlay(seed, 'model_param_rule', rule.rule_key)
  return getOpsPatchValue(overlay, 'routing_weight', 50)
}

function getEffectiveRecommendation(seed: ProviderCloudSeed, rule: ModelParamRuleRecord) {
  const overlay = getOpsOverlay(seed, 'model_param_rule', rule.rule_key)
  return getOpsPatchValue(overlay, 'recommendation_level', 'standard')
}

function getEffectiveDisplayPriority(seed: ProviderCloudSeed, rule: ModelParamRuleRecord) {
  const overlay = getOpsOverlay(seed, 'model_param_rule', rule.rule_key)
  return getOpsPatchValue(overlay, 'display_priority', 100)
}

export function getDictionaryKeys(seed: ProviderCloudSeed, kind: DictionaryItem['kind']) {
  return seed.dictionaries.filter((item) => item.kind === kind).map((item) => item.key)
}

export function filterProviders(seed: ProviderCloudSeed, search: string) {
  const needle = search.trim().toLowerCase()
  if (!needle) {
    return seed.providers
  }
  return seed.providers.filter((provider) => {
    return [
      provider.provider_key,
      provider.name,
      provider.provider_driver,
      provider.base_url ?? '',
      provider.provider_kind,
    ].some((value) => value.toLowerCase().includes(needle))
  })
}

export function getOriginalProviders(seed: ProviderCloudSeed) {
  return Array.from(new Set(seed.model_param_rules.map((rule) => rule.original_provider).filter(Boolean))).sort() as string[]
}

export function getSourceModelIds(seed: ProviderCloudSeed) {
  return Array.from(new Set(seed.model_param_rules.map((rule) => rule.model_id_selector).filter(Boolean))).sort() as string[]
}

export function filterModelRules(
  seed: ProviderCloudSeed,
  filters: {
    search: string
    providerKey: string
    apiType: string
    capability: string
  },
) {
  const needle = filters.search.trim().toLowerCase()
  return seed.model_param_rules.filter((rule) => {
    const providerMatch = !filters.providerKey || rule.provider_key === filters.providerKey
    const apiMatch = !filters.apiType || rule.api_types.includes(filters.apiType)
    const capabilityMatch = !filters.capability || Object.keys(rule.capabilities).includes(filters.capability)
    const textMatch = !needle || [
      rule.rule_key,
      rule.original_provider ?? '',
      rule.model_id_selector ?? '',
      rule.match_type,
      rule.model_driver ?? '',
    ].some((value) => value.toLowerCase().includes(needle))
    return providerMatch && apiMatch && capabilityMatch && textMatch
  })
}

export function filterOpsBulkModelRules(seed: ProviderCloudSeed, filters: OpsBulkOperationInput) {
  return seed.model_param_rules.filter((rule) => {
    const providerMatch = !filters.provider_key || rule.provider_key === filters.provider_key
    const originMatch = !filters.original_provider || rule.original_provider === filters.original_provider
    const patternMatch = !filters.model_id_pattern || wildcardMatch(filters.model_id_pattern, rule.model_id_selector ?? '')
    const apiMatch = !filters.api_type || rule.api_types.includes(filters.api_type)
    const capabilityMatch = !filters.capability || Object.prototype.hasOwnProperty.call(rule.capabilities, filters.capability)
    const recommendationMatch = !filters.recommendation_level || getEffectiveRecommendation(seed, rule) === filters.recommendation_level
    const pricing = getEffectivePricing(seed, rule)
    const priceValue = pricing?.input ?? null
    const priceMinMatch = filters.price_min === undefined || (priceValue !== null && priceValue >= filters.price_min)
    const priceMaxMatch = filters.price_max === undefined || (priceValue !== null && priceValue <= filters.price_max)
    const routingWeight = getEffectiveRoutingWeight(seed, rule)
    const routingMinMatch = filters.routing_weight_min === undefined || routingWeight >= filters.routing_weight_min
    const routingMaxMatch = filters.routing_weight_max === undefined || routingWeight <= filters.routing_weight_max
    return providerMatch
      && originMatch
      && patternMatch
      && apiMatch
      && capabilityMatch
      && recommendationMatch
      && priceMinMatch
      && priceMaxMatch
      && routingMinMatch
      && routingMaxMatch
  })
}

function previewBulkRow(seed: ProviderCloudSeed, rule: ModelParamRuleRecord, input: OpsBulkOperationInput): OpsBulkPreviewRow {
  const current = buildOpsModelPreview(seed, rule)
  const pricingBefore = getEffectivePricing(seed, rule)
  const routingBefore = getEffectiveRoutingWeight(seed, rule)
  const recommendationBefore = getEffectiveRecommendation(seed, rule)
  const displayPriorityBefore = getEffectiveDisplayPriority(seed, rule)
  const priceFactor = 1 + input.price_percent / 100
  const pricingAfter = input.action === 'clear_pricing'
    ? null
    : input.action === 'set_price'
      ? { input: input.pricing_input, output: input.pricing_output }
      : input.action === 'adjust_price_percent' && pricingBefore
        ? {
            input: Number((pricingBefore.input * priceFactor).toFixed(8)),
            output: Number((pricingBefore.output * priceFactor).toFixed(8)),
          }
        : pricingBefore
  const visibleAfter = current.visible
  const routingAfter = input.action === 'set_routing_weight' ? input.routing_weight : routingBefore
  const recommendationAfter = input.action === 'set_recommendation' ? input.target_recommendation_level : recommendationBefore
  const displayPriorityAfter = input.action === 'set_display_priority' ? input.display_priority : displayPriorityBefore

  return {
    rule_key: rule.rule_key,
    provider_key: rule.provider_key,
    original_provider: rule.original_provider,
    model_id_selector: rule.model_id_selector,
    api_types: rule.api_types,
    capabilities: Object.keys(rule.capabilities),
    visible_before: current.visible,
    visible_after: visibleAfter,
    pricing_before: pricingBefore,
    pricing_after: pricingAfter,
    routing_weight_before: routingBefore,
    routing_weight_after: routingAfter,
    recommendation_before: recommendationBefore,
    recommendation_after: recommendationAfter,
    display_priority_before: displayPriorityBefore,
    display_priority_after: displayPriorityAfter,
  }
}

export function previewOpsBulkOperation(seed: ProviderCloudSeed, input: OpsBulkOperationInput): OpsBulkOperationPreview {
  const rows = filterOpsBulkModelRules(seed, input).map((rule) => previewBulkRow(seed, rule, input))
  return {
    hit_count: rows.length,
    samples: rows.slice(0, 10),
    visibility_removed: rows.filter((row) => row.visible_before && !row.visible_after).length,
    visibility_added: rows.filter((row) => !row.visible_before && row.visible_after).length,
    price_changed: rows.filter((row) => JSON.stringify(row.pricing_before) !== JSON.stringify(row.pricing_after)).length,
    routing_changed: rows.filter((row) => row.routing_weight_before !== row.routing_weight_after).length,
    display_priority_changed: rows.filter((row) => row.display_priority_before !== row.display_priority_after).length,
  }
}

function wildcardMatch(pattern: string, value: string) {
  if (pattern === value) {
    return true
  }
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*')
  return new RegExp(`^${escaped}$`).test(value)
}

export function matchesWildcard(pattern: string, value: string) {
  return wildcardMatch(pattern, value)
}

export function getRulesByMatchType(seed: ProviderCloudSeed, matchType: MatchType) {
  const rules = seed.model_param_rules.filter((rule) => rule.match_type === matchType)
  if (matchType === 'pattern') {
    return [...rules].sort((a, b) => (a.priority ?? 9999) - (b.priority ?? 9999))
  }
  return rules
}

export function previewResolverHits(seed: ProviderCloudSeed, rule: ModelParamRuleRecord) {
  const sourceModels = seed.model_param_rules.filter((item) => item.enabled && item.match_type === 'exact' && item.model_id_selector)
  if (rule.match_type === 'default') {
    return sourceModels.filter((model) => {
      const hitExact = seed.model_param_rules.some((item) => {
        return item.enabled && item.match_type === 'exact' && item.model_id_selector === model.model_id_selector
      })
      const hitPattern = seed.model_param_rules.some((item) => {
        return item.enabled && item.match_type === 'pattern' && item.model_id_selector && wildcardMatch(item.model_id_selector, model.model_id_selector ?? '')
      })
      return !hitExact && !hitPattern
    })
  }
  if (!rule.model_id_selector) {
    return []
  }
  if (rule.match_type === 'pattern') {
    return sourceModels.filter((model) => wildcardMatch(rule.model_id_selector ?? '', model.model_id_selector ?? ''))
  }
  return sourceModels.filter((model) => model.model_id_selector === rule.model_id_selector)
}

export function getResolverWarnings(seed: ProviderCloudSeed): WarningRecord[] {
  const warnings: WarningRecord[] = []
  const exactKeys = new Set<string>()
  seed.model_param_rules.forEach((rule) => {
    if (rule.match_type === 'exact' && rule.model_id_selector) {
      const key = `${rule.provider_key ?? 'global'}:${rule.model_id_selector}`
      if (exactKeys.has(key)) {
        warnings.push({
          warning_key: `resolver-duplicate-${rule.rule_key}`,
          severity: 'blocked',
          target_type: 'model_param_rule',
          target_key: rule.rule_key,
          message_key: 'warning.resolverDuplicateExact',
          detail: rule.model_id_selector,
          created_at: Date.now(),
        })
      }
      exactKeys.add(key)
    }
    if (rule.match_type === 'pattern' && previewResolverHits(seed, rule).length === 0) {
      warnings.push({
        warning_key: `resolver-empty-${rule.rule_key}`,
        severity: 'warning',
        target_type: 'model_param_rule',
        target_key: rule.rule_key,
        message_key: 'warning.resolverEmptyPattern',
        detail: rule.model_id_selector ?? '',
        created_at: Date.now(),
      })
    }
  })
  return warnings
}

export function previewVariantHits(seed: ProviderCloudSeed, rule: MetadataVariantRecord | MetadataVersionRuleRecord) {
  return seed.model_param_rules.filter((modelRule) => {
    if (!modelRule.enabled || !modelRule.model_id_selector) {
      return false
    }
    const originMatch = !rule.original_provider || rule.original_provider === modelRule.original_provider
    const idMatch = rule.selector_type === 'exact'
      ? rule.model_id_selector === modelRule.model_id_selector
      : wildcardMatch(rule.model_id_selector, modelRule.model_id_selector)
    return originMatch && idMatch
  })
}

function normalizeDirectoryPath(path: string) {
  const normalized = path.trim().replace(/\./g, '/').replace(/^\/+|\/+$/g, '')
  return normalized ? `/${normalized}` : '/'
}

export function getMaterializedLogicalDirectories(seed: ProviderCloudSeed): LogicalDirectoryRecord[] {
  const now = Date.now()
  const byPath = new Map<string, LogicalDirectoryRecord>()
  const ensureDirectory = (path: string) => {
    const normalizedPath = normalizeDirectoryPath(path)
    const existing = byPath.get(normalizedPath)
    if (existing) {
      return existing
    }
    const explicit = seed.logical_directories.find((directory) => normalizeDirectoryPath(directory.path) === normalizedPath)
    const parentPath = normalizedPath === '/' ? null : normalizedPath.split('/').slice(0, -1).join('/') || '/'
    const parent = parentPath ? ensureDirectory(parentPath) : null
    const directory: LogicalDirectoryRecord = explicit ?? {
      directory_key: `auto-${normalizedPath.replace(/[^a-z0-9]+/gi, '-').replace(/^-+|-+$/g, '') || 'root'}`.toLowerCase(),
      path: normalizedPath,
      title: normalizedPath === '/' ? 'Root' : normalizedPath.split('/').pop() ?? normalizedPath,
      parent_key: parent?.directory_key ?? null,
      model_rule_keys: [],
      created_at: now,
      updated_at: now,
    }
    byPath.set(normalizedPath, {
      ...directory,
      parent_key: parent?.directory_key ?? directory.parent_key ?? null,
      model_rule_keys: [...directory.model_rule_keys],
    })
    return byPath.get(normalizedPath) as LogicalDirectoryRecord
  }

  seed.logical_directories.forEach((directory) => ensureDirectory(directory.path))
  seed.model_param_rules.forEach((rule) => {
    rule.logical_mounts.forEach((mount) => {
      const directory = ensureDirectory(mount)
      if (!directory.model_rule_keys.includes(rule.rule_key)) {
        directory.model_rule_keys.push(rule.rule_key)
      }
    })
  })
  seed.metadata_variants.forEach((variant) => {
    const mounts = Array.isArray(variant.content.logical_mounts) ? variant.content.logical_mounts : []
    mounts.filter((mount): mount is string => typeof mount === 'string').forEach((mount) => ensureDirectory(mount))
  })
  seed.metadata_version_rules.forEach((rule) => {
    const mounts = [
      ...(Array.isArray(rule.content.auto_mounts) ? rule.content.auto_mounts : []),
      rule.content.current_mount,
      rule.content.version_mount,
    ]
    mounts.filter((mount): mount is string => typeof mount === 'string' && mount.length > 0).forEach((mount) => ensureDirectory(mount))
  })
  return Array.from(byPath.values()).sort((a, b) => a.path.localeCompare(b.path))
}

export function materializeDirectoryItems(seed: ProviderCloudSeed, directory: LogicalDirectoryRecord) {
  const directories = getMaterializedLogicalDirectories(seed)
  const childDirectories = directories.filter((item) => item.parent_key === directory.directory_key)
  const models = seed.model_param_rules.filter((rule) => directory.model_rule_keys.includes(rule.rule_key) || rule.logical_mounts.includes(directory.path))
  return { childDirectories, models }
}

export function searchLogicalDirectory(seed: ProviderCloudSeed, search: string) {
  const needle = search.trim().toLowerCase()
  if (!needle) {
    return []
  }
  const directories = getMaterializedLogicalDirectories(seed).filter((directory) => {
    return [directory.directory_key, directory.path, directory.title].some((value) => value.toLowerCase().includes(needle))
  })
  const models = seed.model_param_rules.filter((rule) => {
    return [rule.rule_key, rule.original_provider ?? '', rule.model_id_selector ?? '', ...rule.logical_mounts].some((value) => value.toLowerCase().includes(needle))
  })
  return [...directories.map((directory) => ({ kind: 'directory' as const, item: directory })), ...models.map((model) => ({ kind: 'model' as const, item: model }))]
}

export function getDirectoryBreadcrumbs(seed: ProviderCloudSeed, directoryKey: string) {
  const breadcrumbs: LogicalDirectoryRecord[] = []
  const directories = getMaterializedLogicalDirectories(seed)
  let current = directories.find((directory) => directory.directory_key === directoryKey) ?? null
  while (current) {
    breadcrumbs.unshift(current)
    current = current.parent_key ? directories.find((directory) => directory.directory_key === current?.parent_key) ?? null : null
  }
  return breadcrumbs
}

export function getLogicalDirectoryWarnings(seed: ProviderCloudSeed): WarningRecord[] {
  const warnings: WarningRecord[] = []
  const keys = new Set<string>()
  const paths = new Set<string>()
  const ruleKeys = new Set(seed.model_param_rules.map((rule) => rule.rule_key))
  seed.logical_directories.forEach((directory) => {
    if (keys.has(directory.directory_key) || paths.has(directory.path)) {
      warnings.push({
        warning_key: `directory-duplicate-${directory.directory_key}`,
        severity: 'blocked',
        target_type: 'logical_directory',
        target_key: directory.directory_key,
        message_key: 'warning.directoryDuplicate',
        detail: directory.path,
        created_at: Date.now(),
      })
    }
    keys.add(directory.directory_key)
    paths.add(directory.path)
    if (directory.model_rule_keys.length === 0) {
      warnings.push({
        warning_key: `directory-empty-${directory.directory_key}`,
        severity: 'info',
        target_type: 'logical_directory',
        target_key: directory.directory_key,
        message_key: 'warning.directoryEmpty',
        detail: directory.path,
        created_at: Date.now(),
      })
    }
    directory.model_rule_keys.forEach((ruleKey) => {
      if (!ruleKeys.has(ruleKey)) {
        warnings.push({
          warning_key: `directory-broken-${directory.directory_key}-${ruleKey}`,
          severity: 'warning',
          target_type: 'logical_directory',
          target_key: directory.directory_key,
          message_key: 'warning.directoryBrokenReference',
          detail: ruleKey,
          created_at: Date.now(),
        })
      }
    })
  })
  return warnings
}

export function getDictionaryReferenceCount(seed: ProviderCloudSeed, item: DictionaryItem) {
  if (item.kind === 'api_type') {
    return seed.model_param_rules.filter((rule) => rule.api_types.includes(item.key)).length
  }
  return seed.model_param_rules.filter((rule) => Object.prototype.hasOwnProperty.call(rule.capabilities, item.key)).length
}

export function getDictionarySamples(seed: ProviderCloudSeed, item: DictionaryItem) {
  const rows = item.kind === 'api_type'
    ? seed.model_param_rules.filter((rule) => rule.api_types.includes(item.key))
    : seed.model_param_rules.filter((rule) => Object.prototype.hasOwnProperty.call(rule.capabilities, item.key))
  return rows.slice(0, 6)
}

export function previewSelectionRuleHits(seed: ProviderCloudSeed, rule: { rule_type: string; selector: string }) {
  const models = seed.model_param_rules.filter((modelRule) => modelRule.enabled && modelRule.model_id_selector)
  return models.filter((modelRule) => {
    if (rule.rule_type.endsWith('_origin')) {
      return modelRule.original_provider === rule.selector
    }
    return wildcardMatch(rule.selector, modelRule.model_id_selector ?? '')
  })
}

export function previewNickRewrite(seed: ProviderCloudSeed, providerKey: string) {
  const nicks = seed.model_nicks
    .filter((nick) => nick.provider_key === providerKey)
    .sort((a, b) => a.priority - b.priority)
  return seed.model_param_rules
    .filter((rule) => rule.enabled && rule.model_id_selector)
    .slice(0, 48)
    .map((rule) => {
      const sourceModelId = rule.model_id_selector ?? ''
      const nickRule = nicks.find((nick) => {
        const originMatch = !nick.original_provider || nick.original_provider === rule.original_provider
        const idMatch = nick.selector_type === 'exact' ? nick.model_id === sourceModelId : wildcardMatch(nick.model_id, sourceModelId)
        return originMatch && idMatch
      })
      return {
        source_model_id: sourceModelId,
        original_provider: rule.original_provider,
        published_id: nickRule ? nickRule.nick.replace('{model}', sourceModelId) : sourceModelId,
        nick_key: nickRule?.nick_key ?? null,
      }
    })
}

export function buildTechDiagnostics(seed: ProviderCloudSeed): WarningRecord[] {
  const now = Date.now()
  const diagnostics: WarningRecord[] = []
  const providerKeys = new Map<string, number>()
  seed.providers.forEach((provider) => providerKeys.set(provider.provider_key, (providerKeys.get(provider.provider_key) ?? 0) + 1))
  providerKeys.forEach((count, key) => {
    if (count > 1) {
      diagnostics.push({
        warning_key: `diag-provider-duplicate-${key}`,
        severity: 'blocked',
        target_type: 'provider',
        target_key: key,
        message_key: 'warning.providerDuplicate',
        detail: key,
        created_at: now,
      })
    }
  })

  seed.provider_model_rules.forEach((rule) => {
    if (rule.enabled && previewSelectionRuleHits(seed, rule).length === 0) {
      diagnostics.push({
        warning_key: `diag-selection-empty-${rule.rule_key}`,
        severity: 'warning',
        target_type: 'selection_rule',
        target_key: rule.rule_key,
        message_key: 'warning.selectionEmpty',
        detail: `${rule.rule_type}: ${rule.selector}`,
        created_at: now,
      })
    }
  })

  const apiTypes = new Set(getDictionaryKeys(seed, 'api_type'))
  const capabilities = new Set(getDictionaryKeys(seed, 'capability'))
  seed.model_param_rules.forEach((rule) => {
    rule.api_types.forEach((apiType) => {
      if (!apiTypes.has(apiType)) {
        diagnostics.push({
          warning_key: `diag-api-${rule.rule_key}-${apiType}`,
          severity: 'blocked',
          target_type: 'model_param_rule',
          target_key: rule.rule_key,
          message_key: 'warning.dictionaryMissing',
          detail: apiType,
          created_at: now,
        })
      }
    })
    Object.keys(rule.capabilities).forEach((capability) => {
      if (!capabilities.has(capability)) {
        diagnostics.push({
          warning_key: `diag-capability-${rule.rule_key}-${capability}`,
          severity: 'blocked',
          target_type: 'model_param_rule',
          target_key: rule.rule_key,
          message_key: 'warning.dictionaryMissing',
          detail: capability,
          created_at: now,
        })
      }
    })
  })

  seed.providers.forEach((provider) => {
    const publishedIds = new Map<string, string[]>()
    previewNickRewrite(seed, provider.provider_key).forEach((item) => {
      const keys = publishedIds.get(item.published_id) ?? []
      keys.push(item.source_model_id)
      publishedIds.set(item.published_id, keys)
    })
    publishedIds.forEach((sources, publishedId) => {
      if (sources.length > 1) {
        diagnostics.push({
          warning_key: `diag-nick-conflict-${provider.provider_key}-${publishedId}`,
          severity: 'warning',
          target_type: 'nick_rule',
          target_key: provider.provider_key,
          message_key: 'warning.nickConflict',
          detail: `${publishedId}: ${sources.slice(0, 3).join(', ')}`,
          created_at: now,
        })
      }
    })
  })

  return diagnostics
}

export function buildOpsDiagnostics(seed: ProviderCloudSeed): WarningRecord[] {
  const now = Date.now()
  const diagnostics: WarningRecord[] = []
  if (seed.tech_source.last_error) {
    diagnostics.push({
      warning_key: 'ops-sync-failed',
      severity: 'warning',
      target_type: 'tech_source',
      target_key: 'source',
      message_key: 'warning.opsSyncFailed',
      detail: seed.tech_source.last_error,
      created_at: now,
    })
  }
  if (seed.tech_source.stale) {
    diagnostics.push({
      warning_key: 'ops-source-stale',
      severity: 'warning',
      target_type: 'tech_source',
      target_key: seed.tech_source.cache_revision,
      message_key: 'warning.opsDataStale',
      detail: seed.tech_source.cache_revision,
      created_at: now,
    })
  }
  seed.ops_overlays.forEach((overlay) => {
    const targetExists = overlay.target_type === 'provider'
      ? seed.providers.some((provider) => provider.provider_key === overlay.target_key)
      : overlay.target_type === 'model_param_rule'
        ? seed.model_param_rules.some((rule) => rule.rule_key === overlay.target_key)
        : overlay.target_type === 'variants'
          ? seed.metadata_variants.some((variant) => variant.variant_key === overlay.target_key)
          : seed.metadata_version_rules.some((rule) => rule.version_rule_key === overlay.target_key)
    if (!targetExists) {
      diagnostics.push({
        warning_key: `ops-merge-skipped-${overlay.overlay_key}`,
        severity: 'warning',
        target_type: overlay.target_type,
        target_key: overlay.target_key,
        message_key: 'warning.opsMergeSkipped',
        detail: overlay.overlay_key,
        created_at: now,
      })
    }
    const pollutedFields = Object.keys(overlay.ops_patch).filter((field) => {
      return ['provider_key', 'provider_driver', 'model_id_selector', 'match_type', 'api_types', 'capabilities', 'logical_mounts'].includes(field)
    })
    if (pollutedFields.length) {
      diagnostics.push({
        warning_key: `ops-pollution-${overlay.overlay_key}`,
        severity: 'blocked',
        target_type: overlay.target_type,
        target_key: overlay.target_key,
        message_key: 'warning.opsOverlayPollution',
        detail: pollutedFields.join(', '),
        created_at: now,
      })
    }
    if (overlay.target_type === 'model_param_rule') {
      const rule = seed.model_param_rules.find((item) => item.rule_key === overlay.target_key)
      const pricing = getOpsPatchValue<{ input?: number; output?: number } | null>(overlay, 'pricing_override', null)
      if (pricing && ((pricing.input ?? 0) < 0 || (pricing.output ?? 0) < 0 || (pricing.input ?? 0) > (pricing.output ?? 0))) {
        diagnostics.push({
          warning_key: `ops-invalid-pricing-${overlay.overlay_key}`,
          severity: 'blocked',
          target_type: 'model_param_rule',
          target_key: overlay.target_key,
          message_key: 'warning.opsInvalidPricing',
          detail: `${pricing.input ?? '-'} / ${pricing.output ?? '-'}`,
          created_at: now,
        })
      }
      const routingWeight = getOpsPatchValue<number | null>(overlay, 'routing_weight', null)
      if (routingWeight !== null && (routingWeight < 1 || routingWeight > 95)) {
        diagnostics.push({
          warning_key: `ops-routing-weight-${overlay.overlay_key}`,
          severity: 'warning',
          target_type: 'model_param_rule',
          target_key: overlay.target_key,
          message_key: 'warning.opsRoutingWeightRange',
          detail: `${rule?.model_id_selector ?? overlay.target_key}: ${routingWeight}`,
          created_at: now,
        })
      }
    }
  })
  return diagnostics
}

function rulesByType(rules: ModelParamRuleRecord[], matchType: MatchType) {
  return rules.filter((rule) => rule.enabled && rule.match_type === matchType)
}

function isOriginalProviderRule(provider: ProviderRecord, originalProvider: string | null) {
  return originalProvider === provider.provider_key || originalProvider === provider.provider_driver
}

function isModelRulePublishedForProvider(provider: ProviderRecord, rule: ModelParamRuleRecord) {
  return rule.provider_key === provider.provider_key
    || (rule.provider_key === null && isOriginalProviderRule(provider, rule.original_provider))
}

function isVariantPublishedForProvider(provider: ProviderRecord, variant: MetadataVariantRecord) {
  return variant.provider_key === provider.provider_key
    || (variant.provider_key === null && isOriginalProviderRule(provider, variant.original_provider))
}

function isVersionRulePublishedForProvider(provider: ProviderRecord, rule: MetadataVersionRuleRecord) {
  return rule.provider_key === provider.provider_key
    || (rule.provider_key === null && isOriginalProviderRule(provider, rule.original_provider))
}

function compactObject<T extends object>(value: T) {
  return Object.fromEntries(
    Object.entries(value).filter(([, item]) => {
      if (item === undefined || item === null) {
        return false
      }
      if (Array.isArray(item)) {
        return item.length > 0
      }
      if (typeof item === 'object') {
        return Object.keys(item).length > 0
      }
      return true
    }),
  ) as Partial<T>
}

function getNumericAttribute(rule: ModelParamRuleRecord, key: string) {
  const value = rule.attributes?.[key]
  return typeof value === 'number' ? value : undefined
}

function getStringAttribute(rule: ModelParamRuleRecord, key: string) {
  const value = rule.attributes?.[key]
  return typeof value === 'string' ? value : undefined
}

function getRuleContextLimits(rule: ModelParamRuleRecord) {
  const maxContextTokens = typeof rule.context_limits?.max_context_tokens === 'number'
    ? rule.context_limits.max_context_tokens
    : typeof rule.capabilities.max_context_tokens === 'number'
      ? rule.capabilities.max_context_tokens
      : undefined
  const maxOutputTokens = typeof rule.context_limits?.max_output_tokens === 'number'
    ? rule.context_limits.max_output_tokens
    : typeof rule.capabilities.max_output_tokens === 'number'
      ? rule.capabilities.max_output_tokens
      : undefined
  return compactObject({
    max_context_tokens: maxContextTokens,
    max_output_tokens: maxOutputTokens,
  }) as Record<string, number>
}

function getRawDriverRule(rule: ModelParamRuleRecord) {
  const raw = rule.attributes?.source_rule
  return typeof raw === 'object' && raw !== null && !Array.isArray(raw)
    ? { ...raw } as DriverMetadataRule
    : null
}

function applyPricingOverride(base: DriverMetadataRule, rule: ModelParamRuleRecord, pricingOverride: { input?: number; output?: number } | null) {
  if (!pricingOverride || typeof pricingOverride.input !== 'number') {
    return
  }
  if (typeof base.pricing === 'object' && base.pricing !== null && !Array.isArray(base.pricing)) {
    base.pricing = compactObject({
      ...base.pricing,
      price: pricingOverride.input,
    }) as DriverMetadataRule['pricing']
    return
  }
  if (base.estimated_cost_usd !== undefined || rule.pricing?.estimated_cost_usd !== undefined) {
    base.estimated_cost_usd = pricingOverride.input
  }
}

function getRulePricing(rule: ModelParamRuleRecord, pricingOverride: { input?: number; output?: number } | null) {
  const price = pricingOverride?.input
    ?? (typeof rule.pricing?.price === 'number' ? rule.pricing.price : undefined)
    ?? (typeof rule.pricing?.estimated_cost_usd === 'number' ? rule.pricing.estimated_cost_usd : undefined)
  if (price === undefined) {
    return undefined
  }
  return compactObject({
    price,
    currency: typeof rule.pricing?.currency === 'string' ? rule.pricing.currency : 'USD',
    unit: typeof rule.pricing?.unit === 'string' ? rule.pricing.unit : '1M_tokens',
  }) as DriverMetadataRule['pricing']
}

function findNickRule(seed: ProviderCloudSeed, provider: ProviderRecord, selector: string, originalProvider: string | null) {
  return seed.model_nicks
    .filter((item) => item.provider_key === provider.provider_key)
    .sort((a, b) => a.priority - b.priority)
    .find((item) => {
      const originMatch = !item.original_provider || item.original_provider === originalProvider
      const selectorMatch = item.selector_type === 'exact'
        ? item.model_id === selector
        : wildcardMatch(item.model_id, selector)
      return originMatch && selectorMatch
    })
}

function rewritePublishSelector(seed: ProviderCloudSeed, provider: ProviderRecord, selector: string, originalProvider: string | null) {
  const nick = findNickRule(seed, provider, selector, originalProvider)
  return nick ? nick.nick.replace('{model}', selector) : selector
}

function rewriteModelSelector(seed: ProviderCloudSeed, provider: ProviderRecord, rule: ModelParamRuleRecord) {
  const selector = rule.model_id_selector
  if (!selector) {
    return undefined
  }
  return rewritePublishSelector(seed, provider, selector, rule.original_provider)
}

function materializeDriverRule(seed: ProviderCloudSeed, provider: ProviderRecord, rule: ModelParamRuleRecord): DriverMetadataRule {
  const overlay = getOpsOverlay(seed, 'model_param_rule', rule.rule_key)
  const pricingOverride = getOpsPatchValue<{ input?: number; output?: number } | null>(overlay, 'pricing_override', null)
  const estimatedCost = pricingOverride?.input
    ?? (typeof rule.pricing?.estimated_cost_usd === 'number' ? rule.pricing.estimated_cost_usd : undefined)
  const estimatedLatency = typeof rule.pricing?.estimated_latency_ms === 'number' ? rule.pricing.estimated_latency_ms : undefined
  const contextLimits = getRuleContextLimits(rule)
  const pricing = getRulePricing(rule, pricingOverride)
  const qualityScore = getOpsPatchValue(overlay, 'quality_score', getNumericAttribute(rule, 'quality_score'))
  const latencyClass = getOpsPatchValue(overlay, 'latency_class', getStringAttribute(rule, 'latency_class'))
  const costClass = getOpsPatchValue(overlay, 'cost_class', getStringAttribute(rule, 'cost_class'))
  const selector = rewriteModelSelector(seed, provider, rule)
  const rawRule = getRawDriverRule(rule)
  const base: DriverMetadataRule = rawRule ?? {}
  const hasRawRule = rawRule !== null
  if (rule.match_type === 'exact') {
    base.id = selector
    delete base.pattern
  }
  if (rule.match_type === 'pattern') {
    base.pattern = selector
    delete base.id
  }
  if (rule.match_type === 'default') {
    delete base.id
    delete base.pattern
  }
  if (rule.exclude && rule.match_type !== 'default') {
    return compactObject({
      id: rule.match_type === 'exact' ? selector : undefined,
      pattern: rule.match_type === 'pattern' ? selector : undefined,
      model_driver: rule.model_driver && rule.model_driver !== provider.provider_driver ? rule.model_driver : undefined,
      exclude: true,
    }) as DriverMetadataRule
  }
  if (rule.model_driver && rule.model_driver !== provider.provider_driver) {
    base.model_driver = rule.model_driver
  } else {
    delete base.model_driver
  }
  base.parameter_scale = getStringAttribute(rule, 'parameter_scale') ?? base.parameter_scale
  base.api_types = rule.api_types
  base.logical_mounts = rule.logical_mounts
  base.capabilities = rule.capabilities
  if (!hasRawRule || base.context_limits !== undefined) {
    base.context_limits = contextLimits
  }
  if (!hasRawRule || base.pricing !== undefined) {
    base.pricing = pricing
  }
  if (estimatedCost !== undefined || base.estimated_cost_usd !== undefined) {
    base.estimated_cost_usd = estimatedCost
  }
  if (estimatedLatency !== undefined || base.estimated_latency_ms !== undefined) {
    base.estimated_latency_ms = estimatedLatency
  }
  if (qualityScore !== undefined || base.quality_score !== undefined) {
    base.quality_score = qualityScore
  }
  if (latencyClass !== undefined || base.latency_class !== undefined) {
    base.latency_class = latencyClass
  }
  if (costClass !== undefined || base.cost_class !== undefined) {
    base.cost_class = costClass
  }
  applyPricingOverride(base, rule, pricingOverride)
  return compactObject(base) as DriverMetadataRule
}

function materializeVariant(seed: ProviderCloudSeed, provider: ProviderRecord, variant: MetadataVariantRecord) {
  const base = { ...variant.content }
  const sourceSelector = variant.model_id_selector || '*'
  const publishedSelector = rewritePublishSelector(seed, provider, sourceSelector, variant.original_provider)
  if (typeof base.name === 'string' && sourceSelector !== publishedSelector) {
    base.name = base.name.replace(sourceSelector, publishedSelector)
  }
  return compactObject(base)
}

function materializeVersionRule(seed: ProviderCloudSeed, provider: ProviderRecord, rule: MetadataVersionRuleRecord) {
  const content = { ...rule.content }
  if (typeof content.model_pattern === 'string') {
    content.model_pattern = rewritePublishSelector(seed, provider, content.model_pattern, rule.original_provider)
  } else {
    const sourceSelector = rule.model_id_selector || '*'
    const publishedSelector = rewritePublishSelector(seed, provider, sourceSelector, rule.original_provider)
    if (sourceSelector !== '*' || publishedSelector !== sourceSelector) {
      content.model_pattern = publishedSelector
    }
  }
  return compactObject(content)
}

export function buildPublishedProviderJson(seed: ProviderCloudSeed, provider: ProviderRecord): DriverMetadataDocument {
  const scopedRules = seed.model_param_rules.filter((rule) => {
    return isModelRulePublishedForProvider(provider, rule)
  })
  const patterns = rulesByType(scopedRules, 'pattern')
    .sort((a, b) => (a.priority ?? 9999) - (b.priority ?? 9999))
    .filter((rule) => buildOpsModelPreview(seed, rule).visible)
    .map((rule) => materializeDriverRule(seed, provider, rule))
  const defaultRule = rulesByType(scopedRules, 'default')[0]
  const variants = seed.metadata_variants
    .filter((variant) => {
      return variant.enabled
        && isVariantPublishedForProvider(provider, variant)
    })
    .sort((a, b) => a.priority - b.priority)
    .map((variant) => materializeVariant(seed, provider, variant))
  const versionRules = seed.metadata_version_rules
    .filter((rule) => {
      return rule.enabled
        && isVersionRulePublishedForProvider(provider, rule)
    })
    .sort((a, b) => a.priority - b.priority)
    .map((rule) => materializeVersionRule(seed, provider, rule))

  return {
    schema_version: 1,
    provider_driver: provider.provider_driver,
    name: provider.name || null,
    protocol_family: provider.protocol_family ?? '',
    base_url: provider.base_url,
    revision: provider.revision === 'draft' ? seed.published_revision : provider.revision,
    models: rulesByType(scopedRules, 'exact')
      .filter((rule) => buildOpsModelPreview(seed, rule).visible)
      .map((rule) => materializeDriverRule(seed, provider, rule)),
    patterns,
    defaults: defaultRule && buildOpsModelPreview(seed, defaultRule).visible ? materializeDriverRule(seed, provider, defaultRule) : {},
    variants,
    version_rules: versionRules,
    signature: null,
  }
}

export function buildPublishPreview(seed: ProviderCloudSeed): PublishPreview {
  const opsDiagnostics = buildOpsDiagnostics(seed)
  const warnings = [...seed.warnings, ...buildTechDiagnostics(seed), ...opsDiagnostics]
  const providerPreviews = seed.providers.map((provider) => buildOpsProviderPreview(seed, provider))
  const modelPreviews = seed.model_param_rules.map((rule) => buildOpsModelPreview(seed, rule))
  const discardedTechnicalFields = seed.ops_overlays.reduce((count, overlay) => {
    return count + Object.keys(overlay.ops_patch).filter((field) => {
      return ['provider_key', 'provider_driver', 'model_id_selector', 'match_type', 'api_types', 'capabilities', 'logical_mounts'].includes(field)
    }).length
  }, 0)
  return {
    revision: seed.published_revision,
    provider_count: seed.providers.length,
    model_rule_count: seed.model_param_rules.length,
    pending_change_count: seed.pending_changes.length,
    warnings,
    impact: {
      providers: seed.pending_changes.filter((change) => change.target_type === 'provider').length,
      model_rules: seed.pending_changes.filter((change) => change.target_type === 'model_param_rule').length,
      dictionaries: seed.pending_changes.filter((change) => change.target_type === 'dictionary').length,
    },
    ops_impact: {
      visible_providers: providerPreviews.filter((provider) => provider.visible).length,
      visible_models: modelPreviews.filter((model) => model.visible).length,
      disabled_providers: providerPreviews.filter((provider) => !provider.visible).length,
      disabled_models: modelPreviews.filter((model) => !model.visible).length,
      overlay_hits: seed.ops_overlays.length,
      discarded_technical_fields: discardedTechnicalFields,
      source_stale: seed.tech_source.stale,
    },
    published_json: seed.providers
      .filter((provider) => buildOpsProviderPreview(seed, provider).visible)
      .slice(0, 3)
      .map((provider) => buildPublishedProviderJson(seed, provider)),
  }
}

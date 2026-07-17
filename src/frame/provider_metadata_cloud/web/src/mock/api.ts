import { buildPublishPreview, filterOpsBulkModelRules, getOpsOverlay, getOpsPatchValue } from '../datamodel/selectors'
import type {
  ChangeLogRecord,
  DictionaryItem,
  ImportPlanActionName,
  ImportPlanActionRecord,
  ImportPlanParseResult,
  LogicalDirectoryRecord,
  ModelNickRecord,
  ModelParamRuleRecord,
  MetadataVariantRecord,
  MetadataVersionRuleRecord,
  OriginMappingRuleRecord,
  OriginProviderAliasRecord,
  PendingChangeRecord,
  ProviderCloudSeed,
  ProviderModelRuleRecord,
  ProviderRecord,
  PublishPreview,
  ServiceRole,
  WarningRecord,
} from '../datamodel/types'
import type {
  DeleteModelRuleInput,
  DictionaryBulkApplyInput,
  DictionaryItemInput,
  LogicalDirectoryInput,
  LogicalDirectoryMountInput,
  MetadataVariantInput,
  MetadataVersionRuleInput,
  ModelRuleInput,
  NickRuleInput,
  OriginMappingRuleInput,
  OriginProviderAliasInput,
  ProviderInput,
  ProviderWizardInput,
  ResolverRuleInput,
  SelectionRuleInput,
  ImportPlanDraftInput,
  ImportPlanInput,
  ModelOpsInput,
  OpsBulkOperationInput,
  ProviderOpsInput,
  PublishWizardInput,
  ResolverOpsOverlayInput,
  TechSourceInput,
} from '../datamodel/schemas'
import { providerCloudSeed } from './providerCloudSeed'
import { withMockLatency } from './latency'

let seed: ProviderCloudSeed = structuredClone(providerCloudSeed)
const forcedWorkspaceErrorCounts = new Map<string, number>()
const mockNow = () => Date.now()
const nextRevision = (prefix: string) => `${prefix}-20260710.${String(Math.floor(Math.random() * 900000) + 100000)}`
const supportedImportActions: ImportPlanActionName[] = [
  'upsert_provider',
  'disable_provider',
  'upsert_model_param_rule',
  'delete_model_param_rule',
  'include_models',
  'exclude_models',
  'set_model_nick',
  'upsert_variant',
  'upsert_version_rule',
  'set_logical_mounts',
  'upsert_logical_directory',
  'delete_logical_directory',
  'move_logical_directory',
  'set_api_types',
  'upsert_api_type',
  'delete_api_type',
  'set_capabilities',
  'upsert_capability',
  'delete_capability',
]

type ImportActionSpec = {
  target_type: string
  target_key: string
  selector: string | null
  match_type: ImportPlanActionRecord['match_type']
  priority: number | null
  source_record: string | null
  published_selector: string | null
  fallback_behavior: string | null
  field_changes: ImportPlanActionRecord['field_changes']
}

const importActionSpecs: Record<ImportPlanActionName, ImportActionSpec> = {
  upsert_provider: {
    target_type: 'provider',
    target_key: 'plan-openrouter',
    selector: null,
    match_type: null,
    priority: null,
    source_record: 'openrouter',
    published_selector: null,
    fallback_behavior: null,
    field_changes: [
      { field: 'provider_key', before: '-', after: 'plan-openrouter' },
      { field: 'base_url', before: '-', after: 'https://openrouter.ai/api/v1' },
    ],
  },
  disable_provider: {
    target_type: 'provider',
    target_key: 'openrouter',
    selector: null,
    match_type: null,
    priority: null,
    source_record: 'openrouter',
    published_selector: null,
    fallback_behavior: null,
    field_changes: [{ field: 'enabled', before: 'true', after: 'false' }],
  },
  upsert_model_param_rule: {
    target_type: 'model_param_rule',
    target_key: 'plan-gpt-pattern',
    selector: 'gpt-*',
    match_type: 'pattern',
    priority: 11,
    source_record: 'openai-pattern-8',
    published_selector: 'plan/{model}',
    fallback_behavior: 'exact rules win first, then this pattern by priority, default remains final fallback',
    field_changes: [
      { field: 'match_type', before: '-', after: 'pattern' },
      { field: 'api_types', before: '-', after: 'llm' },
      { field: 'capabilities', before: '-', after: 'streaming, tool_call' },
    ],
  },
  delete_model_param_rule: {
    target_type: 'model_param_rule',
    target_key: 'openai-pattern-8',
    selector: 'gpt-*',
    match_type: 'pattern',
    priority: 8,
    source_record: 'openai-pattern-8',
    published_selector: null,
    fallback_behavior: 'matched models fall back to lower priority pattern or default resolver rules',
    field_changes: [{ field: 'enabled', before: 'true', after: 'deleted' }],
  },
  include_models: {
    target_type: 'selection_rule',
    target_key: 'plan-include-gpt',
    selector: 'gpt-*',
    match_type: null,
    priority: 21,
    source_record: 'provider_model_rules',
    published_selector: 'gpt-*',
    fallback_behavior: null,
    field_changes: [{ field: 'rule_type', before: '-', after: 'include_pattern' }],
  },
  exclude_models: {
    target_type: 'model_param_rule',
    target_key: 'plan-exclude-gpt',
    selector: 'gpt-3.5*',
    match_type: 'pattern',
    priority: 22,
    source_record: 'model_param_rules',
    published_selector: 'gpt-3.5*',
    fallback_behavior: null,
    field_changes: [{ field: 'exclude', before: 'false', after: 'true' }],
  },
  set_model_nick: {
    target_type: 'nick_rule',
    target_key: 'plan-gpt-nick',
    selector: 'gpt-*',
    match_type: null,
    priority: 12,
    source_record: 'openrouter-openai-prefix',
    published_selector: 'plan/{model}',
    fallback_behavior: null,
    field_changes: [{ field: 'nick', before: 'openai/{model}', after: 'plan/{model}' }],
  },
  upsert_variant: {
    target_type: 'variant',
    target_key: 'plan-gpt-variant',
    selector: 'gpt-*',
    match_type: null,
    priority: 13,
    source_record: 'metadata_variants',
    published_selector: 'plan/{model}',
    fallback_behavior: null,
    field_changes: [{ field: 'content.variant', before: '-', after: 'plan-import' }],
  },
  upsert_version_rule: {
    target_type: 'version_rule',
    target_key: 'plan-gpt-version',
    selector: 'gpt-*',
    match_type: null,
    priority: 14,
    source_record: 'metadata_version_rules',
    published_selector: 'plan/{model}',
    fallback_behavior: null,
    field_changes: [{ field: 'content.version', before: '-', after: 'plan-import' }],
  },
  set_logical_mounts: {
    target_type: 'logical_directory',
    target_key: 'llm',
    selector: 'gpt-*',
    match_type: null,
    priority: null,
    source_record: 'logical_directories.llm',
    published_selector: '/llm',
    fallback_behavior: null,
    field_changes: [{ field: 'model_rule_keys', before: 'existing mounts', after: 'existing + gpt matches' }],
  },
  upsert_logical_directory: {
    target_type: 'logical_directory',
    target_key: 'plan-directory',
    selector: '/import/llm',
    match_type: null,
    priority: null,
    source_record: null,
    published_selector: '/import/llm',
    fallback_behavior: null,
    field_changes: [{ field: 'path', before: '-', after: '/import/llm' }],
  },
  delete_logical_directory: {
    target_type: 'logical_directory',
    target_key: 'audio',
    selector: '/audio',
    match_type: null,
    priority: null,
    source_record: 'logical_directories.audio',
    published_selector: null,
    fallback_behavior: null,
    field_changes: [{ field: 'path', before: '/audio', after: 'deleted' }],
  },
  move_logical_directory: {
    target_type: 'logical_directory',
    target_key: 'image',
    selector: '/image',
    match_type: null,
    priority: null,
    source_record: 'logical_directories.image',
    published_selector: '/llm/image',
    fallback_behavior: null,
    field_changes: [
      { field: 'parent_key', before: '-', after: 'llm' },
      { field: 'path', before: '/image', after: '/llm/image' },
    ],
  },
  set_api_types: {
    target_type: 'model_param_rule',
    target_key: 'gpt-matches',
    selector: 'gpt-*',
    match_type: null,
    priority: null,
    source_record: 'model_param_rules',
    published_selector: null,
    fallback_behavior: null,
    field_changes: [{ field: 'api_types', before: 'existing', after: 'llm, plan.chat' }],
  },
  upsert_api_type: {
    target_type: 'dictionary',
    target_key: 'plan.chat',
    selector: 'plan.chat',
    match_type: null,
    priority: null,
    source_record: 'dictionaries.api_type',
    published_selector: null,
    fallback_behavior: null,
    field_changes: [{ field: 'dictionary', before: '-', after: 'api_type plan.chat' }],
  },
  delete_api_type: {
    target_type: 'dictionary',
    target_key: 'embedding.text',
    selector: 'embedding.text',
    match_type: null,
    priority: null,
    source_record: 'dictionaries.api_type.embedding.text',
    published_selector: null,
    fallback_behavior: null,
    field_changes: [{ field: 'dictionary', before: 'embedding.text', after: 'deleted and removed from model refs' }],
  },
  set_capabilities: {
    target_type: 'model_param_rule',
    target_key: 'gpt-matches',
    selector: 'gpt-*',
    match_type: null,
    priority: null,
    source_record: 'model_param_rules',
    published_selector: null,
    fallback_behavior: null,
    field_changes: [{ field: 'capabilities', before: 'existing', after: 'streaming=true, plan_cached_tokens=8192' }],
  },
  upsert_capability: {
    target_type: 'dictionary',
    target_key: 'plan_cached_tokens',
    selector: 'plan_cached_tokens',
    match_type: null,
    priority: null,
    source_record: 'dictionaries.capability',
    published_selector: null,
    fallback_behavior: null,
    field_changes: [{ field: 'dictionary', before: '-', after: 'capability plan_cached_tokens:number' }],
  },
  delete_capability: {
    target_type: 'dictionary',
    target_key: 'vision',
    selector: 'vision',
    match_type: null,
    priority: null,
    source_record: 'dictionaries.capability.vision',
    published_selector: null,
    fallback_behavior: null,
    field_changes: [{ field: 'dictionary', before: 'vision', after: 'deleted and removed from model refs' }],
  },
}

function appendPendingChange(change: PendingChangeRecord) {
  seed = {
    ...seed,
    pending_changes: [
      change,
      ...seed.pending_changes.filter((item) => item.change_key !== change.change_key),
    ],
    edit_session: seed.edit_session
      ? {
          ...seed.edit_session,
          pending_change_count: seed.pending_changes.length + 1,
          updated_at: mockNow(),
        }
      : seed.edit_session,
  }
}

function safeRuleKeyPart(value: string) {
  return value.toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48) || 'model'
}

function sameStringSet(left: string[], right: string[]) {
  if (left.length !== right.length) {
    return false
  }
  const rightSet = new Set(right)
  return left.every((item) => rightSet.has(item))
}

function readCapabilityValues(capabilities: Record<string, boolean | number | string>) {
  const values: Record<string, boolean | number> = {}
  Object.entries(capabilities).forEach(([key, value]) => {
    if (typeof value === 'boolean' || typeof value === 'number') {
      values[key] = value
    }
  })
  return values
}

function getDraftCapabilityValue(draft: ProviderWizardInput['model_rule_drafts'][number], capability: string) {
  return draft.capability_values?.[capability]
}

function buildResolverCapabilities(draft: ProviderWizardInput['resolver_rule_drafts'][number]) {
  const capabilityValues = draft.capability_values ?? {}
  return Object.fromEntries((draft.capabilities ?? []).map((capability) => {
    const value = capabilityValues[capability]
    return [capability, typeof value === 'boolean' || typeof value === 'number' ? value : true]
  }))
}

function buildWizardCapabilities(source: ModelParamRuleRecord | undefined, draft: ProviderWizardInput['model_rule_drafts'][number]) {
  if (source && sameStringSet(draft.capabilities, Object.keys(source.capabilities))) {
    const sourceValues = readCapabilityValues(source.capabilities)
    const draftValues = Object.fromEntries(draft.capabilities.map((capability) => [capability, getDraftCapabilityValue(draft, capability)]))
    const sameValues = draft.capabilities.every((capability) => sourceValues[capability] === draftValues[capability])
    if (sameValues) {
      return source.capabilities
    }
  }
  return Object.fromEntries(draft.capabilities.map((capability) => {
    const value = getDraftCapabilityValue(draft, capability)
    return [capability, typeof value === 'boolean' || typeof value === 'number' ? value : true]
  }))
}

function buildModelRuleCapabilities(input: ModelRuleInput) {
  const capabilityValues = input.capability_values ?? {}
  return Object.fromEntries(input.capabilities.map((capability) => {
    const dictionary = seed.dictionaries.find((item) => item.kind === 'capability' && item.key === capability)
    if (dictionary?.value_type === 'number') {
      const value = capabilityValues[capability]
      return [capability, typeof value === 'number' ? value : 0]
    }
    return [capability, typeof capabilityValues[capability] === 'boolean' ? capabilityValues[capability] : true]
  }))
}

function directoryPathToLogicalMount(path: string) {
  return path.trim().replace(/^\/+|\/+$/g, '').replace(/\//g, '.')
}

function buildInputCapabilities(keys: string[], values?: Record<string, boolean | number>) {
  const capabilityValues = values ?? {}
  return Object.fromEntries(keys.map((capability) => {
    const dictionary = seed.dictionaries.find((item) => item.kind === 'capability' && item.key === capability)
    if (dictionary?.value_type === 'number') {
      const value = capabilityValues[capability]
      return [capability, typeof value === 'number' ? value : 0]
    }
    return [capability, typeof capabilityValues[capability] === 'boolean' ? capabilityValues[capability] : true]
  }))
}

function buildExcludeModelRulesFromSelection(input: SelectionRuleInput, now: number): ModelParamRuleRecord[] {
  if (input.rule_type === 'exclude_pattern') {
    const existing = seed.model_param_rules.find((item) => item.rule_key === input.rule_key)
    return [{
      rule_key: input.rule_key,
      provider_key: input.provider_key,
      source_rule_key: existing?.source_rule_key ?? null,
      match_type: 'pattern',
      original_provider: null,
      model_id_selector: input.selector,
      priority: input.priority,
      model_driver: existing?.model_driver ?? null,
      api_types: existing?.api_types ?? [],
      logical_mounts: existing?.logical_mounts ?? [],
      capabilities: existing?.capabilities ?? {},
      attributes: existing?.attributes ?? null,
      context_limits: existing?.context_limits ?? null,
      pricing: existing?.pricing ?? null,
      exclude: true,
      enabled: true,
      created_at: existing?.created_at ?? now,
      updated_at: now,
    }]
  }
  if (input.rule_type === 'exclude_origin') {
    return seed.model_param_rules
      .filter((rule) => rule.enabled && rule.match_type === 'exact' && rule.original_provider === input.selector && rule.model_id_selector)
      .map((source, index) => {
        const ruleKey = `${input.rule_key}-${safeRuleKeyPart(source.model_id_selector ?? '')}`
        const existing = seed.model_param_rules.find((item) => item.rule_key === ruleKey)
        return {
          rule_key: ruleKey,
          provider_key: input.provider_key,
          source_rule_key: existing?.source_rule_key ?? source.rule_key,
          match_type: 'exact',
          original_provider: input.selector,
          model_id_selector: source.model_id_selector,
          priority: null,
          model_driver: existing?.model_driver ?? source.model_driver,
          api_types: existing?.api_types ?? source.api_types,
          logical_mounts: existing?.logical_mounts ?? source.logical_mounts,
          capabilities: existing?.capabilities ?? source.capabilities,
          attributes: existing?.attributes ?? source.attributes,
          context_limits: existing?.context_limits ?? source.context_limits,
          pricing: existing?.pricing ?? source.pricing,
          exclude: true,
          enabled: true,
          created_at: existing?.created_at ?? now,
          updated_at: now + index,
        } satisfies ModelParamRuleRecord
      })
  }
  return []
}

export async function loadProviderCloudWorkspace() {
  const params = typeof window === 'undefined' ? new URLSearchParams() : new URLSearchParams(window.location.search)
  const scenario = params.get('mockState')
  const errorKey = params.get('mockErrorKey') ?? 'default'
  const errorCount = forcedWorkspaceErrorCounts.get(errorKey) ?? 0
  if (scenario === 'error-once' && errorCount < 2) {
    forcedWorkspaceErrorCounts.set(errorKey, errorCount + 1)
    await withMockLatency(null)
    throw new Error('Forced mock workspace load failure')
  }

  const data = structuredClone(seed)
  if (scenario === 'empty') {
    data.providers = []
    data.model_param_rules = []
    data.metadata_variants = []
    data.metadata_version_rules = []
    data.provider_model_rules = []
    data.model_nicks = []
    data.origin_provider_aliases = []
    data.origin_mapping_rules = []
    data.logical_directories = []
    data.pending_changes = []
    data.change_logs = []
    data.warnings = []
  }
  if (scenario === 'stale') {
    data.tech_source = {
      ...data.tech_source,
      stale: true,
      last_error: 'Mock source is stale for UI state verification',
    }
  }
  return withMockLatency(data)
}

export async function startEditSession(role: ServiceRole) {
  seed = {
    ...seed,
    edit_session: {
      session_id: `edit-${role}-20260710-preview`,
      service_role: role,
      operator_id: role === 'tech' ? 'alice' : 'ops-admin',
      base_revision: role === 'tech' ? seed.published_revision : seed.ops_revision,
      status: 'editing',
      pending_change_count: seed.pending_changes.length,
      created_at: Date.now(),
      updated_at: Date.now(),
    },
  }
  return withMockLatency(structuredClone(seed))
}

export async function previewPublish(): Promise<PublishPreview> {
  if (seed.edit_session) {
    seed = {
      ...seed,
      edit_session: {
        ...seed.edit_session,
        status: 'previewed',
        updated_at: Date.now(),
      },
    }
  }
  return withMockLatency(buildPublishPreview(seed))
}

export async function saveTechSource(input: TechSourceInput) {
  seed = {
    ...seed,
    tech_source: {
      ...seed.tech_source,
      service_url: input.service_url,
      last_error: null,
    },
  }
  appendPendingChange({
    change_key: 'pending-tech-source-url',
    target_type: 'tech_source',
    target_key: 'source-url',
    action: 'update',
    summary: 'Update technical source service URL',
    risk: 'info',
  })
  return withMockLatency(structuredClone(seed))
}

export async function testTechSourceConnection() {
  seed = {
    ...seed,
    tech_source: {
      ...seed.tech_source,
      last_sync_at: mockNow(),
      last_error: null,
    },
  }
  return withMockLatency(structuredClone(seed))
}

export async function syncTechSource() {
  const now = mockNow()
  const sourceRevision = nextRevision('tech')
  seed = {
    ...seed,
    source_revision: sourceRevision,
    tech_source: {
      ...seed.tech_source,
      source_revision: sourceRevision,
      cache_revision: sourceRevision,
      last_sync_at: now,
      last_success_at: now,
      last_error: null,
      stale: false,
    },
  }
  appendPendingChange({
    change_key: `pending-tech-source-sync-${sourceRevision}`,
    target_type: 'tech_source',
    target_key: sourceRevision,
    action: 'update',
    summary: `Sync technical source revision ${sourceRevision}`,
    risk: 'info',
  })
  return withMockLatency(structuredClone(seed))
}

export async function simulateTechSourceStale() {
  seed = {
    ...seed,
    tech_source: {
      ...seed.tech_source,
      stale: true,
      last_sync_at: mockNow(),
      last_error: 'Mock sync timeout; previous cache remains available',
    },
  }
  appendPendingChange({
    change_key: 'pending-tech-source-stale-confirmation',
    target_type: 'tech_source',
    target_key: seed.tech_source.cache_revision,
    action: 'update',
    summary: 'Confirm stale technical source cache before operations publish',
    risk: 'warning',
  })
  return withMockLatency(structuredClone(seed))
}

function upsertOpsOverlay(
  targetType: 'provider' | 'model_param_rule' | 'variants' | 'version_rules',
  targetKey: string,
  disabled: boolean,
  opsPatch: Record<string, unknown>,
) {
  const now = mockNow()
  const overlayKey = `ops-${targetType}-${targetKey}`
  const exists = seed.ops_overlays.some((overlay) => overlay.overlay_key === overlayKey)
  seed = {
    ...seed,
    ops_overlays: exists
      ? seed.ops_overlays.map((overlay) => overlay.overlay_key === overlayKey
        ? { ...overlay, disabled, ops_patch: opsPatch, updated_at: now }
        : overlay)
      : [{
          overlay_key: overlayKey,
          target_type: targetType,
          target_key: targetKey,
          disabled,
          ops_patch: opsPatch,
          created_at: now,
          updated_at: now,
        }, ...seed.ops_overlays],
  }
  appendPendingChange({
    change_key: `pending-${overlayKey}`,
    target_type: targetType,
    target_key: targetKey,
    action: 'update',
    summary: `Update operations overlay for ${targetKey}`,
    risk: disabled ? 'warning' : 'info',
  })
}

export async function saveProviderOpsOverlay(input: ProviderOpsInput): Promise<ProviderCloudSeed> {
  void input
  throw new Error('Operations parameters can only edit model, pattern, and default parameter overlays')
}

export async function saveModelOpsOverlay(input: ModelOpsInput) {
  upsertOpsOverlay('model_param_rule', input.rule_key, false, {
    pricing_override: {
      input: input.pricing_input,
      output: input.pricing_output,
    },
    routing_weight: input.routing_weight,
    cost_class: input.cost_class,
    latency_class: input.latency_class,
    quality_score: input.quality_score,
    recommendation_level: input.recommendation_level,
    display_priority: input.display_priority,
    rollout_strategy: input.rollout_strategy,
    ops_note: input.ops_note,
  })
  return withMockLatency(structuredClone(seed))
}

export async function saveResolverOpsOverlay(input: ResolverOpsOverlayInput): Promise<ProviderCloudSeed> {
  void input
  throw new Error('Operations parameters cannot edit variants or version rules')
}

function getBulkPricing(rule: ModelParamRuleRecord, input: OpsBulkOperationInput) {
  const overlay = getOpsOverlay(seed, 'model_param_rule', rule.rule_key)
  const existing = getOpsPatchValue<{ input: number; output: number } | null>(overlay, 'pricing_override', null)
  const baseCost = typeof rule.pricing?.estimated_cost_usd === 'number' ? rule.pricing.estimated_cost_usd : 0
  const current = existing ?? (baseCost ? { input: baseCost, output: baseCost * 2 } : null)
  if (input.action === 'clear_pricing') {
    return null
  }
  if (input.action === 'set_price') {
    return { input: input.pricing_input, output: input.pricing_output }
  }
  if (input.action === 'adjust_price_percent' && current) {
    const factor = 1 + input.price_percent / 100
    return {
      input: Number((current.input * factor).toFixed(8)),
      output: Number((current.output * factor).toFixed(8)),
    }
  }
  return existing
}

export async function applyOpsBulkOperation(input: OpsBulkOperationInput) {
  const targets = filterOpsBulkModelRules(seed, input)
  targets.forEach((rule) => {
    const overlay = getOpsOverlay(seed, 'model_param_rule', rule.rule_key)
    const disabled = false
    const pricing = getBulkPricing(rule, input)
    const opsPatch: Record<string, unknown> = {
      ...overlay?.ops_patch,
      ...(pricing ? { pricing_override: pricing } : {}),
      ...(input.action === 'clear_pricing' ? { pricing_override: undefined } : {}),
      ...(input.action === 'set_routing_weight' ? { routing_weight: input.routing_weight } : {}),
      ...(input.action === 'set_recommendation' ? { recommendation_level: input.target_recommendation_level } : {}),
      ...(input.action === 'set_display_priority' ? { display_priority: input.display_priority } : {}),
    }
    Object.keys(opsPatch).forEach((key) => {
      if (opsPatch[key] === undefined) {
        delete opsPatch[key]
      }
    })
    upsertOpsOverlay('model_param_rule', rule.rule_key, disabled, opsPatch)
  })
  appendPendingChange({
    change_key: `pending-ops-bulk-${mockNow()}`,
    target_type: 'model_param_rule',
    target_key: `bulk-${targets.length}`,
    action: 'update',
    summary: `Apply operations bulk action ${input.action} to ${targets.length} model rules`,
    risk: targets.length > 20 || input.action === 'adjust_price_percent' ? 'warning' : 'info',
  })
  return withMockLatency(structuredClone(seed))
}

function extractImportActionNames(text: string) {
  const names: string[] = []
  const actionLinePattern = /(?:^|\s)(?:action|type)\s*:\s*["']?([a-z0-9_.-]+)["']?/gim
  for (const match of text.matchAll(actionLinePattern)) {
    names.push(match[1])
  }
  supportedImportActions.forEach((action) => {
    const actionPattern = new RegExp(`(^|[^a-z0-9_])${action}([^a-z0-9_]|$)`, 'i')
    if (actionPattern.test(text) && !names.includes(action)) {
      names.push(action)
    }
  })
  const unsupportedPattern = /unsupported[_-][a-z0-9_-]+/gi
  for (const match of text.matchAll(unsupportedPattern)) {
    names.push(match[0].replace(/-/g, '_'))
  }
  return names.length ? names : ['upsert_provider', 'upsert_model_param_rule', 'set_model_nick']
}

function previewSelectorHits(selector: string | null) {
  if (!selector) {
    return { hit_count: 0, samples: [] }
  }
  if (selector.startsWith('/')) {
    const hits = seed.logical_directories.filter((directory) => directory.path.startsWith(selector))
    return { hit_count: hits.length, samples: hits.slice(0, 3).map((item) => item.path) }
  }
  const pattern = selector.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*')
  const matcher = new RegExp(`^${pattern}$`, 'i')
  const hits = seed.model_param_rules.filter((rule) => rule.model_id_selector && matcher.test(rule.model_id_selector))
  return {
    hit_count: hits.length,
    samples: hits.slice(0, 4).map((item) => item.model_id_selector ?? item.rule_key),
  }
}

function getDictionaryReferenceDetails(kind: DictionaryItem['kind'], key: string) {
  const rows = kind === 'api_type'
    ? seed.model_param_rules.filter((rule) => rule.api_types.includes(key))
    : seed.model_param_rules.filter((rule) => Object.prototype.hasOwnProperty.call(rule.capabilities, key))
  return {
    affected_count: rows.length,
    reference_samples: rows.slice(0, 4).map((rule) => rule.rule_key),
  }
}

function getDirectoryReferenceDetails(directoryKey: string) {
  const directory = seed.logical_directories.find((item) => item.directory_key === directoryKey)
  if (!directory) {
    return { affected_count: 0, reference_samples: [] }
  }
  const children = seed.logical_directories.filter((item) => item.parent_key === directoryKey)
  return {
    affected_count: directory.model_rule_keys.length + children.length,
    reference_samples: [...directory.model_rule_keys.slice(0, 3), ...children.slice(0, 2).map((item) => item.path)],
  }
}

function getActionImpact(action: ImportPlanActionName, spec: ImportActionSpec, hits: { hit_count: number, samples: string[] }) {
  if (action === 'delete_api_type') {
    return getDictionaryReferenceDetails('api_type', spec.target_key)
  }
  if (action === 'delete_capability') {
    return getDictionaryReferenceDetails('capability', spec.target_key)
  }
  if (action === 'delete_logical_directory' || action === 'move_logical_directory') {
    return getDirectoryReferenceDetails(spec.target_key)
  }
  if (action === 'disable_provider') {
    const affected = seed.model_param_rules.filter((rule) => rule.provider_key === spec.target_key)
    return {
      affected_count: affected.length,
      reference_samples: affected.slice(0, 4).map((rule) => rule.rule_key),
    }
  }
  if (action === 'delete_model_param_rule') {
    const mountedIn = seed.logical_directories.filter((directory) => directory.model_rule_keys.includes(spec.target_key))
    return {
      affected_count: 1 + mountedIn.length,
      reference_samples: [spec.target_key, ...mountedIn.map((directory) => directory.path)],
    }
  }
  return {
    affected_count: hits.hit_count,
    reference_samples: hits.samples,
  }
}

function parseImportPlan(input: ImportPlanInput): ImportPlanParseResult {
  const now = mockNow()
  const actions = extractImportActionNames(input.text).map((rawAction, index): ImportPlanActionRecord => {
    const supported = supportedImportActions.includes(rawAction as ImportPlanActionName)
    const actionName = rawAction as ImportPlanActionName
    const spec = supported ? importActionSpecs[actionName] : null
    const hits = previewSelectorHits(spec?.selector ?? null)
    const impact = supported && spec ? getActionImpact(actionName, spec, hits) : { affected_count: 0, reference_samples: [] }
    const destructive = rawAction.startsWith('delete_') || rawAction.startsWith('disable_') || rawAction.startsWith('move_')
    return {
      action_key: `plan-action-${index + 1}`,
      action: supported ? actionName : 'unsupported',
      raw_action: rawAction,
      target_type: spec?.target_type ?? 'unknown',
      target_key: spec?.target_key ?? rawAction,
      selector: spec?.selector ?? null,
      match_type: spec?.match_type ?? null,
      priority: spec?.priority ?? null,
      ...hits,
      affected_count: impact.affected_count,
      reference_samples: impact.reference_samples,
      published_selector: spec?.published_selector ?? null,
      fallback_behavior: spec?.fallback_behavior ?? null,
      source_record: spec?.source_record ?? null,
      field_changes: spec?.field_changes ?? [],
      risk: supported ? (destructive ? 'warning' : 'info') : 'blocked',
      summary: supported ? `${rawAction} -> ${spec?.target_key}` : `Unsupported action: ${rawAction}`,
      errors: supported ? [] : [`Unsupported action "${rawAction}" is not applied`],
    }
  })
  const warnings: WarningRecord[] = actions
    .filter((action) => action.risk !== 'info')
    .map((action) => ({
      warning_key: `import-${action.action_key}`,
      severity: action.risk,
      target_type: action.target_type,
      target_key: action.target_key,
      message_key: action.action === 'unsupported' ? 'import.unsupportedAction' : 'import.riskWarning',
      detail: action.summary,
      created_at: now,
    }))
  return {
    plan_id: `plan-${now}`,
    title: input.title,
    action_count: actions.length,
    supported_count: actions.filter((action) => action.action !== 'unsupported').length,
    error_count: actions.reduce((count, action) => count + action.errors.length, 0),
    actions,
    warnings,
  }
}

function ensureEditSession() {
  if (!seed.edit_session) {
    seed = {
      ...seed,
      edit_session: {
        session_id: 'edit-tech-import-plan',
        service_role: 'tech',
        operator_id: 'alice',
        base_revision: seed.published_revision,
        status: 'editing',
        pending_change_count: seed.pending_changes.length,
        created_at: mockNow(),
        updated_at: mockNow(),
      },
    }
  }
}

function applyPlanAction(action: ImportPlanActionRecord, index: number) {
  const now = mockNow()
  if (action.action === 'unsupported') {
    return
  }
  if (action.action === 'upsert_provider') {
    const provider: ProviderRecord = {
      provider_key: action.target_key,
      provider_driver: 'plan-openrouter',
      name: 'Plan OpenRouter',
      base_url: 'https://openrouter.ai/api/v1',
      provider_kind: 'aggregator',
      protocol_family: 'openai-compatible',
      enabled: true,
      owner_service: 'tech',
      revision: 'draft',
      created_at: now,
      updated_at: now,
    }
    seed = {
      ...seed,
      providers: [provider, ...seed.providers.filter((item) => item.provider_key !== provider.provider_key)],
    }
  }
  if (action.action === 'disable_provider') {
    seed = {
      ...seed,
      providers: seed.providers.map((provider) => provider.provider_key === action.target_key ? { ...provider, enabled: false, updated_at: now } : provider),
    }
  }
  if (action.action === 'upsert_model_param_rule') {
    const rule: ModelParamRuleRecord = {
      rule_key: action.target_key,
      provider_key: 'plan-openrouter',
      source_rule_key: null,
      match_type: action.match_type ?? 'pattern',
      original_provider: 'openai',
      model_id_selector: action.selector,
      priority: action.priority ?? 10 + index,
      model_driver: 'openai',
      api_types: ['llm'],
      logical_mounts: ['/llm'],
      capabilities: { streaming: true, tool_call: true },
      attributes: null,
      context_limits: null,
      pricing: null,
      exclude: false,
      enabled: true,
      created_at: now,
      updated_at: now,
    }
    seed = {
      ...seed,
      model_param_rules: [rule, ...seed.model_param_rules.filter((item) => item.rule_key !== rule.rule_key)],
    }
  }
  if (action.action === 'delete_model_param_rule') {
    seed = {
      ...seed,
      model_param_rules: seed.model_param_rules.filter((rule) => rule.rule_key !== action.target_key),
      logical_directories: seed.logical_directories.map((directory) => ({
        ...directory,
        model_rule_keys: directory.model_rule_keys.filter((ruleKey) => ruleKey !== action.target_key),
        updated_at: directory.model_rule_keys.includes(action.target_key) ? now : directory.updated_at,
      })),
    }
  }
  if (action.action === 'include_models') {
    const rule: ProviderModelRuleRecord = {
      rule_key: action.target_key,
      provider_key: 'plan-openrouter',
      rule_type: 'include_pattern',
      selector: action.selector ?? 'gpt-*',
      priority: 20 + index,
      enabled: true,
      created_at: now,
      updated_at: now,
    }
    seed = {
      ...seed,
      provider_model_rules: [rule, ...seed.provider_model_rules.filter((item) => item.rule_key !== rule.rule_key)],
    }
  }
  if (action.action === 'exclude_models') {
    const excludeInput: SelectionRuleInput = {
      rule_key: action.target_key,
      provider_key: 'plan-openrouter',
      rule_type: 'exclude_pattern',
      selector: action.selector ?? 'gpt-*',
      priority: 20 + index,
    }
    const rules = buildExcludeModelRulesFromSelection(excludeInput, now)
    const ruleKeys = new Set(rules.map((rule) => rule.rule_key))
    seed = {
      ...seed,
      model_param_rules: [...rules, ...seed.model_param_rules.filter((rule) => !ruleKeys.has(rule.rule_key))],
    }
  }
  if (action.action === 'set_model_nick') {
    const nick: ModelNickRecord = {
      nick_key: action.target_key,
      provider_key: 'plan-openrouter',
      original_provider: 'openai',
      model_id: 'gpt-*',
      nick: 'plan/{model}',
      selector_type: 'pattern',
      priority: 10,
      created_at: now,
      updated_at: now,
    }
    seed = {
      ...seed,
      model_nicks: [nick, ...seed.model_nicks.filter((item) => item.nick_key !== nick.nick_key)],
    }
  }
  if (action.action === 'upsert_variant') {
    const variant: MetadataVariantRecord = {
      variant_key: action.target_key,
      provider_key: 'plan-openrouter',
      source_variant_key: null,
      selector_type: 'pattern',
      original_provider: 'openai',
      model_id_selector: 'gpt-*',
      priority: action.priority ?? 11,
      nick: action.published_selector,
      content: { variant: 'plan-import' },
      enabled: true,
      created_at: now,
      updated_at: now,
    }
    seed = {
      ...seed,
      metadata_variants: [variant, ...seed.metadata_variants.filter((item) => item.variant_key !== variant.variant_key)],
    }
  }
  if (action.action === 'upsert_version_rule') {
    const versionRule: MetadataVersionRuleRecord = {
      version_rule_key: action.target_key,
      provider_key: 'plan-openrouter',
      source_version_rule_key: null,
      selector_type: 'pattern',
      original_provider: 'openai',
      model_id_selector: 'gpt-*',
      priority: action.priority ?? 12,
      nick: action.published_selector,
      content: { version: 'plan-import' },
      enabled: true,
      created_at: now,
      updated_at: now,
    }
    seed = {
      ...seed,
      metadata_version_rules: [versionRule, ...seed.metadata_version_rules.filter((item) => item.version_rule_key !== versionRule.version_rule_key)],
    }
  }
  if (action.action === 'upsert_logical_directory' || action.action === 'set_logical_mounts' || action.action === 'move_logical_directory') {
    const matchedRuleKeys = seed.model_param_rules
      .filter((rule) => action.selector && rule.model_id_selector && previewSelectorHits(action.selector).samples.includes(rule.model_id_selector))
      .map((rule) => rule.rule_key)
    if (action.action === 'set_logical_mounts') {
      seed = {
        ...seed,
        logical_directories: seed.logical_directories.map((directory) => {
          if (directory.directory_key !== action.target_key) {
            return directory
          }
          return {
            ...directory,
            model_rule_keys: Array.from(new Set([...directory.model_rule_keys, ...matchedRuleKeys])),
            updated_at: now,
          }
        }),
      }
    } else {
      const existing = seed.logical_directories.find((item) => item.directory_key === action.target_key)
      const directory: LogicalDirectoryRecord = {
        directory_key: action.target_key,
        path: action.published_selector ?? `/import/${index + 1}`,
        title: action.action === 'move_logical_directory' ? 'Image' : `Import ${index + 1}`,
        parent_key: action.action === 'move_logical_directory' ? 'llm' : null,
        model_rule_keys: existing?.model_rule_keys ?? matchedRuleKeys,
        created_at: existing?.created_at ?? now,
        updated_at: now,
      }
      seed = {
        ...seed,
        logical_directories: [directory, ...seed.logical_directories.filter((item) => item.directory_key !== directory.directory_key)],
      }
    }
  }
  if (action.action === 'delete_logical_directory') {
    seed = {
      ...seed,
      logical_directories: seed.logical_directories
        .filter((directory) => directory.directory_key !== action.target_key && directory.parent_key !== action.target_key)
        .map((directory) => directory.parent_key === action.target_key ? { ...directory, parent_key: null, updated_at: now } : directory),
    }
  }
  if (action.action === 'set_api_types') {
    seed = {
      ...seed,
      model_param_rules: seed.model_param_rules.map((rule) => {
        if (!action.selector || !rule.model_id_selector || !previewSelectorHits(action.selector).samples.includes(rule.model_id_selector)) {
          return rule
        }
        return {
          ...rule,
          api_types: Array.from(new Set([...rule.api_types, 'llm', 'plan.chat'])),
          updated_at: now,
        }
      }),
    }
  }
  if (action.action === 'upsert_api_type' || action.action === 'upsert_capability') {
    const item: DictionaryItem = {
      key: action.target_key,
      label: action.action === 'upsert_api_type' ? 'Plan chat' : 'Plan cached tokens',
      kind: action.action === 'upsert_api_type' ? 'api_type' : 'capability',
      value_type: action.action === 'upsert_api_type' ? 'boolean' : 'number',
      referenced_by: 0,
    }
    seed = {
      ...seed,
      dictionaries: [item, ...seed.dictionaries.filter((entry) => entry.kind !== item.kind || entry.key !== item.key)],
    }
  }
  if (action.action === 'delete_api_type') {
    seed = {
      ...seed,
      dictionaries: seed.dictionaries.filter((item) => item.kind !== 'api_type' || item.key !== action.target_key),
      model_param_rules: seed.model_param_rules.map((rule) => ({
        ...rule,
        api_types: rule.api_types.filter((apiType) => apiType !== action.target_key),
        updated_at: rule.api_types.includes(action.target_key) ? now : rule.updated_at,
      })),
    }
  }
  if (action.action === 'set_capabilities') {
    seed = {
      ...seed,
      model_param_rules: seed.model_param_rules.map((rule) => {
        if (!action.selector || !rule.model_id_selector || !previewSelectorHits(action.selector).samples.includes(rule.model_id_selector)) {
          return rule
        }
        return {
          ...rule,
          capabilities: { ...rule.capabilities, streaming: true, plan_cached_tokens: 8192 },
          updated_at: now,
        }
      }),
    }
  }
  if (action.action === 'delete_capability') {
    seed = {
      ...seed,
      dictionaries: seed.dictionaries.filter((item) => item.kind !== 'capability' || item.key !== action.target_key),
      model_param_rules: seed.model_param_rules.map((rule) => {
        if (!Object.prototype.hasOwnProperty.call(rule.capabilities, action.target_key)) {
          return rule
        }
        const capabilities = { ...rule.capabilities }
        delete capabilities[action.target_key]
        return { ...rule, capabilities, updated_at: now }
      }),
    }
  }
  appendPendingChange({
    change_key: `pending-import-${action.action_key}`,
    target_type: action.target_type,
    target_key: action.target_key,
    action: action.action.startsWith('delete_') ? 'delete' : action.action.startsWith('disable_') ? 'disable' : 'update',
    summary: action.summary,
    risk: action.risk,
  })
}

export async function applyImportPlan(input: ImportPlanInput) {
  ensureEditSession()
  const parseResult = parseImportPlan(input)
  parseResult.actions.forEach(applyPlanAction)
  seed = {
    ...seed,
    import_plan_result: parseResult,
    warnings: [
      ...parseResult.warnings,
      ...seed.warnings.filter((warning) => !warning.warning_key.startsWith('import-plan-action-')),
    ],
  }
  return withMockLatency(structuredClone(seed))
}

export async function saveImportPlanDraft(input: ImportPlanDraftInput) {
  const parseResult = parseImportPlan(input)
  seed = {
    ...seed,
    import_plan_result: parseResult,
    import_plan_draft: {
      draft_id: input.plan_id,
      title: input.title,
      text: input.text,
      parse_result: parseResult,
      workspace: structuredClone(seed),
      saved_at: mockNow(),
    },
  }
  return withMockLatency(structuredClone(seed))
}

export async function restoreImportPlanDraft() {
  if (seed.import_plan_draft) {
    const draft = structuredClone(seed.import_plan_draft)
    seed = structuredClone(seed.import_plan_draft.workspace)
    seed = {
      ...seed,
      import_plan_draft: draft,
      import_plan_result: draft.parse_result,
    }
  }
  return withMockLatency(structuredClone(seed))
}

export async function discardImportPlanDraft() {
  seed = {
    ...seed,
    import_plan_result: null,
    import_plan_draft: null,
  }
  return withMockLatency(structuredClone(seed))
}

export async function simulateRevisionConflict() {
  const role = seed.edit_session?.service_role ?? 'tech'
  seed = {
    ...seed,
    published_revision: role === 'tech' ? `tech-20260710.${String(Math.floor(Math.random() * 900000) + 100000)}` : seed.published_revision,
    ops_revision: role === 'ops' ? `ops-20260710.${String(Math.floor(Math.random() * 900000) + 100000)}` : seed.ops_revision,
    revision_conflict: true,
  }
  return withMockLatency(structuredClone(seed))
}

export async function refreshPublishBaseRevision() {
  const role = seed.edit_session?.service_role ?? 'tech'
  seed = {
    ...seed,
    revision_conflict: false,
    edit_session: seed.edit_session ? {
      ...seed.edit_session,
      base_revision: role === 'ops' ? seed.ops_revision : seed.published_revision,
      status: 'previewed',
      updated_at: mockNow(),
    } : seed.edit_session,
  }
  return withMockLatency(structuredClone(seed))
}

export async function publishPendingChanges(input: PublishWizardInput) {
  if (!seed.edit_session) {
    throw new Error('No edit session to publish')
  }
  const role = seed.edit_session.service_role
  const currentRevision = role === 'ops' ? seed.ops_revision : seed.published_revision
  if (seed.revision_conflict || seed.edit_session.base_revision !== currentRevision) {
    throw new Error('Base revision is stale. Refresh preview before publishing.')
  }
  if (seed.tech_source.stale && !input.confirm_stale_publish) {
    throw new Error('Technical source cache is stale. Confirm stale publish before publishing.')
  }
  const now = mockNow()
  const revisionNumber = Number(currentRevision.split('.').at(-1) ?? '1') + 1
  const nextRevision = `${role === 'ops' ? 'ops' : 'tech'}-20260710.${String(revisionNumber).padStart(6, '0')}`
  const changeLog: ChangeLogRecord = {
    change_id: `chg-20260710-${String(revisionNumber).padStart(4, '0')}`,
    service_role: role,
    operator_id: seed.edit_session.operator_id,
    from_revision: currentRevision,
    to_revision: nextRevision,
    source_revision: seed.source_revision,
    source_stale: seed.tech_source.stale,
    summary: input.release_note,
    created_at: now,
  }
  seed = {
    ...seed,
    published_revision: role === 'tech' ? nextRevision : seed.published_revision,
    source_revision: role === 'tech' ? nextRevision : seed.source_revision,
    ops_revision: role === 'ops' ? nextRevision : seed.ops_revision,
    tech_source: {
      ...seed.tech_source,
      ops_revision: role === 'ops' ? nextRevision : seed.tech_source.ops_revision,
    },
    edit_session: null,
    pending_changes: [],
    change_logs: [changeLog, ...seed.change_logs],
    import_plan_result: null,
    revision_conflict: false,
  }
  return withMockLatency(structuredClone(seed))
}

export async function saveProvider(input: ProviderInput) {
  const now = mockNow()
  const template = seed.providers.find((provider) => provider.provider_key === input.template_provider_key)
  const provider: ProviderRecord = {
    provider_key: input.provider_key,
    provider_driver: input.provider_driver,
    name: input.name,
    base_url: input.base_url || null,
    provider_kind: input.provider_kind,
    protocol_family: input.protocol_family || null,
    enabled: true,
    owner_service: 'tech',
    revision: 'draft',
    created_at: template?.created_at ?? now,
    updated_at: now,
  }
  const exists = seed.providers.some((item) => item.provider_key === provider.provider_key)
  seed = {
    ...seed,
    providers: exists
      ? seed.providers.map((item) => (item.provider_key === provider.provider_key ? provider : item))
      : [provider, ...seed.providers],
  }
  appendPendingChange({
    change_key: `pending-provider-${provider.provider_key}`,
    target_type: 'provider',
    target_key: provider.provider_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} provider ${provider.provider_key}`,
    risk: exists ? 'warning' : 'info',
  })
  return withMockLatency(structuredClone(seed))
}

function appendModelDependencyWarnings(sourceKey: string, ownerProvider: string | null) {
  const dependents = Array.from(new Set(seed.model_param_rules
    .filter((rule) => rule.provider_key && rule.provider_key !== ownerProvider && rule.source_rule_key === sourceKey)
    .map((rule) => rule.provider_key as string)))
  dependents.forEach((providerKey) => appendPendingChange({
    change_key: `pending-provider-sync-model-${sourceKey}-${providerKey}`,
    target_type: 'provider',
    target_key: providerKey,
    action: 'update',
    summary: `${providerKey} reuses model rule ${sourceKey}; review and synchronize its copied configuration.`,
    risk: 'warning',
  }))
}

export async function saveModelRule(input: ModelRuleInput) {
  const now = mockNow()
  const existing = seed.model_param_rules.find((item) => item.rule_key === input.rule_key)
  const providerKey = input.scope === 'provider' ? input.provider_key : null
  const rule: ModelParamRuleRecord = {
    rule_key: input.rule_key,
    provider_key: providerKey || null,
    source_rule_key: existing?.source_rule_key ?? null,
    match_type: input.match_type,
    original_provider: input.original_provider || null,
    model_id_selector: input.match_type === 'default' ? null : input.model_id_selector,
    priority: input.match_type === 'pattern' ? input.priority : null,
    model_driver: input.model_driver,
    api_types: input.api_types,
    logical_mounts: input.logical_mounts.map(directoryPathToLogicalMount),
    capabilities: buildModelRuleCapabilities(input),
    attributes: {
      ...(existing?.attributes ?? {}),
      quality_score: input.quality_score ?? null,
      latency_class: input.latency_class ?? null,
      cost_class: input.cost_class ?? null,
    },
    context_limits: existing?.context_limits ?? null,
    pricing: {
      ...(existing?.pricing ?? {}),
      estimated_cost_usd: input.estimated_cost_usd ?? null,
      estimated_latency_ms: input.estimated_latency_ms ?? null,
    },
    exclude: input.match_type === 'default' ? false : input.exclude,
    enabled: true,
    created_at: existing?.created_at ?? now,
    updated_at: now,
  }
  const exists = Boolean(existing)
  seed = {
    ...seed,
    model_param_rules: exists
      ? seed.model_param_rules.map((item) => (item.rule_key === rule.rule_key ? rule : item))
      : [rule, ...seed.model_param_rules],
  }
  appendPendingChange({
    change_key: `pending-model-${rule.rule_key}`,
    target_type: 'model_param_rule',
    target_key: rule.rule_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} ${rule.match_type} model rule ${rule.rule_key}`,
    risk: rule.match_type === 'default' || rule.exclude ? 'warning' : 'info',
  })
  if (exists) appendModelDependencyWarnings(rule.rule_key, rule.provider_key)
  return withMockLatency(structuredClone(seed))
}

export async function saveSelectionRule(input: SelectionRuleInput) {
  const now = mockNow()
  if (input.rule_type === 'exclude_origin' || input.rule_type === 'exclude_pattern') {
    const rules = buildExcludeModelRulesFromSelection(input, now)
    const ruleKeys = new Set(rules.map((rule) => rule.rule_key))
    seed = {
      ...seed,
      model_param_rules: [...rules, ...seed.model_param_rules.filter((rule) => !ruleKeys.has(rule.rule_key))],
    }
    appendPendingChange({
      change_key: `pending-selection-exclude-${input.rule_key}`,
      target_type: 'model_param_rule',
      target_key: input.rule_key,
      action: rules.length ? 'update' : 'create',
      summary: `Create ${rules.length} exclude model rule${rules.length === 1 ? '' : 's'} from selection ${input.rule_key}`,
      risk: 'warning',
    })
    return withMockLatency(structuredClone(seed))
  }
  const rule: ProviderModelRuleRecord = {
    rule_key: input.rule_key,
    provider_key: input.provider_key,
    rule_type: input.rule_type,
    selector: input.selector,
    priority: input.priority,
    enabled: true,
    created_at: now,
    updated_at: now,
  }
  const exists = seed.provider_model_rules.some((item) => item.rule_key === rule.rule_key)
  seed = {
    ...seed,
    provider_model_rules: exists
      ? seed.provider_model_rules.map((item) => (item.rule_key === rule.rule_key ? rule : item))
      : [rule, ...seed.provider_model_rules],
  }
  appendPendingChange({
    change_key: `pending-selection-${rule.rule_key}`,
    target_type: 'selection_rule',
    target_key: rule.rule_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} selection rule ${rule.rule_key}`,
    risk: 'info',
  })
  return withMockLatency(structuredClone(seed))
}

export async function saveNickRule(input: NickRuleInput) {
  const now = mockNow()
  const rule: ModelNickRecord = {
    nick_key: input.nick_key,
    provider_key: input.provider_key,
    original_provider: input.original_provider || null,
    model_id: input.model_id,
    nick: input.nick,
    selector_type: input.selector_type,
    priority: input.priority,
    created_at: now,
    updated_at: now,
  }
  const exists = seed.model_nicks.some((item) => item.nick_key === rule.nick_key)
  seed = {
    ...seed,
    model_nicks: exists
      ? seed.model_nicks.map((item) => (item.nick_key === rule.nick_key ? rule : item))
      : [rule, ...seed.model_nicks],
  }
  appendPendingChange({
    change_key: `pending-nick-${rule.nick_key}`,
    target_type: 'nick_rule',
    target_key: rule.nick_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} nick rule ${rule.nick_key}`,
    risk: 'warning',
  })
  return withMockLatency(structuredClone(seed))
}

export async function deleteNickRule(nickKey: string) {
  seed = { ...seed, model_nicks: seed.model_nicks.filter((rule) => rule.nick_key !== nickKey) }
  appendPendingChange({ change_key: `pending-delete-nick-${nickKey}`, target_type: 'nick_rule', target_key: nickKey, action: 'delete', summary: `Delete nick rule ${nickKey}`, risk: 'warning' })
  return withMockLatency(structuredClone(seed))
}

export async function saveOriginProviderAlias(input: OriginProviderAliasInput) {
  const now = mockNow()
  const alias: OriginProviderAliasRecord = {
    alias_key: input.alias_key,
    provider_key: input.provider_key,
    alias: input.alias,
    driver: input.driver,
    created_at: now,
    updated_at: now,
  }
  const exists = seed.origin_provider_aliases.some((item) => item.alias_key === alias.alias_key)
  seed = {
    ...seed,
    origin_provider_aliases: exists
      ? seed.origin_provider_aliases.map((item) => (item.alias_key === alias.alias_key ? alias : item))
      : [alias, ...seed.origin_provider_aliases],
  }
  appendPendingChange({
    change_key: `pending-origin-alias-${alias.alias_key}`,
    target_type: 'origin_provider_alias',
    target_key: alias.alias_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} origin provider alias ${alias.alias} -> ${alias.driver}`,
    risk: 'warning',
  })
  return withMockLatency(structuredClone(seed))
}

export async function deleteOriginProviderAlias(aliasKey: string) {
  seed = { ...seed, origin_provider_aliases: seed.origin_provider_aliases.filter((rule) => rule.alias_key !== aliasKey) }
  appendPendingChange({
    change_key: `pending-delete-origin-alias-${aliasKey}`,
    target_type: 'origin_provider_alias',
    target_key: aliasKey,
    action: 'delete',
    summary: `Delete origin provider alias ${aliasKey}`,
    risk: 'warning',
  })
  return withMockLatency(structuredClone(seed))
}

export async function saveOriginMappingRule(input: OriginMappingRuleInput) {
  const now = mockNow()
  const rule: OriginMappingRuleRecord = {
    mapping_key: input.mapping_key,
    provider_key: input.provider_key,
    mapping_mode: input.mapping_mode,
    match_pattern: input.match_pattern,
    origin_template: input.origin_template,
    regex: input.regex,
    driver_transforms: input.driver_transforms,
    model_transforms: input.model_transforms,
    priority: input.priority,
    created_at: now,
    updated_at: now,
  }
  const exists = seed.origin_mapping_rules.some((item) => item.mapping_key === rule.mapping_key)
  seed = {
    ...seed,
    origin_mapping_rules: exists
      ? seed.origin_mapping_rules.map((item) => (item.mapping_key === rule.mapping_key ? rule : item))
      : [rule, ...seed.origin_mapping_rules],
  }
  appendPendingChange({
    change_key: `pending-origin-mapping-${rule.mapping_key}`,
    target_type: 'origin_mapping_rule',
    target_key: rule.mapping_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} origin mapping ${rule.mapping_key}`,
    risk: 'warning',
  })
  return withMockLatency(structuredClone(seed))
}

export async function deleteOriginMappingRule(mappingKey: string) {
  seed = { ...seed, origin_mapping_rules: seed.origin_mapping_rules.filter((rule) => rule.mapping_key !== mappingKey) }
  appendPendingChange({
    change_key: `pending-delete-origin-mapping-${mappingKey}`,
    target_type: 'origin_mapping_rule',
    target_key: mappingKey,
    action: 'delete',
    summary: `Delete origin mapping ${mappingKey}`,
    risk: 'warning',
  })
  return withMockLatency(structuredClone(seed))
}

export async function deleteModelRule(input: DeleteModelRuleInput) {
  const deleted = seed.model_param_rules.find((item) => item.rule_key === input.rule_key)
  const exists = Boolean(deleted)
  seed = {
    ...seed,
    model_param_rules: seed.model_param_rules.filter((item) => item.rule_key !== input.rule_key),
  }
  if (exists) {
    appendPendingChange({
      change_key: `pending-delete-model-${input.rule_key}`,
      target_type: 'model_param_rule',
      target_key: input.rule_key,
      action: 'delete',
      summary: `Delete exact model rule ${input.rule_key}`,
      risk: 'warning',
    })
    appendModelDependencyWarnings(input.rule_key, deleted?.provider_key ?? null)
  }
  return withMockLatency(structuredClone(seed))
}

export async function deleteProvider(providerKey: string) {
  seed = {
    ...seed,
    providers: seed.providers.filter((provider) => provider.provider_key !== providerKey),
    model_param_rules: seed.model_param_rules.filter((rule) => rule.provider_key !== providerKey),
    metadata_variants: seed.metadata_variants.filter((rule) => rule.provider_key !== providerKey),
    metadata_version_rules: seed.metadata_version_rules.filter((rule) => rule.provider_key !== providerKey),
    model_nicks: seed.model_nicks.filter((rule) => rule.provider_key !== providerKey),
    origin_provider_aliases: seed.origin_provider_aliases.filter((rule) => rule.provider_key !== providerKey),
    origin_mapping_rules: seed.origin_mapping_rules.filter((rule) => rule.provider_key !== providerKey),
    provider_model_rules: seed.provider_model_rules.filter((rule) => rule.provider_key !== providerKey),
  }
  appendPendingChange({ change_key: `pending-delete-provider-${providerKey}`, target_type: 'provider', target_key: providerKey, action: 'delete', summary: `Delete provider ${providerKey}`, risk: 'blocked' })
  return withMockLatency(structuredClone(seed))
}

export async function saveResolverRule(input: ResolverRuleInput) {
  const now = mockNow()
  const existing = seed.model_param_rules.find((item) => item.rule_key === input.rule_key)
  const rule: ModelParamRuleRecord = {
    rule_key: input.rule_key,
    provider_key: input.scope === 'provider' ? input.provider_key : null,
    source_rule_key: existing?.source_rule_key ?? null,
    match_type: input.match_type,
    original_provider: input.original_provider || null,
    model_id_selector: input.match_type === 'default' ? null : input.model_id_selector,
    priority: input.match_type === 'pattern' ? input.priority : null,
    model_driver: input.model_driver,
    api_types: [input.api_type],
    logical_mounts: existing?.logical_mounts ?? [],
    capabilities: { [input.capability]: true },
    attributes: existing?.attributes ?? null,
    context_limits: existing?.context_limits ?? null,
    pricing: existing?.pricing ?? null,
    exclude: existing?.exclude ?? false,
    enabled: true,
    created_at: existing?.created_at ?? now,
    updated_at: now,
  }
  const exists = Boolean(existing)
  seed = {
    ...seed,
    model_param_rules: exists
      ? seed.model_param_rules.map((item) => (item.rule_key === rule.rule_key ? rule : item))
      : [rule, ...seed.model_param_rules],
  }
  appendPendingChange({
    change_key: `pending-resolver-${rule.rule_key}`,
    target_type: 'model_param_rule',
    target_key: rule.rule_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} ${rule.match_type} resolver rule ${rule.rule_key}`,
    risk: rule.match_type === 'default' ? 'warning' : 'info',
  })
  return withMockLatency(structuredClone(seed))
}

function parseContentPatch(text: string) {
  const trimmed = text.trim()
  if (!trimmed) {
    return {}
  }
  try {
    const parsed = JSON.parse(trimmed)
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, unknown> : {}
  } catch {
    return { raw_content_patch: trimmed }
  }
}

function appendResolverDependencyWarnings(kind: 'variant' | 'version_rule', sourceKey: string, ownerProvider: string | null) {
  const dependents = kind === 'variant'
    ? seed.metadata_variants.filter((rule) => rule.provider_key && rule.provider_key !== ownerProvider && rule.source_variant_key === sourceKey).map((rule) => rule.provider_key as string)
    : seed.metadata_version_rules.filter((rule) => rule.provider_key && rule.provider_key !== ownerProvider && rule.source_version_rule_key === sourceKey).map((rule) => rule.provider_key as string)
  Array.from(new Set(dependents)).forEach((providerKey) => appendPendingChange({
    change_key: `pending-provider-sync-${kind}-${sourceKey}-${providerKey}`,
    target_type: 'provider',
    target_key: providerKey,
    action: 'update',
    summary: `${providerKey} reuses ${kind} ${sourceKey}; review and synchronize its copied configuration.`,
    risk: 'warning',
  }))
}

export async function saveMetadataVariant(input: MetadataVariantInput) {
  const now = mockNow()
  const existing = seed.metadata_variants.find((item) => item.variant_key === input.variant_key)
  const contentPatch = parseContentPatch(input.content_json)
  const providerOptions = parseContentPatch(input.provider_options_json)
  const record: MetadataVariantRecord = {
    variant_key: input.variant_key,
    provider_key: input.provider_key || null,
    source_variant_key: existing?.source_variant_key ?? null,
    selector_type: input.selector_type,
    original_provider: input.original_provider || null,
    model_id_selector: input.model_id_selector,
    priority: input.priority,
    nick: input.nick || null,
    content: {
      ...(existing?.content ?? {}),
      ...contentPatch,
      variant: input.nick || input.variant_key,
      mount_suffix: input.mount_suffix || input.nick || input.variant_key,
      logical_mounts: input.logical_mounts,
      provider_options: providerOptions,
      capabilities: buildInputCapabilities(input.capabilities, input.capability_values),
    },
    enabled: true,
    created_at: existing?.created_at ?? now,
    updated_at: now,
  }
  const exists = Boolean(existing)
  seed = {
    ...seed,
    metadata_variants: exists
      ? seed.metadata_variants.map((item) => (item.variant_key === record.variant_key ? record : item))
      : [record, ...seed.metadata_variants],
  }
  appendPendingChange({
    change_key: `pending-variant-${record.variant_key}`,
    target_type: 'variant',
    target_key: record.variant_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} metadata variant ${record.variant_key}`,
    risk: 'info',
  })
  if (exists) appendResolverDependencyWarnings('variant', record.variant_key, record.provider_key)
  return withMockLatency(structuredClone(seed))
}

export async function deleteMetadataVariant(variantKey: string) {
  seed = { ...seed, metadata_variants: seed.metadata_variants.filter((rule) => rule.variant_key !== variantKey) }
  appendPendingChange({ change_key: `pending-delete-variant-${variantKey}`, target_type: 'resolver_rule', target_key: variantKey, action: 'delete', summary: `Delete variant ${variantKey}`, risk: 'warning' })
  return withMockLatency(structuredClone(seed))
}

export async function saveMetadataVersionRule(input: MetadataVersionRuleInput) {
  const now = mockNow()
  const existing = seed.metadata_version_rules.find((item) => item.version_rule_key === input.version_rule_key)
  const contentPatch = parseContentPatch(input.content_json)
  const record: MetadataVersionRuleRecord = {
    version_rule_key: input.version_rule_key,
    provider_key: input.provider_key || null,
    source_version_rule_key: existing?.source_version_rule_key ?? null,
    selector_type: input.selector_type,
    original_provider: input.original_provider || null,
    model_id_selector: input.model_id_selector,
    priority: input.priority,
    nick: input.nick || null,
    content: {
      ...(existing?.content ?? {}),
      ...contentPatch,
      family: input.family,
      tier: input.tier,
      model_pattern: input.model_pattern || input.model_id_selector,
      tier_tokens: input.tier_tokens,
      exclude_tier_tokens: input.exclude_tier_tokens,
      version_rank: compactRecord({
        prefix: input.version_rank_prefix,
      }),
      stability: compactRecord({
        unstable_tokens: input.stability_unstable_tokens,
        current_requires_stable: input.stability_current_requires_stable,
      }),
      current_mount: input.current_mount || undefined,
      version_mount: input.version_mount || undefined,
      auto_mounts: input.auto_mounts,
      exclude_snapshot_date_suffix: input.exclude_snapshot_date_suffix,
      capabilities: buildInputCapabilities(input.capabilities, input.capability_values),
      version: input.nick || input.version_rule_key,
    },
    enabled: true,
    created_at: existing?.created_at ?? now,
    updated_at: now,
  }
  const exists = Boolean(existing)
  seed = {
    ...seed,
    metadata_version_rules: exists
      ? seed.metadata_version_rules.map((item) => (item.version_rule_key === record.version_rule_key ? record : item))
      : [record, ...seed.metadata_version_rules],
  }
  appendPendingChange({
    change_key: `pending-version-${record.version_rule_key}`,
    target_type: 'version_rule',
    target_key: record.version_rule_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} metadata version rule ${record.version_rule_key}`,
    risk: 'info',
  })
  if (exists) appendResolverDependencyWarnings('version_rule', record.version_rule_key, record.provider_key)
  return withMockLatency(structuredClone(seed))
}

export async function deleteMetadataVersionRule(versionRuleKey: string) {
  seed = { ...seed, metadata_version_rules: seed.metadata_version_rules.filter((rule) => rule.version_rule_key !== versionRuleKey) }
  appendPendingChange({ change_key: `pending-delete-version-rule-${versionRuleKey}`, target_type: 'resolver_rule', target_key: versionRuleKey, action: 'delete', summary: `Delete version rule ${versionRuleKey}`, risk: 'warning' })
  return withMockLatency(structuredClone(seed))
}

export async function saveLogicalDirectory(input: LogicalDirectoryInput) {
  const now = mockNow()
  const existing = seed.logical_directories.find((item) => item.directory_key === input.directory_key)
  const record = {
    directory_key: input.directory_key,
    path: input.path,
    title: input.title,
    parent_key: input.parent_key || null,
    model_rule_keys: existing?.model_rule_keys ?? [],
    created_at: existing?.created_at ?? now,
    updated_at: now,
  }
  const exists = Boolean(existing)
  seed = {
    ...seed,
    logical_directories: exists
      ? seed.logical_directories.map((item) => (item.directory_key === record.directory_key ? record : item))
      : [record, ...seed.logical_directories],
  }
  appendPendingChange({
    change_key: `pending-directory-${record.directory_key}`,
    target_type: 'logical_directory',
    target_key: record.directory_key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} logical directory ${record.path}`,
    risk: 'warning',
  })
  return withMockLatency(structuredClone(seed))
}

export async function applyLogicalDirectoryMounts(input: LogicalDirectoryMountInput) {
  seed = {
    ...seed,
    logical_directories: seed.logical_directories.map((directory) => {
      if (directory.directory_key !== input.directory_key) {
        return directory
      }
      return {
        ...directory,
        model_rule_keys: Array.from(new Set([...directory.model_rule_keys, ...input.model_rule_keys])),
        updated_at: mockNow(),
      }
    }),
  }
  appendPendingChange({
    change_key: `pending-directory-mount-${input.directory_key}`,
    target_type: 'logical_directory',
    target_key: input.directory_key,
    action: 'update',
    summary: `Update model mounts for logical directory ${input.directory_key}`,
    risk: 'info',
  })
  return withMockLatency(structuredClone(seed))
}

export async function saveDictionaryItem(input: DictionaryItemInput) {
  const exists = seed.dictionaries.some((item) => item.kind === input.kind && item.key === input.key)
  seed = {
    ...seed,
    dictionaries: exists
      ? seed.dictionaries.map((item) => (item.kind === input.kind && item.key === input.key ? { ...item, ...input } : item))
      : [{ ...input, referenced_by: 0 }, ...seed.dictionaries],
  }
  appendPendingChange({
    change_key: `pending-dictionary-${input.kind}-${input.key}`,
    target_type: 'dictionary',
    target_key: input.key,
    action: exists ? 'update' : 'create',
    summary: `${exists ? 'Update' : 'Create'} ${input.kind} dictionary item ${input.key}`,
    risk: 'warning',
  })
  return withMockLatency(structuredClone(seed))
}

export async function deleteDictionaryItem(kind: DictionaryItem['kind'], key: string) {
  seed = { ...seed, dictionaries: seed.dictionaries.filter((item) => item.kind !== kind || item.key !== key) }
  appendPendingChange({ change_key: `pending-delete-dictionary-${kind}-${key}`, target_type: 'dictionary', target_key: key, action: 'delete', summary: `Delete dictionary ${kind}:${key}`, risk: 'warning' })
  return withMockLatency(structuredClone(seed))
}

export async function applyDictionaryToModels(input: DictionaryBulkApplyInput) {
  const dictionary = seed.dictionaries.find((item) => item.kind === input.kind && item.key === input.key)
  if (!dictionary) {
    throw new Error(`Dictionary item does not exist: ${input.key}`)
  }
  seed = {
    ...seed,
    model_param_rules: seed.model_param_rules.map((rule) => {
      if (!input.model_rule_keys.includes(rule.rule_key)) {
        return rule
      }
      if (input.kind === 'api_type') {
        return { ...rule, api_types: Array.from(new Set([...rule.api_types, input.key])), updated_at: mockNow() }
      }
      const capabilityValue: boolean | number = dictionary.value_type === 'number' ? input.number_value ?? 0 : input.boolean_value
      return { ...rule, capabilities: { ...rule.capabilities, [input.key]: capabilityValue }, updated_at: mockNow() }
    }),
  }
  appendPendingChange({
    change_key: `pending-dictionary-apply-${input.kind}-${input.key}`,
    target_type: 'dictionary',
    target_key: input.key,
    action: 'update',
    summary: `Apply ${input.kind} ${input.key} to ${input.model_rule_keys.length} model rules`,
    risk: 'info',
  })
  return withMockLatency(structuredClone(seed))
}

function safeKeyPart(value: string) {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'rule'
}

function parseJsonRecord(text?: string) {
  if (!text?.trim()) {
    return undefined
  }
  try {
    const parsed: unknown = JSON.parse(text)
    return typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed) ? parsed as Record<string, unknown> : undefined
  } catch {
    return undefined
  }
}

function compactRecord(record: Record<string, unknown>) {
  return Object.fromEntries(Object.entries(record).filter(([, value]) => {
    if (value === undefined || value === null || value === '') return false
    if (Array.isArray(value) && value.length === 0) return false
    if (typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length === 0) return false
    return true
  }))
}

export async function saveProviderWizard(input: ProviderWizardInput) {
  const now = mockNow()
  const provider: ProviderRecord = {
    provider_key: input.provider_key,
    provider_driver: input.provider_driver,
    name: input.name,
    base_url: input.base_url,
    provider_kind: input.provider_kind,
    protocol_family: input.protocol_family,
    enabled: true,
    owner_service: 'tech',
    revision: 'draft',
    created_at: now,
    updated_at: now,
  }

  const providerExists = seed.providers.some((item) => item.provider_key === provider.provider_key)
  const selectedModelRules = input.model_rule_drafts.map((draft, index): ModelParamRuleRecord => {
    const source = draft.source_rule_key
      ? seed.model_param_rules.find((rule) => rule.rule_key === draft.source_rule_key)
      : seed.model_param_rules.find((rule) => rule.model_id_selector === draft.model_id_selector && rule.match_type === draft.match_type)
    const selectorKey = draft.match_type === 'default' ? 'default' : draft.model_id_selector
    return {
      rule_key: `${input.provider_key}-${draft.match_type}-${selectorKey.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || index}`,
      provider_key: input.provider_key,
      source_rule_key: source?.rule_key ?? null,
      match_type: draft.match_type,
      original_provider: draft.original_provider || source?.original_provider || null,
      model_id_selector: draft.match_type === 'default' ? null : draft.model_id_selector,
      priority: draft.match_type === 'pattern' ? draft.priority : null,
      model_driver: draft.model_driver,
      api_types: draft.exclude ? source?.api_types ?? [] : draft.api_types,
      logical_mounts: draft.exclude ? source?.logical_mounts ?? [] : draft.logical_mounts,
      capabilities: draft.exclude ? source?.capabilities ?? {} : buildWizardCapabilities(source, draft),
      attributes: {
        ...(source?.attributes ?? {}),
        quality_score: draft.quality_score ?? null,
        latency_class: draft.latency_class ?? null,
        cost_class: draft.cost_class ?? null,
      },
      context_limits: source?.context_limits ?? null,
      pricing: {
        ...(source?.pricing ?? {}),
        estimated_cost_usd: draft.estimated_cost_usd ?? null,
        estimated_latency_ms: draft.estimated_latency_ms ?? null,
      },
      exclude: draft.match_type === 'default' ? false : draft.exclude,
      enabled: true,
      created_at: now,
      updated_at: now,
    }
  })

  const nickRules: ModelNickRecord[] = input.nick_rules.map((rule, index) => ({
    nick_key: `${input.provider_key}-nick-${safeKeyPart(rule.draft_key)}`,
    provider_key: input.provider_key,
    original_provider: rule.original_provider,
    model_id: rule.model_id,
    nick: rule.nick,
    selector_type: rule.selector_type,
    priority: rule.priority || index + 1,
    created_at: now,
    updated_at: now,
  }))

  const originMappingRules: OriginMappingRuleRecord[] = input.origin_mapping_rules.map((rule, index) => ({
    mapping_key: `${input.provider_key}-origin-${safeKeyPart(rule.draft_key)}`,
    provider_key: input.provider_key,
    mapping_mode: rule.mapping_mode,
    match_pattern: rule.match_pattern,
    origin_template: rule.origin_template,
    regex: rule.regex,
    driver_transforms: rule.driver_transforms,
    model_transforms: rule.model_transforms,
    priority: rule.priority || index + 1,
    created_at: now,
    updated_at: now,
  }))

  const originProviderAliases: OriginProviderAliasRecord[] = input.origin_provider_aliases.map((alias) => ({
    alias_key: `${input.provider_key}-origin-alias-${safeKeyPart(alias.draft_key)}`,
    provider_key: input.provider_key,
    alias: alias.alias,
    driver: alias.driver,
    created_at: now,
    updated_at: now,
  }))

  const variantRecords = input.resolver_rule_drafts
    .filter((draft) => draft.rule_kind === 'variant')
    .map((draft, index): MetadataVariantRecord => {
      const selector = draft.model_id_selector || '*'
      return {
        variant_key: `${input.provider_key}-variant-${safeKeyPart(draft.original_provider)}-${safeKeyPart(selector)}-${index + 1}`,
        provider_key: input.provider_key,
        source_variant_key: draft.source_rule_key.startsWith('variant:') ? draft.source_rule_key.slice('variant:'.length) : null,
        selector_type: draft.selector_type,
        original_provider: draft.original_provider,
        model_id_selector: selector,
        priority: draft.priority,
        nick: draft.nick || null,
        content: compactRecord({
          name: draft.nick || undefined,
          mount_suffix: draft.mount_suffix || draft.nick || 'variant',
          provider_options: parseJsonRecord(draft.provider_options_json),
        }),
        enabled: true,
        created_at: now,
        updated_at: now,
      }
    })

  const versionRuleRecords = input.resolver_rule_drafts
    .filter((draft) => draft.rule_kind === 'version_rule')
    .map((draft, index): MetadataVersionRuleRecord => {
      const modelPattern = draft.model_pattern || draft.model_id_selector || '*'
      return {
        version_rule_key: `${input.provider_key}-version-${safeKeyPart(draft.original_provider)}-${safeKeyPart(modelPattern)}-${index + 1}`,
        provider_key: input.provider_key,
        source_version_rule_key: draft.source_rule_key.startsWith('version_rule:') ? draft.source_rule_key.slice('version_rule:'.length) : null,
        selector_type: draft.selector_type,
        original_provider: draft.original_provider,
        model_id_selector: modelPattern,
        priority: draft.priority,
        nick: draft.nick || null,
        content: compactRecord({
          family: draft.family || draft.original_provider,
          tier: draft.tier || draft.nick || 'standard',
          model_pattern: modelPattern,
          tier_tokens: draft.tier_tokens ?? [],
          exclude_tier_tokens: draft.exclude_tier_tokens ?? [],
          version_rank: compactRecord({
            prefix: draft.version_rank_prefix,
          }),
          stability: compactRecord({
            unstable_tokens: draft.stability_unstable_tokens ?? [],
            current_requires_stable: draft.stability_current_requires_stable,
          }),
          current_mount: draft.current_mount || undefined,
          version_mount: draft.version_mount || undefined,
          auto_mounts: draft.logical_mounts,
          exclude_snapshot_date_suffix: draft.exclude_snapshot_date_suffix,
          capabilities: buildResolverCapabilities(draft),
        }),
        enabled: true,
        created_at: now,
        updated_at: now,
      }
    })

  const previousModelKeys = seed.model_param_rules.filter((rule) => rule.provider_key === input.provider_key).map((rule) => rule.rule_key)
  const previousVariantKeys = seed.metadata_variants.filter((rule) => rule.provider_key === input.provider_key).map((rule) => rule.variant_key)
  const previousVersionRuleKeys = seed.metadata_version_rules.filter((rule) => rule.provider_key === input.provider_key).map((rule) => rule.version_rule_key)

  seed = {
    ...seed,
    providers: providerExists
      ? seed.providers.map((item) => (item.provider_key === provider.provider_key ? provider : item))
      : [provider, ...seed.providers],
    model_param_rules: [...selectedModelRules, ...seed.model_param_rules.filter((rule) => rule.provider_key !== input.provider_key)],
    provider_model_rules: seed.provider_model_rules.filter((rule) => rule.provider_key !== input.provider_key),
    model_nicks: [...nickRules, ...seed.model_nicks.filter((rule) => rule.provider_key !== input.provider_key)],
    origin_provider_aliases: [...originProviderAliases, ...seed.origin_provider_aliases.filter((rule) => rule.provider_key !== input.provider_key)],
    origin_mapping_rules: [...originMappingRules, ...seed.origin_mapping_rules.filter((rule) => rule.provider_key !== input.provider_key)],
    metadata_variants: [...variantRecords, ...seed.metadata_variants.filter((rule) => rule.provider_key !== input.provider_key)],
    metadata_version_rules: [...versionRuleRecords, ...seed.metadata_version_rules.filter((rule) => rule.provider_key !== input.provider_key)],
  }

  appendPendingChange({
    change_key: `pending-provider-wizard-${input.provider_key}`,
    target_type: 'provider',
    target_key: input.provider_key,
    action: providerExists ? 'update' : 'create',
    summary: `${providerExists ? 'Update' : 'Create'} aggregate provider ${input.provider_key} with ${selectedModelRules.length} exact model rules`,
    risk: 'warning',
  })
  appendPendingChange({
    change_key: `pending-provider-wizard-nick-${input.provider_key}`,
    target_type: 'nick_rule',
    target_key: input.provider_key,
    action: 'create',
    summary: `Configure nick rewrite rules for ${input.provider_key}`,
    risk: 'warning',
  })
  appendPendingChange({
    change_key: `pending-provider-wizard-origin-${input.provider_key}`,
    target_type: 'origin_mapping_rule',
    target_key: input.provider_key,
    action: 'create',
    summary: `Configure origin identity mappings for ${input.provider_key}`,
    risk: 'warning',
  })
  if (variantRecords.length || versionRuleRecords.length) {
    appendPendingChange({
      change_key: `pending-provider-wizard-resolver-${input.provider_key}`,
      target_type: 'resolver_rule',
      target_key: input.provider_key,
      action: 'create',
      summary: `Configure ${variantRecords.length} variants and ${versionRuleRecords.length} version rules for ${input.provider_key}`,
      risk: 'info',
    })
  }
  if (providerExists) {
    const changedSourceKeys = new Set([...previousModelKeys, ...selectedModelRules.map((rule) => rule.rule_key)])
    const changedVariantKeys = new Set([...previousVariantKeys, ...variantRecords.map((rule) => rule.variant_key)])
    const changedVersionRuleKeys = new Set([...previousVersionRuleKeys, ...versionRuleRecords.map((rule) => rule.version_rule_key)])
    const dependentProviders = new Set(seed.model_param_rules
      .filter((rule) => rule.provider_key && rule.provider_key !== input.provider_key && rule.source_rule_key && changedSourceKeys.has(rule.source_rule_key))
      .map((rule) => rule.provider_key as string))
    seed.metadata_variants
      .filter((rule) => rule.provider_key && rule.provider_key !== input.provider_key && rule.source_variant_key && changedVariantKeys.has(rule.source_variant_key))
      .forEach((rule) => dependentProviders.add(rule.provider_key as string))
    seed.metadata_version_rules
      .filter((rule) => rule.provider_key && rule.provider_key !== input.provider_key && rule.source_version_rule_key && changedVersionRuleKeys.has(rule.source_version_rule_key))
      .forEach((rule) => dependentProviders.add(rule.provider_key as string))
    dependentProviders.forEach((providerKey) => appendPendingChange({
      change_key: `pending-provider-sync-${input.provider_key}-${providerKey}`,
      target_type: 'provider',
      target_key: providerKey,
      action: 'update',
      summary: `${providerKey} reuses rules from ${input.provider_key}; review and synchronize its copied configuration.`,
      risk: 'warning',
    }))
  }

  return withMockLatency(structuredClone(seed))
}

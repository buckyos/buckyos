import { useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, Check, GitCompare, Plus, Trash2, WandSparkles } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { StatusBadge } from '../../components/status/StatusBadge'
import { JsonViewer } from '../../components/json-viewer/JsonViewer'
import {
  getDictionaryKeys,
  getMaterializedLogicalDirectories,
  getOriginalProviders,
} from '../../datamodel/selectors'
import { providerWizardInputSchema, supportedProtocolFamilies, type ProviderWizardInput, type ProviderWizardModelRuleDraft, type ProviderWizardResolverRuleDraft } from '../../datamodel/schemas'
import type { DictionaryItem, DriverMetadataDocument, DriverMetadataRule, LogicalDirectoryRecord, MetadataVariantRecord, MetadataVersionRuleRecord, ModelParamRuleRecord, ProviderCloudSeed, ProviderRecord } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { useShellContext } from '../../pages/pageUtils'

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm text-[color:var(--cp-text)]'
const steps = ['basic', 'models', 'params', 'patternOrder', 'resolver', 'resolverParams', 'mounts', 'nick', 'preview'] as const
type RuleArrayField = 'api_types' | 'capabilities' | 'logical_mounts'
type ResolverArrayField = 'capabilities'
type ResolverTokenField = 'tier_tokens' | 'exclude_tier_tokens' | 'stability_unstable_tokens'
type ModelParamTab = 'exact' | 'pattern' | 'default'
type ResolverRuleTab = 'variant' | 'version_rule'
type MountTargetTab = ModelParamTab | 'version_rule'
type NickRewritePreviewTarget = 'model' | 'pattern' | 'default' | 'variant' | 'version_rule'
type NickConfigTab = 'nick' | 'origin_mappings' | 'origin_provider_aliases'

const readStringArray = (value: unknown) => Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []

const readRecord = (value: unknown): Record<string, unknown> => {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : {}
}

const contentString = (content: Record<string, unknown>, key: string) => typeof content[key] === 'string' ? content[key] as string : undefined

const providerOptionsJson = (content: Record<string, unknown>) => {
  const options = readRecord(content.provider_options)
  return Object.keys(options).length ? JSON.stringify(options, null, 2) : ''
}

const parseTokenText = (value: string) => value.split(/[,\s]+/).map((token) => token.trim()).filter(Boolean)
const providerDriverFromName = (name: string) => name.trim().toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '')
const normalizeProtocolFamily = (value?: string | null): ProviderWizardInput['protocol_family'] => {
  return supportedProtocolFamilies.includes(value as ProviderWizardInput['protocol_family'])
    ? value as ProviderWizardInput['protocol_family']
    : 'openai-compatible'
}
const mountToDirectoryPath = (mount?: string) => {
  const normalized = mount?.trim().replace(/\./g, '/').replace(/^\/+|\/+$/g, '')
  return normalized ? `/${normalized}` : ''
}
const directoryPathToMount = (path: string) => path.trim().replace(/^\/+|\/+$/g, '').replace(/\//g, '.')
const mountTabLabel = (tab: MountTargetTab) => {
  if (tab === 'exact') return 'models'
  if (tab === 'pattern') return 'patterns'
  if (tab === 'default') return 'defaults'
  return 'Version rules'
}

const uniqueStrings = (values: Array<string | null | undefined>) => Array.from(new Set(values.filter((value): value is string => Boolean(value))))

function providerSelectionMatches(rule: ModelParamRuleRecord, selector: { rule_type: string; selector: string }) {
  if (!rule.enabled || !rule.model_id_selector) {
    return false
  }
  if (selector.rule_type === 'include_origin') {
    return rule.original_provider === selector.selector
  }
  if (selector.rule_type === 'include_pattern') {
    return wildcardMatch(selector.selector, rule.model_id_selector)
  }
  return false
}

function getEditingSourceModelRules(data: ProviderCloudSeed, provider: ProviderRecord) {
  const ownRules = data.model_param_rules.filter((rule) => rule.provider_key === provider.provider_key)
  if (ownRules.length) {
    return ownRules
  }
  const selections = data.provider_model_rules.filter((rule) => rule.enabled && rule.provider_key === provider.provider_key)
  const selectedRules = data.model_param_rules.filter((rule) => selections.some((selection) => providerSelectionMatches(rule, selection)))
  if (selectedRules.length) {
    return selectedRules
  }
  return data.model_param_rules.filter((rule) => rule.provider_key === null && rule.original_provider === provider.provider_key)
}

function modelRuleToWizardDraft(provider: ProviderRecord, rule: ModelParamRuleRecord): ProviderWizardModelRuleDraft {
  const copiedFromSource = rule.provider_key !== provider.provider_key
  const sourceRuleKey = copiedFromSource ? rule.rule_key : rule.source_rule_key ?? ''
  const capabilityValues = readCapabilityValues(rule.capabilities)
  return {
    draft_key: sourceRuleKey ? `source-${sourceRuleKey}` : `manual-${rule.rule_key}`,
    match_type: rule.match_type,
    original_provider: rule.original_provider ?? provider.provider_key,
    model_id_selector: rule.model_id_selector ?? '',
    priority: rule.priority ?? 999,
    source_rule_key: sourceRuleKey,
    model_driver: rule.model_driver ?? provider.provider_driver,
    api_types: rule.api_types,
    capabilities: Object.keys(rule.capabilities),
    capability_values: capabilityValues,
    estimated_cost_usd: getSourceEstimatedCost(rule),
    estimated_latency_ms: getSourceEstimatedLatency(rule),
    quality_score: getNumericAttribute(rule, 'quality_score'),
    latency_class: getClassAttribute(rule, 'latency_class', ['fast', 'normal', 'slow']),
    cost_class: getClassAttribute(rule, 'cost_class', ['low', 'medium', 'high']),
    logical_mounts: rule.logical_mounts,
    exclude: rule.exclude,
  }
}

function getEditingResolverSources(data: ProviderCloudSeed, provider: ProviderRecord) {
  const ownVariants = data.metadata_variants.filter((rule) => rule.provider_key === provider.provider_key)
  const ownVersionRules = data.metadata_version_rules.filter((rule) => rule.provider_key === provider.provider_key)
  if (ownVariants.length || ownVersionRules.length) {
    return { variants: ownVariants, versionRules: ownVersionRules }
  }
  const origins = uniqueStrings(getEditingSourceModelRules(data, provider).map((rule) => rule.original_provider))
  return {
    variants: data.metadata_variants.filter((rule) => rule.provider_key === null && rule.enabled && origins.includes(rule.original_provider ?? '')),
    versionRules: data.metadata_version_rules.filter((rule) => rule.provider_key === null && rule.enabled && origins.includes(rule.original_provider ?? '')),
  }
}

function variantToWizardDraft(provider: ProviderRecord, rule: MetadataVariantRecord): ProviderWizardResolverRuleDraft {
  const copiedFromSource = rule.provider_key !== provider.provider_key
  const sourceRuleKey = copiedFromSource ? `variant:${rule.variant_key}` : rule.source_variant_key ? `variant:${rule.source_variant_key}` : ''
  const content = readRecord(rule.content)
  const capabilities = readRecord(content.capabilities)
  return {
    draft_key: sourceRuleKey ? `source-${sourceRuleKey}` : `manual-variant-${rule.variant_key}`,
    rule_kind: 'variant',
    selector_type: rule.selector_type,
    original_provider: rule.original_provider ?? provider.provider_key,
    model_id_selector: rule.model_id_selector || '*',
    priority: rule.priority,
    nick: rule.nick ?? '',
    mount_suffix: contentString(content, 'mount_suffix') ?? rule.nick ?? '',
    provider_options_json: providerOptionsJson(content),
    capabilities: Object.keys(capabilities),
    capability_values: readCapabilityValues(capabilities),
    source_rule_key: sourceRuleKey,
    logical_mounts: readStringArray(content.logical_mounts),
  }
}

function versionRuleToWizardDraft(provider: ProviderRecord, rule: MetadataVersionRuleRecord): ProviderWizardResolverRuleDraft {
  const copiedFromSource = rule.provider_key !== provider.provider_key
  const sourceRuleKey = copiedFromSource ? `version_rule:${rule.version_rule_key}` : rule.source_version_rule_key ? `version_rule:${rule.source_version_rule_key}` : ''
  const content = readRecord(rule.content)
  const stability = readRecord(content.stability)
  const versionRank = readRecord(content.version_rank)
  const capabilities = readRecord(content.capabilities)
  return {
    draft_key: sourceRuleKey ? `source-${sourceRuleKey}` : `manual-version-${rule.version_rule_key}`,
    rule_kind: 'version_rule',
    selector_type: rule.selector_type,
    original_provider: rule.original_provider ?? provider.provider_key,
    model_id_selector: rule.model_id_selector || '*',
    priority: rule.priority,
    nick: rule.nick ?? '',
    family: contentString(content, 'family') ?? rule.original_provider ?? provider.provider_key,
    tier: contentString(content, 'tier') ?? rule.nick ?? '',
    model_pattern: contentString(content, 'model_pattern') ?? rule.model_id_selector,
    tier_tokens: readStringArray(content.tier_tokens),
    exclude_tier_tokens: readStringArray(content.exclude_tier_tokens),
    version_rank_prefix: typeof versionRank.prefix === 'string' ? versionRank.prefix : '',
    stability_unstable_tokens: readStringArray(stability.unstable_tokens),
    stability_current_requires_stable: typeof stability.current_requires_stable === 'boolean' ? stability.current_requires_stable : false,
    current_mount: contentString(content, 'current_mount') ?? '',
    version_mount: contentString(content, 'version_mount') ?? '',
    exclude_snapshot_date_suffix: content.exclude_snapshot_date_suffix === true,
    capabilities: Object.keys(capabilities),
    capability_values: readCapabilityValues(capabilities),
    source_rule_key: sourceRuleKey,
    logical_mounts: readStringArray(content.auto_mounts),
  }
}

export function ProviderWizardPage() {
  const { t } = useI18n()
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const { setInspector } = useShellContext()
  const { workspace, runProviderWizard, runPublishPreview } = useProviderMetadataStore()
  const data = workspace.data!
  const editingProvider = data.providers.find((provider) => provider.provider_key === searchParams.get('provider'))
  const [stepIndex, setStepIndex] = useState(0)
  const [selectedResolverDraftIndex, setSelectedResolverDraftIndex] = useState(0)
  const [modelTab, setModelTab] = useState<ModelParamTab>('exact')
  const [resolverTab, setResolverTab] = useState<ResolverRuleTab>('variant')
  const [paramsOrigin, setParamsOrigin] = useState('')
  const [paramsTab, setParamsTab] = useState<ModelParamTab>('exact')
  const [resolverParamsOrigin, setResolverParamsOrigin] = useState('')
  const [resolverParamsTab, setResolverParamsTab] = useState<ResolverRuleTab>('variant')
  const [bulkDraftKeys, setBulkDraftKeys] = useState<string[]>([])
  const [bulkResolverDraftKeys, setBulkResolverDraftKeys] = useState<string[]>([])
  const [paramSessionSnapshot, setParamSessionSnapshot] = useState<ProviderWizardModelRuleDraft[] | null>(null)
  const [resolverParamSessionSnapshot, setResolverParamSessionSnapshot] = useState<ProviderWizardResolverRuleDraft[] | null>(null)
  const [mountOrigin, setMountOrigin] = useState('')
  const [mountTab, setMountTab] = useState<MountTargetTab>('exact')
  const [mountTargetKeys, setMountTargetKeys] = useState<string[]>([])
  const [bulkMountPathOverrides, setBulkMountPathOverrides] = useState<Record<string, 'full' | 'none'>>({})
  const [nickPreviewTarget, setNickPreviewTarget] = useState<NickRewritePreviewTarget>('model')
  const [nickConfigTab, setNickConfigTab] = useState<NickConfigTab>('nick')
  const [submitState, setSubmitState] = useState<'idle' | 'saving' | 'invalid' | 'error'>('idle')
  const [submitMessage, setSubmitMessage] = useState('')
  const originalProviders = useMemo(() => getOriginalProviders(data), [data])
  const [focusedOrigin, setFocusedOrigin] = useState(originalProviders[0] ?? '')
  const apiTypes = useMemo(() => getDictionaryKeys(data, 'api_type'), [data])
  const capabilityItems = useMemo(() => data.dictionaries.filter((item) => item.kind === 'capability'), [data])
  const groupedCapabilityItems = useMemo(() => groupCapabilityItems(capabilityItems), [capabilityItems])
  const capabilities = useMemo(() => capabilityItems.map((item) => item.key), [capabilityItems])
  const protocolOptions = supportedProtocolFamilies
  const logicalDirectories = useMemo(() => getMaterializedLogicalDirectories(data), [data])
  const logicalMounts = useMemo(() => logicalDirectories.map((directory) => directory.path), [logicalDirectories])
  const defaultModels = useMemo(() => {
    return Array.from(new Set(data.model_param_rules
      .filter((rule) => rule.original_provider === 'openai' || rule.original_provider === 'claude')
      .map((rule) => rule.rule_key))).slice(0, 4)
  }, [data])
  const defaultOrigins = useMemo(() => originalProviders.filter((provider) => provider === 'openai' || provider === 'claude').slice(0, 2), [originalProviders])
  const defaultCapabilities = useMemo(() => capabilities.filter((capability) => ['streaming', 'tool_call'].includes(capability)).slice(0, 2), [capabilities])
  const existingModelDrafts = useMemo(() => !editingProvider ? [] : getEditingSourceModelRules(data, editingProvider).map((rule) => modelRuleToWizardDraft(editingProvider, rule)), [data, editingProvider])
  const existingResolverDrafts = useMemo(() => {
    if (!editingProvider) {
      return []
    }
    const sources = getEditingResolverSources(data, editingProvider)
    return [
      ...sources.variants.map((rule) => variantToWizardDraft(editingProvider, rule)),
      ...sources.versionRules.map((rule) => versionRuleToWizardDraft(editingProvider, rule)),
    ]
  }, [data, editingProvider])
  const existingNickDrafts = useMemo(() => !editingProvider ? [] : data.model_nicks.filter((rule) => rule.provider_key === editingProvider.provider_key).map((rule) => ({ draft_key: rule.nick_key, original_provider: rule.original_provider ?? editingProvider.provider_key, selector_type: rule.selector_type, model_id: rule.model_id, nick: rule.nick, priority: rule.priority })), [data.model_nicks, editingProvider])
  const existingOriginMappingDrafts = useMemo(() => !editingProvider ? [] : data.origin_mapping_rules.filter((rule) => rule.provider_key === editingProvider.provider_key).map((rule) => ({ draft_key: rule.mapping_key, mapping_mode: rule.mapping_mode, match_pattern: rule.match_pattern, origin_template: rule.origin_template, regex: rule.regex, driver_transforms: rule.driver_transforms, model_transforms: rule.model_transforms, priority: rule.priority })), [data.origin_mapping_rules, editingProvider])
  const existingOriginAliasDrafts = useMemo(() => !editingProvider ? [] : data.origin_provider_aliases.filter((alias) => alias.provider_key === editingProvider.provider_key).map((alias) => ({ draft_key: alias.alias_key, alias: alias.alias, driver: alias.driver })), [data.origin_provider_aliases, editingProvider])
  const initialSelectedOrigins = useMemo(() => {
    const currentOrigins = uniqueStrings(existingModelDrafts.map((draft) => draft.original_provider))
    return editingProvider
      ? currentOrigins.length ? currentOrigins : [editingProvider.provider_key]
      : defaultOrigins.length ? defaultOrigins : originalProviders.slice(0, 1)
  }, [defaultOrigins, editingProvider, existingModelDrafts, originalProviders])
  const editingApiTypes = useMemo(() => uniqueStrings(existingModelDrafts.flatMap((draft) => draft.api_types)), [existingModelDrafts])
  const editingCapabilities = useMemo(() => uniqueStrings(existingModelDrafts.flatMap((draft) => draft.capabilities)), [existingModelDrafts])
  const editingLogicalMounts = useMemo(() => uniqueStrings(existingModelDrafts.flatMap((draft) => draft.logical_mounts)), [existingModelDrafts])
  const defaultProviderName = useMemo(() => `Provider_${String(data.providers.length + 1).padStart(4, '0')}`, [data.providers.length])
  const initialValues = useMemo<ProviderWizardInput>(() => ({
    provider_key: editingProvider?.provider_key ?? `provider-${String(data.providers.length + 1).padStart(4, '0')}`,
    name: editingProvider?.name ?? defaultProviderName,
    provider_driver: editingProvider?.provider_driver ?? providerDriverFromName(defaultProviderName),
    base_url: editingProvider ? editingProvider.base_url ?? '' : 'https://openrouter.ai/api/v1',
    provider_kind: editingProvider?.provider_kind ?? 'aggregator',
    protocol_family: normalizeProtocolFamily(editingProvider?.protocol_family),
    template_provider_key: '',
    selected_origins: initialSelectedOrigins,
    selected_model_ids: editingProvider ? existingModelDrafts.map((draft) => draft.source_rule_key).filter(Boolean) : defaultModels,
    selected_resolver_rule_keys: editingProvider ? existingResolverDrafts.map((draft) => draft.source_rule_key).filter(Boolean) : [
      ...data.metadata_variants.slice(0, 1).map((rule) => `variant:${rule.variant_key}`),
      ...data.metadata_version_rules.slice(0, 1).map((rule) => `version_rule:${rule.version_rule_key}`),
    ],
    nick_rules: editingProvider
      ? existingNickDrafts
      : [{ draft_key: 'nick-openai-prefix', original_provider: 'openai', selector_type: 'pattern', model_id: '*', nick: 'openai/{model}', priority: 1 }],
    origin_mapping_rules: editingProvider
      ? existingOriginMappingDrafts
      : [{ draft_key: 'origin-path-capture', mapping_mode: 'regex', match_pattern: '*/*', origin_template: '<driver>/<model>', regex: '^(?<driver>[^/]+)/(?<model>.+)$', driver_transforms: ['alias'], model_transforms: ['trim'], priority: 1 }],
    origin_provider_aliases: editingProvider
      ? existingOriginAliasDrafts
      : [{ draft_key: 'alias-openai', alias: 'openai', driver: 'openai' }],
    selected_api_types: editingProvider ? editingApiTypes : apiTypes.includes('llm') ? ['llm'] : apiTypes.slice(0, 1),
    selected_capabilities: editingProvider ? editingCapabilities : defaultCapabilities.length ? defaultCapabilities : capabilities.slice(0, 1),
    selected_logical_mounts: editingProvider ? editingLogicalMounts : logicalMounts.includes('/llm') ? ['/llm'] : logicalMounts.slice(0, 1),
    model_rule_drafts: existingModelDrafts,
    resolver_rule_drafts: existingResolverDrafts,
  }), [
    apiTypes,
    capabilities,
    data.metadata_variants,
    data.metadata_version_rules,
    data.providers.length,
    defaultCapabilities,
    defaultModels,
    defaultProviderName,
    editingApiTypes,
    editingCapabilities,
    editingLogicalMounts,
    editingProvider,
    existingNickDrafts,
    existingOriginMappingDrafts,
    existingOriginAliasDrafts,
    existingResolverDrafts,
    initialSelectedOrigins,
    logicalMounts,
  ])

  const form = useForm<ProviderWizardInput>({
    resolver: zodResolver(providerWizardInputSchema),
    defaultValues: initialValues,
  })

  useEffect(() => {
    form.reset(initialValues)
    setStepIndex(0)
    setSelectedResolverDraftIndex(0)
    setBulkDraftKeys([])
    setBulkResolverDraftKeys([])
    setMountTargetKeys([])
  }, [form, initialValues])

  const values = form.watch()
  const currentStep = steps[stepIndex]
  const nameConflict = useMemo(() => {
    const normalizedName = values.name.trim().toLowerCase()
    if (!normalizedName) {
      return false
    }
    return data.providers.some((provider) => provider.provider_key !== editingProvider?.provider_key && provider.name.trim().toLowerCase() === normalizedName)
  }, [data.providers, editingProvider?.provider_key, values.name])

  useEffect(() => {
    const nextDriver = providerDriverFromName(values.name)
    if (nextDriver && nextDriver !== form.getValues('provider_driver')) {
      form.setValue('provider_driver', nextDriver, { shouldDirty: true, shouldValidate: true })
    }
  }, [form, values.name])

  const originStats = useMemo(() => {
    return originalProviders.map((provider) => {
      const rules = data.model_param_rules.filter((rule) => rule.enabled && rule.original_provider === provider)
      const selectedCount = rules.filter((rule) => values.selected_model_ids.includes(rule.rule_key)).length
      return { provider, totalCount: rules.length, selectedCount }
    })
  }, [data.model_param_rules, originalProviders, values.selected_model_ids])
  const sourceModels = useMemo(() => {
    const seen = new Set<string>()
    return data.model_param_rules.filter((rule) => {
      const uniqueKey = rule.rule_key
      if (!rule.enabled || seen.has(uniqueKey)) {
        return false
      }
      seen.add(uniqueKey)
      return true
    })
  }, [data])
  const focusedSourceModels = useMemo(() => {
    return sourceModels.filter((rule) => rule.original_provider === focusedOrigin)
  }, [focusedOrigin, sourceModels])
  const availableModelTabs = useMemo(() => (['exact', 'pattern', 'default'] as const).filter((tab) => focusedSourceModels.some((rule) => rule.match_type === tab)), [focusedSourceModels])
  const focusedSourceModelsByTab = useMemo(() => focusedSourceModels.filter((rule) => rule.match_type === modelTab), [focusedSourceModels, modelTab])
  const selectedSourceModels = useMemo(() => {
    return sourceModels.filter((rule) => values.selected_model_ids.includes(rule.rule_key))
  }, [sourceModels, values.selected_model_ids])
  const resolverSources = useMemo(() => [
    ...data.metadata_variants.filter((rule) => rule.enabled).map((rule) => {
      const content = readRecord(rule.content)
      const sourceCapabilities = readRecord(content.capabilities)
      return {
        key: `variant:${rule.variant_key}`,
        rule_kind: 'variant' as const,
        original_provider: rule.original_provider ?? '',
        model_id_selector: rule.model_id_selector || '*',
        selector_type: rule.selector_type,
        priority: rule.priority,
        nick: rule.nick ?? '',
        mount_suffix: contentString(content, 'mount_suffix') ?? rule.nick ?? '',
        provider_options_json: providerOptionsJson(content),
        capabilities: Object.keys(sourceCapabilities),
        capability_values: readCapabilityValues(sourceCapabilities),
        logical_mounts: readStringArray(content.logical_mounts),
      }
    }),
    ...data.metadata_version_rules.filter((rule) => rule.enabled).map((rule) => {
      const content = readRecord(rule.content)
      const stability = readRecord(content.stability)
      const versionRank = readRecord(content.version_rank)
      const sourceCapabilities = readRecord(content.capabilities)
      return {
        key: `version_rule:${rule.version_rule_key}`,
        rule_kind: 'version_rule' as const,
        original_provider: rule.original_provider ?? '',
        model_id_selector: rule.model_id_selector || '*',
        selector_type: rule.selector_type,
        priority: rule.priority,
        nick: rule.nick ?? '',
        family: contentString(content, 'family') ?? rule.original_provider ?? '',
        tier: contentString(content, 'tier') ?? rule.nick ?? '',
        model_pattern: contentString(content, 'model_pattern') ?? rule.model_id_selector,
        tier_tokens: readStringArray(content.tier_tokens),
        exclude_tier_tokens: readStringArray(content.exclude_tier_tokens),
        version_rank_prefix: typeof versionRank.prefix === 'string' ? versionRank.prefix : '',
        stability_unstable_tokens: readStringArray(stability.unstable_tokens),
        stability_current_requires_stable: typeof stability.current_requires_stable === 'boolean' ? stability.current_requires_stable : false,
        current_mount: contentString(content, 'current_mount') ?? '',
        version_mount: contentString(content, 'version_mount') ?? '',
        exclude_snapshot_date_suffix: content.exclude_snapshot_date_suffix === true,
        capabilities: Object.keys(sourceCapabilities),
        capability_values: readCapabilityValues(sourceCapabilities),
        logical_mounts: readStringArray(content.auto_mounts),
      }
    }),
  ], [data.metadata_variants, data.metadata_version_rules])
  const resolverOriginStats = useMemo(() => originalProviders.map((provider) => {
    const rules = resolverSources.filter((rule) => rule.original_provider === provider)
    const selectedCount = rules.filter((rule) => values.selected_resolver_rule_keys.includes(rule.key)).length
    return { provider, selectedCount, totalCount: rules.length }
  }).filter((stat) => stat.totalCount > 0), [originalProviders, resolverSources, values.selected_resolver_rule_keys])
  const availableResolverTabs = useMemo(() => (['variant', 'version_rule'] as const).filter((tab) => resolverSources.some((rule) => rule.original_provider === focusedOrigin && rule.rule_kind === tab)), [focusedOrigin, resolverSources])
  const focusedResolverSources = useMemo(() => resolverSources.filter((rule) => rule.original_provider === focusedOrigin && rule.rule_kind === resolverTab), [focusedOrigin, resolverSources, resolverTab])
  const wizardDiagnostics = useMemo(() => buildWizardDiagnostics(data.model_param_rules, values), [data.model_param_rules, values])
  const publishedPreview = useMemo(() => {
    return values.model_rule_drafts.map((draft) => {
      const sourceModelId = draft.model_id_selector || 'defaults'
      return {
        source_model_id: sourceModelId,
        match_type: draft.match_type,
        original_provider: draft.original_provider || null,
        published_id: buildPublishedId(values, draft.match_type === 'default' ? '*' : sourceModelId, draft.original_provider),
        model_driver: draft.model_driver,
        api_types: draft.api_types,
        capabilities: draft.capabilities,
        logical_mounts: draft.logical_mounts,
        exclude: draft.exclude,
      }
    })
  }, [values])
  const clientDriverMetadataPreview = useMemo(() => buildWizardClientDriverMetadata(data.model_param_rules, values), [data.model_param_rules, values])
  const originMappingsJsonPreview = useMemo(() => ({
    origin_mappings: clientDriverMetadataPreview.origin_mappings ?? [],
  }), [clientDriverMetadataPreview.origin_mappings])
  const resolverPublishedPreview = useMemo(() => {
    return values.resolver_rule_drafts.map((draft) => {
      const sourceSelector = getResolverRewriteSelector(draft)
      return {
        source_model_id: sourceSelector,
        target: draft.rule_kind as NickRewritePreviewTarget,
        original_provider: draft.original_provider,
        published_id: buildPublishedId(values, sourceSelector, draft.original_provider),
        logical_mounts: draft.logical_mounts,
      }
    })
  }, [values])
  const nickRewritePreviewSections = useMemo(() => {
    const items = [
      ...publishedPreview.map((item) => ({
        ...item,
        target: previewTargetFromMatchType(item.match_type),
      })),
      ...resolverPublishedPreview,
    ]
    const groups: Array<{ target: NickRewritePreviewTarget; label: string }> = [
      { target: 'model', label: t('wizard.preview.models', 'Models') },
      { target: 'pattern', label: t('wizard.preview.patterns', 'Patterns') },
      { target: 'default', label: t('wizard.preview.defaults', 'Defaults') },
      { target: 'variant', label: t('resolver.variants', 'Variants') },
      { target: 'version_rule', label: t('wizard.preview.versionRules', 'Version rules') },
    ]
    return groups
      .map((group) => ({
        ...group,
        items: items.filter((item) => item.target === group.target),
      }))
      .filter((group) => group.items.length > 0)
  }, [publishedPreview, resolverPublishedPreview, t])
  const activeNickRewritePreviewSection = nickRewritePreviewSections.find((section) => section.target === nickPreviewTarget) ?? nickRewritePreviewSections[0] ?? null
  const paramOrigins = useMemo(() => Array.from(new Set(values.model_rule_drafts.map((draft) => draft.source_rule_key ? draft.original_provider : values.provider_key))).filter(Boolean), [values.model_rule_drafts, values.provider_key])
  const activeParamsOrigin = paramsOrigin || paramOrigins[0] || ''
  const availableParamsTabs = useMemo(() => (['exact', 'pattern', 'default'] as const).filter((tab) => values.model_rule_drafts.some((draft) => (draft.source_rule_key ? draft.original_provider : values.provider_key) === activeParamsOrigin && draft.match_type === tab)), [activeParamsOrigin, values.model_rule_drafts, values.provider_key])
  const paramsDrafts = useMemo(() => values.model_rule_drafts.filter((draft) => (draft.source_rule_key ? draft.original_provider : values.provider_key) === activeParamsOrigin && draft.match_type === paramsTab), [activeParamsOrigin, paramsTab, values.model_rule_drafts, values.provider_key])
  const selectedParamDrafts = useMemo(() => {
    const selectedKeys = new Set(bulkDraftKeys)
    return values.model_rule_drafts.filter((draft) => selectedKeys.has(draft.draft_key))
  }, [bulkDraftKeys, values.model_rule_drafts])
  const selectedParamDraft = selectedParamDrafts[0] ?? null
  const selectedParamOrigins = useMemo(() => Array.from(new Set(selectedParamDrafts.map((draft) => draft.original_provider || values.provider_key))).filter(Boolean), [selectedParamDrafts, values.provider_key])
  const selectedParamProviderOwned = selectedParamDrafts.length > 0 && selectedParamDrafts.every((draft) => (draft.original_provider || values.provider_key) === values.provider_key)
  const selectedParamMissingSelectors = useMemo(() => selectedParamDrafts.filter((draft) => draft.match_type !== 'default' && !draft.model_id_selector.trim()), [selectedParamDrafts])
  const mountTargets = useMemo(() => [
    ...values.model_rule_drafts.map((draft) => ({ key: `model:${draft.draft_key}`, kind: draft.match_type, origin: draft.source_rule_key ? draft.original_provider : values.provider_key, label: draft.model_id_selector || 'defaults' })),
    ...values.resolver_rule_drafts
      .filter((draft) => draft.rule_kind === 'version_rule')
      .map((draft) => ({ key: `resolver:${draft.draft_key}`, kind: draft.rule_kind, origin: draft.source_rule_key ? draft.original_provider : values.provider_key, label: draft.model_id_selector || '*' })),
  ], [values.model_rule_drafts, values.provider_key, values.resolver_rule_drafts])
  const mountOrigins = useMemo(() => Array.from(new Set(mountTargets.map((target) => target.origin))).filter(Boolean), [mountTargets])
  const activeMountOrigin = mountOrigins.includes(mountOrigin) ? mountOrigin : mountOrigins[0] || ''
  const availableMountTabs = useMemo(() => (['exact', 'pattern', 'default', 'version_rule'] as const).filter((kind) => mountTargets.some((target) => target.origin === activeMountOrigin && target.kind === kind)), [activeMountOrigin, mountTargets])
  const visibleMountTargets = useMemo(() => mountTargets.filter((target) => target.origin === activeMountOrigin && target.kind === mountTab), [activeMountOrigin, mountTab, mountTargets])
  const selectedMountTargets = useMemo(() => {
    const selectedKeys = new Set(mountTargetKeys)
    return mountTargets.filter((target) => selectedKeys.has(target.key))
  }, [mountTargetKeys, mountTargets])
  const resolverParamOrigins = useMemo(() => Array.from(new Set(values.resolver_rule_drafts.map((draft) => draft.source_rule_key ? draft.original_provider : values.provider_key))).filter(Boolean), [values.provider_key, values.resolver_rule_drafts])
  const resolverParamOriginsForTab = useMemo(() => resolverParamOrigins.filter((origin) => values.resolver_rule_drafts.some((draft) => (draft.source_rule_key ? draft.original_provider : values.provider_key) === origin && draft.rule_kind === resolverParamsTab)), [resolverParamOrigins, resolverParamsTab, values.provider_key, values.resolver_rule_drafts])
  const activeResolverParamsOrigin = resolverParamOriginsForTab.includes(resolverParamsOrigin) ? resolverParamsOrigin : resolverParamOriginsForTab[0] || ''
  const availableResolverParamsTabs = useMemo(() => (['variant', 'version_rule'] as const).filter((tab) => values.resolver_rule_drafts.some((draft) => (draft.source_rule_key ? draft.original_provider : values.provider_key) === activeResolverParamsOrigin && draft.rule_kind === tab)), [activeResolverParamsOrigin, values.provider_key, values.resolver_rule_drafts])
  const resolverParamsDrafts = useMemo(() => values.resolver_rule_drafts.filter((draft) => (draft.source_rule_key ? draft.original_provider : values.provider_key) === activeResolverParamsOrigin && draft.rule_kind === resolverParamsTab), [activeResolverParamsOrigin, resolverParamsTab, values.provider_key, values.resolver_rule_drafts])
  const selectedResolverParamDrafts = useMemo(() => {
    const selectedKeys = new Set(bulkResolverDraftKeys)
    return values.resolver_rule_drafts.filter((draft) => selectedKeys.has(draft.draft_key) && draft.rule_kind === resolverParamsTab)
  }, [bulkResolverDraftKeys, resolverParamsTab, values.resolver_rule_drafts])
  const selectedResolverParamDraft = selectedResolverParamDrafts[0] ?? null
  const selectedResolverParamOrigins = useMemo(() => Array.from(new Set(selectedResolverParamDrafts.map((draft) => draft.original_provider || values.provider_key))).filter(Boolean), [selectedResolverParamDrafts, values.provider_key])
  const selectedResolverParamProviderOwned = selectedResolverParamDrafts.length > 0 && selectedResolverParamDrafts.every((draft) => (draft.original_provider || values.provider_key) === values.provider_key)

  useEffect(() => {
    if (availableModelTabs.length && !availableModelTabs.includes(modelTab)) {
      setModelTab(availableModelTabs[0])
    }
  }, [availableModelTabs, modelTab])

  useEffect(() => {
    if (availableResolverTabs.length && !availableResolverTabs.includes(resolverTab)) {
      setResolverTab(availableResolverTabs[0])
    }
  }, [availableResolverTabs, resolverTab])

  useEffect(() => {
    if (availableParamsTabs.length && !availableParamsTabs.includes(paramsTab)) {
      setParamsTab(availableParamsTabs[0])
    }
  }, [availableParamsTabs, paramsTab])

  useEffect(() => {
    if (availableResolverParamsTabs.length && !availableResolverParamsTabs.includes(resolverParamsTab)) {
      setResolverParamsTab(availableResolverParamsTabs[0])
    }
  }, [availableResolverParamsTabs, resolverParamsTab])

  useEffect(() => {
    if (resolverParamsDrafts.length && !resolverParamsDrafts.some((draft) => values.resolver_rule_drafts[selectedResolverDraftIndex]?.draft_key === draft.draft_key)) {
      const firstIndex = values.resolver_rule_drafts.findIndex((draft) => draft.draft_key === resolverParamsDrafts[0].draft_key)
      if (firstIndex >= 0) {
        setSelectedResolverDraftIndex(firstIndex)
      }
    }
  }, [resolverParamsDrafts, selectedResolverDraftIndex, values.resolver_rule_drafts])

  useEffect(() => {
    if (availableMountTabs.length && !availableMountTabs.includes(mountTab)) {
      setMountTab(availableMountTabs[0])
    }
  }, [availableMountTabs, mountTab])

  useEffect(() => {
    const existingKeys = new Set(mountTargets.map((target) => target.key))
    setMountTargetKeys((keys) => keys.filter((key) => existingKeys.has(key)))
  }, [mountTargets])

  useEffect(() => {
    const nextDrafts = buildWizardRuleDrafts(data.model_param_rules, form.getValues(), selectedSourceModels, capabilityItems)
    const currentKeys = form.getValues('model_rule_drafts').map((draft) => draft.draft_key).join('|')
    const nextKeys = nextDrafts.map((draft) => draft.draft_key).join('|')
    if (currentKeys !== nextKeys) {
      form.setValue('model_rule_drafts', nextDrafts, { shouldDirty: true, shouldValidate: true })
      const nextDraftKeys = new Set(nextDrafts.map((draft) => draft.draft_key))
      setBulkDraftKeys((keys) => keys.filter((key) => nextDraftKeys.has(key)))
    }
    const nextResolverDrafts = buildWizardResolverDrafts(form.getValues(), resolverSources)
    const currentResolverKeys = form.getValues('resolver_rule_drafts').map((draft) => draft.draft_key).join('|')
    const nextResolverKeys = nextResolverDrafts.map((draft) => draft.draft_key).join('|')
    if (currentResolverKeys !== nextResolverKeys) {
      form.setValue('resolver_rule_drafts', nextResolverDrafts, { shouldDirty: true, shouldValidate: true })
      setSelectedResolverDraftIndex((index) => Math.min(index, Math.max(0, nextResolverDrafts.length - 1)))
    }
  }, [capabilityItems, data.model_param_rules, form, resolverSources, selectedSourceModels, values.selected_model_ids, values.selected_resolver_rule_keys])

  const setSourceModelSelection = (ruleKeys: string[]) => {
    const current = form.getValues('selected_model_ids')
    const next = Array.from(new Set([...current.filter((ruleKey) => !focusedSourceModels.some((rule) => rule.rule_key === ruleKey)), ...ruleKeys]))
    const selectedOrigins = Array.from(new Set(data.model_param_rules
      .filter((rule) => next.includes(rule.rule_key) && rule.original_provider)
      .map((rule) => rule.original_provider as string)))
    form.setValue('selected_model_ids', next, { shouldDirty: true, shouldValidate: true })
    form.setValue('selected_origins', selectedOrigins.length ? selectedOrigins : [focusedOrigin], { shouldDirty: true, shouldValidate: true })
  }

  const toggleSourceModel = (ruleKey: string) => {
    const current = form.getValues('selected_model_ids')
    const next = current.includes(ruleKey) ? current.filter((value) => value !== ruleKey) : [...current, ruleKey]
    const selectedOrigins = Array.from(new Set(data.model_param_rules
      .filter((rule) => next.includes(rule.rule_key) && rule.original_provider)
      .map((rule) => rule.original_provider as string)))
    form.setValue('selected_model_ids', next, { shouldDirty: true, shouldValidate: true })
    form.setValue('selected_origins', selectedOrigins.length ? selectedOrigins : [focusedOrigin], { shouldDirty: true, shouldValidate: true })
  }

  const setResolverSelection = (keys: string[]) => {
    const current = form.getValues('selected_resolver_rule_keys')
    form.setValue('selected_resolver_rule_keys', Array.from(new Set([...current.filter((key) => !focusedResolverSources.some((rule) => rule.key === key)), ...keys])), { shouldDirty: true, shouldValidate: true })
  }

  const toggleResolverSource = (key: string) => {
    const current = form.getValues('selected_resolver_rule_keys')
    setResolverSelection(current.includes(key) ? current.filter((item) => item !== key) : [...current, key])
  }

  const toggleDraftArray = (field: RuleArrayField, item: string) => {
    if (!paramSessionSnapshot) setParamSessionSnapshot(form.getValues('model_rule_drafts'))
    const drafts = form.getValues('model_rule_drafts')
    const selectedKeys = new Set(bulkDraftKeys)
    if (!selectedKeys.size) {
      return
    }
    const shouldRemove = selectedParamDrafts.length > 0 && selectedParamDrafts.every((draft) => (draft[field] ?? []).includes(item))
    const nextDrafts = drafts.map((entry) => {
      if (!selectedKeys.has(entry.draft_key)) {
        return entry
      }
      const nextValues = shouldRemove ? (entry[field] ?? []).filter((value) => value !== item) : Array.from(new Set([...(entry[field] ?? []), item]))
      if (field !== 'capabilities') {
        return { ...entry, [field]: nextValues }
      }
      const capabilityValues = { ...(entry.capability_values ?? {}) }
      if (shouldRemove) {
        delete capabilityValues[item]
      } else {
        const dictionary = capabilityItems.find((capability) => capability.key === item)
        capabilityValues[item] = dictionary?.value_type === 'number' ? getDefaultCapabilityNumber(item) : true
      }
      return {
        ...entry,
        capabilities: nextValues,
        capability_values: capabilityValues,
      }
    })
    form.setValue('model_rule_drafts', nextDrafts, { shouldDirty: true, shouldValidate: true })
  }

  const commonValue = <T,>(items: T[], read: (item: T) => string | number | boolean | undefined) => {
    if (!items.length) return ''
    const first = read(items[0])
    return items.every((item) => read(item) === first) ? first ?? '' : ''
  }

  const commonArrayState = (field: RuleArrayField, item: string) => {
    if (!selectedParamDrafts.length) return 'none'
    const count = selectedParamDrafts.filter((draft) => (draft[field] ?? []).includes(item)).length
    if (count === selectedParamDrafts.length) return 'full'
    return count > 0 ? 'partial' : 'none'
  }

  const updateDraftField = <K extends keyof ProviderWizardModelRuleDraft>(field: K, value: ProviderWizardModelRuleDraft[K]) => {
    if (!paramSessionSnapshot) setParamSessionSnapshot(form.getValues('model_rule_drafts'))
    const drafts = form.getValues('model_rule_drafts')
    const selectedKeys = new Set(bulkDraftKeys)
    const identityField = field === 'match_type' || field === 'model_id_selector' || field === 'priority' || field === 'original_provider'
    const nextDrafts = drafts.map((entry) => selectedKeys.has(entry.draft_key) && (!identityField || selectedKeys.size <= 1) ? { ...entry, [field]: value } : entry)
    form.setValue('model_rule_drafts', nextDrafts, { shouldDirty: true, shouldValidate: true })
  }

  const updateDraftCapabilityValue = (capability: string, value: string) => {
    if (!paramSessionSnapshot) setParamSessionSnapshot(form.getValues('model_rule_drafts'))
    const nextValue = value ? Number(value) : 0
    const selectedKeys = new Set(bulkDraftKeys)
    const nextDrafts = form.getValues('model_rule_drafts').map((entry) => selectedKeys.has(entry.draft_key)
      ? {
          ...entry,
          capability_values: {
            ...(entry.capability_values ?? {}),
            [capability]: nextValue,
          },
        }
      : entry)
      .map((entry) => selectedKeys.has(entry.draft_key) && !entry.capabilities.includes(capability)
        ? { ...entry, capability_values: Object.fromEntries(Object.entries(entry.capability_values ?? {}).filter(([key]) => key !== capability)) }
        : entry)
    form.setValue('model_rule_drafts', nextDrafts, { shouldDirty: true, shouldValidate: true })
  }

  const toggleBulkDraft = (draftKey: string) => {
    if (!bulkDraftKeys.length) setParamSessionSnapshot(form.getValues('model_rule_drafts'))
    setBulkDraftKeys((keys) => keys.includes(draftKey) ? keys.filter((key) => key !== draftKey) : [...keys, draftKey])
  }

  const applyParamSession = () => {
    setBulkDraftKeys([])
    setParamSessionSnapshot(null)
  }
  const cancelParamSession = () => {
    if (paramSessionSnapshot) {
      const selectedKeys = new Set(bulkDraftKeys)
      const snapshot = new Map(paramSessionSnapshot.map((draft) => [draft.draft_key, draft]))
      form.setValue('model_rule_drafts', form.getValues('model_rule_drafts').map((draft) => selectedKeys.has(draft.draft_key) ? snapshot.get(draft.draft_key) ?? draft : draft), { shouldDirty: true, shouldValidate: true })
    }
    setParamSessionSnapshot(null)
  }

  const setSelectedParamProviderOwner = () => {
    if (!bulkDraftKeys.length) return
    if (!paramSessionSnapshot) setParamSessionSnapshot(form.getValues('model_rule_drafts'))
    const selectedKeys = new Set(bulkDraftKeys)
    form.setValue('model_rule_drafts', form.getValues('model_rule_drafts').map((draft) => selectedKeys.has(draft.draft_key) ? { ...draft, original_provider: values.provider_key } : draft), { shouldDirty: true, shouldValidate: true })
  }

  const movePatternDraft = (draftKey: string, direction: -1 | 1) => {
    const ordered = form.getValues('model_rule_drafts').filter((draft) => draft.match_type === 'pattern').sort((a, b) => a.priority - b.priority)
    const index = ordered.findIndex((draft) => draft.draft_key === draftKey)
    const target = index + direction
    if (index < 0 || target < 0 || target >= ordered.length) return
    ;[ordered[index], ordered[target]] = [ordered[target], ordered[index]]
    const priorities = new Map(ordered.map((draft, position) => [draft.draft_key, position + 1]))
    form.setValue('model_rule_drafts', form.getValues('model_rule_drafts').map((draft) => priorities.has(draft.draft_key) ? { ...draft, priority: priorities.get(draft.draft_key) ?? draft.priority } : draft), { shouldDirty: true, shouldValidate: true })
  }

  const toggleMountTarget = (key: string) => setMountTargetKeys((keys) => keys.includes(key) ? keys.filter((item) => item !== key) : [...keys, key])

  const getMountsForTarget = (targetKey: string) => {
    if (targetKey.startsWith('model:')) {
      return form.getValues('model_rule_drafts').find((draft) => `model:${draft.draft_key}` === targetKey)?.logical_mounts ?? []
    }
    return form.getValues('resolver_rule_drafts').find((draft) => `resolver:${draft.draft_key}` === targetKey)?.logical_mounts ?? []
  }

  const getBulkMountPathState = (path: string): 'full' | 'partial' | 'none' => {
    const override = bulkMountPathOverrides[path]
    if (override) {
      return override
    }
    if (!mountTargetKeys.length) {
      return 'none'
    }
    const selectedCount = mountTargetKeys.filter((targetKey) => getMountsForTarget(targetKey).includes(path)).length
    if (selectedCount === mountTargetKeys.length) {
      return 'full'
    }
    return selectedCount > 0 ? 'partial' : 'none'
  }

  const toggleBulkMountPath = (path: string) => {
    const state = getBulkMountPathState(path)
    setBulkMountPathOverrides((overrides) => ({ ...overrides, [path]: state === 'full' ? 'none' : 'full' }))
  }

  const applyBulkMounts = () => {
    const targetKeys = new Set(mountTargetKeys)
    if (!targetKeys.size) return
    const fullPaths = logicalMounts.filter((path) => getBulkMountPathState(path) === 'full')
    const nonePaths = new Set(logicalMounts.filter((path) => getBulkMountPathState(path) === 'none'))
    const applyToMounts = (mounts: string[]) => Array.from(new Set([...mounts.filter((path) => !nonePaths.has(path)), ...fullPaths]))
    form.setValue('model_rule_drafts', form.getValues('model_rule_drafts').map((draft) => targetKeys.has(`model:${draft.draft_key}`) ? { ...draft, logical_mounts: applyToMounts(draft.logical_mounts) } : draft), { shouldDirty: true, shouldValidate: true })
    form.setValue('resolver_rule_drafts', form.getValues('resolver_rule_drafts').map((draft) => targetKeys.has(`resolver:${draft.draft_key}`) ? { ...draft, logical_mounts: applyToMounts(draft.logical_mounts) } : draft), { shouldDirty: true, shouldValidate: true })
    setMountTargetKeys([])
    setBulkMountPathOverrides({})
  }

  const discardBulkMounts = () => {
    setBulkMountPathOverrides({})
  }

  const addModelRuleDraft = (matchType: ProviderWizardModelRuleDraft['match_type']) => {
    const drafts = form.getValues('model_rule_drafts')
    const origin = values.provider_key
    const draft: ProviderWizardModelRuleDraft = {
      draft_key: `manual-${matchType}-${drafts.length + 1}`,
      match_type: matchType,
      original_provider: origin,
      model_id_selector: matchType === 'default' ? '' : matchType === 'pattern' ? `${origin}-*` : `${origin}-model`,
      priority: matchType === 'pattern' ? 200 + drafts.length : 999,
      source_rule_key: '',
      model_driver: values.provider_driver,
      api_types: values.selected_api_types,
      capabilities: values.selected_capabilities,
      capability_values: getDefaultCapabilityValues(values.selected_capabilities, capabilityItems),
      estimated_cost_usd: undefined,
      estimated_latency_ms: undefined,
      quality_score: undefined,
      latency_class: undefined,
      cost_class: undefined,
      logical_mounts: values.selected_logical_mounts,
      exclude: false,
    }
    form.setValue('model_rule_drafts', [...drafts, draft], { shouldDirty: true, shouldValidate: true })
    setBulkDraftKeys([draft.draft_key])
  }

  const updateResolverDraftField = <K extends keyof ProviderWizardResolverRuleDraft>(field: K, value: ProviderWizardResolverRuleDraft[K]) => {
    if (!resolverParamSessionSnapshot) setResolverParamSessionSnapshot(form.getValues('resolver_rule_drafts'))
    const drafts = form.getValues('resolver_rule_drafts')
    const selectedKeys = new Set(selectedResolverParamDrafts.map((draft) => draft.draft_key))
    if (!selectedKeys.size) {
      return
    }
    const identityField = field === 'model_id_selector' || field === 'original_provider' || field === 'rule_kind'
    const nextDrafts = drafts.map((entry) => selectedKeys.has(entry.draft_key) && (!identityField || selectedKeys.size <= 1) ? { ...entry, [field]: value } : entry)
    form.setValue('resolver_rule_drafts', nextDrafts, { shouldDirty: true, shouldValidate: true })
  }

  const toggleResolverDraftArray = (field: ResolverArrayField, item: string) => {
    if (!resolverParamSessionSnapshot) setResolverParamSessionSnapshot(form.getValues('resolver_rule_drafts'))
    const drafts = form.getValues('resolver_rule_drafts')
    const selectedKeys = new Set(selectedResolverParamDrafts.map((draft) => draft.draft_key))
    if (!selectedKeys.size) {
      return
    }
    const shouldRemove = selectedResolverParamDrafts.length > 0 && selectedResolverParamDrafts.every((draft) => (draft[field] ?? []).includes(item))
    form.setValue('resolver_rule_drafts', drafts.map((entry) => {
      if (!selectedKeys.has(entry.draft_key)) {
        return entry
      }
      const nextValues = shouldRemove ? (entry[field] ?? []).filter((value) => value !== item) : Array.from(new Set([...(entry[field] ?? []), item]))
      const capabilityValues = { ...(entry.capability_values ?? {}) }
      if (shouldRemove) {
        delete capabilityValues[item]
      } else {
        const dictionary = capabilityItems.find((capability) => capability.key === item)
        capabilityValues[item] = dictionary?.value_type === 'number' ? getDefaultCapabilityNumber(item) : true
      }
      return { ...entry, [field]: nextValues, capability_values: capabilityValues }
    }), { shouldDirty: true, shouldValidate: true })
  }

  const updateResolverCapabilityValue = (capability: string, value: string) => {
    if (!resolverParamSessionSnapshot) setResolverParamSessionSnapshot(form.getValues('resolver_rule_drafts'))
    const nextValue = value ? Number(value) : 0
    const selectedKeys = new Set(selectedResolverParamDrafts.map((draft) => draft.draft_key))
    form.setValue('resolver_rule_drafts', form.getValues('resolver_rule_drafts').map((entry) => selectedKeys.has(entry.draft_key)
      ? {
          ...entry,
          capability_values: {
            ...(entry.capability_values ?? {}),
            [capability]: nextValue,
          },
        }
      : entry)
      .map((entry) => selectedKeys.has(entry.draft_key) && !(entry.capabilities ?? []).includes(capability)
        ? { ...entry, capability_values: Object.fromEntries(Object.entries(entry.capability_values ?? {}).filter(([key]) => key !== capability)) }
        : entry), { shouldDirty: true, shouldValidate: true })
  }

  const resolverArrayState = (field: ResolverArrayField, item: string) => {
    if (!selectedResolverParamDrafts.length) return 'none'
    const count = selectedResolverParamDrafts.filter((draft) => (draft[field] ?? []).includes(item)).length
    if (count === selectedResolverParamDrafts.length) return 'full'
    return count > 0 ? 'partial' : 'none'
  }

  const commonTokenText = (field: ResolverTokenField) => {
    if (!selectedResolverParamDrafts.length) {
      return ''
    }
    const first = (selectedResolverParamDrafts[0][field] ?? []).join(', ')
    return selectedResolverParamDrafts.every((draft) => (draft[field] ?? []).join(', ') === first) ? first : ''
  }

  const updateResolverTokenField = (field: ResolverTokenField, value: string) => {
    updateResolverDraftField(field, parseTokenText(value))
  }

  const toggleBulkResolverDraft = (draftKey: string) => {
    if (!bulkResolverDraftKeys.length) setResolverParamSessionSnapshot(form.getValues('resolver_rule_drafts'))
    setBulkResolverDraftKeys((keys) => keys.includes(draftKey) ? keys.filter((key) => key !== draftKey) : [...keys, draftKey])
  }

  const applyResolverParamSession = () => {
    setBulkResolverDraftKeys([])
    setResolverParamSessionSnapshot(null)
  }

  const cancelResolverParamSession = () => {
    if (resolverParamSessionSnapshot) {
      const selectedKeys = new Set(bulkResolverDraftKeys)
      const snapshot = new Map(resolverParamSessionSnapshot.map((draft) => [draft.draft_key, draft]))
      form.setValue('resolver_rule_drafts', form.getValues('resolver_rule_drafts').map((draft) => selectedKeys.has(draft.draft_key) ? snapshot.get(draft.draft_key) ?? draft : draft), { shouldDirty: true, shouldValidate: true })
    }
    setResolverParamSessionSnapshot(null)
  }

  const setSelectedResolverProviderOwner = () => {
    if (!bulkResolverDraftKeys.length) return
    if (!resolverParamSessionSnapshot) setResolverParamSessionSnapshot(form.getValues('resolver_rule_drafts'))
    const selectedKeys = new Set(bulkResolverDraftKeys)
    form.setValue('resolver_rule_drafts', form.getValues('resolver_rule_drafts').map((draft) => selectedKeys.has(draft.draft_key) ? { ...draft, original_provider: values.provider_key } : draft), { shouldDirty: true, shouldValidate: true })
  }

  const addResolverRuleDraft = (ruleKind: ProviderWizardResolverRuleDraft['rule_kind']) => {
    const drafts = form.getValues('resolver_rule_drafts')
    const origin = values.provider_key
    const draft: ProviderWizardResolverRuleDraft = {
      draft_key: `manual-${ruleKind}-${drafts.length + 1}`,
      rule_kind: ruleKind,
      selector_type: 'pattern',
      original_provider: origin,
      model_id_selector: ruleKind === 'variant' ? '*' : `${origin}-*`,
      priority: 300 + drafts.length,
      nick: ruleKind === 'variant' ? 'fast' : 'standard',
      mount_suffix: ruleKind === 'variant' ? 'fast' : undefined,
      provider_options_json: ruleKind === 'variant' ? '{\n  "reasoning": {\n    "effort": "low"\n  }\n}' : undefined,
      family: ruleKind === 'version_rule' ? origin : undefined,
      tier: ruleKind === 'version_rule' ? 'standard' : undefined,
      model_pattern: ruleKind === 'version_rule' ? `${origin}-*` : undefined,
      tier_tokens: ruleKind === 'version_rule' ? [] : undefined,
      exclude_tier_tokens: ruleKind === 'version_rule' ? [] : undefined,
      version_rank_prefix: ruleKind === 'version_rule' ? origin : undefined,
      stability_unstable_tokens: ruleKind === 'version_rule' ? ['preview', 'beta'] : undefined,
      stability_current_requires_stable: ruleKind === 'version_rule' ? true : undefined,
      current_mount: ruleKind === 'version_rule' ? `${origin}.current` : undefined,
      version_mount: ruleKind === 'version_rule' ? `${origin}.{model}` : undefined,
      exclude_snapshot_date_suffix: ruleKind === 'version_rule' ? true : undefined,
      capabilities: values.selected_capabilities,
      source_rule_key: '',
      logical_mounts: values.selected_logical_mounts,
    }
    form.setValue('resolver_rule_drafts', [...drafts, draft], { shouldDirty: true, shouldValidate: true })
    setSelectedResolverDraftIndex(drafts.length)
    setBulkResolverDraftKeys([draft.draft_key])
    setResolverParamSessionSnapshot(form.getValues('resolver_rule_drafts'))
    setResolverParamsOrigin(origin)
    setResolverParamsTab(ruleKind)
  }

  const updateNickRule = (index: number, patch: Partial<ProviderWizardInput['nick_rules'][number]>) => {
    form.setValue('nick_rules', form.getValues('nick_rules').map((rule, ruleIndex) => ruleIndex === index ? { ...rule, ...patch } : rule), { shouldDirty: true, shouldValidate: true })
  }

  const addNickRule = () => {
    const rules = form.getValues('nick_rules')
    const origin = focusedOrigin || originalProviders[0] || 'openai'
    form.setValue('nick_rules', [...rules, { draft_key: `nick-${rules.length + 1}`, original_provider: origin, selector_type: 'pattern', model_id: '*', nick: `${origin}/{model}`, priority: rules.length + 1 }], { shouldDirty: true, shouldValidate: true })
  }

  const removeNickRule = (index: number) => {
    const rules = form.getValues('nick_rules')
    form.setValue('nick_rules', rules.filter((_, ruleIndex) => ruleIndex !== index), { shouldDirty: true, shouldValidate: true })
  }

  const updateOriginMappingRule = (index: number, patch: Partial<ProviderWizardInput['origin_mapping_rules'][number]>) => {
    form.setValue('origin_mapping_rules', form.getValues('origin_mapping_rules').map((rule, ruleIndex) => ruleIndex === index ? { ...rule, ...patch } : rule), { shouldDirty: true, shouldValidate: true })
  }

  const addOriginMappingRule = () => {
    const rules = form.getValues('origin_mapping_rules')
    form.setValue('origin_mapping_rules', [...rules, { draft_key: `origin-${rules.length + 1}`, mapping_mode: 'template', match_pattern: '*/*', origin_template: '<driver>/<model>', regex: '^(?<driver>[^/]+)/(?<model>.+)$', driver_transforms: ['alias'], model_transforms: ['trim'], priority: rules.length + 1 }], { shouldDirty: true, shouldValidate: true })
  }

  const removeOriginMappingRule = (index: number) => {
    const rules = form.getValues('origin_mapping_rules')
    form.setValue('origin_mapping_rules', rules.filter((_, ruleIndex) => ruleIndex !== index), { shouldDirty: true, shouldValidate: true })
  }

  const toggleOriginMappingDriverTransform = (index: number, op: 'trim' | 'lowercase' | 'alias') => {
    const rule = form.getValues('origin_mapping_rules')[index]
    const next = rule.driver_transforms.includes(op)
      ? rule.driver_transforms.filter((item) => item !== op)
      : [...rule.driver_transforms, op]
    updateOriginMappingRule(index, { driver_transforms: next })
  }

  const toggleOriginMappingModelTransform = (index: number, op: 'trim' | 'lowercase') => {
    const rule = form.getValues('origin_mapping_rules')[index]
    const next = rule.model_transforms.includes(op)
      ? rule.model_transforms.filter((item) => item !== op)
      : [...rule.model_transforms, op]
    updateOriginMappingRule(index, { model_transforms: next })
  }

  const updateOriginAlias = (index: number, patch: Partial<ProviderWizardInput['origin_provider_aliases'][number]>) => {
    form.setValue('origin_provider_aliases', form.getValues('origin_provider_aliases').map((alias, aliasIndex) => aliasIndex === index ? { ...alias, ...patch } : alias), { shouldDirty: true, shouldValidate: true })
  }

  const addOriginAlias = () => {
    const aliases = form.getValues('origin_provider_aliases')
    form.setValue('origin_provider_aliases', [...aliases, { draft_key: `alias-${aliases.length + 1}`, alias: 'provider-alias', driver: 'provider-driver' }], { shouldDirty: true, shouldValidate: true })
  }

  const removeOriginAlias = (index: number) => {
    const aliases = form.getValues('origin_provider_aliases')
    form.setValue('origin_provider_aliases', aliases.filter((_, aliasIndex) => aliasIndex !== index), { shouldDirty: true, shouldValidate: true })
  }

  const validateModelParamDrafts = (drafts: ProviderWizardModelRuleDraft[]) => {
    const missingSelector = drafts.find((draft) => draft.match_type !== 'default' && !draft.model_id_selector.trim())
    if (missingSelector) {
      form.setError('model_rule_drafts', { message: 'Model selector is required for every exact and pattern rule.' })
      return false
    }
    const identityKeys = drafts.map((draft) => `${draft.match_type}:${draft.match_type === 'default' ? '*' : draft.model_id_selector}`)
    if (new Set(identityKeys).size !== identityKeys.length) {
      form.setError('model_rule_drafts', { message: 'Each exact model, pattern, and default match identity must be unique.' })
      return false
    }
    if (drafts.filter((draft) => draft.match_type === 'default').length > 1) {
      form.setError('model_rule_drafts', { message: 'Only one default is allowed. Convert origin defaults to patterns before adding the provider default.' })
      return false
    }
    form.clearErrors('model_rule_drafts')
    return true
  }

  const validateBasicFields = () => {
    if (nameConflict) {
      form.setError('name', { message: 'Provider name must be unique ignoring case.' })
      return false
    }
    form.clearErrors('name')
    return true
  }

  const nextStep = () => {
    if (currentStep === 'basic' && !validateBasicFields()) {
      return
    }
    if (currentStep === 'params' && !validateModelParamDrafts(form.getValues('model_rule_drafts'))) {
      return
    }
    setStepIndex((index) => Math.min(steps.length - 1, index + 1))
  }

  const complete = form.handleSubmit(async (input) => {
    setSubmitState('saving')
    setSubmitMessage(t('wizard.savingPreview', 'Saving provider draft and building publish preview...'))
    if (!validateBasicFields()) {
      setSubmitState('invalid')
      setSubmitMessage(t('wizard.previewInvalidBasic', 'Basic provider fields need attention before the publish preview can be generated.'))
      setStepIndex(0)
      return
    }
    if (!validateModelParamDrafts(input.model_rule_drafts)) {
      setSubmitState('invalid')
      setSubmitMessage(t('wizard.previewInvalidParams', 'Model parameter drafts need attention before the publish preview can be generated.'))
      setStepIndex(2)
      return
    }
    try {
      await runProviderWizard(input)
      await runPublishPreview()
      navigate(`/publish?source=wizard&provider=${encodeURIComponent(input.provider_key)}`)
    } catch (error) {
      setSubmitState('error')
      setSubmitMessage(error instanceof Error ? error.message : String(error))
    }
  }, () => {
    setSubmitState('invalid')
    setSubmitMessage(t('wizard.previewInvalid', 'Some wizard fields are incomplete. Use Inspect or revisit the highlighted steps before saving.'))
  })

  return (
    <div className="space-y-4" data-testid="provider-wizard-page">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('wizard.title', 'Provider Wizard')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{t('wizard.subtitle', 'Technical provider setup workflow')}</p>
        </div>
        <StatusBadge tone="accent">{t(`wizard.step.${currentStep}`, currentStep)}</StatusBadge>
      </header>

      <div className="grid gap-4 xl:grid-cols-[220px_minmax(0,1fr)]">
        <nav className="shell-card p-3" aria-label={t('wizard.steps', 'Wizard steps')}>
          <div className="space-y-2">
            {steps.map((step, index) => (
              <button
                className={`flex w-full items-center justify-between rounded-md px-3 py-2 text-left text-sm ${index === stepIndex ? 'bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-text)]' : 'text-[color:var(--cp-muted)] hover:bg-[color:var(--cp-surface-2)]'}`}
                key={step}
                onClick={() => setStepIndex(index)}
                type="button"
              >
                <span>{t(`wizard.step.${step}`, step)}</span>
                {index < stepIndex && <Check size={14} />}
              </button>
            ))}
          </div>
        </nav>

        <form className="shell-card p-4" onSubmit={complete}>
          {currentStep === 'basic' && (
            <StepGrid>
              <Field label={t('providers.key', 'Provider key')} error={form.formState.errors.provider_key?.message}>
                <input type="hidden" {...form.register('provider_key')} />
                <div className="flex h-10 items-center rounded-md border border-dashed border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2)] px-3 font-mono text-sm text-[color:var(--cp-muted)]">
                  {values.provider_key}
                </div>
              </Field>
              <Field label={t('providers.name', 'Name')} error={form.formState.errors.name?.message || (nameConflict ? t('wizard.nameConflict', 'Provider name must be unique ignoring case.') : undefined)}>
                <input className={inputClass} {...form.register('name')} />
              </Field>
              <Field label={t('providers.template', 'Template')}>
                <select className={inputClass} {...form.register('template_provider_key')}>
                  <option value="">{t('providers.noTemplate', 'Start from scratch')}</option>
                  {data.providers.map((provider) => <option key={provider.provider_key} value={provider.provider_key}>{provider.name}</option>)}
                </select>
              </Field>
              <Field label={t('providers.kind', 'Kind')}>
                <select className={inputClass} {...form.register('provider_kind')}>
                  <option value="aggregator">{t('providers.aggregator', 'Aggregator')}</option>
                  <option value="origin">{t('providers.origin', 'Origin')}</option>
                </select>
              </Field>
              <Field label={t('providers.driver', 'Driver')}>
                <input className={`${inputClass} border-dashed bg-[color:var(--cp-surface-2)] font-mono text-[color:var(--cp-muted)]`} readOnly {...form.register('provider_driver')} />
                <span className="text-xs font-normal text-[color:var(--cp-muted)]">{t('wizard.driverHint', 'Derived from the lowercase provider name and used as the provider_driver JSON field and delivery filename.')}</span>
              </Field>
              <Field label={t('providers.protocol', 'Protocol family')}>
                <select className={inputClass} {...form.register('protocol_family')}>
                  {protocolOptions.map((family) => <option key={family} value={family}>{family}</option>)}
                </select>
                <span className="text-xs font-normal text-[color:var(--cp-muted)]">{t('wizard.protocolHint', 'Client-facing wire protocol family. Providers can reuse a family.')}</span>
              </Field>
              <Field label={t('providers.baseUrl', 'Base URL')}>
                <input className={inputClass} {...form.register('base_url')} />
              </Field>
            </StepGrid>
          )}

          {currentStep === 'models' && (
            <StepGrid>
              <ChoicePanel title={t('wizard.origins', 'Original providers')}>
                {originStats.map((stat) => (
                  <button
                    className={`flex min-h-12 w-full items-center justify-between rounded-md border px-3 py-2 text-left text-sm ${focusedOrigin === stat.provider ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`}
                    key={stat.provider}
                    onClick={() => setFocusedOrigin(stat.provider)}
                    type="button"
                  >
                    <span className="font-mono text-xs">{stat.provider}</span>
                    <span className="text-xs text-[color:var(--cp-muted)]">{stat.selectedCount}/{stat.totalCount}</span>
                  </button>
                ))}
              </ChoicePanel>
              <ChoicePanel title={t('wizard.sourcePatterns', 'Existing model match rules')}>
                <div className="mb-2 flex gap-2" role="tablist" aria-label={t('wizard.sourcePatterns', 'Existing model match rules')}>
                  {availableModelTabs.map((tab) => {
                    const tabRules = focusedSourceModels.filter((rule) => rule.match_type === tab)
                    const selectedCount = tabRules.filter((rule) => values.selected_model_ids.includes(rule.rule_key)).length
                    return <button className={`rounded-md border px-2 py-1 text-xs font-semibold ${modelTab === tab ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-accent)]' : 'border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2)]'}`} key={tab} onClick={() => setModelTab(tab)} role="tab" type="button">{tab === 'exact' ? 'models' : `${tab}s`} {selectedCount}/{tabRules.length}</button>
                  })}
                </div>
                <div className="mb-2 flex flex-wrap gap-2">
                  <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => setSourceModelSelection(focusedSourceModelsByTab.map((rule) => rule.rule_key))} type="button">{t('action.selectAll', 'Select all')}</button>
                  <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => {
                    const selected = new Set(values.selected_model_ids)
                    setSourceModelSelection(focusedSourceModelsByTab.map((rule) => rule.rule_key).filter((ruleKey) => !selected.has(ruleKey)))
                  }} type="button">{t('action.invert', 'Invert')}</button>
                  <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => setSourceModelSelection([])} type="button">{t('action.clear', 'Clear')}</button>
                </div>
                <div className="grid gap-2 md:grid-cols-2">
                  {focusedSourceModelsByTab.slice(0, 24).map((rule, index) => (
                    <CheckRow
                      checked={values.selected_model_ids.includes(rule.rule_key)}
                      key={`${rule.rule_key}-${index}`}
                      label={rule.model_id_selector ?? rule.rule_key}
                      meta={rule.match_type}
                      onClick={() => toggleSourceModel(rule.rule_key)}
                    />
                  ))}
                </div>
              </ChoicePanel>
              <ChoicePanel title={t('wizard.selectedPatterns', 'Selected match rules')}>
                <StatusBadge tone="accent">{values.selected_model_ids.length}</StatusBadge>
                <div className="mt-2 space-y-2">
                  {(['exact', 'pattern', 'default'] as const).map((kind) => selectedSourceModels.filter((rule) => rule.match_type === kind).map((rule) => (
                    <button className="block w-full rounded-md border border-[color:var(--cp-border)] px-3 py-2 text-left text-xs" key={rule.rule_key} onClick={() => toggleSourceModel(rule.rule_key)} type="button">
                      <div className="font-mono">{rule.model_id_selector ?? 'defaults'}</div>
                      <div className="flex items-center justify-between gap-2 text-[color:var(--cp-muted)]"><span>{kind} · {rule.original_provider}</span><Check size={14} /></div>
                    </button>
                  ))) }
                </div>
              </ChoicePanel>
            </StepGrid>
          )}

          {currentStep === 'patternOrder' && (
            <StepGrid>
              <ChoicePanel title="Pattern order" wide>
                <p className="mb-3 text-xs text-[color:var(--cp-muted)]">Earlier patterns have higher match priority. Move patterns into their intended published order.</p>
                <div className="space-y-2">
                  {values.model_rule_drafts.filter((draft) => draft.match_type === 'pattern').sort((a, b) => a.priority - b.priority).map((draft, index, patterns) => (
                    <div className="flex items-center gap-3 rounded-md border border-[color:var(--cp-border)] px-3 py-2" key={draft.draft_key}>
                      <span className="w-6 text-center text-xs text-[color:var(--cp-muted)]">{index + 1}</span>
                      <span className="min-w-0 flex-1 font-mono text-xs">{draft.original_provider}:{draft.model_id_selector}</span>
                      <button aria-label={`Move ${draft.model_id_selector} up`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)] disabled:opacity-40" disabled={index === 0} onClick={() => movePatternDraft(draft.draft_key, -1)} type="button"><ArrowUp size={14} /></button>
                      <button aria-label={`Move ${draft.model_id_selector} down`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)] disabled:opacity-40" disabled={index === patterns.length - 1} onClick={() => movePatternDraft(draft.draft_key, 1)} type="button"><ArrowDown size={14} /></button>
                    </div>
                  ))}
                </div>
              </ChoicePanel>
            </StepGrid>
          )}

          {currentStep === 'resolver' && (
            <StepGrid>
              <ChoicePanel title={t('wizard.origins', 'Original providers')}>
                {resolverOriginStats.map((stat) => (
                  <button
                    className={`flex min-h-12 w-full items-center justify-between rounded-md border px-3 py-2 text-left text-sm ${focusedOrigin === stat.provider ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`}
                    key={`resolver-${stat.provider}`}
                    onClick={() => setFocusedOrigin(stat.provider)}
                    type="button"
                  >
                    <span className="font-mono text-xs">{stat.provider}</span>
                    <span className="text-xs text-[color:var(--cp-muted)]">{stat.selectedCount}/{stat.totalCount}</span>
                  </button>
                ))}
              </ChoicePanel>
              <ChoicePanel title={t('wizard.resolverDrafts', 'Existing variants and version rules')}>
                <div className="mb-2 flex gap-2" role="tablist">
                  {availableResolverTabs.map((tab) => {
                    const rules = resolverSources.filter((rule) => rule.original_provider === focusedOrigin && rule.rule_kind === tab)
                    const selectedCount = rules.filter((rule) => values.selected_resolver_rule_keys.includes(rule.key)).length
                    return <button className={`rounded-md border px-2 py-1 text-xs font-semibold ${resolverTab === tab ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-accent)]' : 'border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2)]'}`} key={tab} onClick={() => setResolverTab(tab)} type="button">{tab === 'variant' ? t('wizard.variants', 'Variants') : t('wizard.versionRules', 'Version rules')} {selectedCount}/{rules.length}</button>
                  })}
                </div>
                <div className="mb-2 flex flex-wrap gap-2">
                  <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => setResolverSelection(focusedResolverSources.map((rule) => rule.key))} type="button">{t('action.selectAll', 'Select all')}</button>
                  <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => { const selected = new Set(values.selected_resolver_rule_keys); setResolverSelection(focusedResolverSources.map((rule) => rule.key).filter((key) => !selected.has(key))) }} type="button">{t('action.invert', 'Invert')}</button>
                  <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => setResolverSelection([])} type="button">{t('action.clear', 'Clear')}</button>
                </div>
                <div className="grid gap-2 md:grid-cols-2">
                  {focusedResolverSources.slice(0, 24).map((rule) => (
                    <CheckRow
                      checked={values.selected_resolver_rule_keys.includes(rule.key)}
                      key={rule.key}
                      label={rule.model_id_selector}
                      meta={describeResolverSource(rule.key, data.metadata_version_rules, data.metadata_variants)}
                      onClick={() => toggleResolverSource(rule.key)}
                    />
                  ))}
                </div>
              </ChoicePanel>
              <ChoicePanel title={t('wizard.resolverDrafts', 'Variant and version rule drafts')}>
                {values.resolver_rule_drafts.map((draft, index) => (
                  <button
                    className={`block w-full rounded-md border px-3 py-2 text-left text-xs ${index === selectedResolverDraftIndex ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`}
                    key={draft.draft_key}
                    onClick={() => setSelectedResolverDraftIndex(index)}
                    type="button"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <StatusBadge tone={draft.rule_kind === 'variant' ? 'accent' : 'warning'}>{draft.rule_kind}</StatusBadge>
                      <span className="font-mono">{draft.priority}</span>
                    </div>
                    <div className="mt-1 font-mono">{draft.model_id_selector}</div>
                    <div className="text-[color:var(--cp-muted)]">{draft.original_provider}</div>
                  </button>
                ))}
              </ChoicePanel>
            </StepGrid>
          )}

          {currentStep === 'resolverParams' && (
            <div className="space-y-3">
              <h2 className="text-sm font-bold">{t('wizard.editResolverRule', 'Edit selected variant/version rule')}</h2>
              <div className="flex flex-wrap gap-2" role="tablist" aria-label={t('wizard.step.resolverParams', 'Variant/version params')}>
                {(['variant', 'version_rule'] as const).filter((tab) => values.resolver_rule_drafts.some((draft) => draft.rule_kind === tab)).map((tab) => {
                  const count = values.resolver_rule_drafts.filter((draft) => draft.rule_kind === tab).length
                  return (
                    <button className={`h-9 rounded-md px-3 text-sm font-semibold ${resolverParamsTab === tab ? 'bg-[color:var(--cp-accent)] text-white' : 'border border-[color:var(--cp-border)]'}`} key={tab} onClick={() => setResolverParamsTab(tab)} type="button">
                      {tab === 'variant' ? t('wizard.variants', 'Variants') : t('wizard.versionRules', 'Version rules')} {count}
                    </button>
                  )
                })}
              </div>
              <StepGrid>
                <ChoicePanel title={t('wizard.origins', 'Providers')}>
                  {resolverParamOriginsForTab.map((origin) => {
                    const count = values.resolver_rule_drafts.filter((draft) => (draft.source_rule_key ? draft.original_provider : values.provider_key) === origin && draft.rule_kind === resolverParamsTab).length
                    return count ? <button className={`flex w-full items-center justify-between rounded-md border px-3 py-2 text-left text-xs ${activeResolverParamsOrigin === origin ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`} key={origin} onClick={() => setResolverParamsOrigin(origin)} type="button"><span className="font-mono">{origin === values.provider_key ? `${origin} (provider)` : origin}</span><span>{count}</span></button> : null
                  })}
                </ChoicePanel>
                <ChoicePanel title={resolverParamsTab === 'variant' ? t('wizard.variants', 'Variants') : t('wizard.versionRules', 'Version rules')}>
                  <div className="mb-2 flex flex-wrap gap-2">
                    <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => addResolverRuleDraft(resolverParamsTab)} type="button">{resolverParamsTab === 'variant' ? t('wizard.addVariant', 'Add variant') : t('wizard.addVersionRule', 'Add version rule')}</button>
                  </div>
                  {resolverParamsDrafts.map((draft) => {
                    const checked = bulkResolverDraftKeys.includes(draft.draft_key)
                    return (
                      <button
                        className={`mb-2 flex w-full items-center justify-between gap-2 rounded-md border px-3 py-2 text-left text-xs ${checked ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`}
                        key={`resolver-param-${draft.draft_key}`}
                        onClick={() => toggleBulkResolverDraft(draft.draft_key)}
                        type="button"
                      >
                        <span className="min-w-0">
                          <span className="font-mono">{draft.model_id_selector}</span>
                          <span className="ml-2 text-[color:var(--cp-muted)]">{draft.selector_type}</span>
                        </span>
                        {checked && <Check size={14} />}
                      </button>
                    )
                  })}
                </ChoicePanel>
                <ChoicePanel title={t('wizard.editQueue', 'Pending edit list')}>
                  {selectedResolverParamDrafts.length ? selectedResolverParamDrafts.map((draft) => (
                    <button className="block w-full rounded-md border border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] px-3 py-2 text-left text-xs" key={`selected-resolver-${draft.draft_key}`} onClick={() => toggleBulkResolverDraft(draft.draft_key)} type="button">
                      <div className="flex items-center justify-between gap-2"><span className="font-mono">{draft.model_id_selector}</span><Check size={14} /></div>
                      <div className="text-[color:var(--cp-muted)]">{draft.rule_kind}</div>
                    </button>
                  )) : <p className="text-xs text-[color:var(--cp-muted)]">{t('wizard.noEditTargets', 'Select one or more items to edit.')}</p>}
                </ChoicePanel>
              </StepGrid>
              {selectedResolverParamDrafts.length > 0 && selectedResolverParamDraft && resolverParamsTab === 'variant' && (
                <ChoicePanel title={t('wizard.editVariantRule', 'Edit selected variant')} wide>
                  <div className="grid gap-3 md:grid-cols-3">
                    <Field label={t('rules.type', 'Type')}>
                      <div className="flex h-10 items-center rounded-md border border-dashed border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2)] px-3 text-sm font-semibold text-[color:var(--cp-muted)]">{t('wizard.variants', 'Variants')}</div>
                    </Field>
                    <Field label={t('models.originalProvider', 'Original provider')}>
                      <div className="flex min-h-10 items-center justify-between gap-2 rounded-md border border-dashed border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2)] px-3 text-sm">
                        <span className="truncate font-mono text-[color:var(--cp-muted)]">{selectedResolverParamProviderOwned ? values.provider_key : selectedResolverParamOrigins.join(', ')}</span>
                        {!selectedResolverParamProviderOwned && (
                          <button className="shrink-0 rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold text-[color:var(--cp-text)]" onClick={setSelectedResolverProviderOwner} type="button">
                            {t('wizard.setCurrentProvider', 'Set current')}
                          </button>
                        )}
                      </div>
                    </Field>
                    <Field label={t('table.modelId', 'Model selector')}>
                      <input
                        className={`${inputClass} font-mono`}
                        disabled={selectedResolverParamDrafts.length > 1}
                        placeholder={selectedResolverParamDrafts.length > 1 ? t('wizard.mixedValue', 'Mixed values') : ''}
                        value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.model_id_selector))}
                        onChange={(event) => updateResolverDraftField('model_id_selector', event.target.value)}
                      />
                    </Field>
                    <Field label={t('rules.selector', 'Selector')}>
                      <select className={inputClass} value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.selector_type))} onChange={(event) => updateResolverDraftField('selector_type', event.target.value as ProviderWizardResolverRuleDraft['selector_type'])}>
                        <option disabled value="">{t('wizard.mixedValue', 'Mixed values')}</option>
                        <option value="pattern">{t('nick.pattern', 'Pattern rewrite')}</option>
                        <option value="exact">{t('nick.exact', 'Exact nick')}</option>
                      </select>
                    </Field>
                    <Field label={t('nick.exact', 'Exact nick')}>
                      <input className={inputClass} placeholder={t('wizard.mixedValue', 'Mixed values')} value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.nick))} onChange={(event) => updateResolverDraftField('nick', event.target.value)} />
                    </Field>
                    <Field label={t('resolver.mountSuffix', 'Mount suffix')}>
                      <input className={inputClass} placeholder={t('wizard.mixedValue', 'Mixed values')} value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.mount_suffix))} onChange={(event) => updateResolverDraftField('mount_suffix', event.target.value)} />
                    </Field>
                    <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)] md:col-span-3">
                      {t('resolver.providerOptions', 'Provider options JSON')}
                      <textarea
                        className="min-h-28 rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-2 font-mono text-xs text-[color:var(--cp-text)]"
                        placeholder={selectedResolverParamDrafts.length > 1 ? t('wizard.mixedValue', 'Mixed values') : '{\n  "reasoning": {\n    "effort": "high"\n  }\n}'}
                        value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.provider_options_json))}
                        onChange={(event) => updateResolverDraftField('provider_options_json', event.target.value)}
                      />
                    </label>
                  </div>
                  <div className="mt-3 flex justify-end gap-2"><button className="h-9 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" onClick={cancelResolverParamSession} type="button">{t('action.discard', 'Discard')}</button><button className="h-9 rounded-md bg-[color:var(--cp-accent)] px-3 text-xs font-semibold text-white" onClick={applyResolverParamSession} type="button">{t('action.apply', 'Apply')}</button></div>
                </ChoicePanel>
              )}
              {selectedResolverParamDrafts.length > 0 && selectedResolverParamDraft && resolverParamsTab === 'version_rule' && (
                <ChoicePanel title={t('wizard.editVersionRule', 'Edit selected version rule')} wide>
                  <div className="grid gap-3 md:grid-cols-3">
                    <Field label={t('rules.type', 'Type')}>
                      <div className="flex h-10 items-center rounded-md border border-dashed border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2)] px-3 text-sm font-semibold text-[color:var(--cp-muted)]">{t('wizard.versionRules', 'Version rules')}</div>
                    </Field>
                    <Field label={t('models.originalProvider', 'Original provider')}>
                      <div className="flex min-h-10 items-center justify-between gap-2 rounded-md border border-dashed border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2)] px-3 text-sm">
                        <span className="truncate font-mono text-[color:var(--cp-muted)]">{selectedResolverParamProviderOwned ? values.provider_key : selectedResolverParamOrigins.join(', ')}</span>
                        {!selectedResolverParamProviderOwned && (
                          <button className="shrink-0 rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold text-[color:var(--cp-text)]" onClick={setSelectedResolverProviderOwner} type="button">
                            {t('wizard.setCurrentProvider', 'Set current')}
                          </button>
                        )}
                      </div>
                    </Field>
                    <Field label={t('table.modelId', 'Model selector')}>
                      <input
                        className={`${inputClass} font-mono`}
                        disabled={selectedResolverParamDrafts.length > 1}
                        placeholder={selectedResolverParamDrafts.length > 1 ? t('wizard.mixedValue', 'Mixed values') : ''}
                        value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.model_id_selector))}
                        onChange={(event) => updateResolverDraftField('model_id_selector', event.target.value)}
                      />
                    </Field>
                    <Field label={t('rules.selector', 'Selector')}>
                      <select className={inputClass} value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.selector_type))} onChange={(event) => updateResolverDraftField('selector_type', event.target.value as ProviderWizardResolverRuleDraft['selector_type'])}>
                        <option disabled value="">{t('wizard.mixedValue', 'Mixed values')}</option>
                        <option value="pattern">{t('nick.pattern', 'Pattern rewrite')}</option>
                        <option value="exact">{t('nick.exact', 'Exact nick')}</option>
                      </select>
                    </Field>
                    <Field label={t('nick.exact', 'Exact nick')}>
                      <input className={inputClass} placeholder={t('wizard.mixedValue', 'Mixed values')} value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.nick))} onChange={(event) => updateResolverDraftField('nick', event.target.value)} />
                    </Field>
                    <Field label={t('resolver.family', 'Family')}>
                      <input className={inputClass} placeholder={t('wizard.mixedValue', 'Mixed values')} value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.family))} onChange={(event) => updateResolverDraftField('family', event.target.value)} />
                    </Field>
                    <Field label={t('resolver.tier', 'Tier')}>
                      <input className={inputClass} placeholder={t('wizard.mixedValue', 'Mixed values')} value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.tier))} onChange={(event) => updateResolverDraftField('tier', event.target.value)} />
                    </Field>
                    <Field label={t('resolver.modelPattern', 'Model pattern')}>
                      <input className={`${inputClass} font-mono`} placeholder={t('wizard.mixedValue', 'Mixed values')} value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.model_pattern))} onChange={(event) => updateResolverDraftField('model_pattern', event.target.value)} />
                    </Field>
                    <Field label={t('resolver.versionRankPrefix', 'Version rank prefix')}>
                      <input className={inputClass} placeholder={t('wizard.mixedValue', 'Mixed values')} value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.version_rank_prefix))} onChange={(event) => updateResolverDraftField('version_rank_prefix', event.target.value)} />
                    </Field>
                    <MountPathPicker
                      clearLabel={t('action.clear', 'Clear')}
                      directories={logicalDirectories}
                      editLabel={t('mode.edit', 'Edit')}
                      emptyLabel={t('resolver.noMountSelected', 'No mount selected')}
                      label={t('resolver.currentMount', 'Current mount')}
                      value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.current_mount))}
                      onChange={(mount) => updateResolverDraftField('current_mount', mount)}
                    />
                    <MountPathPicker
                      clearLabel={t('action.clear', 'Clear')}
                      directories={logicalDirectories}
                      editLabel={t('mode.edit', 'Edit')}
                      emptyLabel={t('resolver.noMountSelected', 'No mount selected')}
                      label={t('resolver.versionMount', 'Version mount')}
                      value={String(commonValue(selectedResolverParamDrafts, (draft) => draft.version_mount))}
                      onChange={(mount) => updateResolverDraftField('version_mount', mount)}
                    />
                    <label className="flex items-center gap-2 self-end text-sm font-semibold">
                      <input
                        checked={commonValue(selectedResolverParamDrafts, (draft) => draft.stability_current_requires_stable) === true}
                        className="h-4 w-4"
                        onChange={(event) => updateResolverDraftField('stability_current_requires_stable', event.target.checked)}
                        type="checkbox"
                      />
                      {t('resolver.currentRequiresStable', 'Current requires stable')}
                    </label>
                    <label className="flex items-center gap-2 self-end text-sm font-semibold">
                      <input
                        checked={commonValue(selectedResolverParamDrafts, (draft) => draft.exclude_snapshot_date_suffix) === true}
                        className="h-4 w-4"
                        onChange={(event) => updateResolverDraftField('exclude_snapshot_date_suffix', event.target.checked)}
                        type="checkbox"
                      />
                      {t('resolver.excludeSnapshotDateSuffix', 'Exclude snapshot date suffix')}
                    </label>
                  </div>
                  <div className="mt-3 grid gap-3 lg:grid-cols-2">
                    <Field label={t('resolver.tierTokens', 'Tier tokens')}>
                      <input
                        className={`${inputClass} font-mono`}
                        placeholder={t('wizard.freeTextTokens', 'Comma or space separated tokens')}
                        value={commonTokenText('tier_tokens')}
                        onChange={(event) => updateResolverTokenField('tier_tokens', event.target.value)}
                      />
                    </Field>
                    <Field label={t('resolver.excludeTierTokens', 'Exclude tier tokens')}>
                      <input
                        className={`${inputClass} font-mono`}
                        placeholder={t('wizard.freeTextTokens', 'Comma or space separated tokens')}
                        value={commonTokenText('exclude_tier_tokens')}
                        onChange={(event) => updateResolverTokenField('exclude_tier_tokens', event.target.value)}
                      />
                    </Field>
                    <Field label={t('resolver.unstableTokens', 'Unstable tokens')}>
                      <input
                        className={`${inputClass} font-mono`}
                        placeholder={t('wizard.freeTextTokens', 'Comma or space separated tokens')}
                        value={commonTokenText('stability_unstable_tokens')}
                        onChange={(event) => updateResolverTokenField('stability_unstable_tokens', event.target.value)}
                      />
                    </Field>
                    <TokenPanel title={t('resolver.versionCapabilities', 'Version rule capabilities')}>
                      {groupedCapabilityItems.map((capability) => {
                        const state = resolverArrayState('capabilities', capability.key)
                        return (
                          <CapabilityChoice
                            inputPlaceholder={t('wizard.mixedValue', 'Mixed values')}
                            item={capability}
                            key={capability.key}
                            label={state === 'partial' ? `${capability.key} (partial)` : capability.key}
                            onChangeNumber={(value) => updateResolverCapabilityValue(capability.key, value)}
                            onToggle={() => toggleResolverDraftArray('capabilities', capability.key)}
                            selected={state !== 'none'}
                            value={commonValue(selectedResolverParamDrafts.filter((draft) => (draft.capabilities ?? []).includes(capability.key)), (draft) => draft.capability_values?.[capability.key])}
                          />
                        )
                      })}
                    </TokenPanel>
                  </div>
                  <div className="mt-3 flex justify-end gap-2"><button className="h-9 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" onClick={cancelResolverParamSession} type="button">{t('action.discard', 'Discard')}</button><button className="h-9 rounded-md bg-[color:var(--cp-accent)] px-3 text-xs font-semibold text-white" onClick={applyResolverParamSession} type="button">{t('action.apply', 'Apply')}</button></div>
                </ChoicePanel>
              )}
            </div>
          )}
          {currentStep === 'params' && (
            <div className="space-y-3">
              {form.formState.errors.model_rule_drafts?.message && <p className="rounded-md border border-[color:var(--cp-danger)] px-3 py-2 text-xs text-[color:var(--cp-danger)]">{form.formState.errors.model_rule_drafts.message}</p>}
              <StepGrid>
                <ChoicePanel title={t('wizard.origins', 'Providers')}>
                  {paramOrigins.map((origin) => {
                    const count = values.model_rule_drafts.filter((draft) => (draft.source_rule_key ? draft.original_provider : values.provider_key) === origin).length
                    return count ? <button className={`flex w-full items-center justify-between rounded-md border px-3 py-2 text-left text-xs ${activeParamsOrigin === origin ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`} key={origin} onClick={() => setParamsOrigin(origin)} type="button"><span className="font-mono">{origin === values.provider_key ? `${origin} (provider)` : origin}</span><span>{count}</span></button> : null
                  })}
                </ChoicePanel>
                <ChoicePanel title={t('wizard.modelRuleDrafts', 'Models / patterns / defaults')}>
                  <div className="mb-2 flex flex-wrap gap-2">
                    <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => addModelRuleDraft('exact')} type="button">{t('models.createExact', 'Create exact rule')}</button>
                    <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => addModelRuleDraft('pattern')} type="button">{t('models.createPattern', 'Create pattern rule')}</button>
                    <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => addModelRuleDraft('default')} type="button">{t('models.createDefault', 'Create default rule')}</button>
                  </div>
                  <div className="mb-2 flex gap-1" role="tablist">{availableParamsTabs.map((tab) => { const count = values.model_rule_drafts.filter((draft) => (draft.source_rule_key ? draft.original_provider : values.provider_key) === activeParamsOrigin && draft.match_type === tab).length; return <button className={`rounded-md border px-2 py-1 text-xs ${paramsTab === tab ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`} key={tab} onClick={() => setParamsTab(tab)} type="button">{tab === 'exact' ? 'models' : `${tab}s`} {count}</button> })}</div>
                  {paramsDrafts.map((draft) => {
                    const checked = bulkDraftKeys.includes(draft.draft_key)
                    return <button className={`mb-2 flex w-full items-center justify-between gap-2 rounded-md border px-3 py-2 text-left text-xs ${checked ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`} key={draft.draft_key} onClick={() => toggleBulkDraft(draft.draft_key)} type="button"><span className="min-w-0"><span className="font-mono">{draft.model_id_selector || 'defaults'}</span><span className="ml-2 text-[color:var(--cp-muted)]">{draft.match_type}</span></span>{checked && <Check size={14} />}</button>
                  })}
                </ChoicePanel>
                <ChoicePanel title={t('wizard.editQueue', 'Pending edit list')}>
                  {selectedParamDrafts.length ? selectedParamDrafts.map((draft) => (
                    <button className="block w-full rounded-md border border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] px-3 py-2 text-left text-xs" key={`selected-${draft.draft_key}`} onClick={() => toggleBulkDraft(draft.draft_key)} type="button">
                      <div className="flex items-center justify-between gap-2"><span className="font-mono">{draft.model_id_selector || 'defaults'}</span><Check size={14} /></div>
                      <div className="text-[color:var(--cp-muted)]">{draft.match_type}</div>
                    </button>
                  )) : <p className="text-xs text-[color:var(--cp-muted)]">{t('wizard.noEditTargets', 'Select one or more items to edit.')}</p>}
                </ChoicePanel>
              </StepGrid>
              {selectedParamDrafts.length > 0 && selectedParamDraft && (
                <ChoicePanel title={t('wizard.editModelRule', 'Edit selected model rule')} wide>
                  <div className="grid gap-3 md:grid-cols-3">
                    <Field label={t('table.matchType', 'Match')}>
                      <select
                        className={inputClass}
                        disabled={bulkDraftKeys.length > 1}
                        value={String(commonValue(selectedParamDrafts, (draft) => draft.match_type))}
                        onChange={(event) => updateDraftField('match_type', event.target.value as ProviderWizardModelRuleDraft['match_type'])}
                      >
                        <option disabled value="">{t('wizard.mixedValue', 'Mixed values')}</option>
                        <option value="exact">exact</option>
                        <option value="pattern">pattern</option>
                        <option value="default">default</option>
                      </select>
                    </Field>
                    <Field label={t('models.originalProvider', 'Original provider')}>
                      <div className="flex min-h-10 items-center justify-between gap-2 rounded-md border border-dashed border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2)] px-3 text-sm">
                        <span className="truncate font-mono text-[color:var(--cp-muted)]">{selectedParamProviderOwned ? values.provider_key : selectedParamOrigins.join(', ')}</span>
                        {!selectedParamProviderOwned && (
                          <button className="shrink-0 rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold text-[color:var(--cp-text)]" onClick={setSelectedParamProviderOwner} type="button">
                            {t('wizard.setCurrentProvider', 'Set current')}
                          </button>
                        )}
                      </div>
                    </Field>
                    <Field label={t('models.modelDriver', 'Model driver')}>
                      <input
                        className={`${inputClass} font-mono`}
                        placeholder={bulkDraftKeys.length > 1 ? t('wizard.mixedValue', 'Mixed values') : values.provider_driver}
                        value={String(commonValue(selectedParamDrafts, (draft) => draft.model_driver))}
                        onChange={(event) => updateDraftField('model_driver', event.target.value)}
                      />
                    </Field>
                    <Field label={t('table.modelId', 'Model selector')}>
                      <input
                        className={`${inputClass} font-mono`}
                        disabled={selectedParamDraft.match_type === 'default' || bulkDraftKeys.length > 1}
                        placeholder={bulkDraftKeys.length > 1 ? t('wizard.mixedValue', 'Mixed values') : ''}
                        value={selectedParamDraft.match_type === 'default' ? '' : String(commonValue(selectedParamDrafts, (draft) => draft.model_id_selector))}
                        onChange={(event) => updateDraftField('model_id_selector', event.target.value)}
                      />
                    </Field>
                    {selectedParamMissingSelectors.length > 0 && (
                      <div className="rounded-md border border-[color:var(--cp-danger)] px-3 py-2 text-xs text-[color:var(--cp-danger)]">
                        {t('wizard.selectorRequiredEach', 'Model selector must be assigned on each exact or pattern rule.')}
                      </div>
                    )}
                    <Field label={t('models.estimatedCost', 'Estimated cost USD')}>
                      <input className={inputClass} min="0" placeholder={t('wizard.mixedValue', 'Mixed values')} step="0.000001" type="number" value={String(commonValue(selectedParamDrafts, (draft) => draft.estimated_cost_usd))} onChange={(event) => updateDraftField('estimated_cost_usd', event.target.value ? Number(event.target.value) : undefined)} />
                    </Field>
                    <Field label={t('models.estimatedLatency', 'Estimated latency ms')}>
                      <input className={inputClass} min="0" placeholder={t('wizard.mixedValue', 'Mixed values')} step="1" type="number" value={String(commonValue(selectedParamDrafts, (draft) => draft.estimated_latency_ms))} onChange={(event) => updateDraftField('estimated_latency_ms', event.target.value ? Number(event.target.value) : undefined)} />
                    </Field>
                    <Field label={t('models.qualityScore', 'Quality score')}>
                      <input className={inputClass} max="1" min="0" placeholder={t('wizard.mixedValue', 'Mixed values')} step="0.01" type="number" value={String(commonValue(selectedParamDrafts, (draft) => draft.quality_score))} onChange={(event) => updateDraftField('quality_score', event.target.value ? Number(event.target.value) : undefined)} />
                    </Field>
                    <Field label={t('ops.latencyClass', 'Latency class')}>
                      <select className={inputClass} value={String(commonValue(selectedParamDrafts, (draft) => draft.latency_class))} onChange={(event) => updateDraftField('latency_class', event.target.value ? event.target.value as ProviderWizardModelRuleDraft['latency_class'] : undefined)}>
                        <option value="">{t('wizard.mixedValue', 'Mixed values')}</option>
                        <option value="fast">fast</option>
                        <option value="normal">normal</option>
                        <option value="slow">slow</option>
                      </select>
                    </Field>
                    <Field label={t('ops.costClass', 'Cost class')}>
                      <select className={inputClass} value={String(commonValue(selectedParamDrafts, (draft) => draft.cost_class))} onChange={(event) => updateDraftField('cost_class', event.target.value ? event.target.value as ProviderWizardModelRuleDraft['cost_class'] : undefined)}>
                        <option value="">{t('wizard.mixedValue', 'Mixed values')}</option>
                        <option value="low">low</option>
                        <option value="medium">medium</option>
                        <option value="high">high</option>
                      </select>
                    </Field>
                    <label className="flex items-center gap-2 self-end text-sm font-semibold">
                      <input
                        checked={commonValue(selectedParamDrafts, (draft) => draft.exclude) === true}
                        disabled={selectedParamDraft.match_type === 'default'}
                        className="h-4 w-4"
                        onChange={(event) => updateDraftField('exclude', event.target.checked)}
                        type="checkbox"
                      />
                      {t('models.exclude', 'Exclude')}
                    </label>
                  </div>
                  {!selectedParamDraft.exclude && (
                    <div className="mt-3 grid gap-3 lg:grid-cols-3">
                      <TokenPanel title={t('filter.apiType', 'API type')}>
                        {apiTypes.map((apiType) => (
                          <ChoiceButton checked={commonArrayState('api_types', apiType) === 'full'} key={apiType} label={commonArrayState('api_types', apiType) === 'partial' ? `${apiType} (partial)` : apiType} onClick={() => toggleDraftArray('api_types', apiType)} />
                        ))}
                      </TokenPanel>
                      <TokenPanel title={t('filter.capability', 'Capability')}>
                        {groupedCapabilityItems.map((capability) => {
                          const state = commonArrayState('capabilities', capability.key)
                          return (
                            <CapabilityChoice
                              inputPlaceholder={t('wizard.mixedValue', 'Mixed values')}
                              item={capability}
                              key={capability.key}
                              label={state === 'partial' ? `${capability.key} (partial)` : capability.key}
                              onChangeNumber={(value) => updateDraftCapabilityValue(capability.key, value)}
                              onToggle={() => toggleDraftArray('capabilities', capability.key)}
                              selected={state !== 'none'}
                              value={commonValue(selectedParamDrafts.filter((draft) => draft.capabilities.includes(capability.key)), (draft) => getDraftCapabilityValue(draft, capability.key))}
                            />
                          )
                        })}
                      </TokenPanel>
                    </div>
                  )}
                  {selectedParamDraft.exclude && (
                    <p className="mt-3 text-xs text-[color:var(--cp-muted)]">{t('models.excludeKeepsFields', 'Exclude is active; other fields are kept for restore but ignored by publish.')}</p>
                  )}
                  <div className="mt-3 flex justify-end gap-2"><button className="h-9 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" onClick={cancelParamSession} type="button">{t('action.discard', 'Cancel')}</button><button className="h-9 rounded-md bg-[color:var(--cp-accent)] px-3 text-xs font-semibold text-white" onClick={applyParamSession} type="button">{t('action.apply', 'Apply')}</button></div>
                </ChoicePanel>
              )}
            </div>
          )}

          {currentStep === 'mounts' && (
            <div className="space-y-3">
              <h2 className="text-sm font-bold">{t('wizard.logicalMounts', 'Logical mounts')}</h2>
              <StepGrid>
                <ChoicePanel title={t('wizard.origins', 'Providers')}>
                  {mountOrigins.map((origin) => {
                    const count = mountTargets.filter((target) => target.origin === origin).length
                    return count ? <button className={`flex w-full items-center justify-between rounded-md border px-3 py-2 text-left text-xs ${activeMountOrigin === origin ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`} key={origin} onClick={() => setMountOrigin(origin)} type="button"><span className="font-mono">{origin === values.provider_key ? `${origin} (provider)` : origin}</span><span>{count}</span></button> : null
                  })}
                </ChoicePanel>
                <ChoicePanel title={t('wizard.mountTargets', 'Models / patterns / defaults / version rules')}>
                  <div className="mb-2 flex flex-wrap gap-1" role="tablist" aria-label={t('wizard.mountTargets', 'Models / patterns / defaults / version rules')}>
                    {availableMountTabs.map((tab) => {
                      const count = mountTargets.filter((target) => target.origin === activeMountOrigin && target.kind === tab).length
                      return <button className={`rounded-md border px-2 py-1 text-xs font-semibold ${mountTab === tab ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`} key={tab} onClick={() => setMountTab(tab)} type="button">{mountTabLabel(tab)} {count}</button>
                    })}
                  </div>
                  <div className="space-y-2">
                    {visibleMountTargets.map((target) => {
                      const checked = mountTargetKeys.includes(target.key)
                      return (
                        <button className={`flex w-full items-center justify-between gap-2 rounded-md border px-3 py-2 text-left text-xs ${checked ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`} key={target.key} onClick={() => toggleMountTarget(target.key)} type="button">
                          <span className="min-w-0">
                            <StatusBadge tone={target.kind === 'version_rule' || target.kind === 'default' ? 'warning' : 'success'}>{target.kind}</StatusBadge>
                            <span className="ml-2 font-mono">{target.label}</span>
                          </span>
                          {checked && <Check size={14} />}
                        </button>
                      )
                    })}
                  </div>
                </ChoicePanel>
                <ChoicePanel title={t('wizard.editQueue', 'Pending edit list')}>
                  {selectedMountTargets.length ? selectedMountTargets.map((target) => (
                    <button className="block w-full rounded-md border border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] px-3 py-2 text-left text-xs" key={`selected-mount-${target.key}`} onClick={() => toggleMountTarget(target.key)} type="button">
                      <div className="flex items-center justify-between gap-2"><span className="font-mono">{target.label}</span><Check size={14} /></div>
                      <div className="text-[color:var(--cp-muted)]">{target.kind}</div>
                    </button>
                  )) : <p className="text-xs text-[color:var(--cp-muted)]">{t('wizard.noEditTargets', 'Select one or more items to edit.')}</p>}
                </ChoicePanel>
              </StepGrid>
              <DirectoryTreePicker
                directories={logicalDirectories}
                getState={getBulkMountPathState}
                selected={logicalMounts.filter((path) => getBulkMountPathState(path) === 'full')}
                selectedLabel={t('wizard.selectedPaths', 'Selected paths')}
                title={t('wizard.logicalMountHint', 'Full means every selected target will include the path. Partial means only some selected targets currently include it and remains unchanged until clicked.')}
                onToggle={toggleBulkMountPath}
              />
              <div className="flex justify-end gap-2">
                <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold disabled:opacity-40" disabled={!Object.keys(bulkMountPathOverrides).length} onClick={discardBulkMounts} type="button">{t('action.discard', 'Discard')}</button>
                <button className="h-10 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white disabled:opacity-40" disabled={!mountTargetKeys.length} onClick={applyBulkMounts} type="button">{t('logical.applyMounts', 'Apply mounts')}</button>
              </div>
            </div>
          )}

          {currentStep === 'nick' && (
            <div className="space-y-3">
              <section className="rounded-md border border-[color:var(--cp-border)] p-3">
                <h2 className="text-sm font-bold">{t('wizard.nickConcept', 'Nick rewrite role')}</h2>
                <p className="mt-2 text-xs text-[color:var(--cp-muted)]">{t('wizard.nickConceptHint', 'Nick rewrite is a publish-time intermediate mapping. It reuses selected original models, patterns, defaults, variants, and version rules while publishing the provider inventory without copied renamed rules.')}</p>
                <p className="mt-2 text-xs text-[color:var(--cp-muted)]">{t('wizard.nickScopeHint', 'Rules are ordered by priority and also rewrite variants and version rules. Variants use * when no model selector exists; version rules rewrite content.model_pattern.')}</p>
              </section>
              <section className="rounded-md border border-[color:var(--cp-border)] p-2">
                <div className="flex flex-wrap gap-2" role="tablist" aria-label={t('nick.configTabs', 'Nick and origin config tabs')}>
                  {([
                    ['nick', t('nick.title', 'Nick Rules')],
                    ['origin_mappings', t('originMapping.title', 'Origin mappings')],
                    ['origin_provider_aliases', t('originAlias.title', 'Origin provider aliases')],
                  ] as Array<[NickConfigTab, string]>).map(([tab, label]) => (
                    <button className={`h-9 rounded-md border px-3 text-xs font-semibold ${nickConfigTab === tab ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-accent)]' : 'border-[color:var(--cp-border)]'}`} key={tab} onClick={() => setNickConfigTab(tab)} role="tab" type="button">
                      {label}
                    </button>
                  ))}
                </div>
              </section>
              {nickConfigTab === 'nick' && (
                <ChoicePanel title={t('nick.title', 'Nick Rules')} wide>
                  {values.nick_rules.map((rule, index) => (
                    <div className="mb-2 grid gap-2 rounded-md border border-[color:var(--cp-border)] p-3 md:grid-cols-6" key={rule.draft_key}>
                      <select className={inputClass} value={rule.original_provider} onChange={(event) => updateNickRule(index, { original_provider: event.target.value })}>{originalProviders.map((provider) => <option key={provider} value={provider}>{provider}</option>)}</select>
                      <select className={inputClass} value={rule.selector_type} onChange={(event) => updateNickRule(index, { selector_type: event.target.value as 'exact' | 'pattern' })}><option value="pattern">{t('nick.originPrefixRules', 'Origin prefix rules')}</option><option value="exact">{t('nick.exact', 'Exact nick')}</option></select>
                      <input aria-label={t('table.modelId', 'Model selector')} className={`${inputClass} font-mono`} value={rule.model_id} onChange={(event) => updateNickRule(index, { model_id: event.target.value })} />
                      <input aria-label={t('nick.publishedId', 'Published id')} className={`${inputClass} font-mono`} value={rule.nick} onChange={(event) => updateNickRule(index, { nick: event.target.value })} />
                      <input aria-label={t('rules.priority', 'Priority')} className={inputClass} type="number" value={rule.priority} onChange={(event) => updateNickRule(index, { priority: Number(event.target.value) || 1 })} />
                      <button aria-label={t('action.remove', 'Remove')} className="h-10 rounded-md border border-[color:var(--cp-border)]" onClick={() => removeNickRule(index)} type="button"><Trash2 size={16} /></button>
                    </div>
                  ))}
                  <button className="mt-2 inline-flex h-9 items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" onClick={addNickRule} type="button"><Plus size={14} />{t('action.add', 'Add')}</button>
                </ChoicePanel>
              )}
              {nickConfigTab === 'origin_mappings' && (
                <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_420px]">
                  <ChoicePanel title={t('originMapping.title', 'Origin mappings')} wide>
                    {values.origin_mapping_rules.map((rule, index) => (
                      <div className="mb-2 grid gap-2 rounded-md border border-[color:var(--cp-border)] p-3 md:grid-cols-5" key={rule.draft_key}>
                        <select className={inputClass} value={rule.mapping_mode} onChange={(event) => updateOriginMappingRule(index, { mapping_mode: event.target.value as 'template' | 'regex' })}><option value="template">{t('originMapping.templateMode', 'Template')}</option><option value="regex">{t('originMapping.regexMode', 'Regex')}</option></select>
                        <input aria-label={t('originMapping.matchPattern', 'Match pattern')} className={`${inputClass} font-mono`} value={rule.match_pattern} onChange={(event) => updateOriginMappingRule(index, { match_pattern: event.target.value })} />
                        {rule.mapping_mode === 'template' ? (
                          <input aria-label={t('originMapping.originTemplate', 'Origin template')} className={`${inputClass} font-mono`} value={rule.origin_template} onChange={(event) => updateOriginMappingRule(index, { origin_template: event.target.value })} />
                        ) : (
                          <input aria-label={t('originMapping.regex', 'Regex')} className={`${inputClass} font-mono`} value={rule.regex} onChange={(event) => updateOriginMappingRule(index, { regex: event.target.value })} />
                        )}
                        <input aria-label={t('rules.priority', 'Priority')} className={inputClass} type="number" value={rule.priority} onChange={(event) => updateOriginMappingRule(index, { priority: Number(event.target.value) || 1 })} />
                        <button aria-label={t('action.remove', 'Remove')} className="h-10 rounded-md border border-[color:var(--cp-border)]" onClick={() => removeOriginMappingRule(index)} type="button"><Trash2 size={16} /></button>
                        <div className="grid gap-2 md:col-span-5 md:grid-cols-2">
                          <TransformToggles
                            label={t('originMapping.driverTransforms', 'Driver transforms')}
                            options={['trim', 'lowercase', 'alias']}
                            values={rule.driver_transforms}
                            onToggle={(op) => toggleOriginMappingDriverTransform(index, op as 'trim' | 'lowercase' | 'alias')}
                          />
                          <TransformToggles
                            label={t('originMapping.modelTransforms', 'Model transforms')}
                            options={['trim', 'lowercase']}
                            values={rule.model_transforms}
                            onToggle={(op) => toggleOriginMappingModelTransform(index, op as 'trim' | 'lowercase')}
                          />
                        </div>
                      </div>
                    ))}
                    <button className="mt-2 inline-flex h-9 items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" onClick={addOriginMappingRule} type="button"><Plus size={14} />{t('action.add', 'Add')}</button>
                  </ChoicePanel>
                  <section className="rounded-md border border-[color:var(--cp-border)] p-3">
                    <h3 className="mb-2 text-xs font-semibold text-[color:var(--cp-muted)]">{t('originMapping.publishedJson', 'Published origin_mappings JSON')}</h3>
                    <JsonViewer value={originMappingsJsonPreview} filename={`${values.provider_key || 'provider'}-origin-mappings.json`} />
                  </section>
                </div>
              )}
              {nickConfigTab === 'origin_provider_aliases' && (
                <ChoicePanel title={t('originAlias.title', 'Origin provider aliases')} wide>
                  {values.origin_provider_aliases.map((alias, index) => (
                    <div className="mb-2 grid gap-2 rounded-md border border-[color:var(--cp-border)] p-3 md:grid-cols-[1fr_1fr_auto]" key={alias.draft_key}>
                      <input aria-label={t('originAlias.alias', 'Provider alias')} className={`${inputClass} font-mono`} value={alias.alias} onChange={(event) => updateOriginAlias(index, { alias: event.target.value })} />
                      <input aria-label={t('originAlias.driver', 'Origin driver')} className={`${inputClass} font-mono`} value={alias.driver} onChange={(event) => updateOriginAlias(index, { driver: event.target.value })} />
                      <button aria-label={t('action.remove', 'Remove')} className="h-10 rounded-md border border-[color:var(--cp-border)]" onClick={() => removeOriginAlias(index)} type="button"><Trash2 size={16} /></button>
                    </div>
                  ))}
                  <button className="mt-2 inline-flex h-9 items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" onClick={addOriginAlias} type="button"><Plus size={14} />{t('action.add', 'Add')}</button>
                </ChoicePanel>
              )}
              <section className="rounded-md border border-[color:var(--cp-border)] p-3">
                <h2 className="mb-3 text-sm font-bold">{t('nick.preview', 'Rewrite preview')}</h2>
                <div className="space-y-3">
                  <div className="shell-scrollbar flex gap-2 overflow-auto pb-1">
                    {nickRewritePreviewSections.map((section) => {
                      const active = section.target === activeNickRewritePreviewSection?.target
                      return (
                        <button className={`inline-flex h-9 shrink-0 items-center gap-2 rounded-md border px-3 text-xs font-semibold ${active ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-accent)]' : 'border-[color:var(--cp-border)]'}`} key={section.target} onClick={() => setNickPreviewTarget(section.target)} type="button">
                          <span>{section.label}</span>
                          <StatusBadge tone={nickRewritePreviewTone(section.target)}>{section.items.length}</StatusBadge>
                        </button>
                      )
                    })}
                  </div>
                  {activeNickRewritePreviewSection && (
                    <div className="shell-scrollbar grid max-h-72 gap-2 overflow-auto md:grid-cols-2 xl:grid-cols-3">
                      {activeNickRewritePreviewSection.items.map((item, index) => (
                        <div className="rounded-md border border-[color:var(--cp-border)] p-2 text-xs" key={`${activeNickRewritePreviewSection.target}-${item.source_model_id}-${item.published_id}-${index}`}>
                          <StatusBadge tone={nickRewritePreviewTone(item.target)}>{item.target}</StatusBadge>
                          <div className="mt-2 grid gap-1">
                            <PreviewLine label={t('models.originalProvider', 'Original provider')} value={item.original_provider ?? '-'} />
                            <PreviewLine label={t('table.modelId', 'Model selector')} value={item.source_model_id} />
                            <PreviewLine label={t('nick.publishedId', 'Published id')} value={item.published_id} />
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </section>
            </div>
          )}

          {currentStep === 'preview' && (
            <StepGrid>
              {submitMessage && (
                <ChoicePanel title={submitState === 'saving' ? t('status.loading', 'Loading') : submitState === 'error' ? t('status.error', 'Error') : t('status.warning', 'Warning')} wide>
                  <div className="flex items-start gap-3 text-sm">
                    <StatusBadge tone={submitState === 'saving' ? 'accent' : submitState === 'error' ? 'danger' : 'warning'}>{submitState}</StatusBadge>
                    <div className="text-[color:var(--cp-muted)]">{submitMessage}</div>
                  </div>
                </ChoicePanel>
              )}
              <ChoicePanel title={t('publish.risks', 'Risk checks')} wide>
                <div className="grid gap-2 md:grid-cols-2">
                  {wizardDiagnostics.map((diagnostic) => (
                    <div className="rounded-md border border-[color:var(--cp-border)] p-3 text-sm" key={diagnostic.key}>
                      <StatusBadge tone={diagnostic.tone}>{diagnostic.label}</StatusBadge>
                      <div className="mt-2 text-[color:var(--cp-muted)]">{diagnostic.detail}</div>
                    </div>
                  ))}
                </div>
              </ChoicePanel>
              <ChoicePanel title={t('publish.json', 'Published JSON snippet')} wide>
                <JsonViewer
                  value={clientDriverMetadataPreview}
                  filename={`${values.provider_key || 'provider'}-wizard-preview.json`}
                />
              </ChoicePanel>
            </StepGrid>
          )}

          <div className="mt-5 flex flex-wrap items-center justify-between gap-2 border-t border-[color:var(--cp-border)] pt-4">
            <button className="inline-flex h-10 items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" disabled={stepIndex === 0} onClick={() => setStepIndex((index) => Math.max(0, index - 1))} type="button">
              <ArrowLeft size={16} />
              {t('wizard.back', 'Back')}
            </button>
            <div className="flex items-center gap-2">
              <button className="inline-flex h-10 items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" onClick={() => {
                setInspector({
                  title: values.name,
                  subtitle: values.provider_key,
                  status: currentStep,
                  json: { ...values, preview: publishedPreview, resolver_preview: resolverPublishedPreview, diagnostics: wizardDiagnostics },
                })
              }} type="button">
                <WandSparkles size={16} />
                {t('wizard.inspect', 'Inspect')}
              </button>
              {stepIndex < steps.length - 1 ? (
                <button className="inline-flex h-10 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={nextStep} type="button">
                  {t('wizard.next', 'Next')}
                  <ArrowRight size={16} />
                </button>
              ) : (
                <button className="inline-flex h-10 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white disabled:cursor-not-allowed disabled:opacity-50" disabled={submitState === 'saving'} type="submit">
                  <GitCompare size={16} />
                  {submitState === 'saving' ? t('wizard.saving', 'Saving...') : t('wizard.complete', 'Save and preview')}
                </button>
              )}
            </div>
          </div>
        </form>
      </div>
    </div>
  )
}

function buildPublishedId(input: ProviderWizardInput, sourceModelId: string, originalProvider?: string | null) {
  const rule = input.nick_rules
    .filter((item) => item.original_provider === originalProvider)
    .sort((a, b) => a.priority - b.priority)
    .find((item) => item.selector_type === 'exact' ? item.model_id === sourceModelId : wildcardMatch(item.model_id, sourceModelId))
  return rule ? applyModelTemplate(rule.nick, sourceModelId) : sourceModelId
}

function getResolverRewriteSelector(draft: ProviderWizardResolverRuleDraft) {
  if (draft.rule_kind === 'version_rule') {
    return draft.model_pattern?.trim() || draft.model_id_selector || '*'
  }
  return draft.model_id_selector || '*'
}

function previewTargetFromMatchType(matchType: ProviderWizardModelRuleDraft['match_type']): NickRewritePreviewTarget {
  if (matchType === 'exact') {
    return 'model'
  }
  return matchType
}

function nickRewritePreviewTone(target: NickRewritePreviewTarget) {
  if (target === 'model') {
    return 'accent'
  }
  if (target === 'pattern' || target === 'variant') {
    return 'success'
  }
  return 'warning'
}

function wildcardMatch(pattern: string, value: string) {
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*')
  return new RegExp(`^${escaped}$`).test(value)
}

function describeResolverSource(key: string, versionRules: MetadataVersionRuleRecord[], variants: MetadataVariantRecord[]) {
  if (key.startsWith('version_rule:')) {
    const id = key.replace('version_rule:', '')
    const rule = versionRules.find((item) => item.version_rule_key === id)
    const content = rule?.content ?? {}
    const includeTier = Array.isArray(content.tier_tokens) ? `include ${content.tier_tokens.join('|')}` : ''
    const excludeTier = Array.isArray(content.exclude_tier_tokens) ? `exclude ${content.exclude_tier_tokens.join('|')}` : ''
    const unstable = typeof content.stability === 'object' && content.stability && 'unstable_tokens' in content.stability && Array.isArray(content.stability.unstable_tokens) ? `unstable ${content.stability.unstable_tokens.join('|')}` : ''
    return [rule?.nick, rule?.model_id_selector, includeTier, excludeTier, unstable].filter(Boolean).join(' · ')
  }
  const id = key.replace('variant:', '')
  const rule = variants.find((item) => item.variant_key === id)
  const suffix = typeof rule?.content.mount_suffix === 'string' ? `mount ${rule.content.mount_suffix}` : ''
  return [rule?.nick, rule?.model_id_selector || '*', suffix].filter(Boolean).join(' · ')
}

function compactRecord<T extends Record<string, unknown>>(record: T) {
  return Object.fromEntries(Object.entries(record).filter(([, value]) => {
    if (value === undefined || value === null || value === '') return false
    if (Array.isArray(value) && value.length === 0) return false
    if (typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length === 0) return false
    return true
  }))
}

function getRawSourceRule(rule?: ModelParamRuleRecord) {
  const raw = rule?.attributes?.source_rule
  return typeof raw === 'object' && raw !== null && !Array.isArray(raw) ? raw as DriverMetadataRule : null
}

function getNumericAttribute(rule: ModelParamRuleRecord, key: string) {
  const rawValue = getRawSourceRule(rule)?.[key]
  if (typeof rawValue === 'number') return rawValue
  const value = rule.attributes?.[key]
  return typeof value === 'number' ? value : undefined
}

function getClassAttribute<T extends string>(rule: ModelParamRuleRecord, key: string, values: readonly T[]) {
  const rawValue = getRawSourceRule(rule)?.[key]
  if (typeof rawValue === 'string' && values.includes(rawValue as T)) return rawValue as T
  const value = rule.attributes?.[key]
  return typeof value === 'string' && values.includes(value as T) ? value as T : undefined
}

function getSourceEstimatedCost(rule: ModelParamRuleRecord) {
  const rawCost = getRawSourceRule(rule)?.estimated_cost_usd
  if (typeof rawCost === 'number') return rawCost
  return typeof rule.pricing?.estimated_cost_usd === 'number' ? rule.pricing.estimated_cost_usd : undefined
}

function getSourceEstimatedLatency(rule: ModelParamRuleRecord) {
  const rawLatency = getRawSourceRule(rule)?.estimated_latency_ms
  if (typeof rawLatency === 'number') return rawLatency
  return typeof rule.pricing?.estimated_latency_ms === 'number' ? rule.pricing.estimated_latency_ms : undefined
}

function sameStringSet(left: string[], right: string[]) {
  if (left.length !== right.length) return false
  const rightSet = new Set(right)
  return left.every((item) => rightSet.has(item))
}

function readCapabilityValues(capabilities: Record<string, unknown>) {
  const values: Record<string, boolean | number> = {}
  Object.entries(capabilities).forEach(([key, value]) => {
    if (typeof value === 'boolean' || typeof value === 'number') {
      values[key] = value
    }
  })
  return values
}

function buildResolverCapabilities(draft: ProviderWizardResolverRuleDraft) {
  const capabilityValues = draft.capability_values ?? {}
  return Object.fromEntries((draft.capabilities ?? []).map((capability) => {
    const value = capabilityValues[capability]
    return [capability, typeof value === 'boolean' || typeof value === 'number' ? value : true]
  }))
}

function getDraftCapabilityValue(draft: ProviderWizardModelRuleDraft, capability: string) {
  return draft.capability_values?.[capability]
}

function getDefaultCapabilityNumber(_capability: string) {
  return 1
}

function getDefaultCapabilityValues(capabilities: string[], dictionaries: DictionaryItem[]) {
  return Object.fromEntries(capabilities.map((capability) => {
    const dictionary = dictionaries.find((item) => item.kind === 'capability' && item.key === capability)
    return [capability, dictionary?.value_type === 'number' ? getDefaultCapabilityNumber(capability) : true]
  }))
}

function buildWizardCapabilities(source: ModelParamRuleRecord | undefined, draft: ProviderWizardModelRuleDraft) {
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
  })) as Record<string, boolean | number | string>
}

function findSourceRule(seedRules: ModelParamRuleRecord[], draft: ProviderWizardModelRuleDraft) {
  if (draft.source_rule_key) {
    return seedRules.find((rule) => rule.rule_key === draft.source_rule_key)
  }
  return seedRules.find((rule) => rule.match_type === draft.match_type && rule.model_id_selector === draft.model_id_selector)
}

function materializeWizardModelRule(seedRules: ModelParamRuleRecord[], input: ProviderWizardInput, draft: ProviderWizardModelRuleDraft): DriverMetadataRule {
  const source = findSourceRule(seedRules, draft)
  const base: DriverMetadataRule = { ...(getRawSourceRule(source) ?? {}) }
  const selector = draft.match_type === 'default' ? undefined : buildPublishedId(input, draft.model_id_selector, draft.original_provider)
  if (draft.match_type === 'exact') {
    base.id = selector
    delete base.pattern
  } else if (draft.match_type === 'pattern') {
    base.pattern = selector
    delete base.id
  } else {
    delete base.id
    delete base.pattern
  }
  if (draft.exclude && draft.match_type !== 'default') {
    return compactRecord({
      id: draft.match_type === 'exact' ? selector : undefined,
      pattern: draft.match_type === 'pattern' ? selector : undefined,
      exclude: true,
    }) as DriverMetadataRule
  }
  base.api_types = draft.api_types
  if (draft.model_driver && draft.model_driver !== input.provider_driver) {
    base.model_driver = draft.model_driver
  } else {
    delete base.model_driver
  }
  base.logical_mounts = draft.logical_mounts
  base.capabilities = buildWizardCapabilities(source, draft)
  base.estimated_cost_usd = draft.estimated_cost_usd
  base.estimated_latency_ms = draft.estimated_latency_ms
  base.quality_score = draft.quality_score
  base.latency_class = draft.latency_class
  base.cost_class = draft.cost_class
  return compactRecord(base) as DriverMetadataRule
}

function parseJsonRecord(text?: string) {
  if (!text?.trim()) return {}
  try {
    return readRecord(JSON.parse(text))
  } catch {
    return {}
  }
}

function materializeWizardVariant(draft: ProviderWizardResolverRuleDraft) {
  return compactRecord({
    name: draft.nick || undefined,
    mount_suffix: draft.mount_suffix || draft.nick || undefined,
    provider_options: parseJsonRecord(draft.provider_options_json),
  })
}

function materializeWizardVersionRule(input: ProviderWizardInput, draft: ProviderWizardResolverRuleDraft) {
  const modelPattern = draft.model_pattern || draft.model_id_selector || '*'
  return compactRecord({
    family: draft.family || draft.original_provider,
    tier: draft.tier || draft.nick || 'standard',
    model_pattern: buildPublishedId(input, modelPattern, draft.original_provider),
    tier_tokens: draft.tier_tokens ?? [],
    exclude_tier_tokens: draft.exclude_tier_tokens ?? [],
    version_rank: compactRecord({ prefix: draft.version_rank_prefix }),
    stability: compactRecord({
      unstable_tokens: draft.stability_unstable_tokens ?? [],
      current_requires_stable: draft.stability_current_requires_stable,
    }),
    current_mount: draft.current_mount,
    version_mount: draft.version_mount,
    auto_mounts: draft.logical_mounts,
    exclude_snapshot_date_suffix: draft.exclude_snapshot_date_suffix,
    capabilities: buildResolverCapabilities(draft),
  })
}

function buildWizardOriginMappings(input: ProviderWizardInput) {
  return input.origin_mapping_rules
    .sort((a, b) => a.priority - b.priority)
    .map((rule) => {
      return {
        mapping_key: `${input.provider_key}-origin-${safeKeyPart(rule.draft_key)}`,
        priority: rule.priority,
        match: {
          source: 'provider_model_id' as const,
          regex: rule.mapping_mode === 'regex'
            ? rule.regex
            : originTemplateToRegex(rule.origin_template || rule.match_pattern, input.provider_driver),
        },
        transforms: {
          driver: rule.driver_transforms.map((op) => op === 'alias'
            ? { op, table: 'origin_provider_aliases', on_missing: 'keep' as const }
            : { op }),
          model: rule.model_transforms.map((op) => ({ op })),
        },
      }
    })
}

function originTemplateToRegex(template: string, fallbackDriver: string) {
  const escapedDriver = escapeRegex(fallbackDriver)
  let regex = escapeRegex(template)
    .replace(/<driver>/g, '(?<driver>[^/]+)')
    .replace(/<model>/g, '(?<model>.+)')
    .replace(/\\\{driver\\\}/g, '(?<driver>[^/]+)')
    .replace(/\\\{model\\\}/g, '(?<model>.+)')
  if (!regex.includes('(?<driver>')) {
    const modelIndex = regex.indexOf('(?<model>')
    const slashIndex = regex.indexOf('/')
    if (slashIndex > 0 && (modelIndex === -1 || slashIndex < modelIndex)) {
      const prefix = regex.slice(0, slashIndex)
      regex = `(?<driver>${prefix})/${regex.slice(slashIndex + 1)}`
    } else if (regex.startsWith(`${escapedDriver}/`)) {
      regex = `(?<driver>${escapedDriver})/${regex.slice(`${escapedDriver}/`.length)}`
    } else {
      regex = `(?<driver>${escapedDriver})/${regex}`
    }
  }
  return `^${regex}$`
}

function escapeRegex(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

function applyModelTemplate(template: string, modelId: string) {
  return template.replace(/<model>|\{model\}/g, modelId)
}

function safeKeyPart(value: string) {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'rule'
}

function buildWizardClientDriverMetadata(seedRules: ModelParamRuleRecord[], input: ProviderWizardInput): DriverMetadataDocument {
  const rules = input.model_rule_drafts.map((draft) => ({
    draft,
    rule: materializeWizardModelRule(seedRules, input, draft),
  }))
  const defaultRule = rules.find(({ draft }) => draft.match_type === 'default')
  return {
    schema_version: 2,
    provider_driver: input.provider_driver,
    name: input.name || null,
    protocol_family: input.protocol_family,
    base_url: input.base_url || null,
    revision: 'draft',
    origin_provider_aliases: buildWizardOriginProviderAliases(input),
    origin_mappings: buildWizardOriginMappings(input),
    models: rules.filter(({ draft }) => draft.match_type === 'exact').map(({ rule }) => rule),
    patterns: rules
      .filter(({ draft }) => draft.match_type === 'pattern')
      .sort((left, right) => left.draft.priority - right.draft.priority)
      .map(({ rule }) => rule),
    defaults: defaultRule?.rule ?? {},
    variants: input.resolver_rule_drafts.filter((draft) => draft.rule_kind === 'variant').map(materializeWizardVariant),
    version_rules: input.resolver_rule_drafts.filter((draft) => draft.rule_kind === 'version_rule').map((draft) => materializeWizardVersionRule(input, draft)),
    signature: null,
  }
}

function buildWizardOriginProviderAliases(input: ProviderWizardInput) {
  const aliases = Object.fromEntries(input.origin_provider_aliases.map((alias) => [alias.alias, alias.driver]))
  return Object.keys(aliases).length ? aliases : undefined
}

function buildWizardRuleDrafts(_seedRules: ModelParamRuleRecord[], input: ProviderWizardInput, selectedSourceModels: ModelParamRuleRecord[], capabilityDictionaries: DictionaryItem[]) {
  const existing = new Map(input.model_rule_drafts.map((draft) => [draft.draft_key, draft]))
  const defaults = {
    api_types: input.selected_api_types,
    capabilities: input.selected_capabilities,
    capability_values: getDefaultCapabilityValues(input.selected_capabilities, capabilityDictionaries),
    logical_mounts: input.selected_logical_mounts,
  }
  const sourcedDrafts = selectedSourceModels.map((source, index): ProviderWizardModelRuleDraft => {
    const modelId = source.model_id_selector ?? `model-${index + 1}`
    const draftKey = `source-${source.rule_key}`
    const old = existing.get(draftKey)
    return old ?? {
      draft_key: draftKey,
      match_type: source.match_type,
      original_provider: source.original_provider ?? input.selected_origins[0] ?? '',
      model_id_selector: modelId,
      priority: index + 1,
      source_rule_key: source.rule_key,
      model_driver: source.model_driver ?? input.provider_driver,
      api_types: source.api_types.length ? source.api_types : defaults.api_types,
      capabilities: Object.keys(source.capabilities).length ? Object.keys(source.capabilities) : defaults.capabilities,
      capability_values: Object.keys(source.capabilities).length ? readCapabilityValues(source.capabilities) : defaults.capability_values,
      estimated_cost_usd: getSourceEstimatedCost(source),
      estimated_latency_ms: getSourceEstimatedLatency(source),
      quality_score: getNumericAttribute(source, 'quality_score'),
      latency_class: getClassAttribute(source, 'latency_class', ['fast', 'normal', 'slow']),
      cost_class: getClassAttribute(source, 'cost_class', ['low', 'medium', 'high']),
      logical_mounts: source.logical_mounts.length ? source.logical_mounts : defaults.logical_mounts,
      exclude: false,
    }
  })
  const manualDrafts = input.model_rule_drafts.filter((draft) => draft.source_rule_key === '')
  return [...sourcedDrafts, ...manualDrafts]
}

function buildWizardResolverDrafts(input: ProviderWizardInput, sources: Array<{ key: string; rule_kind: ProviderWizardResolverRuleDraft['rule_kind']; original_provider: string; model_id_selector: string; selector_type: ProviderWizardResolverRuleDraft['selector_type']; priority: number; nick: string; mount_suffix?: string; provider_options_json?: string; family?: string; tier?: string; model_pattern?: string; tier_tokens?: string[]; exclude_tier_tokens?: string[]; version_rank_prefix?: string; stability_unstable_tokens?: string[]; stability_current_requires_stable?: boolean; current_mount?: string; version_mount?: string; exclude_snapshot_date_suffix?: boolean; capabilities?: string[]; capability_values?: Record<string, boolean | number>; logical_mounts: string[] }>) {
  const existing = new Map(input.resolver_rule_drafts.map((draft) => [draft.draft_key, draft]))
  const selected = sources.filter((source) => input.selected_resolver_rule_keys.includes(source.key))
  const copied = selected.map((source): ProviderWizardResolverRuleDraft => existing.get(`source-${source.key}`) ?? ({
    draft_key: `source-${source.key}`,
    rule_kind: source.rule_kind,
    selector_type: source.selector_type,
    original_provider: source.original_provider,
    model_id_selector: source.model_id_selector || '*',
    priority: source.priority,
    nick: source.nick,
    mount_suffix: source.mount_suffix,
    provider_options_json: source.provider_options_json,
    family: source.family,
    tier: source.tier,
    model_pattern: source.model_pattern,
    tier_tokens: source.tier_tokens,
    exclude_tier_tokens: source.exclude_tier_tokens,
    version_rank_prefix: source.version_rank_prefix,
    stability_unstable_tokens: source.stability_unstable_tokens,
    stability_current_requires_stable: source.stability_current_requires_stable,
    current_mount: source.current_mount,
    version_mount: source.version_mount,
    exclude_snapshot_date_suffix: source.exclude_snapshot_date_suffix,
    capabilities: source.capabilities,
    capability_values: source.capability_values,
    logical_mounts: source.logical_mounts.length ? source.logical_mounts : input.selected_logical_mounts,
    source_rule_key: source.key,
  }))
  return [...copied, ...input.resolver_rule_drafts.filter((draft) => !draft.source_rule_key)]
}

function buildWizardDiagnostics(seedRules: ModelParamRuleRecord[], input: ProviderWizardInput) {
  const sourceKeys = new Set(seedRules.map((rule) => rule.rule_key))
  const publishedIds = input.model_rule_drafts.filter((draft) => draft.match_type !== 'default').map((draft) => buildPublishedId(input, draft.model_id_selector, draft.original_provider))
  const duplicatePublishedIds = publishedIds.filter((id, index) => publishedIds.indexOf(id) !== index)
  return [
    {
      key: 'selected-models',
      tone: input.selected_model_ids.length ? 'success' : 'danger',
      label: input.selected_model_ids.length ? 'model ok' : 'missing model',
      detail: `${input.selected_model_ids.length} selected source models`,
    },
    {
      key: 'origin-meta',
      tone: input.selected_model_ids.every((ruleKey) => sourceKeys.has(ruleKey)) ? 'success' : 'warning',
      label: 'origin meta',
      detail: input.selected_model_ids.every((ruleKey) => sourceKeys.has(ruleKey)) ? 'All selected rules have source metadata' : 'At least one selected rule is missing source metadata',
    },
    {
      key: 'published-id',
      tone: duplicatePublishedIds.length ? 'warning' : 'success',
      label: 'provider selector',
      detail: duplicatePublishedIds.length ? `Duplicate provider selector: ${duplicatePublishedIds.join(', ')}` : 'No duplicate provider selector in wizard preview',
    },
    {
      key: 'key-risk',
      tone: 'warning',
      label: 'key fields',
      detail: `${input.provider_key}, nick rules, model rules, variants, and version rules will be created`,
    },
  ] as Array<{ key: string; tone: 'success' | 'warning' | 'danger'; label: string; detail: string }>
}

function StepGrid({ children }: { children: ReactNode }) {
  return <div className="grid gap-3 lg:grid-cols-3">{children}</div>
}

function Field({ label, error, children }: { label: string; error?: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
      {label}
      {children}
      {error && <span className="text-[color:var(--cp-danger)]">{error}</span>}
    </label>
  )
}

function ChoicePanel({ title, wide, children }: { title: string; wide?: boolean; children: ReactNode }) {
  return (
    <section className={`rounded-md border border-[color:var(--cp-border)] p-3 ${wide ? 'lg:col-span-2' : ''}`}>
      <h2 className="mb-3 text-sm font-bold">{title}</h2>
      <div className="space-y-2">{children}</div>
    </section>
  )
}

function TransformToggles({ label, options, values, onToggle }: { label: string; options: string[]; values: string[]; onToggle: (op: string) => void }) {
  return (
    <div className="rounded-md border border-[color:var(--cp-border)] p-2">
      <div className="mb-2 text-xs font-semibold text-[color:var(--cp-muted)]">{label}</div>
      <div className="flex flex-wrap gap-2">
        {options.map((option) => (
          <button className={`rounded-md border px-2 py-1 text-xs font-semibold ${values.includes(option) ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-accent)]' : 'border-[color:var(--cp-border)]'}`} key={option} onClick={() => onToggle(option)} type="button">
            {option}
          </button>
        ))}
      </div>
    </div>
  )
}

function PreviewLine({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-[10px] uppercase text-[color:var(--cp-muted)]">{label}</div>
      <div className="break-all font-mono">{value}</div>
    </div>
  )
}

function TokenPanel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-md border border-[color:var(--cp-border)] p-3">
      <h3 className="mb-2 text-xs font-semibold text-[color:var(--cp-muted)]">{title}</h3>
      <div className="shell-scrollbar flex max-h-40 flex-wrap gap-2 overflow-auto">{children}</div>
    </section>
  )
}

function MountPathPicker({ clearLabel, directories, editLabel, emptyLabel, label, value, onChange }: { clearLabel: string; directories: LogicalDirectoryRecord[]; editLabel: string; emptyLabel: string; label: string; value: string; onChange: (mount: string) => void }) {
  const [editing, setEditing] = useState(false)
  const selectedPath = mountToDirectoryPath(value)
  return (
    <section className="rounded-md border border-[color:var(--cp-border)] p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold text-[color:var(--cp-muted)]">{label}</h3>
        <div className="flex items-center gap-2">
          <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => setEditing((current) => !current)} type="button">
            {editLabel}
          </button>
          <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold disabled:opacity-40" disabled={!value} onClick={() => onChange('')} type="button">
            {clearLabel}
          </button>
        </div>
      </div>
      <div className="mb-2 min-h-7 rounded-md bg-[color:var(--cp-surface-2)] px-2 py-1 font-mono text-xs text-[color:var(--cp-muted)]">
        {value || emptyLabel}
      </div>
      {editing && (
        <div className="shell-scrollbar max-h-52 space-y-1 overflow-auto">
          {directories.map((directory) => {
            const checked = selectedPath === directory.path
            return (
              <button className="flex w-full cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-[color:var(--cp-surface-2)]" key={`${label}-${directory.directory_key}`} onClick={() => onChange(directoryPathToMount(directory.path))} style={{ paddingLeft: `${8 + directory.path.split('/').filter(Boolean).length * 12}px` }} type="button">
                <span className={`grid h-4 w-4 place-items-center rounded-full border text-[10px] ${checked ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent)] text-white' : 'border-[color:var(--cp-border)]'}`}>{checked ? <Check size={12} /> : ''}</span>
                <span className="font-mono">{directory.path}</span>
              </button>
            )
          })}
        </div>
      )}
    </section>
  )
}

function DirectoryTreePicker({ directories, selected, selectedLabel, title, getState, onToggle }: { directories: LogicalDirectoryRecord[]; selected: string[]; selectedLabel: string; title: string; getState: (path: string) => 'full' | 'partial' | 'none'; onToggle: (path: string) => void }) {
  return (
    <section className="rounded-md border border-[color:var(--cp-border)] p-3">
      <h3 className="mb-2 text-xs font-semibold text-[color:var(--cp-muted)]">{title}</h3>
      <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_260px]">
        <div className="shell-scrollbar max-h-80 space-y-1 overflow-auto rounded-md border border-[color:var(--cp-border)] p-2">
          {directories.map((directory) => {
            const state = getState(directory.path)
            return (
              <button className="flex w-full cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-[color:var(--cp-surface-2)]" key={directory.directory_key} onClick={() => onToggle(directory.path)} style={{ paddingLeft: `${8 + directory.path.split('/').filter(Boolean).length * 12}px` }} type="button">
                <span className={`grid h-4 w-4 place-items-center rounded border text-[10px] ${state === 'full' ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent)] text-white' : state === 'partial' ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-accent)]' : 'border-[color:var(--cp-border)]'}`}>{state === 'full' ? <Check size={12} /> : state === 'partial' ? '-' : ''}</span>
                <span className="font-mono">{directory.path}</span>
              </button>
            )
          })}
        </div>
        <div className="rounded-md border border-[color:var(--cp-border)] p-2 text-xs">
          <div className="mb-2 font-semibold text-[color:var(--cp-muted)]">{selectedLabel} ({selected.length})</div>
          <div className="shell-scrollbar max-h-72 space-y-1 overflow-auto">{selected.map((path) => <button className="block w-full rounded border border-[color:var(--cp-border)] px-2 py-1 text-left font-mono hover:border-[color:var(--cp-accent)]" key={path} onClick={() => onToggle(path)} type="button">{path}</button>)}</div>
        </div>
      </div>
    </section>
  )
}

function ChoiceButton({ checked, label, onClick }: { checked: boolean; label: string; onClick: () => void }) {
  return (
    <button className={`min-h-8 rounded-md border px-2 py-1.5 font-mono text-xs ${checked ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`} onClick={onClick} type="button">
      {label}
    </button>
  )
}

function CapabilityChoice({ inputPlaceholder, item, label, onChangeNumber, onToggle, selected, value }: { inputPlaceholder: string; item: DictionaryItem; label: string; onChangeNumber: (value: string) => void; onToggle: () => void; selected: boolean; value: string | number | boolean | undefined }) {
  return (
    <div className={`flex min-w-44 flex-col gap-1 rounded-md border p-2 ${selected ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`}>
      <button className="min-h-8 rounded-md border border-transparent px-2 py-1.5 text-left font-mono text-xs" onClick={onToggle} type="button">
        {label}
      </button>
      {item.value_type !== 'boolean' && selected && (
        <input
          aria-label={item.label}
          className={inputClass}
          min="0"
          placeholder={inputPlaceholder}
          type="number"
          value={typeof value === 'number' ? value : ''}
          onChange={(event) => onChangeNumber(event.target.value)}
        />
      )}
    </div>
  )
}

function groupCapabilityItems(items: DictionaryItem[]) {
  return [...items].sort((left, right) => {
    if (left.value_type === right.value_type) {
      return 0
    }
    return left.value_type === 'boolean' ? -1 : 1
  })
}

function CheckRow({ checked, label, meta, onClick }: { checked: boolean; label: string; meta?: string; onClick: () => void }) {
  return (
    <button className={`flex min-h-10 w-full items-center justify-between rounded-md border px-3 py-2 text-left text-sm ${checked ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`} onClick={onClick} type="button">
      <span>
        <span className="font-mono text-xs">{label}</span>
        {meta && <span className="ml-2 text-xs text-[color:var(--cp-muted)]">{meta}</span>}
      </span>
      {checked && <Check size={16} />}
    </button>
  )
}

import { useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { Check, Pencil, Plus, Trash2 } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { LogicalMountTreePicker } from '../../components/forms/LogicalMountTreePicker'
import { StatusBadge } from '../../components/status/StatusBadge'
import {
  getDictionaryKeys,
  getOpsOverlay,
  getOriginalProviders,
  getMaterializedLogicalDirectories,
  previewVariantHits,
} from '../../datamodel/selectors'
import {
  metadataVariantInputSchema,
  metadataVersionRuleInputSchema,
  type MetadataVariantInput,
  type MetadataVersionRuleInput,
} from '../../datamodel/schemas'
import type { DictionaryItem, LogicalDirectoryRecord, MetadataVariantRecord, MetadataVersionRuleRecord, OpsOverlayRecord } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { useShellContext } from '../pageUtils'

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm text-[color:var(--cp-text)]'

export function ResolverRulesPage() {
  const { serviceRole } = useProviderMetadataStore()
  return serviceRole === 'ops' ? <OpsResolverOverlayPage /> : <TechResolverRulesPage />
}

function TechResolverRulesPage() {
  const { t } = useI18n()
  const {
    workspace,
    viewMode,
    upsertMetadataVariant,
    upsertMetadataVersionRule,
    removeMetadataVariant,
    removeMetadataVersionRule,
  } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [activeResolverTab, setActiveResolverTab] = useState<'variant' | 'version_rule'>('variant')
  const [providerFilter, setProviderFilter] = useState('')
  const selectedProvider = data.providers[0]
  const providers = data.providers.map((provider) => ({ key: provider.provider_key, label: provider.name }))
  const originalProviders = useMemo(() => getOriginalProviders(data), [data])
  const defaultOriginalProvider = originalProviders[0] ?? 'openai'
  const [editingVariant, setEditingVariant] = useState(false)
  const [editingVersionRule, setEditingVersionRule] = useState(false)
  const [variantFormOpen, setVariantFormOpen] = useState(false)
  const [versionFormOpen, setVersionFormOpen] = useState(false)
  const capabilityItems = useMemo(() => data.dictionaries.filter((item) => item.kind === 'capability'), [data])
  const groupedCapabilityItems = useMemo(() => groupCapabilityItems(capabilityItems), [capabilityItems])
  const capabilities = useMemo(() => getDictionaryKeys(data, 'capability'), [data])
  const logicalDirectories = useMemo(() => getMaterializedLogicalDirectories(data), [data])
  const logicalMounts = useMemo(() => logicalDirectories.map((directory) => directory.path), [logicalDirectories])

  const defaultCapabilities = capabilities.includes('streaming') ? ['streaming'] : capabilities.slice(0, 1)
  const defaultCapabilityValues = getDefaultCapabilityValues(defaultCapabilities, capabilityItems)
  const defaultVariantValues = (): MetadataVariantInput => ({
    variant_key: buildVariantKey(data.metadata_variants.length + 1),
    provider_key: selectedProvider?.provider_key ?? '',
    selector_type: 'pattern',
    original_provider: defaultOriginalProvider,
    model_id_selector: '*',
    priority: 20,
    nick: 'fast',
    mount_suffix: 'fast',
    logical_mounts: logicalMounts.includes('/llm') ? ['/llm'] : logicalMounts.slice(0, 1),
    capabilities: defaultCapabilities,
    capability_values: defaultCapabilityValues,
    provider_options_json: '{\n  "reasoning": {\n    "effort": "low"\n  }\n}',
    content_json: '',
  })
  const defaultVersionValues = (): MetadataVersionRuleInput => ({
    version_rule_key: buildVersionRuleKey(data.metadata_version_rules.length + 1),
    provider_key: selectedProvider?.provider_key ?? '',
    selector_type: 'pattern',
    original_provider: defaultOriginalProvider,
    model_id_selector: 'gpt-*',
    priority: 20,
    nick: 'standard',
    family: defaultOriginalProvider,
    tier: 'standard',
    model_pattern: 'gpt-*',
    tier_tokens: [],
    exclude_tier_tokens: [],
    version_rank_prefix: defaultOriginalProvider,
    stability_unstable_tokens: ['preview', 'beta'],
    stability_current_requires_stable: true,
    current_mount: `${defaultOriginalProvider}.current`,
    version_mount: `${defaultOriginalProvider}.{model}`,
    exclude_snapshot_date_suffix: true,
    auto_mounts: logicalMounts.includes('/llm') ? ['/llm'] : logicalMounts.slice(0, 1),
    capabilities: defaultCapabilities,
    capability_values: defaultCapabilityValues,
    content_json: '',
  })

  const variantForm = useForm<MetadataVariantInput>({
    resolver: zodResolver(metadataVariantInputSchema),
    defaultValues: defaultVariantValues(),
  })
  const versionForm = useForm<MetadataVersionRuleInput>({
    resolver: zodResolver(metadataVersionRuleInputSchema),
    defaultValues: defaultVersionValues(),
  })
  const watchedVersionMounts = versionForm.watch('auto_mounts') ?? []
  const watchedVersionCapabilities = versionForm.watch('capabilities') ?? []
  const watchedVersionCapabilityValues = versionForm.watch('capability_values') ?? {}

  const toggleVersionArray = (field: 'auto_mounts' | 'capabilities', item: string) => {
    const current = versionForm.getValues(field) ?? []
    const next = current.includes(item) ? current.filter((value) => value !== item) : [...current, item]
    versionForm.setValue(field, next, {
      shouldDirty: true,
      shouldValidate: true,
    })
    if (field === 'capabilities') {
      versionForm.setValue('capability_values', updateCapabilityValues(versionForm.getValues('capability_values'), capabilityItems, item, next.includes(item)), {
        shouldDirty: true,
        shouldValidate: true,
      })
    }
  }

  const setVersionCapabilityNumber = (capability: string, value: string) => {
    versionForm.setValue('capability_values', { ...(versionForm.getValues('capability_values') ?? {}), [capability]: value ? Number(value) : 0 }, { shouldDirty: true, shouldValidate: true })
  }

  const startNewVariant = () => {
    setEditingVariant(false)
    variantForm.reset(defaultVariantValues())
    setVariantFormOpen(true)
    setActiveResolverTab('variant')
  }

  const startNewVersionRule = () => {
    setEditingVersionRule(false)
    versionForm.reset(defaultVersionValues())
    setVersionFormOpen(true)
    setActiveResolverTab('version_rule')
  }

  const variantColumns = useMemo<Array<DataTableColumn<MetadataVariantRecord>>>(() => [
    { key: 'key', title: t('resolver.variantKey', 'Variant key'), render: (rule) => <span className="font-mono text-xs">{rule.variant_key}</span> },
    { key: 'selector', title: t('rules.selector', 'Selector'), render: (rule) => <span className="font-mono text-xs">{rule.model_id_selector}</span> },
    { key: 'priority', title: t('rules.priority', 'Priority'), render: (rule) => rule.priority },
    { key: 'hits', title: t('rules.hits', 'Hits'), render: (rule) => previewVariantHits(data, rule).length },
  ], [data, t])

  const versionColumns = useMemo<Array<DataTableColumn<MetadataVersionRuleRecord>>>(() => [
    { key: 'key', title: t('resolver.versionRuleKey', 'Version rule key'), render: (rule) => <span className="font-mono text-xs">{rule.version_rule_key}</span> },
    { key: 'selector', title: t('rules.selector', 'Selector'), render: (rule) => <span className="font-mono text-xs">{rule.model_id_selector}</span> },
    { key: 'match', title: t('resolver.matchRule', 'Match rule'), render: (rule) => <VersionRuleSummary rule={rule} /> },
    { key: 'priority', title: t('rules.priority', 'Priority'), render: (rule) => rule.priority },
    { key: 'hits', title: t('rules.hits', 'Hits'), render: (rule) => previewVariantHits(data, rule).length },
  ], [data, t])

  const editVariant = (rule: MetadataVariantRecord) => {
    setEditingVariant(true)
    setVariantFormOpen(true)
    const ruleCapabilities = readRecord(rule.content.capabilities)
    variantForm.reset({ variant_key: rule.variant_key, provider_key: rule.provider_key ?? '', selector_type: rule.selector_type, original_provider: rule.original_provider ?? defaultOriginalProvider, model_id_selector: rule.model_id_selector, priority: rule.priority, nick: rule.nick ?? '', mount_suffix: typeof rule.content.mount_suffix === 'string' ? rule.content.mount_suffix : '', logical_mounts: readStringArray(rule.content.logical_mounts), capabilities: Object.keys(ruleCapabilities), capability_values: readCapabilityValues(ruleCapabilities), provider_options_json: providerOptionsJson(rule.content), content_json: '' })
  }

  const editVersionRule = (rule: MetadataVersionRuleRecord) => {
    setEditingVersionRule(true)
    setVersionFormOpen(true)
    const ruleCapabilities = readRecord(rule.content.capabilities)
    const stability = readRecord(rule.content.stability)
    const versionRank = readRecord(rule.content.version_rank)
    versionForm.reset({ version_rule_key: rule.version_rule_key, provider_key: rule.provider_key ?? '', selector_type: rule.selector_type, original_provider: rule.original_provider ?? defaultOriginalProvider, model_id_selector: rule.model_id_selector, priority: rule.priority, nick: rule.nick ?? '', family: typeof rule.content.family === 'string' ? rule.content.family : rule.original_provider ?? defaultOriginalProvider, tier: typeof rule.content.tier === 'string' ? rule.content.tier : 'standard', model_pattern: typeof rule.content.model_pattern === 'string' ? rule.content.model_pattern : rule.model_id_selector, tier_tokens: readStringArray(rule.content.tier_tokens), exclude_tier_tokens: readStringArray(rule.content.exclude_tier_tokens), version_rank_prefix: typeof versionRank.prefix === 'string' ? versionRank.prefix : '', stability_unstable_tokens: readStringArray(stability.unstable_tokens), stability_current_requires_stable: typeof stability.current_requires_stable === 'boolean' ? stability.current_requires_stable : false, current_mount: typeof rule.content.current_mount === 'string' ? rule.content.current_mount : '', version_mount: typeof rule.content.version_mount === 'string' ? rule.content.version_mount : '', exclude_snapshot_date_suffix: rule.content.exclude_snapshot_date_suffix === true, auto_mounts: readStringArray(rule.content.auto_mounts), capabilities: Object.keys(ruleCapabilities), capability_values: readCapabilityValues(ruleCapabilities), content_json: '' })
  }

  return (
    <div className="space-y-4" data-testid="resolver-rules-page">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('resolver.title', 'Resolver Rules')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')} / {t('resolver.variantVersionOnly', 'Variants and version rules')}</p>
        </div>
        <StatusBadge tone="accent">{data.metadata_variants.length + data.metadata_version_rules.length}</StatusBadge>
      </header>

      <section className="shell-card flex flex-wrap items-center gap-2 p-3">
        <button className={`h-9 rounded-md px-3 text-sm font-semibold ${activeResolverTab === 'variant' ? 'bg-[color:var(--cp-accent)] text-white' : 'border border-[color:var(--cp-border)]'}`} onClick={() => setActiveResolverTab('variant')} type="button">{t('resolver.variants', 'Variants')}</button>
        <button className={`h-9 rounded-md px-3 text-sm font-semibold ${activeResolverTab === 'version_rule' ? 'bg-[color:var(--cp-accent)] text-white' : 'border border-[color:var(--cp-border)]'}`} onClick={() => setActiveResolverTab('version_rule')} type="button">{t('resolver.versionRules', 'Version rules')}</button>
        <select className={`${inputClass} ml-auto max-w-56`} value={providerFilter} onChange={(event) => setProviderFilter(event.target.value)}><option value="">{t('filter.all', 'All')}</option>{providers.map((provider) => <option key={provider.key} value={provider.key}>{provider.label}</option>)}</select>
      </section>

      <section>
        {activeResolverTab === 'variant' && <Panel title={t('resolver.variants', 'Variants')}>
          {viewMode === 'edit' && (
            <div className="mb-3 flex justify-end">
              <button className="inline-flex h-9 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={startNewVariant} type="button"><Plus size={14} />{t('wizard.addVariant', 'Add variant')}</button>
            </div>
          )}
          {viewMode === 'edit' && variantFormOpen && <form
            className="mb-3 grid gap-3 md:grid-cols-4"
            onSubmit={variantForm.handleSubmit(async (value) => {
              await upsertMetadataVariant(value)
              setVariantFormOpen(false)
              setEditingVariant(false)
              variantForm.reset(defaultVariantValues())
            })}
          >
            <input type="hidden" {...variantForm.register('variant_key')} />
            <input type="hidden" {...variantForm.register('content_json')} />
            <Field label={t('table.provider', 'Provider')}>
              <select className={inputClass} disabled={editingVariant} {...variantForm.register('provider_key')}>
                <option value="">{t('models.global', 'Global / origin')}</option>
                {providers.map((provider) => <option key={provider.key} value={provider.key}>{provider.label}</option>)}
              </select>
            </Field>
            <Field label={t('rules.selector', 'Selector')}>
              <select className={inputClass} {...variantForm.register('selector_type')}>
                <option value="pattern">{t('nick.pattern', 'Pattern rewrite')}</option>
                <option value="exact">{t('nick.exact', 'Exact nick')}</option>
              </select>
            </Field>
            <Field label={t('models.originalProvider', 'Original provider')}>
              <select className={inputClass} disabled={editingVariant} {...variantForm.register('original_provider')}>
                {originalProviders.map((provider) => <option key={provider} value={provider}>{provider}</option>)}
              </select>
            </Field>
            <Field label={t('table.modelId', 'Model selector')}>
              <input className={inputClass} {...variantForm.register('model_id_selector')} />
            </Field>
            <Field label={t('rules.priority', 'Priority')}>
              <input className={inputClass} type="number" {...variantForm.register('priority', { valueAsNumber: true })} />
            </Field>
            <Field label={t('nick.exact', 'Exact nick')}>
              <input className={inputClass} {...variantForm.register('nick')} />
            </Field>
            <Field label={t('resolver.mountSuffix', 'Mount suffix')}>
              <input className={inputClass} {...variantForm.register('mount_suffix')} />
            </Field>
            <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)] md:col-span-4">
              {t('resolver.providerOptions', 'Provider options JSON')}
              <textarea className="min-h-28 rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-2 font-mono text-xs text-[color:var(--cp-text)]" {...variantForm.register('provider_options_json')} />
            </label>
            <div className="flex items-end justify-end gap-2 md:col-span-4">
              <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" type="button" onClick={() => { setVariantFormOpen(false); setEditingVariant(false) }}>{t('action.discard', 'Discard')}</button>
              <button className="h-10 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" type="submit">{t('action.saveDraft', 'Save draft')}</button>
            </div>
          </form>}
          <DataTable actions={viewMode === 'edit' ? (rule) => <div className="flex gap-1"><button aria-label={`Edit ${rule.variant_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)]" onClick={() => editVariant(rule)} type="button"><Pencil size={14} /></button><button aria-label={`Delete ${rule.variant_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-danger)] text-[color:var(--cp-danger)]" onClick={() => void removeMetadataVariant(rule.variant_key)} type="button"><Trash2 size={14} /></button></div> : undefined} columns={variantColumns} onSelect={(rule) => {
            setInspector({ title: rule.variant_key, subtitle: rule.model_id_selector, status: rule.enabled ? 'enabled' : 'disabled', json: rule })
          }} rowKey={(rule) => rule.variant_key} rows={data.metadata_variants.filter((rule) => !providerFilter || rule.provider_key === providerFilter)} />
        </Panel>}
        {activeResolverTab === 'version_rule' && <Panel title={t('resolver.versionRules', 'Version rules')}>
          {viewMode === 'edit' && (
            <div className="mb-3 flex justify-end">
              <button className="inline-flex h-9 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={startNewVersionRule} type="button"><Plus size={14} />{t('wizard.addVersionRule', 'Add version rule')}</button>
            </div>
          )}
          {viewMode === 'edit' && versionFormOpen && <form
            className="mb-3 grid gap-3 md:grid-cols-4"
            onSubmit={versionForm.handleSubmit(async (value) => {
              await upsertMetadataVersionRule(value)
              setVersionFormOpen(false)
              setEditingVersionRule(false)
              versionForm.reset(defaultVersionValues())
            })}
          >
            <input type="hidden" {...versionForm.register('version_rule_key')} />
            <input type="hidden" {...versionForm.register('content_json')} />
            <Field label={t('table.provider', 'Provider')}>
              <select className={inputClass} disabled={editingVersionRule} {...versionForm.register('provider_key')}>
                <option value="">{t('models.global', 'Global / origin')}</option>
                {providers.map((provider) => <option key={provider.key} value={provider.key}>{provider.label}</option>)}
              </select>
            </Field>
            <Field label={t('rules.selector', 'Selector')}>
              <select className={inputClass} {...versionForm.register('selector_type')}>
                <option value="pattern">{t('nick.pattern', 'Pattern rewrite')}</option>
                <option value="exact">{t('nick.exact', 'Exact nick')}</option>
              </select>
            </Field>
            <Field label={t('models.originalProvider', 'Original provider')}>
              <select className={inputClass} disabled={editingVersionRule} {...versionForm.register('original_provider')}>
                {originalProviders.map((provider) => <option key={provider} value={provider}>{provider}</option>)}
              </select>
            </Field>
            <Field label={t('table.modelId', 'Model selector')}>
              <input className={inputClass} {...versionForm.register('model_id_selector')} />
            </Field>
            <Field label={t('rules.priority', 'Priority')}>
              <input className={inputClass} type="number" {...versionForm.register('priority', { valueAsNumber: true })} />
            </Field>
            <Field label={t('resolver.family', 'Family')}>
              <input className={inputClass} {...versionForm.register('family')} />
            </Field>
            <Field label={t('resolver.tier', 'Tier')}>
              <input className={inputClass} {...versionForm.register('tier')} />
            </Field>
            <Field label={t('resolver.modelPattern', 'Model pattern')}>
              <input className={`${inputClass} font-mono`} {...versionForm.register('model_pattern')} />
            </Field>
            <Field label={t('resolver.versionRankPrefix', 'Version rank prefix')}>
              <input className={inputClass} {...versionForm.register('version_rank_prefix')} />
            </Field>
            <div className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
              {t('resolver.currentMount', 'Current mount')}
              <MountPathPicker clearLabel={t('action.clear', 'Clear')} directories={logicalDirectories} editLabel={t('mode.edit', 'Edit')} emptyLabel={t('resolver.noMountSelected', 'No mount selected')} value={versionForm.watch('current_mount') ?? ''} onChange={(mount) => versionForm.setValue('current_mount', mount, { shouldDirty: true, shouldValidate: true })} />
            </div>
            <div className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
              {t('resolver.versionMount', 'Version mount')}
              <MountPathPicker clearLabel={t('action.clear', 'Clear')} directories={logicalDirectories} editLabel={t('mode.edit', 'Edit')} emptyLabel={t('resolver.noMountSelected', 'No mount selected')} value={versionForm.watch('version_mount') ?? ''} onChange={(mount) => versionForm.setValue('version_mount', mount, { shouldDirty: true, shouldValidate: true })} />
            </div>
            <Field label={t('nick.exact', 'Exact nick')}>
              <input className={inputClass} {...versionForm.register('nick')} />
            </Field>
            <label className="flex items-center gap-2 self-end text-sm font-semibold">
              <input className="h-4 w-4" type="checkbox" {...versionForm.register('stability_current_requires_stable')} />
              {t('resolver.currentRequiresStable', 'Current requires stable')}
            </label>
            <label className="flex items-center gap-2 self-end text-sm font-semibold">
              <input className="h-4 w-4" type="checkbox" {...versionForm.register('exclude_snapshot_date_suffix')} />
              {t('resolver.excludeSnapshotDateSuffix', 'Exclude snapshot date suffix')}
            </label>
            <div className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
              {t('resolver.autoMounts', 'Auto mounts')}
              <AutoMountPicker clearLabel={t('action.clear', 'Clear')} directories={logicalDirectories} editLabel={t('mode.edit', 'Edit')} emptyLabel={t('resolver.noMountSelected', 'No mount selected')} selected={watchedVersionMounts} onClear={() => versionForm.setValue('auto_mounts', [], { shouldDirty: true, shouldValidate: true })} onToggle={(mount) => toggleVersionArray('auto_mounts', mount)} />
            </div>
            <ChoiceGroup title={t('filter.capability', 'Capability')}>
              {groupedCapabilityItems.map((capability) => (
                <CapabilityChoice
                  checked={watchedVersionCapabilities.includes(capability.key)}
                  item={capability}
                  key={capability.key}
                  onChangeNumber={(value) => setVersionCapabilityNumber(capability.key, value)}
                  onToggle={() => toggleVersionArray('capabilities', capability.key)}
                  value={watchedVersionCapabilityValues[capability.key]}
                />
              ))}
            </ChoiceGroup>
            <TokenInput label={t('resolver.tierTokens', 'Tier tokens')} value={(versionForm.watch('tier_tokens') ?? []).join(', ')} onChange={(value) => versionForm.setValue('tier_tokens', parseTokenText(value), { shouldDirty: true, shouldValidate: true })} />
            <TokenInput label={t('resolver.excludeTierTokens', 'Exclude tier tokens')} value={(versionForm.watch('exclude_tier_tokens') ?? []).join(', ')} onChange={(value) => versionForm.setValue('exclude_tier_tokens', parseTokenText(value), { shouldDirty: true, shouldValidate: true })} />
            <TokenInput label={t('resolver.unstableTokens', 'Unstable tokens')} value={(versionForm.watch('stability_unstable_tokens') ?? []).join(', ')} onChange={(value) => versionForm.setValue('stability_unstable_tokens', parseTokenText(value), { shouldDirty: true, shouldValidate: true })} />
            <div className="flex items-end justify-end gap-2 md:col-span-4">
              <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" type="button" onClick={() => { setVersionFormOpen(false); setEditingVersionRule(false) }}>{t('action.discard', 'Discard')}</button>
              <button className="h-10 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" type="submit">{t('action.saveDraft', 'Save draft')}</button>
            </div>
          </form>}
          <DataTable actions={viewMode === 'edit' ? (rule) => <div className="flex gap-1"><button aria-label={`Edit ${rule.version_rule_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)]" onClick={() => editVersionRule(rule)} type="button"><Pencil size={14} /></button><button aria-label={`Delete ${rule.version_rule_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-danger)] text-[color:var(--cp-danger)]" onClick={() => void removeMetadataVersionRule(rule.version_rule_key)} type="button"><Trash2 size={14} /></button></div> : undefined} columns={versionColumns} onSelect={(rule) => {
            setInspector({ title: rule.version_rule_key, subtitle: rule.model_id_selector, status: rule.enabled ? 'enabled' : 'disabled', json: rule })
          }} rowKey={(rule) => rule.version_rule_key} rows={data.metadata_version_rules.filter((rule) => !providerFilter || rule.provider_key === providerFilter)} />
        </Panel>}
      </section>

    </div>
  )
}

type ResolverOverlayRow = {
  target_type: Extract<OpsOverlayRecord['target_type'], 'variants' | 'version_rules'>
  target_key: string
  selector: string
  kind: string
  provider_key: string | null
  enabled: boolean
}

function VersionRuleSummary({ rule }: { rule: MetadataVersionRuleRecord }) {
  const content = rule.content
  const tierTokens = Array.isArray(content.tier_tokens) ? content.tier_tokens.filter((item): item is string => typeof item === 'string') : []
  const excludeTierTokens = Array.isArray(content.exclude_tier_tokens) ? content.exclude_tier_tokens.filter((item): item is string => typeof item === 'string') : []
  const unstableTokens = typeof content.stability === 'object' && content.stability !== null && Array.isArray((content.stability as { unstable_tokens?: unknown }).unstable_tokens)
    ? (content.stability as { unstable_tokens: unknown[] }).unstable_tokens.filter((item): item is string => typeof item === 'string')
    : []
  const parts = [
    typeof content.family === 'string' ? `family=${content.family}` : null,
    typeof content.tier === 'string' ? `tier=${content.tier}` : null,
    tierTokens.length ? `tier tokens: ${tierTokens.join(', ')}` : null,
    excludeTierTokens.length ? `exclude: ${excludeTierTokens.join(', ')}` : null,
    unstableTokens.length ? `unstable: ${unstableTokens.join(', ')}` : null,
    typeof content.current_mount === 'string' ? `current=${content.current_mount}` : null,
  ].filter(Boolean)
  return <span className="text-xs text-[color:var(--cp-muted)]">{parts.join(' / ') || '-'}</span>
}

function OpsResolverOverlayPage() {
  const { t } = useI18n()
  const { workspace, viewMode } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [activeResolverTab, setActiveResolverTab] = useState<'variants' | 'version_rules'>('variants')
  const [providerFilter, setProviderFilter] = useState('')
  const providers = data.providers.map((provider) => ({ key: provider.provider_key, label: provider.name }))
  const rows = useMemo<ResolverOverlayRow[]>(() => [
    ...data.metadata_variants.map((rule) => ({
      target_type: 'variants' as const,
      target_key: rule.variant_key,
      selector: rule.model_id_selector,
      kind: 'variant',
      provider_key: rule.provider_key,
      enabled: rule.enabled,
    })),
    ...data.metadata_version_rules.map((rule) => ({
      target_type: 'version_rules' as const,
      target_key: rule.version_rule_key,
      selector: rule.model_id_selector,
      kind: 'version_rule',
      provider_key: rule.provider_key,
      enabled: rule.enabled,
    })),
  ], [data])
  const visibleRows = useMemo(() => rows.filter((row) => row.target_type === activeResolverTab && (!providerFilter || row.provider_key === providerFilter)), [activeResolverTab, providerFilter, rows])

  const columns = useMemo<Array<DataTableColumn<ResolverOverlayRow>>>(() => [
    { key: 'target', title: t('ops.target', 'Target'), render: (row) => <span className="font-mono text-xs">{row.target_key}</span> },
    { key: 'kind', title: t('rules.type', 'Type'), render: (row) => row.kind },
    { key: 'selector', title: t('rules.selector', 'Selector'), render: (row) => <span className="font-mono text-xs">{row.selector}</span> },
    {
      key: 'overlay',
      title: t('ops.overlay', 'Overlay'),
      render: (row) => {
        const overlay = getOpsOverlay(data, row.target_type, row.target_key)
        return <StatusBadge tone={overlay?.disabled ? 'danger' : overlay ? 'warning' : 'success'}>{overlay?.disabled ? t('ops.disabled', 'Disabled') : overlay ? t('mode.edit', 'Edit') : t('status.published', 'Published')}</StatusBadge>
      },
    },
  ], [data, t])

  function handleSelect(row: ResolverOverlayRow) {
    const overlay = getOpsOverlay(data, row.target_type, row.target_key)
    setInspector({
      title: row.target_key,
      subtitle: t('ops.resolverReadonlyHint', 'Variants and version rules are read-only for operations parameters'),
      status: row.enabled ? 'enabled' : 'disabled',
      json: {
        target: row,
        operations_overlay: overlay,
      },
    })
  }

  return (
    <div className="space-y-4" data-testid="ops-resolver-page">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('ops.resolverTitle', 'Resolver Operations Overlay')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')} / {t('ops.resolverReadonlyHint', 'Variants and version rules are read-only for operations parameters')}</p>
        </div>
        <StatusBadge tone="accent">{rows.length}</StatusBadge>
      </header>

      <section className="shell-card flex flex-wrap items-center gap-2 p-3">
        <button className={`h-9 rounded-md px-3 text-sm font-semibold ${activeResolverTab === 'variants' ? 'bg-[color:var(--cp-accent)] text-white' : 'border border-[color:var(--cp-border)]'}`} onClick={() => setActiveResolverTab('variants')} type="button">{t('resolver.variants', 'Variants')}</button>
        <button className={`h-9 rounded-md px-3 text-sm font-semibold ${activeResolverTab === 'version_rules' ? 'bg-[color:var(--cp-accent)] text-white' : 'border border-[color:var(--cp-border)]'}`} onClick={() => setActiveResolverTab('version_rules')} type="button">{t('resolver.versionRules', 'Version rules')}</button>
        <select className={`${inputClass} ml-auto max-w-56`} value={providerFilter} onChange={(event) => setProviderFilter(event.target.value)}>
          <option value="">{t('filter.all', 'All')}</option>
          {providers.map((provider) => <option key={provider.key} value={provider.key}>{provider.label}</option>)}
        </select>
      </section>

      <DataTable columns={columns} onSelect={handleSelect} rowKey={(row) => `${row.target_type}-${row.target_key}`} rows={visibleRows} />
    </div>
  )
}

function Panel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="shell-card p-4">
      <h2 className="mb-3 text-sm font-bold">{title}</h2>
      {children}
    </section>
  )
}

function ChoiceGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-md border border-[color:var(--cp-border)] p-3 md:col-span-2">
      <h3 className="mb-2 text-xs font-semibold text-[color:var(--cp-muted)]">{title}</h3>
      <div className="shell-scrollbar flex max-h-36 flex-wrap gap-2 overflow-auto">{children}</div>
    </section>
  )
}

function TokenInput({ label, value, onChange }: { label: string; value: string; onChange: (value: string) => void }) {
  return (
    <Field label={label}>
      <input
        className={`${inputClass} font-mono`}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </Field>
  )
}

function MountPathPicker({ clearLabel, directories, editLabel, emptyLabel, value, onChange }: { clearLabel: string; directories: LogicalDirectoryRecord[]; editLabel: string; emptyLabel: string; value: string; onChange: (mount: string) => void }) {
  const [editing, setEditing] = useState(false)
  const selectedPath = mountToDirectoryPath(value)
  return (
    <section className="rounded-md border border-[color:var(--cp-border)] p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
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
              <button className="flex w-full cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-left text-xs hover:bg-[color:var(--cp-surface-2)]" key={directory.directory_key} onClick={() => onChange(directoryPathToMount(directory.path))} style={{ paddingLeft: `${8 + directory.path.split('/').filter(Boolean).length * 12}px` }} type="button">
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

function AutoMountPicker({ clearLabel, directories, editLabel, emptyLabel, selected, onClear, onToggle }: { clearLabel: string; directories: LogicalDirectoryRecord[]; editLabel: string; emptyLabel: string; selected: string[]; onClear: () => void; onToggle: (mount: string) => void }) {
  const [editing, setEditing] = useState(false)
  return (
    <section className="rounded-md border border-[color:var(--cp-border)] p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold" onClick={() => setEditing((current) => !current)} type="button">
            {editLabel}
          </button>
          <button className="rounded-md border border-[color:var(--cp-border)] px-2 py-1 text-xs font-semibold disabled:opacity-40" disabled={!selected.length} onClick={onClear} type="button">
            {clearLabel}
          </button>
        </div>
      </div>
      <div className="mb-2 min-h-7 rounded-md bg-[color:var(--cp-surface-2)] px-2 py-1 font-mono text-xs text-[color:var(--cp-muted)]">
        {selected.length ? selected.join(', ') : emptyLabel}
      </div>
      {editing && (
        <LogicalMountTreePicker directories={directories} selected={selected} onToggle={onToggle} />
      )}
    </section>
  )
}

function CapabilityChoice({ checked, item, onChangeNumber, onToggle, value }: { checked: boolean; item: DictionaryItem; onChangeNumber: (value: string) => void; onToggle: () => void; value: boolean | number | undefined }) {
  return (
    <div className={`flex min-w-44 flex-col gap-1 rounded-md border p-2 ${checked ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)]' : 'border-[color:var(--cp-border)]'}`}>
      <button className="min-h-8 rounded-md border border-transparent px-2 py-1.5 text-left font-mono text-xs" onClick={onToggle} type="button">
        {item.key}
      </button>
      {item.value_type !== 'boolean' && checked && (
        <input
          aria-label={item.label}
          className={inputClass}
          min="0"
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

function updateCapabilityValues(current: Record<string, boolean | number> | undefined, dictionaries: DictionaryItem[], key: string, enabled: boolean) {
  const next = { ...(current ?? {}) }
  if (!enabled) {
    delete next[key]
    return next
  }
  const dictionary = dictionaries.find((item) => item.kind === 'capability' && item.key === key)
  next[key] = dictionary?.value_type === 'number' ? 1 : true
  return next
}

function readRecord(value: unknown) {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : {}
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

function readStringArray(value: unknown) {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : []
}

function providerOptionsJson(content: Record<string, unknown>) {
  const providerOptions = readRecord(content.provider_options)
  return Object.keys(providerOptions).length ? JSON.stringify(providerOptions, null, 2) : ''
}

function parseTokenText(value: string) {
  return value.split(/[,\s]+/).map((token) => token.trim()).filter(Boolean)
}

function safeKeyPart(value: string) {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'rule'
}

function buildVariantKey(index: number) {
  return `variant-${String(index).padStart(4, '0')}-${safeKeyPart(String(Date.now()).slice(-6))}`
}

function buildVersionRuleKey(index: number) {
  return `version-rule-${String(index).padStart(4, '0')}-${safeKeyPart(String(Date.now()).slice(-6))}`
}

function getDefaultCapabilityValues(capabilities: string[], dictionaries: DictionaryItem[]) {
  return Object.fromEntries(capabilities.map((capability) => {
    const dictionary = dictionaries.find((item) => item.kind === 'capability' && item.key === capability)
    return [capability, dictionary?.value_type === 'number' ? 1 : true]
  }))
}

function mountToDirectoryPath(mount?: string) {
  const normalized = mount?.trim().replace(/\./g, '/').replace(/^\/+|\/+$/g, '')
  return normalized ? `/${normalized}` : ''
}

function directoryPathToMount(path: string) {
  return path.trim().replace(/^\/+|\/+$/g, '').replace(/\//g, '.')
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

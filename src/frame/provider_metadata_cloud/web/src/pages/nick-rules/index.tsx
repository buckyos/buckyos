import { useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { Pencil, Plus, Trash2 } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { EmptyView } from '../../components/empty-state/StateView'
import { JsonViewer } from '../../components/json-viewer/JsonViewer'
import { StatusBadge } from '../../components/status/StatusBadge'
import { buildPublishedProviderJson, getOriginalProviders, getSourceModelIds } from '../../datamodel/selectors'
import { nickRuleInputSchema, originMappingRuleInputSchema, originProviderAliasInputSchema, type NickRuleInput, type OriginMappingRuleInput, type OriginProviderAliasInput } from '../../datamodel/schemas'
import type { MatchType, ModelNickRecord, OriginMappingRuleRecord, OriginProviderAliasRecord, ProviderCloudSeed } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { useShellContext } from '../pageUtils'

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm text-[color:var(--cp-text)]'

type NickRewritePreviewTarget = 'model' | 'pattern' | 'default' | 'variant' | 'version_rule'
type ConfigTab = 'nick' | 'origin_mappings' | 'origin_provider_aliases'

interface NickRewritePreviewItem {
  target: NickRewritePreviewTarget
  source_model_id: string
  original_provider: string | null
  published_id: string
  source_key: string
}

export function NickRulesPage() {
  const { t } = useI18n()
  const { workspace, upsertNickRule, removeNickRule, upsertOriginProviderAlias, removeOriginProviderAlias, upsertOriginMappingRule, removeOriginMappingRule, viewMode } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [formOpen, setFormOpen] = useState(false)
  const [editingRule, setEditingRule] = useState(false)
  const [aliasFormOpen, setAliasFormOpen] = useState(false)
  const [editingAlias, setEditingAlias] = useState(false)
  const [mappingFormOpen, setMappingFormOpen] = useState(false)
  const [editingMapping, setEditingMapping] = useState(false)
  const [activeTab, setActiveTab] = useState<ConfigTab>('nick')
  const [selectedNickKey, setSelectedNickKey] = useState(data.model_nicks[0]?.nick_key ?? '')
  const [providerFilter, setProviderFilter] = useState('')
  const [activePreviewTarget, setActivePreviewTarget] = useState<NickRewritePreviewTarget>('model')
  const originalProviders = useMemo(() => getOriginalProviders(data), [data])
  const sourceModels = useMemo(() => getSourceModelIds(data), [data])
  const defaultProviderKey = data.providers.find((provider) => provider.provider_key === 'openrouter')?.provider_key ?? data.providers[0]?.provider_key ?? ''
  const defaultOriginalProvider = originalProviders[0] ?? 'openai'
  const form = useForm<NickRuleInput>({
    resolver: zodResolver(nickRuleInputSchema),
    defaultValues: buildNickRuleDefaults(data, defaultProviderKey, defaultOriginalProvider),
  })
  const aliasForm = useForm<OriginProviderAliasInput>({
    resolver: zodResolver(originProviderAliasInputSchema),
    defaultValues: buildAliasDefaults(defaultProviderKey, defaultOriginalProvider),
  })
  const mappingForm = useForm<OriginMappingRuleInput>({
    resolver: zodResolver(originMappingRuleInputSchema),
    defaultValues: buildMappingDefaults(data, defaultProviderKey),
  })
  const formValues = form.watch()
  const mappingValues = mappingForm.watch()
  const visibleRules = useMemo(() => data.model_nicks.filter((rule) => !providerFilter || rule.provider_key === providerFilter), [data.model_nicks, providerFilter])
  const visibleAliases = useMemo(() => data.origin_provider_aliases.filter((alias) => !providerFilter || alias.provider_key === providerFilter), [data.origin_provider_aliases, providerFilter])
  const visibleMappings = useMemo(() => data.origin_mapping_rules.filter((rule) => !providerFilter || rule.provider_key === providerFilter), [data.origin_mapping_rules, providerFilter])
  const jsonPreviewProvider = useMemo(() => {
    return data.providers.find((provider) => provider.provider_key === (providerFilter || defaultProviderKey)) ?? data.providers[0] ?? null
  }, [data.providers, defaultProviderKey, providerFilter])
  const originMappingsJsonPreview = useMemo(() => {
    if (!jsonPreviewProvider) {
      return { origin_mappings: [] }
    }
    const published = buildPublishedProviderJson(data, jsonPreviewProvider)
    return {
      origin_mappings: published.origin_mappings ?? [],
    }
  }, [data, jsonPreviewProvider])
  const selectedRule = useMemo(() => {
    return data.model_nicks.find((rule) => rule.nick_key === selectedNickKey) ?? visibleRules[0] ?? data.model_nicks[0] ?? null
  }, [data.model_nicks, selectedNickKey, visibleRules])
  const previewRule = formOpen ? formValues : selectedRule
  const previewSections = useMemo(() => {
    return buildNickRewritePreviewSections(data, previewRule, t)
  }, [data, previewRule, t])
  const activePreviewSection = previewSections.find((section) => section.target === activePreviewTarget) ?? previewSections[0] ?? null

  const columns = useMemo<Array<DataTableColumn<ModelNickRecord>>>(() => [
    { key: 'key', title: t('nick.ruleKey', 'Nick key'), render: (rule) => <span className="font-mono text-xs">{rule.nick_key}</span> },
    { key: 'provider', title: t('table.provider', 'Provider'), render: (rule) => rule.provider_key },
    { key: 'origin', title: t('models.originalProvider', 'Original provider'), render: (rule) => rule.original_provider ?? '-' },
    { key: 'selector', title: t('table.modelId', 'Model selector'), render: (rule) => <span className="font-mono text-xs">{rule.model_id}</span> },
    { key: 'type', title: t('rules.type', 'Type'), render: (rule) => <StatusBadge tone={rule.selector_type === 'pattern' ? 'accent' : 'success'}>{rule.selector_type}</StatusBadge> },
    { key: 'nick', title: t('nick.publishedId', 'Published id'), render: (rule) => <span className="font-mono text-xs">{rule.nick}</span> },
    { key: 'priority', title: t('rules.priority', 'Priority'), render: (rule) => rule.priority },
  ], [t])
  const aliasColumns = useMemo<Array<DataTableColumn<OriginProviderAliasRecord>>>(() => [
    { key: 'key', title: t('originAlias.key', 'Alias key'), render: (alias) => <span className="font-mono text-xs">{alias.alias_key}</span> },
    { key: 'provider', title: t('table.provider', 'Provider'), render: (alias) => alias.provider_key },
    { key: 'alias', title: t('originAlias.alias', 'Provider alias'), render: (alias) => <span className="font-mono text-xs">{alias.alias}</span> },
    { key: 'driver', title: t('originAlias.driver', 'Origin driver'), render: (alias) => <span className="font-mono text-xs">{alias.driver}</span> },
  ], [t])
  const mappingColumns = useMemo<Array<DataTableColumn<OriginMappingRuleRecord>>>(() => [
    { key: 'key', title: t('originMapping.key', 'Mapping key'), render: (rule) => <span className="font-mono text-xs">{rule.mapping_key}</span> },
    { key: 'provider', title: t('table.provider', 'Provider'), render: (rule) => rule.provider_key },
    { key: 'mode', title: t('originMapping.mode', 'Mode'), render: (rule) => <StatusBadge tone={rule.mapping_mode === 'regex' ? 'warning' : 'success'}>{rule.mapping_mode}</StatusBadge> },
    { key: 'match', title: t('originMapping.matchPattern', 'Match pattern'), render: (rule) => <span className="font-mono text-xs">{rule.match_pattern}</span> },
    { key: 'origin', title: t('originMapping.originTemplate', 'Origin template'), render: (rule) => <span className="font-mono text-xs">{rule.mapping_mode === 'regex' ? rule.regex : rule.origin_template}</span> },
    { key: 'transforms', title: t('originMapping.transforms', 'Transforms'), render: (rule) => <span className="font-mono text-xs">d:{rule.driver_transforms.join(',') || '-'} m:{rule.model_transforms.join(',') || '-'}</span> },
    { key: 'priority', title: t('rules.priority', 'Priority'), render: (rule) => rule.priority },
  ], [t])

  const openCreateForm = () => {
    const origin = originalProviders[0] ?? 'openai'
    const draft = buildNickRuleDefaults(data, defaultProviderKey, origin)
    form.reset(draft)
    setFormOpen(true)
    setEditingRule(false)
    setSelectedNickKey(draft.nick_key)
    setActivePreviewTarget('model')
  }

  const openEditForm = (rule: ModelNickRecord) => {
    form.reset({
      nick_key: rule.nick_key,
      provider_key: rule.provider_key,
      original_provider: rule.original_provider ?? defaultOriginalProvider,
      model_id: rule.model_id,
      nick: rule.nick,
      selector_type: rule.selector_type,
      priority: rule.priority,
    })
    setSelectedNickKey(rule.nick_key)
    setEditingRule(true)
    setFormOpen(true)
    setActivePreviewTarget('model')
  }

  const openCreateAliasForm = () => {
    const providerKey = providerFilter || defaultProviderKey
    const draft = buildAliasDefaults(providerKey, defaultOriginalProvider)
    aliasForm.reset(draft)
    setAliasFormOpen(true)
    setEditingAlias(false)
  }

  const openEditAliasForm = (alias: OriginProviderAliasRecord) => {
    aliasForm.reset({
      alias_key: alias.alias_key,
      provider_key: alias.provider_key,
      alias: alias.alias,
      driver: alias.driver,
    })
    setEditingAlias(true)
    setAliasFormOpen(true)
  }

  const openCreateMappingForm = () => {
    const draft = buildMappingDefaults(data, providerFilter || defaultProviderKey)
    mappingForm.reset(draft)
    setMappingFormOpen(true)
    setEditingMapping(false)
  }

  const openEditMappingForm = (rule: OriginMappingRuleRecord) => {
    mappingForm.reset({
      mapping_key: rule.mapping_key,
      provider_key: rule.provider_key,
      mapping_mode: rule.mapping_mode,
      match_pattern: rule.match_pattern,
      origin_template: rule.origin_template,
      regex: rule.regex,
      driver_transforms: rule.driver_transforms,
      model_transforms: rule.model_transforms,
      priority: rule.priority,
    })
    setMappingFormOpen(true)
    setEditingMapping(true)
  }

  const toggleMappingFormDriverTransform = (op: 'trim' | 'lowercase' | 'alias') => {
    const values = mappingForm.getValues('driver_transforms')
    mappingForm.setValue('driver_transforms', values.includes(op) ? values.filter((item) => item !== op) : [...values, op], { shouldDirty: true, shouldValidate: true })
  }

  const toggleMappingFormModelTransform = (op: 'trim' | 'lowercase') => {
    const values = mappingForm.getValues('model_transforms')
    mappingForm.setValue('model_transforms', values.includes(op) ? values.filter((item) => item !== op) : [...values, op], { shouldDirty: true, shouldValidate: true })
  }

  return (
    <div className="space-y-4" data-testid="nick-rules-page">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('nick.title', 'Nick Rules')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')}</p>
        </div>
        <button className="inline-flex h-10 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={() => {
          if (activeTab === 'origin_mappings') {
            openCreateMappingForm()
          } else if (activeTab === 'origin_provider_aliases') {
            openCreateAliasForm()
          } else {
            openCreateForm()
          }
        }} type="button">
          <Plus size={16} />
          {activeTab === 'origin_mappings' ? t('originMapping.create', 'Create mapping') : activeTab === 'origin_provider_aliases' ? t('originAlias.create', 'Create alias') : t('nick.create', 'Create nick rule')}
        </button>
      </header>

      <section className="shell-card p-3">
        <select className={inputClass} value={providerFilter} onChange={(event) => setProviderFilter(event.target.value)}>
          <option value="">{t('filter.all', 'All')}</option>
          {data.providers.map((provider) => <option key={provider.provider_key} value={provider.provider_key}>{provider.name}</option>)}
        </select>
      </section>

      <section className="shell-card p-2">
        <div className="flex flex-wrap gap-2" role="tablist" aria-label={t('nick.configTabs', 'Nick and origin config tabs')}>
          {([
            ['nick', t('nick.title', 'Nick Rules')],
            ['origin_mappings', t('originMapping.title', 'Origin mappings')],
            ['origin_provider_aliases', t('originAlias.title', 'Origin provider aliases')],
          ] as Array<[ConfigTab, string]>).map(([tab, label]) => (
            <button className={`h-9 rounded-md border px-3 text-xs font-semibold ${activeTab === tab ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-accent)]' : 'border-[color:var(--cp-border)]'}`} key={tab} onClick={() => setActiveTab(tab)} role="tab" type="button">
              {label}
            </button>
          ))}
        </div>
      </section>

      {activeTab === 'nick' && <section className="shell-card p-4">
        <h2 className="text-sm font-bold">{t('wizard.nickConcept', 'Nick rewrite role')}</h2>
        <p className="mt-2 text-sm text-[color:var(--cp-muted)]">{t('wizard.nickConceptHint', 'Nick rewrite is a publish-time intermediate mapping. It reuses selected original models, patterns, defaults, variants, and version rules while publishing the provider inventory without copied renamed rules.')}</p>
        <p className="mt-2 text-sm text-[color:var(--cp-muted)]">{t('wizard.nickScopeHint', 'Rules are ordered by priority and also rewrite variants and version rules. Variants use * when no model selector exists; version rules rewrite content.model_pattern.')}</p>
      </section>}

      {activeTab === 'origin_provider_aliases' && <section className="shell-card p-4">
        <div className="mb-3 flex items-center justify-between gap-3">
          <div>
            <h2 className="text-sm font-bold">{t('originAlias.title', 'Origin provider aliases')}</h2>
            <p className="mt-1 text-xs text-[color:var(--cp-muted)]">{t('originAlias.hint', 'Provider scoped normalization for origin_mappings driver captures.')}</p>
          </div>
        </div>
        {aliasFormOpen && (
          <form
            className="mb-3 rounded-md border border-[color:var(--cp-border)] p-3"
            onSubmit={aliasForm.handleSubmit(async (value) => {
              await upsertOriginProviderAlias(value)
              setAliasFormOpen(false)
              setEditingAlias(false)
            })}
          >
            <input type="hidden" {...aliasForm.register('alias_key')} />
            <div className="grid gap-2 md:grid-cols-4">
              <Field label={t('table.provider', 'Provider')}>
                <select className={inputClass} disabled={editingAlias} {...aliasForm.register('provider_key')}>
                  {data.providers.map((provider) => <option key={provider.provider_key} value={provider.provider_key}>{provider.name}</option>)}
                </select>
              </Field>
              <Field label={t('originAlias.alias', 'Provider alias')} error={aliasForm.formState.errors.alias?.message}>
                <input className={`${inputClass} font-mono`} {...aliasForm.register('alias')} />
              </Field>
              <Field label={t('originAlias.driver', 'Origin driver')} error={aliasForm.formState.errors.driver?.message}>
                <input className={`${inputClass} font-mono`} list="origin-drivers" {...aliasForm.register('driver')} />
                <datalist id="origin-drivers">
                  {originalProviders.map((provider) => <option key={provider} value={provider} />)}
                </datalist>
              </Field>
              <div className="flex items-end gap-2">
                <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" type="button" onClick={() => {
                  setAliasFormOpen(false)
                  setEditingAlias(false)
                }}>{t('action.discard', 'Discard')}</button>
                <button className="h-10 rounded-md bg-[color:var(--cp-accent)] px-3 text-xs font-semibold text-white" type="submit">{t('action.saveDraft', 'Save draft')}</button>
              </div>
            </div>
          </form>
        )}
        {visibleAliases.length ? (
          <DataTable
            actions={viewMode === 'edit' ? (alias) => <div className="flex gap-1"><button aria-label={`Edit ${alias.alias_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)]" onClick={() => openEditAliasForm(alias)} type="button"><Pencil size={14} /></button><button aria-label={`Delete ${alias.alias_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-danger)] text-[color:var(--cp-danger)]" onClick={() => void removeOriginProviderAlias(alias.alias_key)} type="button"><Trash2 size={14} /></button></div> : undefined}
            columns={aliasColumns}
            onSelect={(alias) => {
              setAliasFormOpen(false)
              setEditingAlias(false)
              setInspector({ title: alias.alias_key, subtitle: `${alias.alias} -> ${alias.driver}`, status: alias.provider_key, json: alias })
            }}
            rowKey={(alias) => alias.alias_key}
            rows={visibleAliases}
          />
        ) : (
          <EmptyView />
        )}
      </section>}

      {activeTab === 'origin_mappings' && <section className="shell-card p-4">
        <div className="mb-3 flex items-center justify-between gap-3">
          <div>
            <h2 className="text-sm font-bold">{t('originMapping.title', 'Origin mappings')}</h2>
            <p className="mt-1 text-xs text-[color:var(--cp-muted)]">{t('originMapping.hint', 'Materializes origin_mappings. Template mode is for simple provider paths; regex mode is for complex provider model ids.')}</p>
          </div>
        </div>
        <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_420px]">
          <div>
            {mappingFormOpen && (
              <form
                className="mb-3 rounded-md border border-[color:var(--cp-border)] p-3"
                onSubmit={mappingForm.handleSubmit(async (value) => {
                  await upsertOriginMappingRule(value)
                  setMappingFormOpen(false)
                  setEditingMapping(false)
                })}
              >
                <input type="hidden" {...mappingForm.register('mapping_key')} />
                <div className="grid gap-2 md:grid-cols-5">
                  <Field label={t('table.provider', 'Provider')}>
                    <select className={inputClass} disabled={editingMapping} {...mappingForm.register('provider_key')}>
                      {data.providers.map((provider) => <option key={provider.provider_key} value={provider.provider_key}>{provider.name}</option>)}
                    </select>
                  </Field>
                  <Field label={t('originMapping.mode', 'Mode')}>
                    <select className={inputClass} {...mappingForm.register('mapping_mode')}>
                      <option value="template">{t('originMapping.templateMode', 'Template')}</option>
                      <option value="regex">{t('originMapping.regexMode', 'Regex')}</option>
                    </select>
                  </Field>
                  <Field label={t('originMapping.matchPattern', 'Match pattern')} error={mappingForm.formState.errors.match_pattern?.message}>
                    <input className={`${inputClass} font-mono`} {...mappingForm.register('match_pattern')} />
                  </Field>
                  {mappingValues.mapping_mode === 'template' ? (
                    <Field label={t('originMapping.originTemplate', 'Origin template')} error={mappingForm.formState.errors.origin_template?.message}>
                      <input className={`${inputClass} font-mono`} {...mappingForm.register('origin_template')} />
                    </Field>
                  ) : (
                    <Field label={t('originMapping.regex', 'Regex')} error={mappingForm.formState.errors.regex?.message}>
                      <input className={`${inputClass} font-mono`} {...mappingForm.register('regex')} />
                    </Field>
                  )}
                  <Field label={t('rules.priority', 'Priority')} error={mappingForm.formState.errors.priority?.message}>
                    <input className={inputClass} type="number" {...mappingForm.register('priority', { valueAsNumber: true })} />
                  </Field>
                  <div className="md:col-span-2">
                    <TransformToggles
                      label={t('originMapping.driverTransforms', 'Driver transforms')}
                      options={['trim', 'lowercase', 'alias']}
                      values={mappingValues.driver_transforms}
                      onToggle={(op) => toggleMappingFormDriverTransform(op as 'trim' | 'lowercase' | 'alias')}
                    />
                  </div>
                  <div className="md:col-span-2">
                    <TransformToggles
                      label={t('originMapping.modelTransforms', 'Model transforms')}
                      options={['trim', 'lowercase']}
                      values={mappingValues.model_transforms}
                      onToggle={(op) => toggleMappingFormModelTransform(op as 'trim' | 'lowercase')}
                    />
                  </div>
                </div>
                <div className="mt-3 flex justify-end gap-2">
                  <button className="h-9 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" type="button" onClick={() => {
                    setMappingFormOpen(false)
                    setEditingMapping(false)
                  }}>{t('action.discard', 'Discard')}</button>
                  <button className="h-9 rounded-md bg-[color:var(--cp-accent)] px-3 text-xs font-semibold text-white" type="submit">{t('action.saveDraft', 'Save draft')}</button>
                </div>
              </form>
            )}
            {visibleMappings.length ? (
              <DataTable
                actions={viewMode === 'edit' ? (rule) => <div className="flex gap-1"><button aria-label={`Edit ${rule.mapping_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)]" onClick={() => openEditMappingForm(rule)} type="button"><Pencil size={14} /></button><button aria-label={`Delete ${rule.mapping_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-danger)] text-[color:var(--cp-danger)]" onClick={() => void removeOriginMappingRule(rule.mapping_key)} type="button"><Trash2 size={14} /></button></div> : undefined}
                columns={mappingColumns}
                onSelect={(rule) => {
                  setMappingFormOpen(false)
                  setEditingMapping(false)
                  setInspector({ title: rule.mapping_key, subtitle: rule.mapping_mode === 'regex' ? rule.regex : rule.origin_template, status: rule.provider_key, json: rule })
                }}
                rowKey={(rule) => rule.mapping_key}
                rows={visibleMappings}
              />
            ) : (
              <EmptyView />
            )}
          </div>
          <aside className="rounded-md border border-[color:var(--cp-border)] p-3">
            <h3 className="mb-2 text-xs font-semibold text-[color:var(--cp-muted)]">{t('originMapping.publishedJson', 'Published origin_mappings JSON')}</h3>
            <JsonViewer value={originMappingsJsonPreview} filename={`${jsonPreviewProvider?.provider_key ?? 'provider'}-origin-mappings.json`} />
          </aside>
        </div>
      </section>}

      {activeTab === 'nick' && <section className="space-y-4">
        <div className="space-y-4">
          {formOpen && (
            <form
              className="shell-card p-4"
              onSubmit={form.handleSubmit(async (value) => {
                await upsertNickRule(value)
                setSelectedNickKey(value.nick_key)
                setFormOpen(false)
                setEditingRule(false)
              })}
            >
              <div className="mb-3 flex items-center justify-between gap-3">
                <h2 className="text-sm font-bold">{editingRule ? t('mode.edit', 'Edit') : t('nick.create', 'Create nick rule')}</h2>
                <StatusBadge tone="accent">{formValues.provider_key}</StatusBadge>
              </div>
              <input type="hidden" {...form.register('nick_key')} />
              <div className="grid gap-2 md:grid-cols-6">
                <Field label={t('table.provider', 'Provider')}>
                  <select className={inputClass} disabled={editingRule} {...form.register('provider_key')}>
                    {data.providers.map((provider) => <option key={provider.provider_key} value={provider.provider_key}>{provider.name}</option>)}
                  </select>
                </Field>
                <Field label={t('models.originalProvider', 'Original provider')}>
                  <select className={inputClass} {...form.register('original_provider')}>
                    {originalProviders.map((provider) => <option key={provider} value={provider}>{provider}</option>)}
                  </select>
                </Field>
                <Field label={t('rules.type', 'Type')}>
                  <select className={inputClass} {...form.register('selector_type')}>
                    <option value="pattern">{t('nick.originPrefixRules', 'Origin prefix rules')}</option>
                    <option value="exact">{t('nick.exact', 'Exact nick')}</option>
                  </select>
                </Field>
                <Field label={t('table.modelId', 'Model selector')} error={form.formState.errors.model_id?.message}>
                  <input className={`${inputClass} font-mono`} list="source-models" {...form.register('model_id')} />
                  <datalist id="source-models">
                    {sourceModels.map((modelId) => <option key={modelId} value={modelId} />)}
                  </datalist>
                </Field>
                <Field label={t('nick.publishedId', 'Published id')} error={form.formState.errors.nick?.message}>
                  <input className={`${inputClass} font-mono`} {...form.register('nick')} />
                </Field>
                <Field label={t('rules.priority', 'Priority')} error={form.formState.errors.priority?.message}>
                  <input className={inputClass} type="number" {...form.register('priority', { valueAsNumber: true })} />
                </Field>
              </div>
              <div className="mt-3 flex items-center justify-between gap-2">
                <button className="inline-flex h-9 items-center gap-2 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" onClick={openCreateForm} type="button">
                  <Plus size={14} />
                  {t('action.add', 'Add')}
                </button>
                <div className="flex gap-2">
                  <button className="h-9 rounded-md border border-[color:var(--cp-border)] px-3 text-xs font-semibold" type="button" onClick={() => {
                    setFormOpen(false)
                    setEditingRule(false)
                  }}>{t('action.discard', 'Discard')}</button>
                  <button className="h-9 rounded-md bg-[color:var(--cp-accent)] px-3 text-xs font-semibold text-white" type="submit">{t('action.saveDraft', 'Save draft')}</button>
                </div>
              </div>
            </form>
          )}

          {data.model_nicks.length ? (
            <DataTable
              actions={viewMode === 'edit' ? (rule) => <div className="flex gap-1"><button aria-label={`Edit ${rule.nick_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)]" onClick={() => openEditForm(rule)} type="button"><Pencil size={14} /></button><button aria-label={`Delete ${rule.nick_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-danger)] text-[color:var(--cp-danger)]" onClick={() => void removeNickRule(rule.nick_key)} type="button"><Trash2 size={14} /></button></div> : undefined}
              columns={columns}
              onSelect={(rule) => {
                setSelectedNickKey(rule.nick_key)
                setFormOpen(false)
                setEditingRule(false)
                setInspector({ title: rule.nick_key, subtitle: rule.nick, status: rule.selector_type, json: rule })
              }}
              rowKey={(rule) => rule.nick_key}
              rows={visibleRules}
            />
          ) : (
            <EmptyView />
          )}
        </div>
      </section>}

      <section className="shell-card p-4">
        <div className="flex items-start justify-between gap-3">
          <div>
            <h2 className="text-sm font-bold">{t('nick.preview', 'Mapping preview')}</h2>
            <p className="mt-1 break-all text-xs text-[color:var(--cp-muted)]">{previewRule ? `${previewRule.provider_key} / ${previewRule.nick_key}` : t('state.empty', 'No records match the current filters')}</p>
          </div>
          {previewRule && <StatusBadge tone={previewRule.selector_type === 'pattern' ? 'accent' : 'success'}>{previewRule.selector_type}</StatusBadge>}
        </div>
        <div className="mt-3 space-y-3">
          <div className="shell-scrollbar flex gap-2 overflow-auto pb-1">
            {previewSections.map((section) => {
              const active = section.target === activePreviewSection?.target
              return (
                <button className={`inline-flex h-9 shrink-0 items-center gap-2 rounded-md border px-3 text-xs font-semibold ${active ? 'border-[color:var(--cp-accent)] bg-[color:var(--cp-accent-soft)] text-[color:var(--cp-accent)]' : 'border-[color:var(--cp-border)]'}`} key={section.target} onClick={() => setActivePreviewTarget(section.target)} type="button">
                  <span>{section.label}</span>
                  <StatusBadge tone={nickRewritePreviewTone(section.target)}>{section.items.length}</StatusBadge>
                </button>
              )
            })}
          </div>
          {activePreviewSection ? (
            <div className="shell-scrollbar grid max-h-96 gap-2 overflow-auto md:grid-cols-2 xl:grid-cols-3">
              {activePreviewSection.items.map((item) => (
                <div className="rounded-md border border-[color:var(--cp-border)] p-2 text-xs" key={`${item.target}-${item.source_key}-${item.source_model_id}`}>
                  <div className="flex items-center justify-between gap-2">
                    <StatusBadge tone={nickRewritePreviewTone(item.target)}>{item.target}</StatusBadge>
                    <span className="truncate text-[color:var(--cp-muted)]">{item.source_key}</span>
                  </div>
                  <div className="mt-2 grid gap-1">
                    <PreviewLine label={t('models.originalProvider', 'Original provider')} value={item.original_provider ?? '-'} />
                    <PreviewLine label={t('table.modelId', 'Model selector')} value={item.source_model_id} />
                    <PreviewLine label={t('nick.publishedId', 'Published id')} value={item.published_id} />
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <EmptyView />
          )}
        </div>
      </section>
    </div>
  )
}

function buildNickRuleDefaults(data: ProviderCloudSeed, providerKey: string, originalProvider: string): NickRuleInput {
  return {
    nick_key: `nick-rule-${String(data.model_nicks.length + 1).padStart(4, '0')}`,
    provider_key: providerKey,
    original_provider: originalProvider,
    model_id: '*',
    nick: `${originalProvider}/{model}`,
    selector_type: 'pattern',
    priority: data.model_nicks.length + 10,
  }
}

function buildAliasDefaults(providerKey: string, originalProvider: string): OriginProviderAliasInput {
  const alias = originalProvider || 'provider'
  return {
    alias_key: `${providerKey}-origin-alias-${safeKeyPart(alias)}`,
    provider_key: providerKey,
    alias,
    driver: alias,
  }
}

function buildMappingDefaults(data: ProviderCloudSeed, providerKey: string): OriginMappingRuleInput {
  return {
    mapping_key: `origin-mapping-${String(data.origin_mapping_rules.length + 1).padStart(4, '0')}`,
    provider_key: providerKey,
    mapping_mode: 'template',
    match_pattern: '*/*',
    origin_template: '<driver>/<model>',
    regex: '^(?<driver>[^/]+)/(?<model>.+)$',
    driver_transforms: ['alias'],
    model_transforms: ['trim'],
    priority: data.origin_mapping_rules.length + 10,
  }
}

function buildNickRewritePreviewSections(data: ProviderCloudSeed, rule: NickRuleInput | ModelNickRecord | null, t: (key: string, fallback: string) => string) {
  const items = rule ? buildNickRewritePreviewItems(data, rule) : []
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
}

function buildNickRewritePreviewItems(data: ProviderCloudSeed, rule: NickRuleInput | ModelNickRecord): NickRewritePreviewItem[] {
  if (!data.providers.some((item) => item.provider_key === rule.provider_key)) {
    return []
  }
  const modelItems = data.model_param_rules
    .filter((item) => item.enabled)
    .flatMap((item): NickRewritePreviewItem[] => {
      const sourceModelId = item.match_type === 'default' ? '*' : item.model_id_selector ?? ''
      if (!sourceModelId || !nickRuleMatches(rule, sourceModelId, item.original_provider)) {
        return []
      }
      return [{
        target: previewTargetFromMatchType(item.match_type),
        source_model_id: sourceModelId,
        original_provider: item.original_provider,
        published_id: applyModelTemplate(rule.nick, sourceModelId),
        source_key: item.rule_key,
      }]
    })
  const variantItems = data.metadata_variants
    .filter((item) => item.enabled)
    .flatMap((item): NickRewritePreviewItem[] => {
      const sourceModelId = item.model_id_selector || '*'
      if (!nickRuleMatches(rule, sourceModelId, item.original_provider)) {
        return []
      }
      return [{
        target: 'variant',
        source_model_id: sourceModelId,
        original_provider: item.original_provider,
        published_id: applyModelTemplate(rule.nick, sourceModelId),
        source_key: item.variant_key,
      }]
    })
  const versionRuleItems = data.metadata_version_rules
    .filter((item) => item.enabled)
    .flatMap((item): NickRewritePreviewItem[] => {
      const sourceModelId = typeof item.content.model_pattern === 'string' && item.content.model_pattern.trim()
        ? item.content.model_pattern.trim()
        : item.model_id_selector || '*'
      if (!nickRuleMatches(rule, sourceModelId, item.original_provider)) {
        return []
      }
      return [{
        target: 'version_rule',
        source_model_id: sourceModelId,
        original_provider: item.original_provider,
        published_id: applyModelTemplate(rule.nick, sourceModelId),
        source_key: item.version_rule_key,
      }]
    })
  return [...modelItems, ...variantItems, ...versionRuleItems]
}

function nickRuleMatches(rule: NickRuleInput | ModelNickRecord, sourceModelId: string, originalProvider: string | null) {
  const originMatch = !rule.original_provider || rule.original_provider === originalProvider
  const selectorMatch = rule.selector_type === 'exact'
    ? rule.model_id === sourceModelId
    : wildcardMatch(rule.model_id, sourceModelId)
  return originMatch && selectorMatch
}

function previewTargetFromMatchType(matchType: MatchType): NickRewritePreviewTarget {
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
  if (pattern === value) {
    return true
  }
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*')
  return new RegExp(`^${escaped}$`).test(value)
}

function applyModelTemplate(template: string, modelId: string) {
  return template.replace(/<model>|\{model\}/g, modelId)
}

function safeKeyPart(value: string) {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '') || 'alias'
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

function Field({ label, error, children }: { label: string; error?: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
      {label}
      {children}
      {error && <span className="text-[color:var(--cp-danger)]">{error}</span>}
    </label>
  )
}

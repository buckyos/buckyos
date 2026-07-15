import { useCallback, useEffect, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { Pencil, Plus, SlidersHorizontal, Trash2 } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { EmptyView } from '../../components/empty-state/StateView'
import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { TableFilterForm } from '../../components/forms/TableFilterForm'
import { LogicalMountTreePicker } from '../../components/forms/LogicalMountTreePicker'
import { StatusBadge } from '../../components/status/StatusBadge'
import { buildOpsModelPreview, filterModelRules, getDictionaryKeys, getMaterializedLogicalDirectories, getOpsOverlay, getOpsPatchValue, getOriginalProviders, previewResolverHits } from '../../datamodel/selectors'
import type { DictionaryItem, MatchType, ModelParamRuleRecord } from '../../datamodel/types'
import { modelOpsInputSchema, modelRuleInputSchema, type ModelOpsInput, type ModelRuleInput, type TableFilterInput } from '../../datamodel/schemas'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { paginate, useShellContext } from '../pageUtils'

const pageSize = 10
const matchTypes: MatchType[] = ['exact', 'pattern', 'default']

export function ModelsPage() {
  const { serviceRole } = useProviderMetadataStore()
  return serviceRole === 'ops' ? <OpsModelsPage /> : <TechModelsPage />
}

function TechModelsPage() {
  const { t } = useI18n()
  const { workspace, viewMode, upsertModelRule, removeModelRule } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [filters, setFilters] = useState<TableFilterInput>({ search: '', providerKey: '', apiType: '', capability: '' })
  const [activeType, setActiveType] = useState<MatchType>('exact')
  const [page, setPage] = useState(1)
  const [formOpen, setFormOpen] = useState(false)
  const [formSessionKey, setFormSessionKey] = useState('create-exact')
  const [selectedRule, setSelectedRule] = useState<ModelParamRuleRecord | null>(null)
  const rules = useMemo(() => filterModelRules(data, filters).filter((rule) => rule.match_type === activeType), [activeType, data, filters])
  const paged = paginate(rules, page, pageSize)
  const apiTypes = useMemo(() => getDictionaryKeys(data, 'api_type'), [data])
  const capabilityItems = useMemo(() => data.dictionaries.filter((item) => item.kind === 'capability'), [data])
  const groupedCapabilityItems = useMemo(() => groupCapabilityItems(capabilityItems), [capabilityItems])
  const capabilities = useMemo(() => capabilityItems.map((item) => item.key), [capabilityItems])
  const logicalDirectories = useMemo(() => getMaterializedLogicalDirectories(data), [data])
  const logicalMounts = useMemo(() => logicalDirectories.map((directory) => directory.path), [logicalDirectories])
  const originalProviders = useMemo(() => getOriginalProviders(data), [data])
  const form = useForm<ModelRuleInput>({
    resolver: zodResolver(modelRuleInputSchema),
    defaultValues: {
      rule_key: 'openrouter-gpt-4o',
      match_type: 'exact',
      provider_key: data.providers[0]?.provider_key ?? '',
      original_provider: originalProviders[0] ?? 'openai',
      model_id_selector: 'gpt-4o',
      priority: 20,
      model_driver: 'openai-compatible',
      api_types: apiTypes.includes('llm') ? ['llm'] : apiTypes.slice(0, 1),
      capabilities: capabilities.includes('streaming') ? ['streaming'] : capabilities.slice(0, 1),
      capability_values: {},
      max_context_tokens: undefined,
      estimated_cost_usd: undefined,
      estimated_latency_ms: undefined,
      quality_score: undefined,
      latency_class: undefined,
      cost_class: undefined,
      logical_mounts: logicalMounts.includes('/llm') ? ['/llm'] : logicalMounts.slice(0, 1),
      scope: 'provider',
      exclude: false,
    },
  })
  const watchedScope = form.watch('scope')
  const watchedOriginalProvider = form.watch('original_provider')
  const watchedRule = form.watch()
  const previewRule = useMemo(() => toPreviewModelRule(watchedRule), [watchedRule])
  const previewHits = useMemo(() => previewResolverHits(data, previewRule), [data, previewRule])
  const watchedApiTypes = form.watch('api_types') ?? []
  const watchedCapabilities = form.watch('capabilities') ?? []
  const watchedCapabilityValues = form.watch('capability_values') ?? {}
  const watchedLogicalMounts = form.watch('logical_mounts') ?? []
  const watchedExclude = form.watch('exclude')
  const watchedEstimatedCostUsd = form.watch('estimated_cost_usd')
  const watchedEstimatedLatencyMs = form.watch('estimated_latency_ms')
  const watchedQualityScore = form.watch('quality_score')
  const watchedLatencyClass = form.watch('latency_class')
  const watchedCostClass = form.watch('cost_class')
  const isEditingRule = Boolean(selectedRule && formOpen)
  const affectedProviders = useMemo(() => {
    if (watchedScope === 'provider') {
      return data.providers.filter((provider) => provider.provider_key === form.getValues('provider_key'))
    }
    return data.providers.filter((provider) => {
      return data.provider_model_rules.some((rule) => rule.provider_key === provider.provider_key && rule.selector === watchedOriginalProvider)
    })
  }, [data, form, watchedOriginalProvider, watchedScope])
  const deleteAffectedProviders = useMemo(() => {
    if (!selectedRule) {
      return []
    }
    if (selectedRule.provider_key) {
      return data.providers.filter((provider) => provider.provider_key === selectedRule.provider_key)
    }
    return data.providers.filter((provider) => {
      return data.provider_model_rules.some((rule) => {
        return rule.provider_key === provider.provider_key && rule.enabled && (
          rule.selector === selectedRule.original_provider ||
          rule.selector === selectedRule.model_id_selector
        )
      })
    })
  }, [data, selectedRule])

  const columns = useMemo<Array<DataTableColumn<ModelParamRuleRecord>>>(() => [
    { key: 'selector', title: t('table.modelId', 'Model selector'), render: (rule) => <span className="font-mono text-xs">{rule.model_id_selector ?? 'defaults'}</span> },
    { key: 'match', title: t('table.matchType', 'Match'), render: (rule) => <StatusBadge tone={rule.match_type === 'default' ? 'warning' : 'accent'}>{rule.match_type}</StatusBadge> },
    { key: 'exclude', title: t('models.exclude', 'Exclude'), render: (rule) => <StatusBadge tone={rule.exclude ? 'danger' : 'success'}>{rule.exclude ? t('yes', 'Yes') : t('no', 'No')}</StatusBadge> },
    { key: 'provider', title: t('table.provider', 'Provider'), render: (rule) => rule.provider_key ?? rule.original_provider ?? 'global' },
    { key: 'api', title: t('table.apiTypes', 'API types'), render: (rule) => rule.exclude ? '-' : rule.api_types.slice(0, 3).join(', ') || '-' },
    { key: 'caps', title: t('table.capabilities', 'Capabilities'), render: (rule) => rule.exclude ? '-' : Object.keys(rule.capabilities).slice(0, 4).join(', ') || '-' },
    { key: 'enabled', title: t('table.enabled', 'Enabled'), render: (rule) => <StatusBadge tone={rule.enabled ? 'success' : 'danger'}>{rule.enabled ? t('yes', 'Yes') : t('no', 'No')}</StatusBadge> },
  ], [t])

  const toggleArrayField = (field: 'api_types' | 'capabilities' | 'logical_mounts', item: string) => {
    const current = form.getValues(field) ?? []
    const next = current.includes(item) ? current.filter((value) => value !== item) : [...current, item]
    form.setValue(field, next, {
      shouldDirty: true,
      shouldValidate: true,
    })
    if (field === 'capabilities') {
      const dictionary = capabilityItems.find((capability) => capability.key === item)
      const capabilityValues = { ...(form.getValues('capability_values') ?? {}) }
      if (next.includes(item)) {
        capabilityValues[item] = dictionary?.value_type === 'number' ? getDefaultCapabilityNumber(item) : true
      } else {
        delete capabilityValues[item]
      }
      form.setValue('capability_values', capabilityValues, { shouldDirty: true, shouldValidate: true })
    }
  }

  const setOptionalNumber = (field: 'estimated_cost_usd' | 'estimated_latency_ms' | 'quality_score', value: string) => {
    form.setValue(field, value ? Number(value) : undefined, {
      shouldDirty: true,
      shouldValidate: true,
    })
  }

  const setCapabilityNumber = (capability: string, value: string) => {
    const nextValue = value ? Number(value) : 0
    form.setValue('capability_values', {
      ...(form.getValues('capability_values') ?? {}),
      [capability]: nextValue,
    }, { shouldDirty: true, shouldValidate: true })
  }

  const handleSelect = useCallback((rule: ModelParamRuleRecord) => {
    setSelectedRule(rule)
    setInspector({
      title: rule.model_id_selector ?? `${rule.original_provider} defaults`,
      subtitle: `${rule.match_type} · ${rule.rule_key}`,
      status: rule.enabled ? 'enabled' : 'disabled',
      json: rule,
    })
  }, [setInspector])

  const openCreateForm = useCallback(() => {
    const input = createModelRuleInput({
      providerKey: data.providers[0]?.provider_key ?? '',
      originalProvider: originalProviders[0] ?? 'openai',
      apiTypes,
      capabilities,
      logicalMounts,
    })
    setSelectedRule(null)
    setActiveType(input.match_type)
    setFormSessionKey(`create-${input.match_type}`)
    form.reset(input)
    setFormOpen(true)
  }, [apiTypes, capabilities, data.providers, form, logicalMounts, originalProviders])

  const openEditForm = useCallback((rule: ModelParamRuleRecord) => {
    const input = modelRuleToInput(rule)
    setSelectedRule(rule)
    setActiveType(input.match_type)
    setFormSessionKey(`edit-${rule.rule_key}-${rule.updated_at}`)
    form.reset(input)
    setFormOpen(true)
  }, [form])

  useEffect(() => {
    if (!formOpen || isEditingRule) {
      return
    }
    const nextRuleKey = buildModelRuleKey(form.getValues(), data.model_param_rules)
    if (nextRuleKey !== form.getValues('rule_key')) {
      form.setValue('rule_key', nextRuleKey, { shouldDirty: true, shouldValidate: true })
    }
  }, [
    activeType,
    data.model_param_rules,
    form,
    formOpen,
    isEditingRule,
    watchedRule.model_id_selector,
    watchedRule.original_provider,
    watchedRule.provider_key,
    watchedRule.scope,
  ])

  return (
    <div className="space-y-4" data-testid="models-page">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('models.title', 'Models')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')}</p>
        </div>
        <div className="mobile-readonly-hide flex items-center gap-2">
          <button className="inline-flex h-10 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={openCreateForm}>
            <Plus size={16} />
            {t('models.createRule', 'Create model rule')}
          </button>
          <StatusBadge tone="accent">{rules.length}</StatusBadge>
        </div>
      </header>
      <section className="shell-card p-3">
        <div className="flex flex-wrap gap-2">
          {matchTypes.map((type) => (
            <button
              className={`h-9 rounded-md px-3 text-sm font-semibold ${activeType === type ? 'bg-[color:var(--cp-accent)] text-white' : 'border border-[color:var(--cp-border)]'}`}
              key={type}
              onClick={() => {
                setActiveType(type)
                setPage(1)
                form.setValue('match_type', type, { shouldDirty: true, shouldValidate: true })
                if (type === 'pattern') {
                  form.setValue('model_id_selector', 'gpt-*', { shouldDirty: true, shouldValidate: true })
                }
                if (type === 'exact') {
                  form.setValue('model_id_selector', 'gpt-4o', { shouldDirty: true, shouldValidate: true })
                }
              }}
              type="button"
            >
              {type}
            </button>
          ))}
        </div>
      </section>
      {formOpen && (
        <form
          key={formSessionKey}
          className="mobile-readonly-hide shell-card grid gap-3 p-4 lg:grid-cols-5"
          onSubmit={form.handleSubmit(async (value) => {
            await upsertModelRule({ ...value, match_type: activeType })
            setFormOpen(false)
          })}
        >
          <Field label={t('models.ruleKey', 'Rule key')} error={form.formState.errors.rule_key?.message}>
            <input className={`${inputClass} bg-[color:var(--cp-surface-2)] font-mono text-[color:var(--cp-muted)]`} readOnly {...form.register('rule_key')} />
          </Field>
          <Field label={t('table.matchType', 'Match')}>
            <select className={inputClass} value={activeType} onChange={(event) => {
              const next = event.target.value as MatchType
              setActiveType(next)
              setPage(1)
              form.setValue('match_type', next, { shouldDirty: true, shouldValidate: true })
            }}>
              {matchTypes.map((type) => <option key={type} value={type}>{type}</option>)}
            </select>
          </Field>
          <Field label={t('models.scope', 'Scope')}>
            <select className={inputClass} {...form.register('scope')}>
              <option value="global">{t('models.global', 'Global / origin')}</option>
              <option value="provider">{t('models.providerOverride', 'Provider override')}</option>
            </select>
          </Field>
          <Field label={t('filter.provider', 'Provider')}>
            <select className={inputClass} {...form.register('provider_key')}>
              {data.providers.map((provider) => (
                <option key={provider.provider_key} value={provider.provider_key}>{provider.name}</option>
              ))}
            </select>
          </Field>
          <Field label={t('models.originalProvider', 'Original provider')}>
            <select className={inputClass} {...form.register('original_provider')}>
              {originalProviders.map((provider) => (
                <option key={provider} value={provider}>{provider}</option>
              ))}
            </select>
          </Field>
          {activeType !== 'default' && (
            <Field label={t('table.modelId', 'Model selector')} error={form.formState.errors.model_id_selector?.message}>
              <input className={inputClass} {...form.register('model_id_selector')} />
            </Field>
          )}
          {activeType === 'pattern' && (
            <Field label={t('rules.priority', 'Priority')}>
              <input className={inputClass} type="number" {...form.register('priority', { valueAsNumber: true })} />
            </Field>
          )}
          <Field label={t('table.driver', 'Driver')}>
            <input className={inputClass} {...form.register('model_driver')} />
          </Field>
          <ParamPanel title={t('wizard.editModelRule', 'Edit selected model rule')}>
            <div className="grid gap-3 md:grid-cols-3">
              <Field label={t('models.estimatedCost', 'Estimated cost USD')}>
                <input className={inputClass} min="0" step="0.000001" type="number" value={watchedEstimatedCostUsd ?? ''} onChange={(event) => setOptionalNumber('estimated_cost_usd', event.target.value)} />
              </Field>
              <Field label={t('models.estimatedLatency', 'Estimated latency ms')}>
                <input className={inputClass} min="0" step="1" type="number" value={watchedEstimatedLatencyMs ?? ''} onChange={(event) => setOptionalNumber('estimated_latency_ms', event.target.value)} />
              </Field>
              <Field label={t('models.qualityScore', 'Quality score')}>
                <input className={inputClass} max="1" min="0" step="0.01" type="number" value={watchedQualityScore ?? ''} onChange={(event) => setOptionalNumber('quality_score', event.target.value)} />
              </Field>
              <Field label={t('ops.latencyClass', 'Latency class')}>
                <select className={inputClass} value={watchedLatencyClass ?? ''} onChange={(event) => form.setValue('latency_class', event.target.value ? event.target.value as ModelRuleInput['latency_class'] : undefined, { shouldDirty: true, shouldValidate: true })}>
                  <option value="">{t('wizard.mixedValue', 'Mixed values')}</option>
                  <option value="fast">fast</option>
                  <option value="normal">normal</option>
                  <option value="slow">slow</option>
                </select>
              </Field>
              <Field label={t('ops.costClass', 'Cost class')}>
                <select className={inputClass} value={watchedCostClass ?? ''} onChange={(event) => form.setValue('cost_class', event.target.value ? event.target.value as ModelRuleInput['cost_class'] : undefined, { shouldDirty: true, shouldValidate: true })}>
                  <option value="">{t('wizard.mixedValue', 'Mixed values')}</option>
                  <option value="low">low</option>
                  <option value="medium">medium</option>
                  <option value="high">high</option>
                </select>
              </Field>
            </div>
            {!watchedExclude ? (
              <div className="mt-3 grid gap-3 lg:grid-cols-2">
                <TokenPanel title={t('filter.apiType', 'API type')}>
                  {apiTypes.map((apiType) => (
                    <ChoiceButton checked={watchedApiTypes.includes(apiType)} key={apiType} label={apiType} onClick={() => toggleArrayField('api_types', apiType)} />
                  ))}
                </TokenPanel>
                <TokenPanel title={t('filter.capability', 'Capability')}>
                  {groupedCapabilityItems.map((capability) => (
                    <CapabilityControl
                      checked={watchedCapabilities.includes(capability.key)}
                      item={capability}
                      key={capability.key}
                      onChangeNumber={(value) => setCapabilityNumber(capability.key, value)}
                      onToggle={() => toggleArrayField('capabilities', capability.key)}
                      value={watchedCapabilityValues[capability.key]}
                    />
                  ))}
                </TokenPanel>
              </div>
            ) : (
              <p className="mt-3 text-xs text-[color:var(--cp-muted)]">{t('models.excludeKeepsFields', 'Exclude is active; other fields are kept for restore but ignored by publish.')}</p>
            )}
          </ParamPanel>
          <ParamPanel title={t('wizard.logicalMounts', 'Logical mounts')}>
            <p className="mb-2 text-xs text-[color:var(--cp-muted)]">{t('wizard.logicalMountHint', 'Full means every selected target will include the path. Partial means only some selected targets currently include it and remains unchanged until clicked.')}</p>
            <LogicalMountTreePicker directories={logicalDirectories} selected={watchedLogicalMounts} onToggle={(mount) => toggleArrayField('logical_mounts', mount)} />
          </ParamPanel>
          {activeType !== 'default' && (
            <label className="flex items-center gap-2 self-end text-sm font-semibold">
              <input className="h-4 w-4" type="checkbox" {...form.register('exclude')} />
              {t('models.exclude', 'Exclude')}
            </label>
          )}
          <div className="rounded-md border border-[color:var(--cp-border)] p-3 text-xs text-[color:var(--cp-muted)]">
            <div className="font-semibold text-[color:var(--cp-text)]">{t('models.affectedProviders', 'Affected providers')}</div>
            <div className="mt-2">{affectedProviders.map((provider) => provider.name).join(', ') || t('state.empty', 'No records match the current filters')}</div>
          </div>
          <div className="rounded-md border border-[color:var(--cp-border)] p-3 text-xs text-[color:var(--cp-muted)] lg:col-span-2">
            <div className="font-semibold text-[color:var(--cp-text)]">{t('resolver.hitPreview', 'Hit preview')}</div>
            <div className="mt-2">{previewHits.slice(0, 6).map((rule) => rule.model_id_selector).join(', ') || t('state.empty', 'No records match the current filters')}</div>
          </div>
          <div className="flex items-end justify-end gap-2 lg:col-span-5">
            <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" type="button" onClick={() => setFormOpen(false)}>{t('action.discard', 'Discard')}</button>
            <button className="h-10 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" type="submit">{t('action.saveDraft', 'Save draft')}</button>
          </div>
        </form>
      )}
      {selectedRule && viewMode === 'edit' && (
        <section className="mobile-readonly-hide shell-card grid gap-3 p-4 lg:grid-cols-[minmax(0,1fr)_auto]">
          <div>
            <h2 className="text-sm font-bold">{t('models.deleteImpact', 'Delete impact')}</h2>
            <p className="mt-1 text-sm text-[color:var(--cp-muted)]">
              {selectedRule.rule_key} · {deleteAffectedProviders.map((provider) => provider.name).join(', ') || t('state.empty', 'No records match the current filters')}
            </p>
          </div>
          <button
            className="inline-flex h-10 items-center justify-center gap-2 rounded-md border border-[color:var(--cp-danger)] px-3 text-sm font-semibold text-[color:var(--cp-danger)]"
            onClick={async () => {
              await removeModelRule({ rule_key: selectedRule.rule_key })
              setSelectedRule(null)
            }}
            type="button"
          >
            <Trash2 size={16} />
            {selectedRule.match_type === 'exact'
              ? t('models.deleteExactRule', 'Delete exact rule')
              : selectedRule.match_type === 'pattern'
                ? t('models.deletePatternRule', 'Delete pattern rule')
                : t('models.deleteDefaultRule', 'Delete default rule')}
          </button>
        </section>
      )}
      <div className="shell-card p-4">
        <TableFilterForm
          apiTypes={apiTypes}
          capabilities={capabilities}
          providers={data.providers.map((provider) => ({ key: provider.provider_key, label: provider.name }))}
          onChange={(value) => {
            setFilters(value)
            setPage(1)
          }}
        />
      </div>
      {paged.rows.length ? (
        <DataTable
          actions={viewMode === 'edit' ? (rule) => <div className="flex gap-1"><button aria-label={`Edit ${rule.rule_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)]" onClick={() => openEditForm(rule)} type="button"><Pencil size={14} /></button><button aria-label={`Delete ${rule.rule_key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-danger)] text-[color:var(--cp-danger)]" onClick={() => void removeModelRule({ rule_key: rule.rule_key })} type="button"><Trash2 size={14} /></button></div> : undefined}
          columns={columns}
          onSelect={handleSelect}
          rowKey={(rule) => rule.rule_key}
          rows={paged.rows}
        />
      ) : (
        <EmptyView />
      )}
      <div className="flex items-center justify-end gap-2 text-sm text-[color:var(--cp-muted)]">
        <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-1.5" disabled={paged.page <= 1} onClick={() => setPage(paged.page - 1)}>-</button>
        {t('pager.page', 'Page {{page}} of {{pages}}', { page: paged.page, pages: paged.totalPages })}
        <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-1.5" disabled={paged.page >= paged.totalPages} onClick={() => setPage(paged.page + 1)}>+</button>
      </div>
    </div>
  )
}

function OpsModelsPage() {
  const { t } = useI18n()
  const { workspace, viewMode, upsertModelOps } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [filters, setFilters] = useState<TableFilterInput>({ search: '', providerKey: '', apiType: '', capability: '' })
  const [activeType, setActiveType] = useState<MatchType>('exact')
  const [page, setPage] = useState(1)
  const [formOpen, setFormOpen] = useState(false)
  const rules = useMemo(() => filterModelRules(data, filters).filter((rule) => rule.match_type === activeType), [activeType, data, filters])
  const paged = paginate(rules, page, pageSize)
  const apiTypes = useMemo(() => getDictionaryKeys(data, 'api_type'), [data])
  const capabilities = useMemo(() => getDictionaryKeys(data, 'capability'), [data])
  const form = useForm<ModelOpsInput>({
    resolver: zodResolver(modelOpsInputSchema),
    defaultValues: {
      rule_key: data.model_param_rules[0]?.rule_key ?? '',
      pricing_input: 0,
      pricing_output: 0,
      routing_weight: 50,
      cost_class: 'medium',
      latency_class: 'normal',
      quality_score: 80,
      recommendation_level: 'standard',
      display_priority: 100,
      rollout_strategy: 'stable',
      ops_note: '',
    },
  })

  const columns = useMemo<Array<DataTableColumn<ModelParamRuleRecord>>>(() => [
    { key: 'selector', title: t('table.modelId', 'Model selector'), render: (rule) => <span className="font-mono text-xs">{rule.model_id_selector ?? 'defaults'}</span> },
    { key: 'match', title: t('table.matchType', 'Match'), render: (rule) => <StatusBadge tone={rule.match_type === 'default' ? 'warning' : 'accent'}>{rule.match_type}</StatusBadge> },
    { key: 'provider', title: t('table.provider', 'Provider'), render: (rule) => rule.provider_key ?? rule.original_provider ?? 'global' },
    { key: 'api', title: t('table.apiTypes', 'API types'), render: (rule) => rule.api_types.slice(0, 3).join(', ') || '-' },
    {
      key: 'visible',
      title: t('ops.clientVisible', 'Client visible'),
      render: (rule) => {
        const preview = buildOpsModelPreview(data, rule)
        return <StatusBadge tone={preview.visible ? 'success' : 'danger'}>{preview.visible ? t('yes', 'Yes') : t('no', 'No')}</StatusBadge>
      },
    },
    { key: 'weight', title: t('ops.routingWeight', 'Routing weight'), render: (rule) => buildOpsModelPreview(data, rule).routing_weight },
    { key: 'recommendation', title: t('ops.recommendation', 'Recommendation'), render: (rule) => buildOpsModelPreview(data, rule).recommendation_level },
  ], [data, t])

  const handleSelect = useCallback((rule: ModelParamRuleRecord) => {
    const overlay = getOpsOverlay(data, 'model_param_rule', rule.rule_key)
    const pricing = getOpsPatchValue<{ input: number; output: number } | null>(overlay, 'pricing_override', null)
    form.reset({
      rule_key: rule.rule_key,
      pricing_input: pricing?.input ?? 0,
      pricing_output: pricing?.output ?? 0,
      routing_weight: getOpsPatchValue(overlay, 'routing_weight', 50),
      cost_class: getOpsPatchValue(overlay, 'cost_class', 'medium') as ModelOpsInput['cost_class'],
      latency_class: getOpsPatchValue(overlay, 'latency_class', 'normal') as ModelOpsInput['latency_class'],
      quality_score: getOpsPatchValue(overlay, 'quality_score', 80),
      recommendation_level: getOpsPatchValue(overlay, 'recommendation_level', 'standard') as ModelOpsInput['recommendation_level'],
      display_priority: getOpsPatchValue(overlay, 'display_priority', 100),
      rollout_strategy: getOpsPatchValue(overlay, 'rollout_strategy', 'stable') as ModelOpsInput['rollout_strategy'],
      ops_note: getOpsPatchValue(overlay, 'ops_note', ''),
    })
    setFormOpen(true)
    setInspector({
      title: rule.model_id_selector ?? rule.rule_key,
      subtitle: t('ops.techFieldsReadonly', 'Technical fields are read-only in operations mode'),
      status: rule.enabled ? 'visible' : 'disabled',
      json: {
        technical: rule,
        operations_overlay: overlay,
        client_preview: buildOpsModelPreview(data, rule),
      },
    })
  }, [data, form, setInspector, t])

  return (
    <div className="space-y-4" data-testid="ops-models-page">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('ops.modelsTitle', 'Operations Models')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')} · {t('ops.techFieldsReadonly', 'Technical fields are read-only in operations mode')}</p>
        </div>
        <StatusBadge tone="accent">{rules.length}</StatusBadge>
      </header>

      <section className="shell-card p-3">
        <div className="flex flex-wrap gap-2">
          {matchTypes.map((type) => (
            <button
              className={`h-9 rounded-md px-3 text-sm font-semibold ${activeType === type ? 'bg-[color:var(--cp-accent)] text-white' : 'border border-[color:var(--cp-border)]'}`}
              key={type}
              onClick={() => {
                setActiveType(type)
                setPage(1)
                setFormOpen(false)
              }}
              type="button"
            >
              {type}
            </button>
          ))}
        </div>
      </section>

      {formOpen && (
        <form
          className="mobile-readonly-hide shell-card grid gap-3 p-4 lg:grid-cols-5"
          onSubmit={form.handleSubmit(async (value) => {
            await upsertModelOps(value)
            setFormOpen(false)
          })}
        >
          <Field label={t('models.ruleKey', 'Rule key')}>
            <input className={`${inputClass} font-mono`} readOnly {...form.register('rule_key')} />
          </Field>
          <Field label={t('ops.pricingInput', 'Input price')}>
            <input className={inputClass} type="number" step="0.000001" {...form.register('pricing_input', { valueAsNumber: true })} />
          </Field>
          <Field label={t('ops.pricingOutput', 'Output price')}>
            <input className={inputClass} type="number" step="0.000001" {...form.register('pricing_output', { valueAsNumber: true })} />
          </Field>
          <Field label={t('ops.routingWeight', 'Routing weight')}>
            <input className={inputClass} type="number" {...form.register('routing_weight', { valueAsNumber: true })} />
          </Field>
          <Field label={t('ops.costClass', 'Cost class')}>
            <select className={inputClass} {...form.register('cost_class')}>
              <option value="low">low</option>
              <option value="medium">medium</option>
              <option value="high">high</option>
            </select>
          </Field>
          <Field label={t('ops.latencyClass', 'Latency class')}>
            <select className={inputClass} {...form.register('latency_class')}>
              <option value="fast">fast</option>
              <option value="normal">normal</option>
              <option value="slow">slow</option>
            </select>
          </Field>
          <Field label={t('ops.qualityScore', 'Quality score')}>
            <input className={inputClass} type="number" {...form.register('quality_score', { valueAsNumber: true })} />
          </Field>
          <Field label={t('ops.recommendation', 'Recommendation')}>
            <select className={inputClass} {...form.register('recommendation_level')}>
              <option value="featured">featured</option>
              <option value="preferred">preferred</option>
              <option value="standard">standard</option>
              <option value="limited">limited</option>
            </select>
          </Field>
          <Field label={t('ops.rolloutStrategy', 'Rollout strategy')}>
            <select className={inputClass} {...form.register('rollout_strategy')}>
              <option value="stable">stable</option>
              <option value="canary">canary</option>
              <option value="hold">hold</option>
            </select>
          </Field>
          <Field label={t('ops.displayPriority', 'Display priority')}>
            <input className={inputClass} type="number" {...form.register('display_priority', { valueAsNumber: true })} />
          </Field>
          <Field label={t('ops.note', 'Operations note')}>
            <input className={inputClass} {...form.register('ops_note')} />
          </Field>
          <div className="rounded-md border border-[color:var(--cp-border)] p-3 text-xs text-[color:var(--cp-muted)] lg:col-span-3">
            <div className="flex items-center gap-2 font-semibold text-[color:var(--cp-text)]"><SlidersHorizontal size={14} />{t('ops.clientPreview', 'Client visible preview')}</div>
            <div className="mt-2">{t('ops.modelPreviewHint', 'Technical metadata remains unchanged; operations overlay controls visibility, price, routing, and rollout fields.')}</div>
          </div>
          <div className="flex items-end justify-end gap-2 lg:col-span-5">
            <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" type="button" onClick={() => setFormOpen(false)}>{t('action.discard', 'Discard')}</button>
            <button className="h-10 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" type="submit">{t('action.saveDraft', 'Save draft')}</button>
          </div>
        </form>
      )}

      <div className="shell-card p-4">
        <TableFilterForm
          apiTypes={apiTypes}
          capabilities={capabilities}
          providers={data.providers.map((provider) => ({ key: provider.provider_key, label: provider.name }))}
          onChange={(value) => {
            setFilters(value)
            setPage(1)
          }}
        />
      </div>
      {paged.rows.length ? (
        <DataTable columns={columns} onSelect={handleSelect} rowKey={(rule) => rule.rule_key} rows={paged.rows} />
      ) : (
        <EmptyView />
      )}
      <div className="flex items-center justify-end gap-2 text-sm text-[color:var(--cp-muted)]">
        <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-1.5" disabled={paged.page <= 1} onClick={() => setPage(paged.page - 1)}>-</button>
        {t('pager.page', 'Page {{page}} of {{pages}}', { page: paged.page, pages: paged.totalPages })}
        <button className="rounded-md border border-[color:var(--cp-border)] px-3 py-1.5" disabled={paged.page >= paged.totalPages} onClick={() => setPage(paged.page + 1)}>+</button>
      </div>
    </div>
  )
}

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm text-[color:var(--cp-text)]'

function Field({ label, error, children }: { label: string; error?: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
      {label}
      {children}
      {error && <span className="text-[color:var(--cp-danger)]">{error}</span>}
    </label>
  )
}

function ParamPanel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-md border border-[color:var(--cp-border)] p-3 lg:col-span-5">
      <h2 className="mb-2 text-xs font-semibold text-[color:var(--cp-muted)]">{title}</h2>
      {children}
    </section>
  )
}

function TokenPanel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="rounded-md border border-[color:var(--cp-border)] p-3">
      <h3 className="mb-2 text-xs font-semibold text-[color:var(--cp-muted)]">{title}</h3>
      <div className="flex max-h-40 flex-wrap gap-2 overflow-auto shell-scrollbar">{children}</div>
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

function CapabilityControl({ checked, item, onChangeNumber, onToggle, value }: { checked: boolean; item: DictionaryItem; onChangeNumber: (value: string) => void; onToggle: () => void; value: boolean | number | undefined }) {
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

function toPreviewModelRule(input: ModelRuleInput): ModelParamRuleRecord {
  return {
    rule_key: input.rule_key,
    provider_key: input.scope === 'provider' ? input.provider_key : null,
    source_rule_key: null,
    match_type: input.match_type,
    original_provider: input.original_provider || null,
    model_id_selector: input.match_type === 'default' ? null : input.model_id_selector,
    priority: input.match_type === 'pattern' ? input.priority : null,
    model_driver: input.model_driver,
    api_types: input.api_types,
    logical_mounts: input.logical_mounts,
    capabilities: buildInputCapabilities(input),
    attributes: {
      quality_score: input.quality_score ?? null,
      latency_class: input.latency_class ?? null,
      cost_class: input.cost_class ?? null,
    },
    context_limits: null,
    pricing: {
      estimated_cost_usd: input.estimated_cost_usd ?? null,
      estimated_latency_ms: input.estimated_latency_ms ?? null,
    },
    exclude: input.match_type === 'default' ? false : input.exclude,
    enabled: true,
    created_at: Date.now(),
    updated_at: Date.now(),
  }
}

function createModelRuleInput({
  providerKey,
  originalProvider,
  apiTypes,
  capabilities,
  logicalMounts,
}: {
  providerKey: string
  originalProvider: string
  apiTypes: string[]
  capabilities: string[]
  logicalMounts: string[]
}): ModelRuleInput {
  const selectedApiTypes = apiTypes.includes('llm') ? ['llm'] : apiTypes.slice(0, 1)
  const selectedCapabilities = capabilities.includes('streaming') ? ['streaming'] : capabilities.slice(0, 1)
  const input: ModelRuleInput = {
    rule_key: '',
    match_type: 'exact',
    provider_key: providerKey,
    original_provider: originalProvider,
    model_id_selector: 'gpt-4o',
    priority: 20,
    model_driver: 'openai-compatible',
    api_types: selectedApiTypes,
    capabilities: selectedCapabilities,
    capability_values: Object.fromEntries(selectedCapabilities.map((capability) => [capability, true])),
    max_context_tokens: undefined,
    estimated_cost_usd: undefined,
    estimated_latency_ms: undefined,
    quality_score: undefined,
    latency_class: undefined,
    cost_class: undefined,
    logical_mounts: logicalMounts.includes('/llm') ? ['/llm'] : logicalMounts.slice(0, 1),
    scope: 'provider',
    exclude: false,
  }
  return {
    ...input,
    rule_key: buildModelRuleKey(input, []),
  }
}

function buildModelRuleKey(input: ModelRuleInput, existingRules: ModelParamRuleRecord[]) {
  const owner = input.scope === 'provider' ? input.provider_key : input.original_provider || 'global'
  const selector = input.match_type === 'default' ? 'default' : input.model_id_selector || input.match_type
  const base = `model-${safeKeyPart(owner)}-${input.match_type}-${safeKeyPart(selector)}`
  const existingKeys = new Set(existingRules.map((rule) => rule.rule_key))
  if (!existingKeys.has(base)) {
    return base
  }
  for (let index = 2; index < 1000; index += 1) {
    const candidate = `${base}-${index}`
    if (!existingKeys.has(candidate)) {
      return candidate
    }
  }
  return `${base}-${Date.now()}`
}

function safeKeyPart(value: string) {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48) || 'rule'
}

function modelRuleToInput(rule: ModelParamRuleRecord): ModelRuleInput {
  const maxContextTokens = getRuleMaxContextTokens(rule)
  const capabilityValues: Record<string, boolean | number> = {}
  Object.entries(rule.capabilities).forEach(([key, value]) => {
    if (typeof value === 'boolean' || typeof value === 'number') {
      capabilityValues[key] = value
    }
  })
  return {
    rule_key: rule.rule_key,
    match_type: rule.match_type,
    provider_key: rule.provider_key ?? '',
    original_provider: rule.original_provider ?? '',
    model_id_selector: rule.model_id_selector ?? '',
    priority: rule.priority ?? 1,
    model_driver: rule.model_driver ?? '',
    api_types: rule.api_types,
    capabilities: Array.from(new Set([...Object.keys(rule.capabilities), ...(maxContextTokens ? ['max_context_tokens'] : [])])),
    capability_values: capabilityValues,
    max_context_tokens: maxContextTokens,
    estimated_cost_usd: getNumberValue(rule.pricing, 'estimated_cost_usd'),
    estimated_latency_ms: getNumberValue(rule.pricing, 'estimated_latency_ms'),
    quality_score: getNumberValue(rule.attributes, 'quality_score'),
    latency_class: getEnumValue(rule.attributes, 'latency_class', ['fast', 'normal', 'slow'] as const),
    cost_class: getEnumValue(rule.attributes, 'cost_class', ['low', 'medium', 'high'] as const),
    logical_mounts: rule.logical_mounts.map(toLogicalDirectoryPath),
    scope: rule.provider_key ? 'provider' : 'global',
    exclude: rule.exclude,
  }
}

function buildInputCapabilities(input: ModelRuleInput) {
  const capabilityValues = input.capability_values ?? {}
  return Object.fromEntries(input.capabilities.map((capability) => {
    const value = capabilityValues[capability]
    return [capability, typeof value === 'boolean' || typeof value === 'number' ? value : true]
  }))
}

function toLogicalDirectoryPath(mount: string) {
  const normalized = mount.trim().replace(/\./g, '/').replace(/^\/+|\/+$/g, '')
  return normalized ? `/${normalized}` : '/'
}

function getDefaultCapabilityNumber(_capability: string) {
  return 1
}

function getRuleMaxContextTokens(rule: ModelParamRuleRecord) {
  const contextLimit = getNumberValue(rule.context_limits, 'max_context_tokens')
  if (contextLimit !== undefined) {
    return contextLimit
  }
  return getNumberValue(rule.capabilities, 'max_context_tokens')
}

function getNumberValue(record: Record<string, unknown> | null, key: string) {
  const value = record?.[key]
  return typeof value === 'number' ? value : undefined
}

function getEnumValue<T extends string>(record: Record<string, unknown> | null, key: string, values: readonly T[]) {
  const value = record?.[key]
  return typeof value === 'string' && values.includes(value as T) ? value as T : undefined
}

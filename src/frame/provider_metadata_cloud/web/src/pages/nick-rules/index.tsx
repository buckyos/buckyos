import { useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { Pencil, Plus, Trash2 } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { EmptyView } from '../../components/empty-state/StateView'
import { StatusBadge } from '../../components/status/StatusBadge'
import { getOriginalProviders, getSourceModelIds } from '../../datamodel/selectors'
import { nickRuleInputSchema, type NickRuleInput } from '../../datamodel/schemas'
import type { MatchType, ModelNickRecord, ProviderCloudSeed } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { useShellContext } from '../pageUtils'

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm text-[color:var(--cp-text)]'

type NickRewritePreviewTarget = 'model' | 'pattern' | 'default' | 'variant' | 'version_rule'

interface NickRewritePreviewItem {
  target: NickRewritePreviewTarget
  source_model_id: string
  original_provider: string | null
  published_id: string
  source_key: string
}

export function NickRulesPage() {
  const { t } = useI18n()
  const { workspace, upsertNickRule, removeNickRule, viewMode } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [formOpen, setFormOpen] = useState(false)
  const [editingRule, setEditingRule] = useState(false)
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
  const formValues = form.watch()
  const visibleRules = useMemo(() => data.model_nicks.filter((rule) => !providerFilter || rule.provider_key === providerFilter), [data.model_nicks, providerFilter])
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

  return (
    <div className="space-y-4" data-testid="nick-rules-page">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('nick.title', 'Nick Rules')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')}</p>
        </div>
        <button className="inline-flex h-10 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={openCreateForm} type="button">
          <Plus size={16} />
          {t('nick.create', 'Create nick rule')}
        </button>
      </header>

      <section className="shell-card p-3">
        <select className={inputClass} value={providerFilter} onChange={(event) => setProviderFilter(event.target.value)}>
          <option value="">{t('filter.all', 'All')}</option>
          {data.providers.map((provider) => <option key={provider.provider_key} value={provider.provider_key}>{provider.name}</option>)}
        </select>
      </section>

      <section className="shell-card p-4">
        <h2 className="text-sm font-bold">{t('wizard.nickConcept', 'Nick rewrite role')}</h2>
        <p className="mt-2 text-sm text-[color:var(--cp-muted)]">{t('wizard.nickConceptHint', 'Nick rewrite is a publish-time intermediate mapping. It reuses selected original models, patterns, defaults, variants, and version rules while publishing the provider inventory without copied renamed rules.')}</p>
        <p className="mt-2 text-sm text-[color:var(--cp-muted)]">{t('wizard.nickScopeHint', 'Rules are ordered by priority and also rewrite variants and version rules. Variants use * when no model selector exists; version rules rewrite content.model_pattern.')}</p>
      </section>

      <section className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_380px]">
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

        <aside className="shell-card p-4">
          <div className="flex items-start justify-between gap-3">
            <div>
              <h2 className="text-sm font-bold">{t('nick.preview', 'Rewrite preview')}</h2>
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
              <div className="shell-scrollbar grid max-h-[34rem] gap-2 overflow-auto">
                {activePreviewSection.items.map((item) => (
                  <div className="rounded-md border border-[color:var(--cp-border)] p-2 text-xs" key={`${item.target}-${item.source_key}-${item.source_model_id}`}>
                    <div className="flex items-center justify-between gap-2">
                      <StatusBadge tone={nickRewritePreviewTone(item.target)}>{item.target}</StatusBadge>
                      <span className="truncate text-[color:var(--cp-muted)]">{item.original_provider ?? '-'}</span>
                    </div>
                    <div className="mt-1 break-all font-mono">{item.source_model_id}</div>
                    <div className="mt-1 break-all text-[color:var(--cp-muted)]">-&gt; {item.published_id}</div>
                  </div>
                ))}
              </div>
            ) : (
              <EmptyView />
            )}
          </div>
        </aside>
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
        published_id: rule.nick.replace('{model}', sourceModelId),
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
        published_id: rule.nick.replace('{model}', sourceModelId),
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
        published_id: rule.nick.replace('{model}', sourceModelId),
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

function Field({ label, error, children }: { label: string; error?: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
      {label}
      {children}
      {error && <span className="text-[color:var(--cp-danger)]">{error}</span>}
    </label>
  )
}

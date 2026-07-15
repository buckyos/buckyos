import { useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { BookMarked, Pencil, Plus, Tags, Trash2 } from 'lucide-react'
import { useForm } from 'react-hook-form'
import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { EmptyView } from '../../components/empty-state/StateView'
import { StatusBadge } from '../../components/status/StatusBadge'
import { filterModelRules, getDictionaryReferenceCount, getDictionarySamples } from '../../datamodel/selectors'
import {
  dictionaryBulkApplyInputSchema,
  dictionaryItemInputSchema,
  tableFilterSchema,
  type DictionaryBulkApplyInput,
  type DictionaryItemInput,
  type TableFilterInput,
} from '../../datamodel/schemas'
import type { DictionaryItem, ModelParamRuleRecord } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { useShellContext } from '../pageUtils'

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm text-[color:var(--cp-text)]'

export function DictionariesPage() {
  const { t } = useI18n()
  const { workspace, viewMode, upsertDictionaryItem, removeDictionaryItem, applyDictionaryTag } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [kind, setKind] = useState<DictionaryItem['kind']>('capability')
  const [formOpen, setFormOpen] = useState(false)
  const [editingDictionary, setEditingDictionary] = useState(false)
  const [filters, setFilters] = useState<TableFilterInput>({ search: '', providerKey: '', apiType: '', capability: '' })
  const items = data.dictionaries.filter((item) => item.kind === kind)
  const filteredModels = useMemo(() => filterModelRules(data, filters), [data, filters])
  const selectedDictionaryKey = items[0]?.key ?? ''
  const itemForm = useForm<DictionaryItemInput>({
    resolver: zodResolver(dictionaryItemInputSchema),
    defaultValues: {
      key: 'reasoning',
      label: 'Reasoning',
      kind,
      value_type: 'boolean',
    },
  })
  const bulkForm = useForm<DictionaryBulkApplyInput>({
    resolver: zodResolver(dictionaryBulkApplyInputSchema.refine((value) => {
      return data.dictionaries.some((item) => item.kind === value.kind && item.key === value.key)
    }, { message: t('dictionary.existingOnly', 'Select an existing dictionary item'), path: ['key'] })),
    defaultValues: {
      kind,
      key: selectedDictionaryKey,
      value_type: items.find((item) => item.key === selectedDictionaryKey)?.value_type ?? 'boolean',
      boolean_value: true,
      number_value: undefined,
      model_rule_keys: filteredModels.slice(0, 3).map((rule) => rule.rule_key),
    },
  })
  const watchedBulkKind = bulkForm.watch('kind')
  const watchedBulkKey = bulkForm.watch('key')
  const watchedBulkValueType = bulkForm.watch('value_type')
  const bulkItems = data.dictionaries.filter((item) => item.kind === watchedBulkKind)

  const startCreateDictionary = () => {
    setEditingDictionary(false)
    itemForm.reset({
      key: buildDictionaryKey('reasoning', kind, data.dictionaries),
      label: 'Reasoning',
      kind,
      value_type: 'boolean',
    })
    setFormOpen(true)
  }

  const startEditDictionary = (item: DictionaryItem) => {
    setEditingDictionary(true)
    itemForm.reset(item)
    setKind(item.kind)
    setFormOpen(true)
  }

  const setBulkDictionary = (nextKind: DictionaryItem['kind'], nextKey?: string) => {
    const nextItems = data.dictionaries.filter((item) => item.kind === nextKind)
    const nextDictionary = nextItems.find((item) => item.key === nextKey) ?? nextItems[0]
    bulkForm.setValue('kind', nextKind, { shouldDirty: true, shouldValidate: true })
    bulkForm.setValue('key', nextDictionary?.key ?? '', { shouldDirty: true, shouldValidate: true })
    bulkForm.setValue('value_type', nextDictionary?.value_type ?? 'boolean', { shouldDirty: true, shouldValidate: true })
    bulkForm.setValue('boolean_value', true, { shouldDirty: true, shouldValidate: true })
    if (nextDictionary?.value_type !== 'number') {
      bulkForm.setValue('number_value', undefined, { shouldDirty: true, shouldValidate: true })
    }
  }

  const columns = useMemo<Array<DataTableColumn<DictionaryItem>>>(() => [
    { key: 'key', title: t('dictionary.key', 'Dictionary key'), render: (item) => <span className="font-mono text-xs">{item.key}</span> },
    { key: 'label', title: t('dictionary.label', 'Label'), render: (item) => item.label },
    { key: 'kind', title: t('dictionary.kind', 'Kind'), render: (item) => <StatusBadge tone={item.kind === 'api_type' ? 'accent' : 'success'}>{item.kind}</StatusBadge> },
    { key: 'value', title: t('dictionary.valueType', 'Value type'), render: (item) => item.value_type },
    { key: 'refs', title: t('dictionary.references', 'References'), render: (item) => getDictionaryReferenceCount(data, item) },
  ], [data, t])

  const modelColumns = useMemo<Array<DataTableColumn<ModelParamRuleRecord>>>(() => [
    { key: 'key', title: t('rules.ruleKey', 'Rule key'), render: (rule) => <span className="font-mono text-xs">{rule.rule_key}</span> },
    { key: 'model', title: t('table.modelId', 'Model selector'), render: (rule) => <span className="font-mono text-xs">{rule.model_id_selector ?? '-'}</span> },
    { key: 'api', title: t('table.apiTypes', 'API types'), render: (rule) => rule.api_types.join(', ') },
    { key: 'capabilities', title: t('table.capabilities', 'Capabilities'), render: (rule) => Object.keys(rule.capabilities).join(', ') },
  ], [t])

  return (
    <div className="space-y-4" data-testid="dictionaries-page">
      <header className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('dictionary.title', 'Dictionaries')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{viewMode === 'edit' ? t('mode.edit', 'Edit') : t('mode.browse', 'Browse')}</p>
        </div>
        <button className="inline-flex h-10 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" onClick={startCreateDictionary}>
          <Plus size={16} />
          {t('dictionary.create', 'Create dictionary item')}
        </button>
      </header>

      <section className="shell-card flex flex-wrap gap-2 p-3">
        <button className={`h-9 rounded-md px-3 text-sm font-semibold ${kind === 'api_type' ? 'bg-[color:var(--cp-accent)] text-white' : 'border border-[color:var(--cp-border)]'}`} onClick={() => {
          setKind('api_type')
          itemForm.setValue('kind', 'api_type')
          setBulkDictionary('api_type')
        }} type="button">
          {t('dictionary.apiTypes', 'API types')}
        </button>
        <button className={`h-9 rounded-md px-3 text-sm font-semibold ${kind === 'capability' ? 'bg-[color:var(--cp-accent)] text-white' : 'border border-[color:var(--cp-border)]'}`} onClick={() => {
          setKind('capability')
          itemForm.setValue('kind', 'capability')
          setBulkDictionary('capability')
        }} type="button">
          {t('dictionary.capabilities', 'Capabilities')}
        </button>
      </section>

      {formOpen && (
        <form
          className="shell-card grid gap-3 p-4 md:grid-cols-5"
          onSubmit={itemForm.handleSubmit(async (value) => {
            const nextKind = value.kind
            await upsertDictionaryItem({
              ...value,
              key: editingDictionary ? value.key : buildDictionaryKey(value.label, nextKind, data.dictionaries),
              kind: nextKind,
            })
            setKind(nextKind)
            setFormOpen(false)
            setEditingDictionary(false)
          })}
        >
          <input type="hidden" {...itemForm.register('key')} />
          <Field label={t('dictionary.label', 'Label')}>
            <input className={inputClass} {...itemForm.register('label')} />
          </Field>
          <Field label={t('dictionary.kind', 'Kind')}>
            <select className={inputClass} {...itemForm.register('kind')}>
              <option value="api_type">{t('dictionary.apiTypes', 'API types')}</option>
              <option value="capability">{t('dictionary.capabilities', 'Capabilities')}</option>
            </select>
          </Field>
          <Field label={t('dictionary.valueType', 'Value type')}>
            <select className={inputClass} {...itemForm.register('value_type')}>
              <option value="boolean">{t('dictionary.booleanValue', 'Boolean')}</option>
              <option value="number">{t('dictionary.numberValue', 'Number')}</option>
            </select>
          </Field>
          <div className="flex items-end justify-end gap-2">
            <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" type="button" onClick={() => { setFormOpen(false); setEditingDictionary(false) }}>{t('action.discard', 'Discard')}</button>
            <button className="h-10 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" type="submit">{t('action.saveDraft', 'Save draft')}</button>
          </div>
        </form>
      )}

      <section className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_380px]">
        <div className="space-y-4">
          {items.length ? (
        <DataTable
          actions={viewMode === 'edit' ? (item) => <div className="flex gap-1"><button aria-label={`Edit ${item.key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-border)]" onClick={() => startEditDictionary(item)} type="button"><Pencil size={14} /></button><button aria-label={`Delete ${item.key}`} className="grid h-8 w-8 place-items-center rounded-md border border-[color:var(--cp-danger)] text-[color:var(--cp-danger)]" onClick={() => void removeDictionaryItem(item.kind, item.key)} type="button"><Trash2 size={14} /></button></div> : undefined}
              columns={columns}
              onSelect={(item) => {
                bulkForm.setValue('kind', item.kind)
                bulkForm.setValue('key', item.key)
                setInspector({
                  title: item.key,
                  subtitle: item.kind,
                  status: `${getDictionaryReferenceCount(data, item)} references`,
                  json: { ...item, samples: getDictionarySamples(data, item) },
                })
              }}
              rowKey={(item) => `${item.kind}-${item.key}`}
              rows={items}
            />
          ) : (
            <EmptyView />
          )}
          <section className="shell-card p-4">
            <h2 className="mb-3 flex items-center gap-2 text-sm font-bold"><BookMarked size={16} />{t('dictionary.matrix', 'Model coverage matrix')}</h2>
            <FilterPanel filters={filters} setFilters={setFilters} />
            <div className="mt-3">
              <DataTable columns={modelColumns} rowKey={(rule) => rule.rule_key} rows={filteredModels} />
            </div>
          </section>
        </div>

        <aside className="shell-card p-4">
          <h2 className="mb-3 flex items-center gap-2 text-sm font-bold"><Tags size={16} />{t('dictionary.bulkApply', 'Bulk apply to models')}</h2>
          <form
            className="space-y-3"
            onSubmit={bulkForm.handleSubmit(async (value) => applyDictionaryTag(value))}
          >
            <Field label={t('dictionary.kind', 'Kind')}>
              <select className={inputClass} value={kind} onChange={(event) => {
                const next = event.target.value as DictionaryItem['kind']
                setKind(next)
                setBulkDictionary(next)
              }}>
                <option value="api_type">{t('dictionary.apiTypes', 'API types')}</option>
                <option value="capability">{t('dictionary.capabilities', 'Capabilities')}</option>
              </select>
            </Field>
            <Field label={t('dictionary.key', 'Dictionary key')} error={bulkForm.formState.errors.key?.message}>
              <select className={inputClass} value={watchedBulkKey} onChange={(event) => setBulkDictionary(watchedBulkKind, event.target.value)}>
                {bulkItems.map((item) => <option key={item.key} value={item.key}>{item.key}</option>)}
              </select>
            </Field>
            {watchedBulkKind === 'capability' && (
              <Field label={t('dictionary.value', 'Value')} error={bulkForm.formState.errors.number_value?.message}>
                {watchedBulkValueType === 'number' ? (
                  <input className={inputClass} min="0" type="number" {...bulkForm.register('number_value', { valueAsNumber: true })} />
                ) : (
                  <select className={inputClass} value={bulkForm.watch('boolean_value') ? 'true' : 'false'} onChange={(event) => bulkForm.setValue('boolean_value', event.target.value === 'true', { shouldDirty: true, shouldValidate: true })}>
                    <option value="true">{t('dictionary.supported', 'Supported')}</option>
                    <option value="false">{t('dictionary.unsupported', 'Unsupported')}</option>
                  </select>
                )}
              </Field>
            )}
            <Field label={t('dictionary.targetModels', 'Target models')}>
              <select className="min-h-44 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 py-2 text-sm text-[color:var(--cp-text)]" multiple {...bulkForm.register('model_rule_keys')}>
                {filteredModels.map((rule) => <option key={rule.rule_key} value={rule.rule_key}>{rule.model_id_selector ?? rule.rule_key}</option>)}
              </select>
            </Field>
            <div className="rounded-md border border-[color:var(--cp-border)] p-3 text-xs text-[color:var(--cp-muted)]">
              {t('dictionary.existingOnly', 'Select an existing dictionary item')}. {t('dictionary.valueHint', 'Capability dictionaries use their configured boolean or number input when applied.')}
            </div>
            <button className="h-10 w-full rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-semibold text-white" type="submit">{t('dictionary.applyTag', 'Apply selected key')}</button>
          </form>
        </aside>
      </section>
    </div>
  )
}

function FilterPanel({ filters, setFilters }: { filters: TableFilterInput; setFilters: (value: TableFilterInput) => void }) {
  const { t } = useI18n()
  const form = useForm<TableFilterInput>({
    resolver: zodResolver(tableFilterSchema),
    defaultValues: filters,
  })
  return (
    <form className="grid gap-2 md:grid-cols-4" onSubmit={form.handleSubmit(setFilters)}>
      <input aria-label={t('filter.search', 'Search')} className={inputClass} {...form.register('search')} />
      <input aria-label={t('filter.provider', 'Provider')} className={inputClass} {...form.register('providerKey')} />
      <input aria-label={t('filter.apiType', 'API type')} className={inputClass} {...form.register('apiType')} />
      <button className="h-10 rounded-md border border-[color:var(--cp-border)] px-3 text-sm font-semibold" type="submit">{t('filter.search', 'Search')}</button>
    </form>
  )
}

function buildDictionaryKey(label: string, kind: DictionaryItem['kind'], items: DictionaryItem[]) {
  const base = label.trim().toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '') || kind.replace(/[^a-z0-9]+/g, '_')
  const existing = new Set(items.filter((item) => item.kind === kind).map((item) => item.key))
  if (!existing.has(base)) {
    return base
  }
  for (let index = 2; index < 1000; index += 1) {
    const candidate = `${base}_${index}`
    if (!existing.has(candidate)) {
      return candidate
    }
  }
  return `${base}_${Date.now()}`
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

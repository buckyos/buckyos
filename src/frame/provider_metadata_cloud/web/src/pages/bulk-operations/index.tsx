import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { ClipboardCheck, Filter, Percent, SlidersHorizontal } from 'lucide-react'
import { useForm, useWatch } from 'react-hook-form'
import { DataTable, type DataTableColumn } from '../../components/data-table/DataTable'
import { EmptyView } from '../../components/empty-state/StateView'
import { StatusBadge } from '../../components/status/StatusBadge'
import { getDictionaryKeys, getOriginalProviders, previewOpsBulkOperation } from '../../datamodel/selectors'
import { opsBulkOperationInputSchema, type OpsBulkOperationFormInput } from '../../datamodel/schemas'
import type { OpsBulkPreviewRow } from '../../datamodel/types'
import { useI18n } from '../../i18n/provider'
import { useProviderMetadataStore } from '../../state/useProviderMetadataStore'
import { useShellContext } from '../pageUtils'

const inputClass = 'h-10 w-full rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm'

const defaultValues: OpsBulkOperationFormInput = {
  provider_key: '',
  original_provider: '',
  model_id_pattern: '*',
  api_type: '',
  capability: '',
  recommendation_level: '',
  price_min: undefined,
  price_max: undefined,
  routing_weight_min: undefined,
  routing_weight_max: undefined,
  action: 'adjust_price_percent',
  target_recommendation_level: 'preferred',
  display_priority: 50,
  price_percent: 10,
  pricing_input: 0.000001,
  pricing_output: 0.000002,
  routing_weight: 60,
}
const parsedDefaultValues = opsBulkOperationInputSchema.parse(defaultValues)

export function BulkOperationsPage() {
  const { t } = useI18n()
  const { workspace, applyBulkOperation, enterEdit, serviceRole, setServiceRole } = useProviderMetadataStore()
  const { setInspector } = useShellContext()
  const data = workspace.data!
  const [applied, setApplied] = useState(false)
  const form = useForm<OpsBulkOperationFormInput>({
    resolver: zodResolver(opsBulkOperationInputSchema),
    defaultValues,
  })
  const values = useWatch({ control: form.control })
  const parsedValues = opsBulkOperationInputSchema.safeParse({ ...defaultValues, ...values })
  const previewInput = parsedValues.success ? parsedValues.data : parsedDefaultValues
  const preview = useMemo(() => previewOpsBulkOperation(data, previewInput), [data, previewInput])
  const providers = data.providers.filter((provider) => provider.provider_kind !== 'origin')
  const originalProviders = useMemo(() => getOriginalProviders(data), [data])
  const apiTypes = useMemo(() => getDictionaryKeys(data, 'api_type'), [data])
  const capabilities = useMemo(() => getDictionaryKeys(data, 'capability'), [data])
  const action = form.watch('action')

  useEffect(() => {
    if (serviceRole !== 'ops') {
      setServiceRole('ops')
    }
  }, [serviceRole, setServiceRole])

  useEffect(() => {
    const defaultOriginalProvider = originalProviders.includes('openai') ? 'openai' : originalProviders[0]
    if (!form.getValues('original_provider') && defaultOriginalProvider) {
      form.setValue('original_provider', defaultOriginalProvider, { shouldDirty: false, shouldValidate: true })
    }
  }, [form, originalProviders])

  useEffect(() => {
    setInspector({
      title: t('bulk.title', 'Bulk Operations'),
      subtitle: `${preview.hit_count} matched model rules`,
      status: preview.hit_count ? t('status.warning', 'Warning') : t('state.empty', 'No records match the current filters'),
      json: preview,
    })
  }, [
    preview.hit_count,
    preview.visibility_removed,
    preview.visibility_added,
    preview.price_changed,
    preview.routing_changed,
    preview.display_priority_changed,
    setInspector,
    t,
  ])

  const columns = useMemo<Array<DataTableColumn<OpsBulkPreviewRow>>>(() => [
    { key: 'model', title: t('table.modelId', 'Model selector'), render: (row) => <span className="font-mono text-xs">{row.model_id_selector ?? 'defaults'}</span> },
    { key: 'origin', title: t('models.originalProvider', 'Original provider'), render: (row) => row.original_provider ?? '-' },
    { key: 'api', title: t('table.apiTypes', 'API types'), render: (row) => row.api_types.slice(0, 2).join(', ') || '-' },
    {
      key: 'price',
      title: t('bulk.price', 'Price'),
      render: (row) => `${formatPrice(row.pricing_before)} to ${formatPrice(row.pricing_after)}`,
    },
    {
      key: 'routing',
      title: t('ops.routingWeight', 'Routing weight'),
      render: (row) => `${row.routing_weight_before} to ${row.routing_weight_after}`,
    },
  ], [t])

  async function handleSubmit(rawInput: OpsBulkOperationFormInput) {
    const input = opsBulkOperationInputSchema.parse(rawInput)
    if (serviceRole !== 'ops') {
      setServiceRole('ops')
    }
    if (!data.edit_session || data.edit_session.service_role !== 'ops') {
      await enterEdit()
    }
    await applyBulkOperation(input)
    setApplied(true)
  }

  return (
    <div className="space-y-4" data-testid="bulk-operations-page">
      <header className="flex items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t('bulk.title', 'Bulk Operations')}</h1>
          <p className="mt-1 text-sm text-[color:var(--cp-muted)]">{t('bulk.subtitle', 'Filter operations model rows, preview impact, then write mock operations overlays.')}</p>
        </div>
        <StatusBadge tone="accent">{preview.hit_count}</StatusBadge>
      </header>

      <form className="shell-card space-y-4 p-4" onSubmit={form.handleSubmit(handleSubmit)}>
        <div className="flex items-center gap-2 text-sm font-bold">
          <Filter size={16} className="text-[color:var(--cp-accent)]" />
          {t('bulk.filters', 'Filters')}
        </div>
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          <Field label={t('filter.provider', 'Provider')}>
            <select className={inputClass} {...form.register('provider_key')}>
              <option value="">{t('filter.all', 'All')}</option>
              {providers.map((provider) => <option key={provider.provider_key} value={provider.provider_key}>{provider.name}</option>)}
            </select>
          </Field>
          <Field label={t('models.originalProvider', 'Original provider')}>
            <select className={inputClass} {...form.register('original_provider')}>
              {originalProviders.map((provider) => <option key={provider} value={provider}>{provider}</option>)}
            </select>
          </Field>
          <Field label={t('bulk.modelPattern', 'Model id pattern')}>
            <input className={`${inputClass} font-mono`} {...form.register('model_id_pattern')} />
          </Field>
          <Field label={t('filter.apiType', 'API type')}>
            <select className={inputClass} {...form.register('api_type')}>
              <option value="">{t('filter.all', 'All')}</option>
              {apiTypes.map((item) => <option key={item} value={item}>{item}</option>)}
            </select>
          </Field>
          <Field label={t('filter.capability', 'Capability')}>
            <select className={inputClass} {...form.register('capability')}>
              <option value="">{t('filter.all', 'All')}</option>
              {capabilities.map((item) => <option key={item} value={item}>{item}</option>)}
            </select>
          </Field>
          <Field label={t('ops.recommendation', 'Recommendation')}>
            <select className={inputClass} {...form.register('recommendation_level')}>
              <option value="">{t('filter.all', 'All')}</option>
              {['featured', 'preferred', 'standard', 'limited'].map((item) => <option key={item} value={item}>{item}</option>)}
            </select>
          </Field>
          <Field label={t('bulk.priceRange', 'Price range')}>
            <div className="grid grid-cols-2 gap-2">
              <input className={inputClass} placeholder="min" step="0.000001" type="number" {...form.register('price_min')} />
              <input className={inputClass} placeholder="max" step="0.000001" type="number" {...form.register('price_max')} />
            </div>
          </Field>
          <Field label={t('bulk.routingRange', 'Routing weight range')}>
            <div className="grid grid-cols-2 gap-2">
              <input className={inputClass} placeholder="min" type="number" {...form.register('routing_weight_min')} />
              <input className={inputClass} placeholder="max" type="number" {...form.register('routing_weight_max')} />
            </div>
          </Field>
        </div>

        <div className="grid gap-3 border-t border-[color:var(--cp-border)] pt-4 md:grid-cols-2 xl:grid-cols-4">
          <Field label={t('bulk.action', 'Bulk action')}>
            <select className={inputClass} {...form.register('action')}>
              <option value="set_recommendation">{t('bulk.setRecommendation', 'Set recommendation')}</option>
              <option value="set_display_priority">{t('bulk.setDisplayPriority', 'Set display priority')}</option>
              <option value="adjust_price_percent">{t('bulk.adjustPrice', 'Adjust price by percent')}</option>
              <option value="set_price">{t('bulk.setPrice', 'Set input/output price')}</option>
              <option value="set_routing_weight">{t('bulk.setRoutingWeight', 'Set routing weight')}</option>
              <option value="clear_pricing">{t('bulk.clearPricing', 'Clear pricing override')}</option>
            </select>
          </Field>
          {action === 'set_recommendation' && (
            <Field label={t('ops.recommendation', 'Recommendation')}>
              <select className={inputClass} {...form.register('target_recommendation_level')}>
                {['featured', 'preferred', 'standard', 'limited'].map((item) => <option key={item} value={item}>{item}</option>)}
              </select>
            </Field>
          )}
          {action === 'set_display_priority' && (
            <Field label={t('ops.displayPriority', 'Display priority')}>
              <input className={inputClass} type="number" {...form.register('display_priority')} />
            </Field>
          )}
          {action === 'adjust_price_percent' && (
            <Field label={t('bulk.pricePercent', 'Price percent')}>
              <input className={inputClass} type="number" {...form.register('price_percent')} />
            </Field>
          )}
          {action === 'set_price' && (
            <>
              <Field label={t('ops.pricingInput', 'Input price')}>
                <input className={inputClass} step="0.000001" type="number" {...form.register('pricing_input')} />
              </Field>
              <Field label={t('ops.pricingOutput', 'Output price')}>
                <input className={inputClass} step="0.000001" type="number" {...form.register('pricing_output')} />
              </Field>
            </>
          )}
          {action === 'set_routing_weight' && (
            <Field label={t('ops.routingWeight', 'Routing weight')}>
              <input className={inputClass} type="number" {...form.register('routing_weight')} />
            </Field>
          )}
        </div>

        <div className="grid gap-3 md:grid-cols-3">
          <Impact label={t('bulk.hitCount', 'Matched rows')} value={preview.hit_count} tone="accent" />
          <Impact label={t('bulk.priceChanged', 'Price changed')} value={preview.price_changed} tone={preview.price_changed ? 'warning' : 'neutral'} />
          <Impact label={t('bulk.routingChanged', 'Routing changed')} value={preview.routing_changed} tone={preview.routing_changed ? 'warning' : 'neutral'} />
        </div>

        <div className="flex items-center justify-between gap-3 rounded-md border border-[color:var(--cp-border)] p-3 text-sm">
          <div className="flex items-center gap-2 text-[color:var(--cp-muted)]">
            <ClipboardCheck size={16} />
            {t('bulk.confirmHint', 'Confirm page shows hit count, samples, and before/after operations parameter values.')}
          </div>
          <button
            className="inline-flex h-10 items-center gap-2 rounded-md bg-[color:var(--cp-accent)] px-3 text-sm font-bold text-white disabled:cursor-not-allowed disabled:opacity-50"
            disabled={!preview.hit_count}
            type="submit"
          >
            <Percent size={16} />
            {t('bulk.apply', 'Apply bulk operation')}
          </button>
        </div>
        {applied && <StatusBadge tone="success">{t('bulk.applied', 'Bulk operation added to pending changes')}</StatusBadge>}
      </form>

      <section className="shell-card p-4">
        <div className="mb-3 flex items-center gap-2 text-sm font-bold">
          <SlidersHorizontal size={16} className="text-[color:var(--cp-accent)]" />
          {t('bulk.samples', 'Matched samples')}
        </div>
        {preview.samples.length ? (
          <DataTable columns={columns} rowKey={(row) => row.rule_key} rows={preview.samples} />
        ) : (
          <EmptyView />
        )}
      </section>
    </div>
  )
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block text-sm font-semibold">
      <span className="mb-1 block text-[color:var(--cp-muted)]">{label}</span>
      {children}
    </label>
  )
}

function Impact({ label, value, tone }: { label: string; value: number; tone: 'neutral' | 'success' | 'warning' | 'accent' }) {
  return (
    <div className="rounded-md border border-[color:var(--cp-border)] p-3 text-sm">
      <StatusBadge tone={tone}>{value}</StatusBadge>
      <div className="mt-2 text-[color:var(--cp-muted)]">{label}</div>
    </div>
  )
}

function formatPrice(value: OpsBulkPreviewRow['pricing_before']) {
  if (!value) {
    return '-'
  }
  return `${value.input}/${value.output}`
}

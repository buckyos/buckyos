import { useId, useMemo, useState, useSyncExternalStore } from 'react'
import { Dialog } from '@mui/material'
import { ChevronDown, Cpu, Download, Layers, Loader2, Monitor, RefreshCw, Search, X } from 'lucide-react'
import { useForm, useWatch } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import useSWR from 'swr'
import { useI18n } from '../../i18n/provider'
import { useAICCStore } from './hooks/use-aicc-store'
import { WeightFactorControl } from './components/routing/WeightFactorControl'
import {
  defaultModelFilters, filterModelCatalog, modelCard, modelFiltersSchema, vendorNames,
  type ModelCardView, type ModelFilters,
} from './datamodel/model-catalog'

const surface = { background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }
const muted = { color: 'var(--cp-muted)' }
const success = 'var(--cp-success)'

export function ModelsPage() {
  const { t } = useI18n()
  const store = useAICCStore()
  const version = useSyncExternalStore(store.subscribe, store.getSnapshotVersion)
  const { data, error, isLoading, isValidating, mutate } = useSWR(
    ['aicc-model-catalog', store, version], () => store.fetchModelCatalog(),
    { refreshInterval: 30000, keepPreviousData: true },
  )
  const { register, control, reset } = useForm<ModelFilters>({ resolver: zodResolver(modelFiltersSchema), defaultValues: defaultModelFilters })
  const filters = useWatch({ control }) as ModelFilters
  const [selected, setSelected] = useState<{ vendor: string; model: string } | null>(null)
  const vendors = useMemo(() => data ? filterModelCatalog(data, filters) : [], [data, filters])
  const allModels = useMemo(() => data?.vendors.flatMap((vendor) => vendor.models.map((model) => modelCard(model, vendor.id))) ?? [], [data])
  const selectedModel = allModels.find((model) => model.vendorId === selected?.vendor && model.id === selected.model)
  const stats = [
    [t('aiCenter.models.known'), allModels.length],
    [t('aiCenter.models.available'), allModels.filter((model) => model.available).length],
    [t('aiCenter.models.deployable'), allModels.filter((model) => model.deployable).length],
    [t('aiCenter.models.local'), allModels.filter((model) => model.local).length],
  ] as const
  const showModel = (vendor: string, model: string) => setSelected({ vendor, model })

  return (
    <div className="flex min-w-0 flex-col gap-5" style={{ color: 'var(--cp-text)' }}>
      <header className="flex items-start justify-between gap-3">
        <div>
          <h2 className="text-lg font-semibold">{t('aiCenter.models.title')}</h2>
          <p className="mt-1 max-w-2xl text-sm leading-6" style={muted}>{t('aiCenter.models.subtitle')}</p>
        </div>
        <button type="button" onClick={() => void mutate()} disabled={isValidating} aria-label={t('aiCenter.models.refresh')} title={t('aiCenter.models.refresh')}
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg disabled:opacity-50" style={surface}>
          <RefreshCw size={17} className={isValidating ? 'animate-spin' : ''} />
        </button>
      </header>

      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        {stats.map(([label, count]) => (
          <div key={label} className="rounded-xl px-4 py-3" style={surface}>
            <div className="text-xs" style={muted}>{label}</div>
            <div className="mt-1 text-2xl font-semibold tabular-nums">{data ? count : '—'}</div>
          </div>
        ))}
      </div>

      <form onSubmit={(event) => event.preventDefault()} className="flex flex-col gap-3" role="search">
        <label className="flex min-h-11 items-center gap-3 rounded-lg px-3" style={surface}>
          <Search size={18} style={muted} />
          <input {...register('query')} aria-label={t('aiCenter.models.search')} placeholder={t('aiCenter.models.search')}
            className="min-w-0 flex-1 bg-transparent py-3 text-sm outline-none" />
        </label>
        <div className="flex flex-wrap gap-2">
          {(['available', 'deployable', 'local'] as const).map((key) => (
            <label key={key} className="flex min-h-11 cursor-pointer items-center gap-2 rounded-lg px-3 text-xs" style={{ ...surface, borderColor: filters[key] ? 'var(--cp-accent)' : 'var(--cp-border)' }}>
              <input type="checkbox" {...register(key)} style={{ accentColor: 'var(--cp-accent)' }} />
              {t(`aiCenter.models.filter.${key}`)}
            </label>
          ))}
          {(filters.query || filters.available || filters.deployable || filters.local) && (
            <button type="button" onClick={() => reset(defaultModelFilters)} className="min-h-11 px-3 text-xs" style={{ color: 'var(--cp-accent)' }}>{t('aiCenter.models.clear')}</button>
          )}
        </div>
      </form>

      {error && <div role="alert" className="flex flex-wrap items-center justify-between gap-2 rounded-lg p-3 text-sm" style={{ ...surface, color: 'var(--cp-danger)' }}>
        {t('aiCenter.models.error')}
        <button type="button" className="min-h-11 px-3 underline" onClick={() => void mutate()}>{t('common.retry')}</button>
      </div>}
      {isLoading && !data && <div role="status" className="flex items-center justify-center gap-2 py-16 text-sm" style={muted}>
        <Loader2 size={20} className="animate-spin" />{t('aiCenter.models.loading')}
      </div>}
      {data && <>
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs" style={muted}>
          <span>{t('aiCenter.models.results', undefined, { count: vendors.reduce((sum, vendor) => sum + vendor.models.length, 0), total: allModels.length })}</span>
          <div className="flex flex-wrap items-center gap-4">
            <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full" style={{ background: success }} />{t('aiCenter.models.available')}</span>
            <span className="flex items-center gap-1.5"><span className="h-2 w-2 rounded-full" style={{ background: 'var(--cp-muted)' }} />{t('aiCenter.models.unavailable')}</span>
            <span className="flex items-center gap-1.5"><Monitor size={14} style={{ color: success }} />{t('aiCenter.models.local')}</span>
          </div>
        </div>
        {vendors.length === 0 && <div className="rounded-xl px-4 py-12 text-center" style={surface}>
          <Cpu size={32} className="mx-auto mb-3" style={muted} />
          <p className="font-medium">{t(allModels.length ? 'aiCenter.models.noMatches' : 'aiCenter.models.empty')}</p>
          <p className="mt-2 text-sm" style={muted}>{t(allModels.length ? 'aiCenter.models.noMatchesHint' : 'aiCenter.models.emptyHint')}</p>
        </div>}
        {vendors.map((vendor) => <VendorSection key={vendor.id} vendor={vendor} showModel={showModel} />)}
      </>}
      <Dialog open={!!selectedModel} onClose={() => setSelected(null)} maxWidth="md" fullWidth aria-labelledby="model-detail-title"
        slotProps={{ paper: { sx: { bgcolor: 'var(--cp-surface)', color: 'var(--cp-text)', borderRadius: 3, margin: 2, width: 'calc(100% - 32px)' } } }}>
        {selectedModel && <ModelDetails model={selectedModel} onClose={() => setSelected(null)} />}
      </Dialog>
    </div>
  )
}

function VendorSection({ vendor, showModel }: {
  vendor: ReturnType<typeof filterModelCatalog>[number]
  showModel: (vendor: string, model: string) => void
}) {
  const { t } = useI18n()
  const sectionId = useId()
  const [collapsed, setCollapsed] = useState(false)
  const [expandedSpecId, setExpandedSpecId] = useState<string | null>(null)
  const expandedSpec = vendor.specs.find((spec) => spec.id === expandedSpecId)
  const vendorName = vendorNames[vendor.id] ?? vendor.id

  return <section aria-label={vendorName} className="flex min-w-0 flex-col gap-2">
    <header className="flex flex-wrap items-center gap-x-3 gap-y-1">
      <h3 className="flex shrink-0 items-center gap-2 text-base font-semibold">
        {vendorName}
        <span className="rounded-md px-2 py-0.5 text-xs font-normal" style={{ background: 'var(--cp-surface)', ...muted }}>{vendor.models.length}</span>
        <button type="button" onClick={() => setCollapsed(!collapsed)} aria-expanded={!collapsed}
          aria-controls={`${sectionId}-specs ${sectionId}-models`}
          aria-label={t(collapsed ? 'aiCenter.models.expandVendor' : 'aiCenter.models.collapseVendor', undefined, { vendor: vendorName })}
          className="flex h-11 w-11 items-center justify-center rounded-lg sm:h-8 sm:w-8" style={muted}>
          <ChevronDown size={17} className={`transition-transform ${collapsed ? '-rotate-90' : ''}`} />
        </button>
      </h3>
      {!collapsed && <WeightFactorControl compact subject={{ kind: 'vendor_factor', vendor: vendor.id, factor: 1 }} label={vendorName} />}
      <div id={`${sectionId}-specs`} hidden={collapsed} role="group" aria-label={t('aiCenter.models.specs')}
        className={collapsed ? 'hidden' : 'flex min-w-0 flex-wrap items-center gap-1.5'}>
        {vendor.specs.map((spec) => (
          <button key={spec.id} id={`${sectionId}-${spec.id}`} type="button" aria-expanded={expandedSpecId === spec.id}
            aria-controls={`${sectionId}-spec-members`} onClick={() => setExpandedSpecId(expandedSpecId === spec.id ? null : spec.id)}
            aria-label={`${spec.id}, ${t('aiCenter.models.memberCount', undefined, { count: spec.members.length })}`}
            className="flex min-h-11 max-w-full items-center gap-1.5 rounded-lg px-2.5 text-xs sm:min-h-8"
            style={{ ...surface, color: expandedSpecId === spec.id ? 'var(--cp-accent)' : 'var(--cp-muted)', borderColor: expandedSpecId === spec.id ? 'var(--cp-accent)' : 'var(--cp-border)' }}>
            <Layers size={12} className="shrink-0" />
            <span className="min-w-0 break-all font-mono">{spec.id}</span>
            <span className="shrink-0 text-[10px] tabular-nums">{spec.members.length}</span>
            <ChevronDown size={12} className={`shrink-0 transition-transform ${expandedSpecId === spec.id ? 'rotate-180' : ''}`} />
          </button>
        ))}
      </div>
    </header>
    <div id={`${sectionId}-models`} hidden={collapsed} className={collapsed ? 'hidden' : 'flex min-w-0 flex-col gap-3'}>
      {expandedSpec && <div id={`${sectionId}-spec-members`} role="region" aria-labelledby={`${sectionId}-${expandedSpec.id}`} className="rounded-lg px-3 py-2" style={surface}>
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <p className="break-all text-xs" style={muted}>{expandedSpec.path}{expandedSpec.direct_only ? ` · ${t('aiCenter.models.directOnly')}` : ''}</p>
          <WeightFactorControl compact subject={{ kind: 'spec_factor', spec: expandedSpec.path, factor: 1 }} label={expandedSpec.id} />
        </div>
        {expandedSpec.members.length ? <>
          <div className="mb-1 flex justify-between gap-3 text-xs" style={muted}><span>{t('aiCenter.models.modelName')}</span><span>{t('aiCenter.models.weight')}</span></div>
          {expandedSpec.members.map((member) => (
            <div key={member.model_id} className="flex items-center justify-between gap-3 border-t py-1" style={{ borderColor: 'var(--cp-border)' }}>
              <button type="button" onClick={() => showModel(vendor.id, member.model_id)} className="min-h-11 min-w-0 py-1 text-left text-xs" style={{ color: 'var(--cp-accent)' }}>
                <span className="block break-all font-medium">{member.model_id}</span>
                <span className="mt-1 block break-all font-mono text-[11px]" style={muted}>{member.target}</span>
              </button>
              <span className="shrink-0 text-right text-xs tabular-nums">{member.weight}<span className="mt-1 block text-[11px]" style={muted}>{t(member.active ? 'aiCenter.models.effectiveWeight' : 'aiCenter.models.defaultWeight')}</span></span>
            </div>
          ))}
        </> : <p className="py-2 text-xs" style={muted}>{t('aiCenter.models.emptySpec')}</p>}
      </div>}
      <div className="grid grid-cols-1 gap-3 xl:grid-cols-2 2xl:grid-cols-3">
        {vendor.models.map((model) => <ModelCard key={model.id} model={model} onClick={() => showModel(vendor.id, model.id)} />)}
      </div>
    </div>
  </section>
}

function ModelCard({ model, onClick }: { model: ModelCardView; onClick: () => void }) {
  const { t } = useI18n()
  const context = model.metadata.capabilities?.max_context_tokens
  return <button type="button" onClick={onClick} data-testid="model-card" data-model={model.id} data-available={model.available} data-local={model.local}
    className="flex min-w-0 flex-col gap-3 rounded-xl p-4 text-left transition-shadow hover:shadow-md focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2"
    style={{ ...surface, borderColor: model.available ? 'color-mix(in srgb, var(--cp-success) 45%, var(--cp-border))' : 'var(--cp-border)' }}>
    <div className="flex w-full items-start justify-between gap-3">
      <span className="min-w-0 break-all font-mono text-sm font-semibold">{model.id}</span>
      {model.deployable && <span role="img" className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg" title={t(model.local ? 'aiCenter.models.local' : 'aiCenter.models.deployable')} aria-label={t(model.local ? 'aiCenter.models.local' : 'aiCenter.models.deployable')}
        style={{ color: model.local ? success : 'var(--cp-muted)', background: model.local ? 'color-mix(in srgb, var(--cp-success) 12%, transparent)' : 'var(--cp-bg)' }}>
        {model.local ? <Monitor size={17} /> : <Download size={17} />}
      </span>}
    </div>
    <div className="flex flex-wrap items-center gap-2 text-[11px]" style={muted}>
      {(model.metadata.api_types ?? []).map((api) => <span key={api} className="rounded px-2 py-1" style={{ background: 'var(--cp-bg)' }}>{api}</span>)}
      {model.metadata.parameter_scale && <span>{model.metadata.parameter_scale}</span>}
      {typeof context === 'number' && <span>{t('aiCenter.models.context', undefined, { count: context.toLocaleString() })}</span>}
    </div>
    <div className="mt-auto flex w-full flex-wrap items-center justify-between gap-2 text-xs">
      <span className="flex items-center gap-1.5" style={{ color: model.available ? success : 'var(--cp-muted)' }}>
        <span className="h-2 w-2 rounded-full" style={{ background: 'currentColor' }} />
        {model.available ? t('aiCenter.models.providerCount', undefined, { count: model.providers.length }) : t('aiCenter.models.unavailable')}
      </span>
    </div>
  </button>
}

function ModelDetails({ model, onClose }: { model: ModelCardView; onClose: () => void }) {
  const { t } = useI18n()
  const llm = model.metadata.llm
  const facts = [
    [t('aiCenter.models.vendor'), vendorNames[model.vendorId] ?? model.vendorId],
    [t('aiCenter.models.apiTypes'), model.metadata.api_types?.join(', ') ?? '—'],
    [t('aiCenter.models.specs'), llm?.spec ?? '—'],
    [t('aiCenter.models.effort'), llm?.effort ?? '—'],
    [t('aiCenter.models.defaultEffort'), llm?.default_effort ?? '—'],
    [t('aiCenter.models.supportedEfforts'), llm?.supported_efforts.join(', ') ?? '—'],
  ]
  return <div className="min-w-0 p-5 sm:p-6">
    <div className="mb-5 flex items-start justify-between gap-3">
      <div className="min-w-0"><p className="mb-1 text-xs" style={muted}>{t('aiCenter.models.details')}</p><h2 id="model-detail-title" className="break-all font-mono text-lg font-semibold">{model.id}</h2></div>
      <button type="button" onClick={onClose} aria-label={t('common.close')} className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg" style={surface}><X size={20} /></button>
    </div>
    <div className="mb-5">
      <WeightFactorControl subject={{ kind: 'model_factor', vendor: model.vendorId, model: model.id, factor: 1 }} label={model.id} />
    </div>
    <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2">
      {facts.map(([label, value]) => <div key={label}><dt className="text-xs" style={muted}>{label}</dt><dd className="mt-1 break-all text-sm">{value}</dd></div>)}
    </dl>
    <h3 className="mb-2 mt-6 text-sm font-semibold">{t('aiCenter.models.providers')}</h3>
    {model.providers.length ? <div className="space-y-2">{model.providers.map((provider) => <div key={provider.id} className="rounded-lg p-3" style={{ background: 'var(--cp-bg)' }}>
      <p className="flex items-center gap-2 break-all text-sm"><Monitor size={16} className="shrink-0" style={{ color: provider.local ? success : 'var(--cp-muted)' }} />{provider.id}{provider.local && <span className="text-xs" style={{ color: success }}>{t('aiCenter.models.local')}</span>}</p>
      {provider.exact_models.map((exact) => <p key={exact} className="mt-1 break-all font-mono text-xs" style={muted}>{exact}</p>)}
    </div>)}</div> : <p className="text-sm" style={muted}>{t('aiCenter.models.noProviders')}</p>}
    {model.deployable && <div className="mt-5 rounded-lg p-4" style={{ background: 'var(--cp-bg)' }}>
      <div className="flex flex-wrap items-center justify-between gap-3"><span className="text-sm font-medium">{t('aiCenter.models.localDeployment')}</span>
        {model.local ? <span className="flex items-center gap-2 text-sm" style={{ color: success }}><Monitor size={17} />{t('aiCenter.models.local')}</span>
          : <button type="button" disabled className="flex min-h-11 items-center gap-2 rounded-lg px-3 text-sm opacity-60" style={surface}><Download size={16} />{t('aiCenter.models.deploy')}</button>}
      </div>
      {!model.local && <p className="mt-2 text-xs leading-5" style={muted}>{t('aiCenter.models.deployHint')}</p>}
    </div>}
    <details className="mt-5 rounded-lg" style={surface}>
      <summary className="min-h-11 cursor-pointer px-3 py-3 text-sm">{t('aiCenter.models.allMetadata')}</summary>
      <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-all px-3 pb-3 text-xs leading-5" style={muted}>{JSON.stringify(model.metadata, null, 2)}</pre>
    </details>
  </div>
}

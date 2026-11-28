import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { Pencil, Plus, RefreshCw, Tags, Trash2 } from 'lucide-react'
import useSWR from 'swr'
import { useI18n } from '../../i18n/provider'
import { useViewerIsAdmin } from '../settings/hooks/use-viewer-is-admin'
import { useAICCStore, useProviders } from './hooks/use-aicc-store'
import { usePolicyApi } from './hooks/use-policy-api'
import {
  exactPriceKey,
  formatRate,
  isZeroPricing,
  modelPriceKey,
  parsePriceKey,
  type ModelCreditPrice,
  type PriceEntry,
  type PriceGetResponse,
} from './datamodel/policy'
import { vendorNames } from './datamodel/model-catalog'
import { PriceEntryDialog } from './components/prices/PriceEntryDialog'
import { CreditPriceBadge, ExactModelLabel, Modal, StatusDot } from './components/policy/PolicyWidgets'
import { buttonStyle, errorText, fieldStyle, panelStyle } from './components/policy/policy-labels'
import { EmptyState } from './components/shared/EmptyState'

type PriceFilter = 'in_use' | 'unpriced' | 'all'

/** A stored entry as the price a person pays at coefficient 1. */
function asPrice(entry: PriceEntry): ModelCreditPrice {
  return { state: isZeroPricing(entry.pricing) ? 'free' : 'priced', source: entry.entry_key.startsWith('exact:') ? 'exact' : 'model', entry_key: entry.entry_key, pricing: entry.pricing, provider_coefficient: 1, model_coefficient: 1 }
}

function PriceCell({ entry, onEdit, testId }: { entry?: PriceEntry; onEdit: () => void; testId?: string }) {
  const { t } = useI18n()
  return (
    <div className="flex items-center gap-2">
      <span className="min-w-0 flex-1">
        {entry ? <CreditPriceBadge price={asPrice(entry)} testId="price-value" /> : <StatusDot testId="price-value" status="none" tone="muted" label={t('aiCenter.prices.noPrice', 'No price')} />}
      </span>
      <button type="button" data-testid={testId} onClick={onEdit} aria-label={t('aiCenter.prices.edit', 'Set price')} title={t('aiCenter.prices.edit', 'Set price')} className="min-h-10 min-w-10 shrink-0 rounded-lg p-2" style={{ color: 'var(--cp-accent)' }}>
        <Pencil size={15} />
      </button>
    </div>
  )
}

function RatesDialog({ rates, revision, onClose, onSave }: { rates: Record<string, number>; revision: number; onClose: () => void; onSave: (rates: Record<string, number>, revision: number) => Promise<void> }) {
  const { t } = useI18n()
  const [rows, setRows] = useState(() => Object.entries(rates).map(([currency, rate]) => ({ currency, rate: String(rate) })))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const valid = rows.every((row) => /^[A-Za-z]{3,8}$/.test(row.currency.trim()) && /^\d+(\.\d+)?$/.test(row.rate.trim())) &&
    new Set(rows.map((row) => row.currency.trim().toUpperCase())).size === rows.length
  const save = async () => {
    setBusy(true)
    setError(null)
    try {
      await onSave(Object.fromEntries(rows.map((row) => [row.currency.trim().toUpperCase(), Number(row.rate)])), revision)
    } catch (reason) {
      setError(errorText(reason))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Modal title={t('aiCenter.prices.ratesTitle', 'AI Credit rates')} onClose={onClose} busy={busy} testId="price-rates-dialog">
      <p className="mb-3 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.prices.ratesHint', 'For models without a price list entry: the Provider’s own cost in a currency × this rate. Only the currencies listed here; nothing is converted automatically.')}</p>
      <div className="flex flex-col gap-2">
        {rows.map((row, index) => (
          <div key={index} className="flex items-center gap-2" data-testid="price-rate-row">
            <span className="text-sm" style={{ color: 'var(--cp-muted)' }}>1</span>
            <input aria-label={t('aiCenter.prices.currency', 'Currency')} data-testid="price-rate-currency" value={row.currency} placeholder="USD" onChange={(event) => setRows(rows.map((item, position) => position === index ? { ...item, currency: event.target.value } : item))} className="min-h-10 w-24 rounded-lg px-3 text-sm uppercase" style={fieldStyle} />
            <span className="text-sm" style={{ color: 'var(--cp-muted)' }}>=</span>
            <input aria-label={t('aiCenter.prices.rate', 'AI Credit')} data-testid="price-rate-value" value={row.rate} inputMode="decimal" placeholder="100" onChange={(event) => setRows(rows.map((item, position) => position === index ? { ...item, rate: event.target.value } : item))} className="min-h-10 w-28 min-w-0 flex-1 rounded-lg px-3 text-sm" style={fieldStyle} />
            <span className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.policy.unit.credit', 'AI Credit')}</span>
            <button type="button" onClick={() => setRows(rows.filter((_, position) => position !== index))} aria-label={t('aiCenter.prices.removeRate', 'Remove rate')} className="min-h-10 min-w-10 rounded-lg p-2" style={{ color: 'var(--cp-muted)' }}><Trash2 size={16} /></button>
          </div>
        ))}
        <button type="button" data-testid="price-rate-add" onClick={() => setRows([...rows, { currency: '', rate: '' }])} className="inline-flex min-h-9 items-center gap-1 self-start rounded-lg px-2 text-sm" style={{ color: 'var(--cp-accent)' }}>
          <Plus size={14} />{t('aiCenter.prices.addRate', 'Add a currency')}
        </button>
      </div>
      {error && <p className="mt-2 text-xs" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
      <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <button type="button" onClick={onClose} disabled={busy} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.cancel', 'Cancel')}</button>
        <button type="button" data-testid="price-rates-save" disabled={busy || !valid} onClick={() => void save()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>{t('aiCenter.prices.saveRates', 'Save rates')}</button>
      </div>
    </Modal>
  )
}

/** The Zone's AI Credit price list (administrators): prices by Model Driver model identity, so they hold for
 * every Provider mapped to the model, exact model exceptions and Credit rates of Provider costs. */
export function PricesPage() {
  const { t, locale } = useI18n()
  const api = usePolicyApi()
  const store = useAICCStore()
  const version = useSyncExternalStore(store.subscribe, store.getSnapshotVersion)
  const isAdmin = useViewerIsAdmin()
  const providers = useProviders()
  const { data: catalog } = useSWR(isAdmin ? ['aicc-price-catalog', store, version] : null, () => store.fetchModelCatalog(), { revalidateOnFocus: false })
  const [prices, setPrices] = useState<PriceGetResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [generation, setGeneration] = useState(0)
  const [filter, setFilter] = useState<PriceFilter>('in_use')
  const [query, setQuery] = useState('')
  const [editing, setEditing] = useState<{ key: string; label: string } | null>(null)
  const [pickingException, setPickingException] = useState(false)
  const [ratesOpen, setRatesOpen] = useState(false)

  useEffect(() => {
    if (!isAdmin) return
    let live = true
    api.getPrices().then((value) => { if (live) { setPrices(value); setError(null) } }).catch((reason: unknown) => { if (live) setError(errorText(reason)) })
    return () => { live = false }
  }, [api, isAdmin, generation, version])

  const entries = useMemo(() => new Map((prices?.entries ?? []).map((entry) => [entry.entry_key, entry])), [prices])
  const unpriced = useMemo(() => new Set(prices?.unpriced_models ?? []), [prices])
  const inventory = useMemo(() => providers.flatMap((provider) => provider.inventory.models.map((model) => ({ exact: model.exact_model, provider: provider.config.provider_instance_name }))), [providers])
  const mapped = useMemo(() => new Set((catalog?.vendors ?? []).flatMap((vendor) => vendor.models.flatMap((model) => model.providers.flatMap((provider) => provider.exact_models)))), [catalog])
  const needle = query.trim().toLowerCase()

  const identities = useMemo(() => (catalog?.vendors ?? []).map((vendor) => ({
    vendor: vendor.id,
    models: vendor.models.map((model) => {
      const exact = model.providers.flatMap((provider) => provider.exact_models)
      return { id: model.id, key: modelPriceKey(vendor.id, model.id), exact, unpriced: exact.filter((item) => unpriced.has(item)) }
    }).filter((model) => {
      const priced = entries.has(model.key)
      if (filter === 'in_use' && model.exact.length === 0 && !priced) return false
      if (filter === 'unpriced' && model.unpriced.length === 0) return false
      return !needle || `${vendor.id} ${vendorNames[vendor.id] ?? ''} ${model.id}`.toLowerCase().includes(needle)
    }),
  })).filter((vendor) => vendor.models.length > 0), [catalog, entries, unpriced, filter, needle])

  const exceptions = (prices?.entries ?? []).filter((entry) => parsePriceKey(entry.entry_key)?.kind === 'exact')
    .filter((entry) => !needle || entry.entry_key.toLowerCase().includes(needle))
  const unmapped = inventory.filter((model) => !mapped.has(model.exact) && unpriced.has(model.exact))
    .filter((model) => !needle || model.exact.toLowerCase().includes(needle))
  const rates = prices?.credit_rates ?? {}

  if (isAdmin === false) {
    return <EmptyState icon={<Tags size={40} />} title={t('aiCenter.prices.adminOnly', 'Only administrators manage the price list')} description={t('aiCenter.prices.adminOnlyHint', 'Your prices are shown on Models.')} />
  }

  return (
    <div className="flex flex-col gap-5" data-testid="prices-page">
      <header className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
        <div>
          <h1 className="text-xl font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.prices.title', 'Price list')}</h1>
          <p className="mt-1 max-w-3xl text-sm" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.prices.subtitle', 'What models cost in AI Credit, the Zone’s unit for AI use. Price a model once by its identity and every Provider that carries it is priced, also ones added later. Policies multiply these prices by their coefficients; 0 is free. This is separate from what Providers charge the Zone.')}
          </p>
        </div>
        <button type="button" onClick={() => setGeneration((value) => value + 1)} aria-label={t('aiCenter.policy.refresh', 'Refresh')} title={t('aiCenter.policy.refresh', 'Refresh')} className="min-h-10 self-start rounded-lg px-3" style={buttonStyle.secondary}><RefreshCw size={16} /></button>
      </header>
      {error && <p className="text-sm" style={{ color: 'var(--cp-danger)' }}>{error}</p>}

      {unpriced.size > 0 && (
        <div data-testid="prices-unpriced" className="flex flex-col gap-2 rounded-xl px-4 py-3 text-sm md:flex-row md:items-center" style={{ background: 'color-mix(in srgb, var(--cp-warning) 10%, var(--cp-surface))', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
          <span>{t('aiCenter.prices.unpricedNotice', '{{count}} models in the model list have no price: users who pay (coefficient not 0) cannot use them.', { count: unpriced.size })}</span>
          <button type="button" data-testid="prices-show-unpriced" onClick={() => setFilter('unpriced')} className="min-h-9 shrink-0 whitespace-nowrap rounded-lg px-3 text-sm md:ml-auto" style={buttonStyle.secondary}>{t('aiCenter.prices.showUnpriced', 'Show them')}</button>
        </div>
      )}

      <div className="flex flex-col gap-2 md:flex-row md:items-center">
        <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t('aiCenter.prices.search', 'Search model or Provider')} aria-label={t('aiCenter.prices.search', 'Search model or Provider')} className="min-h-10 flex-1 rounded-lg px-3 text-sm" style={fieldStyle} />
        <select data-testid="prices-filter" aria-label={t('aiCenter.prices.filter', 'Show')} value={filter} onChange={(event) => setFilter(event.target.value as PriceFilter)} className="min-h-10 rounded-lg px-3 text-sm" style={fieldStyle}>
          <option value="in_use">{t('aiCenter.prices.filter.inUse', 'Models in use or priced')}</option>
          <option value="unpriced">{t('aiCenter.prices.filter.unpriced', 'Models without a price')}</option>
          <option value="all">{t('aiCenter.prices.filter.all', 'The whole Model Driver catalog')}</option>
        </select>
      </div>

      <section className="flex flex-col gap-3" data-testid="prices-models">
        <div>
          <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.prices.byModel', 'Prices by model')}</h2>
          <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.prices.byModelHint', 'From the Model Driver catalog, so a model can be priced before any Provider carries it.')}</p>
        </div>
        {!catalog && <div role="status" className="text-sm" style={{ color: 'var(--cp-muted)' }}>{t('common.loading', 'Loading...')}</div>}
        {catalog && identities.length === 0 && <p className="rounded-xl p-6 text-center text-sm" style={{ ...panelStyle, color: 'var(--cp-muted)' }}>{t('aiCenter.prices.noModels', 'No model matches.')}</p>}
        {identities.map((vendor) => (
          <div key={vendor.vendor} className="overflow-hidden rounded-xl" style={panelStyle}>
            <div className="px-4 py-2 text-xs font-medium" style={{ background: 'var(--cp-bg)', color: 'var(--cp-muted)', borderBottom: '1px solid var(--cp-border)' }}>{vendorNames[vendor.vendor] ?? vendor.vendor}</div>
            {vendor.models.map((model) => (
              <div key={model.key} data-testid="price-model" data-key={model.key} className="grid grid-cols-1 gap-1 px-4 py-2 md:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)] md:items-center" style={{ borderTop: '1px solid var(--cp-border)' }}>
                <div className="min-w-0">
                  <div className="break-all text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{model.id}</div>
                  <div className="text-xs" style={{ color: model.unpriced.length ? 'var(--cp-warning)' : 'var(--cp-muted)' }}>
                    {model.exact.length === 0
                      ? t('aiCenter.prices.noProvider', 'No Provider carries it yet')
                      : t('aiCenter.prices.carriedBy', 'In {{count}} exact models', { count: model.exact.length })}
                    {model.unpriced.length > 0 ? ` · ${t('aiCenter.prices.unpricedCount', '{{count}} without a price', { count: model.unpriced.length })}` : ''}
                  </div>
                </div>
                <PriceCell testId="price-edit" entry={entries.get(model.key)} onEdit={() => setEditing({ key: model.key, label: `${vendorNames[vendor.vendor] ?? vendor.vendor} · ${model.id}` })} />
              </div>
            ))}
          </div>
        ))}
      </section>

      <section className="flex flex-col gap-2" data-testid="prices-exceptions">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.prices.exceptions', 'Exact model exceptions')}</h2>
            <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.prices.exceptionsHint', 'A different price for one Provider’s model; it overrides the model price.')}</p>
          </div>
          <button type="button" data-testid="price-exception-add" onClick={() => setPickingException(true)} className="inline-flex min-h-10 items-center gap-1 rounded-lg px-3 text-sm" style={buttonStyle.secondary}>
            <Plus size={15} />{t('aiCenter.prices.addException', 'Add an exception')}
          </button>
        </div>
        {exceptions.length === 0 && <p className="rounded-xl p-4 text-center text-xs" style={{ ...panelStyle, color: 'var(--cp-muted)' }}>{t('aiCenter.prices.noExceptions', 'No exception.')}</p>}
        {exceptions.length > 0 && (
          <div className="overflow-hidden rounded-xl" style={panelStyle}>
            {exceptions.map((entry) => {
              const exact = entry.entry_key.slice('exact:'.length)
              return (
                <div key={entry.entry_key} data-testid="price-exception" className="grid grid-cols-1 gap-1 px-4 py-2 md:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)] md:items-center" style={{ borderTop: '1px solid var(--cp-border)' }}>
                  <span className="min-w-0 text-sm" style={{ color: 'var(--cp-text)' }}><ExactModelLabel exactModel={exact} />{entry.note ? <span className="block text-xs" style={{ color: 'var(--cp-muted)' }}>{entry.note}</span> : null}</span>
                  <PriceCell entry={entry} onEdit={() => setEditing({ key: entry.entry_key, label: exact })} />
                </div>
              )
            })}
          </div>
        )}
      </section>

      {unmapped.length > 0 && (
        <section className="flex flex-col gap-2" data-testid="prices-unmapped">
          <div>
            <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.prices.unmapped', 'Models without a Model Driver identity')}</h2>
            <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.prices.unmappedHint', 'These can only be priced one by one.')}</p>
          </div>
          <div className="overflow-hidden rounded-xl" style={panelStyle}>
            {unmapped.map((model) => (
              <div key={model.exact} data-testid="price-unmapped" className="grid grid-cols-1 gap-1 px-4 py-2 md:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)] md:items-center" style={{ borderTop: '1px solid var(--cp-border)' }}>
                <span className="min-w-0 text-sm" style={{ color: 'var(--cp-text)' }}><ExactModelLabel exactModel={model.exact} /></span>
                <PriceCell onEdit={() => setEditing({ key: exactPriceKey(model.exact), label: model.exact })} />
              </div>
            ))}
          </div>
        </section>
      )}

      <section className="flex flex-col gap-2 rounded-xl p-4" data-testid="prices-rates" style={panelStyle}>
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <h2 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.prices.ratesTitle', 'AI Credit rates')}</h2>
            <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.prices.ratesShort', 'Price models without an entry from what their Provider reports it costs.')}</p>
          </div>
          <button type="button" data-testid="price-rates-edit" onClick={() => setRatesOpen(true)} className="inline-flex min-h-10 items-center gap-1 rounded-lg px-3 text-sm" style={buttonStyle.secondary}>
            <Pencil size={15} />{t('aiCenter.prices.editRates', 'Edit rates')}
          </button>
        </div>
        {Object.keys(rates).length === 0
          ? <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.prices.noRates', 'No rate: Provider costs are not turned into AI Credit.')}</p>
          : (
            <div className="flex flex-wrap gap-x-6 gap-y-1 text-sm" style={{ color: 'var(--cp-text)' }}>
              {Object.entries(rates).map(([currency, rate]) => <span key={currency} data-testid="price-rate">1 {currency} = {formatRate(rate, locale)} {t('aiCenter.policy.unit.credit', 'AI Credit')}</span>)}
            </div>
          )}
      </section>

      {editing && prices && (
        <PriceEntryDialog
          api={api}
          entryKey={editing.key}
          label={editing.label}
          revision={prices.revision}
          entry={entries.get(editing.key)}
          onClose={() => setEditing(null)}
          onSaved={(next) => { setPrices(next); setEditing(null) }}
        />
      )}
      {pickingException && (
        <Modal title={t('aiCenter.prices.addException', 'Add an exception')} onClose={() => setPickingException(false)} testId="price-exception-pick">
          <ExceptionPicker
            models={inventory.map((model) => model.exact).filter((exact) => !entries.has(exactPriceKey(exact)))}
            onPick={(exact) => { setPickingException(false); setEditing({ key: exactPriceKey(exact), label: exact }) }}
            onCancel={() => setPickingException(false)}
          />
        </Modal>
      )}
      {ratesOpen && prices && (
        <RatesDialog
          rates={rates}
          revision={prices.revision}
          onClose={() => setRatesOpen(false)}
          onSave={async (next, revision) => { setPrices(await api.updatePrices({ expected_revision: revision, credit_rates: next })); setRatesOpen(false) }}
        />
      )}
    </div>
  )
}

function ExceptionPicker({ models, onPick, onCancel }: { models: string[]; onPick: (exact: string) => void; onCancel: () => void }) {
  const { t } = useI18n()
  const [value, setValue] = useState(models[0] ?? '')
  return (
    <>
      <label className="flex flex-col gap-1 text-sm">
        <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.prices.exactModel', 'Exact model')}</span>
        <select data-testid="price-exception-model" value={value} onChange={(event) => setValue(event.target.value)} className="min-h-10 rounded-lg px-3" style={fieldStyle}>
          {models.map((model) => <option key={model} value={model}>{model}</option>)}
        </select>
      </label>
      <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
        <button type="button" onClick={onCancel} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.cancel', 'Cancel')}</button>
        <button type="button" data-testid="price-exception-continue" disabled={!value} onClick={() => onPick(value)} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>{t('aiCenter.policy.continue', 'Continue')}</button>
      </div>
    </>
  )
}

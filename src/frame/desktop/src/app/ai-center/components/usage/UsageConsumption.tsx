import { useRef, useState } from 'react'
import ClickAwayListener from '@mui/material/ClickAwayListener'
import { useI18n } from '../../../../i18n/provider'
import type { Money, UsageDimension, UsageTotals } from '../../mock/types'
import type { UsageEventsPage } from '../../../../api/aicc_mgr'
import { mergeUsageTotals, usageUnits } from '../../datamodel/usage-units'

export function UsageUnits({ tokens, dimensions, count, alignStart = false, compact = false }: { tokens: number; dimensions: Record<string, UsageDimension>; count: number; alignStart?: boolean; compact?: boolean }) {
  const { locale, t } = useI18n()
  const detailsRef = useRef<HTMLDetailsElement>(null)
  const format = (value: number) => value.toLocaleString(locale, { maximumFractionDigits: 6 })
  const visibleUnits = Object.entries(usageUnits).filter(([key]) => dimensions[key]?.reported_events > 0 && dimensions[key].amount > 0)
  return <ClickAwayListener mouseEvent="onPointerDown" touchEvent={false} onClickAway={() => { if (detailsRef.current) detailsRef.current.open = false }}><details ref={detailsRef} className="relative" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { event.stopPropagation(); if (event.key === 'Escape') event.currentTarget.open = false }}>
    <summary className="cursor-pointer truncate text-xs underline decoration-dotted underline-offset-4" style={{ color: 'var(--cp-accent)' }} aria-label={t("aiCenter.usage.usageDetails", "Usage details")} title={`${format(tokens)} tokens`}>{compact ? tokens.toLocaleString(locale, { notation: 'compact', maximumFractionDigits: 1 }) : `${format(tokens)} tokens`}</summary>
    <div className={`absolute ${alignStart ? 'left-0 md:left-auto md:right-0' : 'right-0'} z-30 mt-2 w-72 max-w-[85vw] rounded-lg p-3 shadow-xl text-left`} style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
      <p className="mb-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{t("aiCenter.usage.subsetsOverlapDoNotAddThemToTheTotal", "Subsets overlap; do not add them to the total.")}</p>
      <dl className="max-h-72 overflow-y-auto text-xs">
        {visibleUnits.map(([key, labels]) => {
          const value = dimensions[key]
          return <div key={key} className="flex justify-between gap-3 py-1"><dt>{t(`aiCenter.usage.dimension.${key}`, labels[0])}{value.reported_events < count && <span className="block text-[10px]" style={{ color: 'var(--cp-warning)' }}>{t("aiCenter.usage.knownSubtotal", "Known subtotal")} · {value.reported_events}/{count}</span>}</dt><dd className="text-right tabular-nums">{format(value.amount)} {t(`aiCenter.usage.unit.${labels[2]}`, labels[2])}</dd></div>
        })}
        {visibleUnits.length === 0 && <div>{Object.keys(dimensions).length === 0 ? (t("aiCenter.usage.usageWasNotReported", "Usage was not reported.")) : (t("aiCenter.usage.noNonZeroUsage", "No non-zero usage."))}</div>}
      </dl>
    </div>
  </details></ClickAwayListener>
}

export function UsageMoney({ totals, empty = '--', compact = false, hint }: { totals: Money[]; empty?: string; compact?: boolean; hint?: string }) {
  const { locale, t } = useI18n()
  const detailsRef = useRef<HTMLDetailsElement>(null)
  const ordered = [...totals].sort((a, b) => Number(b.currency === 'USD') - Number(a.currency === 'USD') || a.currency.localeCompare(b.currency))
  const format = (money: Money) => `${money.currency} ${money.amount.toLocaleString(locale, { maximumFractionDigits: 6 })}`
  if (!ordered.length || (!compact && ordered.length < 2)) return <span className="whitespace-nowrap tabular-nums" title={hint}>{ordered.length ? format(ordered[0]) : empty}</span>
  return <ClickAwayListener mouseEvent="onPointerDown" touchEvent={false} onClickAway={() => { if (detailsRef.current) detailsRef.current.open = false }}><details ref={detailsRef} className="relative" data-testid="usage-currencies" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { event.stopPropagation(); if (event.key === 'Escape') event.currentTarget.open = false }}>
    <summary className="cursor-pointer truncate underline decoration-dotted underline-offset-4 tabular-nums" style={{ color: 'var(--cp-accent)' }} aria-label={t("aiCenter.usage.costByCurrency", "Cost by currency")} title={[format(ordered[0]), hint].filter(Boolean).join('\n')}>{compact && /^[A-Z]{3}$/.test(ordered[0].currency) ? ordered[0].amount.toLocaleString(locale, { style: 'currency', currency: ordered[0].currency, currencyDisplay: 'narrowSymbol', notation: 'compact', maximumSignificantDigits: 3 }) : format(ordered[0])}</summary>
    <dl className="absolute right-0 z-30 mt-2 min-w-44 rounded-lg p-3 text-xs shadow-xl" style={{ background: 'var(--cp-surface)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}>
      {ordered.map((money) => <div key={money.currency} className="flex justify-between gap-4 py-1"><dt>{money.currency}</dt><dd className="tabular-nums">{money.amount.toLocaleString(locale, { maximumFractionDigits: 6 })}</dd></div>)}
    </dl>
  </details></ClickAwayListener>
}

export function ConsumptionValue({ total, compact = false }: { total: UsageTotals; compact?: boolean }) {
  const { t } = useI18n()
  return <div className={compact ? "flex min-w-0 flex-col items-end gap-1" : "flex shrink-0 flex-col items-end gap-1"}>
    <UsageUnits tokens={total.total_tokens} dimensions={total.dimensions} count={total.total_requests} compact={compact} />
    <div className="min-w-0 max-w-full text-xs"><UsageMoney compact={compact} totals={total.finance_totals} empty="--" hint={total.finance_complete ? undefined : `${t("aiCenter.usage.knownCostSubtotalUnpriced", "Known cost subtotal · unpriced")} ${total.unpriced_requests}`} /></div>
  </div>
}

export function UsageConsumption({ page, loading, error, groupHasMore, groupLoading, groupErrors, onRetryTotal, onRetryGroup, onLoadMore }: { page: UsageEventsPage; loading: boolean; error: string | null; groupHasMore?: Record<string, boolean>; groupLoading?: Record<string, boolean>; groupErrors?: Record<string, string | null>; onRetryTotal?: () => void; onRetryGroup?: (groupBy: string) => void; onLoadMore?: (groupBy: string) => void }) {
  const { locale, t } = useI18n()
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})
  const groups = [ ['provider_instance_name', 'Providers'], ['provider_model', 'Models'], ['caller_app_id', 'Apps / Agents'], ['capability', 'API types'] ]
  return <section data-testid="usage-consumption" className="aicc-spend-breakdown rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
    <div className="flex items-start justify-between gap-4 mb-4"><div><h3 className="text-sm font-medium">{t("aiCenter.usage.aiSpendBreakdownFilteredResults", "AI Spend Breakdown · filtered results")}</h3><p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>{loading ? (t("aiCenter.usage.loading", "Loading…")) : error ? (t("aiCenter.usage.couldNotLoadFilteredTotals", "Could not load filtered totals")) : `${page.total.total_requests.toLocaleString(locale)} ${t("aiCenter.usage.matchingRecordsAllPages", "matching records, all pages")}`}</p>{error && onRetryTotal && <button type="button" className="text-xs underline" onClick={onRetryTotal}>{t("aiCenter.usage.retry", "Retry")}</button>}</div>{!loading && (!error || page.totalRequests > 0) && <ConsumptionValue total={page.total} />}</div>
    <div className="aicc-spend-grid grid">{groups.map(([key, english]) => {
      const map = new Map<string, UsageTotals[]>()
      for (const row of page.grouped) {
        if (!(key in row.group)) continue
        const label = row.group[key] || (key === 'caller_app_id' ? 'system' : '—')
        map.set(label, [...(map.get(label) ?? []), row.aggregate])
      }
      const rows = Array.from(map, ([label, totals]) => ({ label, total: mergeUsageTotals(totals) })).sort((a,b) => b.total.total_tokens - a.total.total_tokens || b.total.total_requests - a.total.total_requests)
      return <section key={key} data-testid={`usage-consumption-${key}`} aria-busy={groupLoading?.[key] === true} className="flex min-w-0 flex-col rounded-lg p-3" style={{ border: '1px solid var(--cp-border)' }}><h4 className="mb-3 min-h-10 text-xs font-medium">{t(`aiCenter.usage.group.${key}`, english)}{!groupLoading?.[key] && !groupErrors?.[key] && <> · {rows.length}{groupHasMore?.[key] ? '+' : ''}</>}</h4>{groupLoading?.[key] && <p role="status" className="mb-2 text-xs">{t("aiCenter.usage.loading", "Loading…")}</p>}{groupErrors?.[key] && <div role="alert" className="mb-2 text-xs">{groupErrors[key]} {onRetryGroup && <button type="button" className="underline" onClick={() => onRetryGroup(key)}>{t("aiCenter.usage.retry", "Retry")}</button>}</div>}{(expanded[key] ? rows : rows.slice(0,4)).map(({label,total}) => <div key={label} className="mb-3"><div className="grid min-w-0 grid-cols-2 items-center gap-2"><span className="min-w-0 truncate text-xs" title={label}>{label}</span><ConsumptionValue total={total} compact /></div><div className="mt-1 h-1 rounded" style={{ background: 'var(--cp-bg)' }}><div className="h-1 rounded" style={{ background: 'var(--cp-accent)', width: `${!loading && !error && page.total.total_tokens ? total.total_tokens / page.total.total_tokens * 100 : 0}%` }} /></div></div>)}{expanded[key] && groupHasMore?.[key] && onLoadMore && <button type="button" className="mr-3 text-xs" style={{ color: 'var(--cp-accent)' }} disabled={groupLoading?.[key] === true} onClick={() => onLoadMore(key)}>{t("aiCenter.usage.loadMore", "Load more")}</button>}{rows.length > 4 && <button type="button" className="text-xs" style={{ color: 'var(--cp-accent)' }} onClick={() => setExpanded({ ...expanded, [key]: !expanded[key] })}>{expanded[key] ? (t("aiCenter.usage.showLess", "Show less")) : (t("aiCenter.usage.showMore", "Show more"))}</button>}</section>
    })}</div>
  </section>
}

export function UsageCost({ event }: { event: import('../../mock/types').UsageEvent }) {
  const { locale, t } = useI18n()
  const cost = event.finance_snapshot
  const source = event.usage?.cost_source
  const reason = String(event.usage?.cost_unknown_reason ?? '')
  const pending = String(source ?? '').includes('pending_billing') || source === 'billing_pending'
  let explanation = t("aiCenter.usage.costWasNotReportedOrCouldNotBeCalculated", "Cost was not reported or could not be calculated.")
  if (reason.startsWith('billing_retries_exhausted:')) {
    explanation = t('aiCenter.usage.billingUnresolved', 'Billing reconciliation ended without a reliable cost.')
  } else if (reason.startsWith('missing_usage:')) {
    const key = reason.split(':')[1]
    const fields: Record<string,string> = { image: 'image_units', audio_second: 'audio_seconds', video_second: 'video_seconds', character: 'characters', reference_image: 'reference_images', request: 'request_units', second: 'compute_seconds', megapixel: 'megapixels' }
    const labels = usageUnits[(fields[key] ?? key) as keyof typeof usageUnits]
    explanation = (t("aiCenter.usage.missingUsage", "Missing usage: ")) + ((labels ? t(`aiCenter.usage.dimension.${fields[key] ?? key}`, labels[0]) : undefined) ?? (t("aiCenter.usage.requiredQuantity", "required quantity")))
  } else if (reason.startsWith('missing_rate:')) explanation = t("aiCenter.usage.aRequiredPriceIsMissing", "A required price is missing.")
  else if (reason === 'missing_pricing') explanation = t("aiCenter.usage.noVerifiedModelPrice", "No verified model price.")
  else if (reason === 'unverified_reported_currency') explanation = t("aiCenter.usage.reportedAmountHasNoVerifiedCurrency", "Reported amount has no verified currency.")
  else if (reason === 'missing_token_usage') explanation = t("aiCenter.usage.tokenUsageWasNotReported", "Token usage was not reported.")
  if (cost?.currency && cost.amount !== undefined) {
    const label = source === 'provider_billing' ? (t("aiCenter.usage.providerBillingEventActualCharge", "Provider billing event · actual charge")) : String(source ?? '').includes('provider_reported') ? (t("aiCenter.usage.providerReportedCost", "Provider-reported cost")) : String(source ?? '').includes('pric') ? (t("aiCenter.usage.estimatedFromFrozenPrices", "Estimated from frozen prices")) : (t("aiCenter.usage.recordedCost", "Recorded cost"))
    return <span title={label}>{cost.currency} {cost.amount.toLocaleString(locale, { maximumFractionDigits: 6 })}{pending && <span className="block text-[10px]" style={{ color: 'var(--cp-muted)' }}>{t("aiCenter.usage.awaitingBilling", "Awaiting billing")}</span>}</span>
  }
  return <span title={pending ? `${t("aiCenter.usage.awaitingBilling", "Awaiting billing")} · ${explanation}` : explanation}>--</span>
}

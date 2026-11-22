import { useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import type { Money, UsageDimension, UsageTotals } from '../../mock/types'
import type { UsageEventsPage } from '../../../../api/aicc_mgr'
import { mergeUsageTotals, usageUnits } from '../../datamodel/usage-units'

export function UsageUnits({ tokens, dimensions, count }: { tokens: number; dimensions: Record<string, UsageDimension>; count: number }) {
  const { locale } = useI18n()
  const en = locale === 'en'
  const format = (value: number) => value.toLocaleString(en ? 'en-US' : 'zh-CN', { maximumFractionDigits: 6 })
  return <details className="relative" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { event.stopPropagation(); if (event.key === 'Escape') event.currentTarget.open = false }}>
    <summary className="cursor-pointer whitespace-nowrap text-xs underline decoration-dotted underline-offset-4" style={{ color: 'var(--cp-accent)' }} aria-label={en ? 'Usage details' : '用量明细'}>{format(tokens)} tokens</summary>
    <div className="absolute right-0 z-30 mt-2 w-72 max-w-[85vw] rounded-lg p-3 shadow-xl text-left" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
      <p className="mb-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{en ? 'Subsets overlap; do not add them to the total.' : '缓存、多模态、推理为细分子项，不重复累加至总量。'}</p>
      <dl className="max-h-72 overflow-y-auto text-xs">
        {Object.entries(usageUnits).filter(([key]) => dimensions[key]?.reported_events > 0).map(([key, labels]) => {
          const value = dimensions[key]
          return <div key={key} className="flex justify-between gap-3 py-1"><dt>{labels[en ? 0 : 1]}{value.reported_events < count && <span className="block text-[10px]" style={{ color: 'var(--cp-warning)' }}>{en ? 'Known subtotal' : '已知小计'} · {value.reported_events}/{count}</span>}</dt><dd className="text-right tabular-nums">{format(value.amount)} {en ? labels[2] : ({ images: '张', s: '秒', chars: '字符', requests: '请求单位', MP: '百万像素', tokens: 'tokens' }[labels[2]])}</dd></div>
        })}
        {Object.keys(dimensions).length === 0 && <div>{en ? 'Usage was not reported.' : '未上报用量。'}</div>}
      </dl>
    </div>
  </details>
}

export function UsageMoney({ totals, empty = '—' }: { totals: Money[]; empty?: string }) {
  const { locale } = useI18n()
  const en = locale === 'en'
  const ordered = [...totals].sort((a, b) => Number(b.currency === 'USD') - Number(a.currency === 'USD') || a.currency.localeCompare(b.currency))
  const format = (money: Money) => `${money.currency} ${money.amount.toLocaleString(en ? 'en-US' : 'zh-CN', { maximumFractionDigits: 6 })}`
  if (ordered.length < 2) return <span className="whitespace-nowrap tabular-nums">{ordered.length ? format(ordered[0]) : empty}</span>
  return <details className="relative" data-testid="usage-currencies" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { event.stopPropagation(); if (event.key === 'Escape') event.currentTarget.open = false }}>
    <summary className="cursor-pointer whitespace-nowrap underline decoration-dotted underline-offset-4 tabular-nums" style={{ color: 'var(--cp-accent)' }} aria-label={en ? 'Cost by currency' : '各币种费用'}>{format(ordered[0])}</summary>
    <dl className="absolute right-0 z-30 mt-2 min-w-44 rounded-lg p-3 text-xs shadow-xl" style={{ background: 'var(--cp-surface)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}>
      {ordered.map((money) => <div key={money.currency} className="flex justify-between gap-4 py-1"><dt>{money.currency}</dt><dd className="tabular-nums">{money.amount.toLocaleString(en ? 'en-US' : 'zh-CN', { maximumFractionDigits: 6 })}</dd></div>)}
    </dl>
  </details>
}

export function ConsumptionValue({ total }: { total: UsageTotals }) {
  const { locale } = useI18n()
  const en = locale === 'en'
  return <div className="flex shrink-0 flex-col items-end gap-1">
    <UsageUnits tokens={total.total_tokens} dimensions={total.dimensions} count={total.total_requests} />
    <div className="text-xs"><UsageMoney totals={total.finance_totals} empty={total.total_requests === 0 ? '—' : (en ? 'Unknown cost' : '费用未知')} /></div>
    {!total.finance_complete && <span className="text-[10px]" style={{ color: 'var(--cp-warning)' }}>{en ? 'Known cost subtotal · unpriced' : '已知费用小计 · 未计价'} {total.unpriced_requests}</span>}
  </div>
}

export function UsageConsumption({ page, loading, error }: { page: UsageEventsPage; loading: boolean; error: string | null }) {
  const { locale } = useI18n()
  const en = locale === 'en'
  const [expanded, setExpanded] = useState<Record<string, boolean>>({})
  const groups = [ ['provider_instance_name', 'Providers', 'Provider'], ['provider_model', 'Models', '模型'], ['caller_app_id', 'Apps / Agents', 'App / Agent'], ['capability', 'API types', 'API 类型'] ]
  return <section data-testid="usage-consumption" className="rounded-xl p-4" aria-busy={loading} style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}>
    <div className="flex items-start justify-between gap-4 mb-4"><div><h3 className="text-sm font-medium">{en ? 'AI Spend Breakdown · filtered results' : 'AI 消费分布 · 当前筛选结果'}</h3><p className="mt-1 text-xs" style={{ color: 'var(--cp-muted)' }}>{loading ? (en ? 'Loading…' : '加载中…') : error ? (en ? 'Could not load filtered totals' : '筛选统计加载失败') : `${page.total.total_requests} ${en ? 'matching records, all pages' : '条匹配记录，包含全部分页'}`}</p></div>{!loading && !error && <ConsumptionValue total={page.total} />}</div>
    {!loading && !error && <div className="grid gap-4" style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(min(100%, 240px), 1fr))' }}>{groups.map(([key, english, chinese]) => {
      const map = new Map<string, UsageTotals[]>()
      for (const row of page.grouped) {
        const label = row.group[key] || (key === 'caller_app_id' ? 'system' : '—')
        map.set(label, [...(map.get(label) ?? []), row.aggregate])
      }
      const rows = Array.from(map, ([label, totals]) => ({ label, total: mergeUsageTotals(totals) })).sort((a,b) => b.total.total_tokens - a.total.total_tokens || b.total.total_requests - a.total.total_requests)
      return <section key={key} className="min-w-0 rounded-lg p-3" style={{ border: '1px solid var(--cp-border)' }}><h4 className="mb-3 text-xs font-medium">{en ? english : chinese} · {rows.length}</h4>{(expanded[key] ? rows : rows.slice(0,4)).map(({label,total}) => <div key={label} className="mb-3"><div className="flex justify-between gap-2"><span className="min-w-0 truncate text-xs" title={label}>{label}</span><ConsumptionValue total={total} /></div><div className="mt-1 h-1 rounded" style={{ background: 'var(--cp-bg)' }}><div className="h-1 rounded" style={{ background: 'var(--cp-accent)', width: `${page.total.total_tokens ? total.total_tokens / page.total.total_tokens * 100 : 0}%` }} /></div></div>)}{rows.length > 4 && <button type="button" className="text-xs" style={{ color: 'var(--cp-accent)' }} onClick={() => setExpanded({ ...expanded, [key]: !expanded[key] })}>{expanded[key] ? (en ? 'Show less' : '收起') : (en ? 'View all' : '查看全部')}</button>}</section>
    })}</div>}
  </section>
}

export function UsageCost({ event }: { event: import('../../mock/types').UsageEvent }) {
  const { locale } = useI18n()
  const en = locale === 'en'
  const cost = event.finance_snapshot
  const source = event.usage?.cost_source
  const reason = String(event.usage?.cost_unknown_reason ?? '')
  let explanation = en ? 'Cost was not reported or could not be calculated.' : '未上报费用或暂时无法计算。'
  if (reason.startsWith('missing_usage:')) {
    const key = reason.split(':')[1]
    const fields: Record<string,string> = { image: 'image_units', audio_second: 'audio_seconds', video_second: 'video_seconds', character: 'characters', reference_image: 'reference_images', request: 'request_units', second: 'compute_seconds', megapixel: 'megapixels' }
    const labels = usageUnits[(fields[key] ?? key) as keyof typeof usageUnits]
    explanation = (en ? 'Missing usage: ' : '未上报用量：') + (labels?.[en ? 0 : 1] ?? (en ? 'required quantity' : '所需计费量'))
  } else if (reason.startsWith('missing_rate:')) explanation = en ? 'A required price is missing.' : '缺少所需计费单价。'
  else if (reason === 'missing_pricing') explanation = en ? 'No verified model price.' : '尚无已核实的模型价格。'
  else if (reason === 'unverified_reported_currency') explanation = en ? 'Reported amount has no verified currency.' : '返回金额的币种尚未核实。'
  else if (reason === 'missing_token_usage') explanation = en ? 'Token usage was not reported.' : '未上报所需 token 用量。'
  if (cost?.currency && cost.amount !== undefined) {
    const label = String(source ?? '').includes('provider_reported') ? (en ? 'Provider-reported cost' : 'Provider 实报费用') : String(source ?? '').includes('pric') ? (en ? 'Estimated from frozen prices' : '根据调用时价格估算') : (en ? 'Recorded cost' : '已记录费用')
    return <span title={label}>{cost.currency} {cost.amount.toLocaleString(en ? 'en-US' : 'zh-CN', { maximumFractionDigits: 6 })}</span>
  }
  return <span title={explanation} className="inline-flex flex-col"><span>{en ? 'Unknown cost' : '费用未知'}</span><span className="text-[10px]" style={{ color: 'var(--cp-muted)' }}>{explanation}</span></span>
}

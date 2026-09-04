import clsx from 'clsx'
import { ArrowDownRight, ArrowUpRight } from 'lucide-react'
import type { CanvasBlockOf } from '../../domain/types'
import { formatNumber } from '../../domain/selectors'

export function MetricBlockView({ block }: { block: CanvasBlockOf<'metric'> }) {
  const c = block.content
  const value = typeof c.value === 'number' ? formatNumber(c.value, c.format ?? 'plain') : c.value
  const tone = c.tone ?? 'neutral'
  const toneColor =
    tone === 'positive' ? 'var(--cp-success)' : tone === 'negative' ? 'var(--cp-danger)' : tone === 'warning' ? 'var(--cp-warning)' : 'var(--cp-accent)'
  const delta = c.delta
  return (
    <div className="flex h-full flex-col justify-between p-3">
      <div className="text-[11px] font-semibold uppercase tracking-wide text-[color:var(--cp-muted)]">{c.label}</div>
      <div className="flex items-baseline gap-1">
        <span className="font-display text-[26px] font-semibold leading-none" style={{ color: toneColor }}>
          {value}
        </span>
        {c.unit ? <span className="text-xs text-[color:var(--cp-muted)]">{c.unit}</span> : null}
      </div>
      <div className="flex items-center gap-2 text-[11px] text-[color:var(--cp-muted)]">
        {delta ? (
          <span className={clsx('inline-flex items-center gap-[2px] font-medium [&>svg]:size-[12px]', delta.value >= 0 ? 'text-[color:var(--cp-success)]' : 'text-[color:var(--cp-danger)]')}>
            {delta.value >= 0 ? <ArrowUpRight /> : <ArrowDownRight />}
            {delta.format === 'percent' ? `${(Math.abs(delta.value) * 100).toFixed(1)}%` : formatNumber(Math.abs(delta.value))}
            {delta.label ? <span className="ml-1 font-normal text-[color:var(--cp-muted)]">{delta.label}</span> : null}
          </span>
        ) : null}
        {c.note ? <span className="truncate">{c.note}</span> : null}
      </div>
    </div>
  )
}

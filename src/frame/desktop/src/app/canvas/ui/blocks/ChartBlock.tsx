/* ── Host-rendered declarative charts (bar / horizontalBar / line / pie) as SVG ── */

import { useMemo } from 'react'
import type { CanvasBlockOf } from '../../domain/types'
import { formatNumber } from '../../domain/selectors'
import { useStoreState } from '../../store/hooks'
import { resolveChart } from '../chart-data'

const PALETTE = ['oklch(0.63 0.152 257)', 'oklch(0.72 0.13 160)', 'oklch(0.76 0.12 65)', 'oklch(0.62 0.15 300)', 'oklch(0.66 0.14 25)', 'oklch(0.7 0.1 200)']

function niceMax(v: number): number {
  if (v <= 0) return 1
  const p = 10 ** Math.floor(Math.log10(v))
  const n = v / p
  const m = n <= 1 ? 1 : n <= 2 ? 2 : n <= 2.5 ? 2.5 : n <= 5 ? 5 : 10
  return m * p
}

function short(n: number, fmt: 'plain' | 'percent' | 'currency' = 'plain'): string {
  if (fmt === 'percent') return `${(n * 100).toFixed(0)}%`
  const abs = Math.abs(n)
  const prefix = fmt === 'currency' ? '¥' : ''
  if (abs >= 1e8) return `${prefix}${(n / 1e8).toFixed(1)}亿`
  if (abs >= 1e4) return `${prefix}${(n / 1e4).toFixed(abs >= 1e6 ? 0 : 1)}万`
  return `${prefix}${formatNumber(n)}`
}

export function ChartBlockView({ block }: { block: CanvasBlockOf<'chart'> }) {
  const { doc } = useStoreState()
  const c = block.content
  const data = useMemo(() => resolveChart(doc, c), [doc, c])
  const width = Math.max(120, block.rect.width - 2)
  const height = Math.max(80, block.rect.height - 28 - (c.caption ? 22 : 0))
  const fmt = c.numberFormat ?? 'plain'
  const summary = `${c.caption ?? block.title ?? '图表'}：${data.categories.map((cat, i) => `${cat} ${data.series.map((s) => `${s.name} ${formatNumber(s.values[i], fmt)}`).join('，')}`).join('；')}`

  if (!data.categories.length || !data.series.length) {
    return <div className="flex h-full items-center justify-center p-3 text-xs text-[color:var(--cp-muted)]">没有可绘制的数据，请在右侧面板选择字段</div>
  }

  return (
    <div className="flex h-full flex-col" title={summary}>
      <svg width="100%" height={height} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={summary} className="block">
        {c.chartType === 'pie' ? <Pie data={data} w={width} h={height} fmt={fmt} /> : c.chartType === 'horizontalBar' ? <HBar data={data} w={width} h={height} fmt={fmt} /> : <XY data={data} w={width} h={height} fmt={fmt} line={c.chartType === 'line'} />}
      </svg>
      {c.caption ? <div className="truncate px-3 pb-1 text-[11px] text-[color:var(--cp-muted)]">{c.caption}</div> : null}
    </div>
  )
}

type D = ReturnType<typeof resolveChart>

function Legend({ data, x, y }: { data: D; x: number; y: number }) {
  if (data.series.length < 2) return null
  return (
    <g transform={`translate(${x},${y})`}>
      {data.series.map((s, i) => (
        <g key={s.name} transform={`translate(${i * 90},0)`}>
          <rect width="10" height="10" rx="2" fill={PALETTE[i % PALETTE.length]} />
          <text x="14" y="9" fontSize="10" fill="var(--cp-muted)">
            {s.name}
          </text>
        </g>
      ))}
    </g>
  )
}

function XY({ data, w, h, fmt, line }: { data: D; w: number; h: number; fmt: 'plain' | 'percent' | 'currency'; line: boolean }) {
  const pad = { l: 52, r: 12, t: 22, b: 30 }
  const iw = w - pad.l - pad.r
  const ih = h - pad.t - pad.b
  const max = niceMax(Math.max(...data.series.flatMap((s) => s.values), 0))
  const min = Math.min(0, ...data.series.flatMap((s) => s.values))
  const range = max - min || 1
  const y = (v: number) => pad.t + ih - ((v - min) / range) * ih
  const n = data.categories.length
  const slot = iw / n
  const ticks = [0, 0.25, 0.5, 0.75, 1].map((f) => min + f * range)
  const groupW = slot * 0.7
  const barW = groupW / data.series.length
  return (
    <g>
      {ticks.map((t) => (
        <g key={t}>
          <line x1={pad.l} x2={w - pad.r} y1={y(t)} y2={y(t)} stroke="var(--cp-border)" strokeDasharray="2 3" />
          <text x={pad.l - 6} y={y(t) + 3} fontSize="10" textAnchor="end" fill="var(--cp-muted)">
            {short(t, fmt)}
          </text>
        </g>
      ))}
      {data.categories.map((cat, i) => (
        <text key={cat + i} x={pad.l + slot * i + slot / 2} y={h - pad.b + 14} fontSize="10" textAnchor="middle" fill="var(--cp-muted)">
          {cat.length > 6 ? `${cat.slice(0, 6)}…` : cat}
        </text>
      ))}
      {line
        ? data.series.map((s, si) => (
            <g key={s.name}>
              <polyline fill="none" stroke={PALETTE[si % PALETTE.length]} strokeWidth="2" points={s.values.map((v, i) => `${pad.l + slot * i + slot / 2},${y(v)}`).join(' ')} />
              {s.values.map((v, i) => (
                <circle key={i} cx={pad.l + slot * i + slot / 2} cy={y(v)} r="3" fill={PALETTE[si % PALETTE.length]}>
                  <title>{`${data.categories[i]} · ${s.name}: ${formatNumber(v, fmt)}`}</title>
                </circle>
              ))}
            </g>
          ))
        : data.series.map((s, si) =>
            s.values.map((v, i) => {
              const x = pad.l + slot * i + (slot - groupW) / 2 + barW * si
              const top = Math.min(y(v), y(0))
              const hh = Math.abs(y(v) - y(0))
              return (
                <rect key={`${si}-${i}`} x={x} y={top} width={Math.max(1, barW - 2)} height={Math.max(1, hh)} rx="2" fill={PALETTE[si % PALETTE.length]}>
                  <title>{`${data.categories[i]} · ${s.name}: ${formatNumber(v, fmt)}`}</title>
                </rect>
              )
            }),
          )}
      <Legend data={data} x={pad.l} y={6} />
    </g>
  )
}

function HBar({ data, w, h, fmt }: { data: D; w: number; h: number; fmt: 'plain' | 'percent' | 'currency' }) {
  const pad = { l: 80, r: 50, t: 10, b: 10 }
  const iw = w - pad.l - pad.r
  const ih = h - pad.t - pad.b
  const max = niceMax(Math.max(...data.series.flatMap((s) => s.values), 0))
  const n = data.categories.length
  const slot = ih / n
  const barH = (slot * 0.7) / data.series.length
  return (
    <g>
      {data.categories.map((cat, i) => (
        <g key={cat + i}>
          <text x={pad.l - 6} y={pad.t + slot * i + slot / 2 + 3} fontSize="10" textAnchor="end" fill="var(--cp-muted)">
            {cat.length > 8 ? `${cat.slice(0, 8)}…` : cat}
          </text>
          {data.series.map((s, si) => {
            const v = s.values[i]
            const bw = (Math.max(0, v) / max) * iw
            const yy = pad.t + slot * i + (slot - barH * data.series.length) / 2 + barH * si
            return (
              <g key={si}>
                <rect x={pad.l} y={yy} width={Math.max(1, bw)} height={Math.max(1, barH - 1)} rx="2" fill={PALETTE[si % PALETTE.length]}>
                  <title>{`${cat} · ${s.name}: ${formatNumber(v, fmt)}`}</title>
                </rect>
                {si === 0 ? (
                  <text x={pad.l + bw + 4} y={yy + barH / 2 + 3} fontSize="10" fill="var(--cp-muted)">
                    {short(v, fmt)}
                  </text>
                ) : null}
              </g>
            )
          })}
        </g>
      ))}
    </g>
  )
}

function Pie({ data, w, h, fmt }: { data: D; w: number; h: number; fmt: 'plain' | 'percent' | 'currency' }) {
  const s = data.series[0]
  const total = s.values.reduce((a, b) => a + Math.max(0, b), 0) || 1
  const r = Math.min(w * 0.5, h) / 2 - 10
  const cx = w * 0.3
  const cy = h / 2
  const fracs = s.values.map((v) => Math.max(0, v) / total)
  const starts = fracs.map((_, i) => -Math.PI / 2 + fracs.slice(0, i).reduce((a, b) => a + b, 0) * Math.PI * 2)
  const slices = fracs.map((frac, i) => {
    const a0 = starts[i]
    const a1 = a0 + frac * Math.PI * 2
    const large = a1 - a0 > Math.PI ? 1 : 0
    const p = (a: number) => `${cx + r * Math.cos(a)},${cy + r * Math.sin(a)}`
    return { d: `M${cx},${cy} L${p(a0)} A${r},${r} 0 ${large} 1 ${p(a1)} Z`, frac, i }
  })
  return (
    <g>
      {slices.map((sl) => (
        <path key={sl.i} d={sl.d} fill={PALETTE[sl.i % PALETTE.length]} stroke="var(--cp-surface-opaque)" strokeWidth="1">
          <title>{`${data.categories[sl.i]}: ${formatNumber(s.values[sl.i], fmt)} (${(sl.frac * 100).toFixed(1)}%)`}</title>
        </path>
      ))}
      {data.categories.slice(0, 8).map((cat, i) => (
        <g key={cat + i} transform={`translate(${w * 0.6},${cy - (Math.min(8, data.categories.length) * 16) / 2 + i * 16})`}>
          <rect width="10" height="10" rx="2" fill={PALETTE[i % PALETTE.length]} />
          <text x="14" y="9" fontSize="10" fill="var(--cp-text)">
            {cat} · {(slices[i].frac * 100).toFixed(0)}%
          </text>
        </g>
      ))}
    </g>
  )
}

/**
 * Daily views and downloads (same unit, one axis). Two series: legend plus
 * direct end labels; crosshair tooltip on hover; a table view for screen
 * readers and exact values. Colors are the validated categorical slots 1–2
 * (light and dark steps in my-content.css). Day buckets are UTC days
 * (nfs-server groups by `bucket - bucket % 86400`), so they are labelled in UTC.
 */

import { useMemo, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { ShareStatsPoint } from '../../api/shares'
import { DAY, formatBytes, formatDate } from '../../sysdlg/shareFormat'

const PAD = { top: 12, right: 64, bottom: 24, left: 36 }

interface Props {
  series: ShareStatsPoint[]
  from: number
  to: number
  /** Drawing width in CSS pixels at 1:1; pick it near the container width so text stays legible. */
  width?: number
  testId?: string
}

/** One point per day across the range (missing days are zero). */
function fillDays(series: ShareStatsPoint[], from: number, to: number): ShareStatsPoint[] {
  const byDay = new Map(series.map((p) => [p.t - (p.t % DAY), p]))
  const out: ShareStatsPoint[] = []
  for (let t = from - (from % DAY); t < to; t += DAY) {
    out.push(byDay.get(t) ?? { t, views: 0, downloads: 0, bytes: 0, visitors: 0 })
  }
  return out
}

function niceMax(v: number) {
  if (v <= 4) return 4
  const pow = 10 ** Math.floor(Math.log10(v))
  const step = [1, 2, 2.5, 5, 10].find((s) => s * pow * 4 >= v) ?? 10
  return step * pow * 4
}

export function ConsumptionChart({ series, from, to, width = 640, testId }: Props) {
  const W = width
  const H = width < 500 ? 160 : 180
  const { t, locale } = useI18n()
  const [hover, setHover] = useState<number | null>(null)
  const days = useMemo(() => fillDays(series, from, to), [series, from, to])
  const max = niceMax(Math.max(1, ...days.map((d) => Math.max(d.views, d.downloads))))
  const innerW = W - PAD.left - PAD.right
  const innerH = H - PAD.top - PAD.bottom
  const x = (i: number) => PAD.left + (days.length <= 1 ? innerW / 2 : (i * innerW) / (days.length - 1))
  const y = (v: number) => PAD.top + innerH - (v / max) * innerH
  const path = (key: 'views' | 'downloads') => days.map((d, i) => `${i ? 'L' : 'M'}${x(i).toFixed(1)},${y(d[key]).toFixed(1)}`).join('')
  const ticks = [0, max / 4, max / 2, (3 * max) / 4, max]
  const last = days[days.length - 1]
  const labelGap = Math.abs(y(last?.views ?? 0) - y(last?.downloads ?? 0)) < 14
  const hovered = hover == null ? null : days[hover]

  const onMove = (e: React.PointerEvent<SVGRectElement>) => {
    const rect = e.currentTarget.getBoundingClientRect()
    const rel = ((e.clientX - rect.left) / rect.width) * innerW
    const i = Math.round((rel / innerW) * (days.length - 1))
    setHover(Math.max(0, Math.min(days.length - 1, i)))
  }

  return (
    <figure className="mc-chart" data-testid={testId}>
      <div className="mc-legend" aria-hidden>
        <span><i className="mc-swatch mc-views" />{t('myContent.views')}</span>
        <span><i className="mc-swatch mc-downloads" />{t('myContent.downloads')}</span>
      </div>
      <div className="relative">
        <svg viewBox={`0 0 ${W} ${H}`} className="block h-auto w-full" role="img" aria-label={`${t('myContent.views')} / ${t('myContent.downloads')}`}>
          {ticks.map((v) => (
            <g key={v}>
              <line x1={PAD.left} x2={W - PAD.right} y1={y(v)} y2={y(v)} className="mc-grid" />
              <text x={PAD.left - 6} y={y(v) + 3} textAnchor="end" className="mc-axis">{Math.round(v)}</text>
            </g>
          ))}
          {days.length > 0 && [0, Math.floor((days.length - 1) / 2), days.length - 1].map((i) => (
            <text key={i} x={x(i)} y={H - 6} textAnchor={i === 0 ? 'start' : i === days.length - 1 ? 'end' : 'middle'} className="mc-axis">{formatDate(days[i].t, locale, true)}</text>
          ))}
          <path d={path('downloads')} className="mc-line mc-downloads" />
          <path d={path('views')} className="mc-line mc-views" />
          {last && (
            <>
              <text x={x(days.length - 1) + 6} y={y(last.views) + (labelGap && last.views >= last.downloads ? -4 : 4)} className="mc-direct">{t('myContent.views')}</text>
              <text x={x(days.length - 1) + 6} y={y(last.downloads) + (labelGap && last.downloads > last.views ? -4 : labelGap ? 12 : 4)} className="mc-direct">{t('myContent.downloads')}</text>
            </>
          )}
          {hovered && hover != null && (
            <g>
              <line x1={x(hover)} x2={x(hover)} y1={PAD.top} y2={PAD.top + innerH} className="mc-crosshair" />
              <circle cx={x(hover)} cy={y(hovered.views)} r={4} className="mc-dot mc-views" />
              <circle cx={x(hover)} cy={y(hovered.downloads)} r={4} className="mc-dot mc-downloads" />
            </g>
          )}
          <rect
            x={PAD.left}
            y={PAD.top}
            width={innerW}
            height={innerH}
            fill="transparent"
            onPointerMove={onMove}
            onPointerLeave={() => setHover(null)}
            data-testid={testId ? `${testId}-hit` : undefined}
          />
        </svg>
        {hovered && hover != null && (
          <div className="mc-tooltip" role="status" style={{ left: `${(x(hover) / W) * 100}%` }} data-testid={testId ? `${testId}-tooltip` : undefined}>
            <strong>{formatDate(hovered.t, locale, true)}</strong>
            <span><i className="mc-swatch mc-views" />{t('myContent.views')} {hovered.views}</span>
            <span><i className="mc-swatch mc-downloads" />{t('myContent.downloads')} {hovered.downloads}</span>
            <span className="text-[var(--cp-muted)]">{t('myContent.traffic')} {formatBytes(hovered.bytes)}</span>
          </div>
        )}
      </div>
      <details className="mt-1 text-xs">
        <summary className="cursor-pointer text-[var(--cp-muted)]">{t('myContent.table')}</summary>
        <table className="mc-table">
          <thead>
            <tr><th>{t('myContent.date')}</th><th>{t('myContent.views')}</th><th>{t('myContent.downloads')}</th><th>{t('myContent.visitors')}</th><th>{t('myContent.traffic')}</th></tr>
          </thead>
          <tbody>
            {days.filter((d) => d.views || d.downloads).map((d) => (
              <tr key={d.t}><td>{formatDate(d.t, locale, true)}</td><td>{d.views}</td><td>{d.downloads}</td><td>{d.visitors}</td><td>{formatBytes(d.bytes)}</td></tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  )
}

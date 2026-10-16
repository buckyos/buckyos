/* eslint-disable react-refresh/only-export-components -- Block definitions bundle their renderers */
/* Extension sample 1 (phase two §10.5): a metric and a bar chart Block for a TableSource,
 * registered in code without touching the Shell, selection, commit or undo. Both only read the
 * table through the session and render; the chart has a view-mode variant (hover values) and an
 * inspector for its configuration. */

import { useCallback, useEffect, useMemo, useState } from 'react'
import type { Json, QueryPage } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { AssetBlobImage } from '../extensions/AssetBlobImage'
import { cellOp } from './ops'
import { blockRegistry, type BlockDefinition, type RenderContext } from './registry'

function num(v: unknown): number {
  if (typeof v === 'number') return v
  if (typeof v === 'string') { const n = Number(v.replace(/[,¥$%\s]/g, '')); return Number.isFinite(n) ? n : 0 }
  return 0
}

interface TableData { rows: Record<string, Json>[]; fields: { field_id: string; name: string; type: string }[] }

function useTable(context: RenderContext): { data: TableData | undefined; error: string | null } {
  const store = useStore()
  const id = context.source?.entity_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(async (): Promise<TableData> => {
    const [page, meta] = await Promise.all([
      store.session.query({ source_id: id, limit: 1000, consistency: 'best_effort' }) as Promise<QueryPage>,
      store.readBatched<{ fields: TableData['fields'] }>(id),
    ])
    return { rows: page.rows.map((r) => r.values), fields: meta.content.fields }
  }, [store, id])
  const loaded = useLoad<TableData>(load, version)
  return { data: loaded.data, error: loaded.error }
}

function fieldId(data: TableData | undefined, wanted: string | undefined, fallbackType?: string): string | undefined {
  if (!data) return undefined
  if (wanted) { const hit = data.fields.find((f) => f.field_id === wanted || f.name === wanted); if (hit) return hit.field_id }
  return data.fields.find((f) => (fallbackType ? f.type === fallbackType : true))?.field_id
}

const METRIC_TITLE: Record<string, string> = {
  sum: 'aiworkspace.blocks.metric.titleSum', avg: 'aiworkspace.blocks.metric.titleAvg', count: 'aiworkspace.blocks.metric.titleCount', max: 'aiworkspace.blocks.metric.titleMax',
}

function MetricStatic(context: RenderContext) {
  const { t, num: formatNum } = useAiwsText()
  const { data, error } = useTable(context)
  const config = context.payload.config ?? {}
  const field = fieldId(data, typeof config.field === 'string' ? config.field : undefined, 'number') ?? fieldId(data, undefined, 'decimal')
  const agg = typeof config.aggregation === 'string' ? config.aggregation : 'sum'
  const values = data && field ? data.rows.map((r) => num(r[field])) : []
  const value = values.length === 0 ? 0 : agg === 'avg' ? values.reduce((a, b) => a + b, 0) / values.length : agg === 'count' ? values.length : agg === 'max' ? Math.max(...values) : values.reduce((a, b) => a + b, 0)
  const name = data?.fields.find((f) => f.field_id === field)?.name ?? field ?? ''
  return (
    <div className="aiws-metric" data-testid={`aiws-metric-${context.cell.entity_id}`}>
      <div className="aiws-decl-label">{context.payload.title ?? t(METRIC_TITLE[agg] ?? METRIC_TITLE.sum, { field: name })}</div>
      {error ? <div className="aiws-error">{error}</div> : <div className="aiws-metric-value">{formatNum(value, { maximumFractionDigits: 2 })}</div>}
      <div className="aiws-muted">{data ? t('aiworkspace.blocks.metric.rows', { count: data.rows.length }) : '…'}</div>
    </div>
  )
}

function MetricInspector(context: RenderContext) {
  const { t } = useAiwsText()
  const store = useStore()
  const { data } = useTable(context)
  const set = (key: string, value: Json) => {
    const config = { ...(context.payload.config ?? {}), [key]: value }
    void store.submit({ editId: `config:${context.cell.entity_id}`, label: t('aiworkspace.blocks.metric.editProperty', { key }), operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key: 'config', value: config as Json, expect: { rev: context.keyRevs.config ?? 0 } }] }] })
  }
  const numeric = (data?.fields ?? []).filter((f) => f.type === 'number' || f.type === 'decimal')
  return (
    <div className="aiws-inline-form" data-testid="aiws-metric-inspector">
      <label>{t('aiworkspace.blocks.metric.field')} <select value={String(context.payload.config?.field ?? '')} disabled={context.readOnlyReason !== null} onChange={(event) => set('field', event.target.value)}>
        <option value="">{t('aiworkspace.blocks.metric.firstNumber')}</option>{numeric.map((f) => <option key={f.field_id} value={f.field_id}>{f.name}</option>)}
      </select></label>
      <label>{t('aiworkspace.blocks.metric.aggregation')} <select value={String(context.payload.config?.aggregation ?? 'sum')} disabled={context.readOnlyReason !== null} onChange={(event) => set('aggregation', event.target.value)}>
        <option value="sum">{t('aiworkspace.blocks.metric.sum')}</option><option value="avg">{t('aiworkspace.blocks.metric.avg')}</option>
        <option value="count">{t('aiworkspace.blocks.metric.count')}</option><option value="max">{t('aiworkspace.blocks.metric.max')}</option>
      </select></label>
    </div>
  )
}

export const metricBlock: BlockDefinition = {
  type: 'sample.metric', version: 1, get title() { return aiwsT('aiworkspace.blocks.metric.title') }, accepts: ['buckyos.table-source'], allowNoSource: false,
  defaultSize: { w: 220, h: 120 }, cost: { editor: false, html: false },
  catalog: { group: 'sample', get description() { return aiwsT('aiworkspace.blocks.metric.description') }, needs: 'data' },
  Static: MetricStatic, Inspector: MetricInspector,
  create: (args) => [cellOp(args, 'sample.metric', args.existingSourceId, args.config ? { config: args.config } : {})],
}

function BarChart(context: RenderContext & { interactive: boolean }) {
  const { t, num: formatNum } = useAiwsText()
  const { data, error } = useTable(context)
  const [hover, setHover] = useState<string | null>(null)
  const config = context.payload.config ?? {}
  const valueField = fieldId(data, typeof config.value === 'string' ? config.value : undefined, 'number') ?? fieldId(data, undefined, 'decimal')
  const byField = fieldId(data, typeof config.by === 'string' ? config.by : undefined, 'select') ?? fieldId(data, undefined, 'text')
  const groups = useMemo(() => {
    const map = new Map<string, number>()
    if (!data || !valueField || !byField) return map
    for (const row of data.rows) {
      const key = String(row[byField] ?? '')
      const label = data.fields.find((f) => f.field_id === byField)?.type === 'select' ? key : key
      map.set(label, (map.get(label) ?? 0) + num(row[valueField]))
    }
    return map
  }, [data, valueField, byField])
  const max = Math.max(1, ...groups.values())
  const name = (id: string | undefined) => data?.fields.find((f) => f.field_id === id)?.name ?? id ?? ''
  return (
    <div className="aiws-chart" data-testid={`aiws-chart-${context.cell.entity_id}`} data-interactive={context.interactive ? 'true' : 'false'}>
      <div className="aiws-decl-label">{context.payload.title ?? t('aiworkspace.blocks.chart.titleBy', { value: name(valueField), group: name(byField) })}</div>
      {error && <div className="aiws-error">{error}</div>}
      <div className="aiws-decl-bars">
        {[...groups].map(([label, value]) => (
          <div key={label} className="aiws-decl-bar-row" onMouseEnter={context.interactive ? () => setHover(label) : undefined} onMouseLeave={context.interactive ? () => setHover(null) : undefined}>
            <span className="aiws-decl-bar-label">{label || t('aiworkspace.blocks.chart.empty')}</span>
            <span className="aiws-decl-bar" style={{ width: `${(value / max) * 100}%`, background: typeof config.color === 'string' ? config.color : 'var(--cp-accent)' }} />
            {(!context.interactive || hover === label) && <span className="aiws-decl-bar-value" data-testid="aiws-chart-value">{formatNum(value, { maximumFractionDigits: 2 })}</span>}
          </div>
        ))}
      </div>
      {context.interactive && <div className="aiws-muted">{t('aiworkspace.blocks.chart.hoverHint')}</div>}
    </div>
  )
}

const ChartStatic = (context: RenderContext) => <BarChart {...context} interactive={false} />
const ChartView = (context: RenderContext) => <BarChart {...context} interactive />

function ChartInspector(context: RenderContext) {
  const { t } = useAiwsText()
  const store = useStore()
  const { data } = useTable(context)
  const set = (key: string, value: Json) => {
    const config = { ...(context.payload.config ?? {}), [key]: value }
    void store.submit({ editId: `config:${context.cell.entity_id}`, label: t('aiworkspace.blocks.chart.editProperty', { key }), operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key: 'config', value: config as Json, expect: { rev: context.keyRevs.config ?? 0 } }] }] })
  }
  const fields = data?.fields ?? []
  return (
    <div className="aiws-inline-form" data-testid="aiws-chart-inspector">
      <label>{t('aiworkspace.blocks.chart.value')} <select value={String(context.payload.config?.value ?? '')} disabled={context.readOnlyReason !== null} onChange={(event) => set('value', event.target.value)}>
        <option value="">{t('aiworkspace.blocks.chart.auto')}</option>{fields.filter((f) => f.type === 'number' || f.type === 'decimal').map((f) => <option key={f.field_id} value={f.field_id}>{f.name}</option>)}
      </select></label>
      <label>{t('aiworkspace.blocks.chart.group')} <select value={String(context.payload.config?.by ?? '')} disabled={context.readOnlyReason !== null} onChange={(event) => set('by', event.target.value)}>
        <option value="">{t('aiworkspace.blocks.chart.auto')}</option>{fields.map((f) => <option key={f.field_id} value={f.field_id}>{f.name}</option>)}
      </select></label>
      <label>{t('aiworkspace.blocks.color')} <input type="color" value={String(context.payload.config?.color ?? '#4f8df7')} disabled={context.readOnlyReason !== null} onChange={(event) => set('color', event.target.value)} /></label>
    </div>
  )
}

export const chartBlock: BlockDefinition = {
  type: 'sample.bar-chart', version: 1, get title() { return aiwsT('aiworkspace.blocks.chart.title') }, accepts: ['buckyos.table-source'], allowNoSource: false,
  defaultSize: { w: 420, h: 260 }, cost: { editor: false, html: false },
  catalog: { group: 'sample', get description() { return aiwsT('aiworkspace.blocks.chart.description') }, needs: 'data' },
  Static: ChartStatic, View: ChartView, Inspector: ChartInspector,
  actions: [{ id: 'open-source', get label() { return aiwsT('aiworkspace.blocks.chart.openSource') }, modes: ['edit', 'view'], run: (context) => { if (context.source) context.openEntity(context.source.entity_id) } }],
  create: (args) => [cellOp(args, 'sample.bar-chart', args.existingSourceId, args.config ? { config: args.config } : {})],
}

/** A line over an ordered x field (dates, months, numbers): time series and trends. */
function LineChart(context: RenderContext) {
  const { t, num: formatNum, compare } = useAiwsText()
  const { data, error } = useTable(context)
  const config = context.payload.config ?? {}
  const xField = fieldId(data, typeof config.x === 'string' ? config.x : undefined, 'date') ?? fieldId(data, undefined, 'text')
  const yField = fieldId(data, typeof config.y === 'string' ? config.y : undefined, 'number') ?? fieldId(data, undefined, 'decimal')
  const points = useMemo(() => {
    if (!data || !xField || !yField) return [] as { x: string; y: number }[]
    const sums = new Map<string, number>()
    for (const row of data.rows) { const x = String(row[xField] ?? ''); sums.set(x, (sums.get(x) ?? 0) + num(row[yField])) }
    return [...sums].map(([x, y]) => ({ x, y })).sort((a, b) => compare(a.x, b.x))
  }, [data, xField, yField, compare])
  const name = (id: string | undefined) => data?.fields.find((f) => f.field_id === id)?.name ?? id ?? ''
  const W = 400, H = 180, pad = 28
  const max = Math.max(1, ...points.map((p) => p.y))
  const min = Math.min(0, ...points.map((p) => p.y))
  const sx = (i: number) => pad + (points.length <= 1 ? (W - 2 * pad) / 2 : (i * (W - 2 * pad)) / (points.length - 1))
  const sy = (y: number) => H - pad - ((y - min) / (max - min || 1)) * (H - 2 * pad)
  const color = typeof config.color === 'string' ? config.color : 'var(--cp-accent)'
  return (
    <div className="aiws-chart" data-testid={`aiws-line-${context.cell.entity_id}`}>
      <div className="aiws-decl-label">{context.payload.title ?? t('aiworkspace.blocks.lineChart.titleOver', { y: name(yField), x: name(xField) })}</div>
      {error && <div className="aiws-error">{error}</div>}
      <svg viewBox={`0 0 ${W} ${H}`} width="100%" role="img" aria-label={t('aiworkspace.blocks.lineChart.aria', { name: name(yField) })}>
        <line x1={pad} y1={H - pad} x2={W - pad} y2={H - pad} stroke="currentColor" opacity="0.3" />
        <polyline fill="none" stroke={color} strokeWidth="2" points={points.map((p, i) => `${sx(i)},${sy(p.y)}`).join(' ')} />
        {points.map((p, i) => (
          <g key={p.x}>
            <circle cx={sx(i)} cy={sy(p.y)} r="3" fill={color}><title>{t('aiworkspace.blocks.lineChart.point', { x: p.x, value: formatNum(p.y, { maximumFractionDigits: 2 }) })}</title></circle>
            {(points.length <= 12 || i % Math.ceil(points.length / 12) === 0) && <text x={sx(i)} y={H - 8} fontSize="9" textAnchor="middle" fill="currentColor" opacity="0.7">{p.x}</text>}
          </g>
        ))}
      </svg>
    </div>
  )
}

export const lineChartBlock: BlockDefinition = {
  type: 'sample.line-chart', version: 1, get title() { return aiwsT('aiworkspace.blocks.lineChart.title') }, accepts: ['buckyos.table-source'], allowNoSource: false,
  defaultSize: { w: 420, h: 260 }, cost: { editor: false, html: false },
  catalog: { group: 'sample', get description() { return aiwsT('aiworkspace.blocks.lineChart.description') }, needs: 'data' },
  Static: LineChart,
  actions: [{ id: 'open-source', get label() { return aiwsT('aiworkspace.blocks.chart.openSource') }, modes: ['edit', 'view'], run: (context) => { if (context.source) context.openEntity(context.source.entity_id) } }],
  create: (args) => [cellOp(args, 'sample.line-chart', args.existingSourceId, args.config ? { config: args.config } : {})],
}

export function registerSampleBlocks(): () => void {
  const offMetric = blockRegistry.register(metricBlock)
  const offChart = blockRegistry.register(chartBlock)
  const offLine = blockRegistry.register(lineChartBlock)
  return () => { offMetric(); offChart(); offLine() }
}

// ---- frame-sequence video preview (the AIGC demo's "final video"): static poster, explicit playback in view mode

function useFrames(context: RenderContext) {
  const store = useStore()
  const id = context.source?.entity_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(async () => {
    const read = await store.readBatched<{ props: Record<string, Json> }>(id)
    const props = read.content.props
    let frames: { object_id: string; media_type: string; duration_ms: number; caption: string }[] = []
    try { frames = JSON.parse(String(props.frames ?? '[]')) as typeof frames } catch { frames = [] }
    return { frames, caption: String(props.caption ?? ''), duration: Number(props.duration ?? 0) }
  }, [store, id])
  return useLoad(load, version)
}

function VideoStatic(context: RenderContext) {
  const { t } = useAiwsText()
  const { data, error } = useFrames(context)
  if (error) return <div className="aiws-error">{error}</div>
  const poster = data?.frames[0]
  return (
    <div className="aiws-video" data-testid={`aiws-video-${context.cell.entity_id}`}>
      {poster ? <AssetBlobImage objectId={poster.object_id} mediaType={poster.media_type} alt={poster.caption} /> : <div className="aiws-html-placeholder">{t('aiworkspace.blocks.video.noFrames')}</div>}
      <div className="aiws-video-caption">{data?.caption ?? ''}<span className="aiws-chip aiws-chip-derived">{t('aiworkspace.blocks.video.simulated')}</span></div>
    </div>
  )
}

function VideoView(context: RenderContext) {
  const { t } = useAiwsText()
  const { data, error } = useFrames(context)
  const [playing, setPlaying] = useState(false)
  const [index, setIndex] = useState(0)
  const frames = useMemo(() => data?.frames ?? [], [data])
  useEffect(() => {
    if (!playing || frames.length === 0) return
    const timer = window.setTimeout(() => setIndex((i) => (i + 1) % frames.length), frames[index]?.duration_ms ?? 1000)
    return () => window.clearTimeout(timer)
  }, [playing, index, frames])
  if (error) return <div className="aiws-error">{error}</div>
  const frame = frames[index]
  return (
    <div className="aiws-video" data-testid={`aiws-video-${context.cell.entity_id}`} data-playing={playing ? 'true' : 'false'}>
      {frame ? <AssetBlobImage objectId={frame.object_id} mediaType={frame.media_type} alt={frame.caption} /> : <div className="aiws-html-placeholder">{t('aiworkspace.blocks.video.noFrames')}</div>}
      <div className="aiws-video-caption">{frame?.caption ?? data?.caption ?? ''} · {index + 1}/{frames.length}</div>
      <div className="aiws-inline-form">
        <button type="button" data-testid="aiws-video-play" onClick={() => setPlaying((p) => !p)}>{t(playing ? 'aiworkspace.blocks.video.pause' : 'aiworkspace.blocks.video.play')}</button>
        <button type="button" onClick={() => { setPlaying(false); setIndex(0) }}>{t('aiworkspace.blocks.video.rewind')}</button>
        <span className="aiws-chip aiws-chip-derived">{t('aiworkspace.blocks.video.notReal')}</span>
      </div>
    </div>
  )
}

export const videoBlock: BlockDefinition = {
  type: 'sample.video', version: 1, get title() { return aiwsT('aiworkspace.blocks.video.title') }, accepts: ['buckyos.record'], allowNoSource: false,
  defaultSize: { w: 640, h: 452 }, cost: { editor: false, html: false },
  catalog: { group: 'sample', get description() { return aiwsT('aiworkspace.blocks.video.description') }, needs: 'data' },
  Static: VideoStatic, View: VideoView,
}

export function registerVideoBlock() {
  return blockRegistry.register(videoBlock)
}

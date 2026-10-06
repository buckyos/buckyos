/* eslint-disable react-refresh/only-export-components -- Block definitions bundle their renderers */
/* Extension sample 1 (phase two §10.5): a metric and a bar chart Block for a TableSource,
 * registered in code without touching the Shell, selection, commit or undo. Both only read the
 * table through the session and render; the chart has a view-mode variant (hover values) and an
 * inspector for its configuration. */

import { useCallback, useEffect, useMemo, useState } from 'react'
import type { Json, QueryPage } from '../../api/types'
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

function MetricStatic(context: RenderContext) {
  const { data, error } = useTable(context)
  const config = context.payload.config ?? {}
  const field = fieldId(data, typeof config.field === 'string' ? config.field : undefined, 'number') ?? fieldId(data, undefined, 'decimal')
  const agg = typeof config.aggregation === 'string' ? config.aggregation : 'sum'
  const values = data && field ? data.rows.map((r) => num(r[field])) : []
  const value = values.length === 0 ? 0 : agg === 'avg' ? values.reduce((a, b) => a + b, 0) / values.length : agg === 'count' ? values.length : agg === 'max' ? Math.max(...values) : values.reduce((a, b) => a + b, 0)
  const name = data?.fields.find((f) => f.field_id === field)?.name ?? field ?? ''
  return (
    <div className="aiws-metric" data-testid={`aiws-metric-${context.cell.entity_id}`}>
      <div className="aiws-decl-label">{context.payload.title ?? `${name} 的${agg === 'avg' ? '平均' : agg === 'count' ? '计数' : agg === 'max' ? '最大' : '合计'}`}</div>
      {error ? <div className="aiws-error">{error}</div> : <div className="aiws-metric-value">{value.toLocaleString('zh-CN', { maximumFractionDigits: 2 })}</div>}
      <div className="aiws-muted">{data ? `${data.rows.length} 行` : '…'}</div>
    </div>
  )
}

function MetricInspector(context: RenderContext) {
  const store = useStore()
  const { data } = useTable(context)
  const set = (key: string, value: Json) => {
    const config = { ...(context.payload.config ?? {}), [key]: value }
    void store.submit({ editId: `config:${context.cell.entity_id}`, label: `指标属性 ${key}`, operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key: 'config', value: config as Json, expect: { rev: context.keyRevs.config ?? 0 } }] }] })
  }
  const numeric = (data?.fields ?? []).filter((f) => f.type === 'number' || f.type === 'decimal')
  return (
    <div className="aiws-inline-form" data-testid="aiws-metric-inspector">
      <label>字段 <select value={String(context.payload.config?.field ?? '')} disabled={context.readOnlyReason !== null} onChange={(event) => set('field', event.target.value)}>
        <option value="">（第一个数值字段）</option>{numeric.map((f) => <option key={f.field_id} value={f.field_id}>{f.name}</option>)}
      </select></label>
      <label>聚合 <select value={String(context.payload.config?.aggregation ?? 'sum')} disabled={context.readOnlyReason !== null} onChange={(event) => set('aggregation', event.target.value)}>
        <option value="sum">合计</option><option value="avg">平均</option><option value="count">计数</option><option value="max">最大</option>
      </select></label>
    </div>
  )
}

export const metricBlock: BlockDefinition = {
  type: 'sample.metric', version: 1, title: '指标（样本）', accepts: ['buckyos.table-source'], allowNoSource: false,
  defaultSize: { w: 220, h: 120 }, cost: { editor: false, html: false },
  Static: MetricStatic, Inspector: MetricInspector,
  create: (args) => [cellOp(args, 'sample.metric', args.existingSourceId, args.config ? { config: args.config } : {})],
}

function BarChart(context: RenderContext & { interactive: boolean }) {
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
      <div className="aiws-decl-label">{context.payload.title ?? `${name(valueField)} 按 ${name(byField)}`}</div>
      {error && <div className="aiws-error">{error}</div>}
      <div className="aiws-decl-bars">
        {[...groups].map(([label, value]) => (
          <div key={label} className="aiws-decl-bar-row" onMouseEnter={context.interactive ? () => setHover(label) : undefined} onMouseLeave={context.interactive ? () => setHover(null) : undefined}>
            <span className="aiws-decl-bar-label">{label || '（空）'}</span>
            <span className="aiws-decl-bar" style={{ width: `${(value / max) * 100}%`, background: typeof config.color === 'string' ? config.color : 'var(--cp-accent)' }} />
            {(!context.interactive || hover === label) && <span className="aiws-decl-bar-value" data-testid="aiws-chart-value">{value.toLocaleString('zh-CN', { maximumFractionDigits: 2 })}</span>}
          </div>
        ))}
      </div>
      {context.interactive && <div className="aiws-muted">查看模式：悬停显示数值</div>}
    </div>
  )
}

const ChartStatic = (context: RenderContext) => <BarChart {...context} interactive={false} />
const ChartView = (context: RenderContext) => <BarChart {...context} interactive />

function ChartInspector(context: RenderContext) {
  const store = useStore()
  const { data } = useTable(context)
  const set = (key: string, value: Json) => {
    const config = { ...(context.payload.config ?? {}), [key]: value }
    void store.submit({ editId: `config:${context.cell.entity_id}`, label: `图表属性 ${key}`, operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key: 'config', value: config as Json, expect: { rev: context.keyRevs.config ?? 0 } }] }] })
  }
  const fields = data?.fields ?? []
  return (
    <div className="aiws-inline-form" data-testid="aiws-chart-inspector">
      <label>数值 <select value={String(context.payload.config?.value ?? '')} disabled={context.readOnlyReason !== null} onChange={(event) => set('value', event.target.value)}>
        <option value="">（自动）</option>{fields.filter((f) => f.type === 'number' || f.type === 'decimal').map((f) => <option key={f.field_id} value={f.field_id}>{f.name}</option>)}
      </select></label>
      <label>分组 <select value={String(context.payload.config?.by ?? '')} disabled={context.readOnlyReason !== null} onChange={(event) => set('by', event.target.value)}>
        <option value="">（自动）</option>{fields.map((f) => <option key={f.field_id} value={f.field_id}>{f.name}</option>)}
      </select></label>
      <label>颜色 <input type="color" value={String(context.payload.config?.color ?? '#4f8df7')} disabled={context.readOnlyReason !== null} onChange={(event) => set('color', event.target.value)} /></label>
    </div>
  )
}

export const chartBlock: BlockDefinition = {
  type: 'sample.bar-chart', version: 1, title: '柱状图（样本）', accepts: ['buckyos.table-source'], allowNoSource: false,
  defaultSize: { w: 420, h: 260 }, cost: { editor: false, html: false },
  Static: ChartStatic, View: ChartView, Inspector: ChartInspector,
  actions: [{ id: 'open-source', label: '打开数据表', modes: ['edit', 'view'], run: (context) => { if (context.source) context.openEntity(context.source.entity_id) } }],
  create: (args) => [cellOp(args, 'sample.bar-chart', args.existingSourceId, args.config ? { config: args.config } : {})],
}

export function registerSampleBlocks(): () => void {
  const offMetric = blockRegistry.register(metricBlock)
  const offChart = blockRegistry.register(chartBlock)
  return () => { offMetric(); offChart() }
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
  const { data, error } = useFrames(context)
  if (error) return <div className="aiws-error">{error}</div>
  const poster = data?.frames[0]
  return (
    <div className="aiws-video" data-testid={`aiws-video-${context.cell.entity_id}`}>
      {poster ? <AssetBlobImage objectId={poster.object_id} mediaType={poster.media_type} alt={poster.caption} /> : <div className="aiws-html-placeholder">没有画面</div>}
      <div className="aiws-video-caption">{data?.caption ?? ''}<span className="aiws-chip aiws-chip-derived">模拟 · 逐帧预览</span></div>
    </div>
  )
}

function VideoView(context: RenderContext) {
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
      {frame ? <AssetBlobImage objectId={frame.object_id} mediaType={frame.media_type} alt={frame.caption} /> : <div className="aiws-html-placeholder">没有画面</div>}
      <div className="aiws-video-caption">{frame?.caption ?? data?.caption ?? ''} · {index + 1}/{frames.length}</div>
      <div className="aiws-inline-form">
        <button type="button" data-testid="aiws-video-play" onClick={() => setPlaying((p) => !p)}>{playing ? '暂停' : '播放预览'}</button>
        <button type="button" onClick={() => { setPlaying(false); setIndex(0) }}>回到开头</button>
        <span className="aiws-chip aiws-chip-derived">模拟 · 逐帧预览，不是真实视频</span>
      </div>
    </div>
  )
}

export const videoBlock: BlockDefinition = {
  type: 'sample.video', version: 1, title: '逐帧预览（样本）', accepts: ['buckyos.record'], allowNoSource: false,
  defaultSize: { w: 640, h: 452 }, cost: { editor: false, html: false },
  Static: VideoStatic, View: VideoView,
}

export function registerVideoBlock() {
  return blockRegistry.register(videoBlock)
}

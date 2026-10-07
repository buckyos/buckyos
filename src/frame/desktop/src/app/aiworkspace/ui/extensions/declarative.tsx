/* eslint-disable react-refresh/only-export-components -- Block definitions bundle their renderers */
/* The `declarative` Block (phase two §10.3): a definition entity describes bindings, layout
 * primitives, properties and actions; this interpreter renders it. No code of the definition is
 * ever executed; actions map only to existing commands. A definition may give a separate `view`
 * variant and view-mode actions / inspector fields, which is how the extension sample proves mode
 * dispatch without touching the host. */

import { useCallback, useMemo, type ReactNode } from 'react'
import type { Json, QueryPage, RecordContent } from '../../api/types'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { blockRegistry, type BlockAction, type BlockDefinition, type RenderContext } from '../blocks/registry'
import { cellOp } from '../blocks/ops'

interface Item {
  kind: 'text' | 'value' | 'metric' | 'bar' | 'list' | 'badge'
  text?: string
  label?: string
  /** `props.<key>` (record), `sum:<field>` / `avg:<field>` / `count` / `<field>` (table). */
  bind?: string
  by?: string
  format?: 'text' | 'number' | 'percent' | 'currency'
  limit?: number
}
interface Spec {
  layout?: 'stack' | 'row'
  items?: Item[]
  view?: { items?: Item[] }
  actions?: { id: string; label: string; command: 'open_source' | 'activate_editor' | 'open_definition'; modes?: ('edit' | 'view')[] }[]
  inspector?: { key: string; label: string; kind: 'text' | 'number' | 'select' | 'boolean' | 'color'; options?: { value: string; label: string }[]; modes?: ('edit' | 'view')[] }[]
  accent?: string
}

function num(v: unknown): number {
  if (typeof v === 'number') return v
  if (typeof v === 'string') { const n = Number(v.replace(/[,¥$%\s]/g, '')); return Number.isFinite(n) ? n : 0 }
  return 0
}

export function formatNumber(n: number, format: Item['format']): string {
  if (format === 'percent') return `${(n * 100).toFixed(1)}%`
  if (format === 'currency') return `¥${n.toLocaleString('zh-CN', { maximumFractionDigits: 0 })}`
  if (format === 'number') return n.toLocaleString('zh-CN', { maximumFractionDigits: 2 })
  return String(n)
}

type DataView = { kind: 'record'; props: Record<string, Json> } | { kind: 'table'; rows: Record<string, Json>[]; fields: Map<string, string> }

function useSourceData(context: RenderContext): { data: DataView | null; error: string | null } {
  const store = useStore()
  const source = context.source
  const id = source?.entity_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(async (): Promise<DataView | null> => {
    if (!source) return null
    if (source.type_id === 'buckyos.record') {
      const read = await store.readBatched<RecordContent>(id)
      return { kind: 'record', props: read.content.props }
    }
    if (source.type_id === 'buckyos.table-source') {
      const page: QueryPage = await store.session.query({ source_id: id, limit: 500, consistency: 'best_effort' })
      const fields = new Map<string, string>()
      const read = await store.readBatched<{ fields: { field_id: string; name: string }[] }>(id)
      for (const f of read.content.fields) fields.set(f.field_id, f.name)
      return { kind: 'table', rows: page.rows.map((r) => r.values), fields }
    }
    return null
  }, [store, id, source])
  const loaded = useLoad<DataView | null>(load, version)
  return { data: loaded.data ?? null, error: loaded.error }
}

function resolveBind(data: DataView | null, bind: string | undefined, fieldByName?: (name: string) => string): { value: number | string | null; series?: { label: string; value: number }[] } {
  if (!data || !bind) return { value: null }
  if (data.kind === 'record') {
    const key = bind.replace(/^props\./, '')
    const v = data.props[key]
    return { value: v === undefined || v === null ? null : typeof v === 'number' ? v : String(v) }
  }
  const field = (name: string) => fieldByName?.(name) ?? name
  if (bind === 'count') return { value: data.rows.length }
  const agg = /^(sum|avg|min|max):(.+)$/.exec(bind)
  if (agg) {
    const f = field(agg[2])
    const values = data.rows.map((r) => num(r[f]))
    if (values.length === 0) return { value: 0 }
    const sum = values.reduce((a, b) => a + b, 0)
    return { value: agg[1] === 'sum' ? sum : agg[1] === 'avg' ? sum / values.length : agg[1] === 'min' ? Math.min(...values) : Math.max(...values) }
  }
  const f = field(bind)
  return { value: data.rows.length > 0 ? (typeof data.rows[0][f] === 'number' ? (data.rows[0][f] as number) : String(data.rows[0][f] ?? '')) : null }
}

function Items({ items, data, accent, context }: { items: Item[]; data: DataView | null; accent: string; context: RenderContext }) {
  const byName = useMemo(() => {
    if (data?.kind !== 'table') return undefined
    const map = new Map<string, string>()
    for (const [id, name] of data.fields) { map.set(name, id); map.set(id, id) }
    return (name: string) => map.get(name) ?? name
  }, [data])
  return (
    <>
      {items.map((item, index) => {
        if (item.kind === 'text') return <div key={index} className="aiws-decl-text">{item.text}</div>
        if (item.kind === 'badge') return <span key={index} className="aiws-chip" style={{ borderColor: accent, color: accent }}>{item.text ?? item.label}</span>
        if (item.kind === 'value' || item.kind === 'metric') {
          const { value } = resolveBind(data, item.bind, byName)
          const shown = value === null ? '—' : typeof value === 'number' ? formatNumber(value, item.format) : value
          return (
            <div key={index} className={item.kind === 'metric' ? 'aiws-decl-metric' : 'aiws-decl-value'} data-testid={`aiws-decl-${item.kind}`}>
              <div className="aiws-decl-label">{item.label ?? item.bind}</div>
              <div className="aiws-decl-number" style={{ color: item.kind === 'metric' ? accent : undefined }}>{shown}</div>
            </div>
          )
        }
        if (item.kind === 'list') {
          if (data?.kind !== 'table') return <div key={index} className="aiws-muted">列表需要表格数据</div>
          const f = byName?.(item.bind ?? '') ?? item.bind ?? ''
          return <ul key={index} className="aiws-decl-list">{data.rows.slice(0, item.limit ?? 5).map((r, i) => <li key={i}>{String(r[f] ?? '')}</li>)}</ul>
        }
        if (item.kind === 'bar') {
          if (data?.kind !== 'table') return <div key={index} className="aiws-muted">柱图需要表格数据</div>
          const f = byName?.(item.bind ?? '') ?? item.bind ?? ''
          const by = byName?.(item.by ?? '') ?? item.by ?? ''
          const groups = new Map<string, number>()
          for (const r of data.rows) groups.set(String(r[by] ?? ''), (groups.get(String(r[by] ?? '')) ?? 0) + num(r[f]))
          const max = Math.max(1, ...groups.values())
          return (
            <div key={index} className="aiws-decl-bars" data-testid="aiws-decl-bar">
              {[...groups].map(([label, value]) => (
                <div key={label} className="aiws-decl-bar-row"><span className="aiws-decl-bar-label">{label}</span><span className="aiws-decl-bar" style={{ width: `${(value / max) * 100}%`, background: accent }} /><span className="aiws-decl-bar-value">{formatNumber(value, item.format ?? 'number')}</span></div>
              ))}
            </div>
          )
        }
        return null
      })}
      {void context}
    </>
  )
}

function useSpec(context: RenderContext): { spec: Spec | null; error: string | null; title: string } {
  const payload = context.documentDefinition
  return { spec: (payload?.declarative as Spec | undefined) ?? null, error: null, title: payload?.title ?? context.payload.title ?? '声明式 Block' }
}

function DeclarativeBody(context: RenderContext & { variant: 'edit' | 'view' }) {
  const { spec, error, title } = useSpec(context)
  const { data, error: dataError } = useSourceData(context)
  if (error) return <div className="aiws-error" role="alert">无法读取定义：{error}</div>
  if (!spec) return <div className="aiws-muted">载入定义…</div>
  const accent = typeof context.payload.config?.accent === 'string' ? context.payload.config.accent : spec.accent ?? 'var(--cp-accent)'
  const items = context.variant === 'view' && spec.view?.items ? spec.view.items : spec.items ?? []
  return (
    <div className={`aiws-decl aiws-decl-${spec.layout ?? 'stack'}`} data-testid={`aiws-decl-${context.cell.entity_id}`} data-variant={context.variant}>
      <div className="aiws-decl-title">{title}</div>
      {dataError && <div className="aiws-warning">{dataError}</div>}
      <Items items={items} data={data} accent={accent} context={context} />
    </div>
  )
}

const DeclarativeStatic = (context: RenderContext) => <DeclarativeBody {...context} variant="edit" />
const DeclarativeView = (context: RenderContext) => <DeclarativeBody {...context} variant="view" />

/** Actions declared by the definition, mapped to existing commands only. */
function DeclarativeInspector(context: RenderContext) {
  const { spec } = useSpec(context)
  const store = useStore()
  const fields = (spec?.inspector ?? []).filter((f) => !f.modes || f.modes.includes(context.mode === 'view' ? 'view' : 'edit'))
  const actions = (spec?.actions ?? []).filter((a) => !a.modes || a.modes.includes(context.mode === 'view' ? 'view' : 'edit'))
  const run = (command: string) => {
    if (command === 'open_source' && context.source) context.openEntity(context.source.entity_id)
    if (command === 'open_definition' && context.payload.def_ref) context.openEntity(context.payload.def_ref.entity_id)
    if (command === 'activate_editor' && context.mode === 'edit') context.activateEditor()
  }
  const setConfig = (key: string, value: Json) => {
    const config = { ...(context.payload.config ?? {}), [key]: value }
    void store.submit({ editId: `config:${context.cell.entity_id}`, label: `属性 ${key}`, operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key: 'config', value: config as Json, expect: { rev: context.keyRevs.config ?? 0 } }] }] })
  }
  return (
    <div className="aiws-decl-inspector" data-testid="aiws-decl-inspector" data-mode={context.mode}>
      {fields.map((f) => (
        <label key={f.key} className="aiws-inline-form">{f.label}
          {f.kind === 'select'
            ? <select value={String(context.payload.config?.[f.key] ?? '')} disabled={context.readOnlyReason !== null} onChange={(event) => setConfig(f.key, event.target.value)}>{(f.options ?? []).map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}</select>
            : f.kind === 'boolean'
              ? <input type="checkbox" checked={Boolean(context.payload.config?.[f.key])} disabled={context.readOnlyReason !== null} onChange={(event) => setConfig(f.key, event.target.checked)} />
              : <input type={f.kind === 'color' ? 'color' : f.kind === 'number' ? 'number' : 'text'} value={String(context.payload.config?.[f.key] ?? (f.kind === 'color' ? '#4f8df7' : ''))} disabled={context.readOnlyReason !== null}
                  onChange={(event) => setConfig(f.key, f.kind === 'number' ? Number(event.target.value) : event.target.value)} />}
        </label>
      ))}
      {actions.map((a) => <button key={a.id} type="button" data-testid={`aiws-decl-action-${a.id}`} onClick={() => run(a.command)}>{a.label}</button>)}
      {fields.length === 0 && actions.length === 0 && <span className="aiws-muted">此定义在{context.mode === 'view' ? '查看' : '编辑'}模式下没有属性或动作。</span>}
    </div>
  )
}

const declarativeActions: BlockAction[] = [
  { id: 'open-source', label: '打开数据', modes: ['edit', 'view'], when: (context) => Boolean(context.source), run: (context) => { if (context.source) context.openEntity(context.source.entity_id) } },
]

export const declarativeBlock: BlockDefinition = {
  type: 'declarative', version: 1, definitionKind: 'declarative', title: '声明式 Block', accepts: ['buckyos.record', 'buckyos.table-source'], allowNoSource: false,
  defaultSize: { w: 320, h: 200 }, cost: { editor: false, html: false },
  Static: DeclarativeStatic, View: DeclarativeView, Inspector: DeclarativeInspector, actions: declarativeActions,
  create: (args) => [cellOp(args, 'declarative', args.existingSourceId, { def_ref: { entity_id: String(args.config?.def_id ?? '') } as unknown as Json })],
}

export function registerDeclarativeBlock() {
  blockRegistry.register(declarativeBlock)
}

export type { Spec as DeclarativeSpec }
export function renderNothing(): ReactNode { return null }

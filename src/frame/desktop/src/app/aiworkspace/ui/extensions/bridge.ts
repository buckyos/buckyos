/* The host side of the HTML extension API (phase two §10.4; `aiws` v2 of 许愿格 §9.4): everything an
 * extension can do goes through here, so its writes carry the same permissions, locks, undo and
 * save states as a user's. Not a security boundary (D16) — a boundary of bookkeeping.
 *
 * v2 speaks in binding names (`source` is the Block's `source_ref`, the rest its `bindings`), plain
 * values (option labels, numbers) and typed writes; the host turns them into ordinary operations
 * with the right `expect`s. A batch collects typed writes into one commit and one undo step. */

import { randomId } from '../../api/ids'
import type {
  CellPayload, EntityEnvelope, FieldDef, Json, KeyedContent, Operation, QueryPage, QueryParams, QueryRow, RecordContent, RichTextContent, Selector, TableSourceContent,
} from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { modePolicy, type CanvasMode } from '../blocks/registry'
import type { HostBridge } from './htmlRuntime'

export interface BridgeTarget {
  cell: EntityEnvelope
  payload: CellPayload
  keyRevs: Record<string, number>
  source: EntityEnvelope | undefined
  mode: CanvasMode
  /** Extra context handed to the extension (wish executors get the wish and its inputs). */
  extra?: Record<string, Json>
}

interface Binding { entity_id: string; selector?: Selector }

function bindingsOf(target: BridgeTarget): Record<string, Binding> {
  const out: Record<string, Binding> = {}
  if (target.payload.source_ref) out.source = { entity_id: target.payload.source_ref.entity_id }
  for (const [name, b] of Object.entries(target.payload.bindings ?? {})) out[name] = b
  return out
}

function plain(field: FieldDef, v: Json | undefined): Json | undefined {
  if (v === undefined || v === null) return v
  const label = (id: Json) => field.options?.find((o) => o.option_id === id)?.label ?? id
  if (field.type === 'decimal' && typeof v === 'string') { const n = Number(v); return Number.isFinite(n) ? n : v }
  if (field.type === 'select') return label(v)
  if (field.type === 'multi_select' && Array.isArray(v)) return v.map(label)
  return v
}

/** A plain value in the stored form of `field` (labels → option ids, numbers → decimals). */
function stored(field: FieldDef, v: Json | undefined): Json | undefined {
  if (v === undefined || v === null || (v === '' && field.type !== 'text')) return undefined
  const option = (x: Json) => {
    const o = field.options?.find((opt) => opt.label === x || opt.option_id === x)
    if (!o) throw new Error(`字段「${field.name}」没有选项「${String(x)}」`)
    return o.option_id
  }
  switch (field.type) {
    case 'decimal': {
      const n = typeof v === 'number' ? v : Number(String(v).replace(/[,¥$\s]/g, ''))
      if (!Number.isFinite(n)) throw new Error(`字段「${field.name}」需要数值，得到 ${JSON.stringify(v)}`)
      return n.toFixed(field.scale ?? 2)
    }
    case 'number': {
      const n = typeof v === 'number' ? v : Number(String(v).replace(/[,¥$\s]/g, ''))
      if (!Number.isFinite(n)) throw new Error(`字段「${field.name}」需要数值，得到 ${JSON.stringify(v)}`)
      return n
    }
    case 'select': return option(v)
    case 'multi_select': return (Array.isArray(v) ? v : [v]).map(option)
    case 'boolean': return Boolean(v)
    case 'text': return typeof v === 'string' ? v : JSON.stringify(v)
    default: return v
  }
}

export function makeBridge(store: WorkspaceStore, target: BridgeTarget, onSnapshot?: (objectId: string, mediaType: string) => void): HostBridge {
  const policy = modePolicy(target.mode)
  const bindings = bindingsOf(target)
  const batches = new Map<string, { label: string; operations: Operation[] }>()
  const binding = (name: string): Binding => {
    const b = bindings[name]
    if (!b) throw new Error(`没有名为 ${name} 的绑定（有：${Object.keys(bindings).join('、') || '无'}）`)
    return b
  }
  const requireWrites = () => { if (!policy.writes) throw new Error(`当前子模式（${target.mode}）不允许扩展写入文档`) }
  const submit = async (operations: Operation[], label: string): Promise<Json> => {
    requireWrites()
    if (!Array.isArray(operations) || operations.length === 0) throw new Error('operations 必须是非空数组')
    const outcome = await store.submit({ editId: `ext:${target.cell.entity_id}:${Date.now()}`, label: label || `扩展 ${target.payload.view.type} 的修改`, operations })
    return outcome as unknown as Json
  }
  const tableOf = async (name: string) => {
    const b = binding(name)
    const meta = await store.session.read<TableSourceContent>(b.entity_id)
    if (meta.type_id !== 'buckyos.table-source') throw new Error(`绑定 ${name} 不是表格`)
    return { b, meta: meta.content }
  }
  const allRows = async (b: Binding): Promise<QueryRow[]> => {
    const rows: QueryRow[] = []
    const view = b.selector?.kind === 'table_view' ? String((b.selector as { cell_id?: string }).cell_id ?? '') : null
    let cursor: string | undefined
    for (let page = 0; page < 60; page++) {
      const params: QueryParams = view ? { view_id: view, limit: 1000 } : { source_id: b.entity_id, limit: 1000 }
      if (cursor) params.cursor = cursor
      const result: QueryPage = await store.session.query(params)
      rows.push(...result.rows)
      cursor = result.next_cursor ?? undefined
      if (!cursor) break
    }
    return rows
  }
  /** Collect into a batch, or submit now. */
  const emit = async (operations: Operation[], label: string, batch: string | null): Promise<Json> => {
    if (operations.length === 0) return { status: 'unchanged' }
    if (batch) {
      const b = batches.get(batch)
      if (!b) throw new Error('批量写入已结束')
      b.operations.push(...operations)
      return { status: 'batched', operations: operations.length }
    }
    return submit(operations, label)
  }
  return {
    context: () => ({
      cell_id: target.cell.entity_id,
      title: target.payload.title ?? null,
      config: (target.payload.config ?? {}) as Json,
      source: target.source ? { entity_id: target.source.entity_id, type_id: target.source.type_id, name: target.source.name, title: target.source.title ?? null } : null,
      bindings: Object.fromEntries(Object.entries(bindings).map(([name, b]) => {
        const e = store.outline.get(b.entity_id)
        return [name, { entity_id: b.entity_id, type_id: e?.type_id ?? null, title: e?.title ?? e?.name ?? null, selector: (b.selector ?? null) as Json }]
      })),
      mode: target.mode,
      principal: store.session.principal,
      workspace_id: store.session.workspaceId,
      ...(target.extra ?? {}),
    }),
    read: async (entityId, selector) => {
      const result = await store.session.read<Json>(entityId, (selector ?? undefined) as Selector | undefined)
      return result as unknown as Json
    },
    query: async (params) => (await store.session.query(params as unknown as QueryParams)) as unknown as Json,
    submit: (operations, label) => submit(operations, label),
    upload: async (bytes, fileName, mediaType) => {
      const blob = new Blob([bytes as BlobPart], mediaType ? { type: mediaType } : undefined)
      return store.session.uploadAsset(blob, fileName)
    },
    snapshot: async (dataUrl) => {
      requireWrites()
      // the static snapshot is an asset: what the Block shows before it is activated (§10.4 lifecycle)
      const match = /^data:([^;,]+)(;base64)?,(.*)$/s.exec(dataUrl)
      if (!match) throw new Error('snapshot 需要 data URL')
      const mediaType = match[1]
      const bytes = match[2] ? Uint8Array.from(atob(match[3]), (c) => c.charCodeAt(0)) : new TextEncoder().encode(decodeURIComponent(match[3]))
      const uploaded = await store.session.uploadAsset(new Blob([bytes], { type: mediaType }), `snapshot.${mediaType.includes('svg') ? 'svg' : mediaType.includes('png') ? 'png' : 'bin'}`)
      const snapshot = { object_id: uploaded.object_id, media_type: uploaded.media_type, size: uploaded.size }
      // the snapshot is bookkeeping of the Block, not a user edit: no undo entry; a concurrent config edit wins
      const current = await store.session.read<KeyedContent<CellPayload>>(target.cell.entity_id)
      const outcome = await store.submit({
        editId: `ext-snapshot:${target.cell.entity_id}`, label: '扩展快照', undoable: false,
        operations: [{ op: 'entity.set_keys', entity_id: target.cell.entity_id, keys: [{ key: 'config', value: { ...(current.content.payload.config ?? {}), snapshot } as Json, expect: { rev: current.content.key_revs.config ?? 0 } }] }],
      })
      if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') throw new Error('扩展快照未保存，请查看保存状态后重试')
      onSnapshot?.(uploaded.object_id, uploaded.media_type)
    },
    notify: (text) => store.notify('info', text),

    // ---- v2 reads
    input: async (name, what, args) => {
      const b = binding(name)
      const a = (args ?? {}) as { fields?: string[]; filter?: Record<string, Json> | null; sort?: string | string[]; limit?: number }
      switch (what) {
        case 'fields': {
          const { meta } = await tableOf(name)
          return meta.fields.map((f) => ({ name: f.name, type: f.type, id: f.field_id, options: f.options?.map((o) => o.label) ?? null }))
        }
        case 'rows': {
          const { meta } = await tableOf(name)
          const byId = new Map(meta.fields.map((f) => [f.field_id, f]))
          for (const f of a.fields ?? []) if (!meta.fields.some((x) => x.name === f)) throw new Error(`绑定 ${name} 没有字段 ${f}`)
          let rows = (await allRows(b)).map((r) => {
            const o: Record<string, Json> = { _id: r.record_id }
            for (const [fid, v] of Object.entries(r.values)) { const f = byId.get(fid); if (f) o[f.name] = plain(f, v) ?? null }
            return o
          })
          if (a.filter) {
            const conds = Object.entries(a.filter)
            rows = rows.filter((r) => conds.every(([k, v]) => (Array.isArray(v) ? v.includes(r[k]) : r[k] === v)))
          }
          if (a.sort) {
            const keys = (Array.isArray(a.sort) ? a.sort : [a.sort]).map((s) => (s.startsWith('-') ? { f: s.slice(1), d: -1 } : { f: s, d: 1 }))
            rows = [...rows].sort((x, y) => {
              for (const k of keys) {
                const c = String(x[k.f] ?? '').localeCompare(String(y[k.f] ?? ''), 'zh-CN', { numeric: true })
                if (c !== 0) return c * k.d
              }
              return 0
            })
          }
          if (a.fields) rows = rows.map((r) => Object.fromEntries([['_id', r._id], ...(a.fields ?? []).map((f) => [f, r[f] ?? null])]))
          if (typeof a.limit === 'number') rows = rows.slice(0, a.limit)
          return rows as unknown as Json
        }
        case 'markdown': {
          const rt = await store.session.read<RichTextContent>(b.entity_id)
          if (rt.type_id !== 'buckyos.richtext') throw new Error(`绑定 ${name} 不是富文本`)
          return store.core.richtext_to_markdown(JSON.stringify(rt.content.content))
        }
        case 'props': {
          const rec = await store.session.read<RecordContent>(b.entity_id)
          if (rec.type_id !== 'buckyos.record') throw new Error(`绑定 ${name} 不是记录`)
          return Object.fromEntries(rec.content.schema.properties.map((p) => [p.name, plain({ ...p, field_id: p.key, def_rev: 0, type_rev: 0, values_rev: 0 } as FieldDef, rec.content.props[p.key]) ?? null]))
        }
        case 'bytes': {
          const asset = await store.session.read<KeyedContent<{ object_id: string }>>(b.entity_id)
          const blob = await store.session.fetchAsset(asset.content.payload.object_id)
          return Array.from(new Uint8Array(await blob.arrayBuffer()))
        }
        default: throw new Error(`未知的读取 ${what}`)
      }
    },
    outline: async (what, args) => {
      const a = (args ?? {}) as { target?: string | null; depth?: number; text?: string }
      const brief = (e: EntityEnvelope) => ({ entity_id: e.entity_id, type_id: e.type_id, title: e.title ?? e.name ?? null, parent_id: e.parent_id ?? null })
      const all = store.outline.all().filter((e) => !e.deleted)
      if (what === 'find') {
        const t = String(a.text ?? '').toLowerCase()
        return all.filter((e) => String(e.title ?? e.name ?? '').toLowerCase().includes(t)).slice(0, 50).map(brief) as unknown as Json
      }
      if (what === 'resolve') {
        const t = String(a.target ?? '')
        return all.filter((e) => e.entity_id === t || e.title === t || e.name === t).map(brief) as unknown as Json
      }
      const root = a.target ?? 'data'
      const out: ReturnType<typeof brief>[] = []
      const walk = (id: string, depth: number) => {
        for (const c of store.outline.childrenOf(id)) {
          if (c.deleted) continue
          out.push(brief(c))
          if (depth + 1 < (a.depth ?? 1)) walk(c.entity_id, depth + 1)
        }
      }
      walk(root, 0)
      return out as unknown as Json
    },

    // ---- v2 typed writes
    write: async (kind, name, args, batch) => {
      requireWrites()
      const a = (args ?? {}) as Record<string, Json>
      const label = `扩展 ${target.payload.title ?? target.payload.view.type} 的修改`
      switch (kind) {
        case 'upsert': {
          const { b, meta } = await tableOf(name)
          const rows = (a.rows as Record<string, Json>[] | null) ?? []
          const key = (a.key as string[] | null) ?? []
          const byName = new Map(meta.fields.map((f) => [f.name, f]))
          for (const r of rows) for (const k of Object.keys(r)) if (k !== '_id' && !byName.has(k)) throw new Error(`表格没有字段 ${k}`)
          const current = await allRows({ entity_id: b.entity_id })
          const keyOf = (values: Record<string, Json>, byField: boolean) => JSON.stringify(key.map((k) => {
            const f = byName.get(k)
            if (!f) throw new Error(`键字段 ${k} 不存在`)
            return byField ? plain(f, values[f.field_id]) ?? null : values[k] ?? null
          }))
          const index = new Map(current.map((r) => [key.length ? keyOf(r.values, true) : r.record_id, r]))
          const inserts: { record_id: string; values: Record<string, Json> }[] = []
          const sets: { record_id: string; field_id: string; value: Json; expect: { rev: number } }[] = []
          for (const r of rows) {
            const existing = typeof r._id === 'string' ? current.find((c) => c.record_id === r._id) : key.length ? index.get(keyOf(r, false)) : undefined
            if (!existing) {
              const values: Record<string, Json> = {}
              for (const [k, v] of Object.entries(r)) { if (k === '_id') continue; const f = byName.get(k)!; const s = stored(f, v); if (s !== undefined) values[f.field_id] = s }
              inserts.push({ record_id: `r${randomId().slice(0, 16)}`, values })
              continue
            }
            for (const [k, v] of Object.entries(r)) {
              if (k === '_id') continue
              const f = byName.get(k)!
              const s = stored(f, v)
              if (s === undefined || JSON.stringify(existing.values[f.field_id]) === JSON.stringify(s)) continue
              sets.push({ record_id: existing.record_id, field_id: f.field_id, value: s, expect: { rev: existing.revs[f.field_id] ?? 0 } })
            }
          }
          const ops: Operation[] = []
          if (inserts.length) ops.push({ op: 'table.insert_records', source_id: b.entity_id, records: inserts })
          if (sets.length) ops.push({ op: 'table.set_values', source_id: b.entity_id, values: sets })
          return emit(ops, label, batch)
        }
        case 'setColumn': {
          const { b, meta } = await tableOf(name)
          const f = meta.fields.find((x) => x.name === a.field || x.field_id === a.field)
          if (!f) throw new Error(`表格没有字段 ${String(a.field)}`)
          const current = new Map((await allRows({ entity_id: b.entity_id })).map((r) => [r.record_id, r]))
          const values: { record_id: string; field_id: string; value: Json; expect: { rev: number } }[] = []
          for (const [rid, v] of Object.entries((a.values ?? {}) as Record<string, Json>)) {
            const r = current.get(rid)
            if (!r) throw new Error(`记录 ${rid} 不存在`)
            const s = stored(f, v)
            if (s === undefined || JSON.stringify(r.values[f.field_id]) === JSON.stringify(s)) continue
            values.push({ record_id: rid, field_id: f.field_id, value: s, expect: { rev: r.revs[f.field_id] ?? 0 } })
          }
          return emit(values.length ? [{ op: 'table.set_values', source_id: b.entity_id, values }] : [], label, batch)
        }
        case 'recordSet': {
          const b = binding(name)
          const rec = await store.session.read<RecordContent>(b.entity_id)
          if (rec.type_id !== 'buckyos.record') throw new Error(`绑定 ${name} 不是记录`)
          const keys: { key: string; value: Json; expect: { rev: number } }[] = []
          for (const [n, v] of Object.entries((a.props ?? {}) as Record<string, Json>)) {
            const p = rec.content.schema.properties.find((x) => x.name === n || x.key === n)
            if (!p) throw new Error(`记录没有属性 ${n}`)
            const s = stored({ ...p, field_id: p.key, def_rev: 0, type_rev: 0, values_rev: 0 } as FieldDef, v)
            if (s === undefined || JSON.stringify(rec.content.props[p.key]) === JSON.stringify(s)) continue
            keys.push({ key: `p:${p.key}`, value: s, expect: { rev: rec.content.key_revs[`p:${p.key}`] ?? 0 } })
          }
          return emit(keys.length ? [{ op: 'entity.set_keys', entity_id: b.entity_id, keys }] : [], label, batch)
        }
        case 'setMarkdown': {
          const b = binding(name)
          const rt = await store.session.read<RichTextContent>(b.entity_id)
          if (rt.type_id !== 'buckyos.richtext') throw new Error(`绑定 ${name} 不是富文本`)
          const ast = store.core.markdown_to_richtext(String(a.markdown ?? ''), `x${randomId().slice(0, 6).toLowerCase()}`, undefined)
          const ops = JSON.parse(store.core.richtext_diff(b.entity_id, JSON.stringify(rt.content.content), ast, JSON.stringify(rt.content.blocks))) as Operation[]
          return emit(ops, label, batch)
        }
        case 'createBlock': {
          const data = typeof a.data === 'string' ? (bindings[a.data]?.entity_id ?? a.data) : null
          const renderer = String(a.renderer ?? '')
          if (!renderer) throw new Error('createBlock 需要 renderer')
          const parentId = target.cell.parent_id
          if (!parentId) throw new Error('这个 Block 不在画布上')
          const here = target.cell.placement ?? { x: 0, y: 0, w: 320, h: 200 }
          const p = (a.placement ?? {}) as { x?: number; y?: number; w?: number; h?: number }
          const last = store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined
          const id = `blk-${randomId().slice(0, 12).toLowerCase()}`
          const op: Operation = {
            op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parentId, order_key: store.core.order_key_between(last, undefined),
            placement: { x: p.x ?? here.x + here.w + 40, y: p.y ?? here.y, w: p.w ?? 360, h: p.h ?? 240 },
            payload: { view: { type: renderer }, ...(data ? { source_ref: { entity_id: data } } : {}), ...(typeof a.title === 'string' ? { title: a.title } : {}), ...(a.config ? { config: a.config } : {}) },
          }
          const r = await emit([op], label, batch)
          return { ...(r as Record<string, Json>), entity_id: id }
        }
        default: throw new Error(`未知的写入 ${kind}`)
      }
    },
    batch: async (action, id, label) => {
      if (action === 'begin') {
        const key = randomId()
        batches.set(key, { label: label || `扩展 ${target.payload.title ?? target.payload.view.type} 的修改`, operations: [] })
        return key
      }
      const b = id ? batches.get(id) : undefined
      if (id) batches.delete(id)
      if (!b) throw new Error('批量写入不存在')
      if (action === 'abort' || b.operations.length === 0) return { status: action === 'abort' ? 'aborted' : 'unchanged' }
      return submit(b.operations, b.label)
    },
    watch: (name, notify) => {
      const b = bindings[name]
      if (!b) return null
      const key = `e:${b.entity_id}`
      let last = store.versions.get(key)
      return store.versions.subscribe(() => {
        const now = store.versions.get(key)
        if (now !== last) { last = now; notify() }
      })
    },
  }
}

/* Previews of candidate results (许愿格 §11.1): what each result will look like once applied —
 * tables as tables, text as formatted text (the same Markdown conversion the service applies),
 * records as properties, images as images, HTML as its static markup, a mind map drawn by the shared layout
 * (脑图 TODO M3) — not JSON. */

import { useMemo, type ReactNode } from 'react'
import type { Json, WishResult } from '../../api/types'
import type { AiwsText } from '../../i18n'
import { useStore } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { solveControl, type ControlEntity } from '@controls/solve'
import { controlSvg } from '@controls/svg'
import { AssetBlobImage } from '../extensions/AssetBlobImage'
import { nodeMeasurer } from '../controls/measure'

interface Node { type: string; attrs?: Record<string, Json>; content?: Node[]; text?: string; marks?: { type: string; attrs?: Record<string, Json> }[] }

/** `linkLabel`: what an object link without a label shows. */
function inline(nodes: Node[] | undefined, linkLabel: string): ReactNode[] {
  return (nodes ?? []).map((n, i) => {
    if (n.type === 'hard_break') return <br key={i} />
    if (n.type === 'object_link') return <span key={i} className="aiws-object-link">{String(n.attrs?.label || linkLabel)}</span>
    let out: ReactNode = n.text ?? ''
    for (const m of n.marks ?? []) {
      if (m.type === 'strong') out = <strong>{out}</strong>
      else if (m.type === 'em') out = <em>{out}</em>
      else if (m.type === 'strike') out = <s>{out}</s>
      else if (m.type === 'code') out = <code>{out}</code>
      else if (m.type === 'link') out = <a href={String(m.attrs?.href ?? '#')} target="_blank" rel="noopener noreferrer">{out}</a>
    }
    return <span key={i}>{out}</span>
  })
}

function blocks(nodes: Node[] | undefined, linkLabel: string): ReactNode[] {
  return (nodes ?? []).map((n, i) => {
    switch (n.type) {
      case 'paragraph': return <p key={i}>{inline(n.content, linkLabel)}</p>
      case 'heading': {
        const level = Number(n.attrs?.level ?? 1)
        return level === 1 ? <h3 key={i}>{inline(n.content, linkLabel)}</h3> : level === 2 ? <h4 key={i}>{inline(n.content, linkLabel)}</h4> : <h5 key={i}>{inline(n.content, linkLabel)}</h5>
      }
      case 'bullet_list': return <ul key={i}>{(n.content ?? []).map((li, j) => <li key={j}>{blocks(li.content, linkLabel)}</li>)}</ul>
      case 'ordered_list': return <ol key={i} start={Number(n.attrs?.start ?? 1)}>{(n.content ?? []).map((li, j) => <li key={j}>{blocks(li.content, linkLabel)}</li>)}</ol>
      case 'blockquote': return <blockquote key={i}>{blocks(n.content, linkLabel)}</blockquote>
      case 'code_block': return <pre key={i}><code>{(n.content ?? []).map((t) => t.text ?? '').join('')}</code></pre>
      case 'horizontal_rule': return <hr key={i} />
      case 'table': return (
        <table key={i}><tbody>{(n.content ?? []).map((row, r) => (
          <tr key={r}>{(row.content ?? []).map((cell, c) => (cell.attrs?.header === 1 ? <th key={c}>{inline(cell.content, linkLabel)}</th> : <td key={c}>{inline(cell.content, linkLabel)}</td>))}</tr>
        ))}</tbody></table>
      )
      default: return null
    }
  })
}

export function MarkdownView({ markdown }: { markdown: string }) {
  const store = useStore()
  const { t } = useAiwsText()
  const ast = useMemo(() => {
    try { return JSON.parse(store.core.markdown_to_richtext(markdown, 'pv', undefined)) as Node } catch { return null }
  }, [store, markdown])
  if (!ast) return <pre className="aiws-wish-context">{markdown}</pre>
  return <div className="aiws-md" data-testid="aiws-wish-preview-text">{blocks(ast.content, t('aiworkspace.wish.preview.link'))}</div>
}

function cellText(v: Json | undefined, text: AiwsText): string {
  if (v === null || v === undefined) return ''
  if (Array.isArray(v)) return text.list(v.map((x) => cellText(x, text)))
  if (typeof v === 'number') return text.num(v, { maximumFractionDigits: 6 })
  if (typeof v === 'object') return JSON.stringify(v)
  return String(v)
}

export function TablePreview({ fields, rows, total, keyFields, max = 8 }: { fields: { name: string; type?: string }[]; rows: Record<string, Json>[]; total?: number; keyFields?: string[]; max?: number }) {
  const text = useAiwsText()
  const count = total ?? rows.length
  return (
    <div className="aiws-wish-table-preview">
      <table className="aiws-rt-table" data-testid="aiws-wish-preview-table">
        <thead><tr>{fields.map((f) => <th key={f.name} title={f.type}>{f.name}{keyFields?.includes(f.name) ? ' 🔑' : ''}</th>)}</tr></thead>
        <tbody>{rows.slice(0, max).map((r, i) => <tr key={i}>{fields.map((f) => <td key={f.name}>{cellText(r[f.name], text)}</td>)}</tr>)}</tbody>
      </table>
      <div className="aiws-muted">{text.t(count > max ? 'aiworkspace.wish.preview.rowsShown' : 'aiworkspace.wish.preview.rows', { count, max })}</div>
    </div>
  )
}

type Tree = { text: string; children: unknown[] }

/** A control result's tree as the control would draw it (all expanded). */
function ControlPreview({ control }: { control: NonNullable<WishResult['control']> }) {
  const { t } = useAiwsText()
  const svg = useMemo(() => {
    const by = new Map<string, ControlEntity & { parent: string }>()
    by.set('c', { entity_id: 'c', type_id: 'buckyos.container', kind: control.kind, config: control.config ?? {}, parent: '' })
    let n = 0
    const add = (list: Tree[], parent: string) => list.forEach((node) => {
      const id = `n${n++}`
      by.set(id, { entity_id: id, type_id: 'buckyos.cell', view_type: 'mindnode', title: node.text, parent })
      add(node.children as Tree[], id)
    })
    add(control.nodes, 'c')
    const src = { get: (id: string) => by.get(id), childrenOf: (id: string) => [...by.values()].filter((e) => e.parent === id) }
    return controlSvg(src, solveControl(src, 'c', { measurer: nodeMeasurer }), { measurer: nodeMeasurer, theme: { fill: 'var(--cp-surface-opaque, #fff)', rootFill: 'var(--cp-text, #1f2937)', text: 'var(--cp-text, #1f2937)', rootText: 'var(--cp-surface-opaque, #fff)', stroke: 'var(--cp-border, #d0d5dd)' } })
  }, [control])
  return (
    <div className="aiws-wish-control-preview" data-testid="aiws-wish-control-preview">
      {control.under && <div className="aiws-muted">{t('aiworkspace.wish.preview.under', { name: control.under })}</div>}
      <div className="aiws-wish-control-art" dangerouslySetInnerHTML={{ __html: svg }} />
    </div>
  )
}

export function ResultPreview({ result }: { result: WishResult }) {
  const text = useAiwsText()
  const { t } = text
  switch (result.type) {
    case 'table': {
      const t = result.table
      if (!t) return null
      return <TablePreview fields={t.fields} rows={t.rows} keyFields={t.key} />
    }
    case 'table_columns': {
      const c = result.columns
      if (!c) return null
      const record = t('aiworkspace.wish.preview.record')
      const rows = Object.entries(c.values).map(([id, v]) => ({ [record]: id, ...v }))
      return <TablePreview fields={[{ name: record }, ...c.fields]} rows={rows} />
    }
    case 'record': return (
      <dl className="aiws-wish-record-preview">
        {Object.entries(result.record?.props ?? {}).map(([k, v]) => <div key={k}><dt>{k}</dt><dd>{cellText(v, text)}</dd></div>)}
      </dl>
    )
    case 'richtext': return <MarkdownView markdown={result.markdown ?? ''} />
    case 'image': case 'asset': {
      const f = result.file
      if (!f) return null
      if (f.object_id && f.media_type.startsWith('image/')) return <div className="aiws-wish-image-preview"><AssetBlobImage objectId={f.object_id} mediaType={f.media_type} alt={result.title ?? result.name} /></div>
      return <div className="aiws-muted">{f.media_type} · {t('aiworkspace.wish.preview.bytes', { size: text.num(f.size), count: f.size })}</div>
    }
    case 'html': {
      const h = result.html
      if (!h) return null
      // the static markup only: scripts run once the Block is placed and activated
      return (
        <div>
          <iframe className="aiws-wish-html-preview" sandbox="" title={result.title ?? result.name} srcDoc={`<!doctype html><meta charset="utf-8"><style>${h.css ?? ''}</style>${h.html}`} />
          <div className="aiws-muted">{t('aiworkspace.wish.preview.bindings', { bindings: Object.entries(h.bindings ?? {}).map(([k, v]) => `${k} → ${v}`).join(t('aiworkspace.wish.commaSeparator')) || t('aiworkspace.common.none') })}</div>
        </div>
      )
    }
    case 'control': return result.control ? <ControlPreview control={result.control} /> : null
    default: return null
  }
}

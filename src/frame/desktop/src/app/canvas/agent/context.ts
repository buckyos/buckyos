/* ── Build the explicit context snapshot sent to the agent (PRD §13.1 / §16.2) ── */

import { cellDisplay, refKey, tableToRecords } from '../domain/selectors'
import type { CanvasDocument, ContextRef, WishBlock } from '../domain/types'
import type { AgentContextItem } from './contracts'

export const MAX_CONTEXT_ROWS = 2_000
export const MAX_CONTEXT_CHARS = 200_000

export function buildContext(doc: CanvasDocument, wish: WishBlock): { items: AgentContextItem[]; warnings: string[] } {
  const items: AgentContextItem[] = []
  const warnings: string[] = []
  for (const ref of wish.content.contextRefs) {
    const item = contextItemFor(doc, ref)
    if (!item) {
      warnings.push(`数据来源已不存在: ${ref.blockId}`)
      continue
    }
    items.push(item)
  }
  return { items, warnings }
}

export function contextItemFor(doc: CanvasDocument, ref: ContextRef): AgentContextItem | null {
  const block = doc.blocks[ref.blockId]
  if (!block) return null
  const key = refKey(ref)
  const title = block.title ?? block.type
  switch (block.type) {
    case 'table': {
      let content = block.content
      if (ref.kind === 'tableRange') {
        const r = ref.range
        content = {
          ...content,
          columns: content.columns.slice(r.colStart, r.colEnd + 1),
          rows: content.rows.slice(r.rowStart, r.rowEnd + 1),
        }
      }
      const totalRows = content.rows.length
      const truncated = totalRows > MAX_CONTEXT_ROWS
      const rows = tableToRecords({ ...content, rows: content.rows.slice(0, MAX_CONTEXT_ROWS) })
      return {
        kind: 'table',
        refKey: key,
        blockId: block.id,
        title,
        columns: content.columns.map((c) => ({ name: c.name, type: c.inferredType ?? 'string' })),
        rows,
        totalRows,
        truncated,
        revision: block.dataRevision,
      }
    }
    case 'text':
      return { kind: 'text', refKey: key, blockId: block.id, title, text: block.content.text.slice(0, MAX_CONTEXT_CHARS), revision: block.dataRevision }
    case 'metric':
      return {
        kind: 'metric',
        refKey: key,
        blockId: block.id,
        title,
        label: block.content.label,
        value: String(block.content.value),
        revision: block.dataRevision,
      }
    case 'image':
      return {
        kind: 'image',
        refKey: key,
        blockId: block.id,
        title,
        src: block.content.src,
        alt: block.content.alt,
        caption: block.content.caption,
        width: block.content.naturalWidth,
        height: block.content.naturalHeight,
        revision: block.dataRevision,
      }
    case 'video':
      return {
        kind: 'other',
        refKey: key,
        blockId: block.id,
        title,
        type: 'video',
        summary: `${block.content.caption ?? ''} ${block.content.frames ? `${block.content.frames.length} 帧` : ''}`.trim(),
        revision: block.dataRevision,
      }
    case 'group': {
      const children = block.content.childBlockIds.map((id) => doc.blocks[id]).filter(Boolean)
      const items = children
        .map((b) => contextItemFor(doc, { kind: 'block', blockId: b.id, revision: b.dataRevision }))
        .filter((it): it is AgentContextItem => Boolean(it))
      const parts = children.map((b) => {
        if (b.type === 'text') return b.content.text
        if (b.type === 'metric') return `${b.content.label}: ${b.content.value}`
        if (b.type === 'table') return `${b.title ?? '表格'}: ${b.content.rows.length} 行`
        if (b.type === 'image') return `[图片] ${b.title ?? ''} ${b.content.alt ?? ''}`.trim()
        return `${b.type}`
      })
      return { kind: 'group', refKey: key, blockId: block.id, title, summary: block.content.summary ?? parts.join('\n'), items, revision: block.dataRevision }
    }
    case 'chart':
      return { kind: 'other', refKey: key, blockId: block.id, title, type: 'chart', summary: block.content.caption ?? block.content.chartType, revision: block.dataRevision }
    default:
      return { kind: 'other', refKey: key, blockId: block.id, title, type: block.type, summary: '', revision: block.dataRevision }
  }
}

/** Approximate payload size in bytes (for adapter limits and the "what will be sent" panel). */
export function contextSize(items: AgentContextItem[]): number {
  return new Blob([JSON.stringify(items)]).size
}

export function contextPreviewLines(items: AgentContextItem[]): string[] {
  return items.map((it) => {
    if (it.kind === 'table') return `${it.title}：${it.totalRows} 行 × ${it.columns.length} 列${it.truncated ? `（仅发送前 ${MAX_CONTEXT_ROWS} 行）` : ''}`
    if (it.kind === 'text') return `${it.title}：${it.text.length} 字`
    if (it.kind === 'metric') return `${it.title}：${it.label} = ${it.value}`
    if (it.kind === 'image') return `${it.title}：图片${it.width && it.height ? ` ${it.width}×${it.height}` : ''}`
    if (it.kind === 'group') return `${it.title}：结果组（${it.items.length} 个成员）`
    return `${it.title}（${it.type}）`
  })
}

export { cellDisplay }

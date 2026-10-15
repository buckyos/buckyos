/* Exporting a free canvas as a draw.io file (draw.io 导入 TODO D5): the Surface's BlockTree is read (each Cell's
 * payload, the notes' and rich texts' words, the pictures' bytes) and written by the shared exporter
 * (`@drawio/export`), which starts from the draw.io style an import kept. */

import { exportDiagram, type ExportNode } from '@drawio/export'
import { bytesToBase64 } from '@drawio/bytes'
import type { CellPayload, EntityEnvelope, Json, KeyedContent } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'

/** The words of a rich text document (one line per block). */
function plainText(node: unknown): string {
  const n = node as { type?: string; text?: string; content?: unknown[] } | null
  if (!n) return ''
  if (typeof n.text === 'string') return n.text
  const parts = (n.content ?? []).map(plainText)
  return n.type === 'doc' || n.type === 'bullet_list' || n.type === 'ordered_list' || n.type === 'list_item' ? parts.filter(Boolean).join('\n') : parts.join('')
}

const CONNECTOR_KEYS = ['start', 'end', 'flip', 'route', 'controls', 'label'] as const

export async function exportSurfaceDrawio(store: WorkspaceStore, surface: EntityEnvelope): Promise<{ xml: string; skipped: Map<string, number> }> {
  const nodes: ExportNode[] = []
  const walk = async (parentId: string, parent: string | null) => {
    for (const entity of store.outline.childrenOf(parentId)) {
      if (entity.deleted || !entity.placement) continue
      if (entity.type_id === 'buckyos.container' && entity.kind === 'group') {
        nodes.push({ id: entity.entity_id, parent, kind: 'group', placement: entity.placement, ...(entity.title ? { title: entity.title } : {}), ...(entity.locked ? { locked: true } : {}) })
        await walk(entity.entity_id, entity.entity_id)
        continue
      }
      if (entity.type_id !== 'buckyos.cell') continue
      const read = await store.session.read<KeyedContent<CellPayload>>(entity.entity_id)
      const payload = read.content.payload
      const node: ExportNode = {
        id: entity.entity_id, parent, kind: 'cell', view: payload.view.type, placement: entity.placement,
        ...(payload.title ? { title: payload.title } : {}), ...(payload.config ? { config: payload.config as Record<string, unknown> } : {}), ...(entity.locked ? { locked: true } : {}),
      }
      if (payload.view.type === 'connector') node.line = Object.fromEntries(CONNECTOR_KEYS.flatMap((k) => (payload[k] !== undefined ? [[k, payload[k] as Json]] : [])))
      const sourceId = payload.source_ref?.entity_id
      if (sourceId) {
        try {
          const source = await store.session.read<{ payload: Record<string, Json> } & { content?: unknown }>(sourceId)
          const content = source.content as unknown as { payload?: Record<string, Json>; content?: unknown }
          if (payload.view.type === 'asset' && typeof content.payload?.object_id === 'string') {
            const blob = await store.session.fetchAsset(content.payload.object_id)
            node.image = `data:${blob.type || content.payload.media_type || 'application/octet-stream'};base64,${bytesToBase64(new Uint8Array(await blob.arrayBuffer()))}`
          } else if (payload.view.type === 'note') node.text = String(content.payload?.body ?? '')
          else if (payload.view.type === 'richtext') node.text = plainText(content.payload?.content ?? content.content)
        } catch { /* the Block exports with its title only */ }
      }
      nodes.push(node)
    }
  }
  await walk(surface.entity_id, null)
  const read = await store.session.read<{ payload: { config?: Record<string, Json> } }>(surface.entity_id)
  const config = read.content.payload.config ?? {}
  const size = config.page_size as { w?: number; h?: number } | undefined
  return exportDiagram(nodes, {
    name: surface.title ?? surface.name ?? 'Page-1',
    background: typeof config.background === 'string' ? config.background : null,
    pageSize: size && typeof size.w === 'number' && typeof size.h === 'number' ? { w: size.w, h: size.h } : null,
  })
}

export function saveTextFile(fileName: string, text: string, type = 'application/xml') {
  const url = URL.createObjectURL(new Blob([text], { type }))
  const link = document.createElement('a')
  link.href = url
  link.download = fileName
  document.body.appendChild(link)
  link.click()
  link.remove()
  window.setTimeout(() => URL.revokeObjectURL(url), 10_000)
}

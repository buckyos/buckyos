/* Exporting a free canvas as a draw.io file (draw.io 导入 TODO D5): the Surface's BlockTree is read (each Cell's
 * payload, the notes' and rich texts' words, the pictures' bytes) and written by the shared exporter
 * (`@drawio/export`), which starts from the draw.io style an import kept. A mind map is materialised (脑图 TODO M4):
 * draw.io has no such thing, so its nodes become positioned shapes and its implicit edges real lines. */

import { exportDiagram, type ExportNode } from '@drawio/export'
import { bytesToBase64 } from '@drawio/bytes'
import { isControlKind } from '@controls/contract'
import { exportControl, nodeShapeConfig } from '@controls/drawio'
import { plainText as plainInline } from '@controls/inline'
import { fixed, num } from '@controls/placement'
import type { AssetPayload, CellPayload, EntityEnvelope, Json, KeyedContent } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { assetMediaType, mediaKindOf } from '../media/kinds'
import { nodeMeasurer } from '../controls/measure'

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
  /** A Cell as a draw.io node: its payload, the words or picture of its data. */
  const cellNode = async (entity: EntityEnvelope, parent: string | null, placement: ExportNode['placement']): Promise<ExportNode> => {
    const read = await store.session.read<KeyedContent<CellPayload>>(entity.entity_id)
    const payload = read.content.payload
    const node: ExportNode = {
      id: entity.entity_id, parent, kind: 'cell', view: payload.view.type, placement,
      ...(payload.title ? { title: payload.title } : {}), ...(payload.config ? { config: payload.config as Record<string, unknown> } : {}), ...(entity.locked ? { locked: true } : {}),
    }
    if (payload.view.type === 'connector') node.line = Object.fromEntries(CONNECTOR_KEYS.flatMap((k) => (payload[k] !== undefined ? [[k, payload[k] as Json]] : [])))
    const sourceId = payload.source_ref?.entity_id
    if (sourceId) {
      try {
        const source = await store.session.read<{ payload: Record<string, Json> } & { content?: unknown }>(sourceId)
        const content = source.content as unknown as { payload?: Record<string, Json>; content?: unknown }
        if (payload.view.type === 'asset' && typeof content.payload?.object_id === 'string') {
          // a picture as itself; any other file as its poster (多媒体对象 §5), or the title alone
          const asset = content.payload as unknown as AssetPayload
          const picture = mediaKindOf(asset.media_type, asset.file_name) === 'image' ? asset.object_id : asset.poster
          if (picture) {
            const blob = await store.session.fetchAsset(picture)
            const type = picture === asset.object_id ? assetMediaType(asset.media_type, asset.file_name) : 'image/jpeg'
            node.image = `data:${blob.type && blob.type !== 'application/octet-stream' ? blob.type : type};base64,${bytesToBase64(new Uint8Array(await blob.arrayBuffer()))}`
          }
          if (!node.title) node.title = asset.file_name
        } else if (payload.view.type === 'note') node.text = String(content.payload?.body ?? '')
        else if (payload.view.type === 'richtext') node.text = plainText(content.payload?.content ?? content.content)
      } catch { /* the Block exports with its title only */ }
    }
    return node
  }
  /** A mind map materialised (脑图 TODO M4): groups at the control boxes, nodes at their rectangles, the implicit edges
   * as lines bound to the nodes; text nodes as rounded shapes of their depth. */
  const control = async (entity: EntityEnvelope, parent: string | null, origin: { x: number; y: number }) => {
    const out = exportControl(store.outline, entity.entity_id, nodeMeasurer, parent, origin)
    for (const g of out.groups) nodes.push({ id: g.id, parent: g.parent, kind: 'group', placement: g.placement, ...(g.title ? { title: g.title } : {}) })
    for (const n of out.nodes) {
      const e = store.outline.get(n.id)
      if (!e) continue
      if (n.text) nodes.push({ id: n.id, parent: n.control, kind: 'cell', view: 'shape', placement: n.placement, config: nodeShapeConfig(n.depth, n.color), ...(e.title ? { title: plainInline(e.title) } : {}) })
      else if (e.type_id === 'buckyos.cell') nodes.push(await cellNode(e, n.control, n.placement))
      else {
        nodes.push({ id: n.id, parent: n.control, kind: 'group', placement: n.placement, ...(e.title ? { title: e.title } : {}) })
        await walk(n.id, n.id)
      }
    }
    for (const edge of out.edges) {
      nodes.push({ id: edge.id, parent: edge.control, kind: 'cell', view: 'connector', placement: edge.placement, config: { stroke: edge.color, width: edge.width, end_cap: 'none' },
        line: { start: { entity_id: edge.from, anchor: { kind: 'named', id: edge.fromAnchor } }, end: { entity_id: edge.to, anchor: { kind: 'named', id: edge.toAnchor } }, route: edge.route, ...(edge.flip.h || edge.flip.v ? { flip: edge.flip } : {}) } })
    }
  }
  const walk = async (parentId: string, parent: string | null) => {
    for (const entity of store.outline.childrenOf(parentId)) {
      if (entity.deleted || !entity.placement) continue
      if (entity.type_id === 'buckyos.container' && isControlKind(entity.kind)) {
        await control(entity, parent, { x: num(entity.placement.x) ?? 0, y: num(entity.placement.y) ?? 0 })
        continue
      }
      const placement = fixed(entity.placement)
      if (!placement) continue
      if (entity.type_id === 'buckyos.container' && entity.kind === 'group') {
        nodes.push({ id: entity.entity_id, parent, kind: 'group', placement, ...(entity.title ? { title: entity.title } : {}), ...(entity.locked ? { locked: true } : {}) })
        await walk(entity.entity_id, entity.entity_id)
        continue
      }
      if (entity.type_id !== 'buckyos.cell') continue
      nodes.push(await cellNode(entity, parent, placement))
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

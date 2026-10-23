/* Labels and data-creation operations of the data tree (non-component helpers). */

import { randomId } from '../../api/ids'
import { SYSTEM_IDS, type EntityEnvelope, type Json, type Operation } from '../../api/types'
import { aiwsT, localizedLabels } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import type { ImportedAsset } from '../../api/session'
import { MOCK_WISH_DEF_ID, mockWishDefOp } from '../wish/mockWishDef'

export const TYPE_LABEL: Readonly<Record<string, string>> = localizedLabels({
  'buckyos.container': 'aiworkspace.data.type.container', 'buckyos.record': 'aiworkspace.data.type.record', 'buckyos.richtext': 'aiworkspace.data.type.richtext',
  'buckyos.table-source': 'aiworkspace.data.type.tableSource', 'buckyos.cell': 'aiworkspace.data.type.cell', 'buckyos.asset-ref': 'aiworkspace.data.type.assetRef',
  'buckyos.annotation': 'aiworkspace.data.type.annotation', 'buckyos.wish': 'aiworkspace.data.type.wish', 'buckyos.block-def': 'aiworkspace.data.type.blockDef',
  'buckyos.viewport': 'aiworkspace.data.type.viewport', 'buckyos.show-path': 'aiworkspace.data.type.showPath',
})

export function entityLabel(entity: EntityEnvelope): string {
  return entity.title ?? entity.name ?? entity.entity_id
}

export type NewDataKind = 'folder' | 'table' | 'richtext' | 'record' | 'wish' | 'note'

export function importedAssetOps(store: WorkspaceStore, parentId: string, items: ImportedAsset[], canvas?: { parentId: string; free: boolean; x: number; y: number }): Operation[] {
  const operations: Operation[] = []
  const walk = (parent: string, children: ImportedAsset[], surface?: typeof canvas) => {
    let key = store.outline.childrenOf(parent).at(-1)?.order_key
    let viewKey = surface ? store.outline.childrenOf(surface.parentId).at(-1)?.order_key : undefined
    children.forEach((item, index) => {
      key = store.core.order_key_between(key, undefined)
      const id = randomId(item.kind === 'dir' ? 'f' : 'a')
      operations.push({ op: 'entity.create', entity_id: id, type_id: item.kind === 'dir' ? 'buckyos.container' : 'buckyos.asset-ref', parent_id: parent, order_key: key, name: item.name,
        payload: item.kind === 'dir' ? { kind: 'folder', title: item.name } : { object_id: item.object_id, file_name: item.name } })
      let childSurface: typeof canvas
      if (surface) {
        viewKey = store.core.order_key_between(viewKey, undefined)
        const viewId = randomId('c')
        operations.push({ op: 'entity.create', entity_id: viewId, type_id: item.kind === 'dir' ? 'buckyos.container' : 'buckyos.cell', parent_id: surface.parentId, order_key: viewKey,
          ...(surface.free ? { placement: { x: surface.x + (index % 3) * 344, y: surface.y + Math.floor(index / 3) * 264, w: 320, h: 240 } } : {}),
          payload: item.kind === 'dir' ? { kind: 'group', title: item.name, layout: { mode: 'flow' } } : { view: { type: 'asset' }, source_ref: { entity_id: id }, title: item.name } })
        childSurface = { parentId: viewId, free: false, x: 0, y: 0 }
      }
      if (item.kind === 'dir') walk(id, item.children, childSurface)
    })
  }
  walk(parentId, items, canvas)
  return operations
}

/** Operations creating new data in the data tree (no Block). */
export function newDataOps(store: WorkspaceStore, parentId: string, kind: NewDataKind, title: string): Operation[] {
  const key = store.core.order_key_between(store.outline.childrenOf(parentId).filter((e) => !SYSTEM_IDS.has(e.entity_id)).at(-1)?.order_key ?? undefined, undefined)
  const name = title.trim() || undefined
  const create = (id: string, typeId: string, payload: Record<string, Json>): Operation => ({ op: 'entity.create', entity_id: id, type_id: typeId, parent_id: parentId, order_key: key, ...(name ? { name } : {}), payload })
  switch (kind) {
    case 'folder': return [create(randomId('f'), 'buckyos.container', { kind: 'folder', title: title || aiwsT('aiworkspace.data.default.folder') })]
    case 'table': return [create(randomId('s'), 'buckyos.table-source', { title_field_id: 'title', fields: [{ field_id: 'title', name: aiwsT('aiworkspace.data.default.title'), type: 'text', required: true }, { field_id: 'done', name: aiwsT('aiworkspace.data.default.done'), type: 'boolean' }] })]
    case 'richtext': return [create(randomId('t'), 'buckyos.richtext', { content: { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: randomId('b') }, content: title ? [{ type: 'text', text: title }] : [] }] } })]
    case 'record': return [create(randomId('o'), 'buckyos.record', { schema: { properties: [
      { key: 'name', name: aiwsT('aiworkspace.data.default.name'), type: 'text' }, { key: 'date', name: aiwsT('aiworkspace.data.default.date'), type: 'date' },
      { key: 'amount', name: aiwsT('aiworkspace.data.default.amount'), type: 'decimal', scale: 2 },
    ] }, props: { name: title } })]
    case 'note': return [create(randomId('n'), 'buckyos.annotation', { kind: 'note', body: title || aiwsT('aiworkspace.data.stickyNote'), style: { color: '#fff2cc' } })]
    case 'wish': {
      const ops: Operation[] = []
      if (!store.outline.get(MOCK_WISH_DEF_ID)) ops.push(mockWishDefOp(store.core.order_key_between(store.outline.childrenOf('data').filter((e) => !SYSTEM_IDS.has(e.entity_id)).at(-1)?.order_key ?? undefined, undefined)) as Operation)
      const wishTitle = title || aiwsT('aiworkspace.data.default.wish')
      ops.push(create(randomId('w'), 'buckyos.wish', { title: wishTitle, prompt: title, executor: 'mock', output_mode: 'overwrite', output: { container_id: parentId, name: aiwsT('aiworkspace.data.default.wishResult', { name: wishTitle }) } }))
      return ops
    }
  }
}

/* Labels and data-creation operations of the data tree (non-component helpers). */

import { randomId } from '../../api/ids'
import type { EntityEnvelope, Json, Operation } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { MOCK_WISH_DEF_ID, mockWishDefOp } from '../wish/mockWishDef'

export const TYPE_LABEL: Record<string, string> = {
  'buckyos.container': '文件夹', 'buckyos.record': '记录', 'buckyos.richtext': '富文本', 'buckyos.table-source': '表',
  'buckyos.cell': 'Block', 'buckyos.asset-ref': '资产', 'buckyos.annotation': '便签/批注', 'buckyos.wish': '许愿格', 'buckyos.block-def': 'Block 定义',
}

export function entityLabel(entity: EntityEnvelope): string {
  return entity.title ?? entity.name ?? entity.entity_id
}

export type NewDataKind = 'folder' | 'table' | 'richtext' | 'record' | 'wish' | 'note'

/** Operations creating new data in the data tree (no Block). */
export function newDataOps(store: WorkspaceStore, parentId: string, kind: NewDataKind, title: string): Operation[] {
  const key = store.core.order_key_between(store.outline.childrenOf(parentId).filter((e) => e.entity_id !== 'canvas-content').at(-1)?.order_key ?? undefined, undefined)
  const name = title.trim() || undefined
  const create = (id: string, typeId: string, payload: Record<string, Json>): Operation => ({ op: 'entity.create', entity_id: id, type_id: typeId, parent_id: parentId, order_key: key, ...(name ? { name } : {}), payload })
  switch (kind) {
    case 'folder': return [create(randomId('f'), 'buckyos.container', { kind: 'folder', title: title || '文件夹' })]
    case 'table': return [create(randomId('s'), 'buckyos.table-source', { title_field_id: 'title', fields: [{ field_id: 'title', name: '标题', type: 'text', required: true }, { field_id: 'done', name: '完成', type: 'boolean' }] })]
    case 'richtext': return [create(randomId('t'), 'buckyos.richtext', { content: { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: randomId('b') }, content: title ? [{ type: 'text', text: title }] : [] }] } })]
    case 'record': return [create(randomId('o'), 'buckyos.record', { schema: { properties: [{ key: 'name', name: '名称', type: 'text' }, { key: 'date', name: '日期', type: 'date' }, { key: 'amount', name: '金额', type: 'decimal', scale: 2 }] }, props: { name: title } })]
    case 'note': return [create(randomId('n'), 'buckyos.annotation', { kind: 'note', body: title || '便签', style: { color: '#fff2cc' } })]
    case 'wish': {
      const ops: Operation[] = []
      if (!store.outline.get(MOCK_WISH_DEF_ID)) ops.push(mockWishDefOp(store.core.order_key_between(store.outline.childrenOf('data').filter((e) => e.entity_id !== 'canvas-content').at(-1)?.order_key ?? undefined, undefined)) as Operation)
      ops.push(create(randomId('w'), 'buckyos.wish', { title: title || '许愿格', prompt: title, executor: 'mock', output_mode: 'overwrite', output: { container_id: parentId, name: `${title || '许愿格'}的结果` } }))
      return ops
    }
  }
}

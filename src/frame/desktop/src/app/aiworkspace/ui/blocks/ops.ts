/* Operation builders shared by Block definitions and insertion. */

import type { Json, Operation } from '../../api/types'
import type { CreateArgs } from './registry'

export const MAX_EMBED_DEPTH = 3

export function createOp(entityId: string, typeId: string, parentId: string, orderKey: string, payload: Record<string, Json>, name?: string, placement?: CreateArgs['placement']): Operation {
  return { op: 'entity.create', entity_id: entityId, type_id: typeId, parent_id: parentId, order_key: orderKey, ...(name ? { name } : {}), ...(placement ? { placement } : {}), payload }
}

export function cellOp(args: CreateArgs, view: string, sourceId: string | undefined, extra: Record<string, Json> = {}): Operation {
  const payload: Record<string, Json> = { view: { type: view }, ...(sourceId ? { source_ref: { entity_id: sourceId } } : {}), ...(args.title ? { title: args.title } : {}), ...extra }
  if (args.config) payload.config = args.config
  return createOp(args.cellId, 'buckyos.cell', args.parentId, args.orderKey, payload, undefined, args.placement)
}

/** The Renderer used to show a data entity by default ("add to canvas"). */
export function defaultRendererFor(typeId: string): string | null {
  switch (typeId) {
    case 'buckyos.table-source': return 'table'
    case 'buckyos.richtext': return 'richtext'
    case 'buckyos.record': return 'record'
    case 'buckyos.asset-ref': return 'asset'
    case 'buckyos.annotation': return 'note'
    case 'buckyos.wish': return 'wish'
    default: return null
  }
}

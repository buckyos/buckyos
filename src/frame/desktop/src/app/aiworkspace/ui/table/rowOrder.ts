import type { Operation, QueryRow, RowOrder } from '../../api/types'
import type { AiwsCore } from '../../api/wasm'
import type { CellRead } from './TableEditor'

export function placeRow(core: AiwsCore, order: RowOrder | undefined, recordId: string, anchor: QueryRow, side: 'before' | 'after'): RowOrder {
  const next = { ...order }
  if (anchor.record_id === recordId) return next
  delete next[recordId]
  const existing = Object.hasOwn(next, anchor.record_id) ? next[anchor.record_id] : undefined
  const sequence = anchor.created_seq?.toString(16).padStart(16, '0')
  const rank = existing ?? (sequence && `${sequence}!${anchor.record_id}!h`)
  if (!rank) throw new Error('The record has no creation sequence')
  const split = rank.lastIndexOf('!') + 1
  const prefix = rank.slice(0, split)
  const anchorFraction = rank.slice(split)
  const fractions = ['h']
  const group = (key: string) => key.startsWith('0000000000000000!') ? key.slice(0, key.lastIndexOf('!')) : key.slice(16, key.lastIndexOf('!'))
  const anchorGroup = group(rank)
  for (const key of Object.values(next)) {
    if (group(key) === anchorGroup) fractions.push(key.slice(key.lastIndexOf('!') + 1))
  }
  fractions.sort()
  const before = side === 'before' ? fractions.filter((key) => key < anchorFraction).at(-1) : anchorFraction
  const after = side === 'after' ? fractions.find((key) => key > anchorFraction) : anchorFraction
  next[recordId] = prefix + core.order_key_between(before, after)
  return next
}

export function rowOrderOp(cell: CellRead, order: RowOrder): Operation {
  return { op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'row_order', value: order, expect: { rev: cell.content.key_revs.row_order ?? 0 } }] }
}

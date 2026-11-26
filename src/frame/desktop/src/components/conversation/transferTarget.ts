/* A conversation's composer as a window transfer target (剪贴板规范 §5): the Shell asks what the window would do with
 * a selection and then hands it the chosen representations. Zone files and folders and files of this device go into
 * the attachment draft; sending stays the user's decision, so a delivery is `accepted`, never `done`. */
import { createContext } from 'react'
import { readZoneContent, type ZoneContent } from 'buckyos/content'
import { CONTENT_REFS_FORMAT, readContentRefs } from 'buckyos/transfer'
import type { TransferDelivery, TransferDeliveryResult, TransferItemStatus, TransferOperation, TransferProbeAnswer, WindowTransferTarget } from '../../transfer'
import type { ConversationComposerHandle } from './input/ConversationComposer'

/** The desktop window a conversation is shown in; with one, its composer is that window's transfer target. */
export const ConversationWindowContext = createContext<string | undefined>(undefined)

/** A draft never consumes a cut: it copies the bytes (or keeps a reference when that is all the source allows). */
const operationFor = (operations: TransferOperation[]): TransferOperation | undefined =>
  operations.includes('copy') ? 'copy' : operations.includes('link') ? 'link' : undefined

/** The Zone items one delivered value names: a `ZoneContent`, a reference list (its value or parsed), or a list of items. */
function zoneItemsOf(value: unknown): ZoneContent[] | undefined {
  if (typeof value === 'string') return readContentRefs(value)
  if (Array.isArray(value)) {
    const items = value.map(readZoneContent)
    return items.length && items.every(Boolean) ? items as ZoneContent[] : undefined
  }
  if (value && typeof value === 'object' && 'items' in value) return readContentRefs(JSON.stringify(value))
  const item = readZoneContent(value)
  return item ? [item] : undefined
}

function filesOf(value: unknown): File[] | undefined {
  const all = value instanceof File ? [value] : Array.isArray(value) || (typeof FileList !== 'undefined' && value instanceof FileList) ? Array.from(value as ArrayLike<unknown>) : []
  return all.length && all.every((file) => file instanceof File) ? all as File[] : undefined
}

export function composerTransferTarget(targetId: string, action: string, composer: () => ConversationComposerHandle | null): WindowTransferTarget {
  const delivered = new Map<string, Promise<TransferDeliveryResult>>()
  const deliver = async ({ targetId: id, items }: TransferDelivery): Promise<TransferDeliveryResult> => {
    const handle = composer()
    if (id !== targetId || !handle) return { status: 'failed', items: items.map(({ itemKey }) => ({ itemKey, status: 'failed', error: 'stale' })) }
    const out = new Map<string, { status: TransferItemStatus; error?: string }>()
    const zone: Array<{ itemKey: string; items: ZoneContent[] }> = []
    const local: File[] = []
    for (const { itemKey, format, value } of items) {
      if (format === CONTENT_REFS_FORMAT) {
        const refs = zoneItemsOf(value)
        if (refs) zone.push({ itemKey, items: refs })
        else out.set(itemKey, { status: 'failed', error: 'unsupported' })
      } else if (format === 'files') {
        const files = filesOf(value)
        if (files) { local.push(...files); out.set(itemKey, { status: 'accepted' }) } else out.set(itemKey, { status: 'failed', error: 'unsupported' })
      } else out.set(itemKey, { status: 'skipped' })
    }
    if (local.length) handle.addFiles(local)
    if (zone.length) {
      const results = await handle.addZoneContent(zone.flatMap((entry) => entry.items))
      let next = 0
      for (const entry of zone) {
        const own = results.slice(next, next + entry.items.length)
        next += entry.items.length
        const failed = own.find((r) => r.status === 'failed')
        out.set(entry.itemKey, own.some((r) => r.status === 'accepted') ? { status: 'accepted' }
          : failed && failed.status === 'failed' ? { status: 'failed', error: failed.error }
            : own.every((r) => r.status === 'cancelled') ? { status: 'cancelled' } : { status: 'skipped' })
      }
    }
    const receipts = items.map(({ itemKey }) => ({ itemKey, ...(out.get(itemKey) ?? { status: 'skipped' as const }) }))
    const status = receipts.some((r) => r.status === 'accepted') ? 'accepted' : receipts.every((r) => r.status === 'cancelled') ? 'cancelled' : 'failed'
    return { status, items: receipts }
  }
  return {
    probe({ items }): TransferProbeAnswer | null {
      if (!composer()) return null
      const accept = items.flatMap((item) => {
        const operation = operationFor(item.operations)
        if (!operation) return []
        if ((item.kind === 'zone-file' || item.kind === 'zone-folder') && item.formats.includes(CONTENT_REFS_FORMAT)) return [{ itemKey: item.itemKey, format: CONTENT_REFS_FORMAT, operation }]
        if (item.kind === 'local-file' && item.formats.includes('files')) return [{ itemKey: item.itemKey, format: 'files', operation }]
        return []
      })
      return accept.length ? { targetId, accept, action } : null
    },
    deliver(delivery) {
      // One commit per operation: a repeated delivery gets the first one's receipt.
      let pending = delivered.get(delivery.operationId)
      if (!pending) {
        pending = deliver(delivery)
        delivered.set(delivery.operationId, pending)
      }
      return pending
    },
  }
}

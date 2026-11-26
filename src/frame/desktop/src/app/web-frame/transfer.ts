/* `transfer.v1` for a framed App (剪贴板规范 §5, §6): while a document of the App has its transfer link bound, its
 * window is a transfer target (probes and deliveries go to the App) and its offers are desktop records whose
 * representations are read from the App only when the Shell delivers them. A reload, navigation or close of the
 * document unregisters the target and withdraws its offers; a read still waiting on it fails `stale`. */
import { readTransferValue, TransferError, type AppFrameHost, type AppFrameHostCallbacks, type TransferOfferMessage, type TransferPoint } from 'buckyos/app-frame'
import { desktopTransfers, type TransferOfferItem, type TransferOperation } from '../../transfer/records'
import { registerWindowTransferTarget } from '../../transfer/targets'

/** Live offers kept for one document; an older one is withdrawn when the App offers more without withdrawing. */
const MAX_OFFERS = 8

interface Batch { operationId: string; offerId: string; generation: number; items: Array<{ itemKey: string; format: string }>; waiters: Map<string, { resolve: (v: unknown) => void; reject: (e: unknown) => void }> }

export function frameTransfer(options: { windowId: string; app: string; frame: () => HTMLIFrameElement | null }) {
  let host: AppFrameHost | undefined
  const offers = new Map<string, string[]>()
  let unregister: (() => void) | undefined
  const batches = new Map<string, Batch>()

  /** A point of the window's content box in the App's own page (the iframe may not fill the content box). */
  const toFrame = (at: TransferPoint): TransferPoint => {
    const iframe = options.frame()
    if (at === 'focus' || !iframe) return at
    const frame = iframe.getBoundingClientRect()
    const content = iframe.closest('[data-window-content]')?.getBoundingClientRect() ?? frame
    return { x: at.x + content.left - frame.left, y: at.y + content.top - frame.top }
  }

  // Representations asked for by one operation within a moment go to the App in one `transfer.read`.
  const read = (documentId: string, offer: TransferOfferMessage, itemKey: string, format: string, operationId: string): Promise<unknown> => {
    if (!host || host.transferDocument !== documentId) return Promise.reject(new TransferError('stale'))
    const key = `${offer.offerId}\n${operationId}`
    let batch = batches.get(key)
    if (!batch) {
      const created: Batch = { operationId, offerId: offer.offerId, generation: offer.generation, items: [], waiters: new Map() }
      batches.set(key, created)
      batch = created
      setTimeout(() => {
        batches.delete(key)
        const current = host
        if (!current || current.transferDocument !== documentId) { created.waiters.forEach((w) => w.reject(new TransferError('stale'))); return }
        void current.transferRead({ operationId, offerId: created.offerId, generation: created.generation, items: created.items }).then((values) => {
          for (const value of values) {
            const waiter = created.waiters.get(`${value.itemKey}\n${value.format}`)
            if ('value' in value) waiter?.resolve(value.value)
            else waiter?.reject(new TransferError(value.error === 'failed' ? 'unsupported' : value.error))
          }
        }, (error) => created.waiters.forEach((w) => w.reject(error)))
      }, 0)
    }
    const waiting = batch
    return new Promise((resolve, reject) => {
      waiting.items.push({ itemKey, format })
      waiting.waiters.set(`${itemKey}\n${format}`, { resolve, reject })
    })
  }

  const withdrawDocument = (documentId: string) => {
    for (const offerId of offers.get(documentId) ?? []) desktopTransfers.withdraw(offerId)
    offers.delete(documentId)
  }

  const callbacks: Pick<AppFrameHostCallbacks, 'onTransferConnect' | 'onTransferDisconnect' | 'onTransferOffer' | 'onTransferWithdraw'> = {
    onTransferConnect: (documentId) => {
      unregister?.()
      unregister = registerWindowTransferTarget(options.windowId, {
        probe: async (probe) => {
          if (!host || host.transferDocument !== documentId) return null
          return host.transferProbe({ ...probe, at: toFrame(probe.at) })
        },
        deliver: async (delivery) => {
          if (!host || host.transferDocument !== documentId) throw new TransferError('stale')
          return host.transferDeliver({ ...delivery, at: toFrame(delivery.at) })
        },
        status: async (operationId, items) => (host && host.transferDocument === documentId ? host.transferStatus(operationId, items) : null),
        cancel: (operationId) => { if (host?.transferDocument === documentId) host.transferCancel(operationId) },
      })
    },
    onTransferDisconnect: (documentId) => {
      unregister?.()
      unregister = undefined
      withdrawDocument(documentId)
    },
    onTransferOffer: (offer, documentId) => {
      if (host?.transferDocument !== documentId) return { ok: false, code: 'stale' }
      const items: TransferOfferItem[] = offer.items.map((item, index) => ({
        kind: item.kind,
        operations: item.operations,
        formats: Object.fromEntries(item.formats.map((format) => {
          const inline = item.inline?.[format] !== undefined ? readTransferValue(format, item.inline[format]) : undefined
          return [format, inline !== undefined ? inline : ({ operationId }: { operationId: string }) => read(documentId, offer, String(index), format, operationId)]
        })),
      }))
      const operations = [...new Set(offer.items.flatMap((item) => item.operations))] as TransferOperation[]
      // The window and App are the Shell's own knowledge; the App only names its selection.
      const record = desktopTransfers.offerWithId(offer.offerId, {
        purpose: offer.purpose, source: { app: options.app, windowId: options.windowId }, scope: offer.scope,
        itemKeys: items.map((_, index) => String(index)), items, formats: {}, operations,
      })
      if (!record) return { ok: false, code: 'denied' }
      const list = offers.get(documentId) ?? []
      list.push(record.offerId)
      while (list.length > MAX_OFFERS) desktopTransfers.withdraw(list.shift()!)
      offers.set(documentId, list)
      return { ok: true }
    },
    onTransferWithdraw: (offerId, documentId) => {
      const list = offers.get(documentId)
      if (!list?.includes(offerId)) return
      offers.set(documentId, list.filter((id) => id !== offerId))
      desktopTransfers.withdraw(offerId)
    },
  }
  return {
    callbacks,
    bind(next: AppFrameHost) { host = next },
    dispose() {
      unregister?.()
      unregister = undefined
      for (const documentId of [...offers.keys()]) withdrawDocument(documentId)
      host = undefined
    },
  }
}

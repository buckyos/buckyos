/**
 * Viewer-local state that never goes to the backend: unsent drafts (text and
 * files), the Action Message display filter and the tunnel write
 * confirmation are all keyed by `(viewerDid, ownerDid, sessionId)`.
 */
import type { ComposerAttachmentInput } from '../conversation/input/attachmentDraft'

interface LocalRecord {
  drafts: Record<string, string>
  draftAttachments: Record<string, ComposerAttachmentInput[]>
  showActions: Record<string, boolean>
  policies: Record<string, 'default' | 'allow' | 'deny'>
}

const DB_NAME = 'messagehub-local-v1'

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1)
    request.onupgradeneeded = () => request.result.createObjectStore('state')
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error)
  })
}

export class LocalStateStore {
  private db?: IDBDatabase
  private state: LocalRecord = { drafts: {}, draftAttachments: {}, showActions: {}, policies: {} }
  private queue = Promise.resolve()

  async load(): Promise<void> {
    if (typeof indexedDB === 'undefined') return
    try {
      this.db = await openDatabase()
      const stored = await new Promise<LocalRecord | undefined>((resolve, reject) => {
        const request = this.db!.transaction('state').objectStore('state').get('snapshot')
        request.onsuccess = () => resolve(request.result)
        request.onerror = () => reject(request.error)
      })
      if (stored) this.state = { ...this.state, ...stored, draftAttachments: stored.draftAttachments ?? {} }
    } catch (error) {
      console.warn('MessageHub local state unavailable, drafts will not persist.', error)
    }
  }

  get(): LocalRecord { return this.state }

  update(mutate: (next: LocalRecord) => void): Promise<void> {
    const result = this.queue.then(async () => {
      const next: LocalRecord = { drafts: { ...this.state.drafts }, draftAttachments: { ...this.state.draftAttachments }, showActions: { ...this.state.showActions }, policies: { ...this.state.policies } }
      mutate(next)
      this.state = next
      if (!this.db) return
      await new Promise<void>((resolve, reject) => {
        const transaction = this.db!.transaction('state', 'readwrite')
        transaction.objectStore('state').put(next, 'snapshot')
        transaction.oncomplete = () => resolve()
        transaction.onerror = () => reject(transaction.error ?? Error('storage_failed'))
        transaction.onabort = () => reject(transaction.error ?? Error('storage_failed'))
      })
    })
    this.queue = result.then(() => undefined, () => undefined)
    return result
  }
}

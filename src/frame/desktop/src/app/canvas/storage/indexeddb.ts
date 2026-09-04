/* ── IndexedDB storage adapter (PRD §14.1). Later: BuckyObjectCanvasStorage. ── */

import type { CanvasDocument, CanvasSnapshot } from '../domain/types'
import { newId, nowIso } from '../domain/ids'

export interface CanvasListItem {
  id: string
  title: string
  updatedAt: string
  blockCount: number
  sheetCount: number
  templateId?: string
}

export interface CanvasStorageAdapter {
  list(): Promise<CanvasListItem[]>
  load(id: string): Promise<CanvasDocument | null>
  save(doc: CanvasDocument, expectedRevision?: number): Promise<void>
  delete(id: string): Promise<void>
  createSnapshot(doc: CanvasDocument, name: string): Promise<string>
  listSnapshots(docId: string): Promise<CanvasSnapshot[]>
  deleteSnapshot(id: string): Promise<void>
}

const DB_NAME = 'buckyos-ai-canvas'
const DB_VERSION = 1

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    if (typeof indexedDB === 'undefined') return reject(new Error('当前环境不支持 IndexedDB'))
    const req = indexedDB.open(DB_NAME, DB_VERSION)
    req.onupgradeneeded = () => {
      const db = req.result
      if (!db.objectStoreNames.contains('documents')) db.createObjectStore('documents', { keyPath: 'id' })
      if (!db.objectStoreNames.contains('snapshots')) {
        const s = db.createObjectStore('snapshots', { keyPath: 'id' })
        s.createIndex('docId', 'docId')
      }
      if (!db.objectStoreNames.contains('feedback')) db.createObjectStore('feedback', { keyPath: 'id' })
    }
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error ?? new Error('打开数据库失败'))
  })
}

function tx<T>(store: string, mode: IDBTransactionMode, fn: (s: IDBObjectStore) => IDBRequest<T> | IDBRequest): Promise<T> {
  return openDb().then(
    (db) =>
      new Promise<T>((resolve, reject) => {
        const t = db.transaction(store, mode)
        const req = fn(t.objectStore(store))
        req.onsuccess = () => resolve(req.result as T)
        req.onerror = () => reject(req.error ?? new Error('数据库操作失败'))
        t.oncomplete = () => db.close()
      }),
  )
}

export class IndexedDbCanvasStorage implements CanvasStorageAdapter {
  async list(): Promise<CanvasListItem[]> {
    const docs = await tx<CanvasDocument[]>('documents', 'readonly', (s) => s.getAll())
    return docs
      .map((d) => ({
        id: d.id,
        title: d.title,
        updatedAt: d.updatedAt,
        blockCount: Object.keys(d.blocks ?? {}).length,
        sheetCount: d.sheets?.length ?? 0,
        templateId: d.metadata?.sourceTemplateId,
      }))
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
  }

  async load(id: string): Promise<CanvasDocument | null> {
    const doc = await tx<CanvasDocument | undefined>('documents', 'readonly', (s) => s.get(id))
    return doc ?? null
  }

  async save(doc: CanvasDocument, expectedRevision?: number): Promise<void> {
    if (expectedRevision !== undefined) {
      const current = await this.load(doc.id)
      if (current && current.revision > expectedRevision && current.revision !== doc.revision) {
        throw new Error('文档已被其他窗口修改')
      }
    }
    await tx('documents', 'readwrite', (s) => s.put(doc))
  }

  async delete(id: string): Promise<void> {
    await tx('documents', 'readwrite', (s) => s.delete(id))
    const snaps = await this.listSnapshots(id)
    for (const s of snaps) await this.deleteSnapshot(s.id)
  }

  async createSnapshot(doc: CanvasDocument, name: string): Promise<string> {
    const snap: CanvasSnapshot = { id: newId('snap'), docId: doc.id, name, createdAt: nowIso(), revision: doc.revision, doc: structuredClone(doc) }
    await tx('snapshots', 'readwrite', (s) => s.put(snap))
    return snap.id
  }

  async listSnapshots(docId: string): Promise<CanvasSnapshot[]> {
    const all = await tx<CanvasSnapshot[]>('snapshots', 'readonly', (s) => s.index('docId').getAll(docId))
    return all.sort((a, b) => b.createdAt.localeCompare(a.createdAt))
  }

  async deleteSnapshot(id: string): Promise<void> {
    await tx('snapshots', 'readwrite', (s) => s.delete(id))
  }
}

export interface FeedbackRecord {
  id: string
  createdAt: string
  canvasId?: string
  answers: Record<string, string>
  events: Record<string, number>
}

export async function saveFeedback(record: FeedbackRecord): Promise<void> {
  await tx('feedback', 'readwrite', (s) => s.put(record))
}

export async function listFeedback(): Promise<FeedbackRecord[]> {
  return tx<FeedbackRecord[]>('feedback', 'readonly', (s) => s.getAll())
}

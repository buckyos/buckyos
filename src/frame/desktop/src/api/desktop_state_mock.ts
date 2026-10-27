/* Mock-runtime stand-in for control-panel `/api/desktop` and the account's wallpapers bucket: the state lives
 * in this browser's localStorage and the pictures in IndexedDB by bucket key, so a reload (or a "new device"
 * that only loses the page's own caches) finds them again. Shapes follow ui_session_mgr.rs and the bucket. */

import { bucket } from 'buckyos'
import { WALLPAPER_APP_ID, WALLPAPER_BUCKET, wallpaperKey, wallpapersToPrune, type DesktopStateRead } from './desktop_state'

const STATE_PREFIX = 'buckyos.mock.desktop-state.'
const DB_NAME = 'buckyos-desktop-wallpaper'
const DB_STORE = 'assets'
const MOCK_USER = 'alice'

function readJson(key: string): unknown {
  try {
    const raw = window.localStorage.getItem(key)
    return raw === null ? null : JSON.parse(raw)
  } catch {
    return null
  }
}

export function readMockDesktopState<T>(stateKey: string): DesktopStateRead<T> {
  const value = readJson(STATE_PREFIX + stateKey) as T | null
  return { value, exists: value !== null }
}

export function writeMockDesktopState(stateKey: string, value: unknown, jsonPath?: string) {
  let root: unknown = value
  if (jsonPath) {
    const current = readJson(STATE_PREFIX + stateKey)
    const next: Record<string, unknown> = current && typeof current === 'object' ? { ...(current as Record<string, unknown>) } : {}
    const parts = jsonPath.split('.').filter(Boolean)
    let cursor = next
    parts.slice(0, -1).forEach((part) => {
      const child = cursor[part]
      cursor[part] = child && typeof child === 'object' ? { ...(child as Record<string, unknown>) } : {}
      cursor = cursor[part] as Record<string, unknown>
    })
    cursor[parts[parts.length - 1]] = value
    root = next
  }
  window.localStorage.setItem(STATE_PREFIX + stateKey, JSON.stringify(root))
}

function openDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = window.indexedDB.open(DB_NAME, 1)
    request.onupgradeneeded = () => request.result.createObjectStore(DB_STORE)
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error ?? new Error('mock wallpaper store unavailable'))
  })
}

async function inStore<T>(mode: IDBTransactionMode, run: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await openDatabase()
  try {
    return await new Promise<T>((resolve, reject) => {
      const transaction = db.transaction(DB_STORE, mode)
      const request = run(transaction.objectStore(DB_STORE))
      transaction.oncomplete = () => resolve(request.result)
      transaction.onerror = () => reject(transaction.error ?? request.error)
      transaction.onabort = () => reject(transaction.error ?? new Error('mock wallpaper transaction aborted'))
    })
  } finally {
    db.close()
  }
}

const mockWallpaperBucket = () => bucket.userBucketId(WALLPAPER_APP_ID, MOCK_USER, WALLPAPER_BUCKET)

export async function uploadMockWallpaper(blob: Blob): Promise<string> {
  const key = await wallpaperKey(blob)
  await inStore('readwrite', (store) => store.put({ blob, createdAt: Date.now() }, key))
  const kept = await inStore<IDBValidKey[]>('readonly', (store) => store.getAllKeys())
  const entries = await Promise.all(
    kept.map(async (item) => {
      const value = await inStore<unknown>('readonly', (store) => store.get(item))
      return { key: String(item), created_at: (value as { createdAt?: number } | undefined)?.createdAt ?? 0 }
    }),
  )
  for (const stale of wallpapersToPrune(entries, key)) await inStore('readwrite', (store) => store.delete(stale))
  return bucket.bucketObjUrl(mockWallpaperBucket(), key)
}

const urls = new Map<string, string>()

export async function mockWallpaperUrl(ref: string): Promise<string | null> {
  const key = bucket.parseBucketObjUrl(ref)?.key
  if (!key) return null
  const known = urls.get(key)
  if (known) return known
  const value = await inStore<unknown>('readonly', (store) => store.get(key)).catch(() => null)
  const blob = (value as { blob?: unknown } | null)?.blob
  if (!(blob instanceof Blob)) return null
  const url = URL.createObjectURL(blob)
  urls.set(key, url)
  return url
}

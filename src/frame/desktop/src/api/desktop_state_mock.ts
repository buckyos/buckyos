/* Mock-runtime stand-in for control-panel `/api/desktop`: the account's state lives in this browser's
 * localStorage and its wallpaper pictures in IndexedDB, so a reload (or a "new device" that only loses the
 * page's own caches) finds them again. Shapes follow ui_session_mgr.rs / desktop_wallpaper.rs. */

import type { DesktopStateRead } from './desktop_state'

const STATE_PREFIX = 'buckyos.mock.desktop-state.'
const DB_NAME = 'buckyos-desktop-wallpaper'
const DB_STORE = 'assets'

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

const extensions: Record<string, string> = {
  'image/jpeg': 'jpg',
  'image/png': 'png',
  'image/gif': 'gif',
  'image/webp': 'webp',
  'image/avif': 'avif',
}

export async function uploadMockWallpaper(blob: Blob): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', await blob.arrayBuffer()))
  const hash = Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('').slice(0, 32)
  const asset = `${hash}.${extensions[blob.type] ?? 'jpg'}`
  await inStore('readwrite', (store) => store.put(blob, asset))
  return asset
}

const urls = new Map<string, string>()

export async function mockWallpaperUrl(asset: string): Promise<string | null> {
  const known = urls.get(asset)
  if (known) return known
  const blob = await inStore<unknown>('readonly', (store) => store.get(asset)).catch(() => null)
  if (!(blob instanceof Blob)) return null
  const url = URL.createObjectURL(blob)
  urls.set(asset, url)
  return url
}

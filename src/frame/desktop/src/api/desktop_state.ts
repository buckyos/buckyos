/* ── The desktop state of the signed-in account (`shared/controlPanel/desktopState.ts`) ──
 *
 * Wallpaper pictures are kept in the account's `user/control-panel/{user}/wallpapers` bucket, keyed by content
 * hash and read back straight from the bucket data plane. The mock runtime keeps both in this browser.
 */

import { bucket, buckyos } from 'buckyos'
import { bucketObjectUrl } from '../components/preview/bucketSource'
import { isMockRuntime } from '../runtime'
import * as state from '../shared/controlPanel/desktopState.ts'
import { shellChannel } from './channel.ts'

export { DESKTOP_API, DESKTOP_SESSION_ID, DesktopApiError, type DesktopStateRead } from '../shared/controlPanel/desktopState.ts'

export const WALLPAPER_APP_ID = 'control-panel'
export const WALLPAPER_BUCKET = 'wallpapers'
/** Pictures kept, the newest included: a device that has not refreshed its state yet still finds the one it shows. */
export const KEPT_WALLPAPERS = 3

export const sendDesktopApi = <T>(url: string, init: RequestInit) => state.sendDesktopApi<T>(shellChannel, url, init)

export async function readDesktopState<T>(stateKey: string): Promise<state.DesktopStateRead<T>> {
  if (isMockRuntime()) {
    const { readMockDesktopState } = await import('./desktop_state_mock')
    return readMockDesktopState<T>(stateKey)
  }
  return state.readDesktopState<T>(shellChannel, stateKey)
}

/** Writes the whole state, or with `jsonPath` (dot-separated) one field of it, leaving the rest as stored. */
export async function writeDesktopState(stateKey: string, value: unknown, jsonPath?: string): Promise<void> {
  if (isMockRuntime()) {
    const { writeMockDesktopState } = await import('./desktop_state_mock')
    writeMockDesktopState(stateKey, value, jsonPath)
    return
  }
  await state.writeDesktopState(shellChannel, stateKey, value, jsonPath)
}

const wallpaperExtensions: Record<string, string> = {
  'image/jpeg': 'jpg',
  'image/png': 'png',
  'image/gif': 'gif',
  'image/webp': 'webp',
  'image/avif': 'avif',
}

/** `{sha256[..32]}.{ext}`: the picture's key in the wallpapers bucket. */
export async function wallpaperKey(blob: Blob): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', await blob.arrayBuffer()))
  const hash = Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('').slice(0, 32)
  return `${hash}.${wallpaperExtensions[blob.type] ?? 'jpg'}`
}

/** Keys to unbind: all but the newest `keep` pictures, never `current`. */
export function wallpapersToPrune(items: readonly { key: string; created_at: number }[], current: string, keep = KEPT_WALLPAPERS): string[] {
  return [...items]
    .sort((a, b) => b.created_at - a.created_at || a.key.localeCompare(b.key))
    .filter((item) => item.key !== current)
    .slice(Math.max(0, keep - 1))
    .map((item) => item.key)
}

async function accountWallpaperBucket(): Promise<string> {
  const account = await buckyos.getAccountInfo().catch(() => null)
  if (!account?.user_id) throw new state.DesktopApiError(401, 'authentication required')
  return bucket.userBucketId(WALLPAPER_APP_ID, account.user_id, WALLPAPER_BUCKET)
}

async function pruneWallpapers(client: bucket.BucketClient, bucketId: string, current: string) {
  const { items } = await client.listObjects(bucketId, { limit: 1000 })
  for (const key of wallpapersToPrune(items, current)) await client.delete(bucketId, key)
}

/** Stores a picture in the account's wallpapers bucket; answers its `obj://bucket/...` reference. */
export async function uploadWallpaper(blob: Blob): Promise<string> {
  if (isMockRuntime()) {
    const { uploadMockWallpaper } = await import('./desktop_state_mock')
    return uploadMockWallpaper(blob)
  }
  const bucketId = await accountWallpaperBucket()
  const key = await wallpaperKey(blob)
  const client = buckyos.getBucketClient()
  try {
    await client.putBytes(bucketId, key, blob, { contentType: blob.type || undefined })
  } catch (error) {
    const status = error instanceof bucket.BucketError ? (error.status ?? 0) : 0
    throw new state.DesktopApiError(status, error instanceof Error ? error.message : String(error))
  }
  // Keeping only the newest few is this app's choice; a failed clean-up is retried by the next upload.
  void pruneWallpapers(client, bucketId, key).catch(() => {})
  return bucket.bucketObjUrl(bucketId, key)
}

/** The picture's URL when it is known without a round trip (the bucket's); null in the mock runtime. */
export function wallpaperUrlNow(ref: string): string | null {
  if (isMockRuntime()) return null
  try {
    return bucketObjectUrl(ref)
  } catch {
    return null
  }
}

/**
 * The picture fetched with the session token in a header, as a `blob:` URL: for when a plain load is refused
 * because the page has no usable session cookie. Null when that fails too.
 */
export async function fetchWallpaperWithToken(ref: string): Promise<string | null> {
  const target = bucket.parseBucketObjUrl(ref)
  if (!target || isMockRuntime()) return null
  try {
    const response = await buckyos.getBucketClient().fetchObject(target.bucketId, target.key)
    return URL.createObjectURL(await response.blob())
  } catch {
    return null
  }
}

/** A URL the page can draw the picture from; the bucket's is immutable (the key is its hash). */
export async function wallpaperUrl(ref: string): Promise<string | null> {
  if (isMockRuntime()) {
    const { mockWallpaperUrl } = await import('./desktop_state_mock')
    return mockWallpaperUrl(ref)
  }
  return wallpaperUrlNow(ref)
}

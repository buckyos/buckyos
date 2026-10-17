/* ── The desktop state of the signed-in account (control-panel `/api/desktop`) ──
 *
 * State is JSON under `users/{user}/desktop/{session}/{state_key}` in SystemConfig; every device of the user
 * uses the well-known `default` session, so what is saved here follows the account. Wallpaper pictures are
 * files in the user's home, read back by content-hash name. The mock runtime keeps both in this browser.
 */

import { buckyos, getActiveSessionToken } from 'buckyos'
import { isMockRuntime } from '../runtime'

export const DESKTOP_SESSION_ID = 'default'
const DESKTOP_API = '/api/desktop'
export const WALLPAPER_API = `${DESKTOP_API}/wallpaper`

export interface DesktopStateRead<T> {
  value: T | null
  /** False when the account never saved this state. */
  exists: boolean
}

/** A failed `/api/desktop` call; `status` is the HTTP status (0 when the server was not reached). */
export class DesktopApiError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function authHeaders(): Promise<Record<string, string>> {
  // The session cookie also authenticates; the header covers a browser that keeps no cookie for the page.
  const account = await buckyos.getAccountInfo().catch(() => null)
  const token = account?.session_token || (await getActiveSessionToken().catch(() => null))
  return token ? { 'X-Auth': token } : {}
}

async function send<T>(url: string, init: RequestInit): Promise<T> {
  let response: Response
  try {
    response = await fetch(url, { credentials: 'same-origin', ...init, headers: { ...(await authHeaders()), ...init.headers } })
  } catch (error) {
    throw new DesktopApiError(0, error instanceof Error ? error.message : String(error))
  }
  const payload = (await response.json().catch(() => null)) as { ok?: boolean; data?: T; error?: string } | null
  if (!response.ok || !payload?.ok) {
    throw new DesktopApiError(response.status, payload?.error ?? `${url} answered ${response.status}`)
  }
  return payload.data as T
}

export async function readDesktopState<T>(stateKey: string): Promise<DesktopStateRead<T>> {
  if (isMockRuntime()) {
    const { readMockDesktopState } = await import('./desktop_state_mock')
    return readMockDesktopState<T>(stateKey)
  }
  const data = await send<{ value: T | null; exists?: boolean }>(DESKTOP_API, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'state.get', session_id: DESKTOP_SESSION_ID, state_key: stateKey }),
  })
  return { value: data.value ?? null, exists: data.exists !== false && data.value !== null }
}

/** Writes the whole state, or with `jsonPath` (dot-separated) one field of it, leaving the rest as stored. */
export async function writeDesktopState(stateKey: string, value: unknown, jsonPath?: string): Promise<void> {
  if (isMockRuntime()) {
    const { writeMockDesktopState } = await import('./desktop_state_mock')
    writeMockDesktopState(stateKey, value, jsonPath)
    return
  }
  await send(DESKTOP_API, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'state.set', session_id: DESKTOP_SESSION_ID, state_key: stateKey, json_path: jsonPath, value }),
    // Finishes even when the page is closing (saved as it is hidden).
    keepalive: true,
  })
}

/** Stores a wallpaper picture in the user's home; answers its asset name. */
export async function uploadWallpaper(blob: Blob): Promise<string> {
  if (isMockRuntime()) {
    const { uploadMockWallpaper } = await import('./desktop_state_mock')
    return uploadMockWallpaper(blob)
  }
  const data = await send<{ asset: string }>(WALLPAPER_API, {
    method: 'POST',
    headers: { 'Content-Type': blob.type || 'application/octet-stream' },
    body: blob,
  })
  return data.asset
}

/** The picture's URL when it is known without a round trip (the server's); null in the mock runtime. */
export function wallpaperAssetUrlNow(asset: string): string | null {
  return isMockRuntime() ? null : `${WALLPAPER_API}/${encodeURIComponent(asset)}`
}

/** A URL the page can draw the picture from; the server's is cached for good (the name is its hash). */
export async function wallpaperAssetUrl(asset: string): Promise<string | null> {
  if (isMockRuntime()) {
    const { mockWallpaperUrl } = await import('./desktop_state_mock')
    return mockWallpaperUrl(asset)
  }
  return `${WALLPAPER_API}/${encodeURIComponent(asset)}`
}

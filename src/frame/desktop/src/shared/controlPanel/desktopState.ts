/* ── The desktop state of the signed-in account (control-panel `/api/desktop`) ──
 *
 * JSON under `users/{user}/desktop/{session}/{state_key}` in SystemConfig; every device of the user uses the
 * well-known `default` session, so what is saved here follows the account.
 */

import type { ZoneChannel } from '../channel.ts'

export const DESKTOP_SESSION_ID = 'default'
export const DESKTOP_API = '/api/desktop'

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

export async function sendDesktopApi<T>(channel: ZoneChannel, url: string, init: RequestInit): Promise<T> {
  let response: Response
  try {
    response = await channel.fetch(url, init)
  } catch (error) {
    throw new DesktopApiError(0, error instanceof Error ? error.message : String(error))
  }
  const payload = (await response.json().catch(() => null)) as { ok?: boolean; data?: T; error?: string } | null
  if (!response.ok || !payload?.ok) {
    throw new DesktopApiError(response.status, payload?.error ?? `${url} answered ${response.status}`)
  }
  return payload.data as T
}

export async function readDesktopState<T>(channel: ZoneChannel, stateKey: string): Promise<DesktopStateRead<T>> {
  const data = await sendDesktopApi<{ value: T | null; exists?: boolean }>(channel, DESKTOP_API, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'state.get', session_id: DESKTOP_SESSION_ID, state_key: stateKey }),
  })
  return { value: data.value ?? null, exists: data.exists !== false && data.value !== null }
}

/** Writes the whole state, or with `jsonPath` (dot-separated) one field of it, leaving the rest as stored. */
export async function writeDesktopState(channel: ZoneChannel, stateKey: string, value: unknown, jsonPath?: string): Promise<void> {
  await sendDesktopApi(channel, DESKTOP_API, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ action: 'state.set', session_id: DESKTOP_SESSION_ID, state_key: stateKey, json_path: jsonPath, value }),
    // Finishes even when the page is closing (saved as it is hidden).
    keepalive: true,
  })
}

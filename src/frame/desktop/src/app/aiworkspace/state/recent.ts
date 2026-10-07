/* The most recently opened workspace (UI improvement §4): app-level state of this browser, kept apart
 * per service target and principal, so another identity never sees it. It cannot live in the user work
 * state, which is only readable once a workspace is open. Written only after an open succeeded. */

import type { Transport } from '../api/transport'

const KEY = 'aiworkspace.recent'

interface Recent { workspace_id: string; at: string }

function readAll(): Record<string, Recent> {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(KEY) ?? '{}') as unknown
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, Recent> : {}
  } catch {
    return {}
  }
}

function writeAll(all: Record<string, Recent>) {
  try { window.localStorage.setItem(KEY, JSON.stringify(all)) } catch { /* the record is a convenience: without storage nothing is restored */ }
}

function slot(transport: Transport, principal: string): string {
  return `${transport.mode}|${transport.target}|${principal}`
}

export async function readRecent(transport: Transport): Promise<Recent | null> {
  const principal = await transport.principal()
  if (!principal) return null
  return readAll()[slot(transport, principal)] ?? null
}

export async function rememberRecent(transport: Transport, workspaceId: string): Promise<void> {
  const principal = await transport.principal()
  if (!principal) return
  writeAll({ ...readAll(), [slot(transport, principal)]: { workspace_id: workspaceId, at: new Date().toISOString() } })
}

export async function forgetRecent(transport: Transport, workspaceId: string): Promise<void> {
  const principal = await transport.principal()
  if (!principal) return
  const all = readAll()
  if (all[slot(transport, principal)]?.workspace_id !== workspaceId) return
  delete all[slot(transport, principal)]
  writeAll(all)
}

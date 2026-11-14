/* What this identity opened, as app-level state of this browser, kept apart per service target and principal, so
 * another identity never sees it. It cannot live in the user work state, which is only readable once a workspace is
 * open. Two records:
 *
 * - the restore record (UI improvement §4): the most recently opened workspace, written only after an open succeeded;
 *   a normal start reopens it;
 * - the home's "Recent" (UI framework PRD §9.1): the workspaces and public classes opened from the home or in this
 *   window, newest first, at most RECENT_LIMIT. Until the service keeps when a user last opened something (K6) this is
 *   all there is: a workspace origin's own storage and other devices are not seen here.
 *
 * The home's personal settings (its filters, ui/home/prefs.ts) are kept the same way, by `identitySlot`. */

import type { Transport } from '../api/transport'

const KEY = 'aiworkspace.recent'
const LIST_KEY = 'aiworkspace.recent.list'
export const RECENT_LIMIT = 20

interface Recent { workspace_id: string; at: string }

export type RecentKind = 'workspace' | 'session'
/** One thing opened: a workspace (`id` its workspace id) or a public class (`id` its session id). */
export interface RecentEntry { kind: RecentKind; id: string; at: string }

function readKey<T>(key: string): Record<string, T> {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(key) ?? '{}') as unknown
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, T> : {}
  } catch {
    return {}
  }
}

function writeKey<T>(key: string, all: Record<string, T>) {
  try { window.localStorage.setItem(key, JSON.stringify(all)) } catch { /* the record is a convenience: without storage nothing is remembered */ }
}

function slot(transport: Transport, principal: string): string {
  return `${transport.mode}|${transport.target}|${principal}`
}

/** The key under which this identity's app-level records are kept; null for nobody (a guest without a principal). */
export async function identitySlot(transport: Transport): Promise<string | null> {
  const principal = await transport.principal()
  return principal ? slot(transport, principal) : null
}

export async function readRecent(transport: Transport): Promise<Recent | null> {
  const key = await identitySlot(transport)
  if (!key) return null
  return readKey<Recent>(KEY)[key] ?? null
}

export async function rememberRecent(transport: Transport, workspaceId: string): Promise<void> {
  const key = await identitySlot(transport)
  if (!key) return
  writeKey(KEY, { ...readKey<Recent>(KEY), [key]: { workspace_id: workspaceId, at: new Date().toISOString() } })
  noteIn(key, 'workspace', workspaceId)
}

export async function forgetRecent(transport: Transport, workspaceId: string): Promise<void> {
  const key = await identitySlot(transport)
  if (!key) return
  forgetIn(key, 'workspace', workspaceId)
  const all = readKey<Recent>(KEY)
  if (all[key]?.workspace_id !== workspaceId) return
  delete all[key]
  writeKey(KEY, all)
}

function validEntries(value: unknown): RecentEntry[] {
  return Array.isArray(value) ? value.filter((e): e is RecentEntry => !!e && typeof e === 'object' && (e.kind === 'workspace' || e.kind === 'session') && typeof e.id === 'string' && typeof e.at === 'string') : []
}

function noteIn(key: string, kind: RecentKind, id: string) {
  const all = readKey<unknown>(LIST_KEY)
  const rest = validEntries(all[key]).filter((e) => !(e.kind === kind && e.id === id))
  writeKey(LIST_KEY, { ...all, [key]: [{ kind, id, at: new Date().toISOString() }, ...rest].slice(0, RECENT_LIMIT) })
}

function forgetIn(key: string, kind: RecentKind, id: string) {
  const all = readKey<unknown>(LIST_KEY)
  const list = validEntries(all[key])
  if (!list.some((e) => e.kind === kind && e.id === id)) return
  writeKey(LIST_KEY, { ...all, [key]: list.filter((e) => !(e.kind === kind && e.id === id)) })
}

/** The home's "Recent", newest first. The restore record counts too (it was written before this list existed). */
export async function readRecentList(transport: Transport): Promise<RecentEntry[]> {
  const key = await identitySlot(transport)
  if (!key) return []
  const list = validEntries(readKey<unknown>(LIST_KEY)[key])
  const restore = readKey<Recent>(KEY)[key]
  if (restore && !list.some((e) => e.kind === 'workspace' && e.id === restore.workspace_id)) {
    list.push({ kind: 'workspace', id: restore.workspace_id, at: restore.at })
    list.sort((a, b) => b.at.localeCompare(a.at))
  }
  return list.slice(0, RECENT_LIMIT)
}

/** Something was opened from the home (before going to it: a workspace may open on its own origin, a public class in
 * a tab of its own). The restore record is not touched: it follows only opens that succeeded in this window. */
export async function noteOpened(transport: Transport, kind: RecentKind, id: string): Promise<void> {
  const key = await identitySlot(transport)
  if (key) noteIn(key, kind, id)
}

/** A workspace that is gone (deleted from the home) leaves "Recent" and the restore record. */
export async function forgetOpened(transport: Transport, kind: RecentKind, id: string): Promise<void> {
  if (kind === 'workspace') return forgetRecent(transport, id)
  const key = await identitySlot(transport)
  if (key) forgetIn(key, kind, id)
}

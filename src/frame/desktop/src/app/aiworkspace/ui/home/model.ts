/* What the home shows (UI framework PRD §9): `ws.list` with the card fields of K6 — owner, last modification, when I
 * last opened it, public, live classes — and `session.list_open`, every public class I may open without a link (K7),
 * with my role and when I was last in it. "Recent" is what the service records I opened, on any device, joined with
 * what this window opened a moment ago (a class opens in a tab of its own, and its room records the visit only once it
 * is entered). Covers are not generated yet: the cards draw their placeholder. */

import type { WorkspaceListItem } from '../../api/types'
import type { RecentEntry } from '../../state/recent'
import { roleOf as roleOfCaps, type WorkspaceRole } from '../../state/settings'
import type { OpenSession } from '../../session/types'
import type { HomePrefs, SessionStateFilter } from './prefs'

/** My role in a workspace (§9.2 “我的角色”): its owner, or what its capabilities let me do. */
export type Role = WorkspaceRole

export function roleOf(workspace: WorkspaceListItem, me: string | null): Role {
  return roleOfCaps(workspace.capabilities, isMine(workspace, me))
}

export function isMine(workspace: WorkspaceListItem, me: string | null): boolean {
  return Boolean(me) && workspace.owner_principal === me
}

export const ROLE_KEY: Readonly<Record<Role, string>> = {
  owner: 'aiworkspace.home.role.owner', editor: 'aiworkspace.home.role.edit', commenter: 'aiworkspace.home.role.comment', viewer: 'aiworkspace.home.role.read',
}

/** A hue from the workspace id: cards without a cover still tell apart. */
export function hueOf(id: string): number {
  let h = 2166136261
  for (let i = 0; i < id.length; i += 1) { h ^= id.charCodeAt(i); h = Math.imul(h, 16777619) }
  return (h >>> 0) % 360
}

/** What a click on a public class card does: go in (live, not started), replay (ended, saved), nothing (expired). */
export type SessionAction = 'enter' | 'replay' | 'expired'

export function sessionAction(session: Pick<OpenSession, 'state'>): SessionAction {
  if (session.state === 'expired') return 'expired'
  return session.state === 'ended' || session.state === 'archived' ? 'replay' : 'enter'
}

const sessionTime = (s: OpenSession) => s.started_at ?? s.created_at

/** Live first, the rest by start time, newest first (§9.1). */
export function sortSessions(list: readonly OpenSession[]): OpenSession[] {
  return [...list].sort((a, b) => Number(b.state === 'live') - Number(a.state === 'live') || sessionTime(b).localeCompare(sessionTime(a)))
}

const STATE_FILTER: Readonly<Record<SessionStateFilter, (s: OpenSession) => boolean>> = {
  all: () => true,
  live: (s) => s.state === 'live',
  upcoming: (s) => s.state === 'prepared',
  replay: (s) => s.state === 'ended' || s.state === 'archived',
  expired: (s) => s.state === 'expired',
}

const norm = (text: string) => text.toLocaleLowerCase().normalize('NFKC')

/** Search terms: every one must occur in one of the texts. */
export function matches(query: string, texts: ReadonlyArray<string | null | undefined>): boolean {
  const terms = norm(query).split(/\s+/).filter(Boolean)
  if (terms.length === 0) return true
  const hay = texts.filter(Boolean).map((text) => norm(text as string))
  return terms.every((term) => hay.some((text) => text.includes(term)))
}

export interface WorkspaceFacts {
  /** Who I am (the owner check), null before it is known. */
  me: string | null
  /** Prepared for offline on this device. */
  prepared: ReadonlySet<string>
}

export function filterWorkspaces(list: readonly WorkspaceListItem[], prefs: HomePrefs, query: string, facts: WorkspaceFacts): WorkspaceListItem[] {
  const { owner, permission, offline } = prefs.workspaces
  return list.filter((ws) => {
    if (offline && !facts.prepared.has(ws.workspace_id)) return false
    if (permission !== 'all') {
      const role = roleOf(ws, facts.me)
      const editable = role === 'owner' || role === 'editor'
      if (permission === 'edit' ? !editable : editable) return false
    }
    if (owner !== 'all' && (owner === 'mine') !== isMine(ws, facts.me)) return false
    return matches(query, [ws.title, isMine(ws, facts.me) ? null : ws.owner_principal])
  })
}

const desc = (a: string | null | undefined, b: string | null | undefined) => (b ?? '').localeCompare(a ?? '')

export function sortWorkspaces(list: readonly WorkspaceListItem[], prefs: HomePrefs, compare: (a: string, b: string) => number): WorkspaceListItem[] {
  const byName = (a: WorkspaceListItem, b: WorkspaceListItem) => compare(a.title, b.title) || a.workspace_id.localeCompare(b.workspace_id)
  switch (prefs.workspaces.sort) {
    case 'name': return [...list].sort(byName)
    case 'opened': return [...list].sort((a, b) => desc(a.opened_at, b.opened_at) || desc(a.updated_at, b.updated_at) || byName(a, b))
    default: return [...list].sort((a, b) => desc(a.updated_at, b.updated_at) || byName(a, b))
  }
}

export function sessionTexts(s: OpenSession): Array<string | undefined> {
  return [s.title, s.baseline?.source.title, s.presenter]
}

export function filterSessions(list: readonly OpenSession[], prefs: HomePrefs, query: string): OpenSession[] {
  const keep = STATE_FILTER[prefs.sessions.state]
  return list.filter((s) => keep(s) && (!prefs.sessions.mine || s.presenting) && matches(query, sessionTexts(s)))
}

export type RecentItem = { kind: 'workspace'; at: string; workspace: WorkspaceListItem } | { kind: 'session'; at: string; session: OpenSession }

/** At most this many in "Recent" (§9.1). */
export const RECENT_LIMIT = 20

/** "Recent" (§9.1): workspaces and public classes I opened, newest first — the service's record of when (`opened_at`),
 * or this window's own when later; only what the service lists now. */
export function recentItems(workspaces: readonly WorkspaceListItem[] | undefined, sessions: readonly OpenSession[] | undefined, local: readonly RecentEntry[]): RecentItem[] {
  const localAt = new Map(local.map((e) => [`${e.kind}:${e.id}`, e.at]))
  const latest = (key: string, server: string | null | undefined) => {
    const here = localAt.get(key)
    return !server ? here ?? null : here && here > server ? here : server
  }
  const out: RecentItem[] = []
  for (const workspace of workspaces ?? []) {
    const at = latest(`workspace:${workspace.workspace_id}`, workspace.opened_at)
    if (at) out.push({ kind: 'workspace', at, workspace })
  }
  for (const session of sessions ?? []) {
    const at = latest(`session:${session.session_id}`, session.opened_at)
    if (at) out.push({ kind: 'session', at, session })
  }
  return out.sort((a, b) => b.at.localeCompare(a.at)).slice(0, RECENT_LIMIT)
}

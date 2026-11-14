/* The home's personal settings (UI framework PRD §9.3, §12): the category it shows, its filters and its sort, kept for
 * next time on any device — the cross-workspace preference `home` (state/prefs.ts, K4). A session that keeps no
 * preferences (refused: a restricted account) keeps them in this browser instead, per service target and principal like
 * the home's other app-level records (state/recent.ts): key `aiworkspace.homePrefs`. Without a category kept, the home
 * starts on "Recent" when there is any, else on "All workspaces" (§9.1). */

import { useCallback, useEffect, useState } from 'react'
import type { Transport } from '../../api/transport'
import type { Json } from '../../api/types'
import { useUserPrefs } from '../../state/prefs'
import { identitySlot } from '../../state/recent'

const KEY = 'aiworkspace.homePrefs'
/** The preference holding the home's settings. */
export const HOME_PREF = 'home'

export type HomeCategory = 'recent' | 'all' | 'sessions'
export type OwnerFilter = 'all' | 'mine' | 'shared'
export type PermissionFilter = 'all' | 'edit' | 'read'
export type WorkspaceSort = 'modified' | 'opened' | 'name'
export type SessionStateFilter = 'all' | 'live' | 'upcoming' | 'replay' | 'expired'

export interface HomePrefs {
  /** The category last picked; null: the default (§9.1). */
  category: HomeCategory | null
  workspaces: { owner: OwnerFilter; permission: PermissionFilter; offline: boolean; sort: WorkspaceSort }
  sessions: { state: SessionStateFilter; mine: boolean }
}

/** "All workspaces" sorts by the last modification first (§9.1). */
export const DEFAULT_PREFS: HomePrefs = {
  category: null,
  workspaces: { owner: 'all', permission: 'all', offline: false, sort: 'modified' },
  sessions: { state: 'all', mine: false },
}

const pick = <T extends string>(value: unknown, allowed: readonly T[], fallback: T): T => (allowed.includes(value as T) ? value as T : fallback)

function parse(raw: unknown): HomePrefs {
  const p = (raw && typeof raw === 'object' ? raw : {}) as { category?: unknown; workspaces?: Record<string, unknown>; sessions?: Record<string, unknown> }
  const w = p.workspaces ?? {}
  const s = p.sessions ?? {}
  return {
    category: (['recent', 'all', 'sessions'] as const).includes(p.category as HomeCategory) ? p.category as HomeCategory : null,
    workspaces: {
      owner: pick(w.owner, ['all', 'mine', 'shared'], DEFAULT_PREFS.workspaces.owner),
      permission: pick(w.permission, ['all', 'edit', 'read'], DEFAULT_PREFS.workspaces.permission),
      offline: w.offline === true,
      sort: pick(w.sort, ['modified', 'opened', 'name'], DEFAULT_PREFS.workspaces.sort),
    },
    sessions: {
      state: pick(s.state, ['all', 'live', 'upcoming', 'replay', 'expired'], DEFAULT_PREFS.sessions.state),
      mine: s.mine === true,
    },
  }
}

function readAll(): Record<string, unknown> {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(KEY) ?? '{}') as unknown
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, unknown> : {}
  } catch {
    return {}
  }
}

/** This browser's copy, for a session that keeps no preferences. */
function useLocalPrefs(transport: Transport, active: boolean): [HomePrefs | null, (next: HomePrefs) => void] {
  const [state, setState] = useState<{ slot: string | null; prefs: HomePrefs | null }>({ slot: null, prefs: null })
  useEffect(() => {
    if (!active) return
    let live = true
    void identitySlot(transport).then((slot) => { if (live) setState({ slot, prefs: slot ? parse(readAll()[slot]) : DEFAULT_PREFS }) })
    return () => { live = false }
  }, [transport, active])
  const set = useCallback((prefs: HomePrefs) => {
    setState((current) => {
      if (current.slot) {
        try { window.localStorage.setItem(KEY, JSON.stringify({ ...readAll(), [current.slot]: prefs })) } catch { /* a convenience: without storage the settings last for this page */ }
      }
      return { ...current, prefs }
    })
  }, [])
  return [state.prefs, set]
}

/** The home's settings of this person and their setter; `ready` once they are known (before: the defaults). */
export function useHomePrefs(transport: Transport): { prefs: HomePrefs; ready: boolean; set: (next: HomePrefs) => void } {
  const prefs = useUserPrefs(transport)
  const local = prefs.state === 'unavailable'
  const [localPrefs, setLocal] = useLocalPrefs(transport, local)
  if (local) return { prefs: localPrefs ?? DEFAULT_PREFS, ready: localPrefs !== null, set: setLocal }
  return {
    prefs: parse(prefs.get<Json>(HOME_PREF, null)),
    ready: prefs.state !== 'loading',
    set: (next) => { void prefs.set({ [HOME_PREF]: next as unknown as Json }) },
  }
}

export function isDefaultWorkspaceFilter(prefs: HomePrefs): boolean {
  const w = prefs.workspaces
  return w.owner === 'all' && w.permission === 'all' && !w.offline
}

export function isDefaultSessionFilter(prefs: HomePrefs): boolean {
  return prefs.sessions.state === 'all' && !prefs.sessions.mine
}

/** Whether the filters that apply to this category leave anything out. */
export function filtersActive(category: HomeCategory, prefs: HomePrefs): boolean {
  if (category === 'all') return !isDefaultWorkspaceFilter(prefs)
  if (category === 'sessions') return !isDefaultSessionFilter(prefs)
  return !isDefaultWorkspaceFilter(prefs) || !isDefaultSessionFilter(prefs)
}

/** The filters (and the sort) back to the defaults; the category stays. */
export function clearedFilters(prefs: HomePrefs): HomePrefs {
  return { ...DEFAULT_PREFS, category: prefs.category }
}

/* Macros (UI 框架 PRD §4.2): a macro is a wish's saved program. The "宏" entry lists the wishes of the outline whose
 * macro the current user can run now — they may update the wish, the analysis is ready and the macro fits its inputs
 * (not "needs coding"; the Mock executor counts as having one, as in the wish panel). The one used last comes first
 * (per workspace, user state), then the macros on the current canvas, then the rest. Running one is exactly the wish
 * panel's "运行" (`WishService.runMacro`): same identity, same result handling — a candidate waits in the wish. Its start,
 * progress and result are entries of the status bar's work log (K3), not notices; only a run the service never took (it
 * could not start) is said above the status bar, since the log has nothing of it. */

import { useEffect, useSyncExternalStore } from 'react'
import { describeError } from '../../api/session'
import type { EntityEnvelope } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useOutlineVersion, useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { isActive } from '../wish/WishService'

export const RECENT_MACRO_KEY = 'ui:recent-macro'

export interface Macro {
  wishId: string
  title: string
  /** Its Block on the current canvas, else any of its Blocks. */
  cellId: string | null
  onSurface: boolean
  /** A run of it is in progress in this window. */
  busy: boolean
}

function wishes(store: WorkspaceStore): EntityEnvelope[] {
  return store.outline.all().filter((e) => e.type_id === 'buckyos.wish' && !e.deleted)
}

function writable(store: WorkspaceStore): boolean {
  const mode = store.session.mode()
  if (mode.kind === 'direct' && mode.reason !== 'not_prepared' && store.session.status().kind === 'offline') return false
  return !(store.session.showLock() && mode.kind === 'direct')
}

/** The runnable macros, in menu order. Reads the freshness the toolbars keep watched (`useMacroWatch`). */
export function listMacros(store: WorkspaceStore, surfaceId: string | null): Macro[] {
  if (!writable(store)) return []
  const recent = store.userState.get<string>(RECENT_MACRO_KEY)
  const cells = store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && !e.deleted && e.source_id)
  const out: Macro[] = []
  for (const wish of wishes(store)) {
    if (!wish.capabilities.includes('update')) continue
    const fresh = store.freshness.get(wish.entity_id)
    if (!fresh || fresh.needs_analysis || fresh.analysis_status !== 'ready') continue
    if (wish.executor !== 'mock' && fresh.needs_coding) continue
    const views = cells.filter((cell) => cell.source_id === wish.entity_id)
    const here = surfaceId ? views.find((cell) => store.outline.ancestors(cell.entity_id).includes(surfaceId)) : undefined
    out.push({
      wishId: wish.entity_id, title: wish.title ?? wish.name ?? wish.entity_id, cellId: (here ?? views[0])?.entity_id ?? null, onSurface: Boolean(here),
      busy: isActive(store.wish.run(wish.entity_id)) || store.wish.busy.has(wish.entity_id),
    })
  }
  const rank = (m: Macro) => (m.wishId === recent ? 0 : m.onSurface ? 1 : 2)
  return out.map((m, i) => ({ m, i })).sort((a, b) => rank(a.m) - rank(b.m) || a.i - b.i).map(({ m }) => m)
}

/** Keep the wishes' freshness fetched and re-render on run changes, while a toolbar that offers macros is shown. */
export function useMacroWatch() {
  const store = useStore()
  const outlineVersion = useOutlineVersion()
  useSyncExternalStore(store.wish.subscribe, store.wish.snapshot)
  useSyncExternalStore(store.freshness.subscribe, store.freshness.snapshot)
  useEffect(() => {
    const ids = wishes(store).map((e) => e.entity_id)
    const offs = ids.map((id) => store.freshness.watch(id, () => undefined))
    return () => { for (const off of offs) off() }
  }, [store, outlineVersion])
}

/** Run a macro as the wish panel's "运行" does; remember it as the last one used. What becomes of it is in the work log. */
export function runMacro(store: WorkspaceStore, macro: Macro, openEntity: (id: string) => void) {
  store.userState.set(RECENT_MACRO_KEY, macro.wishId)
  const name = macro.title
  store.wish.runMacro(macro.wishId, macro.cellId ? { cellId: macro.cellId } : {}).then(
    () => undefined,
    (error: unknown) => {
      const show = macro.cellId ? { label: aiwsT('aiworkspace.framework.macro.open'), run: () => openEntity(macro.cellId ?? macro.wishId) } : undefined
      store.notify('error', aiwsT('aiworkspace.framework.macro.failed', { name, error: describeError(error) }), show)
    },
  )
}

/* The person's own layout across Workspaces (UI 框架 PRD §4.6, §12; K4): the tool zones' switches (and, later, their
 * personal order), whether the status bar is expanded. They live in the cross-workspace preferences
 * (state/prefs.ts, `user.get_prefs`), so they follow the person from one Workspace origin to another and to other
 * devices; nobody else sees them. What refers to a Workspace's own entities — the pinned extension controls, the
 * macro used last — stays in that Workspace's user state (`ui:pinned-defs`, `ui:recent-macro`). */

import type { Transport } from '../../api/transport'
import type { Json } from '../../api/types'
import { useUserPrefs, userPrefsStore, type UserPrefs } from '../../state/prefs'
import { useShell } from '../shell/shellContext'
import type { ZoneId } from './actions'
import { parseToolbarConfig, type ToolbarConfig } from './config'

/** The personal layer of the toolbar configuration: `{ <zone>: { hidden?, order? } }`. */
export const ZONES_PREF = 'zones'
/** The status bar is expanded (§4.5). */
export const STATUS_EXPANDED_PREF = 'status.expanded'
/** Where the properties panel is pinned (§10.2; written by ui/blockui/PropertyPanel's host). */
export const PROPS_PANEL_PREF = 'inspector.dock'

type ZonesPref = Partial<Record<ZoneId, Record<string, Json>>>

function zonesOf(prefs: UserPrefs): ZonesPref {
  const value = prefs.get<Json>(ZONES_PREF, {})
  return value && typeof value === 'object' && !Array.isArray(value) ? value as ZonesPref : {}
}

/** The personal layer as the merge takes it. */
export function personalToolbar(prefs: UserPrefs): ToolbarConfig | null {
  return parseToolbarConfig({ zones: zonesOf(prefs) as Json })
}

/** The zone switches of "视图 ▸ 工具区" as the person set them (true: shown, or never set). */
export function personalZoneShown(prefs: UserPrefs, zone: ZoneId): boolean {
  return zonesOf(prefs)[zone]?.hidden !== true
}

/** Show or hide a zone for this person (null: back to what the Workspace and the canvas say). */
export function setPersonalZone(prefs: UserPrefs, zone: ZoneId, hidden: boolean | null): Promise<void> {
  const next: ZonesPref = { ...zonesOf(prefs) }
  const own: Record<string, Json> = { ...(next[zone] ?? {}) }
  delete own.hidden
  if (hidden !== null) own.hidden = hidden
  if (Object.keys(own).length) next[zone] = own
  else delete next[zone]
  return prefs.set({ [ZONES_PREF]: Object.keys(next).length ? next as Json : null })
}

/** "恢复默认布局": the personal layout (zones, the status bar, the pinned properties panel) back to the defaults. */
export function resetPersonalLayout(prefs: UserPrefs): Promise<void> {
  return prefs.set({ [ZONES_PREF]: null, [STATUS_EXPANDED_PREF]: null, [PROPS_PANEL_PREF]: null })
}

/** The status bar expanded or not, with its setter (the shell's transport). */
export function useStatusExpanded(): [boolean, (open: boolean) => void] {
  const prefs = useUserPrefs(useShell().client.transport)
  return [prefs.get<Json>(STATUS_EXPANDED_PREF, false) === true, (open) => { void prefs.set({ [STATUS_EXPANDED_PREF]: open ? true : null }) }]
}

/** Expand the status bar from outside React (a new problem that needs attention, §4.5). */
export function expandStatus(transport: Transport): void {
  void userPrefsStore(transport).set({ [STATUS_EXPANDED_PREF]: true })
}

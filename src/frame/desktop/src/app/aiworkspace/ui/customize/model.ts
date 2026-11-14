/* The customize dialog's model (UI 框架 PRD §4.6): one shared layer — this canvas or this Workspace — edited as the
 * zones it results in, and written back as the smallest layer that gives that result.
 *
 * The dialog starts from what the layers below give (built-in, plus the Workspace for a canvas) with the edited layer on
 * top, hidden items kept and marked. What it writes for a zone:
 *   - nothing when the zone is as the layers below give it;
 *   - the items that changed (shown / hidden, their name and icon) and the ones added at the end, merged in place by
 *     their keys (`append`), when the order of what was there is kept — later built-in items still appear;
 *   - the whole list (`override`) once the order changed.
 * Hiding and locking a whole zone are flags of their own; App mode is all four zones hidden and locked. */

import type { Json } from '../../api/types'
import type { ZoneId } from '../framework/actions'
import { CUSTOMIZABLE_ZONES, itemKey, mergeToolbar, type ConfigLayer, type ResolvedItem, type ResolvedToolbar, type ToolbarConfig, type ToolItemConfig, type ZoneConfig } from '../framework/config'

export interface DraftItem {
  key: string
  action: string
  args?: Json
  title?: string
  icon?: string
  when?: ToolItemConfig['when']
  hidden: boolean
  /** Added in this layer (it can be removed; the others only hidden). */
  added: boolean
}

export interface DraftZone { hidden: boolean; locked: boolean; items: DraftItem[] }
export type Draft = Record<ZoneId, DraftZone>

/** What the layers below the edited one give, hidden items kept. */
export function baseOf(below: ConfigLayer[]): ResolvedToolbar {
  return mergeToolbar(below, { keepHidden: true })
}

function draftItem(item: ResolvedItem, added: boolean): DraftItem {
  return { key: item.key, action: item.action, args: item.args, title: item.title, icon: item.icon, when: item.when, hidden: item.hidden ?? false, added }
}

/** The dialog's starting point: the layers below with the edited one on top. */
export function draftOf(below: ConfigLayer[], layer: ConfigLayer): Draft {
  const base = baseOf(below)
  const edited = mergeToolbar([...below, layer], { keepHidden: true })
  const out = {} as Draft
  for (const zone of CUSTOMIZABLE_ZONES) {
    const known = new Set(base[zone].items.map((item) => item.key))
    out[zone] = { hidden: edited[zone].hidden, locked: edited[zone].locked, items: edited[zone].items.map((item) => draftItem(item, !known.has(item.key))) }
  }
  return out
}

/** The layers below as a draft (恢复默认: the edited layer removed). */
export function defaultDraft(below: ConfigLayer[]): Draft {
  const base = baseOf(below)
  const out = {} as Draft
  for (const zone of CUSTOMIZABLE_ZONES) out[zone] = { hidden: base[zone].hidden, locked: base[zone].locked, items: base[zone].items.map((item) => draftItem(item, false)) }
  return out
}

function itemConfig(item: DraftItem, full: boolean, base?: ResolvedItem): ToolItemConfig {
  const custom = item.key !== itemKey({ action: item.action, args: item.args })
  const out: ToolItemConfig = { ...(custom ? { id: item.key } : {}), action: item.action, ...(item.args !== undefined ? { args: item.args } : {}) }
  if (full || item.title !== base?.title) if (item.title !== undefined) out.title = item.title
  if (full || item.icon !== base?.icon) if (item.icon !== undefined) out.icon = item.icon
  if (full ? item.hidden : item.hidden !== (base?.hidden ?? false)) out.hidden = item.hidden
  if (full && item.when) out.when = item.when
  return out
}

/** The layer that, over the layers below, gives the draft; null when the draft is what they give. */
export function layerOf(below: ConfigLayer[], draft: Draft): ToolbarConfig | null {
  const base = baseOf(below)
  const zones: Partial<Record<ZoneId, ZoneConfig>> = {}
  for (const zone of CUSTOMIZABLE_ZONES) {
    const want = draft[zone]
    const have = base[zone]
    const zc: ZoneConfig = {}
    if (want.hidden !== have.hidden) zc.hidden = want.hidden
    if (want.locked && !have.locked) zc.locked = true
    const baseKeys = have.items.map((item) => item.key)
    const kept = want.items.filter((item) => baseKeys.includes(item.key)).map((item) => item.key)
    const lastKept = Math.max(-1, ...kept.map((key) => want.items.findIndex((item) => item.key === key)))
    const addedAtEnd = want.items.every((item, i) => baseKeys.includes(item.key) || i > lastKept)
    const sameOrder = kept.length === baseKeys.length && kept.every((key, i) => key === baseKeys[i])
    if (sameOrder && addedAtEnd) {
      const items: ToolItemConfig[] = []
      for (const item of want.items) {
        const before = have.items.find((entry) => entry.key === item.key)
        if (!before) { items.push(itemConfig(item, true)); continue }
        const changed = itemConfig(item, false, before)
        if (Object.keys(changed).some((k) => k !== 'id' && k !== 'action' && k !== 'args')) items.push(changed)
      }
      if (items.length) zc.items = items
    } else {
      zc.relation = 'override'
      zc.items = want.items.map((item) => itemConfig(item, true))
    }
    if (Object.keys(zc).length) zones[zone] = zc
  }
  return Object.keys(zones).length ? { zones } : null
}

/** App mode (§4.6): every zone hidden and locked. */
export function appModeDraft(draft: Draft): Draft {
  const out = { ...draft }
  for (const zone of CUSTOMIZABLE_ZONES) out[zone] = { ...draft[zone], hidden: true, locked: true }
  return out
}

export function isAppModeDraft(draft: Draft): boolean {
  return CUSTOMIZABLE_ZONES.every((zone) => draft[zone].hidden && draft[zone].locked)
}

/** Move an item of a zone by `delta` places. */
export function moveItem(zone: DraftZone, key: string, delta: number): DraftZone {
  const from = zone.items.findIndex((item) => item.key === key)
  const to = from + delta
  if (from < 0 || to < 0 || to >= zone.items.length) return zone
  const items = [...zone.items]
  const [item] = items.splice(from, 1)
  items.splice(to, 0, item)
  return { ...zone, items }
}

/** Put an item before another (a drag ended over it). */
export function placeBefore(zone: DraftZone, key: string, beforeKey: string | null): DraftZone {
  const item = zone.items.find((entry) => entry.key === key)
  if (!item || key === beforeKey) return zone
  const rest = zone.items.filter((entry) => entry.key !== key)
  const at = beforeKey === null ? rest.length : rest.findIndex((entry) => entry.key === beforeKey)
  rest.splice(at < 0 ? rest.length : at, 0, item)
  return { ...zone, items: rest }
}
